#!/usr/bin/env bash
# archive-make.sh: the archive holds every file git sees, hidden ones and uncommitted edits
# included, and nothing git ignores. Every refusal holds. Runs in throwaway repositories.
#
#   bash tools/archive-make_test.sh
#   ARCHIVE=/path/to/other.sh bash tools/archive-make_test.sh   # one that packs ignored files FAILS
set -euo pipefail
unset GIT_DIR GIT_INDEX_FILE GIT_WORK_TREE GIT_PREFIX GIT_COMMON_DIR
ARCHIVE="${ARCHIVE:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/archive-make.sh}"
. "$(dirname "${BASH_SOURCE[0]}")/test-sandbox.sh"
sandbox_make archive-test
sandbox_enter --multi
t=$SANDBOX_REPO
fail=0
check() { # check <label> <command...>
    local label=$1
    shift
    if "$@"; then echo "ok   $label"; else
        echo "FAIL $label"
        fail=1
    fi
}
refused() { # refused <label> <needle> <archive-make.sh args...>: exit 1 and the needle on stderr
    local label=$1 needle=$2 rc=0 err
    shift 2
    err=$(bash "$ARCHIVE" "$@" 2>&1 >/dev/null) || rc=$?
    if [[ $rc == 1 && $err == *"$needle"* ]]; then echo "ok   $label"; else
        echo "FAIL $label (want rc=1 '$needle', got rc=$rc: $err)"
        fail=1
    fi
}

r="$t/proj"
git init -q -b main "$r"
mkdir -p "$r/src" "$r/.config" "$r/target/debug" "$r/.cache" "$r/docs/drafts"
printf 'target/\n.cache/\n*.log\n/docs/drafts/\n.env\n' >"$r/.gitignore"
echo main >"$r/src/main.rs"
echo hidden >"$r/.config/settings.toml"
echo '-x' >"$r/-dash.txt"
echo spaces >"$r/with space.txt"
echo umlaut >"$r/grüße.txt"
git -C "$r" add -A
git -C "$r" commit -q -m "feat: first"
echo edited >"$r/src/main.rs"
echo new >"$r/.hidden-new"
echo bin >"$r/target/debug/app"
echo cache >"$r/.cache/x"
echo log >"$r/run.log"
echo draft >"$r/docs/drafts/d.md"
echo secret >"$r/.env"
echo gone >"$r/gone.txt"
git -C "$r" add gone.txt
git -C "$r" commit -q -m "feat: gone"
rm "$r/gone.txt"

sub="$t/sub"
git init -q -b main "$sub"
printf 'out/\n' >"$sub/.gitignore"
echo lib >"$sub/lib.rs"
git -C "$sub" add -A
git -C "$sub" commit -q -m "feat: sub"
git -C "$r" -c protocol.file.allow=always submodule add -q "$sub" vendor/sub
mkdir -p "$r/vendor/sub/out"
echo built >"$r/vendor/sub/out/o"

entries() { bsdtar -tf "$1" 2>/dev/null | LC_ALL=C sed 's#^\./##; s#/$##' | LC_ALL=C sort; }
has() { LC_ALL=C grep -qxF -e "$2" <<<"$1"; }

out="$(bash "$ARCHIVE" --repo "$r" "$SANDBOX/a.tar.zst" 2>/dev/null)"
check "the last line is the archive path" test "$out" = "$SANDBOX/a.tar.zst"
list="$(entries "$out")"
for want in src/main.rs .config/settings.toml .gitignore .hidden-new -dash.txt "with space.txt" \
    .gitmodules vendor/sub/lib.rs vendor/sub/.gitignore; do
    check "packs $want" has "$list" "$want"
done
for not in target/debug/app .cache/x run.log docs/drafts/d.md .env gone.txt vendor/sub/out/o \
    .git/HEAD vendor/sub/.git; do
    check "leaves out $not" bash -c '! grep -qxF "$1" <<<"$2"' _ "$not" "$list"
done
mkdir "$SANDBOX/x"
bsdtar -xf "$out" -C "$SANDBOX/x"
check "an uncommitted edit packs as it is on disk" grep -qx edited "$SANDBOX/x/src/main.rs"
# The archive may store a non-ASCII name in another Unicode form; it extracts to the same file.
check "a non-ASCII name packs" test -f "$SANDBOX/x/grüße.txt"

g="$(bash "$ARCHIVE" --repo "$r" --with-git "$SANDBOX/g.tar.gz" 2>/dev/null)"
glist="$(entries "$g")"
check "--with-git packs .git" has "$glist" .git/HEAD
check "--with-git still leaves ignored files out" bash -c '! grep -qxF target/debug/app <<<"$1"' _ "$glist"
mkdir "$SANDBOX/y"
bsdtar -xf "$g" -C "$SANDBOX/y"
check "--with-git restores the history" test "$(git -C "$SANDBOX/y" log --format=%s -1)" = "feat: gone"

z="$(bash "$ARCHIVE" --repo "$r" "$SANDBOX/z.zip" 2>/dev/null)"
check ".zip writes a zip" bash -c 'head -c2 "$1" | grep -q PK' _ "$z"
check ".zip leaves ignored files out" bash -c '! bsdtar -tf "$1" | grep -q target/' _ "$z"

d="$(cd "$SANDBOX" && bash "$ARCHIVE" --repo "$r" 2>/dev/null)"
check "the default name says the repo, sha and dirty" \
    test "$d" = "$t/proj-$(git -C "$r" rev-parse --short HEAD)-dirty.tar.zst"

refused "an out path inside the repo" "inside the repo" --repo "$r" "$r/self.tar.zst"
refused "an out file that exists" "exists" --repo "$r" "$SANDBOX/a.tar.zst"
refused "an unknown extension" "name it" --repo "$r" "$SANDBOX/a.rar"
nogit="$SANDBOX/nogit"
mkdir -p "$nogit"
GIT_CEILING_DIRECTORIES="$SANDBOX" refused "a directory outside any checkout" "no git checkout" \
    --repo "$nogit"
rc=0
bash "$ARCHIVE" --nope >/dev/null 2>&1 || rc=$?
check "an unknown flag prints the usage" test "$rc" = 1

if [[ $fail != 0 ]]; then
    echo "archive-make_test: FAILED"
    exit 1
fi
echo "archive-make_test: ok"
