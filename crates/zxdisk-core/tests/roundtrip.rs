use zxdisk_core::{DiskType, Format, Image, SclArchive, TrFile, TrdImage};

fn name8(s: &str) -> [u8; 8] {
    let mut n = [b' '; 8];
    for (i, b) in s.bytes().take(8).enumerate() {
        n[i] = b;
    }
    n
}

#[test]
fn trd_add_reload_delete_recover() {
    let mut trd = TrdImage::blank(DiskType::Ds80, "MYDISK");
    let free0 = trd.free_sectors();

    let data1 = vec![0xAAu8; 300]; // spans 2 sectors
    trd.add_file(&TrFile::new(name8("HELLO"), b'C', 0x8000, data1.clone()))
        .unwrap();
    let data2 = vec![0x55u8; 256];
    trd.add_file(&TrFile::new(name8("WORLD"), b'B', 10, data2.clone()))
        .unwrap();

    // Free space dropped by exactly the sectors consumed (2 + 1).
    assert_eq!(trd.free_sectors(), free0 - 3);

    let entries = trd.entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].display_name(), "HELLO.C");
    assert_eq!(entries[1].display_name(), "WORLD.B");
    assert_eq!(entries[0].data.len(), 512); // padded to whole sectors
    assert_eq!(&entries[0].data[..300], &data1[..]);
    assert_eq!(&entries[1].data[..256], &data2[..]);

    // Persist and re-open.
    let bytes = trd.to_bytes();
    assert_eq!(bytes.len(), DiskType::Ds80.size());
    let mut trd2 = TrdImage::from_bytes(&bytes).unwrap();
    assert_eq!(trd2.label(), "MYDISK");
    assert_eq!(trd2.entries().len(), 2);

    // Delete HELLO and confirm it is still recoverable with its data intact.
    assert!(trd2.mark_deleted("HELLO.C"));
    let e = trd2.entries();
    assert_eq!(e.len(), 2);
    assert!(e[0].deleted);
    assert_eq!(e[0].display_name(), "_ELLO.C");
    assert_eq!(&e[0].data[..300], &data1[..]);
    assert!(!e[1].deleted);
}

#[test]
fn trd_truncated_image_is_padded() {
    let mut trd = TrdImage::blank(DiskType::Ds80, "X");
    trd.add_file(&TrFile::new(name8("A"), b'C', 0, vec![1, 2, 3]))
        .unwrap();
    // Chop off trailing empty tracks, as many real .trd files are stored.
    let mut bytes = trd.to_bytes();
    bytes.truncate(0x2000); // keep catalog + a couple of tracks
    let trd2 = TrdImage::from_bytes(&bytes).unwrap();
    assert_eq!(trd2.entries().len(), 1);
    assert_eq!(&trd2.entries()[0].data[..3], &[1, 2, 3]);
}

#[test]
fn scl_roundtrip_and_checksum() {
    let mut scl = SclArchive::blank();
    let d = vec![1u8, 2, 3, 4, 5];
    scl.add_file(&TrFile::new(name8("BOOT"), b'B', 0, d.clone()))
        .unwrap();
    scl.add_file(&TrFile::new(name8("CODE"), b'C', 0x8000, vec![9u8; 600]))
        .unwrap();

    let bytes = scl.to_bytes();
    assert_eq!(&bytes[0..8], b"SINCLAIR");
    assert_eq!(bytes[8], 2);

    // Trailing 4 bytes are the little-endian additive sum of everything before.
    let sum: u32 = bytes[..bytes.len() - 4]
        .iter()
        .fold(0u32, |a, &b| a.wrapping_add(b as u32));
    let stored = u32::from_le_bytes([
        bytes[bytes.len() - 4],
        bytes[bytes.len() - 3],
        bytes[bytes.len() - 2],
        bytes[bytes.len() - 1],
    ]);
    assert_eq!(sum, stored);

    let mut scl2 = SclArchive::from_bytes(&bytes).unwrap();
    assert_eq!(scl2.files.len(), 2);
    assert_eq!(scl2.files[0].display_name(), "BOOT.B");
    assert_eq!(&scl2.files[0].data[..5], &d[..]);
    assert_eq!(scl2.files[0].data.len(), 256); // 1 sector
    assert_eq!(scl2.files[1].data.len(), 768); // 3 sectors for 600 bytes
    assert!(scl2.remove_file("BOOT.B"));
    assert_eq!(scl2.files.len(), 1);
}

