#!/usr/bin/env bash
# Every external linter and every shared lint, one PASS or FAIL line each. A missing tool is a
# FAIL with its install line, never a skip. Run from the product's gate (`make lint`).
#
#   bash <this part>/run.sh [--repo DIR]
#
# The linters read their own config at the repository root (_typos.toml, .vale.ini,
# .gitleaks.toml, .markdownlint-cli2.jsonc, ruff.toml), and only run when that config or the
# files they lint exist. Product values come from config/schwarzwald/lint.toml [run]:
# steps (extra "name|tool|install hint|command" checks), shell_exclude, shellcheck_exclude,
# shfmt_indent and markdownlint (the pinned markdownlint-cli2). The part is found from this
# file's own location, so the copy in a product runs the copy beside it.
set -euo pipefail

part="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
repo=""
while (($#)); do
  case $1 in
  --repo)
    repo=${2:?--repo needs a directory}
    shift 2
    ;;
  *)
    echo "run.sh: unknown argument $1" >&2
    exit 2
    ;;
  esac
done
cd "${repo:-$(git rev-parse --show-toplevel)}"

# The shared lints need tomllib: the first python3 that is 3.11 or newer.
py=python3
for c in python3 python3.15 python3.14 python3.13 python3.12 python3.11; do
  if command -v "${c}" >/dev/null 2>&1 && "${c}" -c 'import sys; sys.exit(sys.version_info < (3, 11))'; then
    py=${c}
    break
  fi
done

