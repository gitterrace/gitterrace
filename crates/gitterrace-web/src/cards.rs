//! Shareable cards and badges from `gitterrace-cards`, served as SVG.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use gitterrace_cards::{Badge, Layout, Theme};
use serde::Deserialize;

use crate::{
    app::{App, blocking},
    error::Result,
    fmt,
};

#[derive(Deserialize)]
pub(crate) struct CardQuery {
    layout: Option<String>,
    theme: Option<String>,
}

fn theme(q: Option<&str>) -> Theme {
    if q == Some("dark") {
        Theme::Dark
    } else {
        Theme::Light
    }
}

fn svg(body: String) -> Response {
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("image/svg+xml; charset=utf-8"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
        ],
        body,
    )
        .into_response()
}

fn pending() -> Response {
    (StatusCode::SERVICE_UNAVAILABLE, "index is still building").into_response()
}

pub(crate) async fn wrapped(
    State(app): State<Arc<App>>,
    Path(year): Path<String>,
    Query(q): Query<CardQuery>,
) -> Result<Response> {
    let Ok(year) = year.strip_suffix(".svg").unwrap_or(&year).parse::<i32>() else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let layout = if q.layout.as_deref() == Some("portrait") {
        Layout::Portrait
    } else {
        Layout::Landscape
    };
    let theme = theme(q.theme.as_deref());
    let body = blocking(&app, move |a| {
        let head = a.repo.resolve("HEAD")?;
        Ok(a.records(&head)?.map(|r| {
            let w = gitterrace_cards::wrapped_stats(&a.name, &r, year);
            gitterrace_cards::wrapped_svg(&w, layout, theme)
        }))
    })
    .await?;
    Ok(body.map_or_else(pending, svg))
}

pub(crate) async fn badge(
    State(app): State<Arc<App>>,
    Path(name): Path<String>,
    Query(q): Query<CardQuery>,
) -> Result<Response> {
    let Some(badge) = Badge::parse(&name) else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let theme = theme(q.theme.as_deref());
    let body = blocking(&app, move |a| {
        let head = a.repo.resolve("HEAD")?;
        Ok(a.records(&head)?
            .map(|r| gitterrace_cards::badge_svg(badge, &r, fmt::now(), theme)))
    })
    .await?;
    Ok(body.map_or_else(pending, svg))
}
