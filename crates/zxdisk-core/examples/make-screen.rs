//! Generate demo ZX Spectrum screen files to try the WLX viewer with.
//!
//!   cargo run -p zxdisk-core --example make-screen -- out_dir/
//!
//! Writes `demo.scr` (6912: colour bars + a diagonal ink texture + a flashing
//! cell) and `demo-mono.scr` (6144: bitmap only, shown white-on-black).

use std::path::PathBuf;

const W: usize = 256;
const H: usize = 192;
const BITMAP: usize = W * H / 8;

// Byte offset of pixel (x, y) in the ZX "thirds" interleave.
fn addr(x: usize, y: usize) -> usize {
    (y >> 6) * 2048 + (y & 7) * 256 + ((y >> 3) & 7) * 32 + (x >> 3)
}

fn bitmap() -> Vec<u8> {
    let mut b = vec![0u8; BITMAP];
    // A fine diagonal texture so ink pixels show over the paper colour.
    for y in 0..H {
        for x in 0..W {
            if (x + y) % 4 == 0 {
                b[addr(x, y)] |= 0x80 >> (x & 7);
            }
        }
    }
    b
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "samples".into());
    let dir = PathBuf::from(out);
    std::fs::create_dir_all(&dir).unwrap();

    // 6912: bitmap + attributes. 8 vertical colour bars (paper), white ink,
    // bright on the bottom half, and the top-left cell set to FLASH.
    let mut scr = bitmap();
    for cy in 0..24 {
        for cx in 0..32 {
            let paper = (cx / 4) as u8 & 7; // 8 bars, 4 cells wide
            let ink = 7u8; // white
            let bright = if cy >= 12 { 0x40 } else { 0 };
            let mut attr = (paper << 3) | ink | bright;
            if cx == 0 && cy == 0 {
                attr |= 0x80; // flashing cell
            }
            scr.push(attr);
        }
    }
    let p = dir.join("demo.scr");
    std::fs::write(&p, &scr).unwrap();
    println!("wrote {} ({} bytes)", p.display(), scr.len());

    // 6144: bitmap only.
    let p = dir.join("demo-mono.scr");
    let mono = bitmap();
    std::fs::write(&p, &mono).unwrap();
    println!("wrote {} ({} bytes)", p.display(), mono.len());
}
