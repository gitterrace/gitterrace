#!/usr/bin/env bash
# The shared adapter of this name, at a path a harness config keeps.
exec "$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)/shared/schwarzwald/guards/adapters/jcode-post.sh" "$@"