#[test]
fn detect_and_host_filename() {
    let trd = TrdImage::blank(DiskType::Ss40, "X").to_bytes();
    assert_eq!(Image::detect(&trd), Some(Format::Trd));
    let scl = SclArchive::blank().to_bytes();
    assert_eq!(Image::detect(&scl), Some(Format::Scl));
    assert_eq!(Image::detect(b"nonsense-bytes-here-............"), None);

    let f = TrFile::from_host_filename("GAME.C", vec![0u8; 10]);
    assert_eq!(f.file_type, b'C');
    assert_eq!(f.display_name(), "GAME.C");
    // A 3-char extension sets the type (1st char) and the address bytes (2nd/3rd).
    let g = TrFile::from_host_filename("DATA.Xyz", vec![0u8; 10]);
    assert_eq!(g.file_type, b'X');
    assert_eq!(g.start, (b'y' as u16) | ((b'z' as u16) << 8));
    // A 4-char (non TR-DOS) extension falls back to code.
    let h = TrFile::from_host_filename("thing.data", vec![0u8; 10]);
    assert_eq!(h.file_type, b'C');
}

#[test]
fn extension_modes() {
    use zxdisk_core::ExtMode;
    // Non-standard type with printable address bytes 'c','r'.
    let start = (b'c' as u16) | ((b'r' as u16) << 8);
    let f = TrFile::new(name8("SCREEN"), b'S', start, vec![1u8; 10]);
    assert_eq!(f.ext_string_with(ExtMode::Single), "S");
    assert_eq!(f.ext_string_with(ExtMode::Triple), "Scr");
    assert_eq!(f.ext_string_with(ExtMode::Smart), "Scr");
    assert_eq!(f.display_name_with(ExtMode::Single), "SCREEN.S");
    assert_eq!(f.display_name_with(ExtMode::Smart), "SCREEN.Scr");

    // A CODE file with a binary load address (low byte 0x00).
    let c = TrFile::new(name8("CODE"), b'C', 0x8000, vec![1u8; 10]);
    assert_eq!(c.ext_string_with(ExtMode::Smart), "C"); // not both printable
    assert_eq!(c.ext_string_with(ExtMode::Single), "C");
    assert_eq!(c.ext_string_with(ExtMode::Triple), "C__"); // forced, binary -> '_'

    assert_eq!(ExtMode::parse("1"), Some(ExtMode::Single));
    assert_eq!(ExtMode::parse("3"), Some(ExtMode::Triple));
    assert_eq!(ExtMode::parse("SMART"), Some(ExtMode::Smart));
    assert_eq!(ExtMode::parse("nope"), None);
}

#[test]
fn rename_three_char_extension() {
    // Default mode is Smart, so this file displays with a 3-char extension.
    let mut trd = TrdImage::blank(DiskType::Ds80, "X");
    let start = (b'c' as u16) | ((b'r' as u16) << 8);
    trd.add_file(&TrFile::new(name8("SCREEN"), b'S', start, vec![1u8; 100]))
        .unwrap();
    assert_eq!(trd.entries()[0].display_name(), "SCREEN.Scr");
    // Renaming with a 3-char extension updates the type and the address bytes.
    assert!(trd.rename("SCREEN.Scr", "PIC.Abc"));
    let e = trd.entries();
    assert_eq!(e[0].file_type, b'A');
    assert_eq!(e[0].start, (b'b' as u16) | ((b'c' as u16) << 8));
    assert_eq!(e[0].display_name(), "PIC.Abc");
}

