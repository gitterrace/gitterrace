#!/usr/bin/env bash
#
# An Opus lane spawn needs an Opus class in its prompt: prose alone kept losing to "be safe".
# The class list is a named config point (GUARD_OPUS_CLASS_RE in guards.env), so each
# org keeps its own list without editing the mechanism.
# Words inside a double-quoted or backticked run are an EXAMPLE, not the brief, and do not
# count. Not caught: a class word in single quotes (an apostrophe would make stripping
# those unreliable), or a prompt that names a class it does not need.

guard_register lane_model_tier
guard_rule_lane_model_tier() {
    [ "${GUARD_TOOL:-}" = swarm ] || return 0
    [ "${GUARD_ACTION:-}" = spawn ] || return 0
    case "${GUARD_MODEL:-}" in
        *opus* | *Opus*) ;;
        *) return 0 ;;
    esac
    local prompt classes="${GUARD_OPUS_CLASS_RE:-schema|migration|security|irreversible}"
    prompt=$(printf '%s' "${GUARD_PROMPT:-}" | tr '[:upper:]' '[:lower:]' |
        sed 's/"[^"]*"/ /g; s/`[^`]*`/ /g')
    guard_match "$prompt" "($classes)" && return 0
    guard_block "an Opus lane whose prompt names no Opus class." \
        "The cheaper tier is the default lane. Opus is for: ${GUARD_OPUS_CLASS_NAMES:-$classes}." \
        "Name the class in the brief (not inside a quoted example), or spawn on the cheaper model."
}
