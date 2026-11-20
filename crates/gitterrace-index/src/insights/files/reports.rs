//! Hotspots, ownership, coupling and code age over the followed file history.

use std::collections::BTreeMap;

use gitterrace_git::civil_from_days;
use rustc_hash::FxHashMap as FastMap;

use crate::feed::{PathId, local_day_of};
use crate::insights::files::{
    AgeBucket, AgeReport, COUPLING_MAX_FILES, Coupling, DirOwnership, Hotspot, HotspotReport,
    Owner, OwnershipReport, SOLO_SHARE, STALE_SECS,
};
use crate::insights::{Counted, HeadFile, Insights, Period};

impl Insights<'_> {
    /// Hotspots among `files` (the head tree) for the period: [`Insights::hotspots_in`] over
    /// the period's local days ending on this history's newest day.
    pub fn hotspots(&self, files: &[HeadFile], period: Period) -> HotspotReport {
        self.in_period(period, |c| self.hotspots_in(files, c))
    }

    /// Hotspots among `files` changed by the commits `c` counts.
    pub fn hotspots_in(&self, files: &[HeadFile], counted: &Counted<'_>) -> HotspotReport {
        let mut rows = Vec::new();
        let shown = self.shown(files, counted);
        let names: Vec<&str> = shown.iter().map(|f| &*f.path).collect();
        // (changes, churn, last change, authors) per shown file, filled in one visit. A file's
        // touches come together, so a person stamp per file counts its authors without a set.
        let mut acc: Vec<(usize, u64, i64, usize)> = vec![(0, 0, i64::MIN, 0); shown.len()];
        let mut seen: Vec<usize> = vec![usize::MAX; self.d.people.list.len()];
        self.visit_touches_in(&names, counted, |i, lines, person, c| {
            let a = &mut acc[i];
            a.0 += 1;
            a.1 += u64::from(lines);
            a.2 = a.2.max(c.time);
            if let Some(s) = seen.get_mut(person)
                && *s != i
            {
                *s = i;
                a.3 += 1;
            }
            true
        });
        for (f, (count, churn, last, who)) in shown.iter().zip(acc) {
            if count == 0 {
                continue;
            }
            rows.push(Hotspot {
                path: f.path.clone(),
                language: crate::insights::lang::language(&f.path),
                lines: f.lines,
                changes: count,
                lines_changed: churn,
                authors: who,
                last_change: last,
                score: count as f64 * f64::from(f.lines).sqrt(),
            });
        }
        rows.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
        let total: usize = rows.iter().map(|r| r.changes).sum();
        let tenth = rows.len().div_ceil(10).max(1);
        let mut by_changes: Vec<usize> = rows.iter().map(|r| r.changes).collect();
        by_changes.sort_unstable_by(|a, b| b.cmp(a));
        let top: usize = by_changes.iter().take(tenth).sum();
        HotspotReport {
            concentration: if total == 0 { 0.0 } else { top as f64 / total as f64 },
            top_tenth: tenth,
            single_author_files: rows.iter().filter(|r| r.authors == 1).count(),
            total_changes: total,
            files: rows,
        }
    }

    /// Lines changed per person for each head file in the window, by counted commits.
    ///
    /// Over all history a file whose name never took part in a rename reads the path x author
    /// aggregate: every row under that name is its own, so the sums are exact without a pass
    /// over the file rows. Renamed files and windows go through the touches.
    pub(super) fn owner_lines(
        &self,
        files: &[&HeadFile],
        counted: &Counted<'_>,
        whole: bool,
    ) -> Vec<BTreeMap<usize, u64>> {
        let src = self.source();
        // The aggregate holds every commit: usable only when the counted set drops none,
        // which `whole` says.
        let plain = |f: &HeadFile| -> Option<PathId> {
            let follow = self.follow();
            let id = src.path_id(&f.path)?;
            (!follow.log.contains_key(&id) && !follow.names.contains_key(&id)).then_some(id)
        };
        // The stored tip touches answer every file in one pass, cheaper than the aggregate.
        let whole = whole && !src.has_tip();
        let ids: Vec<Option<PathId>> =
            files.iter().map(|f| if whole { plain(f) } else { None }).collect();
        let mut out: Vec<BTreeMap<usize, u64>> = vec![BTreeMap::new(); files.len()];
        // Ordered by person (most commits first), so a tie for top owner and the float sums
        // come out the same on every run; a HashMap made "Stale lines" vary.
        let at: FastMap<PathId, usize> =
            ids.iter().enumerate().filter_map(|(i, id)| id.map(|id| (id, i))).collect();
        if !at.is_empty() {
            let bots = self.opts.include_bots;
            for row in src.agg_path_author("") {
                let Some(&i) = at.get(&row.path) else {
                    continue;
                };
                let p = self.person_of(row.ident);
                if !bots && self.d.people.list[p].bot {
                    continue;
                }
                let s = &row.sums;
                let lines = if self.opts.count_whitespace {
                    s.add + s.del
                } else {
                    s.code_add + s.code_del
                };
                *out[i].entry(p).or_default() += lines;
            }
        }
        let rest: Vec<usize> = (0..files.len()).filter(|&i| ids[i].is_none()).collect();
        let names: Vec<&str> = rest.iter().map(|&i| &*files[i].path).collect();
        // A file's touches come together: sum them per person in a dense scratch row and move
        // the row into the file's map when the next file starts, one insert per author.
        let mut sums: Vec<u64> = vec![0; self.d.people.list.len()];
        // Per person, the file it last joined `held` for.
        let mut mark: Vec<usize> = vec![usize::MAX; sums.len()];
        let mut held: Vec<usize> = Vec::new();
        let mut cur: Option<usize> = None;
        let mut flush = |cur: Option<usize>, sums: &mut [u64], held: &mut Vec<usize>| {
            let Some(k) = cur else { return };
            let map = &mut out[rest[k]];
            for p in held.drain(..) {
                *map.entry(p).or_default() += std::mem::take(&mut sums[p]);
            }
        };
        self.visit_touches_in(&names, counted, |k, lines, p, _| {
            if cur != Some(k) {
                flush(cur, &mut sums, &mut held);
                cur = Some(k);
            }
            if let (Some(s), Some(m)) = (sums.get_mut(p), mark.get_mut(p)) {
                // A zero-line touch still makes its author hold the file, as the map entry did.
                if *m != k {
                    *m = k;
                    held.push(p);
                }
                *s += u64::from(lines);
            }
            true
        });
        flush(cur, &mut sums, &mut held);
        // A person whose rows sum to zero lines still held the file in the touches; drop
        // nobody, the touches kept zero entries too.
        out
    }

    /// Knowledge share per file, weighted by its size now, summed per directory `depth` levels
    /// deep (`depth` 1 groups by top-level directory; root files group under `""`).
    pub fn ownership(&self, files: &[HeadFile], period: Period, depth: usize) -> OwnershipReport {
        self.in_period(period, |c| self.ownership_in(files, c, depth))
    }

    pub fn ownership_in(
        &self,
        files: &[HeadFile],
        c: &Counted<'_>,
        depth: usize,
    ) -> OwnershipReport {
        let s = c.scope();
        let whole = s.period() == Period::All && s.range().is_none() && s.matcher().is_empty();
        let shown = self.shown(files, c);
        let per_file = self.owner_lines(&shown.iter().collect::<Vec<_>>(), c, whole);
        let mut dirs: BTreeMap<String, Acc> = BTreeMap::new();
        let mut all = Acc::default();
        let (mut solo, mut stale, mut untracked) = (0u64, 0u64, 0u64);
        for (f, who) in shown.iter().zip(per_file) {
            let changed: u64 = who.values().sum();
            if changed == 0 {
                untracked += u64::from(f.lines);
                continue;
            }
            let parts: Vec<&str> = f.path.split('/').collect();
            let key = if parts.len() > 1 {
                parts[..depth.min(parts.len() - 1)].join("/")
            } else {
                String::new()
            };
            let d = dirs.entry(key).or_default();
            let (mut top_p, mut top_w) = (0, 0.0);
            for (p, c) in who {
                let w = f64::from(f.lines) * c as f64 / changed as f64;
                *d.w.entry(p).or_default() += w;
                *all.w.entry(p).or_default() += w;
                if w.total_cmp(&top_w).then_with(|| self.person_order(top_p, p)).is_gt() {
                    top_w = w;
                    top_p = p;
                }
            }
            d.lines += u64::from(f.lines);
            d.files += 1;
            all.lines += u64::from(f.lines);
            all.files += 1;
            if top_w / f64::from(f.lines) >= SOLO_SHARE {
                solo += u64::from(f.lines);
            }
            if self.d.last_seen[top_p] < self.now.saturating_sub(STALE_SECS) {
                stale += u64::from(f.lines);
            }
        }
        let finish = |path: String, a: Acc| -> DirOwnership {
            let mut owners: Vec<(usize, f64)> = a.w.into_iter().collect();
            owners.sort_by(|x, y| y.1.total_cmp(&x.1).then_with(|| self.person_order(x.0, y.0)));
            let total: f64 = owners.iter().map(|o| o.1).sum::<f64>().max(f64::MIN_POSITIVE);
            let bus = crate::insights::bus_factor(owners.iter().map(|o| o.1));
            DirOwnership {
                path,
                lines: a.lines,
                files: a.files,
                bus_factor: bus,
                owners: owners
                    .into_iter()
                    .map(|(p, w)| Owner {
                        name: self.d.people.list[p].name.clone(),
                        share: w / total,
                    })
                    .collect(),
            }
        };
        let mut dirs: Vec<DirOwnership> = dirs.into_iter().map(|(k, a)| finish(k, a)).collect();
        dirs.sort_by(|a, b| b.lines.cmp(&a.lines).then_with(|| a.path.cmp(&b.path)));
        OwnershipReport {
            dirs_with_bus_factor_one: dirs
                .iter()
                .filter(|d| d.bus_factor == 1 && !d.owners.is_empty())
                .count(),
            overall: finish(String::new(), all),
            dirs,
            solo_lines: solo,
            stale_lines: stale,
            untracked_lines: untracked,
        }
    }

    /// Pairs of head files that changed in the same commits, strongest first. Pairs need at
    /// least `min_together` shared commits. Commits touching more than 30 files are skipped.
    pub fn coupling(
        &self,
        files: &[HeadFile],
        period: Period,
        min_together: usize,
        limit: usize,
    ) -> Vec<Coupling> {
        self.in_period(period, |c| self.coupling_in(files, c, min_together, limit))
    }

    pub fn coupling_in(
        &self,
        files: &[HeadFile],
        c: &Counted<'_>,
        min_together: usize,
        limit: usize,
    ) -> Vec<Coupling> {
        let counted = c;
        let files = under_path(files, c);
        let mut head: Vec<&str> =
            files.iter().filter(|f| !self.skips_head(&f.path)).map(|f| &*f.path).collect();
        head.sort_unstable();
        head.dedup();
        // Flat (commit, file) pairs sorted by commit replace a map of vectors: one allocation
        // and a sequential pass. State is sized by the touches of head files in the window.
        let mut touches: Vec<(u32, u32)> = Vec::new();
        let mut changes = vec![0_usize; head.len()];
        self.visit_touches_in(&head, counted, |i, _, _, c| {
            touches.push((c.rank.0, u32::try_from(i).unwrap_or(u32::MAX)));
            changes[i] += 1;
            true
        });
        // By commit, files ascending in one: the visit is per file in order, so a stable
        // counting sort on the rank gives what sorting the pairs did, in linear time. Only the
        // files that can be in a kept pair (`min_together` changes) are placed, and only for
        // commits that count for coupling (2 to 30 files, counted before that filter).
        let ranks = touches.iter().map(|t| t.0 as usize + 1).max().unwrap_or(0);
        let mut size = vec![0u32; ranks];
        for t in &touches {
            size[t.0 as usize] += 1;
        }
        let counts = |r: u32| (2..=COUPLING_MAX_FILES as u32).contains(&size[r as usize]);
        let live = |t: &(u32, u32)| counts(t.0) && changes[t.1 as usize] >= min_together;
        let mut begin = vec![0u32; ranks + 1];
        for t in touches.iter().filter(|t| live(t)) {
            begin[t.0 as usize + 1] += 1;
        }
        for i in 1..begin.len() {
            begin[i] += begin[i - 1];
        }
        let mut fill = begin.clone();
        let mut by_commit = vec![0u32; begin[ranks] as usize];
        for t in touches.iter().filter(|t| live(t)) {
            let slot = &mut fill[t.0 as usize];
            by_commit[*slot as usize] = t.1;
            *slot += 1;
        }
        // Per file `a`, the files after it in each of its commits, counted in a dense row:
        // the pairs a map keyed by (a, b) counted, one file's row at a time. A file twice in
        // one commit pairs as the nested loop over the commit's list paired it.
        let mut pairs: Vec<(u32, u32, usize)> = Vec::new();
        let mut row = vec![0usize; head.len()];
        let mut hit: Vec<u32> = Vec::new();
        for list in touches.chunk_by(|x, y| x.1 == y.1) {
            let a = list[0].1;
            if changes[a as usize] < min_together {
                continue;
            }
            for same in list.chunk_by(|x, y| x.0 == y.0) {
                let r = same[0].0;
                if !counts(r) {
                    continue;
                }
                let group = &by_commit[begin[r as usize] as usize..begin[r as usize + 1] as usize];
                let first = group.partition_point(|&f| f < a);
                for j in 0..same.len() {
                    for &b in group.get(first + j + 1..).unwrap_or_default() {
                        if row[b as usize] == 0 {
                            hit.push(b);
                        }
                        row[b as usize] += 1;
                    }
                }
            }
            for b in std::mem::take(&mut hit) {
                let n = std::mem::take(&mut row[b as usize]);
                pairs.push((a, b, n));
            }
        }
        // Cheap tuples first, strings only for the pairs that are shown.
        let mut kept: Vec<(u32, u32, usize)> =
            pairs.into_iter().filter(|p| p.2 >= min_together).collect();
        let degree = |&(a, b, together): &(u32, u32, usize)| {
            together as f64 / changes[a as usize].min(changes[b as usize]) as f64
        };
        let order = |x: &(u32, u32, usize), y: &(u32, u32, usize)| {
            y.2.cmp(&x.2)
                .then_with(|| degree(y).total_cmp(&degree(x)))
                .then_with(|| head[x.0 as usize].cmp(head[y.0 as usize]))
                .then_with(|| head[x.1 as usize].cmp(head[y.1 as usize]))
        };
        if kept.len() > limit {
            if limit > 0 {
                kept.select_nth_unstable_by(limit - 1, order);
            }
            kept.truncate(limit);
        }
        kept.sort_by(order);
        kept.into_iter()
            .map(|t| {
                let (a, b, together) = t;
                let (ac, bc) = (changes[a as usize], changes[b as usize]);
                Coupling {
                    a: head[a as usize].to_owned(),
                    b: head[b as usize].to_owned(),
                    together,
                    a_changes: ac,
                    b_changes: bc,
                    degree: degree(&t),
                }
            })
            .collect()
    }

    /// Lines now grouped by the year of the file's last counted change. See [`AgeReport`].
    pub fn code_age(&self, files: &[HeadFile]) -> AgeReport {
        self.in_period(Period::All, |c| self.code_age_in(files, c))
    }

    /// [`Insights::code_age`] over the commits `c` counts: its days, selection and options. The
    /// patterns page passes an all-time `c`, so a file's age is its last counted change, and a
    /// merge or a filtered-out author never makes it younger.
    pub fn code_age_in(&self, files: &[HeadFile], counted: &Counted<'_>) -> AgeReport {
        let mut years: BTreeMap<i64, (u64, usize)> = BTreeMap::new();
        let mut total = 0;
        let shown = self.shown(files, counted);
        let names: Vec<&str> = shown.iter().map(|f| &*f.path).collect();
        // Touches come newest commit first: the first one is the file's last change.
        let mut newest: Vec<Option<crate::feed::CommitRow>> = vec![None; shown.len()];
        self.visit_touches_in(&names, counted, |i, _, _, c| {
            newest[i] = Some(*c);
            false
        });
        for (f, c) in shown.iter().zip(newest) {
            let Some(c) = c else {
                continue;
            };
            let day = local_day_of(c.time, c.offset);
            let e = years.entry(civil_from_days(day).0).or_default();
            e.0 += u64::from(f.lines);
            e.1 += 1;
            total += u64::from(f.lines);
        }
        AgeReport {
            buckets: years
                .into_iter()
                .map(|(year, (lines, files))| AgeBucket { year, lines, files })
                .collect(),
            total_lines: total,
        }
    }
}

impl Insights<'_> {
    /// The head files under the selection that have lines and are not skipped.
    fn shown(&self, files: &[HeadFile], c: &Counted<'_>) -> Vec<HeadFile> {
        under_path(files, c)
            .into_iter()
            .filter(|f| f.lines != 0 && !self.skips_head(&f.path))
            .collect()
    }

    /// Ties in a ranked list of people: display name, then the first e-mail. Never the person
    /// index, which follows commit counts of whatever history the tables were built over.
    fn person_order(&self, a: usize, b: usize) -> std::cmp::Ordering {
        let (x, y) = (&self.d.people.list[a], &self.d.people.list[b]);
        x.name.cmp(&y.name).then_with(|| x.emails.cmp(&y.emails))
    }
}

/// The head files a path selection keeps: a file outside it has no counted change.
fn under_path(files: &[HeadFile], c: &Counted<'_>) -> Vec<HeadFile> {
    let m = c.scope().matcher();
    files.iter().filter(|f| m.row(&f.path)).cloned().collect()
}

#[derive(Default)]
struct Acc {
    lines: u64,
    files: usize,
    w: BTreeMap<usize, f64>,
}
