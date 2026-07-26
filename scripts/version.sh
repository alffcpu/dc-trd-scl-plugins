#!/usr/bin/env bash
#
# Show or change the project version stored in ./VERSION (release.sh reads it).
#
#   ./scripts/version.sh                    print the current version
#   ./scripts/version.sh 1.2.3              set an explicit version
#   ./scripts/version.sh patch|minor|major  bump one semver component
#
# This only updates the VERSION file. Building a package (release.sh) and
# tagging/publishing are separate steps.
set -euo pipefail
cd "$(dirname "$0")/.."
FILE=VERSION

current() { if [ -f "$FILE" ]; then tr -d ' \t\r\n' < "$FILE"; else printf '0.0.0'; fi; }  # \r too (CRLF on Windows)

arg="${1:-}"
if [ -z "$arg" ]; then
  current; echo; exit 0
fi

cur="$(current)"
case "$arg" in
  major|minor|patch)
    base="${cur%%-*}"                       # drop any -suffix before bumping
    IFS=. read -r MA MI PA <<< "$base"
    MA="${MA:-0}"; MI="${MI:-0}"; PA="${PA:-0}"
    case "$arg" in
      major) MA=$((MA+1)); MI=0; PA=0 ;;
      minor) MI=$((MI+1)); PA=0 ;;
      patch) PA=$((PA+1)) ;;
    esac
    new="$MA.$MI.$PA"
    ;;
  *)
    # explicit X.Y.Z (optionally a -suffix); reject malformed input like 1.2 or 1x.2.3
    if [[ "$arg" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-.+)?$ ]]; then
      new="$arg"
    else
      echo "error: version must be X.Y.Z (optionally -suffix), or one of: major minor patch" >&2
      exit 1
    fi
    ;;
esac

printf '%s\n' "$new" > "$FILE"

# Keep the Cargo workspace version in lockstep so crate/DLL metadata matches the
# VERSION file (crates inherit it via version.workspace = true). Only the
# [workspace.package] version is a literal `version = "..."`, so a first-match
# replace is safe.
if [ -f Cargo.toml ]; then
  # Portable in-place edit (BSD/macOS sed leaves a stray *-E backup with `-i -E`).
  # Rewrite the original file (cat, not mv) so its mode/owner are kept - `mv` from
  # a 0600 mktemp file would silently drop Cargo.toml to 0600.
  tmp="$(mktemp)"
  sed -E "s/^version = \"[^\"]*\"/version = \"$new\"/" Cargo.toml > "$tmp" \
    && cat "$tmp" > Cargo.toml && rm -f "$tmp"
fi

echo "version: $cur -> $new"
