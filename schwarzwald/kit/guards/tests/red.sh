#!/usr/bin/env bash
#
# The red runner: does neutering a guard actually turn the suite red, on the case it names?
# A green that proves nothing is as bad as a missing guard, so each probe copies the guards,
# replaces ONE literal in the copy (tests/red-probes.tsv says which), runs the selftest on the
# copy and asserts it fails on the named case. It never touches the tree it tests.
#
#   bash tests/red.sh                       the shared probes, against config.example.env
#   GUARDS_SELFTEST_PRODUCT=<root> bash tests/red.sh
#                                           also the product's probes (GUARD_PRODUCT_PROBES),
#                                           against the product's own config
#   GUARDS_RED_ONLY="id id" bash tests/red.sh   just those probes (a partial count)
#   GUARDS_RED_JOBS=8                        probes run at once (default 8)
#
# Exit 0 when every probe went red for its own case, 1 otherwise.
. "$(dirname "${BASH_SOURCE[0]}")/prelude.sh"

SUITE="$GUARDS_TESTS/selftest.sh"
PROBES=("$GUARDS_TESTS/red-probes.tsv")
JOBS="${GUARDS_RED_JOBS:-8}"
# A product that is not there would load no config and no probes, and the run would pass over the
# shared probes alone while claiming to have checked the product.
if [ -n "${GUARDS_SELFTEST_PRODUCT:-}" ] && [ ! -f "$GUARDS_SELFTEST_PRODUCT/schwarzwald/config/guards.env" ]; then
    echo "guards-red: GUARDS_SELFTEST_PRODUCT=$GUARDS_SELFTEST_PRODUCT has no schwarzwald/config/guards.env" >&2
    exit 2
fi
# The disk floor next: a copy made on a full disk can neuter nothing and still look red.
if [ -f "$GUARDS_SRC/../tools/disk-floor.sh" ]; then
    . "$GUARDS_SRC/../tools/disk-floor.sh"
    disk_floor "${GUARDS_SELFTEST_PRODUCT:-$PWD}" "${TMPDIR:-/tmp}" || exit 1
fi
WORK="$(mktemp -d "${TMPDIR:-/tmp}/guards-red.XXXXXX")" || { echo "red.sh: refused, cannot make a work dir under ${TMPDIR:-/tmp}" >&2; exit 1; }
trap 'rm -rf "$WORK"' EXIT
# A killed run takes its probes with it: a pool left behind keeps loading the machine.
trap 'kill $(jobs -p) 2>/dev/null; exit 143' TERM INT

