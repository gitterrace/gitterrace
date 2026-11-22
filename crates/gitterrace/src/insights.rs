//! The `insights` command: the registry's answer for one repository, read offline.

use std::path::Path;

use anyhow::Context;
use gitterrace_api::{ApiState, Params};

/// The text to print: the insight `name` as the one JSON every surface answers, or the list
/// (plain lines, JSON with `json`) when no name is given.
pub(crate) fn run(
    path: &Path,
    cfg: &gitterrace_config::Config,
    name: Option<&str>,
    rev: Option<&str>,
    params: &Params,
    json: bool,
) -> anyhow::Result<String> {
    let Some(name) = name else {
        return if json {
            Ok(serde_json::to_string(&gitterrace_api::insight_index())?)
        } else {
            Ok(gitterrace_api::insight_list()
                .map(|(n, s)| format!("{n:<16}{s}"))
                .collect::<Vec<_>>()
                .join("\n"))
        };
    };
    let state = ApiState::open(path)?.configured(cfg);
    state.refresh_index().context("updating the insights cache")?;
    Ok(serde_json::to_string(&state.insight(name, rev, params)?)?)
}
