//! Generate demo `.trd` and `.scl` images to try the plugin with.
//!
//!   cargo run -p zxdisk-core --example make-samples -- samples/
//!
//! The TRD contains a BASIC loader, two CODE blocks, and one erased file so the
//! deleted-file recovery can be seen. The SCL contains two files.

use std::path::PathBuf;

use zxdisk_core::{DiskType, SclArchive, TrFile, TrdImage};

fn name8(s: &str) -> [u8; 8] {
    let mut n = [b' '; 8];
    for (i, b) in s.bytes().take(8).enumerate() {
        n[i] = b;
    }
    n
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "samples".into());
    let dir = PathBuf::from(out);
    std::fs::create_dir_all(&dir).unwrap();

    let mut trd = TrdImage::blank(DiskType::Ds80, "DEMODISK");
    trd.add_file(&TrFile::new(
        name8("boot"),
        b'B',
        0,
        b"10 REM demo loader\r".to_vec(),
    ))
    .unwrap();
    trd.add_file(&TrFile::new(
        name8("screen"),
        b'C',
        0x4000,
        vec![0x00u8; 6912],
    ))
    .unwrap();
    trd.add_file(&TrFile::new(
        name8("music"),
        b'C',
        0x8000,
        vec![0xA5u8; 1500],
    ))
    .unwrap();
    trd.add_file(&TrFile::new(
        name8("oldtune"),
        b'C',
        0x8000,
        vec![0x5Au8; 800],
    ))
    .unwrap();
    trd.mark_deleted("oldtune.C"); // leave a recoverable deleted entry
    let p = dir.join("demo.trd");
    std::fs::write(&p, trd.to_bytes()).unwrap();
    println!("wrote {}", p.display());

    let mut scl = SclArchive::blank();
    scl.add_file(&TrFile::new(
        name8("intro"),
        b'C',
        0x6000,
        vec![0x11u8; 1024],
    ))
    .unwrap();
    scl.add_file(&TrFile::new(
        name8("part2"),
        b'C',
        0x6000,
        vec![0x22u8; 2048],
    ))
    .unwrap();
    let p = dir.join("demo.scl");
    std::fs::write(&p, scl.to_bytes()).unwrap();
    println!("wrote {}", p.display());
}
