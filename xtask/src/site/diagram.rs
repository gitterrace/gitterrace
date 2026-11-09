//! Mermaid at build time: the site renders the `flowchart` the architecture page carries as an
//! inline SVG, from `templates/diagram.html`, so the page needs no diagram script and the CSP
//! stays as tight as it is. Only the shape the crate diagram is written in is understood
//! (`a["label"]` nodes and `a --> b` edges); any other mermaid block fails generation instead
//! of showing as source.

use std::collections::BTreeMap;

use anyhow::{Context as _, Result};
use askama::Template;

/// One box: its two lines, the name and the role after `: `.
struct Node {
    name: String,
    role: String,
}

const BOX_H: u32 = 44;
const ROW_GAP: u32 = 40;
const COL_GAP: u32 = 16;
const PAD: u32 = 8;
/// Pixels per character of the 12 px mono label, plus the box's inner margin.
const CHAR_W: u32 = 8;
const MARGIN: u32 = 20;

#[derive(Template)]
#[template(path = "diagram.html")]
struct Svg<'a> {
    id: &'a str,
    title: &'a str,
    width: u32,
    height: u32,
    box_h: u32,
    edges: Vec<(u32, u32, u32, u32, u32)>,
    nodes: Vec<(u32, u32, u32, &'a str, &'a str)>,
}

/// Edges as (from, to) node indices.
type Edges = Vec<(usize, usize)>;

/// The parsed flowchart: nodes in source order and edges as (from, to) indices.
fn parse(src: &str) -> Result<(Vec<Node>, Edges)> {
    let mut lines = src.lines().map(str::trim).filter(|l| !l.is_empty());
    let head = lines.next().unwrap_or("");
    anyhow::ensure!(
        head == "flowchart TD",
        "a mermaid block that is not `flowchart TD` (the site renders only that): {head}"
    );
    let mut nodes: Vec<Node> = Vec::new();
    let mut edges = Vec::new();
    let mut index = BTreeMap::new();
    for line in lines {
        if let Some((from, to)) = line.split_once(" --> ") {
            let at = |id: &str| index.get(id.trim()).copied();
            let (Some(a), Some(b)) = (at(from), at(to)) else {
                anyhow::bail!("mermaid edge to an undeclared node: {line}");
            };
            edges.push((a, b));
        } else if let Some((id, rest)) = line.split_once("[\"") {
            let label = rest
                .strip_suffix("\"]")
                .context("mermaid node without `\"]`")?;
            let (name, role) = label.split_once(": ").unwrap_or((label, ""));
            index.insert(id.to_owned(), nodes.len());
            nodes.push(Node {
                name: name.to_owned(),
                role: role.to_owned(),
            });
        } else {
            anyhow::bail!("a mermaid line the site cannot draw: {line}");
        }
    }
    Ok((nodes, edges))
}

/// The layer of each node: the longest path from a node nothing uses, so every arrow points
/// down at least one row.
fn layers(n: usize, edges: &[(usize, usize)]) -> Result<Vec<usize>> {
    let mut layer = vec![0; n];
    for _ in 0..=n {
        let mut moved = false;
        for &(a, b) in edges {
            if layer[b] <= layer[a] {
                layer[b] = layer[a] + 1;
                moved = true;
            }
        }
        if !moved {
            return Ok(layer);
        }
    }
    anyhow::bail!("the mermaid flowchart has a cycle")
}

/// The flowchart `src` as an inline SVG figure titled `title`. `id` keeps its ids unique on
/// the page.
pub(super) fn svg(src: &str, id: &str, title: &str) -> Result<String> {
    let (nodes, edges) = parse(src)?;
    let layer = layers(nodes.len(), &edges)?;
    let rows = layer.iter().max().map_or(0, |m| m + 1);
    let width_of = |n: &Node| {
        let chars = n.name.chars().count().max(n.role.chars().count());
        u32::try_from(chars)
            .unwrap_or(u32::MAX)
            .saturating_mul(CHAR_W)
            + MARGIN
    };
    let mut row_w = vec![0; rows];
    for (i, n) in nodes.iter().enumerate() {
        row_w[layer[i]] += width_of(n) + COL_GAP;
    }
    let width = row_w.iter().max().copied().unwrap_or(0) - COL_GAP + 2 * PAD;
    let mut place = vec![(0, 0, 0); nodes.len()];
    let mut cursor: Vec<u32> = row_w
        .iter()
        .map(|w| PAD + (width - 2 * PAD - (w - COL_GAP)) / 2)
        .collect();
    for (i, n) in nodes.iter().enumerate() {
        let r = layer[i];
        let w = width_of(n);
        let y = PAD + u32::try_from(r).unwrap_or(0) * (BOX_H + ROW_GAP);
        place[i] = (cursor[r], y, w);
        cursor[r] += w + COL_GAP;
    }
    let height = PAD * 2 + u32::try_from(rows).unwrap_or(0) * (BOX_H + ROW_GAP) - ROW_GAP;
    let edges = edges
        .iter()
        .map(|&(a, b)| {
            let (ax, ay, aw) = place[a];
            let (bx, by, bw) = place[b];
            let (y1, y2) = (ay + BOX_H, by - 2);
            (ax + aw / 2, y1, bx + bw / 2, y2, u32::midpoint(y1, y2))
        })
        .collect();
    let nodes = nodes
        .iter()
        .zip(&place)
        .map(|(n, &(x, y, w))| (x, y, w, n.name.as_str(), n.role.as_str()))
        .collect();
    let html = Svg {
        id,
        title,
        width,
        height,
        box_h: BOX_H,
        edges,
        nodes,
    }
    .render()?;
    Ok(format!("{html}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "flowchart TD\n  a[\"a: top\"]\n  b[\"b: mid\"]\n  c[\"c: base & co\"]\n\
                       a --> b\n  b --> c\n  a --> c\n";

    #[test]
    fn every_node_is_a_box_every_edge_an_arrow_and_arrows_point_down() {
        let s = svg(SRC, "d", "Crates").expect("svg");
        assert_eq!(s.matches("<rect").count(), 3, "{s}");
        assert_eq!(s.matches("marker-end").count(), 3, "{s}");
        assert!(
            s.contains("base &amp; co") || s.contains("base &#38; co"),
            "{s}"
        );
        assert!(s.lines().all(|l| l.chars().count() <= 100), "{s}");
        assert_eq!(
            layers(3, &[(0, 1), (1, 2), (0, 2)]).expect("layers"),
            [0, 1, 2]
        );
    }

    #[test]
    fn a_mermaid_block_the_site_cannot_draw_fails() {
        for bad in [
            "sequenceDiagram\n  a->>b: hi\n",
            "flowchart TD\n  a[\"a\"]\n  a --> z\n",
            "flowchart TD\n  a[\"a\"]\n  b[\"b\"]\n  a --> b\n  b --> a\n",
            "flowchart TD\n  a((round))\n",
        ] {
            assert!(svg(bad, "d", "t").is_err(), "{bad}");
        }
    }
}
