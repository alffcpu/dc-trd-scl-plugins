#!/usr/bin/env bash
#
# Build and package a ready-to-hand-off release under dist/.
#
#   ./scripts/release.sh [VERSION]
#
# VERSION defaults to the VERSION file, else `git describe`, else "dev".
# Produces, for the current OS/arch:
#
#   dist/dc-zx-plugins-<version>-<platform>/     folder you can hand to anyone
#   dist/dc-zx-plugins-<version>-<platform>.tar.gz
#
# The recipient needs no Rust: they unpack and run the installer inside the
# folder (it finds the bundled binaries next to itself) - ./install.sh on
# macOS/Linux, install.cmd on Windows. macOS builds are one universal folder
# (Intel + Apple Silicon); Linux needs one per arch, built on that arch; Windows
# is one folder holding both 32-bit and 64-bit plugins (a .zip, not a tarball).
set -euo pipefail
cd "$(dirname "$0")/.."   # repo root

VERSION="${1:-}"
if [ -z "$VERSION" ]; then
  if [ -f VERSION ]; then
    VERSION="$(tr -d ' \t\r\n' < VERSION)"   # \r too: VERSION may be CRLF on Windows
  elif command -v git >/dev/null 2>&1 && git rev-parse --git-dir >/dev/null 2>&1; then
    VERSION="$(git describe --tags --always --dirty 2>/dev/null || echo dev)"
  else
    VERSION="dev"
  fi
fi

echo "building binaries ..."
# Remove any previously built binaries first, so a missing toolchain cannot smuggle
# a stale artifact (older version or the other bitness) into the package. build.sh
# regenerates whatever it can for this host.
rm -f dist/zxdisk.wcx64 dist/zxdisk.wcx dist/zxdisk.wlx64 dist/zxdisk.wlx \
      dist/zxdisk-x64.exe dist/zxdisk-x86.exe dist/zxdisk dist/zxdisk.exe
./scripts/build.sh >/dev/null

case "$(uname -s)" in
  # ------------------------------------------------------------------ Windows
  MINGW*|MSYS*|CYGWIN*|Windows_NT)
    # build.sh writes zxdisk.wcx64 / zxdisk-x64.exe (64-bit) and zxdisk.wcx /
    # zxdisk-x86.exe (32-bit), plus zxdisk.wlx64 / zxdisk.wlx; ship whichever
    # were produced (a warning is printed for any that were not).
    [ -f dist/zxdisk.wcx64 ] || [ -f dist/zxdisk.wcx ] \
      || { echo "build produced no Windows plugin (.wcx64/.wcx)" >&2; exit 1; }

    PLAT="windows"
    NAME="dc-zx-plugins-$VERSION-$PLAT"
    OUT="dist/$NAME"
    rm -rf "$OUT"; mkdir -p "$OUT"

    for f in zxdisk.wcx64 zxdisk.wcx zxdisk.wlx64 zxdisk.wlx zxdisk-x64.exe zxdisk-x86.exe; do
      if [ -f "dist/$f" ]; then
        cp "dist/$f" "$OUT/"
      else
        echo "warning: $f was not built (toolchain missing?) - not included in the package" >&2
      fi
    done
    cp scripts/install.cmd      "$OUT/"
    cp scripts/install-core.ps1 "$OUT/"
    # No zxrename.lua here: the Windows installer generates its own (LuaJIT/FFI,
    # hidden launch) with the CLI path baked in - the repo's scripts/zxrename.lua
    # is the Unix (os.execute) reference used by install.sh.
    printf '%s\n' "$VERSION" > "$OUT/VERSION"

    cat > "$OUT/INSTALL.txt" <<EOF
dc-zx-plugins $VERSION ($PLAT)
ZX Spectrum .trd/.scl disk-image plugins for Double Commander.

This folder holds both architectures:
  zxdisk.wcx64 / zxdisk.wcx     packer plugin - browse/extract/add .trd/.scl
  zxdisk.wlx64 / zxdisk.wlx     lister plugin - view ZX screens (.scr 6912/6144)
  zxdisk-x64.exe / zxdisk-x86.exe   command-line tool
(the plugin extension is DC's own arch marker: .wcx/.wlx = x86, .wcx64/.wlx64 = x64;
 .wlx64 is x64, .wlx is x86). The installer picks the set matching your Double
Commander automatically. The screen viewer needs nothing extra - it uses only
standard Windows system libraries.

INSTALL  (no admin rights needed)
  Double-click  install.cmd  and follow the prompts (language, variant, folder).
  Close Double Commander first so it does not overwrite the config on quit.
  (install.cmd just launches install-core.ps1 with the PowerShell execution
  policy bypassed for that one run - nothing is changed system-wide. You can
  also run install-core.ps1 directly if you prefer.)

  Auto-refresh after rename works out of the box: Double Commander ships the
  Lua 5.1 library, so nothing extra to install on Windows.

