#!/usr/bin/env bash
# Print the release date of a version: the committer date, in UTC, of the commit that set it (the
# bump). One source for the changelog section date (changelog-gen.sh, lint/changelog.py) and for
# SOURCE_DATE_EPOCH in a reproducible build. The author date and the clock are never read.
#
#   release-date.sh [--repo DIR] [--date|--epoch] X.Y.Z | TAG
#
# Output, two lines unless a flag picks one:
#   2026-03-05
#   SOURCE_DATE_EPOCH=1772755170
# X.Y.Z gets the tag prefix from schwarzwald/config/lint.toml [changelog] tag_prefix (default v).
# The bump is the oldest commit reachable from the tag (from HEAD while the version is not tagged
# yet) that adds a line `version = "X.Y.Z"` (or `"version": "X.Y.Z"`) to a file other than a lock
# file. With no such commit the tagged commit itself is used, and HEAD for an untagged version,
# with a note on stderr. A TAG that does not exist is exit 1, a bad flag or no argument exit 2.
set -euo pipefail

repo=""
only=""
name=""
while (($#)); do
  case $1 in
  --repo)
    repo=${2:?--repo needs a directory}
    shift 2
    ;;
  --date | --epoch)
    only=$1
    shift
    ;;
  -*)
    echo "release-date: unknown flag $1" >&2
    exit 2
    ;;
  *)
    name=$1
    shift
    ;;
  esac
done
[[ -n ${name} ]] || {
  echo "usage: release-date.sh [--repo DIR] [--date|--epoch] X.Y.Z | TAG" >&2
  exit 2
}
repo=${repo:-$(git rev-parse --show-toplevel)}
cd "${repo}"

prefix=v
f=schwarzwald/config/lint.toml
if [[ -f ${f} ]]; then
  p=$(awk '/^\[/ { on = ($0 == "[changelog]"); next }
    on && $1 == "tag_prefix" && $2 == "=" { gsub(/^[^"]*"|"[^"]*$/, ""); print; exit }' "${f}")
  prefix=${p:-v}
fi
if [[ ${name} =~ ^[0-9]+(\.[0-9]+)*$ ]]; then
  version=${name}
  tag=${prefix}${name}
else
  tag=${name}
  version=${name#"${prefix}"}
fi
if git rev-parse -q --verify "refs/tags/${tag}^{commit}" >/dev/null; then
  ref=refs/tags/${tag}^{commit}
  fallback="the tagged commit"
elif [[ ${name} =~ ^[0-9]+(\.[0-9]+)*$ ]]; then
  ref=HEAD
  fallback="HEAD"
else
  echo "release-date: no tag ${tag} here (fetch the tags)" >&2
  exit 1
fi
re="^[[:space:]]*\"?version\"?[[:space:]]*[:=][[:space:]]*\"${version//./\\.}\""
bump=$(git log -G"${re}" --format=%H --reverse "${ref}" -- . ':(exclude)*.lock' ':(exclude)*lock.json' | sed -n 1p)
if [[ -z ${bump} ]]; then
  echo "release-date: no commit sets version ${version}, using ${fallback}" >&2
  bump=${ref}
fi
epoch=$(git log -1 --format=%ct "${bump}")
if day=$(date -u -d "@${epoch}" +%Y-%m-%d 2>/dev/null); then :; else day=$(date -u -r "${epoch}" +%Y-%m-%d); fi
case ${only} in
--date) echo "${day}" ;;
--epoch) echo "${epoch}" ;;
*)
  echo "${day}"
  echo "SOURCE_DATE_EPOCH=${epoch}"
  ;;
esac
