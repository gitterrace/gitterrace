#!/usr/bin/env bash
# Runtime smoke test of a built git-terrace binary. One script, many callers: the Windows CI job,
# the release check job and the wine-run, linux-run and macos-run scripts.
# Usage: smoke-check.sh <binary> [port].
# GT_WINE=1 runs the binary under Wine (the binary is a .exe, paths go through winepath).
# shellcheck disable=SC2015  # `A && ok || bad` is meant: bad runs when the check fails
set -euo pipefail
bin=${1:?usage: smoke-check.sh <binary> [port]}
port=${2:-4261}
tmp=$(mktemp -d)
fails=0
pid=
cleanup() {
  if [ -n "$pid" ]; then kill "$pid" 2>/dev/null || true; fi
  rm -rf "$tmp"
}
trap cleanup EXIT
ok() { echo "PASS: $*"; }
bad() {
  echo "FAIL: $*"
  fails=$((fails + 1))
}

# Paths the binary understands: Windows form under Git Bash or Wine, as is elsewhere.
native() {
  if [ "${GT_WINE:-}" = 1 ]; then
    winepath -w "$1"
  elif command -v cygpath >/dev/null 2>&1; then
    cygpath -w "$1"
  else
    echo "$1"
  fi
}
exe() {
  if [ "${GT_WINE:-}" = 1 ]; then wine "$bin" "$@"; else "$bin" "$@"; fi
}
# Is the hang action's own child still running? On unix it is found by its unique argument, so
# another process of the same name (a runner's, an earlier test's) never reads as ours. Windows
# lists image names only.
alive() {
  case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) tasklist 2>/dev/null | grep -qi "^$1" ;;
  *)
    if [ "${GT_WINE:-}" = 1 ]; then wine tasklist 2>/dev/null | grep -qi "$1"; else pgrep -f "$2" >/dev/null; fi
    ;;
  esac
}

# A fixture repository with a nested path, a markdown file and CRLF text.
repo="$tmp/fixture"
mkdir -p "$repo/src/deep"
git -C "$repo" init -q -b main
git -C "$repo" config user.email smoke@example.com
git -C "$repo" config user.name Smoke
printf '# Fixture\n\nhello\n' >"$repo/README.md"
printf 'fn main() {}\r\nfn two() {}\r\n' >"$repo/src/deep/a.rs"
git -C "$repo" add README.md src
git -C "$repo" commit -q -m "first commit"
printf 'more\n' >>"$repo/README.md"
git -C "$repo" commit -q -am "second commit"
sha=$(git -C "$repo" rev-parse HEAD)
cfg="$tmp/config"
mkdir -p "$cfg"

exe --version | grep -q gitterrace && ok "--version" || bad "--version"
exe --config "$(native "$cfg")" init >/dev/null && [ -f "$cfg/config.toml" ] && [ -f "$cfg/actions.toml" ] && ok "init writes config" || bad "init"

# The action sleeps far past its 1s timeout, so only the kill ends it.
if [ "${GT_WINE:-}" = 1 ] || [ "$(uname -s | cut -c1-4)" = MING ] || [ "$(uname -s | cut -c1-4)" = MSYS ]; then
  sleeper='"ping.exe", "-n", "600", "127.0.0.1"'
  image=PING.EXE
else
  # A duration no other process uses: the kill check looks for exactly this child.
  nap=$((600000 + $$))
  sleeper="\"sleep\", \"$nap\""
  image="sleep $nap"
fi
cat >"$cfg/actions.toml" <<TOML
version = 1

[[action]]
id      = "hang"
label   = "Hang"
kinds   = ["line"]
command = [$sleeper]
stdin   = "none"
timeout = "1s"
TOML

if [ "${GT_WINE:-}" = 1 ]; then
  wine "$bin" --config "$(native "$cfg")" --addr "127.0.0.1:$port" "$(native "$repo")" >"$tmp/serve.log" 2>&1 &
else
  "$bin" --config "$(native "$cfg")" --addr "127.0.0.1:$port" "$(native "$repo")" >"$tmp/serve.log" 2>&1 &
fi
pid=$!
base="http://127.0.0.1:$port"
up=0
for _ in $(seq 1 90); do
  if curl -fsS "$base/" >/dev/null 2>&1; then
    up=1
    break
  fi
  sleep 1
done
if [ "$up" = 1 ]; then ok "serve answers"; else
  bad "serve did not answer"
  cat "$tmp/serve.log"
  exit 1
fi

# Every repository is served under /r/<name>/, the name the server lists.
rname=$(curl -fsS "$base/api/repos" 2>/dev/null | sed -n 's/.*"name":"\([^"]*\)".*/\1/p' | head -1)
rbase="$base/r/${rname:-fixture}"
page() {
  case "$1" in
  /static/*) curl -sS -o "$tmp/body" -w '%{http_code}' "$base$1" ;;
  *) curl -sS -o "$tmp/body" -w '%{http_code}' "$rbase$1" ;;
  esac
}
for p in / /tree/HEAD /tree/HEAD/src/deep /blob/HEAD/README.md /blob/HEAD/src/deep/a.rs /log/HEAD "/commit/$sha" /refs /stats /static/app.css /static/nav.js; do
  code=$(page "$p")
  if [ "$code" = 200 ]; then ok "GET $p 200"; else bad "GET $p gave $code"; fi
done
page / >/dev/null
grep -qi 'rel="icon"\|favicon' "$tmp/body" && ok "overview links the favicon" || bad "overview has no favicon"
[ "$(page '/blob/HEAD/src%5Cdeep%5Ca.rs')" != 200 ] && ok "backslash path is not a file" || bad "backslash path resolved"
[ "$(page /blob/HEAD/nope)" = 404 ] && ok "missing blob 404" || bad "missing blob not 404"

# Export a static copy.
out="$tmp/export"
exe --config "$(native "$cfg")" export "$(native "$out")" "$(native "$repo")" >/dev/null 2>&1 && [ -f "$out/index.html" ] && ok "export" || bad "export"

# An action past its timeout is killed, child included.
page "/blob/HEAD/README.md" >/dev/null
token=$(sed -n 's/.*name="gitterrace-actions" content="\([^"]*\)".*/\1/p' "$tmp/body" | head -1)
name=$rname
if [ -z "$token" ]; then
  bad "no action token in the page"
else
  req="{\"action\":\"hang\",\"kind\":\"line\",\"locator\":{\"repo\":\"${name:-fixture}\",\"rev\":\"HEAD\",\"path\":\"README.md\",\"lines\":{\"start\":1,\"end\":1}}}"
  reply=$(curl -sS -X POST "$base/_actions/run" -H 'Content-Type: application/json' \
    -H 'Sec-Fetch-Site: same-origin' -H "Origin: $base" -H "X-Gitterrace-Token: $token" -d "$req")
  echo "$reply" | grep -q 'timed out' && ok "action timed out" || bad "action did not time out: $reply"
  # The kill is asynchronous: give it up to 10 s, polling, before calling the child alive.
  for _ in $(seq 1 50); do
    alive "$image" "$image" || break
    sleep 0.2
  done
  if alive "$image" "$image"; then bad "action child still alive"; else ok "action child killed"; fi
fi

echo "failures: $fails"
[ "$fails" = 0 ]
