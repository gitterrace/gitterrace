#!/usr/bin/env bash
# cross-clippy.sh: a finding only a non-host target compiles fails that target, and the host's own
# clippy never sees it (the class this tool exists for). Targets come from [cross] in lint.toml,
# none is exit 2, and a target that cannot run is exit 2, never a pass. Runs in a throwaway crate.
#
#   bash tools/cross-clippy_test.sh
#   CROSS=/path/to/other.sh bash tools/cross-clippy_test.sh   # one that lints only the host FAILS
set -uo pipefail
unset GIT_DIR GIT_INDEX_FILE GIT_WORK_TREE GIT_PREFIX GIT_COMMON_DIR
CROSS="${CROSS:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/cross-clippy.sh}"
. "$(dirname "${BASH_SOURCE[0]}")/test-sandbox.sh"
sandbox_make cross-clippy-test
sandbox_enter
t=$SANDBOX_REPO
fail=0
expect() { # expect <label> <want-rc> <needle> <cross-clippy args...>
    local label=$1 want=$2 needle=$3 rc=0 out
    shift 3
    out=$(CROSS_JOBS=2 bash "$CROSS" --repo "$t" "$@" 2>&1) || rc=$?
    if [[ $rc == "$want" && $out == *"$needle"* ]]; then echo "ok   $label"; else
        echo "FAIL $label (want rc=$want '$needle', got rc=$rc: ${out: -400})"
        fail=1
    fi
}
mkdir -p "$t/src" "$t/schwarzwald/config"
printf '[package]\nname = "probe"\nversion = "0.0.0"\nedition = "2021"\n[workspace]\n' >"$t/Cargo.toml"
# A product pins the kit's toolchain, and rustup's installed targets are that toolchain's.
cp "$(dirname "${BASH_SOURCE[0]}")/../configs/rust-toolchain-base.toml" "$t/rust-toolchain.toml"
# The Windows arm imports a name it never uses: only a Windows build compiles that arm.
printf '#[cfg(windows)]\nuse std::fs::File;\n\npub fn ok() -> u8 {\n    1\n}\n' >"$t/src/lib.rs"
git -C "$t" add -A && git -C "$t" -c user.name=t -c user.email=t@t commit -qm probe

expect "no targets anywhere is exit 2, not a pass" 2 "no targets"
printf '[cross]\ntargets = ["no-such-target-xyz"]\n' >"$t/schwarzwald/config/lint.toml"
expect "a target that cannot run is exit 2, named" 2 "not run on no-such-target-xyz"
expect "--dry-run reads the targets from lint.toml" 0 "no-such-target-xyz: cargo clippy" --dry-run
if (cd "$t" && rustup target list --installed 2>/dev/null) | grep -qx x86_64-pc-windows-gnu; then
    (cd "$t" && cargo clippy -q -j 2 -- -D warnings >/dev/null 2>&1) &&
        echo "ok   positive control: the host's clippy passes the Windows-only finding" ||
        { echo "FAIL the host's clippy already fails the probe, so it proves nothing"; fail=1; }
    expect "a Windows-only finding fails the Windows target" 1 "red on x86_64-pc-windows-gnu" x86_64-pc-windows-gnu
    printf 'pub fn ok() -> u8 {\n    1\n}\n' >"$t/src/lib.rs"
    expect "the clean crate passes the Windows target" 0 "clean on x86_64-pc-windows-gnu" x86_64-pc-windows-gnu
else
    echo "skip the Windows cases: rustup target add x86_64-pc-windows-gnu"
fi
if [[ $fail != 0 ]]; then
    echo "cross-clippy_test: FAILED"
    exit 1
fi
echo "cross-clippy_test: ok"
