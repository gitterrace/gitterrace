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
#   GUARDS_RED_JOBS=4                        probes run at once (default 4)
#
# Exit 0 when every probe went red for its own case, 1 otherwise.
. "$(dirname "${BASH_SOURCE[0]}")/prelude.sh"

SUITE="$GUARDS_TESTS/selftest.sh"
PROBES=("$GUARDS_TESTS/red-probes.tsv")
JOBS="${GUARDS_RED_JOBS:-4}"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/guards-red.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

# The product's probes name files relative to the product root: those are neutered in a copy
# of the product's rule tree, which the selftest then uses in place of the real one.
PRODUCT_PROBES=""
if [ -n "${GUARDS_SELFTEST_PRODUCT:-}" ]; then
    PRODUCT_PROBES=$(bash -c '. "$1/config.sh"; GUARD_CONFIG="$2/config/schwarzwald/guards.env"
        for k in $GUARD_CONFIG_KEYS; do unset "$k"; done
        guard_config_load >/dev/null 2>&1; printf %s "${GUARD_PRODUCT_PROBES:-}"' _ "$GUARDS_SRC" "$GUARDS_SELFTEST_PRODUCT")
    for f in $PRODUCT_PROBES; do PROBES+=("$GUARDS_SELFTEST_PRODUCT/$f"); done
fi

# A fingerprint of the tree under test, compared after the run: a probe only ever edits a copy.
_fingerprint() { (cd "$GUARDS_SRC" && find . -type f -print0 | sort -z | xargs -0 cksum) 2>/dev/null; }
before=$(_fingerprint)

# CONTROL: an UNMODIFIED copy must be GREEN before any probe means anything. A copy whose
# suite is red for another reason would make every probe read as teeth.
mkdir -p "$WORK/control"
cp -Rp "$GUARDS_SRC/." "$WORK/control/"
if ! GUARDS_SELFTEST_SRC="$WORK/control" bash "$SUITE" >"$WORK/control.out" 2>&1; then
    echo "  FAIL CONTROL: the suite is RED against an UNMODIFIED copy of the guards."
    echo "       Every probe would go red for that reason instead of the guard it names. First failures:"
    grep 'FAIL' "$WORK/control.out" | head -3 | sed 's/^/       /'
    echo "guards-red: control failed, probes not run"
    exit 1
fi
echo "  ok   [CONTROL] an unmodified copy is GREEN -- a red below means the guard"

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
                mkdir -p "$prod/config/schwarzwald"
                cp -p "$GUARDS_SELFTEST_PRODUCT/config/schwarzwald/guards.env" "$prod/config/schwarzwald/"
                for _d in $(bash -c '. "$1/config.sh"; GUARD_CONFIG="$2/config/schwarzwald/guards.env"
                    guard_config_load >/dev/null 2>&1; printf "%s %s" "${GUARD_PRODUCT_RULE_DIRS:-}" "${GUARD_PRODUCT_CASES%/*}"' \
                    _ "$GUARDS_SRC" "$GUARDS_SELFTEST_PRODUCT"); do
                    mkdir -p "$prod/$_d" && cp -Rp "$GUARDS_SELFTEST_PRODUCT/$_d/." "$prod/$_d/"
                done
                base="$prod"
                file=config/schwarzwald/guards.env
            fi
            ;;
        *)
            # A product probe: its file is under the product root, copied whole into a
            # throwaway product so the selftest reads the neutered rule.
            prod="$copy/product"
            mkdir -p "$prod"
            cp -Rp "$GUARDS_SELFTEST_PRODUCT/." "$prod/" 2>/dev/null
            base="$prod"
            ;;
    esac
    target="$base/$file"
    if [ ! -f "$target" ]; then
        printf 'ERROR\t%s\tnames a missing file: %s\n' "$id" "$file" >"$WORK/$n.res"
        return
    fi
    local text
    text=$(<"$target")
    if [[ "$text" != *"$needle"* ]]; then
        printf 'ERROR\t%s\tthe literal is not in %s: the probe is STALE\n' "$id" "$file" >"$WORK/$n.res"
        return
    fi
    printf '%s\n' "${text//"$needle"/$replacement}" >"$target"
    local rc=0
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
running=0
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
        _probe "$n" "$src" "$id" "$file" "$needle" "$expect" "$replacement" "$parts" &
        running=$((running + 1))
        # bash 3.2 has no `wait -n`: a full batch is awaited before the next starts.
        if [ "$running" -ge "$JOBS" ]; then
            wait
            running=0
        fi
    done <"$src"
done
wait

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
