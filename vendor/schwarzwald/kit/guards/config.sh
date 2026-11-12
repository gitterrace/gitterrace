#!/usr/bin/env bash
#
# The product's guard config, read with builtins only. The fail-closed fallback sources this
# while the process table is full, so nothing here forks: no $(...), no external command.
#
# The config is <product>/schwarzwald/config/guards.env. GUARD_REPO or GUARD_CONFIG set by the
# caller wins.
# The file is data: each line is blank, a comment, NAME="literal" or NAME="${NAME:-literal}".
# Those are the only two forms; a single-quoted or bare value is refused like code.
# A literal may hold $HOME or ${HOME} and nothing else that expands (no other $, no backtick, no
# quote, no brace, no backslash before one of those). The first form sets the
# value; the second keeps a value the environment already has. Any other line is refused and
# named in GUARD_CONFIG_PROBLEM, and the engine then gives no verdict, so the adapter degrades
# loudly. A missing file is the same. A NAME not in GUARD_CONFIG_KEYS is NOT set, and is named in
# GUARD_CONFIG_UNKNOWN: the engine warns on every call and keeps judging with every rule. One
# stray key must never turn every guard into the fallback.

# Every key a product may set. build.py --check holds config.example.env to exactly this list.
GUARD_CONFIG_KEYS="GUARD_PRODUCT GUARD_APP_RE GUARD_SPOOL_NAME GUARD_SCRATCH_ROOTS \
GUARD_OPUS_CLASS_RE GUARD_OPUS_CLASS_NAMES GUARD_PROTECTED_GLOBS GUARD_PROTECTED_HINT \
GUARD_GENERATED_FEEDS GUARD_GENERATED_HINT GUARD_WORKTREE_DIR GUARD_LANE_ENV GUARD_LANE_ROLES \
GUARD_WRITE_CMDS_RE GUARD_BUILD_TARGETS_RE GUARD_LONG_TARGETS_RE GUARD_DETACHED_RE \
GUARD_BUILD_LOCK_ENV GUARD_MAKE_HINT GUARD_PRODUCT_RULE_DIRS GUARD_PRODUCT_RULE_FILES \
GUARD_PRODUCT_CASES GUARD_PRODUCT_PROBES GUARD_READ_MAX_LINES GUARD_READ_MODE MAIN_CHECKOUT_LANDS_ONLY"

GUARD_CONFIG_PROBLEM=""
GUARD_CONFIG_UNKNOWN=""

# The product a path inside the kit belongs to, printed into GUARD_ROOT_OF. The kit sits at
# <product>/vendor/schwarzwald/kit/ or <product>/schwarzwald/kit/: the product is what comes
# before that suffix. A path in neither layout (the kit's own dist/guards/) keeps the old three
# levels above guards/. Builtins only, and the adapters inline these same lines, so they and the
# engine can never disagree about where the product is.
guard_root_of() {
    local p="${1%/}"
    case "$p" in
        */schwarzwald/kit/guards | */schwarzwald/kit/guards/*)
            p="${p%%/schwarzwald/kit/guards*}"
            GUARD_ROOT_OF="${p%/vendor}"
            ;;
        *) GUARD_ROOT_OF="${p%/*/*/*}" ;;
    esac
}

# Set GUARD_PRODUCT_ROOT, GUARD_REPO (when unset) and GUARD_CONFIG (when unset). $1 is the guards
# directory. GUARD_KIT_DIR is the kit's folder relative to the product, for messages that name a
# kit tool.
guard_config_locate() {
    local d
    guard_root_of "$1"
    d="$GUARD_ROOT_OF"
    GUARD_KIT_DIR="${1%/}"
    GUARD_KIT_DIR="${GUARD_KIT_DIR%/guards}"
    GUARD_KIT_DIR="${GUARD_KIT_DIR#"$d"/}"
    # The product this install belongs to: its own rules live here, wherever GUARD_REPO points.
    GUARD_PRODUCT_ROOT="$d"
    [ -n "${GUARD_REPO:-}" ] || GUARD_REPO="$d"
    # The config belongs to the product this install sits in, whatever GUARD_REPO says.
    GUARD_CONFIG="${GUARD_CONFIG:-$d/schwarzwald/config/guards.env}"
}

# The two line shapes, one ERE held in a variable (a bracket expression cannot hold a backslash
# escape). A literal is anything but a quote, a backtick, a $, a brace or a backslash, a
# backslash before none of those (bash keeps it), or the words $HOME and ${HOME}. Groups: 1 the
# name, 2 the `${NAME:-` of the default form, 3 its name, 4 the literal, 6 its closing brace.
_GUARD_CFG_LINE='^([A-Z][A-Z0-9_]*)="(\$\{([A-Z][A-Z0-9_]*):-)?(([^"`$}\\]|\\[^"`$}\\]|\$HOME|\$\{HOME\})*)(\}?)"$'

_guard_config_line() {
    local line="$1" name val
    case "$line" in "" | "#"*) return 0 ;; esac
    if [[ $line =~ $_GUARD_CFG_LINE ]] && [ -z "${BASH_REMATCH[2]}${BASH_REMATCH[6]}" ]; then
        name="${BASH_REMATCH[1]}" val="${BASH_REMATCH[4]}"
    elif [[ $line =~ $_GUARD_CFG_LINE ]] && [ "${BASH_REMATCH[1]}" = "${BASH_REMATCH[3]}" ] &&
        [ -n "${BASH_REMATCH[6]}" ]; then
        name="${BASH_REMATCH[1]}" val="${BASH_REMATCH[4]}"
        # The environment's value wins in this form.
        [ -z "${!name:-}" ] || return 0
    else
        GUARD_CONFIG_PROBLEM="$GUARD_CONFIG_PROBLEM line not data: $line;"
        return 0
    fi
    case " $GUARD_CONFIG_KEYS " in
        *" $name "*) ;;
        *)
            GUARD_CONFIG_UNKNOWN="$GUARD_CONFIG_UNKNOWN unknown key $name;"
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

# The value of key $2 in the config file $1, into GUARD_CONFIG_VALUE, read by the same parser
# as the guards, so every spelling the shape allows counts. The environment's value is ignored
# (a caller cannot switch a product's flag off), and a broken line elsewhere does not hide it.
guard_config_value() {
    GUARD_CONFIG_VALUE=""
    [ -f "$1" ] || return 0
    local GUARD_CONFIG="$1" GUARD_CONFIG_PROBLEM="" GUARD_CONFIG_UNKNOWN=""
    local line
    unset "$2"
    while IFS= read -r line || [ -n "$line" ]; do
        _guard_config_line "$line"
    done <"$GUARD_CONFIG"
    GUARD_CONFIG_VALUE="${!2:-}"
}
