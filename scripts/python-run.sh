#!/usr/bin/env bash
# Runs a python script with the first python 3.11 or newer found (tomllib needs it).
#   bash scripts/python-run.sh SCRIPT [ARGS...]
for p in python3 python3.15 python3.14 python3.13 python3.12 python3.11; do
  if command -v "$p" >/dev/null && "$p" -c 'import tomllib' 2>/dev/null; then
    exec "$p" "$@"
  fi
done
echo "python-run.sh: needs python 3.11 or newer" >&2
exit 1
