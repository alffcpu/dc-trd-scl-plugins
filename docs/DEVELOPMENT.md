# Development

Building, testing, packaging releases, and the internal architecture of
dc-zx-plugins. For what the plugins do and how to install them, see the
[README](../README.md).

## Stack

- **Rust** (stable, 2021 edition), single cargo workspace.
- **`zxdisk-core`** - the format library: pure Rust, no dependencies, no file
  I/O (works on byte buffers), fully unit-tested. All TRD/SCL logic - and the
  ZX screen decoder (`screen.rs`: deinterleave, attributes/palette, FLASH,
  border, scaling to an RGBA buffer) - lives here, so it is cross-platform and
  shared by both plugins and the CLI.
- **`zxdisk-wcx`** - a thin, panic-safe C-ABI shell over the core that exports
  the WCX packer plugin functions. Built as a `cdylib` (`.wcx`).
- **`zxdisk-wlx`** - the WLX screen viewer. A thin native window (Win32 GDI on
  Windows, Cocoa `NSView` on macOS, a Qt `QWidget` on Linux) that blits the
  RGBA buffer `zxdisk-core` produces; the view-model, settings, and key/click
  semantics live in one cross-platform module (`viewer.rs`) all three shells
  reuse. Built as a `cdylib` (`.wlx`). The Linux shell targets Double
  Commander's qt5/qt6 builds and links nothing: it binds the flat QtPas C API
  already loaded inside DC's own process (in a gtk2 build it stays inert).
- **`zxdisk-cli`** - a small `zxdisk` command-line tool over the core (list,
  rename, delete, extract, add). Also powers the "rename from a hotkey" recipe.

The WCX API is a plain C ABI, so the exact same code compiles to a native
shared library on every platform, renamed to `.wcx`:

| Platform | Built file | Plugin file |
|---|---|---|
| macOS (arm64 / x86_64 / universal) | `libzxdisk_wcx.dylib` | `zxdisk.wcx` |
| Linux (x86_64 / arm64) | `libzxdisk_wcx.so` | `zxdisk.wcx` |
| Windows (x64) | `zxdisk_wcx.dll` | `zxdisk.wcx64` |
| Windows (x86) | `zxdisk_wcx.dll` | `zxdisk.wcx` |

On Windows, Double Commander uses the file extension to tell a plugin's
bitness apart: `.wcx` is the 32-bit plugin and `.wcx64` the 64-bit one, so
both can sit in the same folder and DC loads the one matching its own build.

## Repository layout

```
dc-zx-plugins/
  Cargo.toml                 workspace
  crates/
    zxdisk-core/             format library (TRD, SCL, hobeta) - no FFI, no I/O
      src/{entry,trd,scl,hobeta,image,error}.rs
      tests/roundtrip.rs
      src/screen.rs          ZX screen decoder (6912/6144 -> RGBA) + palettes
      examples/ls.rs                    list a TRD/SCL image's catalog
      examples/make-samples.rs          generate demo .trd/.scl images
      examples/{make-screen,render-bmp}.rs   demo screens / render to BMP
    zxdisk-wcx/              the .wcx plugin (C-ABI cdylib)
      src/{lib,wcx}.rs
      tests/ffi.rs           end-to-end test through the C entry points
    zxdisk-wlx/              the .wlx screen viewer (C-ABI cdylib)
      src/lib.rs              module wiring
      src/viewer.rs           cross-platform view-model, settings, detect string
      src/win.rs              Windows GDI window
      src/mac.rs              macOS Cocoa NSView
      src/linux.rs            Linux Qt widget (DC qt5/qt6 builds, via QtPas)
    zxdisk-cli/              the zxdisk command-line tool
  scripts/
    build.sh                 build into dist/ (universal on macOS, 32+64-bit on Windows)
    build-linux-portable.sh  containerized old-glibc Linux build (runs on any 2020+ distro)
    install.sh               interactive installer + uninstaller (macOS + Linux)
    install.cmd              Windows installer entry point (double-click)
    install-core.ps1         the Windows installer itself (PowerShell; launched by install.cmd)
    release.sh               build + package a versioned release folder/tarball (.zip on Windows)
    version.sh               show / set / bump the version in ./VERSION
    zxrename.lua             reference Lua hotkey (rename + auto-refresh)
  VERSION                    project version, read by release.sh
  docs/                      this file + the format reference
  dist/                      build output (git-ignored)
```

