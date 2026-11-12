#!/usr/bin/env bash
#
# Pre-commit arm: refuse a commit whose AUTHOR is not the identity this checkout is configured
# with. A test run with the primary checkout as cwd once committed under a forged identity
# (`git -c user.email=...` or GIT_AUTHOR_*), which no real session sets, so an author that differs
# from the configured one means a script, not a session, is committing.
#
# The configured email is read with the command-line and environment overrides removed, because
# `git -c` reaches hooks through GIT_CONFIG_PARAMETERS and would otherwise make the forged
# identity look configured. Nothing configured (a fresh clone) means nothing to compare: pass.
#
# A deliberate foreign author (a patch from someone else): <prefix>ALLOW_FOREIGN_AUTHOR=1, where
# the prefix is schwarzwald/config/lint.toml [commit] allow_prefix.
set -u

# prefix: [commit] allow_prefix of the product's lint.toml, builtins and awk only.
prefix=SCHWARZWALD_
f=schwarzwald/config/lint.toml
if [[ -f ${f} ]]; then
  v=$(awk '/^\[/ { on = ($0 == "[commit]"); next }
    on && $1 == "allow_prefix" && $2 == "=" { gsub(/^[^"]*"|"[^"]*$/, ""); print; exit }' "${f}")
  prefix=${v:-${prefix}}
fi
# Only a shell name: bash evaluates an array subscript in ${!x}, so any other value runs code.
[[ ${prefix} =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]] || prefix=SCHWARZWALD_
override="${prefix}ALLOW_FOREIGN_AUTHOR"
[[ ${!override:-0} == 1 ]] && exit 0

configured=$(env -u GIT_CONFIG_PARAMETERS -u GIT_CONFIG_COUNT -u GIT_AUTHOR_EMAIL -u GIT_COMMITTER_EMAIL \
  git config user.email 2>/dev/null || true)
author=$(git var GIT_AUTHOR_IDENT 2>/dev/null | awk -F'[<>]' '{print $2}')

if [[ -n ${configured} && -n ${author} && ${configured} != "${author}" ]]; then
  {
    printf 'pre-commit: refused. The commit author is <%s> but this checkout is configured as <%s>.\n' \
      "${author}" "${configured}"
    printf '            A script that commits with a forged identity is running in this checkout\n'
    printf '            (a test that belongs in a throwaway repo: tools/test-sandbox.sh).\n'
    printf '            A deliberate foreign author: %s=1.\n' "${override}"
  } >&2
  exit 1
fi
exit 0
