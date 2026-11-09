#!/usr/bin/env bash
#
# A commit may not add markdown that fails the markdown rules. The full rule runs in the gate,
# so an author could commit a long line or a body H1 and never meet it. This lints only the
# staged .md files with the same pinned markdownlint-cli2 and config, so the cost is one file
# list, not the tree. Exits 0 when nothing markdown is staged, when [run] markdownlint is empty
# or when npx is missing outside CI (the gate still catches it then). Under CI=true no npx fails.
#
# From schwarzwald/config/lint.toml: [run] markdownlint (the pinned version).
set -uo pipefail

f=schwarzwald/config/lint.toml
# run_key KEY: the raw value of a [run] key, builtins and awk only. Arrays are one line.
run_key() {
  [[ -f ${f} ]] || return 0
  awk -v k="$1" '/^\[/ { on = ($0 == "[run]"); next }
    on && $1 == k && $2 == "=" { sub(/^[^=]*=[ \t]*/, ""); print; exit }' "${f}"
}
version=$(run_key markdownlint)
version=${version//\"/}
[[ -n ${version} ]] || exit 0
# npx --yes installs any npm spec (a URL, npm:other) with no prompt: only a version runs.
if [[ ! ${version} =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "[CONV-MARKDOWN] [run] markdownlint must be a version like 0.18.1, not ${version}" >&2
  exit 1
fi

staged=()
while IFS= read -r file; do
  # After -- and as ./name: markdownlint-cli2 reads a leading # or ! as a negated glob.
  [[ -f ${file} ]] && staged+=("./${file}")
done < <(git diff --cached --name-only --diff-filter=ACMR -- '*.md')
[[ ${#staged[@]} -eq 0 ]] && exit 0
if ! command -v npx >/dev/null 2>&1; then
  # The gate still runs it on a laptop; CI has no later gate, so a skip there is a refusal.
  echo "[CONV-MARKDOWN] npx is missing: staged markdown is not linted (install node)." >&2
  [[ ${CI:-} == true ]] && exit 1
  exit 0
fi

out=$(npx --yes "markdownlint-cli2@${version}" -- "${staged[@]}" 2>&1)
rc=$?
if [[ ${out} =~ Linting:\ 0\ file ]]; then
  echo "[CONV-MARKDOWN] markdownlint-cli2 linted 0 of ${#staged[@]} staged file(s): refused." >&2
  exit 1
fi
[[ ${rc} -eq 0 ]] && exit 0

printf '%s\n' "${out}" | grep -E '^[^ ]+\.md:[0-9]+' | head -20 >&2
{
  echo ""
  echo "[CONV-MARKDOWN] staged markdown fails the markdown rules."
  echo "  Fix the lines above, then stage the files again."
} >&2
exit 1
