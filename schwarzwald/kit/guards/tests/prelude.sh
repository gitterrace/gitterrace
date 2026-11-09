#!/usr/bin/env bash
# The suites' shared chrome: where the guards under test live, colour, and the PASS/FAIL line
# writers. Sourced first by every script in this directory. `-e` is off on purpose: an
# incidental non-zero grep must not end a suite as if it had found something.
set -uo pipefail

# The guards under test: this directory's parent, or GUARDS_SELFTEST_SRC (a red probe points
# it at a neutered copy). Never the product's checkout: the cases run in a fixture (lib.sh).
GUARDS_SRC="${GUARDS_SELFTEST_SRC:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
GUARDS_TESTS="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export GUARDS_SRC GUARDS_TESTS

if [ -n "${NO_COLOR:-}" ] || [ ! -t 1 ]; then
    RED='' GREEN='' YELLOW='' NC=''
else
    RED=$'\033[0;31m' GREEN=$'\033[0;32m' YELLOW=$'\033[0;33m' NC=$'\033[0m'
fi

emit_pass() { printf '%sPASS%s [%s] %s\n' "$GREEN" "$NC" "$1" "${2:-}"; }
emit_fail() { printf '%sFAIL%s [%s] %s\n' "$RED" "$NC" "$1" "${2:-}"; }

# Exit 1 when any violation was counted, 0 otherwise.
gate_exit() {
    [ "${1:-0}" -eq 0 ]
    exit $?
}
