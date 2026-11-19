//! The tool and prompt registries as documentation data for the API reference.

use serde::Deserialize;

use crate::mcp::McpServer;

/// One MCP tool parameter, from the tool's input schema.
#[derive(Debug)]
pub struct Param {
    pub name: String,
    pub ty: String,
    pub required: bool,
    pub description: String,
}

/// One MCP tool as the API reference shows it.
#[derive(Debug)]
pub struct ToolDoc {
    pub name: String,
    pub description: String,
    pub params: Vec<Param>,
    pub read_only: bool,
}

/// The parts of a tool's input schema the docs read.
#[derive(Default, Deserialize)]
struct InputSchema {
    #[serde(default)]
    required: Vec<String>,
    #[serde(default)]
    properties: std::collections::BTreeMap<String, PropSchema>,
}

#[derive(Deserialize)]
struct PropSchema {
    #[serde(default, rename = "type")]
    ty: Option<SchemaType>,
    #[serde(default)]
    description: Option<String>,
}

/// A property's JSON type. `Option<T>` fields come as `["integer", "null"]`.
#[derive(Deserialize)]
#[serde(untagged)]
enum SchemaType {
    One(String),
    Many(Vec<String>),
}

impl PropSchema {
    fn type_name(&self) -> String {
        match &self.ty {
            Some(SchemaType::One(t)) => t.clone(),
            Some(SchemaType::Many(ts)) => ts
                .iter()
                .find(|t| *t != "null")
                .map_or_else(|| "any".to_owned(), Clone::clone),
            None => "any".to_owned(),
        }
    }
}

impl McpServer {
    /// Every tool as documentation data, sorted by name, read from the live registry.
    pub fn tool_docs() -> Vec<ToolDoc> {
        let mut docs: Vec<ToolDoc> = Self::tool_router()
            .list_all()
            .into_iter()
            .map(|t| {
                let schema = InputSchema::deserialize(&*t.input_schema).unwrap_or_default();
                let mut params: Vec<Param> = schema
                    .properties
                    .iter()
                    .map(|(name, prop)| Param {
                        name: name.clone(),
                        ty: prop.type_name(),
                        required: schema.required.contains(name),
                        description: prop.description.clone().unwrap_or_default(),
                    })
                    .collect();
                params.sort_by(|a, b| a.name.cmp(&b.name));
                ToolDoc {
                    name: t.name.to_string(),
                    description: t.description.as_deref().unwrap_or_default().to_owned(),
                    params,
                    read_only: t
                        .annotations
                        .as_ref()
                        .and_then(|a| a.read_only_hint)
                        .unwrap_or(false),
                }
            })
            .collect();
        docs.sort_by(|a, b| a.name.cmp(&b.name));
        docs
    }

    /// Every prompt as documentation data, sorted by name, read from the live registry. A
    /// prompt reads nothing itself, so `read_only` is always true.
    pub fn prompt_docs() -> Vec<ToolDoc> {
        let mut docs: Vec<ToolDoc> = Self::prompt_router()
            .list_all()
            .into_iter()
            .map(|p| {
                let mut params: Vec<Param> = p
                    .arguments
                    .unwrap_or_default()
                    .into_iter()
                    .map(|a| Param {
                        name: a.name,
                        ty: "string".to_owned(),
                        required: a.required.unwrap_or(false),
                        description: a.description.unwrap_or_default(),
                    })
                    .collect();
                params.sort_by(|a, b| a.name.cmp(&b.name));
                ToolDoc {
                    name: p.name,
                    description: p.description.unwrap_or_default(),
                    params,
                    read_only: true,
                }
            })
            .collect();
        docs.sort_by(|a, b| a.name.cmp(&b.name));
        docs
    }
}
