#!/usr/bin/env bash
# Land one commit from a work clone, then sweep that clone away when it is clean and landed.
#
#   scripts/dev/land-run.sh [land.sh options] <repo> <clone-dir> <sha>
#
# The repo-owned entry around vendor/schwarzwald/kit/tools/land.sh, which lands but never
# removes a clone. Arguments pass through unchanged. Only when land.sh exits 0 does
# scripts/dev/sweep-run.sh look at <clone-dir>, and it keeps the clone when anything is
# unlanded, dirty or in use, so the sweep never fails the landing.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
[ $# -ge 3 ] || {
  echo "usage: scripts/dev/land-run.sh [land.sh options] <repo> <clone-dir> <sha>" >&2
  exit 2
}
clone=${*: -2:1}
repo=$(cd "${*: -3:1}" && pwd)
bash "$here/../../vendor/schwarzwald/kit/tools/land.sh" "$@"
# The sweep is the repo's own copy and runs from the repo, so neither this script nor its cwd
# keeps the clone busy or vanishes under it. A shell the caller left inside the clone still
# keeps it, which the sweep names.
cd "$repo"
bash scripts/dev/sweep-run.sh "$clone" || true
