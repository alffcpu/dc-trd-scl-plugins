//! TR-DOS `.trd` disk image: a headerless, flat, sector-by-sector dump.
//!
//! Geometry is fixed at 256-byte sectors, 16 sectors per track. The catalog
//! lives in track 0 sectors 0..8 (offset 0x000..0x800), followed by the disk
//! info sector at offset 0x800. File data starts at logical track 1.

use crate::entry::{TrFile, MAX_SECTORS, NAME_LEN, SECTOR_SIZE};
use crate::error::{Error, Result};

/// Sectors per logical track.
pub const TRACK_SECTORS: usize = 16;
/// Maximum catalog entries.
pub const CATALOG_ENTRIES: usize = 128;
/// Bytes per catalog entry.
pub const ENTRY_SIZE: usize = 16;
/// Absolute offset of the disk-info sector (track 0, sector 8).
pub const INFO_SECTOR: usize = 8 * SECTOR_SIZE;
/// TR-DOS filesystem id byte, found at INFO_SECTOR + 0xE7.
pub const TRDOS_ID: u8 = 0x10;

// Field offsets inside the info sector (absolute file offsets).
const OFF_FIRST_FREE_SECTOR: usize = INFO_SECTOR + 0xE1;
const OFF_FIRST_FREE_TRACK: usize = INFO_SECTOR + 0xE2;
const OFF_DISK_TYPE: usize = INFO_SECTOR + 0xE3;
const OFF_NUM_FILES: usize = INFO_SECTOR + 0xE4;
const OFF_FREE_SECTORS: usize = INFO_SECTOR + 0xE5; // u16 little-endian
const OFF_TRDOS_ID: usize = INFO_SECTOR + 0xE7;
const OFF_DELETED_COUNT: usize = INFO_SECTOR + 0xF4;
const OFF_LABEL: usize = INFO_SECTOR + 0xF5; // 8 bytes

/// TR-DOS disk geometry variants, keyed by the disk-type byte at 0x8E3.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DiskType {
    /// 0x16 - 80 tracks, double sided, 640 KB (the common case).
    Ds80,
    /// 0x17 - 40 tracks, double sided, 320 KB.
    Ds40,
    /// 0x18 - 80 tracks, single sided, 320 KB.
    Ss80,
    /// 0x19 - 40 tracks, single sided, 160 KB.
    Ss40,
}

impl DiskType {
    pub fn from_byte(b: u8) -> Option<DiskType> {
        match b {
            0x16 => Some(DiskType::Ds80),
            0x17 => Some(DiskType::Ds40),
            0x18 => Some(DiskType::Ss80),
            0x19 => Some(DiskType::Ss40),
            _ => None,
        }
    }

    pub fn to_byte(self) -> u8 {
        match self {
            DiskType::Ds80 => 0x16,
            DiskType::Ds40 => 0x17,
            DiskType::Ss80 => 0x18,
            DiskType::Ss40 => 0x19,
        }
    }

    /// Number of logical tracks (cylinders * sides).
    pub fn tracks(self) -> usize {
        match self {
            DiskType::Ds80 => 160,
            DiskType::Ds40 => 80,
            DiskType::Ss80 => 80,
            DiskType::Ss40 => 40,
        }
    }

    pub fn total_sectors(self) -> usize {
        self.tracks() * TRACK_SECTORS
    }

    pub fn size(self) -> usize {
        self.total_sectors() * SECTOR_SIZE
    }
}

/// An in-memory TR-DOS disk image. `data` is always resized to the full nominal
/// size for its disk type so that offset arithmetic never runs past the end.
pub struct TrdImage {
    pub disk_type: DiskType,
    data: Vec<u8>,
}

impl TrdImage {
    /// Create a freshly formatted, empty disk.
    pub fn blank(disk_type: DiskType, label: &str) -> TrdImage {
        let mut data = vec![0u8; disk_type.size()];
        data[OFF_FIRST_FREE_SECTOR] = 0;
        data[OFF_FIRST_FREE_TRACK] = 1;
        data[OFF_DISK_TYPE] = disk_type.to_byte();
        data[OFF_NUM_FILES] = 0;
        let free = ((disk_type.tracks() - 1) * TRACK_SECTORS) as u16;
        data[OFF_FREE_SECTORS] = (free & 0xff) as u8;
        data[OFF_FREE_SECTORS + 1] = (free >> 8) as u8;
        data[OFF_TRDOS_ID] = TRDOS_ID;
        data[OFF_DELETED_COUNT] = 0;
        let lb = label.as_bytes();
        for i in 0..NAME_LEN {
            data[OFF_LABEL + i] = *lb.get(i).unwrap_or(&b' ');
        }
        TrdImage { disk_type, data }
    }

