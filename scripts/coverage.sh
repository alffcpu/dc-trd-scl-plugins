#!/usr/bin/env bash
#
# How much of this workspace the test suite actually executes.
#
#   ./scripts/coverage.sh            # table per file, weakest first, plus a total
#   ./scripts/coverage.sh --min 70   # a different floor for this run
#   ./scripts/coverage.sh --html     # also write an annotated report to target/coverage
#
# It installs nothing. Rust can instrument a build on its own with
# -C instrument-coverage, and the profile it writes is read here with the
# llvm-profdata/llvm-cov that come with the platform toolchain. cargo-llvm-cov
# is a nicer front end for exactly this and is one more thing to have installed
# before the number can be checked; a script anyone can run today is worth more
# than a better one they cannot.
#
# WHAT IS COUNTED
#
# crates/ only, which is all of this project's code. Within it, `#[cfg]` decides
# what a given host can even compile: the Windows and Linux halves of the lister
# plugin are not in a macOS build and cannot be measured there. The table says
# what it saw; it is a per-platform answer, not a universal one.
set -euo pipefail
cd "$(dirname "$0")/.."

# The floor the suite is held to. Set under what it reaches, deliberately: what
# a host can compile decides what it can measure, so a Windows or Linux run sees
# a different set of files from a macOS one and lands on a different number.
# The gap is that tolerance, not slack to spend.
MIN=70
HTML=0
while [ $# -gt 0 ]; do
  case "$1" in
    --min)  MIN="${2:?--min needs a percentage}"; shift 2 ;;
    --html) HTML=1; shift ;;
    -h|--help) sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

# The platform's llvm tools. On macOS they are behind xcrun; elsewhere they are
# on PATH, and a rustup-installed llvm-tools-preview works too.
llvm() {
  local tool="$1"; shift
  if command -v "$tool" >/dev/null 2>&1; then "$tool" "$@"
  elif command -v xcrun >/dev/null 2>&1 && xcrun --find "$tool" >/dev/null 2>&1; then
    xcrun "$tool" "$@"
  else
    echo "error: $tool not found. It ships with clang (Xcode Command Line Tools" >&2
    echo "       on macOS, the llvm package on Debian/Ubuntu), or with rustup:" >&2
    echo "       rustup component add llvm-tools-preview" >&2
    exit 1
  fi
}

PROF=target/coverage-profiles
rm -rf "$PROF"; mkdir -p "$PROF"

# %p keeps the test binaries from writing over each other's profile.
export RUSTFLAGS="-C instrument-coverage"
export LLVM_PROFILE_FILE="$PWD/$PROF/%p-%m.profraw"

echo "building and running the tests, instrumented"
cargo test --workspace >/dev/null

# The binaries that ran, asked of cargo rather than guessed from the file names:
# every test target carries a hash, and a stale one left in target/ would
# otherwise be measured instead of the one that just ran.
BINS=$(cargo test --workspace --no-run --message-format=json 2>/dev/null \
       | "${PYTHON:-python3}" -c '
import sys, json
for line in sys.stdin:
    try: m = json.loads(line)
    except ValueError: continue
    if m.get("profile", {}).get("test") and m.get("executable"):
        print(m["executable"])
')
[ -n "$BINS" ] || { echo "error: cargo listed no test executables" >&2; exit 1; }

llvm llvm-profdata merge -sparse "$PROF"/*.profraw -o "$PROF/merged.profdata"

OBJ=()
first=1
for b in $BINS; do
  if [ $first -eq 1 ]; then OBJ+=("$b"); first=0; else OBJ+=(-object "$b"); fi
done

# The report. Test and example sources are dropped from it: a suite that reports
# on itself flatters itself, and the examples are documentation that happens to
# compile.
REPORT=$(llvm llvm-cov report "${OBJ[@]}" \
           -instr-profile="$PROF/merged.profdata" \
           -ignore-filename-regex='(tests|examples)/|tests\.rs$|_tests\.rs$' crates 2>/dev/null)

# Weakest first: the point of the table is what to test next. Column 10 is the
# line-coverage percentage; sort -n reads the number and stops at the '%'.
echo
printf '%s\n' "$REPORT" | head -1
printf '%s\n' "$REPORT" | sed -n '2p'
printf '%s\n' "$REPORT" | sed -n '3,$p' | grep -v '^-\{10,\}' | grep -v '^TOTAL' \
  | sed '/^$/d' | LC_ALL=C sort -k10 -n
printf '%s\n' "$REPORT" | sed -n '2p'
printf '%s\n' "$REPORT" | grep '^TOTAL'

if [ "$HTML" -eq 1 ]; then
  llvm llvm-cov show "${OBJ[@]}" -instr-profile="$PROF/merged.profdata" \
       -ignore-filename-regex='(tests|examples)/|tests\.rs$|_tests\.rs$' -format=html \
       -output-dir=target/coverage crates >/dev/null
  echo
  echo "annotated report: target/coverage/index.html"
fi

LINES=$(printf '%s\n' "$REPORT" | LC_ALL=C awk '/^TOTAL/ {gsub("%","",$(NF-3)); print $(NF-3)}')
echo
echo "The lister plugin's Windows and Linux halves are #[cfg]-ed out of this build,"
echo "so this is what one platform can see. core/ and the WCX plugin are host-neutral."
echo
LC_ALL=C awk -v got="$LINES" -v min="$MIN" 'BEGIN {
  if (got + 0.05 < min) { printf "FAIL: %.2f%% of lines, below the %.1f%% floor\n", got, min; exit 1 }
  printf "OK: %.2f%% of lines, floor is %.1f%%\n", got, min
}'
