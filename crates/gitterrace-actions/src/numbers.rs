//! The fixed numbers of the actions contract, once each. A macro expands to the literal, so a
//! const and a `concat!` doc string in [`crate::schema`] read the same source and cannot
//! disagree.

/// `[action.vars]` and `[send.vars]`: most bytes of one value, in KiB.
macro_rules! var_kib {
    () => {
        4
    };
}
/// `[action.vars]`: most entries, and most characters of a name.
macro_rules! max_vars {
    () => {
        32
    };
}
/// Payload `text`: most KiB.
macro_rules! text_kib {
    () => {
        64
    };
}
/// Payload `text`: most lines.
macro_rules! text_lines {
    () => {
        400
    };
}
/// An action's `timeout` ceiling, in seconds.
macro_rules! timeout_secs {
    () => {
        120
    };
}

pub(crate) use {max_vars, text_kib, text_lines, timeout_secs, var_kib};
