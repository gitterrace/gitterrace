//! The registry of generated doc blocks: which block of which page comes from which module.
//! Pages: `docs/actions.md`, `docs/architecture.md`, `docs/benchmarks.md`, `docs/landing.md`,
//! `docs/mcp.md` and `docs/using.md`.

use std::path::Path;

use anyhow::Result;

use crate::action_scripts::Script;
use crate::docs_actions_md as actions;
use crate::docs_arch_md as arch;
use crate::docs_bench_md as bench;
use crate::docs_using_md as using;
use crate::facts::Workspace;
use crate::out::Out;
use crate::readme_agents_md::crate_diagram;

const ACTIONS: &str = "docs/actions.md";
const LANDING: &str = "docs/landing.md";
const ARCH: &str = "docs/architecture.md";
const BENCH: &str = "docs/benchmarks.md";
const MCP: &str = "docs/mcp.md";
const USING: &str = "docs/using.md";

pub fn render(root: &Path, ws: &Workspace, scripts: &[Script]) -> Result<Vec<Out>> {
    Ok(vec![
        Out::block(ACTIONS, "kinds", actions::kinds()),
        Out::block(ACTIONS, "payload", actions::payload()),
        Out::block(ACTIONS, "env", actions::env()),
        Out::block(ACTIONS, "action-keys", actions::action_keys()),
        Out::block(ACTIONS, "placeholders", actions::placeholders()),
        Out::block(ACTIONS, "starter", actions::starter()),
        Out::block(ACTIONS, "send", actions::send()),
        Out::block(ACTIONS, "scripts", actions::script_table(scripts)),
        Out::block(ACTIONS, "limits", actions::limits()),
        Out::block(ACTIONS, "source-line", actions::source_line()),
        Out::block(ACTIONS, "agents", actions::agents()),
        Out::block(ACTIONS, "windows", actions::windows()),
        Out::block(ACTIONS, "load-limits", actions::load_limits()),
        Out::block(ACTIONS, "example-send", actions::example(ACTIONS, "example-send")?),
        Out::block(ACTIONS, "example-webhook", actions::example(ACTIONS, "example-webhook")?),
        Out::block(ACTIONS, "example-search", actions::example(ACTIONS, "example-search")?),
        Out::block(
            ACTIONS,
            "example-agent-other",
            actions::example(ACTIONS, "example-agent-other")?,
        ),
        Out::block(ACTIONS, "example-script", actions::example(ACTIONS, "example-script")?),
        Out::block(LANDING, "example-claude", actions::example(LANDING, "example-claude")?),
        Out::block(ARCH, "crates", crate_diagram(ws)?),
        Out::block(ARCH, "routes", arch::routes()),
        Out::block(ARCH, "languages", arch::languages()),
        Out::block(ARCH, "cache", arch::cache()),
        Out::block(ARCH, "user-files", arch::user_files()),
        Out::block(BENCH, "corpus", bench::bench_report(root, "--corpus")?),
        Out::block(BENCH, "results", bench::bench_report(root, "--print")?),
        Out::block(USING, "log-filter", using::log_filter()),
        Out::block(MCP, "risk-terms", using::risk_terms()),
        Out::block(MCP, "prompt-args", using::prompt_args()),
    ])
}

#[cfg(test)]
mod tests {
    use super::render;
    use crate::docs_actions_md::TABLE_ROW_MAX;

    /// Blocks whose rows are still wider than `TABLE_ROW_MAX`, with their widest row today. A
    /// ratchet that only falls: a block may not get wider, and when it gets narrower the cap must
    /// be lowered to match (or the entry dropped once it fits). Any block not listed must fit.
    const STILL_WIDE: &[(&str, usize)] = &[
        ("send", 283),
        ("placeholders", 266),
        ("action-keys", 263),
        ("payload", 154),
        ("scripts", 322),
        ("cache", 165),
        ("routes", 270),
    ];
    use crate::facts;
    use crate::out::Out;

    /// No generated markdown table row is wider than `TABLE_ROW_MAX`, in any block. A long cell
    /// grows the page past its line cap once the rows wrap.
    #[test]
    fn generated_table_rows_stay_narrow() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let ws = facts::load(&root).expect("workspace facts");
        let scripts = crate::action_scripts::load(&root).expect("action scripts");
        let mut bad = Vec::new();
        for out in render(&root, &ws, &scripts).expect("render") {
            let Out::Block { path, id, body } = out else {
                continue;
            };
            let widest = body
                .lines()
                .filter(|l| l.starts_with('|'))
                .map(|l| l.chars().count())
                .max()
                .unwrap_or(0);
            let cap = STILL_WIDE
                .iter()
                .find(|(name, _)| *name == id)
                .map_or(TABLE_ROW_MAX, |(_, cap)| *cap);
            if widest > cap {
                let row = body.lines().find(|l| l.chars().count() == widest).unwrap_or("");
                bad.push(format!(
                    "{path} [{id}]: widest row is {widest} columns, cap {cap}: {}",
                    row.chars().take(40).collect::<String>()
                ));
            } else if cap > TABLE_ROW_MAX && widest < cap {
                bad.push(format!(
                    "{path} [{id}]: widest row shrank to {widest}, lower its STILL_WIDE cap from \
                     {cap} (drop the entry if it fits {TABLE_ROW_MAX})"
                ));
            }
        }
        assert!(bad.is_empty(), "generated table rows too wide:\n{}", bad.join("\n"));
    }
}
