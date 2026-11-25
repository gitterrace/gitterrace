#!/usr/bin/env bash
# sonar-scan.sh: an on-demand SonarQube review of HEAD, run before a release. Never part of the
# per-commit gate: a cold start takes about 2 minutes and the image needs 4 GB.
#
#   sonar-scan.sh [--update-baseline]
#
# Starts a throwaway SonarQube container (127.0.0.1 only, fixed name, named volume), scans a
# `git archive HEAD` snapshot, reads issues and hotspots through the API, then always stops and
# removes the container (success, failure, interrupt). Runs under quiet.sh unless SONAR_NO_QUIET=1.
#
# Config: <repo>/schwarzwald/config/sonar.toml (override the dir with SONAR_CONFIG_DIR), all keys
# optional:
#   project_key = "scan"          project key inside the throwaway server
#   exclusions  = ["vendor/**"]   passed to sonar.exclusions
#   languages   = ["py", "js"]    keep only rules whose key starts with one of these repo prefixes
#                                 (python, javascript, Web ...), empty = all
#   rules       = ["python:S1481"] rule keys that count IN ADDITION to the kit default allowlist
#   ignore      = ["Web:S5247"]   rule keys that never count (issues and hotspots)
# Kit default allowlist: Web:S5247, javascript:S9383, python:S5443, plus every security hotspot.
# Baseline: <config>/sonar-baseline.json, keys rule|file|sha1(line text), so a line shift keeps a
# finding baselined. --update-baseline rewrites it from the current counted findings.
#
# stdout: JSON {"issues": [...], "hotspots": [...]} of counted findings (rule, file, line, message,
# new). stderr: the path of the log holding the full raw dump.
# Exit: 0 nothing new outside the baseline, 1 new findings, 2 setup or docker error (with a remedy).
set -uo pipefail

SONARQUBE_IMAGE='sonarqube@sha256:7b2296713a92b8ea0dd7a083a25faadc59376c9a1e6daf2744a89711c54ea1ba'  # community 26.11.0.134175
SCANNER_IMAGE='sonarsource/sonar-scanner-cli@sha256:8bb84373f00346f9a579adb7ccb2999f8cd050ab7167fa9ae08a16069de03c84'
CONTAINER=sonar-scan-server
VOLUME=sonar-scan-data
PORT=9000
HOST_URL="http://127.0.0.1:$PORT"

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
if [ -z "${SONAR_NO_QUIET:-}" ] && [ -x "$here/quiet.sh" ]; then
  SONAR_NO_QUIET=1 exec "$here/quiet.sh" -- "${BASH_SOURCE[0]}" "$@"
fi

update=0
case "${1:-}" in
  "") ;;
  --update-baseline) update=1 ;;
  *) echo "usage: sonar-scan.sh [--update-baseline]" >&2; exit 2 ;;
esac

die() { # die <what failed> <remedy>
  echo "sonar-scan: $1" >&2; echo "sonar-scan: remedy: $2" >&2; exit 2
}

