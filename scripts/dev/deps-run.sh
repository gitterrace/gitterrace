#!/usr/bin/env bash
# Install everything `make build`, `make check` and the browser checks need, at the versions CI
# pins, so a fresh machine needs only this and git. Usage:
#
#   scripts/dev/deps-run.sh            install what is missing, leave what is there
#   scripts/dev/deps-run.sh --check    list what is missing, install nothing (exit 1 if any)
#   scripts/dev/deps-run.sh --browsers also fetch the Playwright browsers (about 500 MB)
#
# Rust comes from rustup at the channel rust-toolchain.toml pins. On macOS the other tools come
# from Homebrew, which must be installed first (https://brew.sh): the script says so and stops,
# it never pipes a remote installer into a shell for you. On Linux it uses apt for what apt has
# and pinned release builds for the rest, the same as CI. ruff and cargo-machete are pinned
# here and in CI by the same lines below.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

RUFF_VERSION=0.17.3
MACHETE_VERSION=0.9.2
TYPOS_VERSION=1.51.1
BREW_TOOLS=(typos-cli vale gitleaks shellcheck shfmt actionlint ast-grep node uv)
# command name for each brew formula above, in the same order
BREW_CMDS=(typos vale gitleaks shellcheck shfmt actionlint ast-grep node uv)

mode=install
browsers=0
for a in "$@"; do
  case "$a" in
  --check) mode=check ;;
  --browsers) browsers=1 ;;
  *)
    sed -n '2,8s/^# \{0,1\}//p' "$0" >&2
    exit 2
    ;;
  esac
done

missing=()
have() { command -v "$1" >/dev/null 2>&1; }
note() { printf 'deps: %s\n' "$*"; }
need() { # need <label> <present-test...>: record <label> when the test fails
  local label=$1
  shift
  "$@" >/dev/null 2>&1 || missing+=("$label")
}

channel=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml)
toolchain_ok() { have rustup && rustup run "$channel" cargo --version; }
ruff_ok() { [ "$(ruff --version 2>/dev/null | awk '{print $2}')" = "$RUFF_VERSION" ]; }
machete_ok() { cargo machete --version 2>/dev/null | grep -qx "$MACHETE_VERSION"; }

need "git" have git
need "python3 (3.11 or newer)" python3 -c 'import sys; sys.exit(sys.version_info < (3, 11))'
need "rustup" have rustup
need "rust $channel (rust-toolchain.toml)" toolchain_ok
for i in "${!BREW_CMDS[@]}"; do need "${BREW_CMDS[$i]}" have "${BREW_CMDS[$i]}"; done
need "ruff $RUFF_VERSION" ruff_ok
need "cargo-machete $MACHETE_VERSION" machete_ok
need "npm packages (node_modules)" test -d node_modules/playwright-core

if [ "$mode" = check ]; then
  if [ "${#missing[@]}" -eq 0 ]; then
    note "everything is installed"
    exit 0
  fi
  note "missing:"
  printf '  %s\n' "${missing[@]}"
  note "run: make deps"
  exit 1
fi

if [ "${#missing[@]}" -eq 0 ] && [ "$browsers" = 0 ]; then
  note "everything is installed"
  exit 0
fi

have git || {
  note "install git first (macOS: xcode-select --install)"
  exit 1
}

os=$(uname -s)
if [ "$os" = Darwin ]; then
  have brew || {
    note "Homebrew is needed on macOS: install it from https://brew.sh, then run make deps again"
    exit 1
  }
  todo=()
  for i in "${!BREW_CMDS[@]}"; do have "${BREW_CMDS[$i]}" || todo+=("${BREW_TOOLS[$i]}"); done
  python3 -c 'import sys; sys.exit(sys.version_info < (3, 11))' 2>/dev/null || todo+=(python@3.13)
  have rustup || todo+=(rustup)
  if [ "${#todo[@]}" -gt 0 ]; then
    note "brew install ${todo[*]}"
    brew install "${todo[@]}"
  fi
  # brew's rustup is keg-only: put its shims on PATH for the rest of this run.
  have rustup || PATH="$(brew --prefix rustup)/bin:$PATH"
elif [ "$os" = Linux ]; then
  have apt-get || {
    note "this script installs with apt on Linux; install the tools in its header by hand"
    exit 1
  }
  sudo apt-get update -q
  sudo apt-get install -y -q shellcheck shfmt python3 python3-pip curl unzip nodejs npm
  have rustup || {
    note "rustup: install it from https://rustup.rs, then run make deps again"
    exit 1
  }
  have typos || cargo install typos-cli --version "$TYPOS_VERSION" --locked
  if ! have vale || ! have gitleaks || ! have actionlint || ! have ast-grep; then
    # The same pinned, digest-checked downloads CI uses, read from its action so the two agree.
    bin="$HOME/.local/bin"
    dl=$(mktemp -d)
    mkdir -p "$bin"
    # shellcheck disable=SC2016 # the action's text holds a literal $dl
    sed -n 's/^ *curl -fsSL -o "$dl\/\([^"]*\)" \(https:[^ ]*\)$/\1 \2/p' \
      .github/actions/lint-tools/action.yml | while read -r name url; do
      curl -fsSL -o "$dl/$name" "$url"
    done
    sed -n '/<<.SUMS.$/,/^ *SUMS$/p' .github/actions/lint-tools/action.yml | sed '1d;$d' |
      sed 's/^ *//' | (cd "$dl" && sha256sum -c -)
    tar -xzf "$dl/vale.tar.gz" -C "$bin" vale
    tar -xzf "$dl/gitleaks.tar.gz" -C "$bin" gitleaks
    tar -xzf "$dl/actionlint.tar.gz" -C "$bin" actionlint
    unzip -q -o "$dl/ast-grep.zip" ast-grep -d "$bin"
    rm -rf "$dl"
    PATH="$bin:$PATH"
    note "installed into $bin: put it on your PATH"
  fi
  have uv || python3 -m pip install --user -q uv
else
  note "unsupported OS $os: install the tools in this script's header by hand"
  exit 1
fi

toolchain_ok || {
  note "rustup toolchain install $channel"
  rustup toolchain install "$channel" --profile minimal --component clippy rustfmt
}
ruff_ok || {
  note "ruff $RUFF_VERSION"
  if have uv; then uv tool install --force "ruff==$RUFF_VERSION"; else
    python3 -m pip install --user -q "ruff==$RUFF_VERSION"
  fi
}
machete_ok || {
  note "cargo-machete $MACHETE_VERSION"
  rustup run "$channel" cargo install cargo-machete --version "$MACHETE_VERSION" --locked
}
[ -d node_modules/playwright-core ] || {
  note "npm ci"
  npm ci --no-audit --no-fund
}
if [ "$browsers" = 1 ]; then
  note "Playwright browsers (chromium, webkit)"
  PLAYWRIGHT_SKIP_BROWSER_GC=1 node node_modules/playwright-core/cli.js install chromium webkit
fi

missing=()
need "rust $channel" toolchain_ok
need "ruff $RUFF_VERSION" ruff_ok
need "cargo-machete $MACHETE_VERSION" machete_ok
for c in "${BREW_CMDS[@]}"; do need "$c" have "$c"; done
if [ "${#missing[@]}" -gt 0 ]; then
  note "still missing after install:"
  printf '  %s\n' "${missing[@]}"
  exit 1
fi
note "done. Next: make setup (once per machine), then make build"
