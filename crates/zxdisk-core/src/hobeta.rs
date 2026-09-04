//! `.hobeta` single-file container (`.$B`, `.$C`, `.$D`, ...).
//!
//! A hobeta file is one TR-DOS file's sector data prefixed with a 17-byte header
//! that carries the catalog metadata (name, type, start address, byte length,
//! sector count) plus a 2-byte header checksum. This is the standard way to move
//! a single TR-DOS file around losslessly, so the plugin extracts to and imports
//! from hobeta.
//!
//! Header layout (17 bytes), then `sectors * 256` bytes of data:
//! ```text
//!   0x00  8  filename
//!   0x08  1  type letter
//!   0x09  2  start address        (LE)
//!   0x0B  2  length in bytes      (LE)
//!   0x0D  1  length in sectors
//!   0x0E  1  must be 0x00
//!   0x0F  2  header checksum       (LE)
//! ```

use crate::entry::{TrFile, NAME_LEN, SECTOR_SIZE};

/// Total size of the hobeta header.
pub const HEADER_LEN: usize = 17;

/// Header checksum over the first 15 bytes (0x00..=0x0E):
/// `S = sum over i of header[i]*257 + i`, taken modulo 0x10000.
pub fn header_crc(h15: &[u8]) -> u16 {
    let mut s: u32 = 0;
    for (i, &b) in h15.iter().take(15).enumerate() {
        s = s.wrapping_add((b as u32).wrapping_mul(257).wrapping_add(i as u32));
    }
    (s & 0xFFFF) as u16
}

/// Wrap a TR-DOS file into hobeta bytes (17-byte header + sector-padded data).
pub fn wrap(file: &TrFile) -> Vec<u8> {
    let sectors = file.data.len().div_ceil(SECTOR_SIZE).min(255) as u8;
    let mut name = file.name;
    if name[0] == 0x00 || name[0] == 0x01 {
        name[0] = b'_'; // recovered/deleted entries: give the lost char a placeholder
    }
    let mut h = [0u8; HEADER_LEN];
    h[0..NAME_LEN].copy_from_slice(&name);
    h[8] = file.file_type;
    h[9..11].copy_from_slice(&file.start.to_le_bytes());
    h[11..13].copy_from_slice(&file.length.to_le_bytes());
    h[13] = sectors;
    h[14] = 0;
    let crc = header_crc(&h[0..15]);
    h[15..17].copy_from_slice(&crc.to_le_bytes());

    let mut out = Vec::with_capacity(HEADER_LEN + sectors as usize * SECTOR_SIZE);
    out.extend_from_slice(&h);
    let mut data = file.data.clone();
    data.resize(sectors as usize * SECTOR_SIZE, 0);
    out.extend_from_slice(&data);
    out
}

/// Parse hobeta bytes back into a TR-DOS file, restoring all metadata. Returns
/// `None` if the bytes are not a valid hobeta (bad checksum or shape), so callers
/// can fall back to treating the input as raw data.
pub fn parse(bytes: &[u8]) -> Option<TrFile> {
    if bytes.len() < HEADER_LEN || bytes[14] != 0 {
        return None;
    }
    if header_crc(&bytes[0..15]) != u16::from_le_bytes([bytes[15], bytes[16]]) {
        return None;
    }
    let mut name = [0u8; NAME_LEN];
    name.copy_from_slice(&bytes[0..NAME_LEN]);
    let file_type = bytes[8];
    let start = u16::from_le_bytes([bytes[9], bytes[10]]);
    let length = u16::from_le_bytes([bytes[11], bytes[12]]);
    let sectors = bytes[13];
    let dlen = sectors as usize * SECTOR_SIZE;
    let mut data = vec![0u8; dlen];
    let avail = (bytes.len() - HEADER_LEN).min(dlen);
    data[..avail].copy_from_slice(&bytes[HEADER_LEN..HEADER_LEN + avail]);
    Some(TrFile {
        name,
        file_type,
        start,
        length,
        sectors,
        deleted: false,
        data,
    })
}
