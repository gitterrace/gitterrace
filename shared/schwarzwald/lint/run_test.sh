#!/usr/bin/env bash
# run.sh runs configs/drift.py under the newest python 3.11 or newer on PATH. Stubs named like
# every candidate record which one was asked to run drift.py and pass everything else on. Only
# the part of run.sh before the shared lints runs, on a stub part, so the test stays fast.
#
#   bash lint/run_test.sh
set -u
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
. "${here}/../tools/test-sandbox.sh"
sandbox_make run-test
sandbox_enter

real=$(command -v python3.15 || command -v python3.14 || command -v python3.13 || command -v python3.12 || command -v python3.11 || command -v python3)
mkdir -p bin config/schwarzwald
for name in python3.15 python3.14 python3.13 python3.12 python3.11; do
  cat >"bin/${name}" <<S
#!/usr/bin/env bash
case "\$*" in
*drift.py*) echo "${name}" >"${SANDBOX}/drift-ran"; exit 0 ;;
esac
exec "${real}" "\$@"
S
  chmod +x "bin/${name}"
done
git config user.email t@example.org
git config user.name t
: >config/schwarzwald/lint.toml
git add bin config
git commit -qm case
mkdir -p part/lint/lib part/configs
cp "${here}/lib/pick-python.sh" part/lint/lib/
: >part/configs/drift.py
awk '/^# The shared lints, each/ { exit } { print }' "${here}/run.sh" >part/lint/run.sh
PATH="${PWD}/bin:${PATH}" bash part/lint/run.sh --repo "${PWD}" >out.txt 2>&1
fail=0
if [[ $(cat "${SANDBOX}/drift-ran" 2>/dev/null) == python3.15 ]] && grep -q '^PASS drift' out.txt; then
  echo "ok   run.sh runs drift.py under the newest python"
else
  echo "FAIL run.sh did not run drift.py under the newest python"
  fail=1
fi
# A linter whose config is missing says so: a deleted .gitleaks.toml may not drop the secret scan.
for tool in typos gitleaks; do
  if grep -q "^FAIL ${tool}: no config" out.txt; then echo "ok   a missing ${tool} config is a FAIL"; else
    echo "FAIL a missing ${tool} config left no FAIL line"
    fail=1
  fi
done
[[ ${fail} == 0 ]] || cat out.txt
exit "${fail}"
