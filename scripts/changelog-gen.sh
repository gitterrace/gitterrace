#!/bin/sh
# Write the changelog section of the release about to be tagged: make changelog VERSION=x.y.z
# runs this. It prepends the commits since the last tag to CHANGELOG.md with git-cliff
# (cliff.toml). Run it on a clean main, commit the result with the release, then tag.
# With --print it writes the section to stdout and touches nothing.
set -eu
# The git-cliff the output was checked with. A different version may render differently, and
# the docs-check comparison would then fail on a file nobody edited.
want=2.14.2
cd "$(dirname "$0")/.."

version=${1:?usage: changelog-gen.sh X.Y.Z [--print]}
mode=${2:-}
case $version in
*[!0-9.]* | '' | .* | *. | *..*)
  echo "bad version '$version', expected X.Y.Z" >&2
  exit 1
  ;;
*) ;;
esac
tag=v$version

command -v git-cliff >/dev/null || {
  echo "git-cliff $want is not installed (brew install git-cliff, or cargo install git-cliff --version $want --locked)" >&2
  exit 1
}
have=$(git-cliff --version | awk '{print $2}')
[ "$have" = "$want" ] || {
  echo "git-cliff $have found, $want expected (cargo install git-cliff --version $want --locked)" >&2
  exit 1
}
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  echo "tag $tag already exists" >&2
  exit 1
fi
if [ "$mode" = --print ]; then
  git-cliff --unreleased --tag "$tag" --strip header
  exit 0
fi
if grep -q "^## \[$version\]" CHANGELOG.md; then
  echo "CHANGELOG.md already has a section for $version" >&2
  exit 1
fi
git-cliff --unreleased --tag "$tag" --prepend CHANGELOG.md
