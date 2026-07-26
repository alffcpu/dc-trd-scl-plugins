//! Cross-platform heart of the ZX-screen lister plugin.
//!
//! Everything here is platform-independent: the detect string, the view-model
//! (`State`: which palette/zoom/border/invert is active and the last rendered
//! RGBA frame), the key/click semantics as an [`Action`] the native shells map
//! their events onto, and the shared settings file. The per-OS window - Windows
//! GDI in [`crate::win`], macOS Cocoa in [`crate::mac`] - is only a thin shell
//! that creates a native surface, blits `State::render`, and forwards native
//! events through [`apply_action`].
//!
//! The actual pixel work (decode, palette, scale, border, FLASH) lives one layer
//! down in [`zxdisk_core::screen`], shared with the CLI's screen examples.

use std::os::raw::{c_char, c_int};

/// Detect by size only (a ZX screen has no signature). Parenthesised, no spaces,
/// single `=` - the form Double Commander's own plugins use in `<DetectString>`.
pub const DETECT: &[u8] = b"(SIZE=6912)|(SIZE=6144)";

/// Tell Double Commander which files to hand us: by size only. Identical on every
/// platform, so it is defined once here (the load/close exports, which traffic in
/// native window handles, live in the per-OS modules).
///
/// # Safety
/// `detect_string` must point to a writable buffer of at least `maxlen` bytes.
#[no_mangle]
pub unsafe extern "system" fn ListGetDetectString(detect_string: *mut c_char, maxlen: c_int) {
    if detect_string.is_null() || maxlen <= 0 {
        return;
    }
    // Always leave room for (and write) the NUL terminator, even for a 1-byte buf.
    let cap = (maxlen as usize) - 1;
    let n = DETECT.len().min(cap);
    core::ptr::copy_nonoverlapping(DETECT.as_ptr() as *const c_char, detect_string, n);
    *detect_string.add(n) = 0;
}

// The view-model and settings are only needed where there is a native window to
// drive (Windows / macOS / Linux-Qt). Gating them keeps any other build - which
// would only export the detect string - free of dead-code warnings.
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
pub mod model {
    use std::path::{Path, PathBuf};

    use zxdisk_core::screen::{self, RenderOpts, Rgba, Screen};

    /// Default integer zoom (2x), used when the settings file has no saved value.
    pub const DEFAULT_SCALE: u32 = 2;
    /// Border thickness in source pixels around the 256x192 screen (scaled too).
    pub const BORDER: u32 = 22; // ~30% smaller than 32, emulator-ish
    /// Authentic FLASH toggle period, in milliseconds.
    pub const FLASH_MS: u32 = 320;

    // ---- shared settings: remember zoom + border in the plugin's conf file ----

    /// The writable shared settings file, same one the WCX plugin and CLI read.
    /// macOS/Linux: `~/.config/zxdisk.conf`. Windows: `%APPDATA%\zxdisk\zxdisk.conf`
    /// (falling back to `%USERPROFILE%\.config`), where the installer puts it.
    fn config_path() -> Option<PathBuf> {
        #[cfg(windows)]
        {
            if let Some(a) = std::env::var_os("APPDATA") {
                return Some(Path::new(&a).join("zxdisk").join("zxdisk.conf"));
            }
            std::env::var_os("USERPROFILE")
                .map(|h| Path::new(&h).join(".config").join("zxdisk.conf"))
        }
        #[cfg(not(windows))]
        {
            std::env::var_os("HOME").map(|h| Path::new(&h).join(".config").join("zxdisk.conf"))
        }
    }

