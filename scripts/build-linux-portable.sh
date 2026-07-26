#!/usr/bin/env bash
#
# Portable Linux release build: runs ./scripts/release.sh inside a container
# with an OLD glibc (Debian 11 "bullseye", glibc 2.31), so the produced
# binaries run on any glibc distro from ~2020 on: Ubuntu 20.04+, Debian 11+,
# Fedora 32+, RHEL 9+, Arch/Manjaro, openSUSE 15.3+ ... A binary only needs
# the glibc it was built against OR NEWER, and nothing else - the plugins and
# CLI have no other runtime dependencies (the Qt viewer binds the QtPas
# library already inside Double Commander at runtime, so no Qt is linked).
#
# One build per CPU architecture is still needed (there is no universal Linux
# binary):
#
#   ./scripts/build-linux-portable.sh            # host arch (usually x86_64)
#   ./scripts/build-linux-portable.sh aarch64    # arm64; on an x86_64 host it
#                                                # runs emulated and needs
#                                                # qemu-user-static + binfmt
#
# Requires docker or podman (rootless is fine). For a quick host-native build
# without the old-glibc guarantee, plain ./scripts/release.sh is enough.
#
# Output: dist/dc-zx-plugins-<version>-linux-<arch>/ + .tar.gz, exactly as
# release.sh produces, owned by the invoking user.
set -euo pipefail
cd "$(dirname "$0")/.."

ARCH="${1:-$(uname -m)}"
case "$ARCH" in
  x86_64|amd64)  PLATFORM=linux/amd64 ;;
  aarch64|arm64) PLATFORM=linux/arm64 ;;
  *) echo "error: unsupported arch: $ARCH (use x86_64 or aarch64)" >&2; exit 1 ;;
esac

# Rust pinned to a known version; bullseye = glibc 2.31. If you also run CI,
# keep its Rust version and this one bumped together.
IMAGE=rust:1.96-bullseye

if command -v docker >/dev/null 2>&1; then
  RUNNER=docker
  # Build as the invoking user so dist/ is not root-owned. The official rust
  # images mark CARGO_HOME/RUSTUP_HOME world-writable, so this works.
  USERFLAGS=(--user "$(id -u):$(id -g)" -e HOME=/tmp)
elif command -v podman >/dev/null 2>&1; then
  RUNNER=podman
  # Rootless podman: keep the host uid inside the container instead of
  # mapping it to container-root, so created files stay owned by the user.
  USERFLAGS=(--userns=keep-id -e HOME=/tmp)
else
  echo "error: docker or podman is required (e.g. sudo pacman -S podman)" >&2
  exit 1
fi

# A separate target dir keeps host and container builds apart - they link
# against different glibc versions and must not share incremental artifacts.
exec "$RUNNER" run --rm --platform "$PLATFORM" \
  "${USERFLAGS[@]}" \
  -e CARGO_TARGET_DIR=/work/target/portable \
  -v "$PWD":/work -w /work \
  "$IMAGE" ./scripts/release.sh
