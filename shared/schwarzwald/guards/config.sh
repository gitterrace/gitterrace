#!/usr/bin/env bash
#
# The product's guard config, read with builtins only. The fail-closed fallback sources this
# while the process table is full, so nothing here forks: no $(...), no external command.
#
# The config is <product>/config/schwarzwald/guards.env. GUARD_REPO or GUARD_CONFIG set by the
# caller wins.
# The file is data: each line is blank, a comment, NAME='literal' or NAME="${NAME:-literal}".
# A literal may hold $HOME or ${HOME} and nothing else that expands. The first form sets the
# value; the second keeps a value the environment already has. Any other line, or a NAME not
# in GUARD_CONFIG_KEYS, is refused and named in GUARD_CONFIG_PROBLEM, and the engine then
# gives no verdict, so the adapter degrades loudly. A missing file is the same.

# Every key a product may set. build.py --check holds config.example.env to exactly this list.
GUARD_CONFIG_KEYS="GUARD_PRODUCT GUARD_APP_RE GUARD_SPOOL_NAME GUARD_SCRATCH_ROOTS \
GUARD_OPUS_CLASS_RE GUARD_OPUS_CLASS_NAMES GUARD_PROTECTED_GLOBS GUARD_PROTECTED_HINT \
GUARD_GENERATED_FEEDS GUARD_GENERATED_HINT GUARD_WORKTREE_DIR GUARD_LANE_ENV GUARD_LANE_ROLES \
GUARD_WRITE_CMDS_RE GUARD_BUILD_TARGETS_RE GUARD_LONG_TARGETS_RE GUARD_DETACHED_RE \
GUARD_BUILD_LOCK_ENV GUARD_MAKE_HINT GUARD_PRODUCT_RULE_DIRS GUARD_PRODUCT_RULE_FILES \
GUARD_PRODUCT_CASES GUARD_PRODUCT_PROBES"

GUARD_CONFIG_PROBLEM=""

# Set GUARD_PRODUCT_ROOT, GUARD_REPO (when unset) and GUARD_CONFIG (when unset). $1 is the guards directory, at
# <product>/shared/schwarzwald/guards/: the product root is three levels up. A fixed depth, not
# a search, so the adapters (which strip the same levels to find the org) and the engine can
# never disagree about where the product is.
guard_config_locate() {
    local d="${1%/}"
    d="${d%/*/*/*}"
    # The product this install belongs to: its own rules live here, wherever GUARD_REPO points.
    GUARD_PRODUCT_ROOT="$d"
    [ -n "${GUARD_REPO:-}" ] || GUARD_REPO="$d"
    # The config belongs to the product this install sits in, whatever GUARD_REPO says.
    GUARD_CONFIG="${GUARD_CONFIG:-$d/config/schwarzwald/guards.env}"
}

# The two line shapes, as EREs held in variables (a bracket expression cannot hold a backslash
# escape, so the default's literal is "anything but a quote, a backtick, a $ or a brace, or the
# words $HOME and ${HOME}").
_GUARD_CFG_SET="^([A-Z][A-Z0-9_]*)='([^']*)'\$"
_GUARD_CFG_DEFAULT='^([A-Z][A-Z0-9_]*)="\$\{([A-Z][A-Z0-9_]*):-(([^"`$}]|\$HOME|\$\{HOME\})*)\}"$'

_guard_config_line() {
    local line="$1" name val
    case "$line" in "" | "#"*) return 0 ;; esac
    if [[ $line =~ $_GUARD_CFG_SET ]]; then
        name="${BASH_REMATCH[1]}" val="${BASH_REMATCH[2]}"
    elif [[ $line =~ $_GUARD_CFG_DEFAULT ]] &&
        [ "${BASH_REMATCH[1]}" = "${BASH_REMATCH[2]}" ]; then
        name="${BASH_REMATCH[1]}" val="${BASH_REMATCH[3]}"
        # The environment's value wins in this form.
        [ -z "${!name:-}" ] || return 0
    else
        GUARD_CONFIG_PROBLEM="$GUARD_CONFIG_PROBLEM line not data: $line;"
        return 0
    fi
    case " $GUARD_CONFIG_KEYS " in
        *" $name "*) ;;
        *)
            GUARD_CONFIG_PROBLEM="$GUARD_CONFIG_PROBLEM unknown key $name;"
            return 0
            ;;
    esac
    # shellcheck disable=SC2016 # the literal text $HOME is what the file holds
    val="${val//'${HOME}'/$HOME}"
    # shellcheck disable=SC2016
    val="${val//'$HOME'/$HOME}"
    printf -v "$name" '%s' "$val"
    export "${name?}"
}

# Read GUARD_CONFIG. Safe to call twice: a value already set is only overwritten by the
# first form, which states the product's value.
guard_config_load() {
    if [ ! -f "${GUARD_CONFIG:-}" ]; then
        GUARD_CONFIG_PROBLEM="no config at ${GUARD_CONFIG:-<unset>}"
        return 1
    fi
    local line
    while IFS= read -r line || [ -n "$line" ]; do
        _guard_config_line "$line"
    done <"$GUARD_CONFIG"
    [ -z "$GUARD_CONFIG_PROBLEM" ]
}