#[test]
fn hobeta_wrap_parse_is_lossless() {
    let f = TrFile::new(name8("SPRITE"), b'C', 0x7B00, vec![0x42u8; 500]);
    let wrapped = zxdisk_core::hobeta::wrap(&f);
    assert_eq!(wrapped.len(), zxdisk_core::hobeta::HEADER_LEN + 512); // header + 2 sectors

    let g = zxdisk_core::hobeta::parse(&wrapped).unwrap();
    assert_eq!(g.name, f.name);
    assert_eq!(g.file_type, b'C');
    assert_eq!(g.start, 0x7B00);
    assert_eq!(g.length, 500);
    assert_eq!(g.sectors, 2);
    assert_eq!(&g.data[..500], &vec![0x42u8; 500][..]);

    // A corrupted checksum is rejected (so callers fall back to raw import).
    let mut bad = wrapped.clone();
    bad[16] ^= 0xFF;
    assert!(zxdisk_core::hobeta::parse(&bad).is_none());
    assert!(zxdisk_core::hobeta::parse(b"too short").is_none());
}

#[test]
fn rename_trd_and_scl() {
    // TRD: rename keeps data, changes name (and type when the extension changes).
    let mut trd = TrdImage::blank(DiskType::Ds80, "X");
    trd.add_file(&TrFile::new(name8("OLDNAME"), b'C', 0x8000, vec![0x11u8; 300]))
        .unwrap();
    trd.add_file(&TrFile::new(name8("KEEP"), b'B', 0, vec![0x22u8; 50]))
        .unwrap();
    assert!(trd.rename("OLDNAME.C", "NEWNAME.C"));
    let e = trd.entries();
    assert_eq!(e[0].display_name(), "NEWNAME.C");
    assert_eq!(&e[0].data[..300], &vec![0x11u8; 300][..]); // data untouched
    assert_eq!(e[0].start, 0x8000); // start untouched
    assert_eq!(e[1].display_name(), "KEEP.B"); // others untouched
    // Renaming with a new type letter changes the TR-DOS type.
    assert!(trd.rename("KEEP.B", "KEEP.C"));
    assert_eq!(trd.entries()[1].file_type, b'C');
    // Unknown target does nothing.
    assert!(!trd.rename("NOPE.C", "X.C"));

    // Deleted entries are not renamed.
    trd.mark_deleted("NEWNAME.C");
    assert!(!trd.rename("_EWNAME.C", "BACK.C"));

    // SCL: same behaviour.
    let mut scl = SclArchive::blank();
    scl.add_file(&TrFile::new(name8("PART1"), b'C', 0x6000, vec![9u8; 400]))
        .unwrap();
    assert!(scl.rename("PART1.C", "INTRO.C"));
    assert_eq!(scl.files[0].display_name(), "INTRO.C");
    assert_eq!(&scl.files[0].data[..400], &vec![9u8; 400][..]);
}

#[test]
fn blank_geometry_sizes() {
    use zxdisk_core::Image;
    // Default helper is 640K.
    match Image::blank_for_ext("trd") {
        Image::Trd(t) => assert_eq!(t.to_bytes().len(), DiskType::Ds80.size()),
        _ => panic!("expected trd"),
    }
    // Explicit geometry is honoured.
    for dt in [DiskType::Ds80, DiskType::Ds40, DiskType::Ss80, DiskType::Ss40] {
        match Image::blank_for_ext_with("trd", dt) {
            Image::Trd(t) => assert_eq!(t.to_bytes().len(), dt.size()),
            _ => panic!("expected trd"),
        }
    }
    // SCL ignores geometry.
    assert!(matches!(
        Image::blank_for_ext_with("scl", DiskType::Ss40),
        Image::Scl(_)
    ));
}

#[test]
fn image_wrapper_trd_and_scl() {
    for ext in ["trd", "scl"] {
        let mut img = Image::blank_for_ext(ext);
        img.add_file(&TrFile::new(name8("TEST"), b'C', 0, vec![7u8; 400]))
            .unwrap();
        let bytes = img.to_bytes();
        let img2 = Image::from_bytes(&bytes, Some(ext)).unwrap();
        let e = img2.entries();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].display_name(), "TEST.C");
        assert_eq!(&e[0].data[..400], &[7u8; 400][..]);
    }
}
