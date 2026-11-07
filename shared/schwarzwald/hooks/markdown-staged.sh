#!/usr/bin/env bash
#
# A commit may not add markdown that fails the markdown rules. The full rule runs in the gate,
# so an author could commit a long line or a body H1 and never meet it. This lints only the
# staged .md files with the same pinned markdownlint-cli2 and config, so the cost is one file
# list, not the tree. Exits 0 when nothing markdown is staged, when [run] markdownlint is empty
# or when npx is missing (the gate still catches it then).
#
# From config/schwarzwald/lint.toml: [run] markdownlint (the pinned version) and
# markdown_exclude (git pathspecs, keep them equal to the gate's markdownlint ignores).
set -uo pipefail

f=config/schwarzwald/lint.toml
# run_key KEY: the raw value of a [run] key, builtins and awk only. Arrays are one line.
run_key() {
  [[ -f ${f} ]] || return 0
  awk -v k="$1" '/^\[/ { on = ($0 == "[run]"); next }
    on && $1 == k && $2 == "=" { sub(/^[^=]*=[ \t]*/, ""); print; exit }' "${f}"
}
version=$(run_key markdownlint)
version=${version//\"/}
[[ -n ${version} ]] || exit 0
excl=()
list=$(run_key markdown_exclude)
while [[ ${list} =~ \"([^\"]+)\" ]]; do
  excl+=(":!:${BASH_REMATCH[1]}")
  list=${list#*"${BASH_REMATCH[0]}"}
done

staged=()
while IFS= read -r file; do
  [[ -f ${file} ]] && staged+=("${file}")
done < <(git diff --cached --name-only --diff-filter=ACMR -- '*.md' "${excl[@]}")
[[ ${#staged[@]} -eq 0 ]] && exit 0
command -v npx >/dev/null 2>&1 || exit 0

out=$(npx --yes "markdownlint-cli2@${version}" "${staged[@]}" 2>&1)
rc=$?
[[ ${rc} -eq 0 ]] && exit 0

printf '%s\n' "${out}" | grep -E '^[^ ]+\.md:[0-9]+' | head -20 >&2
{
  echo ""
  echo "[CONV-MARKDOWN] staged markdown fails the markdown rules."
  echo "  Fix the lines above, then stage the files again."
} >&2
exit 1
