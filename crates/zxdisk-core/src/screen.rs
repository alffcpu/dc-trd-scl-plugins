//! ZX Spectrum screen decoder: turn a raw `.scr`-style screen dump into an RGBA
//! image ready to blit. This is pure, cross-platform pixel work - the WLX
//! plugins are only a thin native window over the buffer this produces.
//!
//! Two formats, told apart purely by length (the format has no signature):
//!   * **6912 bytes** - full screen: a 6144-byte bitmap + a 768-byte attribute
//!     map (32x24 cells of 8x8 pixels).
//!   * **6144 bytes** - bitmap only, no colour: rendered white ink on a black
//!     paper by default.
//!
//! The 256x192 bitmap is stored in the ZX "thirds" interleave: the screen is
//! three 64-line bands, and within a band the pixel-row order is shuffled. The
//! byte holding pixel (x, y) is at
//!   `(y>>6)*2048 + (y&7)*256 + ((y>>3)&7)*32 + (x>>3)`,
//! with the most-significant bit the leftmost pixel.
//!
//! An attribute byte is `FLASH(7) BRIGHT(6) PAPER(5..3) INK(2..0)`; the 3-bit
//! colour is `G R B` (bit2 green, bit1 red, bit0 blue). Eight base colours times
//! BRIGHT give 15 distinct colours (bright black == black). FLASH swaps INK and
//! PAPER on a timer; [`Screen::render`] takes the flash phase so the caller can
//! animate it (authentic toggle is ~320 ms).
//!
//! Colours come from a 16-entry [`Palette`] (0..8 normal, 8..16 bright). Several
//! presets are bundled ([`named_palettes`]), ported from the Xpeccy emulator, so
//! the exact tint is a matter of taste; the default is [`PAL_PULSAR`].

/// Screen width in pixels.
pub const SCREEN_W: usize = 256;
/// Screen height in pixels.
pub const SCREEN_H: usize = 192;
/// Length of the bitmap area (6144 bytes).
pub const BITMAP_LEN: usize = SCREEN_W * SCREEN_H / 8;
/// Length of the attribute area (768 bytes).
pub const ATTR_LEN: usize = (SCREEN_W / 8) * (SCREEN_H / 8);
/// Length of a full screen with attributes (6912 bytes).
pub const SCREEN_LEN: usize = BITMAP_LEN + ATTR_LEN;

/// Upper bound on the integer zoom [`Screen::render`] will honour (a guard
/// against overflow, far above any sensible viewer zoom).
pub const MAX_SCALE: usize = 8;
/// Upper bound on the border thickness [`Screen::render`] will honour.
pub const MAX_BORDER: usize = 512;

/// One RGB colour.
pub type Rgb = [u8; 3];
/// A ZX palette: entries 0..8 are the normal colours, 8..16 their BRIGHT twins.
pub type Palette = [Rgb; 16];

// ---- bundled palettes (ported from Xpeccy's conf/palettes/*.txt) ------------
// Order per entry: Black, Blue, Red, Magenta, Green, Cyan, Yellow, White, then
// the same 8 in BRIGHT. This matches the ZX colour index (bits G R B).

