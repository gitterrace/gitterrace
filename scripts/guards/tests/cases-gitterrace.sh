#!/usr/bin/env bash
# gitterrace's cases, sourced by the shared selftest after its own when it runs on gitterrace
# (GUARD_PRODUCT_CASES). gitterrace has no rules of its own: these prove the shared rules with
# gitterrace's values in schwarzwald/config/guards.env, read from the fixture's copy of it.

# REPO, EVENT, UNSET_ENV and t come from the shared selftest that sources this file.
# shellcheck disable=SC2154,SC2034
UNSET_ENV+=(-u GITTERRACE_LANE -u GITTERRACE_BUILD_LOCK)

echo
echo "gitterrace values -- vendored, golden and generated files, and what feeds them"
t BLOCK protected_files "$REPO" edit '{"file_path":"'"$REPO"'/crates/gitterrace-web/static/vendor/mermaid.min.js"}' "vendored mermaid"
t BLOCK protected_files "$REPO" write '{"file_path":"'"$REPO"'/crates/gitterrace-web/tests/golden/revisions.txt"}' "a golden file, written only via TEST_BLESS"
t BLOCK protected_files "$REPO" write '{"file_path":"'"$REPO"'/docs/api.md"}' "the generated api doc"
t BLOCK protected_files "$REPO" write '{"file_path":"'"$REPO"'/docs/configuration.md"}' "the generated configuration doc"
t BLOCK protected_files "$REPO" edit '{"file_path":"'"$REPO"'/vendor/schwarzwald/kit/tokens/theme-tokens.json"}' "the synced theme copy"
t ALLOW protected_files "$REPO" edit '{"file_path":"'"$REPO"'/crates/gitterrace-web/static/app.js"}' "a static file that is not vendored"
t ALLOW protected_files "$REPO" edit '{"file_path":"'"$REPO"'/docs/architecture.md"}' "a hand-written doc"
EVENT=post_tool
t ADVISE post_check_generated "$REPO" edit '{"file_path":"'"$REPO"'/xtask/src/api_md.rs"}' "an xtask generator edit reminds about docs-gen"
t ADVISE post_check_generated "$REPO" edit '{"file_path":"'"$REPO"'/scripts/land-check.sh"}' "a script header feeds the scripts table"
t ALLOW post_check_generated "$REPO" edit '{"file_path":"'"$REPO"'/crates/gitterrace-web/src/lib.rs"}' "a crate source edit says nothing"
EVENT=pre_tool
# The app names (GUARD_APP_RE) matter where a keystroke is addressed by process name.
t BLOCK live_desktop_input "$REPO" macos_computer_use '{"action":"run_applescript","script":"tell application \"System Events\" to tell process \"Gitterrace\" to keystroke \"z\""}' "a keystroke to the app"
t BLOCK live_desktop_input "$REPO" macos_computer_use '{"action":"run_applescript","script":"tell application \"System Events\" to tell process \"git-terrace\" to keystroke \"z\""}' "a keystroke to the binary"
t ALLOW workspace_test "$REPO" bash '{"command":"cargo test -p gitterrace-git"}' "scoped to one crate"
