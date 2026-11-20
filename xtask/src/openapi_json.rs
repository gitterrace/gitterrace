//! `docs/openapi.json`: the HTTP API as `OpenAPI` 3.1, rendered from the same [`ENDPOINTS`]
//! registry that builds the routes and `docs/api.md`, so the three cannot drift. Each response
//! schema comes from the type its handler returns, under `components/schemas`.

use gitterrace_api::{ENDPOINTS, ERROR_CODES, Endpoint, Param, insight_list, schema_generator};
use schemars::SchemaGenerator;
use serde_json::{Map, Value, json};

use crate::facts::Workspace;
use crate::out::Out;

const PATH: &str = "docs/openapi.json";

pub fn render(ws: &Workspace) -> Out {
    let bin = ws.packages.get("gitterrace");
    let version = bin.map_or("0.0.0", |p| p.version.as_str());
    let license = bin.map_or("", |p| p.license.as_str());
    let codes: Vec<&str> = ERROR_CODES.iter().map(|(c, _, _)| *c).collect();
    let mut paths = Map::new();
    let mut generator = schema_generator();
    for e in ENDPOINTS {
        paths.insert(e.path.to_owned(), json!({ "get": operation(e, &mut generator) }));
    }
    let mut schemas = Map::new();
    schemas.insert("Error".to_owned(), error_schema(&codes));
    schemas.extend(generator.take_definitions(false));
    let doc = json!({
        "openapi": "3.1.0",
        "info": {
            "title": "gitterrace API",
            "version": version,
            "license": { "name": license, "identifier": license },
            "description": "Read-only and local. Every answer is pinned to a revision (`rev`, \
                default `HEAD`). Each repository's API lives at `/r/{name}/api/v1`."
        },
        "servers": [{
            "url": format!("http://{}/r/{{name}}/api/v1", gitterrace_config::DEFAULT_ADDR),
            "description": "the default address",
            "variables": { "name": { "default": "repo", "description": "the repository's URL \
            name" } }
        }],
        // Loopback only, read-only, no credentials: an empty requirement says so explicitly.
        "security": [],
        "paths": paths,
        "components": {
            "schemas": schemas,
            "responses": {
                "Error": {
                    "description": "Bad request, not found or server error. See `code`.",
                    "content": { "application/json": {
                        "schema": { "$ref": "#/components/schemas/Error" }
                    } }
                }
            }
        }
    });
    let mut body = serde_json::to_string_pretty(&doc).unwrap_or_default();
    body.push('\n');
    Out::file(PATH, body)
}

fn error_schema(codes: &[&str]) -> Value {
    json!({
        "type": "object",
        "required": ["error"],
        "properties": { "error": {
            "type": "object",
            "required": ["code", "message"],
            "properties": {
                "code": { "type": "string", "enum": codes },
                "message": { "type": "string" }
            }
        }}
    })
}

fn operation(e: &Endpoint, generator: &mut SchemaGenerator) -> Value {
    let mut params: Vec<Value> = path_params(e.path)
        .map(|name| {
            json!({ "name": name, "in": "path", "required": true, "schema": { "type": "string" },
                    "description": path_param_doc(name) })
        })
        .collect();
    params.extend(e.params.iter().map(query_param));
    let schema = (e.schema)(generator);
    let returns = e.returns(generator);
    let mut op = json!({
        "summary": e.summary,
        "operationId": operation_id(e.path),
        "parameters": params,
        "responses": {
            "200": {
                "description": returns,
                "headers": { "ETag": { "schema": { "type": "string" } } },
                "content": { "application/json": { "schema": schema } }
            },
            "304": { "description": "Not modified (`If-None-Match` matched the `ETag`)." }
        }
    });
    let mut statuses: Vec<u16> = ERROR_CODES.iter().map(|(_, s, _)| *s).collect();
    statuses.dedup();
    for s in statuses {
        op["responses"][s.to_string()] = json!({ "$ref": "#/components/responses/Error" });
    }
    op
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
    path.split('/').filter_map(|s| s.strip_prefix('{').and_then(|s| s.strip_suffix('}')))
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
    if id.is_empty() { "index".to_owned() } else { id }
}

#[cfg(test)]
mod tests {
    use crate::openapi_json::{operation_id, path_params};

    #[test]
    fn ids_and_path_params() {
        assert_eq!(operation_id("/"), "index");
        assert_eq!(operation_id("/commits/{id}"), "commits_id");
        assert_eq!(path_params("/insights/{name}").collect::<Vec<_>>(), ["name"]);
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
            assert!(doc["paths"][e.path]["get"].is_object(), "{} missing", e.path);
        }
    }
}