    /// Read one `key = value` from the settings file (comments/sections ignored).
    fn read_setting(key: &str) -> Option<String> {
        let text = std::fs::read_to_string(config_path()?).ok()?;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with(['#', ';', '[']) {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                if k.trim().eq_ignore_ascii_case(key) {
                    return Some(v.trim().to_string());
                }
            }
        }
        None
    }

    /// Persist `key=value`, updating it in place (or appending), leaving the rest
    /// of the settings/comments untouched.
    fn write_setting(key: &str, value: &str) {
        let path = match config_path() {
            Some(p) => p,
            None => return,
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut lines: Vec<String> = std::fs::read_to_string(&path)
            .map(|t| t.lines().map(str::to_string).collect())
            .unwrap_or_default();
        let mut found = false;
        for line in lines.iter_mut() {
            let t = line.trim_start();
            if t.starts_with(['#', ';', '[']) {
                continue;
            }
            if let Some((k, _)) = t.split_once('=') {
                if k.trim().eq_ignore_ascii_case(key) {
                    *line = format!("{key}={value}");
                    found = true;
                    break;
                }
            }
        }
        if !found {
            lines.push(format!("{key}={value}"));
        }
        // Write atomically: a crash/power-cut mid-write must not truncate the
        // shared conf (dropping the WCX plugin's settings and all the comments).
        // Native line endings: CRLF on Windows, LF elsewhere - to match how the
        // installer and the WCX plugin write the same file.
        let sep = if cfg!(windows) { "\r\n" } else { "\n" };
        let body = lines.join(sep) + sep;
        let tmp = path.with_extension("conf.tmp");
        if std::fs::write(&tmp, body).is_ok() {
            let _ = std::fs::rename(&tmp, &path); // replaces the target
        }
    }

    /// Saved zoom (1..6), if valid.
    fn read_scale() -> Option<u32> {
        read_setting("screen_scale")?.parse::<u32>().ok().filter(|n| (1..=6).contains(n))
    }
    fn write_scale(n: u32) {
        write_setting("screen_scale", &n.to_string());
    }

    /// How the border is coloured.
    #[derive(Copy, Clone)]
    pub enum BorderMode {
        /// Most frequent pixel colour of the screen (the default).
        Dominant,
        /// A fixed ZX colour 0..7, no bright.
        Fixed(u8),
    }

    /// Saved border mode. Missing/`auto`/invalid -> Dominant (the default).
    fn read_border() -> BorderMode {
        match read_setting("screen_border_color").as_deref().map(str::trim) {
            Some(v) => match v.parse::<u8>() {
                Ok(n) if n <= 7 => BorderMode::Fixed(n),
                _ => BorderMode::Dominant,
            },
            None => BorderMode::Dominant,
        }
    }
    fn write_border(mode: BorderMode) {
        let v = match mode {
            BorderMode::Fixed(c) => c.to_string(),
            BorderMode::Dominant => "auto".to_string(),
        };
        write_setting("screen_border_color", &v);
    }

    /// Run a closure, swallowing any panic and returning `default` instead. Used
    /// on the FFI boundary so a panic never unwinds into the C host.
    pub fn guard<T>(default: T, f: impl FnOnce() -> T) -> T {
        use std::panic::{catch_unwind, AssertUnwindSafe};
        match catch_unwind(AssertUnwindSafe(f)) {
            Ok(v) => v,
            Err(_) => default,
        }
    }

    /// Convert an RGBA frame to the BGRA a 32-bit top-down Windows DIB expects.
    #[cfg(windows)]
    fn rgba_to_bgra(rgba: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; rgba.len()];
        for (o, i) in out.chunks_exact_mut(4).zip(rgba.chunks_exact(4)) {
            o[0] = i[2]; // B
            o[1] = i[1]; // G
            o[2] = i[0]; // R
            o[3] = i[3]; // A
        }
        out
    }

    // ---------------------------------------------------------- view-model ----

    /// One user gesture, mapped from a native key/mouse event by the OS shell and
    /// applied through [`apply_action`]. Keeps the view semantics in one place so
    /// the Windows and macOS shells only translate their own event codes.
    #[derive(Copy, Clone)]
    pub enum Action {
        /// Choose palette by index (0..7: pulsar, wiki1, wiki2, spectaculator,
        /// atm, next, schafft).
        Palette(usize),
        /// Cycle to the next palette (a click).
        NextPalette,
        /// Integer zoom 1x..6x.
        Zoom(u32),
        /// Fixed border colour, ZX colour 0..7.
        BorderFixed(u8),
        /// Border follows the most frequent screen colour (the default).
        BorderDominant,
        /// Swap ink/paper (mono: invert; colour: invert all attributes).
        ToggleInvert,
        /// Enter: mono -> brightness on/off; colour -> attributes off/all-bright/original.
        Cycle,
    }

    /// The whole viewer state for one open file: the parsed screen, the active
    /// display options, and the last rendered RGBA frame the shell blits.
    pub struct State {
        screen: Screen,
        /// A 6144 bitmap-only screen (white-on-black, no attribute map).
        pub is_mono: bool,
        /// Whether any cell flashes (so the shell arms a timer).
        pub has_flash: bool,
        phase: bool,             // current FLASH phase
        pal: usize,              // palette index 0..6
        scale: u32,              // 1..6
        invert: bool,            // Space / right-click
        bright: bool,            // 6144 brightness (Enter); default on
        mode: u8,                // 6912 attribute mode (Enter): 0 mono, 1 all-bright, 2 original
        border_mode: BorderMode, // fixed 0..7 / dominant (default)
        /// Last rendered frame (RGBA, top row first). The shell converts/blits it.
        pub render: Rgba,
        /// Windows blits a 32-bit BGRA DIB; cache the converted frame so a repaint
        /// (expose / focus / FLASH tick) does not reconvert the whole image each
        /// time. Rebuilt in `rerender`, alongside `render`.
        #[cfg(windows)]
        pub bgra: Vec<u8>,
    }

    impl State {
        /// Build the state for a parsed screen and render the first frame.
        pub fn new(screen: Screen, is_mono: bool) -> State {
            let has_flash = screen.has_flash();
            let mut s = State {
                screen,
                is_mono,
                has_flash,
                phase: false,
                pal: 0,
                scale: read_scale().unwrap_or(DEFAULT_SCALE), // last chosen zoom, else 2x
                invert: false,
                bright: true,
                mode: 2,
                border_mode: read_border(),
                render: Rgba { width: 0, height: 0, pixels: Vec::new(), border: [0, 0, 0] },
                #[cfg(windows)]
                bgra: Vec::new(),
            };
            s.rerender();
            s
        }

        fn opts(&self) -> RenderOpts {
            let pal = screen::named_palettes()[self.pal].1;
            let mut o = RenderOpts {
                scale: self.scale.clamp(1, 6),
                border: BORDER,
                palette: pal,
                invert: self.invert,
                ..RenderOpts::default()
            };
            if self.is_mono {
                // brightness toggles the image; the border follows the background
                // (paper) colour and ignores brightness.
                o.force_bright = Some(self.bright);
                o.border_rgb = pal[if self.invert { 7 } else { 0 }];
            } else {
                match self.mode {
                    0 => {
                        o.mono = true;
                        o.force_bright = Some(true);
                    }
                    1 => o.force_bright = Some(true),
                    _ => {}
                }
            }
            // Border: dominant screen colour (default) or a fixed colour.
            match self.border_mode {
                BorderMode::Dominant => o.border_dominant = true,
                BorderMode::Fixed(c) => o.border_rgb = pal[(c & 7) as usize],
            }
            o
        }

        fn rerender(&mut self) {
            let o = self.opts();
            self.render = self.screen.render(self.phase, &o);
            #[cfg(windows)]
            {
                self.bgra = rgba_to_bgra(&self.render.pixels);
            }
        }

        /// Advance the FLASH phase and re-render (called from the shell's timer).
        pub fn tick(&mut self) {
            self.phase = !self.phase;
            self.rerender();
        }
    }

    /// Apply a user gesture, persisting zoom/border to the settings file, and
    /// re-render. Returns whether anything changed (so the shell repaints).
    pub fn apply_action(st: &mut State, action: Action) -> bool {
        match action {
            Action::Palette(p) if p < screen::named_palettes().len() => st.pal = p,
            Action::NextPalette => st.pal = (st.pal + 1) % screen::named_palettes().len(),
            Action::Zoom(z) => {
                st.scale = z.clamp(1, 6);
                write_scale(st.scale);
            }
            Action::BorderFixed(c) => {
                st.border_mode = BorderMode::Fixed(c & 7);
                write_border(st.border_mode);
            }
            Action::BorderDominant => {
                st.border_mode = BorderMode::Dominant;
                write_border(st.border_mode);
            }
            Action::ToggleInvert => st.invert = !st.invert,
            Action::Cycle => {
                if st.is_mono {
                    st.bright = !st.bright;
                } else {
                    st.mode = (st.mode + 1) % 3;
                }
            }
            // Palette index out of range -> nothing to do.
            Action::Palette(_) => return false,
        }
        st.rerender();
        true
    }

    /// Map a number-key press (with its modifiers) to an [`Action`], shared by
    /// both OS shells so the modifier semantics stay identical (they drifted
    /// before, which let Shift+7 wrongly select palette 7 on both). `digit` is the
    /// main-row digit 0..9, or `None` for a non-digit key. Alt takes precedence,
    /// then Shift; a bare digit is a palette key.
    ///   Alt+0..7 fixed border   Alt+8 dominant
    ///   Shift+1..6 zoom         (Shift with any other digit is NOT a shortcut)
    ///   1..7 palette            (only with neither Shift nor Alt)
    pub fn digit_action(digit: Option<u8>, shift: bool, alt: bool) -> Option<Action> {
        let d = digit?;
        if alt {
            match d {
                8 => Some(Action::BorderDominant),
                0..=7 => Some(Action::BorderFixed(d)),
                _ => None,
            }
        } else if shift {
            // Zoom is 1..6 only; Shift+7 (etc.) must not fall through to palette.
            (1..=6).contains(&d).then_some(Action::Zoom(d as u32))
        } else {
            // `then` (lazy), NOT `then_some`: the plain `0` key reaches here (it is
            // simply not a palette), and `d - 1` would underflow u8 for d == 0.
            (1..=7).contains(&d).then(|| Action::Palette((d - 1) as usize))
        }
    }

    /// Read a file and, if it is a 6912/6144 ZX screen, build its viewer state.
    /// Returns `None` (so the shell hands back a null window) for anything else.
    pub fn build_state(path: &Path) -> Option<Box<State>> {
        let bytes = std::fs::read(path).ok()?;
        let fmt = screen::detect(bytes.len())?;
        let scr = Screen::parse(&bytes)?;
        let is_mono = fmt == screen::ScreenFormat::BitmapOnly;
        Some(Box::new(State::new(scr, is_mono)))
    }

    #[cfg(test)]
    mod tests {
        use super::{digit_action, Action};

        #[test]
        fn plain_digits_select_palettes() {
            assert!(matches!(digit_action(Some(1), false, false), Some(Action::Palette(0))));
            assert!(matches!(digit_action(Some(7), false, false), Some(Action::Palette(6))));
            // 0/8/9 are not palette keys
            assert!(digit_action(Some(0), false, false).is_none());
            assert!(digit_action(Some(8), false, false).is_none());
            assert!(digit_action(Some(9), false, false).is_none());
            assert!(digit_action(None, false, false).is_none());
        }

        #[test]
        fn shift_is_zoom_1_to_6_only() {
            assert!(matches!(digit_action(Some(1), true, false), Some(Action::Zoom(1))));
            assert!(matches!(digit_action(Some(6), true, false), Some(Action::Zoom(6))));
            // The reported bug: Shift+7 must NOT fall through to palette 7 (nor anything).
            assert!(digit_action(Some(7), true, false).is_none());
            assert!(digit_action(Some(0), true, false).is_none());
            assert!(digit_action(Some(8), true, false).is_none());
        }

        #[test]
        fn alt_is_border_and_wins_over_shift() {
            assert!(matches!(digit_action(Some(0), false, true), Some(Action::BorderFixed(0))));
            assert!(matches!(digit_action(Some(7), false, true), Some(Action::BorderFixed(7))));
            assert!(matches!(digit_action(Some(8), false, true), Some(Action::BorderDominant)));
            // Alt takes precedence when both are held.
            assert!(matches!(digit_action(Some(3), true, true), Some(Action::BorderFixed(3))));
        }

        // Exhaustive: for every digit and every Shift/Alt combination, the action
        // must stay inside the category that modifier selects - it must never
        // "leak" across (the Shift+7 -> palette bug was exactly such a leak). This
        // proves there are no hidden cross-combination overlaps.
        #[test]
        fn no_cross_category_leaks() {
            for d in 0u8..=9 {
                // plain digit: palette or nothing - never zoom/border
                assert!(
                    matches!(digit_action(Some(d), false, false), None | Some(Action::Palette(_))),
                    "plain {d} leaked"
                );
                // Shift+digit: zoom or nothing - never palette/border
                assert!(
                    matches!(digit_action(Some(d), true, false), None | Some(Action::Zoom(_))),
                    "shift {d} leaked"
                );
                // Alt+digit: border or nothing - never palette/zoom
                assert!(
                    matches!(
                        digit_action(Some(d), false, true),
                        None | Some(Action::BorderFixed(_)) | Some(Action::BorderDominant)
                    ),
                    "alt {d} leaked"
                );
                // Alt+Shift+digit: Alt wins (border) - never zoom/palette
                assert!(
                    matches!(
                        digit_action(Some(d), true, true),
                        None | Some(Action::BorderFixed(_)) | Some(Action::BorderDominant)
                    ),
                    "alt+shift {d} leaked"
                );
                // Palette/border indices never go out of range.
                if let Some(Action::Palette(p)) = digit_action(Some(d), false, false) {
                    assert!(p < 7, "palette index {p} out of range");
                }
                if let Some(Action::BorderFixed(c)) = digit_action(Some(d), false, true) {
                    assert!(c <= 7, "border colour {c} out of range");
                }
            }
        }
    }
}
