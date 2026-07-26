//! Error type for the disk-image library. The library never touches the
//! filesystem itself - it works on byte buffers - so there is no I/O variant
//! here; file I/O and its errors live in the plugin layer.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The bytes are not a recognisable TRD or SCL image.
    UnknownFormat,
    /// The image is recognised but structurally broken.
    BadArchive(String),
    /// Catalog is full (128 files for TRD, 255 for SCL).
    TooManyFiles,
    /// Not enough free sectors on the disk.
    DiskFull,
    /// File exceeds the 255-sector (65280 byte) TR-DOS limit.
    FileTooBig,
    /// Named file was not found in the catalog.
    NotFound,
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnknownFormat => write!(f, "not a recognisable TRD or SCL image"),
            Error::BadArchive(m) => write!(f, "broken image: {m}"),
            Error::TooManyFiles => write!(f, "catalog is full"),
            Error::DiskFull => write!(f, "not enough free space on disk"),
            Error::FileTooBig => write!(f, "file exceeds the 255-sector TR-DOS limit"),
            Error::NotFound => write!(f, "file not found in catalog"),
        }
    }
}

impl std::error::Error for Error {}
