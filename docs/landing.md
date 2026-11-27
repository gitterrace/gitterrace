# Landing page

The words on gitterrace.dev's front page, one section per block. The site
builds the page from this file, and from the package description for the
tagline. Each sentence lives here once.

## Why

See what your agents wrote, with its history, blame and insights, on your own
machine. No account, no database, no network.

## What it does with your code

- **Local.** Serves on `127.0.0.1:4242`, so only your machine reaches it.
- **Read-only.** Never changes your files or history.
- **No telemetry.** No update check, nothing loaded from outside.
- **Open source.** Read the [security model](security.md).

## Hand-off

A finding is plain text with a source line, so it goes to the agent, editor,
tracker or script you already use.

One entry in `actions.toml` adds a target.

<!-- BEGIN GENERATED: example-claude -->
```toml
[[action]]
id      = "ask"
label   = "Ask Claude"
kinds   = ["line"]
command = ["~/.config/gitterrace/agent.sh", "claude"]
```

<!-- END GENERATED: example-claude -->

Agents can also read the repository themselves, over a read-only MCP server.
Register it once.

```bash
claude mcp add gitterrace -- git terrace mcp --served
```
