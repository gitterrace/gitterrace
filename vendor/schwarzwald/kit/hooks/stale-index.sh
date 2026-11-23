#!/usr/bin/env bash
# Refuse a commit from a stale private index: one read-tree'd from an older HEAD that would
# silently undo work committed since. The index's base is the recent first-parent commit whose
# tree is closest to the staged tree (newest wins a tie, so a fresh index's base is HEAD). On each
# path that changed between base and HEAD and that the index stages differently from HEAD:
#   - HEAD deleted it and the index still has it: a re-add of a cut file.
#   - HEAD added it and the index lacks it: a drop of a file added since.
#   - HEAD changed it and the staged blob lacks that change (merging base -> HEAD onto the
#     staged blob changes it or conflicts): a revert of lines changed since.
#   - The staged blob equals an older blob of that same path from the last STALE_DEPTH commits
#     while HEAD has a different one: a single-path revert no base explains (the history sweep).
# The kit's commit-msg runs it with the message file as $1, so a `restore:` line in the message
# restores deliberately. Without an argument (a product's own pre-commit) only
# <prefix>ALLOW_RESTORE=1 does. <prefix>STALE_DEPTH sets how
# many commits to look back (default 20). The prefix is schwarzwald/config/lint.toml [commit]
# allow_prefix. Bash 3.2-safe. The line check needs git 2.43 (merge-file --object-id); an older
# git checks whole-file reverts only.
set -u

prefix=SCHWARZWALD_
f=schwarzwald/config/lint.toml
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
if [[ -n ${1:-} && -f ${1} ]] && grep -q '^restore:' "${1}"; then exit 0; fi
depth=${!depth:-20}
[[ ${depth} =~ ^[0-9]+$ ]] || depth=20

# Two git reads, about 40 ms each on a loaded machine: the staged diff and one first-parent log.
staged=$(git diff --cached --no-renames --raw --no-abbrev HEAD 2>/dev/null) || exit 0
[[ -z ${staged} ]] && exit 0

# The awk prints the base, then a "<base blob> <head blob> <index blob> <path>" line for each path
# changed between the base and HEAD that the index stages differently from HEAD, then a "H <path>"
# line for each staged blob a recent commit replaced or deleted at the same path.
# The base is the recent commit whose tree is closest to the index. Walking newest first, a
# commit's raw lines turn the state from its tree into its parent's, and d counts the paths where
# that tree and the index differ. An older base must explain at least two staged paths HEAD does
# not: undoing one path is an ordinary edit as often as a stale index, and the H lines cover it.
out=$(
  {
    printf '%s\n' "${staged}" | awk '{ split($0, t, "\t"); print "S", $4, t[2] }'
    git log --first-parent -n "${depth}" --no-renames --format='C %H' --raw --no-abbrev HEAD |
      awk '/^C / { print; next } /^:/ { split($0, t, "\t"); print "R", $3, $4, t[2] }'
  } | awk '
    function ne(a, b) { return a != b ? 1 : 0 }
    $1 == "S" { idx[$3] = $2; n++; d++; next }
    $1 == "C" { c++; sha[c] = $2; if (best == "" || d < best) { best = d; bc = c }; next }
    $1 == "R" {
      p = $4; r++; rc[r] = c; rp[r] = p; rs[r] = $2
      if (!(p in hb)) { hb[p] = $3; cur[p] = $3; if (!(p in idx)) idx[p] = $3 }
      if ($2 !~ /^0+$/ && ne(idx[p], hb[p]) && idx[p] == $2) hist[p] = 1
      d += ne($2, idx[p]) - ne(cur[p], idx[p]); cur[p] = $2
    }
    END {
      if (c == 0) exit
      if (n - best < 2) bc = 1
      print sha[bc]
      for (i = 1; i <= r && rc[i] < bc; i++) at[rp[i]] = rs[i]
      for (p in at) if (at[p] != hb[p] && idx[p] != hb[p]) { print at[p], hb[p], idx[p], p; done[p] = 1 }
      for (p in hist) if (!(p in done)) print "H - -", p
    }'
)
[[ -z ${out} ]] && exit 0
base=${out%%$'\n'*}
hits=''
while read -r b h i p; do
  [[ -z ${p} ]] && continue
  why=''
  if [[ ${b} == H ]]; then
    why='puts back content a recent commit replaced'
  elif [[ ${h} =~ ^0+$ && ! ${i} =~ ^0+$ ]]; then
    why='re-adds a file deleted since'
  elif [[ ${b} =~ ^0+$ && ${i} =~ ^0+$ ]]; then
    why='drops a file added since'
  elif [[ ${i} == "${b}" ]]; then
    why='puts back the content before a change made since'
  elif [[ ! ${b} =~ ^0+$ && ! ${h} =~ ^0+$ && ! ${i} =~ ^0+$ ]]; then
    # Merge base -> HEAD onto the staged blob: a clean merge that changes nothing means the staged
    # blob already carries HEAD's change. A conflict or a changed result means it undoes some.
    m=$(git merge-file --object-id -q -p "${i}" "${b}" "${h}" 2>/dev/null) || m=x
    [[ ${m} == x || ${m}x != "$(git cat-file blob "${i}")x" ]] && why='reverts lines changed since'
  fi
  [[ -n ${why} ]] && hits="${hits}  ${p}: ${why}
"
done <<<"${out#*$'\n'}"
[[ -z ${hits} ]] && exit 0
{
  printf 'stale-index: refused. This index (closest to %s) undoes work HEAD has since:\n' "${base:0:10}"
  printf '%s' "${hits}" | sort -u
  printf '             Rebuild it from HEAD (git read-tree HEAD) and stage again.\n'
  printf '             A deliberate restore: a `restore:` line in the message, or %s=1.\n' "${allow}"
} >&2
exit 1
