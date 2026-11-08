#!/usr/bin/env bash
# Refuse a commit that puts a path back to content HEAD's recent commits replaced or deleted.
# A private index read before those commits still carries the old blobs, and committing it
# reverts them silently. <prefix>ALLOW_RESTORE=1 restores deliberately, <prefix>STALE_DEPTH sets
# how many commits to look back (default 20). The prefix is config/schwarzwald/lint.toml
# [commit] allow_prefix.
set -u

prefix=SCHWARZWALD_
f=config/schwarzwald/lint.toml
if [[ -f ${f} ]]; then
  v=$(awk '/^\[/ { on = ($0 == "[commit]"); next }
    on && $1 == "allow_prefix" && $2 == "=" { gsub(/^[^"]*"|"[^"]*$/, ""); print; exit }' "${f}")
  prefix=${v:-${prefix}}
fi
# Only a shell name: bash evaluates an array subscript in ${!x}, so any other value runs code.
[[ ${prefix} =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]] || prefix=SCHWARZWALD_
allow="${prefix}ALLOW_RESTORE"
depth="${prefix}STALE_DEPTH"
[[ ${!allow:-0} == 1 ]] && exit 0
git rev-parse -q --verify HEAD >/dev/null || exit 0
depth=${!depth:-20}
hits=$(
  {
    git log -n "${depth}" --no-merges --no-renames --format= --raw --no-abbrev HEAD |
      awk '$1 ~ /^:/ && $3 !~ /^0+$/ { split($0, t, "\t"); print "old", $3, t[2] }'
    git diff --cached --no-renames --raw --no-abbrev HEAD |
      awk '$4 !~ /^0+$/ { split($0, t, "\t"); print "new", $4, t[2] }'
  } | awk '$1 == "old" { seen[$2 " " $3] = 1; next } ($2 " " $3) in seen { print "  " $3 }'
)
[[ -z ${hits} ]] && exit 0
{
  printf 'pre-commit: refused. These staged paths go back to content a recent HEAD commit replaced:\n%s\n' "${hits}"
  printf '            A private index read before those commits? Rebuild it from HEAD and stage again.\n'
  printf '            A deliberate restore: %s=1.\n' "${allow}"
} >&2
exit 1