/// Well-known "Pulsar" palette (0xCD / 0xFF). The default.
pub const PAL_PULSAR: Palette = [
    [0x00, 0x00, 0x00], [0x00, 0x00, 0xcd], [0xcd, 0x00, 0x00], [0xcd, 0x00, 0xcd],
    [0x00, 0xcd, 0x00], [0x00, 0xcd, 0xcd], [0xcd, 0xcd, 0x00], [0xcd, 0xcd, 0xcd],
    [0x00, 0x00, 0x00], [0x00, 0x00, 0xff], [0xff, 0x00, 0x00], [0xff, 0x00, 0xff],
    [0x00, 0xff, 0x00], [0x00, 0xff, 0xff], [0xff, 0xff, 0x00], [0xff, 0xff, 0xff],
];
/// Wikipedia palette #1.
pub const PAL_WIKI1: Palette = [
    [0x00, 0x00, 0x00], [0x01, 0x00, 0xce], [0xcf, 0x01, 0x00], [0xcf, 0x01, 0xce],
    [0x00, 0xcf, 0x15], [0x01, 0xcf, 0xcf], [0xcf, 0xcf, 0x15], [0xcf, 0xcf, 0xcf],
    [0x00, 0x00, 0x00], [0x02, 0x00, 0xfd], [0xff, 0x02, 0x01], [0xff, 0x02, 0xfd],
    [0x00, 0xff, 0x1c], [0x02, 0xff, 0xff], [0xff, 0xff, 0x1d], [0xff, 0xff, 0xff],
];
/// Wikipedia palette #2.
pub const PAL_WIKI2: Palette = [
    [0x00, 0x00, 0x00], [0x00, 0x1d, 0xc8], [0xd8, 0x24, 0x0f], [0xd5, 0x30, 0xc9],
    [0x00, 0xc7, 0x21], [0x00, 0xc9, 0xcb], [0xce, 0xca, 0x27], [0xcb, 0xcb, 0xcb],
    [0x00, 0x00, 0x00], [0x00, 0x27, 0xfb], [0xff, 0x30, 0x16], [0xff, 0x3f, 0xfc],
    [0x00, 0xf9, 0x2c], [0x00, 0xfc, 0xfe], [0xff, 0xfd, 0x33], [0xff, 0xff, 0xff],
];
/// Palette used by the Spectaculator emulator.
pub const PAL_SPECTACULATOR: Palette = [
    [0x00, 0x00, 0x00], [0x00, 0x00, 0xce], [0xce, 0x00, 0x00], [0xce, 0x00, 0xce],
    [0x00, 0xcb, 0x00], [0x00, 0xcb, 0xce], [0xce, 0xcb, 0x00], [0xce, 0xcb, 0xce],
    [0x00, 0x00, 0x00], [0x00, 0x00, 0xff], [0xff, 0x00, 0x00], [0xff, 0x00, 0xff],
    [0x00, 0xfb, 0x00], [0x00, 0xfb, 0xff], [0xff, 0xfb, 0x00], [0xff, 0xfb, 0xff],
];
/// ATM-Turbo palette (0xAA / 0xFF - matches Xpeccy's built-in default).
pub const PAL_ATM: Palette = [
    [0x00, 0x00, 0x00], [0x00, 0x00, 0xaa], [0xaa, 0x00, 0x00], [0xaa, 0x00, 0xaa],
    [0x00, 0xaa, 0x00], [0x00, 0xaa, 0xaa], [0xaa, 0xaa, 0x00], [0xaa, 0xaa, 0xaa],
    [0x00, 0x00, 0x00], [0x00, 0x00, 0xff], [0xff, 0x00, 0x00], [0xff, 0x00, 0xff],
    [0x00, 0xff, 0x00], [0x00, 0xff, 0xff], [0xff, 0xff, 0x00], [0xff, 0xff, 0xff],
];
/// ZX Spectrum Next FPGA core HDMI palette (0xB0 / 0xFF).
pub const PAL_NEXT: Palette = [
    [0x00, 0x00, 0x00], [0x00, 0x00, 0xb0], [0xb0, 0x00, 0x00], [0xb0, 0x00, 0xb0],
    [0x00, 0xb0, 0x00], [0x00, 0xb0, 0xb0], [0xb0, 0xb0, 0x00], [0xb0, 0xb0, 0xb0],
    [0x00, 0x00, 0x00], [0x00, 0x00, 0xff], [0xff, 0x00, 0x00], [0xff, 0x00, 0xff],
    [0x00, 0xff, 0x00], [0x00, 0xff, 0xff], [0xff, 0xff, 0x00], [0xff, 0xff, 0xff],
];
/// Art palette by Schafft.
pub const PAL_SCHAFFT: Palette = [
    [0x00, 0x00, 0x00], [0x1c, 0x00, 0x77], [0xa2, 0x23, 0x2a], [0x84, 0x17, 0xa8],
    [0x7b, 0x87, 0x07], [0x2d, 0x91, 0xc3], [0xda, 0xa7, 0x3e], [0xba, 0xba, 0xba],
    [0x00, 0x00, 0x00], [0x21, 0x00, 0xa5], [0xe0, 0x2c, 0x35], [0xb7, 0x1b, 0xe8],
    [0xa7, 0xba, 0x08], [0x42, 0xc2, 0xff], [0xff, 0xd6, 0x6d], [0xfc, 0xfc, 0xfc],
];

