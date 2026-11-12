#!/usr/bin/env bash
# The shared adapter of this name, at a path a harness config keeps.
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
exec "${root}/vendor/schwarzwald/kit/guards/adapters/cli.sh" "$@"