    /// Parse an existing image. Truncated images (trailing empty tracks chopped
    /// off) are accepted and zero-padded back to full size.
    pub fn from_bytes(bytes: &[u8]) -> Result<TrdImage> {
        if bytes.len() < INFO_SECTOR + SECTOR_SIZE {
            return Err(Error::UnknownFormat);
        }
        if bytes[OFF_TRDOS_ID] != TRDOS_ID {
            return Err(Error::UnknownFormat);
        }
        let disk_type = DiskType::from_byte(bytes[OFF_DISK_TYPE]).ok_or(Error::UnknownFormat)?;
        let mut data = bytes.to_vec();
        let want = disk_type.size();
        if data.len() < want {
            data.resize(want, 0);
        }
        Ok(TrdImage { disk_type, data })
    }

    pub fn label(&self) -> String {
        String::from_utf8_lossy(&self.data[OFF_LABEL..OFF_LABEL + NAME_LEN])
            .trim_end()
            .to_string()
    }

    pub fn num_files(&self) -> u8 {
        self.data[OFF_NUM_FILES]
    }

    pub fn free_sectors(&self) -> u16 {
        u16::from_le_bytes([self.data[OFF_FREE_SECTORS], self.data[OFF_FREE_SECTORS + 1]])
    }

    fn read_sectors(&self, track: u8, sector: u8, count: u8) -> Vec<u8> {
        let start = (track as usize * TRACK_SECTORS + sector as usize) * SECTOR_SIZE;
        let len = count as usize * SECTOR_SIZE;
        let mut out = vec![0u8; len];
        if start < self.data.len() {
            let avail = (self.data.len() - start).min(len);
            out[..avail].copy_from_slice(&self.data[start..start + avail]);
        }
        out
    }

    /// All catalog entries up to the end-of-catalog terminator, both live and
    /// deleted, in slot order, each with its data attached.
    pub fn entries(&self) -> Vec<TrFile> {
        let mut out = Vec::new();
        for slot in 0..CATALOG_ENTRIES {
            let o = slot * ENTRY_SIZE;
            let b0 = self.data[o];
            if b0 == 0x00 {
                break; // end of catalog
            }
            let mut name = [0u8; NAME_LEN];
            name.copy_from_slice(&self.data[o..o + NAME_LEN]);
            let file_type = self.data[o + 8];
            let start = u16::from_le_bytes([self.data[o + 9], self.data[o + 10]]);
            let length = u16::from_le_bytes([self.data[o + 11], self.data[o + 12]]);
            let sectors = self.data[o + 13];
            let start_sector = self.data[o + 14];
            let start_track = self.data[o + 15];
            let deleted = b0 == 0x01;
            let data = self.read_sectors(start_track, start_sector, sectors);
            out.push(TrFile {
                name,
                file_type,
                start,
                length,
                sectors,
                deleted,
                data,
            });
        }
        out
    }

