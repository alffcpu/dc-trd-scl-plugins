//! Tests for the ZX screen decoder and renderer.
//!
//! In a file of their own so scripts/coverage.sh can drop them: they were
//! inline, which put 118 lines of test code in the denominator of a number
//! whose script claimed test code was excluded.
use super::*;

fn opts(scale: u32, border: u32) -> RenderOpts {
    RenderOpts {
        scale,
        border,
        ..RenderOpts::default()
    }
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
    let o = RenderOpts {
        invert: true,
        ..opts(1, 0)
    };
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
    let o = RenderOpts {
        mono: true,
        force_bright: Some(false),
        ..opts(1, 0)
    };
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
    let on = s.render(
        false,
        &RenderOpts {
            force_bright: Some(true),
            ..opts(1, 0)
        },
    );
    assert_eq!(&on.pixels[0..3], &PAL_PULSAR[15]); // forced bright white
    let off = s.render(
        false,
        &RenderOpts {
            force_bright: Some(false),
            ..opts(1, 0)
        },
    );
    assert_eq!(&off.pixels[0..3], &PAL_PULSAR[7]); // forced normal white
}

#[test]
fn border_colour_applies() {
    let s = Screen::parse(&vec![0u8; SCREEN_LEN]).unwrap();
    let img = s.render(
        false,
        &RenderOpts {
            border_rgb: PAL_PULSAR[7],
            ..opts(1, 8)
        },
    );
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
    let img = s.render(
        false,
        &RenderOpts {
            border_dominant: true,
            ..opts(1, 4)
        },
    );
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
