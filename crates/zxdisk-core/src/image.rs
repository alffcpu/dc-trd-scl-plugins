//! A format-agnostic wrapper over [`TrdImage`] and [`SclArchive`] so the plugin
//! layer can treat both the same way.

use crate::entry::TrFile;
use crate::error::{Error, Result};
use crate::scl::SclArchive;
use crate::trd::{DiskType, TrdImage, INFO_SECTOR, TRDOS_ID};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Format {
    Trd,
    Scl,
}

pub enum Image {
    Trd(TrdImage),
    Scl(SclArchive),
}

impl Image {
    /// Detect the format from the leading bytes alone (content sniffing).
    pub fn detect(bytes: &[u8]) -> Option<Format> {
        if bytes.len() >= 8 && &bytes[0..8] == b"SINCLAIR" {
            return Some(Format::Scl);
        }
        if bytes.len() >= INFO_SECTOR + 256
            && bytes[INFO_SECTOR + 0xE7] == TRDOS_ID
            && DiskType::from_byte(bytes[INFO_SECTOR + 0xE3]).is_some()
        {
            return Some(Format::Trd);
        }
        None
    }

    /// Parse an image, preferring content detection and falling back to the file
    /// extension when the content is ambiguous.
    pub fn from_bytes(bytes: &[u8], ext_hint: Option<&str>) -> Result<Image> {
        match Image::detect(bytes) {
            Some(Format::Scl) => Ok(Image::Scl(SclArchive::from_bytes(bytes)?)),
            Some(Format::Trd) => Ok(Image::Trd(TrdImage::from_bytes(bytes)?)),
            None => match ext_hint.map(|e| e.to_ascii_lowercase()).as_deref() {
                Some("scl") => Ok(Image::Scl(SclArchive::from_bytes(bytes)?)),
                Some("trd") => Ok(Image::Trd(TrdImage::from_bytes(bytes)?)),
                _ => Err(Error::UnknownFormat),
            },
        }
    }

    /// A blank image appropriate for a target extension (used when packing into a
    /// path that does not exist yet). Unknown extensions default to a 640 KB TRD.
    pub fn blank_for_ext(ext: &str) -> Image {
        Image::blank_for_ext_with(ext, DiskType::Ds80)
    }

    /// Like [`Image::blank_for_ext`] but with a chosen TRD geometry (ignored for
    /// SCL, which has no geometry).
    pub fn blank_for_ext_with(ext: &str, trd_geometry: DiskType) -> Image {
        if ext.eq_ignore_ascii_case("scl") {
            Image::Scl(SclArchive::blank())
        } else {
            Image::Trd(TrdImage::blank(trd_geometry, ""))
        }
    }

    pub fn format(&self) -> Format {
        match self {
            Image::Trd(_) => Format::Trd,
            Image::Scl(_) => Format::Scl,
        }
    }

    pub fn entries(&self) -> Vec<TrFile> {
        match self {
            Image::Trd(t) => t.entries(),
            Image::Scl(s) => s.entries(),
        }
    }

    pub fn add_file(&mut self, f: &TrFile) -> Result<()> {
        match self {
            Image::Trd(t) => t.add_file(f),
            Image::Scl(s) => s.add_file(f),
        }
    }

    /// Delete by display name. For TRD this marks the entry deleted (recoverable);
    /// for SCL it removes the entry outright. Returns whether a match was found.
    pub fn delete_file(&mut self, target: &str) -> bool {
        match self {
            Image::Trd(t) => t.mark_deleted(target),
            Image::Scl(s) => s.remove_file(target),
        }
    }

    /// Rename a file (by display name) to `new_basename`. Returns whether a match
    /// was found.
    pub fn rename_file(&mut self, target: &str, new_basename: &str) -> bool {
        match self {
            Image::Trd(t) => t.rename(target, new_basename),
            Image::Scl(s) => s.rename(target, new_basename),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            Image::Trd(t) => t.to_bytes(),
            Image::Scl(s) => s.to_bytes(),
        }
    }
}