scratch=""; child=""
cleanup() {
  local rc=$?
  trap - EXIT INT TERM
  [ -z "$child" ] || kill "$child" 2>/dev/null
  if command -v docker >/dev/null 2>&1; then
    docker stop "$CONTAINER" >/dev/null 2>&1
    docker rm -f "$CONTAINER" >/dev/null 2>&1
    docker volume rm -f "$VOLUME" >/dev/null 2>&1
  fi
  [ -z "$scratch" ] || { [ -n "${SONAR_KEEP_SCRATCH:-}" ] || rm -rf "$scratch/snap" "$scratch"/*.cfg; }
  exit "$rc"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# run_bg <cmd...>: run in the background and wait, so a signal reaches the trap at once.
run_bg() { "$@" & child=$!; wait "$child"; local rc=$?; child=""; return "$rc"; }

PY=""
for cand in python3.15 python3.14 python3.13 python3.12 python3.11 python3; do
  if command -v "$cand" >/dev/null 2>&1 && "$cand" -c 'import tomllib' 2>/dev/null; then PY=$cand; break; fi
done
[ -n "$PY" ] || die "no python with tomllib (3.11 or newer)" "install python 3.11 or newer"
for tool in docker git curl; do
  command -v "$tool" >/dev/null 2>&1 || die "$tool is not installed" "install $tool and re-run"
done
repo=$(git rev-parse --show-toplevel 2>/dev/null) || die "not inside a git repository" "run from the product checkout"
cfgdir=${SONAR_CONFIG_DIR:-$repo/schwarzwald/config}
scratch=$(mktemp -d "${TMPDIR:-/tmp}/sonar-scan.XXXXXX") || die "cannot create a scratch dir" "free disk space"
chmod 700 "$scratch"
log=$scratch/sonar-raw.log
: >"$log"

settings=$("$PY" - "$cfgdir/sonar.toml" <<'PY'
import json, sys, tomllib, os
p = sys.argv[1]
d = tomllib.load(open(p, "rb")) if os.path.exists(p) else {}
print(json.dumps(d))
PY
) || die "cannot read $cfgdir/sonar.toml" "fix the TOML syntax"
set_get() { "$PY" -c 'import json,sys;v=json.loads(sys.argv[1]).get(sys.argv[2],sys.argv[3]);print(",".join(v) if isinstance(v,list) else v)' "$settings" "$1" "$2"; }
project_key=$(set_get project_key scan)
exclusions=$(set_get exclusions "")

# Secrets live only in 0600 curl config files, never in argv.
# Random hex, plus one of each class SonarQube's password policy demands (upper, lower, digit, special).
admin_pw="$(head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \n')Aa1!"
token=""
api_cfg() { # api_cfg <file> <user:pass> <path-and-query> [data...]
  local f=$1 cred=$2 url=$3; shift 3
  ( umask 077; { printf 'url = "%s%s"\nuser = "%s"\n' "$HOST_URL" "$url" "$cred"
      local d; for d in "$@"; do printf 'data-urlencode = "%s"\n' "$d"; done; } >"$f" )
}
api() { # api <cfg-file>: print the response, nonzero on HTTP failure
  curl -sS -f -m 60 -K "$1"
}

docker volume create "$VOLUME" >/dev/null 2>&1
if ! docker run -d --name "$CONTAINER" -p "127.0.0.1:$PORT:9000" -e SONAR_ES_BOOTSTRAP_CHECKS_DISABLE=true \
     -v "$VOLUME:/opt/sonarqube/data" --memory 4g "$SONARQUBE_IMAGE" >>"$log" 2>&1; then
  die "docker could not start the SonarQube container" "start the docker daemon, free port $PORT, remove a stale container with: docker rm -f $CONTAINER"
fi

api_cfg "$scratch/status.cfg" "" /api/system/status
up=0
for _ in $(seq 1 "${SONAR_POLL_TRIES:-90}"); do
  if api "$scratch/status.cfg" 2>/dev/null | grep -q '"status" *: *"UP"'; then up=1; break; fi
  run_bg sleep "${SONAR_POLL_SLEEP:-5}"
done
[ "$up" = 1 ] || die "SonarQube did not report UP in time" "check docker logs $CONTAINER, give docker 4 GB of memory, re-run"

api_cfg "$scratch/pw.cfg" "admin:admin" /api/users/change_password "login=admin" "previousPassword=admin" "password=$admin_pw"
curl -sS -f -m 60 -X POST -K "$scratch/pw.cfg" >>"$log" 2>&1 || die "could not change the admin password" "re-run on a fresh container"
# A USER_TOKEN, not GLOBAL_ANALYSIS_TOKEN: an analysis token uploads a scan but gets 403 on
# /api/hotspots/search. The admin's user token does both, on a server that lives for this run.
api_cfg "$scratch/tok.cfg" "admin:$admin_pw" /api/user_tokens/generate "name=scan" "type=USER_TOKEN"
token=$(curl -sS -f -m 60 -X POST -K "$scratch/tok.cfg" 2>>"$log" \
  | "$PY" -c 'import json,sys;print(json.load(sys.stdin)["token"])') || die "could not create an API token" "re-run on a fresh container"

snap=$scratch/snap
mkdir -p "$snap"
git -C "$repo" archive HEAD | tar -x -C "$snap" || die "git archive HEAD failed" "commit at least once and re-run"

export SONAR_HOST_URL=$HOST_URL SONAR_TOKEN=$token
if ! run_bg docker run --rm --network host -v "$snap:/usr/src" -e SONAR_HOST_URL -e SONAR_TOKEN "$SCANNER_IMAGE" \
     "-Dsonar.projectKey=$project_key" -Dsonar.sources=. "-Dsonar.exclusions=$exclusions" >>"$log" 2>&1; then
  die "the sonar scanner failed (see $log)" "check the log, make sure the docker daemon can pull $SCANNER_IMAGE"
fi
unset SONAR_TOKEN

# Wait for the server to finish processing the analysis report.
api_cfg "$scratch/ce.cfg" "$token:" "/api/ce/component?component=$project_key"
for _ in $(seq 1 "${SONAR_POLL_TRIES:-90}"); do
  ce=$(api "$scratch/ce.cfg" 2>/dev/null) || ce=""
  if [ -n "$ce" ] && ! printf '%s' "$ce" | grep -q '"queue" *: *\[ *{'; then break; fi
  run_bg sleep "${SONAR_POLL_SLEEP:-5}"
done

fetch_all() { # fetch_all <endpoint> <query> <outprefix>: pages of 500 into <outprefix>.N.json
  local ep=$1 q=$2 out=$3 p=1 total
  while :; do
    api_cfg "$scratch/page.cfg" "$token:" "$ep?$q&ps=500&p=$p"
    api "$scratch/page.cfg" >"$out.$p.json" || die "API call to $ep failed" "check docker logs $CONTAINER"
    total=$("$PY" -c 'import json,sys;print(json.load(open(sys.argv[1]))["paging"]["total"])' "$out.$p.json") \
      || die "bad API answer from $ep" "check $out.$p.json"
    [ $((p * 500)) -lt "$total" ] || break
    p=$((p + 1))
  done
}
# Each endpoint names the project its own way; 26.x answers 400 to a parameter it does not take.
fetch_all /api/issues/search "projectKey=$project_key&componentKeys=$project_key" "$scratch/issues"
fetch_all /api/hotspots/search "project=$project_key" "$scratch/hotspots"

"$PY" - "$scratch" "$snap" "$project_key" "$cfgdir/sonar-baseline.json" "$update" "$settings" "$log" <<'PY'
import glob, hashlib, json, os, sys
scratch, snap, key, basefile, update, settings, log = sys.argv[1:8]
update = update == "1"
cfg = json.loads(settings)
default_rules = {"Web:S5247", "javascript:S9383", "python:S5443"}
rules = default_rules | set(cfg.get("rules", []))
ignore = set(cfg.get("ignore", []))
langs = cfg.get("languages", [])

def load(prefix, field):
    out = []
    for f in sorted(glob.glob(f"{scratch}/{prefix}.*.json")):
        out += json.load(open(f)).get(field, [])
    return out

issues, hotspots = load("issues", "issues"), load("hotspots", "hotspots")
with open(log, "a") as fh:
    json.dump({"issues": issues, "hotspots": hotspots}, fh, indent=1)

def path_of(c):
    return c.split(":", 1)[1] if c.startswith(key + ":") else c

def text_of(path, line):
    try:
        lines = open(os.path.join(snap, path), errors="replace").read().split("\n")
        return lines[int(line) - 1].strip()
    except Exception:
        return ""

def norm(item, rule):
    path = path_of(item.get("component", ""))
    line = item.get("line") or item.get("textRange", {}).get("startLine") or 0
    h = hashlib.sha1(text_of(path, line).encode()).hexdigest()
    return {"rule": rule, "file": path, "line": line, "message": item.get("message", ""),
            "key": f"{rule}|{path}|{h}"}

def lang_ok(rule):
    return not langs or any(rule.startswith(l) for l in langs)

counted_i = [norm(i, i["rule"]) for i in issues
             if i["rule"] in rules and i["rule"] not in ignore and lang_ok(i["rule"])]
counted_h = [norm(h, h.get("ruleKey", "hotspot")) for h in hotspots
             if h.get("ruleKey", "") not in ignore and lang_ok(h.get("ruleKey", ""))]
if update:
    keys = sorted({c["key"] for c in counted_i + counted_h})
    os.makedirs(os.path.dirname(basefile) or ".", exist_ok=True)
    json.dump(keys, open(basefile, "w"), indent=1)
    open(basefile, "a").write("\n")
base = set(json.load(open(basefile))) if os.path.exists(basefile) else set()
new = 0
for c in counted_i + counted_h:
    c["new"] = c["key"] not in base
    new += c["new"]
    del c["key"]
json.dump({"issues": counted_i, "hotspots": counted_h}, sys.stdout, indent=1)
print()
sys.exit(1 if new else 0)
PY
rc=$?
echo "sonar-scan: raw dump in $log" >&2
[ -z "${SONAR_KEEP_SCRATCH:-}" ] || true
# keep the log: move it out of the scratch dir that cleanup removes only in part
exit "$rc"
