//! Facts read from the workspace through `cargo metadata`: the one place xtask learns crate
//! names, descriptions, binaries and dependency edges, so no generator keeps its own list.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

pub struct Package {
    pub name: String,
    pub description: String,
    pub rust_version: String,
    pub version: String,
    pub license: String,
    /// The `repository` URL, from the workspace for every member.
    pub repository: String,
    /// The `homepage` URL: the website, which the site canonicals to.
    pub homepage: String,
    pub bins: Vec<String>,
    /// Workspace members this package depends on (normal dependencies only).
    pub deps: BTreeSet<String>,
}

pub struct Workspace {
    pub packages: BTreeMap<String, Package>,
    /// The Homebrew tap the release publishes to (`owner/homebrew-tap`), from
    /// `[workspace.metadata.dist]`; empty when there is none.
    pub tap: String,
    /// The release's target triples, from `[workspace.metadata.dist] targets`.
    pub targets: Vec<String>,
    /// The archive Windows releases ship as (`.zip`), from `[workspace.metadata.dist]
    /// windows-archive`; empty when there is none.
    pub windows_archive: String,
    /// The live demo's URL, from `[workspace.metadata.demo]`; empty when there is none. The
    /// demo itself is built in github.com/gitterrace/demo.
    pub demo: String,
    /// The repositories the demo shows, `(name, tag)` from `[workspace.metadata.demo] repos`.
    pub demo_repos: Vec<(String, String)>,
}

pub fn load(root: &Path) -> Result<Workspace> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args([
            "metadata",
            "--no-deps",
            "--offline",
            "--format-version",
            "1",
        ])
        .current_dir(root)
        .output()
        .context("running cargo metadata")?;
    if !out.status.success() {
        bail!("cargo metadata: {}", String::from_utf8_lossy(&out.stderr));
    }
    let meta: Value = serde_json::from_slice(&out.stdout).context("cargo metadata output")?;
    let list = meta["packages"].as_array().context("packages")?;
    let members: BTreeSet<String> = list
        .iter()
        .filter_map(|p| p["name"].as_str().map(str::to_owned))
        .collect();
    let mut packages = BTreeMap::new();
    for p in list {
        let name = p["name"].as_str().context("name")?.to_owned();
        let text = |k: &str| p[k].as_str().unwrap_or_default().to_owned();
        let bins = p["targets"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|t| {
                t["kind"]
                    .as_array()
                    .is_some_and(|k| k.iter().any(|k| k == "bin"))
            })
            .filter_map(|t| t["name"].as_str().map(str::to_owned))
            .collect();
        let deps = p["dependencies"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| d["kind"].is_null())
            .filter_map(|d| d["name"].as_str())
            .filter(|n| members.contains(*n))
            .map(str::to_owned)
            .collect();
        packages.insert(
            name.clone(),
            Package {
                name,
                description: text("description"),
                rust_version: text("rust_version"),
                version: text("version"),
                license: text("license"),
                repository: text("repository"),
                homepage: text("homepage"),
                bins,
                deps,
            },
        );
    }
    let tap = meta["metadata"]["dist"]["tap"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let dist = &meta["metadata"]["dist"];
    let targets = dist["targets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let demo = &meta["metadata"]["demo"];
    Ok(Workspace {
        packages,
        tap,
        targets,
        windows_archive: dist["windows-archive"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        demo: demo["url"].as_str().unwrap_or_default().to_owned(),
        demo_repos: demo_repos(demo),
    })
}

/// `"name tag"` entries of `[workspace.metadata.demo] repos`, in order.
fn demo_repos(demo: &Value) -> Vec<(String, String)> {
    demo["repos"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|s| {
            let (name, tag) = s.split_once(' ')?;
            Some((name.to_owned(), tag.trim().to_owned()))
        })
        .collect()
}

impl Workspace {
    /// The shipped crates: every member but `xtask`.
    pub fn crates(&self) -> impl Iterator<Item = &Package> {
        self.packages.values().filter(|p| p.name != "xtask")
    }

    /// Dependency edges with the transitive ones removed, so the diagram shows the layers and
    /// not every pair. `(from, to)` means `from` depends on `to`.
    pub fn reduced_edges(&self) -> Vec<(String, String)> {
        let reaches = |from: &str, skip: &str, to: &str| -> bool {
            let mut seen = BTreeSet::new();
            let mut stack: Vec<&str> = self.packages[from]
                .deps
                .iter()
                .map(String::as_str)
                .filter(|d| *d != skip)
                .collect();
            while let Some(n) = stack.pop() {
                if n == to {
                    return true;
                }
                if seen.insert(n)
                    && let Some(p) = self.packages.get(n)
                {
                    stack.extend(p.deps.iter().map(String::as_str));
                }
            }
            false
        };
        let mut out = Vec::new();
        for p in self.crates() {
            for d in &p.deps {
                if !reaches(&p.name, d, d) {
                    out.push((p.name.clone(), d.clone()));
                }
            }
        }
        out
    }
}
