---
name: find-hotspots
description: Find risky or central code and the people who know it, with the gitterrace MCP.
---

# Find hotspots

Copy this into your agent's skills folder to find risky code. It needs the
[gitterrace MCP server](../api.md#mcp-stdio).

## Steps

1. Call `hotspots` with `period` (`1w`, `1m`, `3m`, `1y` or `all`) and
   `limit`. The score is changes times the square root of lines.
2. Call `contributors` for ownership and `bus_factor`, the fewest people who
   made half of all commits.
3. For the top files, call `log` with `path` to see why they change, and
   `blame` to see who holds the lines now.
4. Call `activity` to see whether the project is speeding up or slowing down.
5. Report a ranked list. Each entry holds the file, its score, its owners and
   one reason it changes.

A high score is a question and not a verdict: a changelog or a lockfile
scores high and is harmless. Say what the numbers leave out.
