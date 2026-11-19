#!/usr/bin/env bash
# Install the pinned Linux lint tools of linux-pins.txt for this machine's arch into BIN.
#   bash .github/actions/lint-tools/linux-install.sh BIN [PINS]
# Every asset is fetched first and digest-checked before any is unpacked, so a failed fetch,
# a changed asset or an arch with no pins fails without installing anything.
set -euo pipefail
bin=${1:?usage: linux-install.sh BIN [PINS]}
pins=${2:-$(dirname "${BASH_SOURCE[0]}")/linux-pins.txt}
arch=$(uname -m)
dl=$(mktemp -d)
trap 'rm -rf "$dl"' EXIT
mkdir -p "$bin"

sums=
while read -r a name sum url; do
  [[ $a == "$arch" ]] || continue
  curl -fsSL -o "$dl/$name" "$url"
  sums+="$sum  $name"$'\n'
done < <(grep -v '^#' "$pins")
[[ -n $sums ]] || {
  echo "linux-install: no pins for $arch in $pins" >&2
  exit 1
}
printf '%s' "$sums" | (cd "$dl" && sha256sum -c --quiet -)

tar -xzf "$dl/vale.tar.gz" -C "$bin" vale
tar -xzf "$dl/gitleaks.tar.gz" -C "$bin" gitleaks
tar -xzf "$dl/actionlint.tar.gz" -C "$bin" actionlint
# The archives nest the binary in a versioned folder: match it, so a pin bump edits one file.
tar -xzf "$dl/git-cliff.tar.gz" -C "$bin" --strip-components=1 --wildcards '*/git-cliff'
tar -xJf "$dl/shellcheck.tar.xz" -C "$bin" --strip-components=1 --wildcards '*/shellcheck'
unzip -q -o "$dl/ast-grep.zip" ast-grep -d "$bin"
install -m 0755 "$dl/shfmt" "$bin/shfmt"
