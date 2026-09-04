//! Unit tests for the WCX plugin helpers. The FFI boundary itself is driven
//! from tests/ffi.rs, the way Double Commander drives it.
//!
//! In a file of their own for the same reason as the others: test code that
//! sits in the source file is test code inside the coverage denominator.
use super::*;

fn name8(s: &str) -> [u8; 8] {
    let mut n = [b' '; 8];
    for (i, b) in s.bytes().take(8).enumerate() {
        n[i] = b;
    }
    n
}

#[test]
fn names_and_encoding_follow_mode() {
    let e = TrFile::new(name8("GAME"), b'C', 0x8000, vec![1, 2, 3]);
    // Default mode: plain name, raw bytes (no hobeta header).
    assert_eq!(entry_name(&e, false), "GAME.C");
    assert_eq!(encode_entry(&e, false), vec![1, 2, 3]);
    assert_eq!(extracted_len(&e, false), 3);
    // Hobeta mode: $-name, header + padded data, metadata round-trips.
    assert_eq!(entry_name(&e, true), "GAME.$C");
    let hob = encode_entry(&e, true);
    assert_eq!(hob.len(), zxdisk_core::hobeta::HEADER_LEN + 256);
    assert_eq!(
        extracted_len(&e, true),
        zxdisk_core::hobeta::HEADER_LEN + 256
    );
    let back = zxdisk_core::hobeta::parse(&hob).unwrap();
    assert_eq!(back.start, 0x8000);
    assert_eq!(back.length, 3);
}

#[test]
fn truthy_parses() {
    for v in ["1", "true", "YES", "On", " on "] {
        assert!(truthy(v), "{v:?} should be true");
    }
    for v in ["0", "false", "no", "", "maybe"] {
        assert!(!truthy(v), "{v:?} should be false");
    }
}

#[test]
fn geometry_parses() {
    assert_eq!(parse_geometry("640k"), Some(DiskType::Ds80));
    assert_eq!(parse_geometry(" 80X2 "), Some(DiskType::Ds80));
    assert_eq!(parse_geometry("320k-ds"), Some(DiskType::Ds40));
    assert_eq!(parse_geometry("320k-ss"), Some(DiskType::Ss80));
    assert_eq!(parse_geometry("160k"), Some(DiskType::Ss40));
    assert_eq!(parse_geometry("nonsense"), None);
}
