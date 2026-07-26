//! Double Commander WLX (lister) plugin for viewing ZX Spectrum screen dumps.
//!
//! Files are recognised purely by size - 6912 (bitmap + attributes) or 6144
//! (bitmap only) - via the detect string `(SIZE=6912)|(SIZE=6144)`, so it works
//! for a file on disk or one opened from inside a `.trd`/`.scl` (Double Commander
//! extracts it to a temp file and lists that). The heavy lifting - decoding,
//! palette, integer scaling, the border and the FLASH frames - is all in the
//! cross-platform [`zxdisk_core::screen`].
//!
//! The crate is split into a platform-independent core and a thin native window
//! per OS:
//!   * [`viewer`]   - the detect string, the view-model, the key/click
//!     [`viewer::model::Action`] semantics, and the shared settings file;
//!   * [`win`]      - the Windows window (GDI blit), built only on Windows;
//!   * [`mac`]      - the macOS Cocoa window (NSView), built only on macOS;
//!   * [`linux`]    - the Linux Qt widget (for DC's qt5/qt6 builds, via the
//!     QtXPas C API already loaded in DC's process), built only on Linux.
//!
//! Each shell exports the handle-trafficking entry points (`ListLoad` /
//! `ListLoadW` / `ListCloseWindow`) in terms of that OS's native window type;
//! `ListGetDetectString` is identical everywhere and lives in [`viewer`].
//!
//! Hotkeys, when the plugin window has keyboard focus (the F3 Lister):
//!   * `1`..`7` - palette (pulsar, wiki1, wiki2, spectaculator, atm, next, schafft);
//!   * `Shift`+`1`..`6` - zoom 1x..6x;   left-click - next palette;
//!   * `Alt`+`0`..`7` - fixed border colour, `Alt`+`8` - dominant (the default);
//!   * `Space` / right-click - invert;   `Enter` - brightness / attribute mode.

mod viewer;

#[cfg(windows)]
mod win;

#[cfg(target_os = "macos")]
mod mac;

#[cfg(target_os = "linux")]
mod linux;
