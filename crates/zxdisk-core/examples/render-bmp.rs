//! Render a ZX screen file to a 24-bit BMP, for eyeballing the decoder.
//!   cargo run -p zxdisk-core --example render-bmp -- input.scr output.bmp
//!
//! Uses the default render options (2x, 32px black border, Pulsar palette).

use zxdisk_core::screen::{RenderOpts, Screen};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (inp, outp) = (&args[0], &args[1]);
    let bytes = std::fs::read(inp).expect("read input");
    let scr = Screen::parse(&bytes).expect("not a 6912/6144 screen");
    let img = scr.render(false, &RenderOpts::default());

    let (w, h) = (img.width as i32, img.height as i32);
    let row = ((w * 3) as u32).div_ceil(4) * 4; // rows padded to 4 bytes
    let pix = row * h as u32;
    let mut out: Vec<u8> = Vec::with_capacity(54 + pix as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + pix).to_le_bytes()); // file size
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes()); // pixel data offset
    out.extend_from_slice(&40u32.to_le_bytes()); // BITMAPINFOHEADER size
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes()); // positive -> bottom-up
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&pix.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for y in (0..h as usize).rev() {
        let mut n = 0u32;
        for x in 0..w as usize {
            let o = (y * w as usize + x) * 4;
            out.push(img.pixels[o + 2]); // B
            out.push(img.pixels[o + 1]); // G
            out.push(img.pixels[o]); // R
            n += 3;
        }
        while !n.is_multiple_of(4) {
            out.push(0);
            n += 1;
        }
    }
    std::fs::write(outp, &out).expect("write bmp");
    println!("wrote {} ({}x{})", outp, w, h);
}
