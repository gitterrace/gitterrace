//! Ahead and behind counts from the commit-graph file, checked against the walks.

/// `(ahead, behind)` of `tip` against `base` in one pass over the commit-graph: commits are
/// taken highest generation first, so each carries its final marks (reached from the tip, from
/// the base, or both) when taken, and the pass stops once every queued commit is reached from
/// both. Two hidden walks read every commit down to where gix proves the sides met, about a
/// quarter second each on rust; this reads only the commits that differ plus the frontier.
/// `None` when either tip is missing from the graph (written before it), so the caller walks.
pub(crate) fn graph_divergence(
    graph: &gix::commitgraph::Graph,
    tip: gix::ObjectId,
    base: gix::ObjectId,
) -> Option<(usize, usize)> {
    use std::collections::{BinaryHeap, HashMap};
    const TIP: u8 = 1;
    const BASE: u8 = 2;
    if tip == base {
        return Some((0, 0));
    }
    let start = [(graph.lookup(tip)?, TIP), (graph.lookup(base)?, BASE)];
    let mut marks: HashMap<gix::commitgraph::Position, u8> = HashMap::new();
    let mut heap = BinaryHeap::new();
    // Queued commits marked from one side only: the pass ends when none is left.
    let mut one_sided = 0usize;
    for (pos, mark) in start {
        marks.insert(pos, mark);
        heap.push((graph.commit_at(pos).generation(), pos.0));
        one_sided += 1;
    }
    let (mut ahead, mut behind) = (0, 0);
    while one_sided > 0 {
        let (_, raw) = heap.pop()?;
        let pos = gix::commitgraph::Position(raw);
        let mark = marks.get(&pos).copied().unwrap_or(0);
        if mark != TIP | BASE {
            one_sided -= 1;
        }
        match mark {
            TIP => ahead += 1,
            BASE => behind += 1,
            _ => {}
        }
        for parent in graph.commit_at(pos).iter_parents() {
            let parent = parent.ok()?;
            let seen = marks.entry(parent).or_insert(0);
            let before = *seen;
            *seen |= mark;
            if before == 0 {
                heap.push((graph.commit_at(parent).generation(), parent.0));
                if *seen != TIP | BASE {
                    one_sided += 1;
                }
            } else if before != TIP | BASE && *seen == TIP | BASE {
                one_sided -= 1;
            }
        }
    }
    Some((ahead, behind))
}

#[cfg(test)]
mod tests {
    use gitterrace_testkit::git;

    use crate::Repo;
    use crate::gitdir::tests::{base, commit};

    /// The commit-graph pass counts what the two hidden walks count, on merges, criss-cross
    /// histories and branches with no common commit.
    #[test]
    fn graph_divergence_matches_the_walk() {
        let (_t, d) = base();
        git(&d, &["checkout", "-qb", "a"]);
        commit(&d, "a1", "1\n");
        git(&d, &["checkout", "-qb", "b", "main"]);
        commit(&d, "b1", "1\n");
        git(&d, &["merge", "-q", "--no-edit", "a"]);
        commit(&d, "b2", "1\n");
        git(&d, &["checkout", "-q", "a"]);
        git(&d, &["merge", "-q", "--no-edit", "main"]);
        commit(&d, "a2", "1\n");
        git(&d, &["checkout", "-q", "main"]);
        commit(&d, "m1", "1\n");
        git(&d, &["merge", "-q", "--no-edit", "b"]);
        git(&d, &["checkout", "-q", "--orphan", "lone"]);
        commit(&d, "l1", "1\n");
        git(&d, &["checkout", "-q", "main"]);
        git(&d, &["commit-graph", "write", "--reachable"]);
        let repo = Repo::discover(&d).expect("open");
        let r = repo.repo();
        let graph = r.commit_graph().expect("graph");
        let names = ["main", "a", "b", "lone", "main~1", "b~1", "a~2"];
        let id = |n: &str| r.rev_parse_single(n).expect(n).detach();
        for x in names {
            for y in names {
                let walked = (
                    crate::gitdir::count_only(&r, id(x), id(y)).expect("walk"),
                    crate::gitdir::count_only(&r, id(y), id(x)).expect("walk"),
                );
                assert_eq!(
                    super::graph_divergence(&graph, id(x), id(y)),
                    Some(walked),
                    "{x} against {y}"
                );
            }
        }
    }
}