/// The default palette used when none is chosen.
pub const DEFAULT_PALETTE: &Palette = &PAL_PULSAR;

/// All bundled palettes as `(name, palette)`, in a stable order.
pub fn named_palettes() -> &'static [(&'static str, &'static Palette)] {
    &[
        ("pulsar", &PAL_PULSAR),
        ("wiki1", &PAL_WIKI1),
        ("wiki2", &PAL_WIKI2),
        ("spectaculator", &PAL_SPECTACULATOR),
        ("atm", &PAL_ATM),
        ("next", &PAL_NEXT),
        ("schafft", &PAL_SCHAFFT),
    ]
}

/// Look up a bundled palette by name (case-insensitive).
pub fn palette_by_name(name: &str) -> Option<&'static Palette> {
    let n = name.trim().to_ascii_lowercase();
    named_palettes().iter().find(|(k, _)| *k == n).map(|(_, p)| *p)
}

/// Which screen format a byte length corresponds to.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ScreenFormat {
    /// 6912 bytes: bitmap + attributes.
    WithAttributes,
    /// 6144 bytes: bitmap only (white on black).
    BitmapOnly,
}

/// Detect the screen format from a byte length alone (the only reliable cue -
/// the format carries no magic). Returns `None` for any other size.
pub fn detect(len: usize) -> Option<ScreenFormat> {
    match len {
        SCREEN_LEN => Some(ScreenFormat::WithAttributes),
        BITMAP_LEN => Some(ScreenFormat::BitmapOnly),
        _ => None,
    }
}

/// Rendering options. Defaults: 2x integer scale, 32-pixel black border, the
/// [`PAL_PULSAR`] palette.
#[derive(Copy, Clone, Debug)]
pub struct RenderOpts {
    /// Integer zoom applied to the whole bordered image (nearest-neighbour).
    pub scale: u32,
    /// Border thickness, in source pixels, around the 256x192 screen (scaled too).
    pub border: u32,
    /// Colour palette (0..8 normal, 8..16 bright).
    pub palette: &'static Palette,
    /// Swap ink/paper for every cell. For a bitmap-only (6144) screen this
    /// turns the default white-on-black into black-on-white; on a full screen it
    /// inverts all the attributes.
    pub invert: bool,
    /// Force the BRIGHT bit of every cell (`None` = use the screen's own bits).
    /// Used for the brightness toggles.
    pub force_bright: Option<bool>,
    /// Border fill colour (RGB), used when `border_dominant` is false.
    pub border_rgb: Rgb,
    /// Colour the border with the most frequent pixel colour of the screen
    /// (overrides `border_rgb`). Recomputed each render.
    pub border_dominant: bool,
    /// Ignore the attribute map and render the bitmap as white ink on black
    /// paper (the "attributes off" view; also how a 6144 screen looks).
    pub mono: bool,
}

impl Default for RenderOpts {
    fn default() -> RenderOpts {
        RenderOpts {
            scale: 2,
            border: 32,
            palette: DEFAULT_PALETTE,
            invert: false,
            force_bright: None,
            border_rgb: [0, 0, 0],
            border_dominant: false,
            mono: false,
        }
    }
}

/// A decoded RGBA image (8 bits/channel, row-major, top-down, opaque).
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, in R, G, B, A order.
    pub pixels: Vec<u8>,
    /// The border colour actually used (handy in `border_dominant` mode).
    pub border: Rgb,
}

/// A parsed ZX screen, ready to [`render`](Screen::render) at any flash phase.
pub struct Screen {
    bitmap: Vec<u8>, // BITMAP_LEN bytes
    attrs: Vec<u8>,  // ATTR_LEN bytes (synthesised for bitmap-only screens)
}

