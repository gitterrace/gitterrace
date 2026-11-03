# data-line: oracle classification

The Markdown renderer adds `data-line="a-b"` to block elements. Classified against a9b0c7c,
bodies compared after stripping ` data-line="\d+-\d+"`:

| Set | Bodies | Identical | Equal after strip | Real diffs |
| --- | --- | --- | --- | --- |
| `equivalence.txt` and `revisions.txt` (TEST_ORACLE_DUMP, old vs new) | 25000 | 25000 | 0 | 0 |
| every tracked `*.md` rendered by `render_with` | 18 | 0 | 18 (566 spans) | 0 |

The golden pages are insights pages and APIs. None renders Markdown, so no digest moved,
both goldens pass unchanged, and nothing was re-blessed.
