#!/usr/bin/env bash
#
# The tool-use guards still guard what they claim to: every rule, both directions, the
# fail-closed fallback and the jcode-pre front door. One implementation, so each case
# asserts a direction a guard can fail, rather than parity between two dispatchers.
#
#   bash schwarzwald/kit/guards/tests/selftest.sh                  the example config
#   GUARDS_SELFTEST_PRODUCT=. bash schwarzwald/kit/guards/tests/selftest.sh   a product's own
#   GUARDS_SELFTEST_SRC=<copy> bash .../selftest.sh                   a neutered copy (red.sh)
_here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
. "$_here/lib.sh"
# GUARDS_SELFTEST_PARTS names the case files to run (a red probe runs the one its case is in).
# cases-bypass reads the corpus and helpers cases-fallback defines, so it never runs alone.
_parts="${GUARDS_SELFTEST_PARTS:-cases-rules cases-state cases-fallback cases-bypass}"
case " $_parts " in
    *" cases-bypass "*) case " $_parts " in *" cases-fallback "*) ;; *) _parts="cases-fallback $_parts" ;; esac ;;
esac
for _part in $_parts; do
    # shellcheck source=/dev/null
    . "$_here/$_part.sh"
done
# The product's cases for its own rules, when the suite runs against a product.
for _part in $PRODUCT_CASES; do
    # shellcheck source=/dev/null
    . "$_part"
done
echo
# A partial run (a red probe) proves one case: the coverage and the case floor are the full
# run's to check.
if [ -n "${GUARDS_SELFTEST_ONLY:-}${GUARDS_SELFTEST_PARTS:-}" ]; then
    echo "guards-selftest: partial run, $CASES expectations"
    [ "$FAILED" -eq 0 ] && exit 0
    printf '%s' "$FAIL_LOG" >&2
    exit 1
fi
# COVERAGE. Per-rule cases structurally cannot detect a MISSING rule, so the
# registry is asserted against the cases separately. Every rule registers on a line of its own
# (`guard_register <id> [event]`), so the registry is read from the files, not by loading them.
_rule_files="${ENGINE%/*}/rules/*.sh"
for _d in $PRODUCT_RULE_DIRS; do _rule_files="$_rule_files $REPO/$_d/*.sh"; done
# shellcheck disable=SC2086 # the globs expand on purpose
RULES=$(sed -n 's/^guard_register \([A-Za-z0-9_]*\).*/\1/p' $_rule_files 2>/dev/null)
real_missing=''
for r in $RULES; do
    case "$COVERED" in
        *" $r "*) ;;
        *) real_missing="$real_missing $r" ;;
    esac
done
# $RULES is a newline-separated rule-name list, same as the `for r in $RULES`
# pattern above; word-split on purpose.
# shellcheck disable=SC2086
RULE_COUNT=$(printf '%s\n' $RULES | grep -c .)
# The engine's own floor: fewer registered rules than it will run with means the registry did not
# load here, and a coverage pass over nothing proves nothing.
MIN_RULES=$(sed -n 's/^GUARD_MIN_RULES=\([0-9][0-9]*\)$/\1/p' "$ENGINE")
if [ "$RULE_COUNT" -lt "${MIN_RULES:-1}" ]; then
    echo "  FAIL coverage read $RULE_COUNT registered rules: the registry did not load, so it proves nothing"
    FAILED=1
elif [ -n "$real_missing" ]; then
    echo "  FAIL rule(s) with no test case:$real_missing"
    echo "       Add a case for each."
    FAILED=1
else
    echo "  ok   [COVER ] all $RULE_COUNT registered rules have a case"
fi

# A floor. If the case list is emptied or the loop breaks, "0 cases, 0 failures"
# is true of the empty set and reads exactly like a clean run.
MIN_CASES=80
if [ "$CASES" -lt "$MIN_CASES" ]; then
    echo "  FAIL case list collapsed to $CASES (expected at least $MIN_CASES)"
    FAILED=1
fi

echo
if [ "$FAILED" -eq 0 ]; then
    echo "guards-selftest: $CASES expectations met"
    exit 0
fi
echo "guards-selftest: FAILURES" >&2
printf '%s' "$FAIL_LOG" >&2
exit 1
