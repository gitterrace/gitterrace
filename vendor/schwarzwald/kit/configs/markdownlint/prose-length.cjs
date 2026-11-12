// prose-length: prose wraps at a column limit, but an inline code span or a link target cannot
// wrap (the reader copies it whole), so a line is too long only when its text outside code spans
// and link targets still exceeds the limit. Fences, code blocks, tables, HTML blocks, headings and
// link definitions are exempt. Replaces markdownlint's MD013, which counts code spans.
// Limit: "prose-length": { "line_length": 80 } in the product's config. Default 80.
"use strict";

module.exports = {
  names: ["prose-length"],
  description: "Prose line longer than the limit (code spans and link targets not counted)",
  tags: ["line_length"],
  parser: "markdownit",
  function: (params, onError) => {
    const limit = (params.config && params.config.line_length) || 80;
    const skip = new Set();
    // markdown-it block tokens carry map = [startLine0, endLine0Exclusive].
    for (const t of params.parsers.markdownit.tokens) {
      if (t.map && ["fence", "code_block", "table_open", "html_block"].includes(t.type)) {
        for (let l = t.map[0] + 1; l <= t.map[1]; l++) skip.add(l);
      }
    }
    params.lines.forEach((line, i) => {
      const n = i + 1;
      if (skip.has(n) || line.length <= limit) return;
      if (/^\s*#{1,6}\s/.test(line)) return;
      if (/^\s*\[[^\]]+\]:\s/.test(line)) return;
      const prose = line.replace(/`[^`]*`/g, "").replace(/\]\([^)]*\)/g, "]");
      if (prose.length > limit) {
        onError({ lineNumber: n, detail: `Expected: ${limit}; Actual: ${prose.length} outside code spans and link targets` });
      }
    });
  },
};
