# Landing page

The words on gitterrace.dev's front page, one section per block. The site
builds the page from this file, and from the package description for the
tagline. Each sentence lives here once.

## Why

Read a repository the way a reviewer will, before you push it. Review what your
agents wrote, then pass each finding on with its context.

## What it does with your code

- **Local.** Serves on `127.0.0.1:4242`, so only your machine reaches it.
- **Read-only.** Never changes your files or history.
- **No telemetry.** No update check, nothing loaded from outside.
- **Open source.** Read the [security model](security.md).

## Hand-off

A finding is plain text with a source line, so it goes to the agent, editor,
tracker or script you already use. One entry in `actions.toml` adds a target.

```toml
[[action]]
id      = "claude"
label   = "Ask Claude Code"
command = ["~/.config/gitterrace/agent.sh", "claude"]
stdin   = "markdown"
```

Agents can also read the repository themselves, over a read-only MCP server.
Register it once.

```bash
claude mcp add gitterrace -- git terrace mcp --served
```
