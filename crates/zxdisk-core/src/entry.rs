//! A single TR-DOS file plus its data, shared by both the TRD and SCL models.

/// Length of a TR-DOS filename field, in bytes.
pub const NAME_LEN: usize = 8;
/// Size of one disk sector, in bytes.
pub const SECTOR_SIZE: usize = 256;
/// Largest file TR-DOS can hold: 255 sectors.
pub const MAX_SECTORS: usize = 255;

/// One catalog entry with its file data attached.
///
/// `name` holds the raw 8 bytes as stored on disk. For a deleted TRD entry the
/// first byte on disk is `0x01`; that is reflected in `deleted` and the original
/// first character is lost (rendered as `_` by [`TrFile::display_name`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrFile {
    pub name: [u8; NAME_LEN],
    /// TR-DOS type letter, e.g. `b'B'`, `b'C'`, `b'D'`, `b'#'`.
    pub file_type: u8,
    /// Start address / type-specific parameter (dir bytes 0x09..0x0B).
    pub start: u16,
    /// Length in bytes as recorded in the catalog (dir bytes 0x0B..0x0D).
    pub length: u16,
    /// Length in 256-byte sectors (dir byte 0x0D).
    pub sectors: u8,
    /// True if this entry was marked deleted on a TRD (name[0] == 0x01).
    pub deleted: bool,
    /// Raw file data, padded to a whole number of sectors (`sectors * 256`).
    pub data: Vec<u8>,
}

impl TrFile {
    /// Build a fresh (non-deleted) file from a name, type, start address and data.
    /// `sectors`/`length` are derived from `data`.
    pub fn new(name: [u8; NAME_LEN], file_type: u8, start: u16, data: Vec<u8>) -> TrFile {
        let sectors = data.len().div_ceil(SECTOR_SIZE).min(MAX_SECTORS) as u8;
        let length = data.len().min(u16::MAX as usize) as u16;
        TrFile {
            name,
            file_type,
            start,
            length,
            sectors,
            deleted: false,
            data,
        }
    }

    /// The sanitized 8-char name, trailing spaces trimmed. Non-printable bytes
    /// become `?`, path-unsafe characters become `_`, and a deleted/blank first
    /// character becomes `_`.
    pub fn base_name(&self) -> String {
        let mut raw = self.name;
        if self.deleted || raw[0] == 0x00 || raw[0] == 0x01 {
            raw[0] = b'_';
        }
        let base: String = raw
            .iter()
            .map(|&b| match b {
                0x20..=0x7e => b as char,
                _ => '?',
            })
            .collect();
        let base = sanitize(base.trim_end());
        if base.is_empty() {
            "_".to_string()
        } else {
            base
        }
    }

    /// Just the TR-DOS type byte as an extension token (`C`, or the hex byte for
    /// a non-printable type).
    pub fn type_char_ext(&self) -> String {
        match self.file_type {
            0x20..=0x7e => sanitize(&(self.file_type as char).to_string()),
            other => format!("{other:02X}"),
        }
    }

    /// The filename extension for a given [`ExtMode`]: 1 character (the type byte)
    /// or 3 characters.
    ///
    /// In a TR-DOS entry the type byte (0x08) is followed by 2 bytes (0x09-0x0A)
    /// that are normally the load address, but for many files are instead 2 extra
    /// extension letters. `Single` always shows 1 char; `Triple` always 3; `Smart`
    /// shows 3 only when both of those bytes are printable ASCII, else 1.
    pub fn ext_string_with(&self, mode: ExtMode) -> String {
        let base = self.type_char_ext();
        if base.chars().count() != 1 {
            return base; // non-printable type -> hex token, no extra chars
        }
        let lo = (self.start & 0xff) as u8;
        let hi = (self.start >> 8) as u8;
        let three = match mode {
            ExtMode::Single => false,
            ExtMode::Triple => true,
            ExtMode::Smart => lo.is_ascii_graphic() && hi.is_ascii_graphic(),
        };
        if three {
            format!("{base}{}{}", ext_byte_char(lo), ext_byte_char(hi))
        } else {
            base
        }
    }

    /// The extension using the process-wide default [`ExtMode`].
    pub fn ext_string(&self) -> String {
        self.ext_string_with(default_ext_mode())
    }

