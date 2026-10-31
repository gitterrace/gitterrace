# API

Read-only. Everything is pinned to a revision (`rev`, default `HEAD`). Errors are
`{"error": {"code", "message"}}` with 400 (`bad_request`) or 404 (`rev_not_found`,
`path_not_found`). Every response carries `ETag: "<head commit>-<index version>"` and honours
`If-None-Match` with 304. JSON is `snake_case`.

Rust: `gitterrace_api::router(ApiState)` (nest under `/api/v1`), `gitterrace_api::serve_mcp(path)`.

## HTTP (`/api/v1`)

| Endpoint | Query | Returns |
| --- | --- | --- |
| `GET /` | | endpoint and insight list |
| `GET /repo` | | name, head, head_branch, totals |
| `GET /refs` | | branches, tags, remotes |
| `GET /log` | `rev`, `path`, `page`, `per_page` (max 500) | commits, has_more |
| `GET /commits/{id}` | `diff` (default true), `max_lines` | message, files, hunks, truncated |
| `GET /tree` | `rev`, `path` | directory entries |
| `GET /blob` | `rev`, `path`, `start_line`, `limit` | text, total_lines, truncated, binary |
| `GET /blame` | `rev`, `path` | per-line commit, author, time |
| `GET /contributors` | `rev` | authors, bus_factor |
| `GET /insights` | | available insights |
| `GET /insights/{name}` | `rev`, `granularity`, `label`, `since`, `limit` | `{insight, data}` |

Insights: `totals`, `activity`, `heatmap`, `authors`, `file_churn`, `hotspots`, `dir_churn`,
`label_breakdown`, `label_over_time`. Add one by appending to `INSIGHTS` in
`crates/gitterrace-api/src/insights.rs`.

## MCP (stdio, all tools read-only)

| Tool | Use |
| --- | --- |
| `repo_overview` | first call: name, HEAD, branch, totals |
| `log` | history, optional `path`, paged |
| `show_commit` | message, files, capped diff |
| `read_file` | file text at a revision, line window |
| `list_tree` | one directory level |
| `blame` | per-line authorship |
| `hotspots` | most changed files, optional `since` |
| `contributors` | authors and bus factor |
| `activity` | commits per day, week or month |
