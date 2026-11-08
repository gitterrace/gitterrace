#!/usr/bin/env bash
# reference-transaction: main reads in the order it landed. Every commit an update brings onto
# main carries one date (author = committer) no older than its parent's. This hook sees every
# update to main, fast-forward merges included, which a message step never sees. A train
# fast-forwarded by hand keeps each lane's authoring time, and `git log` then reads out of order.
# <prefix>ALLOW_OLD_DATES=1 lets a deliberate history import through. The prefix is
# config/schwarzwald/lint.toml [commit] allow_prefix, the branch [commit] main (default main).
set -u
[[ ${1:-} == prepared ]] || exit 0
prefix=SCHWARZWALD_ main=main
f=config/schwarzwald/lint.toml
if [[ -f ${f} ]]; then
  read_key() {
    awk -v k="$1" '/^\[/ { on = ($0 == "[commit]"); next }
      on && $1 == k && $2 == "=" { gsub(/^[^"]*"|"[^"]*$/, ""); print; exit }' "${f}"
  }
  v=$(read_key allow_prefix)
  prefix=${v:-${prefix}}
  v=$(read_key main)
  main=${v:-${main}}
fi
# Only a shell name: bash evaluates an array subscript in ${!x}, so any other value runs code.
[[ ${prefix} =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]] || prefix=SCHWARZWALD_
allow="${prefix}ALLOW_OLD_DATES"
[[ ${!allow:-0} == 1 ]] && exit 0
zero=0000000000000000000000000000000000000000
status=0
while read -r old new ref; do
  [[ ${ref} == "refs/heads/${main}" && ${new} != "${zero}" ]] || continue
  # git passes zeros for `old` when the caller did not pin it: the branch's tip is the real before.
  if [[ ${old} == "${zero}" ]]; then
    old=$(git rev-parse -q --verify "${ref}" 2>/dev/null) || old=""
  fi
  [[ ${old} == "${new}" ]] && continue
  [[ $(git cat-file -t "${new}" 2>/dev/null) == commit ]] || continue
  span=${new}
  [[ -n ${old} ]] && span="${old}..${new}"
  # Fail closed: a git log that fails refuses the update instead of finding nothing.
  if ! log=$(git log --reverse --format='%h %at %ct %P' "${span}"); then
    printf 'reference-transaction: refused %s: git log %s failed, the dates cannot be checked.\n' "${ref}" "${span}" >&2
    status=1
    continue
  fi
  bad=$(printf '%s\n' "${log}" |
    while read -r h at ct parent _; do
      [[ -n ${h} ]] || continue
      pt=0
      if [[ -n ${parent} ]] && ! pt=$(git log -1 --format=%ct "${parent}"); then
        echo "${h}(parent-unreadable)"
        continue
      fi
      if [[ ${at} != "${ct}" ]] || ((ct < pt)); then echo "${h}"; fi
    done | head -5 | tr '\n' ' ')
  [[ -z ${bad} ]] && continue
  {
    printf 'reference-transaction: refused %s: %scarry an author date that is not the commit date, or a date older than the parent.\n' "${ref}" "${bad}"
    printf '            Restamp first (git rebase --reset-author-date <base>), or land through land.sh.\n'
    printf '            A deliberate history import: %s=1.\n' "${allow}"
  } >&2
  status=1
done
exit "${status}"