impl Screen {
    /// Parse a raw screen. Accepts 6912 (with attributes) or 6144 (bitmap only,
    /// defaulted to white ink on black paper). Returns `None` for other sizes.
    pub fn parse(bytes: &[u8]) -> Option<Screen> {
        match detect(bytes.len())? {
            ScreenFormat::WithAttributes => Some(Screen {
                bitmap: bytes[..BITMAP_LEN].to_vec(),
                attrs: bytes[BITMAP_LEN..SCREEN_LEN].to_vec(),
            }),
            ScreenFormat::BitmapOnly => Some(Screen {
                bitmap: bytes[..BITMAP_LEN].to_vec(),
                // paper=0 (black), ink=7 (white), no bright/flash -> 0x07.
                attrs: vec![0x07; ATTR_LEN],
            }),
        }
    }

    /// Whether any cell has the FLASH bit set (so a viewer needs a timer).
    pub fn has_flash(&self) -> bool {
        self.attrs.iter().any(|a| a & 0x80 != 0)
    }

    /// Render to RGBA at the given flash phase (`false` = normal, `true` =
    /// swapped ink/paper for flashing cells).
    pub fn render(&self, flash_on: bool, opts: &RenderOpts) -> Rgba {
        // Clamp to sane maxima so an extreme `scale`/`border` from an outside
        // caller cannot overflow the size math (a capacity-overflow panic, or a
        // wrapped `usize` product on 32-bit). All real callers pass small values
        // (the WLX viewer uses scale 1..6, border 22), so this never bites them.
        let bp = (opts.border as usize).min(MAX_BORDER);
        let scale = (opts.scale.max(1) as usize).min(MAX_SCALE);
        let w1 = SCREEN_W + 2 * bp;
        let h1 = SCREEN_H + 2 * bp;

        // Draw the 256x192 screen into the centre, counting how often each
        // palette colour occurs (to pick the dominant border colour).
        let mut base = vec![0u8; w1 * h1 * 4];
        let mut counts = [0u32; 16];
        for y in 0..SCREEN_H {
            for x in 0..SCREEN_W {
                let byte_off = (y >> 6) * 2048 + (y & 7) * 256 + ((y >> 3) & 7) * 32 + (x >> 3);
                let on = (self.bitmap[byte_off] >> (7 - (x & 7))) & 1 == 1;
                // `mono` ignores the attribute map: plain white-on-black.
                let (mut ink, mut paper, bright, flash) = if opts.mono {
                    (7u8, 0u8, opts.force_bright.unwrap_or(true), false)
                } else {
                    let attr = self.attrs[(y >> 3) * (SCREEN_W / 8) + (x >> 3)];
                    let b = opts.force_bright.unwrap_or(attr & 0x40 != 0);
                    (attr & 7, (attr >> 3) & 7, b, attr & 0x80 != 0)
                };
                if opts.invert {
                    std::mem::swap(&mut ink, &mut paper);
                }
                if flash && flash_on {
                    std::mem::swap(&mut ink, &mut paper);
                }
                let bi = if bright { 8 } else { 0 };
                let pi = (if on { ink } else { paper }) as usize + bi;
                counts[pi] += 1;
                let [r, g, b] = opts.palette[pi];
                let o = ((bp + y) * w1 + (bp + x)) * 4;
                base[o] = r;
                base[o + 1] = g;
                base[o + 2] = b;
                base[o + 3] = 255;
            }
        }

        // The border is a single solid colour: the most common screen colour
        // (dominant mode) or the chosen fixed colour.
        let border = if opts.border_dominant {
            let best = (0..16).max_by_key(|&i| counts[i]).unwrap_or(0);
            opts.palette[best]
        } else {
            opts.border_rgb
        };
        // Fill the border margin (everything outside the 256x192 screen).
        for y in 0..h1 {
            for x in 0..w1 {
                if x < bp || x >= bp + SCREEN_W || y < bp || y >= bp + SCREEN_H {
                    let o = (y * w1 + x) * 4;
                    base[o] = border[0];
                    base[o + 1] = border[1];
                    base[o + 2] = border[2];
                    base[o + 3] = 255;
                }
            }
        }

        if scale == 1 {
            return Rgba { width: w1 as u32, height: h1 as u32, pixels: base, border };
        }
        let w = w1 * scale;
        let h = h1 * scale;
        let mut out = vec![0u8; w * h * 4];
        for yy in 0..h {
            let so_row = (yy / scale) * w1;
            for xx in 0..w {
                let so = (so_row + xx / scale) * 4;
                let d = (yy * w + xx) * 4;
                out[d..d + 4].copy_from_slice(&base[so..so + 4]);
            }
        }
        Rgba { width: w as u32, height: h as u32, pixels: out, border }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(scale: u32, border: u32) -> RenderOpts {
        RenderOpts { scale, border, ..RenderOpts::default() }
    }

    #[test]
    fn detect_by_length() {
        assert_eq!(detect(6912), Some(ScreenFormat::WithAttributes));
        assert_eq!(detect(6144), Some(ScreenFormat::BitmapOnly));
        assert_eq!(detect(6913), None);
        assert_eq!(detect(0), None);
    }

    #[test]
    fn top_left_pixel_uses_ink_then_paper() {
        let mut b = vec![0u8; SCREEN_LEN];
        b[0] = 0x80; // pixel (0,0) on, (1,0) off
        b[BITMAP_LEN] = (1 << 3) | 7; // cell(0,0): paper=1 blue, ink=7 white
        let s = Screen::parse(&b).unwrap();
        let img = s.render(false, &opts(1, 0));
        assert_eq!(&img.pixels[0..3], &PAL_PULSAR[7]); // on -> white ink
        assert_eq!(&img.pixels[4..7], &PAL_PULSAR[1]); // off -> blue paper
    }

    #[test]
    fn bitmap_only_is_white_on_black() {
        let mut b = vec![0u8; BITMAP_LEN];
        b[0] = 0x80;
        let s = Screen::parse(&b).unwrap();
        assert!(!s.has_flash());
        let img = s.render(false, &opts(1, 0));
        assert_eq!(&img.pixels[0..3], &PAL_PULSAR[7]); // ink white
        assert_eq!(&img.pixels[4..7], &PAL_PULSAR[0]); // paper black
    }

    #[test]
    fn flash_swaps_ink_and_paper() {
        let mut b = vec![0u8; SCREEN_LEN];
        b[0] = 0x80;
        b[BITMAP_LEN] = 0x80 | (1 << 3) | 7; // flash, paper=1 blue, ink=7 white
        let s = Screen::parse(&b).unwrap();
        assert!(s.has_flash());
        let off = s.render(false, &opts(1, 0));
        let on = s.render(true, &opts(1, 0));
        assert_eq!(&off.pixels[0..3], &PAL_PULSAR[7]); // normal: ink white
        assert_eq!(&on.pixels[0..3], &PAL_PULSAR[1]); // flashed: paper blue
    }

    #[test]
    fn bright_maps_to_upper_palette_half() {
        // A bright red ink cell, pixel on -> palette[2 + 8].
        let mut b = vec![0u8; SCREEN_LEN];
        b[0] = 0x80;
        b[BITMAP_LEN] = 0x40 | 2; // bright, ink=2 red, paper=0
        let s = Screen::parse(&b).unwrap();
        let img = s.render(false, &opts(1, 0));
        assert_eq!(&img.pixels[0..3], &PAL_PULSAR[10]); // bright red
        // bright black paper still black
        assert_eq!(&img.pixels[4..7], &PAL_PULSAR[8]);
        assert_eq!(PAL_PULSAR[8], [0, 0, 0]);
    }

    #[test]
    fn invert_flips_mono_ink_and_paper() {
        let mut b = vec![0u8; BITMAP_LEN]; // 6144: white on black by default
        b[0] = 0x80; // pixel (0,0) on
        let s = Screen::parse(&b).unwrap();
        let o = RenderOpts { invert: true, ..opts(1, 0) };
        let img = s.render(false, &o);
        assert_eq!(&img.pixels[0..3], &PAL_PULSAR[0]); // on -> now black ink
        assert_eq!(&img.pixels[4..7], &PAL_PULSAR[7]); // off -> now white paper
    }

    #[test]
    fn mono_mode_ignores_attributes() {
        // A colour screen rendered with mono=true is plain white-on-black.
        let mut b = vec![0u8; SCREEN_LEN];
        b[0] = 0x80; // pixel (0,0) on
        b[BITMAP_LEN] = (2 << 3) | 4; // cell(0,0): paper=2 red, ink=4 green (ignored in mono)
        let s = Screen::parse(&b).unwrap();
        let o = RenderOpts { mono: true, force_bright: Some(false), ..opts(1, 0) };
        let img = s.render(false, &o);
        assert_eq!(&img.pixels[0..3], &PAL_PULSAR[7]); // on -> white (not green)
        assert_eq!(&img.pixels[4..7], &PAL_PULSAR[0]); // off -> black (not red)
    }

    #[test]
    fn force_bright_overrides_attribute() {
        let mut b = vec![0u8; SCREEN_LEN];
        b[0] = 0x80;
        b[BITMAP_LEN] = 7; // white ink, no bright bit
        let s = Screen::parse(&b).unwrap();
        let on = s.render(false, &RenderOpts { force_bright: Some(true), ..opts(1, 0) });
        assert_eq!(&on.pixels[0..3], &PAL_PULSAR[15]); // forced bright white
        let off = s.render(false, &RenderOpts { force_bright: Some(false), ..opts(1, 0) });
        assert_eq!(&off.pixels[0..3], &PAL_PULSAR[7]); // forced normal white
    }

    #[test]
    fn border_colour_applies() {
        let s = Screen::parse(&vec![0u8; SCREEN_LEN]).unwrap();
        let img = s.render(false, &RenderOpts { border_rgb: PAL_PULSAR[7], ..opts(1, 8) });
        assert_eq!(img.border, PAL_PULSAR[7]);
        assert_eq!(&img.pixels[0..3], &PAL_PULSAR[7]); // border corner is the set colour
    }

    #[test]
    fn dominant_border_picks_most_common_colour() {
        // Whole bitmap off -> every pixel is paper; set paper = red (2).
        let mut b = vec![0u8; SCREEN_LEN];
        for c in b[BITMAP_LEN..SCREEN_LEN].iter_mut() {
            *c = 2 << 3; // paper=2 red, ink=0
        }
        let s = Screen::parse(&b).unwrap();
        let img = s.render(false, &RenderOpts { border_dominant: true, ..opts(1, 4) });
        assert_eq!(img.border, PAL_PULSAR[2]); // dominant colour is red
        assert_eq!(&img.pixels[0..3], &PAL_PULSAR[2]); // border corner painted red
    }

    #[test]
    fn thirds_interleave_second_band() {
        // y=64 is the first line of the middle third -> byte offset 2048.
        let mut b = vec![0u8; SCREEN_LEN];
        b[2048] = 0x80; // pixel (0,64)
        b[BITMAP_LEN + (64 >> 3) * 32] = 0x07; // that cell: white on black
        let s = Screen::parse(&b).unwrap();
        let img = s.render(false, &opts(1, 0));
        let o = (64 * SCREEN_W) * 4;
        assert_eq!(&img.pixels[o..o + 3], &PAL_PULSAR[7]);
    }

    #[test]
    fn border_and_scale_dimensions() {
        let s = Screen::parse(&vec![0u8; SCREEN_LEN]).unwrap();
        let img = s.render(false, &opts(2, 32));
        assert_eq!(img.width, (256 + 64) * 2);
        assert_eq!(img.height, (192 + 64) * 2);
        assert_eq!(img.pixels.len(), (img.width * img.height * 4) as usize);
        assert_eq!(&img.pixels[0..4], &[0, 0, 0, 255]); // border corner: opaque black
    }

    #[test]
    fn default_opts_and_palettes() {
        let o = RenderOpts::default();
        assert_eq!((o.scale, o.border), (2, 32));
        assert_eq!(named_palettes().len(), 7);
        assert_eq!(palette_by_name("PULSAR").unwrap(), &PAL_PULSAR); // by value
        assert_eq!(palette_by_name(" atm ").unwrap(), &PAL_ATM);
        assert!(palette_by_name("nope").is_none());
    }
}