UNINSTALL
  double-click  uninstall.cmd  inside the folder you installed into.

RU
  Установка:   двойной клик по install.cmd (язык, вариант, папка; права
               администратора не нужны). Перед установкой закрой Double Commander.
               install.cmd просто запускает install-core.ps1 в обход политики
               выполнения только для этого запуска - в системе ничего не меняется.
  Авто-обновление после переименования работает сразу - Lua идёт в составе DC.
  Удаление:    двойной клик по uninstall.cmd в папке установки.
EOF

    # Zip with a top-level folder (mirrors the tarball layout) via PowerShell,
    # so no external zip tool is required.
    ZIP="dist/$NAME.zip"; rm -f "$ZIP"
    powershell.exe -NoProfile -ExecutionPolicy Bypass -Command \
      "Compress-Archive -Force -Path '$(cygpath -w "$OUT")' -DestinationPath '$(cygpath -w "$ZIP")'" \
      >/dev/null

    echo
    echo "package folder : $OUT"
    echo "zip            : $ZIP"
    echo "contents:"
    ls -1 "$OUT" | sed 's/^/  /'
    ;;

  # ------------------------------------------------------------- macOS / Linux
  *)
    [ -f dist/zxdisk.wcx ] || { echo "build produced no dist/zxdisk.wcx" >&2; exit 1; }
    [ -f dist/zxdisk ]     || { echo "build produced no dist/zxdisk" >&2; exit 1; }

    case "$(uname -s)" in
      Darwin)
        if lipo -info dist/zxdisk.wcx 2>/dev/null | grep -q 'x86_64' \
           && lipo -info dist/zxdisk.wcx 2>/dev/null | grep -q 'arm64'; then
          PLAT="macos-universal"
        else
          PLAT="macos-$(uname -m)"
        fi ;;
      Linux) PLAT="linux-$(uname -m)" ;;
      *)     PLAT="$(uname -s)-$(uname -m)" ;;
    esac

    NAME="dc-zx-plugins-$VERSION-$PLAT"
    OUT="dist/$NAME"
    rm -rf "$OUT"; mkdir -p "$OUT"

    cp dist/zxdisk.wcx    "$OUT/"
    cp dist/zxdisk        "$OUT/"
    # The WLX screen viewer (macOS Cocoa / Linux Qt); ship it when it was built.
    have_wlx=0
    if [ -f dist/zxdisk.wlx ]; then
      cp dist/zxdisk.wlx "$OUT/"; have_wlx=1
    fi
    cp scripts/install.sh "$OUT/"
    cp scripts/zxrename.lua "$OUT/"
    chmod +x "$OUT/install.sh"
    printf '%s\n' "$VERSION" > "$OUT/VERSION"

    cat > "$OUT/INSTALL.txt" <<EOF
dc-zx-plugins $VERSION ($PLAT)
ZX Spectrum .trd/.scl disk-image plugins for Double Commander.

INSTALL
  ./install.sh
then follow the prompts (language, variant, install folder).
No sudo needed - everything installs into your home directory.

AUTO-REFRESH AFTER RENAME (optional) needs a Lua 5.1 library:
  macOS:  brew install luajit
  Linux:  sudo apt install luajit      # or: dnf install luajit / pacman -S luajit
Without it, rename still works via the CLI and you refresh with Ctrl+R.

UNINSTALL
  run the uninstall.sh created inside the folder you installed into.

RU
  Установка:   ./install.sh  (язык, вариант, папка). Sudo не нужен.
  Авто-обновление после переименования требует LuaJIT (см. выше).
  Удаление:    uninstall.sh в папке установки.
EOF

    if [ "$have_wlx" = 1 ]; then
      cat >> "$OUT/INSTALL.txt" <<'EOF'

SCREEN VIEWER
  A ZX screen viewer (zxdisk.wlx) is installed too: press F3 on a .scr file
  (6912 or 6144 bytes), on disk or inside a .trd/.scl. Nothing extra to install.
  In the viewer: 1..7 palette, Shift+1..6 zoom, Alt+0..8 border, Space invert.
  On Linux it works with the qt5/qt6 builds of Double Commander (the usual
  packaged ones); in a gtk2 build the viewer stays inactive.
EOF
    fi

    ( cd dist && tar czf "$NAME.tar.gz" "$NAME" )

    echo
    echo "package folder : $OUT"
    echo "tarball        : dist/$NAME.tar.gz"
    echo "contents:"
    ls -1 "$OUT" | sed 's/^/  /'
    ;;
esac
