#!/usr/bin/env bash
#
# Synthetic pointer, keyboard or window input on the LIVE desktop has no target.
# A keystroke aimed at the product's app (GUARD_APP_RE in guards.env) is the same defect: it
# lands in whichever instance is frontmost, the owner's own included.

guard_register live_desktop_input
guard_rule_live_desktop_input() {
    # A screen point names no window: a click "for the fixture" can land in the
    # owner's own window. BLOCK.
    local why=""
    if [ "$GUARD_TOOL" = macos_computer_use ]; then
        case "${GUARD_ACTION:-}" in
            click | double_click | right_click | drag | scroll | move | type | key | key_down | key_up | \
                focus_window | move_window | resize_window | activate_app)
                why="the action '$GUARD_ACTION' posts input or moves a window on the live desktop"
                ;;
        esac
    fi

    if [ -z "$why" ]; then
        local src="$GUARD_SCRIPT"
        if [ -z "$src" ]; then
            if guard_match "$(guard_cmd_head)" "$GUARD_ANCHOR"'(osascript|cliclick)( |$)'; then
                src="$GUARD_CMD"
            fi
        fi
        # Not caught: input addressed to a pid reaches that process only.
        if [ -n "$src" ] &&
            guard_match "$src" '(click at|set (the )?(position|size|frontmost) of|set frontmost|AXRaise|^[[:space:]]*(cliclick)[[:space:]])'; then
            why="AppleScript that clicks at a position, or sets a window's position, size or frontmost"
        fi
        # A keystroke "to the app by pid" goes to whichever instance is FRONTMOST
        # (activation is async): a Cmd-Z can undo the owner's edit.
        if [ -z "$why" ] && [ -n "$src" ] &&
            guard_match "$src" '(keystroke|key code) ' &&
            guard_match "$src" "${GUARD_APP_RE:-x^}|unix id"; then
            why="AppleScript that sends a keystroke to an app process, which goes to whichever instance is FRONTMOST"
        fi
    fi
    [ -n "$why" ] || return 0

    guard_block \
        "$why." \
        "" \
        "  Screen coordinates have no target; the window on top receives them. A fixture" \
        "  verification clicked the owner's own app window and the owner watched it." \
        "" \
        "  Instead:" \
        "    - drive a fixture through a headless browser or the app's own code (addressed to the app)" \
        "    - screenshots: screencapture -l <windowid> (or macos_computer_use window_screenshot)" \
        "    - pid-addressed menu clicks stay allowed: click menu item ... of (first process whose unix id is <pid>)," \
        "      which is delivered to that pid only, unlike a keystroke"
}
