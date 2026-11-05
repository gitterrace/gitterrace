//! `GET /_settings`: the settings page. Rust builds every value it shows: the form's fields
//! from `gitterrace_config::fields`, the file switch and the Form/Plain switch from the shared
//! segmented control, the empty state when there is no file yet. The template renders them and
//! `settings.js` sends edits to the routes in `settings.rs`; the browser never writes TOML.

use std::path::Path;
use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Query, State},
    http::{HeaderValue, header},
    response::{Html, IntoResponse, Response},
};
use gitterrace_config::{Field, Input};
use gitterrace_types::SettingsFile;

use crate::empty::Empty;
use crate::seg::{Choice, Seg};
use crate::settings::Settings;

/// One input row of the form, as shown.
pub(crate) struct Row {
    pub(crate) id: &'static str,
    pub(crate) name: &'static str,
    pub(crate) doc: &'static str,
    pub(crate) default: String,
    /// The value in the file, without TOML quoting for text; empty when unset.
    pub(crate) value: String,
    /// `default`, `file` or `flag`.
    pub(crate) source: &'static str,
    pub(crate) restart: bool,
    /// `checkbox`, `tri`, `number`, `text`, `list` or `select`.
    pub(crate) kind: &'static str,
    pub(crate) min: String,
    pub(crate) max: String,
    pub(crate) options: Vec<(&'static str, bool)>,
    pub(crate) checked: bool,
    /// For `tri`: `unset`, `true` or `false`.
    pub(crate) tri: &'static str,
}

/// One `[section]` of the form.
pub(crate) struct Section {
    pub(crate) name: &'static str,
    pub(crate) rows: Vec<Row>,
}

#[derive(Template)]
#[template(path = "settings.html")]
struct Page {
    token: String,
    file: &'static str,
    head: crate::dochead::DocHead,
    text: String,
    stamp: String,
    sections: Vec<Section>,
    /// `[repo.NAME]` overrides, shown read-only: the plain editor edits them.
    overrides: Vec<String>,
    /// Why the form is not offered: the first error of the file, with its line.
    form_off: Option<String>,
    missing: Option<Empty>,
}

#[derive(serde::Deserialize)]
pub(crate) struct Which {
    #[serde(default)]
    f: Option<SettingsFile>,
}

/// `path` with the home folder written as `~`, so a page never shows the user's name.
fn shorten(path: &Path) -> String {
    let home = gitterrace_config::home_var().map(std::path::PathBuf::from);
    home.as_deref()
        .and_then(|h| path.strip_prefix(h).ok())
        .map_or_else(
            || path.display().to_string(),
            |rest| format!("~/{}", rest.display()),
        )
}

fn row(f: Field) -> Row {
    let value = f.plain();
    let (kind, min, max, options) = match &f.input {
        Input::Checkbox => ("checkbox", None, None, Vec::new()),
        Input::Tri => ("tri", None, None, Vec::new()),
        Input::Number { min, max } => ("number", *min, *max, Vec::new()),
        Input::Text => ("text", None, None, Vec::new()),
        Input::List => ("list", None, None, Vec::new()),
        Input::Select(v) => (
            "select",
            None,
            None,
            v.iter().map(|o| (*o, *o == value)).collect(),
        ),
    };
    Row {
        id: f.id,
        name: f.name,
        doc: f.doc,
        checked: f.value == "true",
        tri: match f.value.as_str() {
            "true" => "true",
            "false" => "false",
            _ => "unset",
        },
        default: f.default,
        value,
        source: f.source.label(),
        restart: f.restart,
        kind,
        min: min.map(|n| n.to_string()).unwrap_or_default(),
        max: max.map(|n| n.to_string()).unwrap_or_default(),
        options,
    }
}

/// The form's sections for config text that loads.
pub(crate) fn sections(loaded: &gitterrace_config::Loaded) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    for f in gitterrace_config::fields(loaded) {
        match out.last_mut() {
            Some(s) if s.name == f.section => s.rows.push(row(f)),
            _ => out.push(Section {
                name: f.section,
                rows: vec![row(f)],
            }),
        }
    }
    out
}