# [run] settings as lines "key<TAB>value" (lists joined by the unit separator), through tomllib.
cfg=$(
  "${py}" - <<'P'
import sys
from pathlib import Path
try:
    import tomllib
except ImportError:
    sys.exit("run.sh: python 3.11 or newer is needed for tomllib")
run = {}
p = Path("config/schwarzwald/lint.toml")
if p.exists():
    run = tomllib.loads(p.read_text(encoding="utf-8")).get("run", {})
d = {"steps": [], "shell_exclude": [], "shellcheck_exclude": ["SC2250", "SC2292", "SC2310", "SC2312"],
     "shfmt_indent": 2, "markdownlint": "", "markdown_exclude": []}
d.update(run)
for k, v in d.items():
    print(k + "\t" + ("\x1f".join(map(str, v)) if isinstance(v, list) else str(v)))
P
)
get() { printf '%s\n' "${cfg}" | awk -F'\t' -v k="$1" '$1 == k { print $2 }'; }
IFS=$'\x1f' read -r -a steps <<<"$(get steps)"
IFS=$'\x1f' read -r -a shell_exclude <<<"$(get shell_exclude)"
sc_exclude=$(get shellcheck_exclude)
sc_exclude=${sc_exclude//$'\x1f'/,}
indent=$(get shfmt_indent)
mdl=$(get markdownlint)

fails=0
# run NAME TOOL INSTALL CMD...: run CMD when TOOL exists, report the verdict.
run() {
  local name=$1 tool=$2 install=$3 out
  shift 3
  if ! command -v "${tool}" >/dev/null 2>&1; then
    echo "FAIL ${name}: ${tool} is not installed (${install})"
    fails=$((fails + 1))
    return
  fi
  if out=$("$@" 2>&1); then
    echo "PASS ${name}"
  else
    printf '%s\n' "${out}"
    echo "FAIL ${name}"
    fails=$((fails + 1))
  fi
}

# Tracked shell scripts, minus the [run] shell_exclude pathspecs (the synced copy carries its
# own checks, so a product lists it there).
sh_files() {
  local spec=()
  local e
  for e in ${shell_exclude[@]+"${shell_exclude[@]}"}; do [[ -n ${e} ]] && spec+=(":!:${e}"); done
  git ls-files -z -- '*.sh' ${spec[@]+"${spec[@]}"} | xargs -0 "$@"
}
# Vale needs its packages in styles/ (git-ignored), fetched once.
vale_run() {
  [[ -d styles ]] && [[ -z $(find styles -mindepth 1 -maxdepth 1 -type d ! -name config 2>/dev/null | head -1) ]] && { vale sync >/dev/null || return 1; }
  git ls-files -z -- '*.md' | xargs -0 vale --minAlertLevel=error
}

[[ -f _typos.toml || -f .typos.toml ]] && run typos typos "brew install typos-cli" typos
[[ -f .vale.ini ]] && run vale vale "brew install vale" vale_run
[[ -f .gitleaks.toml ]] && run gitleaks gitleaks "brew install gitleaks" gitleaks detect --no-banner -c .gitleaks.toml
[[ -n ${mdl} ]] && run markdownlint npx "install node, then npx fetches markdownlint-cli2" npx --yes "markdownlint-cli2@${mdl}"
if [[ -n $(git ls-files -- '*.sh' | head -1) ]]; then
  run shellcheck shellcheck "brew install shellcheck" sh_files shellcheck -o all -e "${sc_exclude}"
  run shfmt shfmt "brew install shfmt" sh_files shfmt -d -i "${indent}"
fi
if [[ -f ruff.toml || -f .ruff.toml || -f pyproject.toml ]] && [[ -n $(git ls-files -- '*.py' | head -1) ]]; then
  run ruff-check ruff "brew install ruff" ruff check .
  run ruff-format ruff "brew install ruff" ruff format --check .
fi
[[ -d .github/workflows ]] && run actionlint actionlint "brew install actionlint" actionlint

# Drift: the tool configs keep to the shared bases. drift.py needs tomllib, so it runs under the
# newest python 3.11 or newer on PATH (not the plain python3, which may be 3.9). None is a FAIL.
drift_py=""
for c in python3.15 python3.14 python3.13 python3.12 python3.11 python3; do
  if command -v "${c}" >/dev/null 2>&1 && "${c}" -c 'import sys; sys.exit(sys.version_info < (3, 11))'; then
    drift_py=${c}
    break
  fi
done
if [[ -f ${part}/configs/drift.py ]]; then
  if [[ -n ${drift_py} ]]; then
    run drift "${drift_py}" "install python 3.11" "${drift_py}" "${part}/configs/drift.py" --repo .
  else
    echo "FAIL drift: no python 3.11 or newer on PATH (install python3.13)"
    fails=$((fails + 1))
  fi
fi

# The shared lints, each proved on its red case by red.py.
lint="${part}/lint"
for rule in internal-refs personal-data own-shas actions-pin dash-standins make-targets file-length script-orphans docs-a11y changelog; do
  [[ -f ${lint}/${rule}.py ]] && run "${rule}" "${py}" "install python 3.11" "${py}" "${lint}/${rule}.py" --repo .
done
[[ -f ${lint}/red.py ]] && run lint-red "${py}" "install python 3.11" "${py}" "${lint}/red.py" --repo .
[[ -f ${lint}/run_test.sh ]] && run run-drift-step bash "install bash" bash "${lint}/run_test.sh"
[[ -f ${part}/hooks/commit-msg_test.sh ]] && run commit-msg-hook bash "install bash" bash "${part}/hooks/commit-msg_test.sh"
for t in author-guard stale-index markdown-staged; do
  [[ -f ${part}/hooks/${t}_test.sh ]] && run "${t}-hook" bash "install bash" bash "${part}/hooks/${t}_test.sh"
done
[[ -f ${part}/guards/tests/suite.sh ]] && run guards bash "install bash" bash "${part}/guards/tests/suite.sh"

# Product steps: "name|tool|install hint|command". A step's command sees PY, the python 3.11 or
# newer this runner picked, so `$PY scripts/x.py` never falls back to an older plain python3.
export PY=${py}
for s in ${steps[@]+"${steps[@]}"}; do
  [[ -n ${s} ]] || continue
  IFS='|' read -r name tool hint cmd <<<"${s}"
  run "${name}" "${tool}" "${hint}" bash -c "${cmd}"
done

[[ ${fails} -eq 0 ]] || {
  echo "lint: ${fails} failed"
  exit 1
}
echo "lint: all passed"