## Building from source

Prerequisite: [Rust via rustup](https://rustup.rs/).

**Quick, any platform** (writes `dist/zxdisk.wcx`, `dist/zxdisk.wlx` and the
`dist/zxdisk` CLI):

```sh
./scripts/build.sh
```

Or the raw cargo build:

```sh
cargo build --release -p zxdisk-wcx
# -> target/release/libzxdisk_wcx.dylib | .so  (or zxdisk_wcx.dll on Windows)
```

### macOS universal (Apple Silicon + Intel)

Build both slices and lipo them:

```sh
rustup target add x86_64-apple-darwin      # once; arm64 is your host default
./scripts/build.sh                         # detects both, produces a fat .wcx / .wlx
```

### Linux (Intel / arm64)

```sh
./scripts/build.sh                              # host-native
```

Cross-compiling to a different Linux arch needs that target's std
(`rustup target add ...`) and a matching linker (e.g. the `aarch64-linux-gnu`
toolchain).

**Portable (old-glibc) build.** A binary built above runs on the build
machine's glibc **or newer** - that is its only runtime requirement (no Qt is
linked; the viewer binds Double Commander's own QtPas at runtime). To produce
a package for any glibc distro from ~2020 (glibc >= 2.31: Ubuntu 20.04+,
Debian 11+, Fedora 32+, Arch, openSUSE 15.3+...), build inside an old-glibc
container instead (needs docker or podman, rootless fine):

```sh
./scripts/build-linux-portable.sh            # x86_64 package
./scripts/build-linux-portable.sh aarch64    # arm64 package (emulated on an
                                             # x86_64 host: needs qemu-user-static)
```

It runs `release.sh` in `rust:1.96-bullseye` (Debian 11, glibc 2.31) and
writes the same `dist/dc-zx-plugins-<version>-linux-<arch>` folder + tarball.
musl distros (Alpine) are not covered - a plugin must match the host DC's
libc.

### Windows (native, both bitnesses)

Run `scripts/build.sh` from **Git Bash** (the MSYS shell that ships with Git
for Windows). It uses the self-contained GNU Rust toolchains, so no Visual
Studio Build Tools are needed. Install Rust from [rustup](https://rustup.rs/)
with the GNU host, then add the toolchains:

```sh
rustup toolchain install stable-x86_64-pc-windows-gnu   # 64-bit
rustup toolchain install stable-i686-pc-windows-gnu     # 32-bit (optional)
./scripts/build.sh
```

It builds each bitness for which a matching GNU toolchain is installed
(picking the toolchain whose host equals the target, so the right-bitness
linker is used) and writes:

```
dist/zxdisk.wcx64     x64 plugin      dist/zxdisk.wlx64    x64 viewer      dist/zxdisk-x64.exe  x64 CLI
dist/zxdisk.wcx       x86 plugin      dist/zxdisk.wlx      x86 viewer      dist/zxdisk-x86.exe  x86 CLI
```

The CLI exes carry the usual Windows `x86` / `x64` arch tags. The plugins use
Double Commander's own arch convention instead - the extension is the marker
(`.wcx`/`.wlx` = x86, `.wcx64`/`.wlx64` = x64). If only one toolchain is
present you get just that architecture; the other is skipped with a note.

## Releasing

To make a self-contained package that anyone can install without Rust:

```sh
./scripts/release.sh            # version comes from the ./VERSION file
```

The version is stored in `./VERSION`. Change it with the helper, then run
`release.sh`:

```sh
./scripts/version.sh            # print the current version
./scripts/version.sh 0.2.0      # set an explicit version
./scripts/version.sh patch      # or bump: patch | minor | major
```

`release.sh` resolves the version as: its argument if given, else
`./VERSION`, else `git describe`, else `dev`. It builds and assembles, for
the current OS/arch, a versioned folder plus a tarball/zip under `dist/`,
containing the installer, the built binaries, and an `INSTALL.txt`.

Same command everywhere - `./scripts/release.sh` - run on the target
platform (from Git Bash on Windows):

| Platform | Build on | Produces |
|---|---|---|
| macOS (Intel + Apple Silicon) | any Mac, once: `rustup target add aarch64-apple-darwin x86_64-apple-darwin` | one `macos-universal` package |
| Linux x86_64 | an x86_64 Linux box or container | `linux-x86_64` package |
| Linux arm64  | an arm64 Linux box or container  | `linux-aarch64` package |
| Windows (x64 + x86) | any Windows, once: `rustup toolchain install stable-x86_64-pc-windows-gnu stable-i686-pc-windows-gnu` | one `windows` package (a `.zip`) |

- macOS = one universal package for both Mac architectures (fused with
  `lipo`).
- Linux = one package per architecture; there is no universal Linux binary.
  Build on the oldest glibc you want to support so it runs on that glibc and
  newer - `./scripts/build-linux-portable.sh` does exactly that in a
  container (glibc 2.31 = any 2020+ distro). A musl distro like Alpine needs
  its own build.
- Windows = one `.zip` holding both bitnesses.

Publish it by tagging a version (`git tag v0.1.0 && git push --tags`) and
attaching the tarball(s)/zip as downloadable release assets wherever the
repository is hosted.

## Settings: full precedence and paths

Each setting is read with first match wins:

1. its environment variable;
2. a key in the ini Double Commander itself hands the plugin (its own plugin
   config, when DC provides one);
3. a key in a fallback config file, checked in this order:
   - `$HOME/.config/zxdisk.conf` (shared with the CLI)
   - `$HOME/.config/zxdisk-wcx.conf`
   - `$HOME/.config/doublecmd/zxdisk-wcx.conf`
   - `$HOME/Library/Application Support/doublecmd/zxdisk-wcx.conf` (macOS)
   - `%USERPROFILE%/.config/zxdisk.conf` (Windows, HOME is usually unset there)
   - `%USERPROFILE%/.config/zxdisk-wcx.conf`
   - `%APPDATA%/zxdisk/zxdisk.conf` (written by `install-core.ps1`)
   - `%APPDATA%/zxdisk/zxdisk-wcx.conf`
   - `%APPDATA%/doublecmd/zxdisk-wcx.conf`

The CLI only ever reads the shared `zxdisk.conf` (not `zxdisk-wcx.conf`), at
`$HOME/.config/zxdisk.conf`, `%USERPROFILE%/.config/zxdisk.conf`, or
`%APPDATA%/zxdisk/zxdisk.conf`.

| Config key | Env var | Default | Meaning |
|---|---|---|---|
| `ext_mode` | `ZXDISK_WCX_EXT_MODE` (plugin, also accepts the CLI's `ZXDISK_EXT_MODE`), `ZXDISK_EXT_MODE` (CLI, also accepts the plugin's `ZXDISK_WCX_EXT_MODE`) | `smart` | Extension length: `1`/`single`, `3`/`triple`, or `smart`. A TR-DOS type byte is followed by 2 bytes that are normally the load address but are sometimes 2 extra extension letters. `smart` shows a 3-char extension only when both bytes are printable ASCII (e.g. `spisok.CRD`), else 1 char. |
| `extract_hobeta` | `ZXDISK_WCX_HOBETA` | `false` | Extract as `.hobeta` (lossless: type, start address, length) instead of raw data. When on, files also list as `.$C` / `.$B`. |
| `new_trd_geometry` | `ZXDISK_WCX_TRD_GEOMETRY` | `640k` | Geometry of a newly created TRD: `640k` (80x2), `320k-ds` (40x2), `320k-ss` (80x1), `160k` (40x1). |
| `debug_log` | `ZXDISK_WCX_DEBUG` | `false` | Append one diagnostic line per operation to the debug log (troubleshooting). |
| `debug_log_path` | `ZXDISK_WCX_LOG` | `~/zxdisk-wcx.log` | Where the WCX debug log is written. |

The two env-var names for `ext_mode` are accepted by both the plugin and the
CLI specifically so a CLI-driven rename and the plugin's listing always agree
on how names are formed, whichever one you set.

## Advanced: wiring the rename hotkey manually

The `rename` variant of the installer already does all of this. To do it by
hand instead (e.g. a custom hotkey or command):

The WCX packer API cannot rename a file in place, so the recipe runs the
`zxdisk` CLI on the selected file from the WCX browse view. Inside a WCX
archive, Double Commander exposes `%A` (the real image path) and `%f` (the
entry name), which is all the tool needs.

1. Install the CLI (`./scripts/install.sh`, rename variant, puts `zxdisk` in
   the chosen folder).
2. Add a Double Commander **toolbar button** (right-click the toolbar >
   Configuration) of type *external command*:
   - Command: the installed `zxdisk`
   - Parameters: `rename %A %f %[New name for ZX file;%f]`
   - Hot key: `Ctrl+Shift+R` (the physical Ctrl key - Double Commander's
     shortcuts do not follow Mac's Cmd convention, on any platform)
3. Inside an image, select a file, press `Ctrl+Shift+R`, type the new name,
   then `Ctrl+R` to refresh the listing.

The `%[prompt;default]` token makes DC pop up an input box for the new name.
This route needs no extra dependencies but does not auto-refresh.

**Auto-refresh variant (needs a Lua 5.1 library):** bind a hotkey to
`cm_ExecuteScript` with its parameter pointing at the installed
`zxrename.lua`; it prompts, renames and refreshes in one keypress. LuaJIT
works and is ABI compatible; set its path in Configuration > Options > Lua.
On Windows the hotkey launches the CLI hidden via the Win32 API (no flashing
console window) before refreshing the panel.

## Testing

```sh
cargo test --workspace
./scripts/coverage.sh          # per file, weakest first, fails below a floor
```

- `zxdisk-core/tests/roundtrip.rs` - the format library: add, reload, delete and
  recover, truncated images, the SCL checksum, format detection, and the text of
  every error the user can be shown.
- `zxdisk-core/src/screen/tests.rs` - the screen decoder and renderer.
- `zxdisk-wcx/tests/ffi.rs` - the exported WCX functions driven the way Double
  Commander drives them: open, list, extract, pack, delete, detect. Including
  the one data-loss path this plugin has had, where an image that cannot be read
  must be an error rather than a reason to write a blank one over it.
- `zxdisk-wcx/src/tests.rs` - the helpers behind those exports.
- `zxdisk-wlx/src/viewer/model/tests.rs` - the key and click mapping, the shared
  settings file and what writing to it must not disturb, the panic guard, and
  the view model.
- `zxdisk-cli/src/tests.rs` - every command, the `image.trd/ENTRY` path split,
  and the shared `zxdisk.conf` reader.

Test code lives in files of its own rather than in `#[cfg(test)]` modules inside
the sources, so `scripts/coverage.sh` can leave it out of its own denominator.
Test code is covered by definition; counted, it raises the number without
covering anything.

`cargo clippy --workspace --all-targets --release -- -D warnings` and
`cargo fmt --all -- --check` should both stay clean.

CI runs `cargo test --workspace`, clippy with `-D warnings`, and a full
`./scripts/build.sh` on every push, against a pinned Rust image so that a new
stable's new lints cannot start failing the build on their own. Its
configuration is tied to the hosting of the working repository and is not part
of this copy.

## Possible future work

- A native config dialog (settings are currently file/env based).
- Support for other ZX Spectrum disk/tape formats in the same workspace
  structure.

## Format reference

The TRD/SCL binary layout is documented separately in
[docs/FORMATS.md](FORMATS.md).
