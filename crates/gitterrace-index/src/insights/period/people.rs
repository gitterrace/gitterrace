//! Contributors over a [`Counted`] set: rows, lines, active days and the per-bucket series of
//! every person the scope counts. Bots follow the options like every other count.

use std::collections::BTreeSet;

use crate::insights::fasthash::FastMap;
use crate::insights::{
    Contributor, ContributorReport, Counted, Item, Metric, Period, Reducer, Scope, ScopeParams,
    TimeAxis,
};

struct Acc {
    commits: usize,
    add: u64,
    del: u64,
    first: i64,
    last: i64,
    days: BTreeSet<i64>,
    series: Vec<u32>,
    /// Deleted lines per bucket, read for the net metric.
    dels: Vec<u32>,
}

/// Contributors over the stream, see [`Counted::contributors`].
pub struct ContributorTally {
    metric: Metric,
    len: usize,
    acc: FastMap<usize, Acc>,
}

impl Reducer for ContributorTally {
    fn add(&mut self, c: &Counted<'_>, item: &Item) {
        let len = self.len;
        let t = item.commit.time;
        let a = self.acc.entry(item.person).or_insert_with(|| Acc {
            commits: 0,
            add: 0,
            del: 0,
            first: t,
            last: t,
            days: BTreeSet::new(),
            series: vec![0; len],
            dels: vec![0; len],
        });
        let l = item.lines;
        a.commits += 1;
        a.add += u64::from(l.add);
        a.del += u64::from(l.del);
        a.first = a.first.min(t);
        a.last = a.last.max(t);
        a.days.insert(item.day);
        if let Some(slot) = c.axis().slot_day(item.day).filter(|s| *s < len) {
            match self.metric {
                Metric::Commits => a.series[slot] += 1,
                Metric::Added | Metric::Net => {
                    a.series[slot] = a.series[slot].saturating_add(l.add);
                    a.dels[slot] = a.dels[slot].saturating_add(l.del);
                }
                Metric::Deleted => a.series[slot] = a.series[slot].saturating_add(l.del),
            }
        }
    }
}

impl Counted<'_> {
    /// Every person the set counts, ordered by `metric`, then name. The series carries that
    /// metric per bucket (net floored at zero). `first_time`: the person's first commit
    /// counted under the same options, over the whole history and whatever the selection,
    /// lies on or after the first counted day.
    pub fn contributors(&self, metric: Metric) -> ContributorReport {
        let mut people = ContributorTally::new(metric, self.axis());
        self.reduce(&mut [&mut people]);
        people.finish(self)
    }
}

impl ContributorTally {
    pub fn new(metric: Metric, axis: &TimeAxis) -> Self {
        Self {
            metric,
            len: axis.len(),
            acc: FastMap::default(),
        }
    }

    /// The report of what the stream fed in, over the set `set` it came from.
    pub fn finish(self, set: &Counted<'_>) -> ContributorReport {
        report(self, set)
    }
}

fn report(people: ContributorTally, set: &Counted<'_>) -> ContributorReport {
    let (ins, axis, metric) = (set.insights(), set.axis(), people.metric);
    let start = *set.days().start();
    let fixed = set.scope().period().days().is_some() || set.scope().range().is_some();
    let mut first_day: FastMap<usize, i64> = FastMap::default();
    if fixed {
        // New means new to the project: every day, the same options (merges, bots), no
        // selection. A filter chip never turns a long-time contributor into a newcomer, and a
        // merge or an uncounted bot never makes anyone a returning contributor.
        let ever = Scope::from_params(
            &ScopeParams::default(),
            set.scope().options().clone(),
            Period::All,
        )
        .unwrap_or_else(|_| unreachable!("an empty query is always a valid scope"));
        for row in ins.counted(&ever, *set.context()).stream() {
            let who = ins.person_of(row.ident);
            if !people.acc.contains_key(&who) {
                continue;
            }
            let day = crate::feed::local_day_of(row.time, row.offset);
            let e = first_day.entry(who).or_insert(i64::MAX);
            *e = (*e).min(day);
        }
    }
    let mut out: Vec<Contributor> = people
        .acc
        .into_iter()
        .map(|(who, acc)| {
            let person = &ins.d.people.list[who];
            Contributor {
                name: person.name.clone(),
                emails: person.emails.clone(),
                bot: person.bot,
                commits: acc.commits,
                additions: acc.add,
                deletions: acc.del,
                first_commit: acc.first,
                last_commit: acc.last,
                active_days: acc.days.len(),
                series: if metric == Metric::Net {
                    acc.series
                        .iter()
                        .zip(&acc.dels)
                        .map(|(x, y)| x.saturating_sub(*y))
                        .collect()
                } else {
                    acc.series
                },
                first_time: fixed && first_day.get(&who).is_some_and(|d| *d >= start),
            }
        })
        .collect();
    let key = |row: &Contributor| metric.of(row.commits, row.additions, row.deletions);
    out.sort_by(|x, y| {
        key(y)
            .cmp(&key(x))
            .then_with(|| x.name.cmp(&y.name))
            .then_with(|| x.emails.cmp(&y.emails))
    });
    ContributorReport {
        labels: (0..axis.len()).map(|i| axis.label(i)).collect(),
        unit: axis.unit(),
        contributors: out,
    }
}
