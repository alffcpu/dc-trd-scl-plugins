//! Tests for the cross-platform half of the lister plugin: the key and click
//! semantics, the shared settings file, the panic guard and the view model.
//!
//! In a file of their own so the coverage report can drop them; see the note
//! at the `mod tests` declaration in viewer.rs.
use super::{apply_action, digit_action, Action, BorderMode, Screen, State};

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

// ---- the settings file -------------------------------------------
//
// Zoom and border survive closing the viewer because they are written
// into the same zxdisk.conf the WCX plugin and the CLI read. Writing it
// is a read-modify-write of a file somebody else also owns, so the
// property that matters is not "the value came back" - it is that
// nothing else in the file moved.

/// Point config_path() at a directory of our own for the duration.
///
/// HOME belongs to the process, not to a test, so the ones that move it
/// hold a lock while they do. Rust runs tests in parallel threads by
/// default, and without this they read each other's settings file - which
/// they did, and it looked like the settings code was broken.
static CONF_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Conf {
    dir: std::path::PathBuf,
    home: Option<std::ffi::OsString>,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Conf {
    fn new(tag: &str) -> Conf {
        // A test that fails while holding the lock poisons it; the next
        // one wants the environment back regardless.
        let _guard = CONF_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir()
            .join(format!("zxdisk-wlx-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".config")).unwrap();
        let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let home = std::env::var_os(key);
        std::env::set_var(key, &dir);
        Conf { dir, home, _guard }
    }
    fn path(&self) -> std::path::PathBuf {
        self.dir.join(".config").join("zxdisk.conf")
    }
    fn text(&self) -> String {
        std::fs::read_to_string(self.path()).unwrap_or_default()
    }
}

impl Drop for Conf {
    fn drop(&mut self) {
        let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        match &self.home {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn a_setting_written_is_a_setting_read_back() {
    let conf = Conf::new("roundtrip");
    super::write_setting("screen_scale", "4");
    assert_eq!(super::read_setting("screen_scale").as_deref(), Some("4"));
    assert_eq!(super::read_scale(), Some(4));

    // Rewriting replaces the line rather than adding a second one, which
    // would leave the file saying two different things.
    super::write_setting("screen_scale", "2");
    assert_eq!(super::read_scale(), Some(2));
    assert_eq!(conf.text().matches("screen_scale").count(), 1, "{}", conf.text());
}

#[test]
fn writing_one_setting_leaves_the_rest_of_the_file_alone() {
    let conf = Conf::new("preserve");
    std::fs::write(
        conf.path(),
        "# the shared settings file\n\
         [zxdisk]\n\
         ext_mode=dot\n\
         something_this_build_never_heard_of=7\n",
    )
    .unwrap();

    super::write_setting("screen_scale", "3");
    let after = conf.text();
    assert!(after.contains("# the shared settings file"), "{after}");
    assert!(after.contains("[zxdisk]"), "{after}");
    assert!(after.contains("ext_mode=dot"), "{after}");
    assert!(after.contains("something_this_build_never_heard_of=7"), "{after}");
    assert!(after.contains("screen_scale=3"), "{after}");
}

#[test]
fn a_setting_outside_its_range_is_no_setting_at_all() {
    let _conf = Conf::new("range");
    // Zoom is 1..6. Anything else is somebody's hand-edit and the
    // viewer falls back to its default rather than to a 400x window.
    for bad in ["0", "7", "-1", "999", "two", ""] {
        super::write_setting("screen_scale", bad);
        assert_eq!(super::read_scale(), None, "screen_scale={bad}");
    }
    super::write_setting("screen_scale", "6");
    assert_eq!(super::read_scale(), Some(6));
}

#[test]
fn the_border_setting_round_trips_through_both_of_its_shapes() {
    let _conf = Conf::new("border");
    super::write_border(BorderMode::Dominant);
    assert!(matches!(super::read_border(), BorderMode::Dominant));
    for c in 0u8..8 {
        super::write_border(BorderMode::Fixed(c));
        match super::read_border() {
            BorderMode::Fixed(got) => assert_eq!(got, c),
            other => panic!("colour {c} came back as {other:?}"),
        }
    }
}

// ---- the panic guard ---------------------------------------------
//
// It exists for one reason: this code is called from Double Commander
// across a C boundary, and a panic that unwinds into a C host aborts the
// whole process. Losing the file manager because a screen was malformed
// is not an acceptable failure.

#[test]
fn guard_returns_the_default_instead_of_unwinding() {
    assert_eq!(super::guard(7, || 1), 1);
    assert_eq!(super::guard(7, || panic!("as a malformed screen would")), 7);
    // And it does not poison anything: the next call still works.
    assert_eq!(super::guard(7, || 2), 2);
}

// ---- applying a gesture ------------------------------------------

fn a_screen() -> Screen {
    // A 6912-byte screen: every attribute cell a different colour, so a
    // palette or invert change has something to change.
    let mut bytes = vec![0u8; 6912];
    for (i, b) in bytes[6144..].iter_mut().enumerate() {
        *b = (i % 128) as u8;
    }
    for (i, b) in bytes[..6144].iter_mut().enumerate() {
        *b = (i % 251) as u8;
    }
    Screen::parse(&bytes).expect("6912 bytes is a screen")
}

#[test]
fn every_gesture_changes_what_it_says_it_changes() {
    let _conf = Conf::new("actions");
    let mut st = State::new(a_screen(), false);

    let pal0 = st.pal;
    assert!(apply_action(&mut st, Action::NextPalette));
    assert_ne!(st.pal, pal0);

    assert!(apply_action(&mut st, Action::Palette(0)));
    assert_eq!(st.pal, 0);

    assert!(apply_action(&mut st, Action::Zoom(5)));
    assert_eq!(st.scale, 5);
    // Out of range is clamped rather than refused: the shells pass a
    // window size through here.
    assert!(apply_action(&mut st, Action::Zoom(99)));
    assert_eq!(st.scale, 6);
    assert!(apply_action(&mut st, Action::Zoom(0)));
    assert_eq!(st.scale, 1);

    assert!(apply_action(&mut st, Action::BorderFixed(3)));
    assert!(matches!(st.border_mode, BorderMode::Fixed(3)));
    // Only the low three bits are a ZX colour.
    assert!(apply_action(&mut st, Action::BorderFixed(0xFF)));
    assert!(matches!(st.border_mode, BorderMode::Fixed(7)));
    assert!(apply_action(&mut st, Action::BorderDominant));
    assert!(matches!(st.border_mode, BorderMode::Dominant));

    let inv = st.invert;
    assert!(apply_action(&mut st, Action::ToggleInvert));
    assert_ne!(st.invert, inv);
}

#[test]
fn a_palette_that_does_not_exist_changes_nothing() {
    let _conf = Conf::new("badpal");
    let mut st = State::new(a_screen(), false);
    let before = st.pal;
    // False means "nothing changed", which is how the shell knows not to
    // repaint.
    assert!(!apply_action(&mut st, Action::Palette(9999)));
    assert_eq!(st.pal, before);
}

#[test]
fn cycle_means_different_things_on_a_mono_screen() {
    let _conf = Conf::new("cycle");

    // Colour: three attribute modes, and it comes back round.
    let mut colour = State::new(a_screen(), false);
    let m0 = colour.mode;
    apply_action(&mut colour, Action::Cycle);
    assert_ne!(colour.mode, m0);
    apply_action(&mut colour, Action::Cycle);
    apply_action(&mut colour, Action::Cycle);
    assert_eq!(colour.mode, m0, "three cycles return to the start");

    // Mono: there are no attributes, so it toggles brightness instead
    // and leaves the attribute mode exactly where it was.
    let mut mono = State::new(a_screen(), true);
    let b0 = mono.bright;
    let mode0 = mono.mode;
    apply_action(&mut mono, Action::Cycle);
    assert_ne!(mono.bright, b0);
    assert_eq!(mono.mode, mode0, "a mono screen has no attribute mode to cycle");
    apply_action(&mut mono, Action::Cycle);
    assert_eq!(mono.bright, b0, "and it toggles back");
}

#[test]
fn zoom_and_border_survive_the_viewer_being_closed() {
    // The gesture writes through to the settings file, which is what
    // makes the choice stick for the next file opened.
    let conf = Conf::new("persist");
    let mut st = State::new(a_screen(), false);
    apply_action(&mut st, Action::Zoom(4));
    apply_action(&mut st, Action::BorderFixed(2));

    assert_eq!(super::read_scale(), Some(4));
    assert!(matches!(super::read_border(), BorderMode::Fixed(2)));
    assert!(conf.text().contains("screen_scale=4"), "{}", conf.text());
}
