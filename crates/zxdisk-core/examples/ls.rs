//! List the catalog of a TRD or SCL image.
//!
//!   cargo run -p zxdisk-core --example ls -- path/to/image.trd

use zxdisk_core::Image;

fn main() {
    let path = match std::env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("usage: ls <image.trd|image.scl>");
            std::process::exit(2);
        }
    };
    let bytes = std::fs::read(&path).expect("cannot read file");
    let ext = std::path::Path::new(&path)
        .extension()
        .and_then(|e| e.to_str());
    let img = match Image::from_bytes(&bytes, ext) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("open failed: {e}");
            std::process::exit(1);
        }
    };

    let entries = img.entries();
    let live = entries.iter().filter(|e| !e.deleted).count();
    let deleted = entries.len() - live;
    println!(
        "{:?}  ({} live, {} deleted)",
        img.format(),
        live,
        deleted
    );
    println!("{:<11} {:>4} {:>7} {:>8} {:>8}", "name", "type", "start", "bytes", "status");
    for e in &entries {
        println!(
            "{:<11} {:>4} {:>7} {:>8} {:>8}",
            e.display_name(),
            e.file_type as char,
            e.start,
            e.length,
            if e.deleted { "deleted" } else { "ok" }
        );
    }
}
