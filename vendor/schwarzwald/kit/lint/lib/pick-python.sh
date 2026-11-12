#!/usr/bin/env bash
# Sourced: pick_py sets PY to the newest python 3.11 or newer on PATH (the shared lints need
# tomllib), checked newest first, plain python3 last. Returns 1 and leaves PY empty when none is.
#
#   . "<part>/lint/lib/pick-python.sh" && pick_py || { echo "needs python 3.11" >&2; exit 1; }
pick_py() {
  local c
  PY=""
  for c in python3.15 python3.14 python3.13 python3.12 python3.11 python3; do
    if command -v "${c}" >/dev/null 2>&1 && "${c}" -c 'import sys; sys.exit(sys.version_info < (3, 11))' 2>/dev/null; then
      PY=${c}
      return 0
    fi
  done
  return 1
}
