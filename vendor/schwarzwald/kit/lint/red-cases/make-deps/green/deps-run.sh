#!/usr/bin/env bash
# Installs every tool make lint runs, at CI's ruff.
DEPS_TOOLS=(actionlint gitleaks node ruff shellcheck shfmt typos vale)
RUFF_VERSION=0.2.0
