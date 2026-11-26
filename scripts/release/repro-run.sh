#!/usr/bin/env bash
# repro-run.sh <target>: the release build repro-check.sh proves reproducible. It builds the dist
# profile locked, then packs the binary and the licence files into target/repro/ with
# repro-archive.sh, so the archive bytes depend on the contents only. repro-check.sh applies
# repro-env.sh (SOURCE_DATE_EPOCH, path remaps) before it runs this. Usage: repro-run.sh <target>
set -euo pipefail

target=${1:?usage: repro-run.sh <target>}
kit=vendor/schwarzwald/kit/tools
name=gitterrace-$target

cargo build --profile dist --locked -p gitterrace --target "$target"
stage=target/repro/$name
mkdir -p "$stage"
cp "target/$target/dist/git-terrace" LICENSE THIRD-PARTY-NOTICES "$stage/"
bash "$kit/repro-archive.sh" --base target/repro -o "target/repro/$name.tar.gz" "$name"
