//! `zxdisk-core` - a pure, dependency-free library for reading and writing
//! ZX Spectrum TR-DOS (`.trd`) and Sinclair (`.scl`) disk images.
//!
//! It never touches the filesystem: everything works on byte buffers, which
//! keeps it trivially portable and unit-testable. The Double Commander plugin
//! (`zxdisk-wcx`) is a thin C-ABI shell over this crate.
//!
//! Capabilities: list live and deleted files, extract data, add files, delete
//! files (TR-DOS-style recoverable erase for TRD), and round-trip both formats.

pub mod entry;
pub mod error;
pub mod hobeta;
pub mod image;
pub mod scl;
pub mod screen;
pub mod trd;

pub use entry::{default_ext_mode, set_default_ext_mode, ExtMode, TrFile};
pub use error::{Error, Result};
pub use image::{Format, Image};
pub use scl::SclArchive;
pub use screen::{RenderOpts, Screen, ScreenFormat};
pub use trd::{DiskType, TrdImage};
