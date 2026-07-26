#!/usr/bin/env bash
#
# Build the plugins into dist/.
#
#   macOS   : universal (arm64 + x86_64) binaries when both Rust targets are
#             installed, otherwise a single-arch binary.
#   Linux   : host-native binaries.
#   Windows : both 64-bit and 32-bit when the matching Rust toolchains are
#             installed, otherwise whichever is available (run from Git Bash /
#             MSYS2). See below for the file names produced.
#
# Output (macOS / Linux): dist/zxdisk.wcx (packer plugin), dist/zxdisk.wlx
# (lister plugin - ZX screen viewer) and dist/zxdisk (CLI).
# Output (Windows):
#   dist/zxdisk.wcx64   64-bit packer plugin   (loaded by 64-bit Double Commander)
#   dist/zxdisk.wcx     32-bit packer plugin   (loaded by 32-bit Double Commander)
#   dist/zxdisk.wlx64   64-bit lister plugin (ZX screen viewer)   dist/zxdisk.wlx  32-bit
#   dist/zxdisk-x64.exe  64-bit CLI    dist/zxdisk-x86.exe  32-bit CLI
# The plugin extension is Double Commander's own arch convention (.wcx = x86,
# .wcx64 = x64), so both plugins live side by side and DC loads the one matching
# its bitness; the CLI exes use the usual Windows x86 / x64 tags.
set -euo pipefail
cd "$(dirname "$0")/.."

OUT=dist
mkdir -p "$OUT"

