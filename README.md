# dc-zx-plugins
![AI Assisted](https://img.shields.io/badge/AI-assisted-brightgreen)

Plugins for [Double Commander](https://doublecmd.github.io/) to work with ZX
Spectrum disk images: browse `.trd`/`.scl` archives, extract/add/delete/rename
files, recover deleted TR-DOS entries, and preview ZX screen dumps (`.scr`)
without leaving the file manager.

- **zxdisk.wcx** - packer plugin: press Enter on a `.trd`/`.scl` to browse it
  like an archive; copy files out and in, delete, and recover deleted TR-DOS
  files.
- **zxdisk.wlx** - lister plugin: press F3 or Ctrl+Q to preview a ZX Spectrum
  **screen** - a standalone `.scr`, or one opened from inside a `.trd`/`.scl`.
- **zxdisk** - an optional command-line tool, also used to enable in-place
  rename from inside the archive view (see [Renaming in place](#renaming-in-place)).

Works on Windows, macOS, and Linux.

## Install

Close Double Commander first - the installer edits its config, and it
rewrites that file on quit, which would undo the changes.

### Windows

1. Get the plugin files: unpack a release, or [build them yourself](#building-from-source).
2. Double-click **`install.cmd`** (no admin rights needed) and follow the
   prompts: language, variant (`basic` or `rename`), install folder.
3. It detects your Double Commander's bitness automatically and installs the
   matching files.

### macOS

```sh
./install.sh
```

Answer the three prompts (language, variant, install folder).

Nice to have: [Homebrew](https://brew.sh/). The `rename` variant benefits from
a Lua library for auto-refresh after rename; if Homebrew is present and no Lua
library is found, the installer offers to run `brew install luajit` for you.
Without it, renaming still works via a toolbar button, just refreshed manually
(Ctrl+R - the physical Ctrl key, Double Commander doesn't use Cmd for its
shortcuts on macOS) instead of automatically.

### Linux

```sh
./install.sh
```

Same three prompts as macOS. For rename auto-refresh, install `luajit` (or
`lua5.1`) with your package manager beforehand; without it, renaming still
works via the toolbar button with a manual refresh.

The screen viewer (F3 preview) only lights up in Double Commander's Qt-based
builds (qt5/qt6 - the common packaged ones); in a GTK2 build it simply stays
inactive, and the archive plugin is unaffected either way.

### Uninstalling

Every install writes an uninstaller next to the installed files
(`uninstall.cmd` on Windows, `uninstall.sh` on macOS/Linux) that reverts every
config change it made and removes the files.

### After updating

Double Commander loads a plugin once and keeps it in memory, so after
installing a newer version, **restart Double Commander** so it picks it up.

## Using it

Double Commander's shortcuts use the physical **Ctrl** key on every platform,
including macOS - not Cmd (it follows Total Commander's Windows-style
shortcuts everywhere, e.g. Ctrl+R to refresh, Ctrl+Q for Quick View).

**Browsing an archive**: open a `.trd`/`.scl` with Enter or Ctrl+PgDn like any
other archive.

- **Extract** with F5. By default the raw file data is written; a setting can
  switch extraction to metadata-preserving `.hobeta` files (see
  [Settings](#settings)).
- **Add** with F5 back into the image. If the target image does not exist, it
  is created (640 KB TRD, or an SCL, based on the extension). Dropping in a
  `.hobeta` file restores its original name/type/address losslessly; any other
  file is added as raw data, with its TR-DOS type taken from the extension.
- **Delete** with F8 - a recoverable, TR-DOS-style erase on TRD images.
- **Recover a deleted file**: erased entries are listed read-only under a
  virtual `deleted\` folder; copy one back out to recover it.

**Previewing a screen**: press F3 or Ctrl+Q on any file that is exactly 6912
or 6144 bytes (recognised purely by size, whatever it's named) - on disk or
inside an image.

| Input | Action |
|---|---|
| `1`-`7`, or left-click | choose / cycle palette |
| `Shift`+`1`-`6` | zoom 1x-6x (remembered) |
| `Alt`+`0`-`7` | fixed border colour (0 black .. 7 white); `Alt`+`8` = auto/dominant (remembered) |
| `Space`, or right-click | invert |
| `Enter` | cycle brightness/attributes |

Palettes: Pulsar (default), wiki1, wiki2, Spectaculator, ATM, Next, Schafft.
If `1`-`7` don't reach the plugin, hold **Ctrl** - Double Commander's Lister
reserves the plain digit keys for its own shortcuts; the mouse always works.

### Renaming in place

The WCX archive API has no in-place rename. The `rename` install variant
works around this: it adds a toolbar button and a `Ctrl+Shift+R` hotkey
(physical Ctrl, same as on Windows/Linux - see the note above) that renames
the selected file right inside the archive view, refreshing automatically
when a Lua library is available (see [Install](#install)). Setting this up
by hand instead is covered in [Development](docs/DEVELOPMENT.md).

## Settings

Everything works with no configuration. To customize, edit the shared
settings file the installer creates for you with defaults and comments
already filled in (an existing file is left untouched):

- macOS/Linux: `~/.config/zxdisk.conf`
- Windows: `%APPDATA%\zxdisk\zxdisk.conf`

| Setting | Default | Meaning |
|---|---|---|
| `ext_mode` | `smart` | Extension length shown for files: `1`, `3`, or `smart` (shows 3 letters only when they look like real ASCII text). |
| `extract_hobeta` | `false` | Extract as lossless `.hobeta` files (preserves type, start address, length) instead of raw data. |
| `new_trd_geometry` | `640k` | Geometry used when creating a brand-new TRD image: `640k`, `320k-ds`, `320k-ss`, or `160k`. |

Each of these can also be set via an environment variable instead of the
file; the full list (including a diagnostic logging option) and the exact
lookup order are in [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

## Limitations

- The plugin's bitness/architecture must match Double Commander's (a 64-bit
  DC needs the 64-bit plugin). The installer always picks the right one for
  you automatically.
- On Linux, the screen viewer only works with Double Commander's Qt-based
  builds (qt5/qt6); it stays inactive without affecting anything else on a
  GTK2 build.
- Recovered (previously deleted) files are read-only.
- TR-DOS filenames are 8 characters; longer host file names are truncated
  when added, unless you add a `.hobeta` file (which preserves the original
  name).
- Adding a plain (non-`.hobeta`) file sets its TR-DOS type from the file
  extension and its start address to 0; add a `.hobeta` file instead to
  preserve those exactly.

## Command-line tool

`zxdisk` (installed with the `rename` variant, or usable on its own) works on
images from the shell:

```sh
zxdisk ls      image.trd
zxdisk rename  image.trd/OLD.C  NEW.C      # or: zxdisk rename image.trd OLD.C NEW.C
zxdisk delete  image.trd/NAME.C
zxdisk extract image.trd/NAME.C  out.bin
zxdisk add     image.trd  hostfile  [asname]
```

The combined `image.trd/ENTRY` form is exactly what Double Commander exposes
for a file inside a WCX archive, which is what the rename hotkey relies on.

## Building from source

Prerequisite: [Rust via rustup](https://rustup.rs/).

```sh
./scripts/build.sh
```

writes `dist/zxdisk.wcx`, `dist/zxdisk.wlx` and the `dist/zxdisk` CLI (both
bitnesses on Windows, a universal binary on macOS). Packaging a versioned,
installable release, the full per-platform build matrix, the architecture,
and the on-disk format reference are covered in
[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) and
[docs/FORMATS.md](docs/FORMATS.md).

## Tests

```sh
cargo test --workspace          # 65 tests
./scripts/coverage.sh           # how much of the code they reach, per file
```

The tests are where the formats are: `zxdisk-core` reads and writes TR-DOS
catalogues, SCL archives, Hobeta headers and ZX screens, and a byte wrong in any
of them is a corrupted disk image rather than a visible error. That crate is at
87% to 100% of lines across its files, including a round trip - build an image,
add files, save, reload, delete, recover - that checks the free-sector count
against what the entries actually took.

Around it: the WCX plugin's FFI boundary is driven the way Double Commander
drives it, through the exported C functions; the CLI's commands are called the
way `main` calls them, including every way each one refuses; and the lister
plugin's key semantics, settings file and panic guard are checked without a
window. That last one matters more than its size suggests - it is what stops a
panic on a malformed screen taking the whole file manager down with it.

`scripts/coverage.sh` prints a table per file, weakest first, and fails below a
floor. It installs nothing: Rust instruments a build itself and the profile is
read with the llvm tools the platform toolchain already carries.

What it cannot see is said where the number is. `#[cfg]` decides what a host can
compile, so the lister's Windows and Linux window code is not in a macOS build
and not in a macOS measurement; and the native window shells are not covered on
any host, because driving a Cocoa or Win32 window from a test needs a harness
this project does not have.

## License

[MIT](LICENSE).