fn first_error(r: &crate::settings::CheckReply) -> Option<String> {
    r.errors.first().map(|e| {
        if e.line > 0 {
            format!("Fix line {} to use the form: {}", e.line, e.msg)
        } else {
            format!("Fix the file to use the form: {}", e.msg)
        }
    })
}

/// One line per `[repo.NAME]` override, shown read-only.
fn overrides(c: &gitterrace_config::Config) -> Vec<String> {
    c.repo
        .iter()
        .map(|(name, o)| {
            let off: Vec<&str> = o.features_off.iter().map(|x| x.key()).collect();
            let off = if off.is_empty() {
                String::new()
            } else {
                format!(", features off: {}", off.join(", "))
            };
            format!("[repo.{name}] exclude {:?}{off}", o.exclude)
        })
        .collect()
}

/// `GET /_settings?f=config|actions`.
pub(crate) async fn page(
    State(settings): State<Arc<Settings>>,
    Query(which): Query<Which>,
) -> Response {
    let f = which.f.unwrap_or(SettingsFile::Config);
    let Some(path) = settings.path(f) else {
        return crate::error::not_found("Actions are off on this server.");
    };
    let actions_on = settings.path(SettingsFile::Actions).is_some();
    let token = settings.token().to_owned();
    let env = settings.env.clone();
    let r = tokio::task::spawn_blocking(move || {
        let stamp = gitterrace_config::Stamp::of(&path).ok().flatten();
        let text = match std::fs::symlink_metadata(&path) {
            Ok(m) if m.file_type().is_file() => std::fs::read_to_string(&path).unwrap_or_default(),
            _ => String::new(),
        };
        let check = crate::settings::validate(f, &text, &text, &env);
        let (sections, overrides) = match (f, gitterrace_config::parse(&text)) {
            (SettingsFile::Config, Ok(l)) => (sections(&l), overrides(&l.config)),
            _ => (Vec::new(), Vec::new()),
        };
        let file_name = match f {
            SettingsFile::Config => "config.toml",
            SettingsFile::Actions => "actions.toml",
        };
        let mut files = vec![Choice::new(
            "config.toml",
            "/_settings?f=config",
            f == SettingsFile::Config,
        )];
        if actions_on {
            files.push(Choice::new(
                "actions.toml",
                "/_settings?f=actions",
                f == SettingsFile::Actions,
            ));
        }
        let form_off = first_error(&check);
        Page {
            token,
            file: f.as_str(),
            head: crate::dochead::DocHead {
                title: shorten(&path),
                switch: Some((
                    Seg::links("File", files).no_caption(),
                    Seg {
                        name: "Editor".to_owned(),
                        buttons: true,
                        choices: vec![
                            Choice::new("Form", "form", form_off.is_none()),
                            Choice::new("Plain", "plain", form_off.is_some()),
                        ],
                        ..Seg::default()
                    },
                )),
                tools: concat!(
                    "<span class=\"setnote\" id=\"setnote\" role=\"status\"></span>",
                    "<button class=\"primary\" type=\"button\" id=\"setsave\" disabled>",
                    "Save</button>"
                )
                .to_owned(),
                ..crate::dochead::DocHead::default()
            },
            missing: stamp.is_none().then(|| {
                Empty::new(format!(
                    "No {file_name} yet: every setting is at its default."
                ))
            }),
            stamp: gitterrace_config::Stamp::tag(stamp),
            text,
            sections,
            overrides,
            form_off,
        }
        .render()
    })
    .await;
    match r {
        Ok(Ok(html)) => {
            let mut res = Html(html).into_response();
            res.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            res
        }
        _ => crate::error::not_found("The settings page could not be built."),
    }
}
