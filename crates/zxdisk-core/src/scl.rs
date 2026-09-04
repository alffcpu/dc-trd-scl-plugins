//! Sinclair `.scl` archive: a packed list of TR-DOS files, not a disk image.
//!
//! Layout: `"SINCLAIR"` (8) + file count N (1) + N * 14-byte descriptors +
//! concatenated file data (each `sectors * 256` bytes) + a trailing 4-byte
//! little-endian additive sum of every preceding byte. SCL has no geometry,
//! no free-space map and no deleted files.

use crate::entry::{TrFile, MAX_SECTORS, NAME_LEN, SECTOR_SIZE};
use crate::error::{Error, Result};

const MAGIC: &[u8; 8] = b"SINCLAIR";
const HEADER_LEN: usize = 9; // magic + count
const DESC_LEN: usize = 14;
/// SCL stores the file count in a single byte.
pub const MAX_FILES: usize = 255;

/// An in-memory SCL archive.
pub struct SclArchive {
    pub files: Vec<TrFile>,
}

impl SclArchive {
    pub fn blank() -> SclArchive {
        SclArchive { files: Vec::new() }
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<SclArchive> {
        if bytes.len() < HEADER_LEN || &bytes[0..8] != MAGIC {
            return Err(Error::UnknownFormat);
        }
        let n = bytes[8] as usize;
        let cat_start = HEADER_LEN;
        let cat_end = cat_start + n * DESC_LEN;
        if bytes.len() < cat_end {
            return Err(Error::BadArchive("catalog truncated".into()));
        }
        let mut files = Vec::with_capacity(n);
        let mut data_off = cat_end;
        for i in 0..n {
            let o = cat_start + i * DESC_LEN;
            let mut name = [0u8; NAME_LEN];
            name.copy_from_slice(&bytes[o..o + NAME_LEN]);
            let file_type = bytes[o + 8];
            let start = u16::from_le_bytes([bytes[o + 9], bytes[o + 10]]);
            let length = u16::from_le_bytes([bytes[o + 11], bytes[o + 12]]);
            let sectors = bytes[o + 13];
            let dlen = sectors as usize * SECTOR_SIZE;
            let mut data = vec![0u8; dlen];
            if data_off < bytes.len() {
                let avail = (bytes.len() - data_off).min(dlen);
                data[..avail].copy_from_slice(&bytes[data_off..data_off + avail]);
            }
            data_off += dlen;
            files.push(TrFile {
                name,
                file_type,
                start,
                length,
                sectors,
                deleted: false,
                data,
            });
        }
        Ok(SclArchive { files })
    }

    pub fn entries(&self) -> Vec<TrFile> {
        self.files.clone()
    }

    pub fn add_file(&mut self, file: &TrFile) -> Result<()> {
        if self.files.len() >= MAX_FILES {
            return Err(Error::TooManyFiles);
        }
        if file.data.len() > MAX_SECTORS * SECTOR_SIZE {
            return Err(Error::FileTooBig);
        }
        let mut f = file.clone();
        f.sectors = f.data.len().div_ceil(SECTOR_SIZE) as u8;
        // Preserve a caller-supplied catalog length (e.g. from hobeta); only
        // derive it when unset.
        if f.length == 0 && !f.data.is_empty() {
            f.length = f.data.len().min(u16::MAX as usize) as u16;
        }
        f.deleted = false;
        if f.name[0] == 0x00 || f.name[0] == 0x01 {
            f.name[0] = b'_';
        }
        self.files.push(f);
        Ok(())
    }

    /// Rename the first file whose display name matches `target` to
    /// `new_basename` (updates name and type; data unchanged).
    pub fn rename(&mut self, target: &str, new_basename: &str) -> bool {
        for f in &mut self.files {
            if f.display_name() == target || f.hobeta_name() == target {
                let (mut nn, nt, ns) = TrFile::split_name_type(new_basename);
                if nn[0] == 0x00 || nn[0] == 0x01 {
                    nn[0] = b'_';
                }
                f.name = nn;
                f.file_type = nt;
                if let Some(s) = ns {
                    f.start = s;
                }
                return true;
            }
        }
        false
    }

    /// Remove the first file whose display name matches `target`.
    pub fn remove_file(&mut self, target: &str) -> bool {
        if let Some(pos) = self
            .files
            .iter()
            .position(|f| f.display_name() == target || f.hobeta_name() == target)
        {
            self.files.remove(pos);
            true
        } else {
            false
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.push(self.files.len() as u8);
        for f in &self.files {
            let sectors = f.data.len().div_ceil(SECTOR_SIZE) as u8;
            let length = f.length;
            out.extend_from_slice(&f.name);
            out.push(f.file_type);
            out.extend_from_slice(&f.start.to_le_bytes());
            out.extend_from_slice(&length.to_le_bytes());
            out.push(sectors);
        }
        for f in &self.files {
            let need = f.data.len().div_ceil(SECTOR_SIZE) * SECTOR_SIZE;
            let mut padded = f.data.clone();
            padded.resize(need, 0);
            out.extend_from_slice(&padded);
        }
        let sum: u32 = out.iter().fold(0u32, |a, &b| a.wrapping_add(b as u32));
        out.extend_from_slice(&sum.to_le_bytes());
        out
    }
}
