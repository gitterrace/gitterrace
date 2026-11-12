#!/usr/bin/env bash
# Clippy for the targets a product ships but its host never lints: code behind a windows, linux or
# unix cfg compiles only there, so a host-only `cargo clippy` never sees it.
#
#   bash vendor/schwarzwald/kit/tools/cross-clippy.sh [--repo <repo>] [--dry-run] [<target>...]
#
# Targets default to `targets` under [cross] in schwarzwald/config/lint.toml. A Windows target runs
# natively (`cargo clippy --target`; install it with `rustup target add`). A Linux target runs in
# Docker on `git archive HEAD`, in rust:1-bookworm with libclang (C build scripts do not
# cross-compile from macOS), so commit before you run it. Each target lints the whole workspace,
# all targets, -D warnings, --keep-going, at -j CROSS_JOBS (default 4).
#
# Exit 0 when every target is clean, 1 naming each red target, 2 when a target cannot run (no
# Docker, no rustup target), so an unrun check never reads as a pass. --dry-run prints the
# command per target and runs nothing.
set -uo pipefail
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE
usage() { sed -n '2,15s/^# \{0,1\}//p' "$0" >&2; exit 2; }
repo="" dry=0 targets=()
while [ $# -gt 0 ]; do
    case "$1" in
        --repo) repo="${2:?--repo needs a path}"; shift 2 ;;
        --dry-run) dry=1; shift ;;
        -*) usage ;;
        *) targets+=("$1"); shift ;;
    esac
done
repo="$(git -C "${repo:-$PWD}" rev-parse --show-toplevel 2>/dev/null)" || {
    echo "cross-clippy: ${repo:-$PWD} is in no git checkout" >&2
    exit 2
}
jobs="${CROSS_JOBS:-4}"

# [cross] targets = ["a", "b"] from lint.toml, read with builtins (one line, the shape lint.toml uses).
if [ ${#targets[@]} -eq 0 ] && [ -f "$repo/schwarzwald/config/lint.toml" ]; then
    in=0
    while IFS= read -r line; do
        case "$line" in
            '['*) [ "$line" = "[cross]" ] && in=1 || in=0 ;;
            targets*)
                [ "$in" = 1 ] || continue
                list="${line#*[}"
                list="${list%]*}"
                IFS=',' read -ra raw <<<"$list"
                for t in "${raw[@]}"; do
                    t="${t//[[:space:]\"]/}"
                    [ -n "$t" ] && targets+=("$t")
                done
                ;;
        esac
    done <"$repo/schwarzwald/config/lint.toml"
fi
if [ ${#targets[@]} -eq 0 ]; then
    echo "cross-clippy: no targets: name them, or set targets under [cross] in schwarzwald/config/lint.toml" >&2
    exit 2
fi

# Docker's name for a Rust Linux target's platform.
platform() {
    case "${1%%-*}" in
        x86_64) echo linux/amd64 ;;
        aarch64) echo linux/arm64 ;;
        *) echo "linux/${1%%-*}" ;;
    esac
}

lints='-- -D warnings'
red=() unrun=()
for t in "${targets[@]}"; do
    case "$t" in
        *-linux-*)
            cmd="git archive HEAD | tar -x -C <tmp>; docker run --rm --platform $(platform "$t") rust:1-bookworm: apt-get install libclang-dev; cargo clippy -j $jobs --workspace --all-targets --keep-going $lints"
            ;;
        *-windows-* | *-apple-* | *) cmd="cargo clippy -j $jobs --workspace --all-targets --keep-going --target $t $lints" ;;
    esac
    if [ "$dry" = 1 ]; then
        echo "$t: $cmd"
        continue
    fi
    echo "== $t"
    case "$t" in
        *-linux-*)
            command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1 || {
                echo "cross-clippy: $t needs Docker, which is not running" >&2
                unrun+=("$t")
                continue
            }
            tree="$(mktemp -d "${TMPDIR:-/tmp}/cross-clippy.XXXXXX")"
            git -C "$repo" archive HEAD | tar -x -C "$tree"
            docker run --rm --platform "$(platform "$t")" -v "$tree":/src -w /src rust:1-bookworm sh -c \
                "apt-get update -qq >/dev/null && apt-get install -y -qq libclang-dev >/dev/null && rustup component add clippy >/dev/null && cargo clippy -j $jobs --workspace --all-targets --keep-going $lints" ||
                red+=("$t")
            rm -rf "$tree"
            ;;
        *)
            (cd "$repo" && rustup target list --installed 2>/dev/null) | grep -qx "$t" || {
                echo "cross-clippy: $t is not installed: rustup target add $t" >&2
                unrun+=("$t")
                continue
            }
            # shellcheck disable=SC2086 # $lints is two words on purpose
            (cd "$repo" && cargo clippy -j "$jobs" --workspace --all-targets --keep-going --target "$t" $lints) ||
                red+=("$t")
            ;;
    esac
done
[ "$dry" = 1 ] && exit 0
[ ${#red[@]} -eq 0 ] || echo "cross-clippy: red on ${red[*]}" >&2
[ ${#unrun[@]} -eq 0 ] || echo "cross-clippy: not run on ${unrun[*]}" >&2
[ ${#unrun[@]} -eq 0 ] || exit 2
[ ${#red[@]} -eq 0 ] || exit 1
echo "cross-clippy: clean on ${targets[*]}"
