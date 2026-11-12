#!/usr/bin/env bash
#
# The guards' gate in a product: the selftest against the product's own config, rules and
# cases, the adapters' agreement, and the hook wiring. Run from the product root:
#
#   bash vendor/schwarzwald/kit/guards/tests/suite.sh
#
# The red runner (red.sh) runs the selftest once per probe and is run on its own, after a
# change to the guards or to the product's config.
. "$(dirname "${BASH_SOURCE[0]}")/prelude.sh"

violations=0
root="$PWD"

if GUARDS_OUT="$(GUARDS_SELFTEST_PRODUCT="$root" bash "$GUARDS_TESTS/selftest.sh" 2>&1)"; then
    emit_pass guards "$(printf '%s' "$GUARDS_OUT" | grep -c '  ok ') guard expectations met"
else
    emit_fail guards "a guard stopped matching, or matches something it should not."
    echo "$GUARDS_OUT"
    violations=$((violations + 1))
fi

if ADAPTERS_OUT="$(GUARDS_SELFTEST_PRODUCT="$root" bash "$GUARDS_TESTS/adapters.sh" 2>&1)"; then
    emit_pass guard-adapters "${ADAPTERS_OUT#PASS  }"
else
    emit_fail guard-adapters "the harness adapters disagree."
    echo "$ADAPTERS_OUT"
    violations=$((violations + 1))
fi

if WIRING_OUT="$(bash "$GUARDS_TESTS/wiring.sh" "$root" 2>&1)"; then
    printf '%s\n' "$WIRING_OUT"
else
    echo "$WIRING_OUT"
    violations=$((violations + 1))
fi

gate_exit "$violations"