    /// Append a new file to the catalog, allocating contiguous sectors forward
    /// from the free pointer (the way TR-DOS itself does - no fragmentation).
    pub fn add_file(&mut self, file: &TrFile) -> Result<()> {
        if file.data.len() > MAX_SECTORS * SECTOR_SIZE {
            return Err(Error::FileTooBig);
        }
        let need = file.data.len().div_ceil(SECTOR_SIZE);
        let num = self.data[OFF_NUM_FILES] as usize;
        if num >= CATALOG_ENTRIES {
            return Err(Error::TooManyFiles);
        }
        if need > self.free_sectors() as usize {
            return Err(Error::DiskFull);
        }
        let first_free_sector = self.data[OFF_FIRST_FREE_SECTOR];
        let first_free_track = self.data[OFF_FIRST_FREE_TRACK];

        // Directory entry.
        let mut name = file.name;
        if name[0] == 0x00 || name[0] == 0x01 {
            name[0] = b'_'; // never collide with terminator/deleted markers
        }
        let o = num * ENTRY_SIZE;
        self.data[o..o + NAME_LEN].copy_from_slice(&name);
        self.data[o + 8] = file.file_type;
        let start = file.start.to_le_bytes();
        self.data[o + 9] = start[0];
        self.data[o + 10] = start[1];
        // Preserve the catalog byte-length supplied by the caller (which may be
        // less than the sector-padded data size, e.g. restored from hobeta).
        let length = file.length.to_le_bytes();
        self.data[o + 11] = length[0];
        self.data[o + 12] = length[1];
        self.data[o + 13] = need as u8;
        self.data[o + 14] = first_free_sector;
        self.data[o + 15] = first_free_track;

        // File data (padded to a whole sector).
        let lin = first_free_track as usize * TRACK_SECTORS + first_free_sector as usize;
        let byte_off = lin * SECTOR_SIZE;
        let end = byte_off + need * SECTOR_SIZE;
        if end > self.data.len() {
            self.data.resize(end, 0);
        }
        self.data[byte_off..byte_off + file.data.len()].copy_from_slice(&file.data);
        for b in &mut self.data[byte_off + file.data.len()..end] {
            *b = 0;
        }

        // Update the info sector.
        let new_lin = lin + need;
        self.data[OFF_FIRST_FREE_SECTOR] = (new_lin % TRACK_SECTORS) as u8;
        self.data[OFF_FIRST_FREE_TRACK] = (new_lin / TRACK_SECTORS) as u8;
        let free = self.free_sectors() - need as u16;
        self.data[OFF_FREE_SECTORS] = (free & 0xff) as u8;
        self.data[OFF_FREE_SECTORS + 1] = (free >> 8) as u8;
        self.data[OFF_NUM_FILES] = (num + 1) as u8;
        if num + 1 < CATALOG_ENTRIES {
            self.data[(num + 1) * ENTRY_SIZE] = 0x00; // keep terminator
        }
        Ok(())
    }

    /// Mark the first live file whose display name matches `target` as deleted,
    /// exactly as TR-DOS ERASE does: set name[0] = 0x01, bump the deleted counter,
    /// leave the data untouched (so it stays recoverable). Returns whether a match
    /// was found.
    pub fn mark_deleted(&mut self, target: &str) -> bool {
        for slot in 0..CATALOG_ENTRIES {
            let o = slot * ENTRY_SIZE;
            let b0 = self.data[o];
            if b0 == 0x00 {
                break;
            }
            if b0 == 0x01 {
                continue; // already deleted
            }
            let mut name = [0u8; NAME_LEN];
            name.copy_from_slice(&self.data[o..o + NAME_LEN]);
            let probe = TrFile {
                name,
                file_type: self.data[o + 8],
                start: u16::from_le_bytes([self.data[o + 9], self.data[o + 10]]),
                length: 0,
                sectors: 0,
                deleted: false,
                data: Vec::new(),
            };
            if probe.display_name() == target || probe.hobeta_name() == target {
                self.data[o] = 0x01;
                self.data[OFF_DELETED_COUNT] = self.data[OFF_DELETED_COUNT].wrapping_add(1);
                return true;
            }
        }
        false
    }

    /// Rename the first live file whose display name matches `target` to
    /// `new_basename` (e.g. `"NEWNAME.C"`). Updates the 8-char name and the type
    /// letter in the catalog entry; data, start address and geometry are
    /// unchanged. Returns whether a match was found.
    pub fn rename(&mut self, target: &str, new_basename: &str) -> bool {
        for slot in 0..CATALOG_ENTRIES {
            let o = slot * ENTRY_SIZE;
            let b0 = self.data[o];
            if b0 == 0x00 {
                break;
            }
            if b0 == 0x01 {
                continue; // never rename a deleted entry
            }
            let mut name = [0u8; NAME_LEN];
            name.copy_from_slice(&self.data[o..o + NAME_LEN]);
            let probe = TrFile {
                name,
                file_type: self.data[o + 8],
                start: u16::from_le_bytes([self.data[o + 9], self.data[o + 10]]),
                length: 0,
                sectors: 0,
                deleted: false,
                data: Vec::new(),
            };
            if probe.display_name() == target || probe.hobeta_name() == target {
                let (mut nn, nt, ns) = TrFile::split_name_type(new_basename);
                if nn[0] == 0x00 || nn[0] == 0x01 {
                    nn[0] = b'_';
                }
                self.data[o..o + NAME_LEN].copy_from_slice(&nn);
                self.data[o + 8] = nt;
                // A 3-char extension also sets the two address bytes; a 1-char
                // extension leaves the load address untouched.
                if let Some(s) = ns {
                    let b = s.to_le_bytes();
                    self.data[o + 9] = b[0];
                    self.data[o + 10] = b[1];
                }
                return true;
            }
        }
        false
    }

    /// The full image bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.data.clone()
    }
}
