# Third-party notices

A built plugin is not only this repository's code. It statically links a few
Rust crates and carries a set of palette tables that came from another project.
Each is listed below with what it is, where it came from, and the notice its
licence asks to travel with a distributed binary.

This file ships in the release packages beside `LICENSE`. Anyone redistributing
a build should carry both.

## Rust crates, statically linked

All of these are MIT-licensed (several are dual MIT/Apache-2.0; MIT is the one
taken here). The exact versions are pinned in [`Cargo.lock`](Cargo.lock).

| Crate | Platform | Purpose |
|---|---|---|
| `windows-sys` | Windows | Win32 API bindings: the GDI window, the menu, code-page conversion |
| `objc2`, `objc2-foundation`, `objc2-app-kit`, `block2` | macOS | Cocoa bindings for the viewer window |

The MIT text they are distributed under is the same as this project's own; see
[`LICENSE`](LICENSE). Their copyright holders are named in each crate's own
repository, and their source is what `Cargo.lock` pins.

Nothing is vendored: cargo fetches these at build time and they are not copied
into this repository.

## ZX Spectrum palettes

`crates/zxdisk-core/src/screen.rs` carries seven named palettes - Pulsar, two
Wikipedia sets, Spectaculator, ATM, Next and Schafft - ported from the
[Xpeccy](https://github.com/samstyle/Xpeccy) emulator's `conf/palettes/*.txt`.

Xpeccy is MIT, Copyright (c) 2009-..., SAM style. What was taken is the RGB
values, transcribed into Rust constants; no Xpeccy code is used or linked.

## The disk formats themselves

TR-DOS, `.trd`, `.scl` and the Hobeta header are file formats, documented in
[`docs/FORMATS.md`](docs/FORMATS.md) from public descriptions and from reading
real images. Formats are not copyrightable and nothing here is derived from
another implementation of them.