# The product's probes name files relative to the product root: those are neutered in a copy
# of the product's rule tree, which the selftest then uses in place of the real one.
PRODUCT_PROBES=""
if [ -n "${GUARDS_SELFTEST_PRODUCT:-}" ]; then
    PRODUCT_PROBES=$(bash -c '. "$1/config.sh"; GUARD_CONFIG="$2/schwarzwald/config/guards.env"
        for k in $GUARD_CONFIG_KEYS; do unset "$k"; done
        guard_config_load >/dev/null 2>&1; printf %s "${GUARD_PRODUCT_PROBES:-}"' _ "$GUARDS_SRC" "$GUARDS_SELFTEST_PRODUCT")
    for f in $PRODUCT_PROBES; do PROBES+=("$GUARDS_SELFTEST_PRODUCT/$f"); done
fi

# A fingerprint of the tree under test, compared after the run: a probe only ever edits a copy.
_fingerprint() { (cd "$GUARDS_SRC" && find . -type f -print0 | sort -z | xargs -0 cksum) 2>/dev/null; }
before=$(_fingerprint)

# CONTROL: an UNMODIFIED copy must be GREEN before any probe means anything. A copy whose
# suite is red for another reason would make every probe read as teeth. It is the slowest run
# (the full selftest), so it runs alongside the probes and its verdict is read before theirs.
mkfifo "$WORK/pool" || exit 1
exec 3<>"$WORK/pool"
_i=0
while [ "$_i" -lt "$JOBS" ]; do echo t >&3; _i=$((_i + 1)); done
mkdir -p "$WORK/control"
cp -Rp "$GUARDS_SRC/." "$WORK/control/"
read -r -u 3 _tok
(GUARDS_SELFTEST_SRC="$WORK/control" bash "$SUITE" >"$WORK/control.out" 2>&1
    echo $? >"$WORK/control.rc"
    echo t >&3) &
control_pid=$!

# _product_slice <dir>: the parts of the product the selftest reads (its guards.env, its rule
# dirs and its cases dir), never the whole tree.
_product_slice() {
    local _d
    mkdir -p "$1/schwarzwald/config"
    cp -p "$GUARDS_SELFTEST_PRODUCT/schwarzwald/config/guards.env" "$1/schwarzwald/config/"
    for _d in $(bash -c '. "$1/config.sh"; GUARD_CONFIG="$2/schwarzwald/config/guards.env"
        guard_config_load >/dev/null 2>&1; printf "%s %s" "${GUARD_PRODUCT_RULE_DIRS:-}" "${GUARD_PRODUCT_CASES%/*}"' \
        _ "$GUARDS_SRC" "$GUARDS_SELFTEST_PRODUCT"); do
        mkdir -p "$1/$_d" && cp -Rp "$GUARDS_SELFTEST_PRODUCT/$_d/." "$1/$_d/"
    done
}

# One probe: $1 index, $2 probe file, then the fields. Writes $WORK/<index>.res.
_probe() {
    local n="$1" src="$2" id="$3" file="$4" needle="$5" expect="$6" replacement="$7" parts="$8"
    local copy="$WORK/$n" prod="" base target
    needle="${needle//\{NL\}/$'\n'}"
    replacement="${replacement//\{NL\}/$'\n'}"
    [ -n "$replacement" ] || replacement=zzz_never_matches_zzz
    mkdir -p "$copy/guards"
    cp -Rp "$GUARDS_SRC/." "$copy/guards/"
    base="$copy/guards"
    case "$src" in
        "$GUARDS_TESTS"/*)
            # A shared probe on the example config: a product run reads the product's config,
            # never the example, so the same literal is neutered there instead.
            if [ -n "${GUARDS_SELFTEST_PRODUCT:-}" ] && [ "$file" = config.example.env ]; then
                prod="$copy/product"
                _product_slice "$prod"
                base="$prod"
                file=schwarzwald/config/guards.env
            fi
            ;;
        *)
            # A product probe: its file is under the product root. Only what the selftest reads
            # is copied (never the whole product: a build tree there filled the disk).
            prod="$copy/product"
            _product_slice "$prod"
            mkdir -p "$prod/$(dirname "$file")"
            cp -p "$GUARDS_SELFTEST_PRODUCT/$file" "$prod/$file" 2>/dev/null
            base="$prod"
            ;;
    esac
    target="$base/$file"
    if [ ! -f "$target" ]; then
        printf 'ERROR\t%s\tnames a missing file: %s\n' "$id" "$file" >"$WORK/$n.res"
        return
    fi
    # The replacement runs outside bash: macOS's bash 3.2 never finishes `${text//needle/rep}`
    # once the file passes about 35 KB (fail-closed.sh is 41 KB), and spins at 100% CPU. awk
    # reads the literals from the environment, so no character in them is special.
    local rc=0
    if ! NEEDLE="$needle" awk 'BEGIN { RS = "\001" } index($0, ENVIRON["NEEDLE"]) { f = 1 } END { exit !f }' "$target"; then
        printf 'ERROR\t%s\tthe literal is not in %s: the probe is STALE\n' "$id" "$file" >"$WORK/$n.res"
        return
    fi
    NEEDLE="$needle" REPLACEMENT="$replacement" awk '
        BEGIN { RS = "\001"; n = ENVIRON["NEEDLE"]; r = ENVIRON["REPLACEMENT"] }
        {
            out = ""; s = $0
            while ((i = index(s, n)) > 0) { out = out substr(s, 1, i - 1) r; s = substr(s, i + length(n)) }
            printf "%s", out s
        }' "$target" >"$target.new" || {
        printf 'ERROR\t%s\tcould not rewrite %s\n' "$id" "$file" >"$WORK/$n.res"
        return
    }
    cat "$target.new" >"$target" && rm -f "$target.new"
    # Only the case files the probe names, and among their table cases only the one it must turn
    # red: the hand-rolled cases in those files always run.
    # The product goes through env: an assignment that comes out of an expansion is a word, so
    # bash would run it as a command and the suite would never start.
    local penv=()
    [ -z "$prod" ] || penv=("GUARDS_SELFTEST_PRODUCT=$prod")
    env ${penv[@]+"${penv[@]}"} GUARDS_SELFTEST_SRC="$copy/guards" GUARDS_SELFTEST_PARTS="$parts" \
        GUARDS_SELFTEST_ONLY="$expect" bash "$SUITE" >"$WORK/$n.out" 2>&1 || rc=$?
    if [ "$rc" -eq 0 ]; then
        printf 'GREEN\t%s\tthe suite stayed GREEN with this guard removed: nothing tests it\n' "$id" >"$WORK/$n.res"
    elif grep -Eq "FAIL.*$expect" "$WORK/$n.out"; then
        printf 'RED\t%s\tremoving the guard fails the case %s\n' "$id" "$expect" >"$WORK/$n.res"
    else
        printf 'WRONG\t%s\tred, but NOT on %s: %s\n' "$id" "$expect" \
            "$(grep -m1 FAIL "$WORK/$n.out" | tr -s ' ')" >"$WORK/$n.res"
    fi
    rm -rf "$copy"
}

n=0
for src in "${PROBES[@]}"; do
    [ -f "$src" ] || {
        echo "  FAIL probe file missing: $src"
        printf 'ERROR\t%s\tprobe file missing\n' "$src" >"$WORK/missing.res"
        continue
    }
    while IFS=$'\t' read -r id file needle expect replacement parts; do
        case "$id" in "" | "#"*) continue ;; esac
        [ -z "${GUARDS_RED_ONLY:-}" ] || case " $GUARDS_RED_ONLY " in *" $id "*) ;; *) continue ;; esac
        n=$((n + 1))
        # A pool of JOBS: bash 3.2 has no `wait -n`, and `jobs` in a command substitution sees
        # an empty table, so a FIFO holds one token per slot. A probe takes a token before it
        # starts and returns it when it ends, so a new one starts as soon as any one finishes.
        read -r -u 3 _tok
        { _probe "$n" "$src" "$id" "$file" "$needle" "$expect" "$replacement" "$parts"; echo t >&3; } &
    done <"$src"
done
wait
wait "$control_pid" 2>/dev/null
if [ "$(cat "$WORK/control.rc" 2>/dev/null)" != 0 ]; then
    echo "  FAIL CONTROL: the suite is RED against an UNMODIFIED copy of the guards."
    echo "       Every probe would go red for that reason instead of the guard it names. First failures:"
    grep 'FAIL' "$WORK/control.out" | head -3 | sed 's/^/       /'
    echo "guards-red: control failed, probe verdicts discarded"
    exit 1
fi
echo "  ok   [CONTROL] an unmodified copy is GREEN -- a red below means the guard"

verified=0
failures=0
for r in "$WORK"/*.res; do
    [ -e "$r" ] || continue
    IFS=$'\t' read -r verdict id why <"$r"
    if [ "$verdict" = RED ]; then
        echo "  ok   [$id] $why"
        verified=$((verified + 1))
    else
        echo "  FAIL [$id] $why"
        failures=$((failures + 1))
    fi
done

if [ "$(_fingerprint)" != "$before" ]; then
    echo "  FAIL the probe run modified the tree under test"
    failures=$((failures + 1))
fi

echo
if [ "$verified" -eq 0 ] && [ "$failures" -eq 0 ]; then
    echo "  FAIL no probe ran -- the probe files are empty or did not parse"
    failures=1
fi
if [ "$failures" -eq 0 ]; then
    echo "guards-red: $verified probes verified, the suite has teeth"
    exit 0
fi
echo "guards-red: $failures probe(s) failed, $verified verified" >&2
exit 1