# "crate  lib_name  out_name" (Linux builds both plugins; the WLX screen viewer
# targets DC's qt5/qt6 builds and needs no extra build or runtime dependencies -
# it binds the QtPas library already loaded inside Double Commander itself).
PLUGINS=(
  "zxdisk-wcx zxdisk_wcx zxdisk.wcx"
  "zxdisk-wlx zxdisk_wlx zxdisk.wlx"
)

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*|Windows_NT)
    # ---- Windows: build each bitness with a matching Rust toolchain ----------
    #
    # A GNU toolchain whose *host* equals the *target* triple is required, since
    # its bundled mingw provides the right-bitness linker. (A 64-bit toolchain
    # can have the 32-bit target's std installed yet not link it - no 32-bit
    # linker - so we match host==target rather than trusting the target list.)
    if ! command -v rustup >/dev/null 2>&1; then
      echo "error: rustup not found. Install Rust from https://rustup.rs and, for" >&2
      echo "       self-contained builds, the GNU toolchains:" >&2
      echo "         rustup toolchain install stable-x86_64-pc-windows-gnu" >&2
      echo "         rustup toolchain install stable-i686-pc-windows-gnu   # 32-bit" >&2
      exit 1
    fi

    tc_host() { rustc +"$1" -vV 2>/dev/null | sed -n 's/^host: //p'; }
    # Echo a toolchain whose host == $1 and that has $1 as an installed target.
    toolchain_for() {
      local target="$1" tc host
      while read -r tc; do
        tc="${tc%% *}"                       # strip the "(default)" suffix
        [ -n "$tc" ] || continue
        host="$(tc_host "$tc")"
        [ "$host" = "$target" ] || continue
        if rustup target list --toolchain "$tc" --installed 2>/dev/null \
             | grep -qx "$target"; then
          echo "$tc"; return 0
        fi
      done < <(rustup toolchain list)
      return 1
    }

    # "arch  candidate-triples(comma)  plugin_out  cli_out"
    # Plugins keep DC's mandated extension (.wcx/.wlx = x86, .wcx64/.wlx64 = x64);
    # the CLI exes carry the usual Windows x86 / x64 arch tags.
    # Fields: arch  candidate-triples  wcx_out  cli_out  wlx_out
    WIN_ARCHES=(
      "x64 x86_64-pc-windows-gnu,x86_64-pc-windows-msvc zxdisk.wcx64 zxdisk-x64.exe zxdisk.wlx64"
      "x86 i686-pc-windows-gnu,i686-pc-windows-msvc     zxdisk.wcx   zxdisk-x86.exe zxdisk.wlx"
    )

    built=0
    for row in "${WIN_ARCHES[@]}"; do
      # shellcheck disable=SC2086
      set -- $row
      arch="$1"; triples="$2"; plugin_out="$3"; cli_out="$4"; wlx_out="$5"
      tc=""; target=""
      IFS=',' read -ra cands <<< "$triples"
      for t in "${cands[@]}"; do
        if tc="$(toolchain_for "$t")"; then target="$t"; break; fi
      done
      if [ -z "$target" ]; then
        echo "skip $arch  (no toolchain; e.g. rustup toolchain install stable-${cands[0]%%,*})"
        continue
      fi
      echo "building $arch via $tc ($target) ..."
      cargo "+$tc" build --release -p zxdisk-wcx -p zxdisk-wlx -p zxdisk-cli --target "$target" >&2
      cp "target/$target/release/zxdisk_wcx.dll" "$OUT/$plugin_out"
      cp "target/$target/release/zxdisk_wlx.dll" "$OUT/$wlx_out"
      cp "target/$target/release/zxdisk.exe"     "$OUT/$cli_out"
      for f in "$plugin_out" "$wlx_out" "$cli_out"; do
        echo "== $OUT/$f =="; file "$OUT/$f" 2>/dev/null || true
      done
      built=$((built + 1))
    done
    [ "$built" -gt 0 ] || { echo "error: no Windows toolchain found to build with" >&2; exit 1; }
    ;;

  Darwin)
    installed=$(rustup target list --installed 2>/dev/null || true)
    targets=()
    for t in aarch64-apple-darwin x86_64-apple-darwin; do
      if echo "$installed" | grep -qx "$t"; then
        targets+=("$t")
      else
        echo "skip $t  (enable universal with: rustup target add $t)"
      fi
    done
    # macOS builds both plugins: the WCX packer and the WLX screen viewer.
    MAC_PLUGINS=(
      "zxdisk-wcx zxdisk_wcx zxdisk.wcx"
      "zxdisk-wlx zxdisk_wlx zxdisk.wlx"
    )
    for plugin in "${MAC_PLUGINS[@]}"; do
      # shellcheck disable=SC2086
      set -- $plugin
      crate="$1"; libname="$2"; outname="$3"
      if [ "${#targets[@]}" -eq 0 ]; then
        echo "building $crate (host) ..."
        cargo build --release -p "$crate" >&2
        cp "target/release/lib${libname}.dylib" "$OUT/$outname"
      else
        slices=()
        for t in "${targets[@]}"; do
          echo "building $crate for $t ..."
          cargo build --release -p "$crate" --target "$t" >&2
          slices+=("target/$t/release/lib${libname}.dylib")
        done
        if [ "${#slices[@]}" -eq 1 ]; then
          cp "${slices[0]}" "$OUT/$outname"
        else
          lipo -create -output "$OUT/$outname" "${slices[@]}"
        fi
      fi
      echo "== $OUT/$outname =="
      lipo -info "$OUT/$outname" 2>/dev/null || file "$OUT/$outname"
    done
    # CLI tool (universal)
    if [ "${#targets[@]}" -eq 0 ]; then
      cargo build --release -p zxdisk-cli >&2
      cp "target/release/zxdisk" "$OUT/zxdisk"
    else
      cli_slices=()
      for t in "${targets[@]}"; do
        cargo build --release -p zxdisk-cli --target "$t" >&2
        cli_slices+=("target/$t/release/zxdisk")
      done
      if [ "${#cli_slices[@]}" -eq 1 ]; then
        cp "${cli_slices[0]}" "$OUT/zxdisk"
      else
        lipo -create -output "$OUT/zxdisk" "${cli_slices[@]}"
      fi
    fi
    echo "== $OUT/zxdisk =="
    lipo -info "$OUT/zxdisk" 2>/dev/null || file "$OUT/zxdisk"
    ;;

  Linux)
    for plugin in "${PLUGINS[@]}"; do
      # shellcheck disable=SC2086
      set -- $plugin
      crate="$1"; libname="$2"; outname="$3"
      echo "building $crate (host) ..."
      cargo build --release -p "$crate" >&2
      cp "target/release/lib${libname}.so" "$OUT/$outname"
      echo "== $OUT/$outname =="
      file "$OUT/$outname"
    done
    cargo build --release -p zxdisk-cli >&2
    cp "target/release/zxdisk" "$OUT/zxdisk"
    echo "== $OUT/zxdisk =="
    file "$OUT/zxdisk"
    ;;

  *)
    cargo build --release
    echo "copy the built library from target/release/ and rename it to .wcx (and the zxdisk binary)"
    ;;
esac
