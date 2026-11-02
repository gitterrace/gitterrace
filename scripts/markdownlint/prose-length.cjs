// GITTERRACE-MD013-PROSE: prose wraps at 100 columns, but an inline code span cannot
// wrap (it is a command, a path or a symbol the reader copies whole), so a line
// is too long only when its text OUTSIDE code spans would still exceed 100.
// This replaces markdownlint's MD013, which counts code spans and so forced a
// `markdownlint-disable-next-line` above every long command.
"use strict";

const LIMIT = 100;

module.exports = {
  names: ["GITTERRACE-MD013-PROSE", "prose-line-length"],
  description: "Prose line longer than 100 columns (inline code spans not counted)",
  tags: ["line_length"],
  parser: "markdownit",
  function: (params, onError) => {
    const skip = new Set();
    // markdown-it block tokens carry map = [startLine0, endLine0Exclusive].
    for (const t of params.parsers.markdownit.tokens) {
      if (t.map && ["fence", "code_block", "table_open", "html_block"].includes(t.type)) {
        for (let l = t.map[0] + 1; l <= t.map[1]; l++) skip.add(l);
      }
    }
    // params.lines and the token maps both exclude front matter.
    params.lines.forEach((line, i) => {
      const n = i + 1;
      if (skip.has(n) || line.length <= LIMIT) return;
      if (/^\s*#{1,6}\s/.test(line)) return; // headings: MD013 had headings off
      if (/^\s*\[[^\]]+\]:\s/.test(line)) return; // link reference definitions
      const prose = line.replace(/`[^`]*`/g, "").replace(/\]\([^)]*\)/g, "]");
      if (prose.length > LIMIT) {
        onError({ lineNumber: n, detail: `Expected: ${LIMIT}; Actual: ${prose.length} outside code spans and link targets` });
      }
    });
  },
};
