//! `docs/openapi.json`: the HTTP API as `OpenAPI` 3.1, rendered from the same [`ENDPOINTS`]
//! registry that builds the routes and `docs/api.md`, so the three cannot drift. Responses are
//! described by their top-level fields only; the registry holds no response types.

use gitterrace_api::{ENDPOINTS, Endpoint, Param, insight_list};
use serde_json::{Map, Value, json};

use crate::facts::Workspace;
use crate::out::Out;

const PATH: &str = "docs/openapi.json";

pub fn render(ws: &Workspace) -> Out {
    let bin = ws.packages.get("gitterrace");
    let version = bin.map_or("0.0.0", |p| p.version.as_str());
    let license = bin.map_or("", |p| p.license.as_str());
    let mut paths = Map::new();
    for e in ENDPOINTS {
        paths.insert(e.path.to_owned(), json!({ "get": operation(e) }));
    }
    let doc = json!({
        "openapi": "3.1.0",
        "info": {
            "title": "gitterrace API",
            "version": version,
            "license": { "name": license, "identifier": license },
            "description": "Read-only and local. Every answer is pinned to a revision (`rev`, \
                default `HEAD`). Under several repositories each one's API lives at \
                `/r/{name}/api/v1`."
        },
        "servers": [{ "url": "http://127.0.0.1:4242/api/v1", "description": "the default address" }],
        // Loopback only, read-only, no credentials: an empty requirement says so explicitly.
        "security": [],
        "paths": paths,
        "components": {
            "schemas": {
                "Error": {
                    "type": "object",
                    "required": ["error"],
                    "properties": { "error": {
                        "type": "object",
                        "required": ["code", "message"],
                        "properties": {
                            "code": { "type": "string", "enum": ["bad_request", "rev_not_found", "path_not_found"] },
                            "message": { "type": "string" }
                        }
                    }}
                }
            },
            "responses": {
                "Error": {
                    "description": "Bad request or not found.",
                    "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Error" } } }
                }
            }
        }
    });
    let mut body = serde_json::to_string_pretty(&doc).unwrap_or_default();
    body.push('\n');
    Out::file(PATH, body)
}

fn operation(e: &Endpoint) -> Value {
    let mut params: Vec<Value> = path_params(e.path)
        .map(|name| {
            json!({ "name": name, "in": "path", "required": true, "schema": { "type": "string" },
                    "description": path_param_doc(name) })
        })
        .collect();
    params.extend(e.params.iter().map(query_param));
    let fields: Map<String, Value> = top_fields(e.returns)
        .map(|f| (f.to_owned(), json!({})))
        .collect();
    json!({
        "summary": e.summary,
        "operationId": operation_id(e.path),
        "parameters": params,
        "responses": {
            "200": {
                "description": e.returns,
                "headers": { "ETag": { "schema": { "type": "string" } } },
                "content": { "application/json": { "schema": { "type": "object", "properties": fields } } }
            },
            "304": { "description": "Not modified (`If-None-Match` matched the `ETag`)." },
            "400": { "$ref": "#/components/responses/Error" },
            "404": { "$ref": "#/components/responses/Error" }
        }
    })
}

fn query_param(p: &Param) -> Value {
    json!({
        "name": p.name,
        "in": "query",
        "required": p.required,
        "description": p.description,
        "schema": { "type": p.ty }
    })
}

/// `{id}` and `{name}` segments of a route.
fn path_params(path: &str) -> impl Iterator<Item = &str> {
    path.split('/')
        .filter_map(|s| s.strip_prefix('{').and_then(|s| s.strip_suffix('}')))
}

fn path_param_doc(name: &str) -> String {
    match name {
        "name" => {
            let names: Vec<&str> = insight_list().map(|(n, _)| n).collect();
            format!("Insight name, one of: {}.", names.join(", "))
        }
        "id" => "A commit id or any revision git understands.".to_owned(),
        other => format!("The `{other}` segment."),
    }
}

/// The field names of a `returns` line: `lines (commit, author, time), truncated` gives
/// `lines` and `truncated`.
fn top_fields(returns: &str) -> impl Iterator<Item = &str> {
    let mut depth = 0u32;
    let mut out = Vec::new();
    let mut start = 0;
    for (i, c) in returns.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                out.push(&returns[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&returns[start..]);
    out.into_iter()
        .map(|f| f.split('(').next().unwrap_or(f).trim())
        .filter(|f| !f.is_empty())
}

/// `GET /commits/{id}` becomes `commits_id`; `/` becomes `index`.
fn operation_id(path: &str) -> String {
    let id: String = path
        .trim_matches('/')
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    if id.is_empty() {
        "index".to_owned()
    } else {
        id
    }
}

#[cfg(test)]
mod tests {
    use crate::openapi_json::{operation_id, path_params, top_fields};

    #[test]
    fn fields_ids_and_path_params() {
        let f: Vec<&str> = top_fields("lines (commit, author, time), truncated").collect();
        assert_eq!(f, ["lines", "truncated"]);
        assert_eq!(operation_id("/"), "index");
        assert_eq!(operation_id("/commits/{id}"), "commits_id");
        assert_eq!(
            path_params("/insights/{name}").collect::<Vec<_>>(),
            ["name"]
        );
    }

    #[test]
    fn every_registry_endpoint_is_a_path() {
        let ws = crate::facts::load(&crate::repo_root()).expect("facts");
        let crate::out::Out::File { body, .. } = super::render(&ws) else {
            panic!("not a file");
        };
        let doc: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(doc["openapi"], "3.1.0");
        for e in gitterrace_api::ENDPOINTS {
            assert!(
                doc["paths"][e.path]["get"].is_object(),
                "{} missing",
                e.path
            );
        }
    }
}
