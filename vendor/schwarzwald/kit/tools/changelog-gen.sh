#!/usr/bin/env bash
# Write the changelog section of the release about to be tagged.
#
#   changelog-gen.sh [--repo DIR] X.Y.Z [--print]
#
# Prepends the commits since the last tag to the changelog file with git-cliff (the product's
# cliff.toml). Run it on a clean main, commit the result with the release, then tag. With --print
# it writes the section to stdout and touches nothing. The pinned git-cliff version, the file and
# the tag prefix come from schwarzwald/config/lint.toml [changelog]; a different git-cliff may
# render differently and the changelog lint would then fail on a file nobody edited.
#
# The date in the heading is the release date from tools/release-date.sh (the committer date, in
# UTC, of the commit that set the version), never today's. Bump the version first and commit it,
# then run this: with no such commit yet the date is HEAD's committer date and the lint fails
# after the bump if that day differs.
set -euo pipefail

repo=""
args=()
while (($#)); do
  case $1 in
  --repo)
    repo=${2:?--repo needs a directory}
    shift 2
    ;;
  *)
    args+=("$1")
    shift
    ;;
  esac
done
repo=${repo:-$(git rev-parse --show-toplevel)}
cd "${repo}"

version=${args[0]:?usage: changelog-gen.sh [--repo DIR] X.Y.Z [--print]}
mode=${args[1]:-}

# conf KEY DEFAULT: a string value of [changelog] in the product's lint.toml, builtins and awk only.
conf() {
  local f=schwarzwald/config/lint.toml v=""
  [[ -f ${f} ]] && v=$(awk -v k="$1" '
    /^\[/ { on = ($0 == "[changelog]"); next }
    on && $1 == k && $2 == "=" { gsub(/^[^"]*"|"[^"]*$/, ""); print; exit }' "${f}")
  printf '%s' "${v:-$2}"
}
want=$(conf git_cliff "")
file=$(conf file CHANGELOG.md)
prefix=$(conf tag_prefix v)

[[ -n ${want} ]] || {
  echo "changelog-gen: [changelog] git_cliff is not set in schwarzwald/config/lint.toml" >&2
  exit 1
}
[[ ${version} =~ ^[0-9]+(\.[0-9]+)*$ ]] || {
  echo "bad version '${version}', expected X.Y.Z" >&2
  exit 1
}
tag=${prefix}${version}

command -v git-cliff >/dev/null || {
  echo "git-cliff ${want} is not installed (brew install git-cliff, or cargo install git-cliff --version ${want} --locked)" >&2
  exit 1
}
have=$(git-cliff --version | awk '{print $2}')
[[ ${have} == "${want}" ]] || {
  echo "git-cliff ${have} found, ${want} expected (cargo install git-cliff --version ${want} --locked)" >&2
  exit 1
}
if git rev-parse -q --verify "refs/tags/${tag}" >/dev/null; then
  echo "tag ${tag} already exists" >&2
  exit 1
fi
day=$(bash "$(dirname "${BASH_SOURCE[0]}")/release-date.sh" --repo "${repo}" --date "${version}")
# git-cliff stamps an unreleased --tag section with the clock: put the release date in its place.
fix_date() { awk -v v="## [${version}] - " -v d="${day}" \
  '!done && index($0, v) == 1 { print v d; done = 1; next } { print }'; }
if [[ ${mode} == --print ]]; then
  git-cliff --unreleased --tag "${tag}" --strip header | fix_date
  exit 0
fi
if grep -q "^## \[${version}\]" "${file}"; then
  echo "${file} already has a section for ${version}" >&2
  exit 1
fi
git-cliff --unreleased --tag "${tag}" --prepend "${file}"
fixed=$(mktemp)
fix_date <"${file}" >"${fixed}"
cat "${fixed}" >"${file}"
rm -f "${fixed}"