    /// Display name for a given mode, e.g. `"HELLO.C"` or `"PIC.SCR"`.
    pub fn display_name_with(&self, mode: ExtMode) -> String {
        format!("{}.{}", self.base_name(), self.ext_string_with(mode))
    }

    /// Display name using the process-wide default [`ExtMode`].
    pub fn display_name(&self) -> String {
        self.display_name_with(default_ext_mode())
    }

    /// Hobeta-style name, e.g. `"HELLO.$C"` (always the single type char - the
    /// hobeta header preserves the address/params).
    pub fn hobeta_name(&self) -> String {
        format!("{}.${}", self.base_name(), self.type_char_ext())
    }

    /// Parse a filename (its basename) into a TR-DOS 8-char name, type letter, and
    /// optional start-address override. A 1-character extension sets just the type;
    /// a 3-character extension sets the type (1st char) and the two address bytes
    /// (2nd/3rd chars, little-endian); any other length defaults to type `C`.
    pub fn split_name_type(basename: &str) -> ([u8; NAME_LEN], u8, Option<u16>) {
        let (stem, ext) = match basename.rfind('.') {
            Some(i) if i > 0 && i + 1 < basename.len() => (&basename[..i], &basename[i + 1..]),
            _ => (basename, ""),
        };
        let eb = ext.as_bytes();
        let (file_type, start_override) = match eb.len() {
            1 => (eb[0], None),
            3 => (eb[0], Some((eb[1] as u16) | ((eb[2] as u16) << 8))),
            _ => (b'C', None),
        };
        let mut name = [b' '; NAME_LEN];
        for (i, b) in stem.bytes().take(NAME_LEN).enumerate() {
            name[i] = b;
        }
        (name, file_type, start_override)
    }

    /// Parse a host filename (its basename) into a TR-DOS file.
    pub fn from_host_filename(basename: &str, data: Vec<u8>) -> TrFile {
        let (name, file_type, start) = TrFile::split_name_type(basename);
        TrFile::new(name, file_type, start.unwrap_or(0), data)
    }
}

/// How a file's extension is rendered and parsed (see [`TrFile::ext_string_with`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ExtMode {
    /// Always a single-character extension; the 2 following bytes stay a hidden
    /// load address.
    Single,
    /// Always a three-character extension (type + the 2 following bytes).
    Triple,
    /// Three characters when both following bytes are printable ASCII, else one.
    Smart,
}

impl ExtMode {
    /// Parse a setting value (`1`/`single`, `3`/`triple`, `smart`/`auto`).
    pub fn parse(s: &str) -> Option<ExtMode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "1" | "single" | "one" => Some(ExtMode::Single),
            "3" | "triple" | "three" => Some(ExtMode::Triple),
            "smart" | "auto" | "intelligent" => Some(ExtMode::Smart),
            _ => None,
        }
    }
}

use std::sync::atomic::{AtomicU8, Ordering};
static DEFAULT_EXT_MODE: AtomicU8 = AtomicU8::new(2); // 0=Single 1=Triple 2=Smart

/// Set the process-wide default extension mode (used by the parameterless name
/// methods). Plugins set this from their configuration.
pub fn set_default_ext_mode(mode: ExtMode) {
    let v = match mode {
        ExtMode::Single => 0,
        ExtMode::Triple => 1,
        ExtMode::Smart => 2,
    };
    DEFAULT_EXT_MODE.store(v, Ordering::Relaxed);
}

/// The current process-wide default extension mode (defaults to `Smart`).
pub fn default_ext_mode() -> ExtMode {
    match DEFAULT_EXT_MODE.load(Ordering::Relaxed) {
        0 => ExtMode::Single,
        1 => ExtMode::Triple,
        _ => ExtMode::Smart,
    }
}

/// Render one of the two "extension address" bytes as a filename character.
fn ext_byte_char(b: u8) -> char {
    match b {
        b'/' | b'\\' | b':' => '_',
        0x21..=0x7e => b as char,
        _ => '_',
    }
}

/// Replace characters that are unsafe in a host path or in an in-archive path.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '/' | '\\' | ':' => '_',
            c => c,
        })
        .collect()
}
