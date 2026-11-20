use std::collections::{HashMap, HashSet};

use crate::CommitRecord;

const BOT_PREFIXES: [&str; 10] = [
    "dependabot",
    "renovate",
    "greenkeeper",
    "snyk",
    "pre-commit-ci",
    "github-actions",
    "mergify",
    "imgbot",
    "allcontributors",
    "semantic-release",
];

const SHARED_MAILBOXES: [&str; 4] = ["noreply@", "no-reply@", "none@", "root@"];

/// One person: every (name, e-mail) pair that shares an address or a normalised name.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Person {
    pub name: String,
    pub bot: bool,
    pub commits: usize,
    pub emails: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct People {
    pub list: Vec<Person>,
    index: HashMap<(String, String), usize>,
}

impl People {
    /// Merges identities. The mailmap is already applied to the records, so this only joins
    /// what it left apart: the same address under several names, the same name under several
    /// addresses.
    pub fn build(records: &[CommitRecord]) -> Self {
        Self::from_counts(records.iter().map(|r| ((r.author.as_str(), r.email.as_str()), 1)))
    }

    /// Same as [`Self::build`] from commit counts per (name, e-mail) pair, so a source that
    /// streams commits builds people without holding records. Repeated pairs add up.
    pub fn from_counts<'s>(counts: impl IntoIterator<Item = ((&'s str, &'s str), usize)>) -> Self {
        let mut seen: HashMap<(&str, &str), usize> = HashMap::new();
        for (pair, n) in counts {
            *seen.entry(pair).or_default() += n;
        }
        let mut dsu = Dsu::default();
        for (name, email) in seen.keys() {
            let a = dsu.node(format!("n:{}", norm_name(name)));
            let mail = email.trim().to_lowercase();
            if mail.contains('@') && !SHARED_MAILBOXES.iter().any(|p| mail.starts_with(p)) {
                let b = dsu.node(format!("e:{mail}"));
                dsu.union(a, b);
            }
        }
        let mut groups: HashMap<usize, Group> = HashMap::new();
        for ((name, email), n) in &seen {
            let root = {
                let a = dsu.node(format!("n:{}", norm_name(name)));
                dsu.find(a)
            };
            let g = groups.entry(root).or_insert_with(|| Group {
                names: HashMap::new(),
                emails: HashSet::new(),
                commits: 0,
                bot: false,
            });
            *g.names.entry((*name).to_owned()).or_default() += n;
            g.emails.insert((*email).to_owned());
            g.commits += n;
            g.bot |= is_bot(name, email);
        }
        let mut ranked: Vec<(usize, Group)> = groups.into_iter().collect();
        let display = |g: &Group| {
            g.names
                .iter()
                .max_by(|a, b| {
                    (a.1, a.0.contains(' '), a.0.len(), b.0).cmp(&(
                        b.1,
                        b.0.contains(' '),
                        b.0.len(),
                        a.0,
                    ))
                })
                .map(|(n, _)| n.clone())
                .unwrap_or_default()
        };
        ranked.sort_by(|a, b| {
            b.1.commits.cmp(&a.1.commits).then_with(|| display(&a.1).cmp(&display(&b.1))).then_with(
                || {
                    let mut x: Vec<&String> = a.1.emails.iter().collect();
                    let mut y: Vec<&String> = b.1.emails.iter().collect();
                    x.sort();
                    y.sort();
                    x.cmp(&y)
                },
            )
        });
        let mut list = Vec::with_capacity(ranked.len());
        let mut slot: HashMap<usize, usize> = HashMap::new();
        for (root, g) in ranked {
            slot.insert(root, list.len());
            let mut emails: Vec<String> = g.emails.iter().cloned().collect();
            emails.sort();
            list.push(Person { name: display(&g), bot: g.bot, commits: g.commits, emails });
        }
        let mut index = HashMap::new();
        for (name, email) in seen.keys() {
            let a = dsu.node(format!("n:{}", norm_name(name)));
            index.insert(((*name).to_owned(), (*email).to_owned()), slot[&dsu.find(a)]);
        }
        Self { list, index }
    }

    /// Person for an author pair; pairs never seen map to person 0.
    pub fn index_of(&self, name: &str, email: &str) -> usize {
        self.index.get(&(name.to_owned(), email.to_owned())).copied().unwrap_or(0)
    }
}

/// Bot accounts by the usual naming: a `[bot]` marker or a well-known service name.
pub fn is_bot(name: &str, email: &str) -> bool {
    let local = email.split('@').next().unwrap_or_default().to_lowercase();
    let name = name.to_lowercase();
    name.contains("[bot]")
        || local.contains("[bot]")
        || has_bot_prefix(&name)
        || has_bot_prefix(&local)
}

fn has_bot_prefix(s: &str) -> bool {
    BOT_PREFIXES.iter().any(|p| {
        s.strip_prefix(p).is_some_and(|rest| {
            rest.chars().next().is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
        })
    })
}

fn norm_name(name: &str) -> String {
    let key: String = name.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect();
    if key.is_empty() { name.trim().to_lowercase() } else { key }
}

struct Group {
    names: HashMap<String, usize>,
    emails: HashSet<String>,
    commits: usize,
    bot: bool,
}

#[derive(Default)]
struct Dsu {
    ids: HashMap<String, usize>,
    parent: Vec<usize>,
}

impl Dsu {
    fn node(&mut self, key: String) -> usize {
        if let Some(i) = self.ids.get(&key) {
            return *i;
        }
        let i = self.parent.len();
        self.parent.push(i);
        self.ids.insert(key, i);
        i
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        self.parent[rb] = ra;
    }
}

#[cfg(test)]
mod tests {
    use super::{People, is_bot};
    use crate::CommitRecord;

    fn r(name: &str, email: &str) -> CommitRecord {
        CommitRecord { author: name.into(), email: email.into(), ..CommitRecord::default() }
    }

    #[test]
    fn merges_by_email_and_by_name() {
        let recs = [
            r("Ada Lovelace", "ada@a.org"),
            r("Ada Lovelace", "ada@a.org"),
            r("ada", "ada@a.org"),
            r("ada lovelace", "ada@home.net"),
            r("Bob", "noreply@x.io"),
            r("Carol", "noreply@x.io"),
        ];
        let p = People::build(&recs);
        assert_eq!(p.list.len(), 3);
        assert_eq!(p.list[0].name, "Ada Lovelace");
        assert_eq!(p.list[0].commits, 4);
        assert_eq!(p.index_of("ada", "ada@a.org"), p.index_of("ada lovelace", "ada@home.net"));
        assert_ne!(p.index_of("Bob", "noreply@x.io"), p.index_of("Carol", "noreply@x.io"));
    }

    #[test]
    fn detects_bots() {
        assert!(is_bot("dependabot[bot]", "49699333+dependabot[bot]@users.noreply.github.com"));
        assert!(is_bot("Renovate Bot", "bot@renovateapp.com"));
        assert!(is_bot("x", "github-actions@github.com"));
        assert!(!is_bot("Robot Ross", "ross@x.io"));
        assert!(!is_bot("snykes", "s@x.io"));
    }
}
