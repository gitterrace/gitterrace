//! What a statistic is asked about, in the form every surface reads from a query string: the
//! commit selection (type, scope, author, path) and the counting options (bots, generated files,
//! ignore rules, whitespace, extra excludes). The web pages, the HTTP API and the MCP server all
//! parse these two structs, so the same URL parameters select the same commits everywhere.

use std::borrow::Cow;

use crate::CommitRecord;
use crate::insights::Options;

/// Which commits count. Empty strings count as unset.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct Selection {
    /// Conventional-commit type; [`Selection::OTHER`] selects commits without one.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// Conventional-commit scope; [`Selection::NO_SCOPE`] selects commits without one.
    pub scope: Option<String>,
    /// E-mail (any case) or exact author name.
    pub author: Option<String>,
    /// Path prefix: a kept commit lists only its files under the prefix.
    pub path: Option<String>,
}

fn set(v: Option<&String>) -> Option<&str> {
    v.map(String::as_str).filter(|s| !s.is_empty())
}

impl Selection {
    /// Type value of commits without a type.
    pub const OTHER: &str = "other";
    /// Scope value of commits without a scope.
    pub const NO_SCOPE: &str = "none";

    pub fn kind(&self) -> Option<&str> {
        set(self.kind.as_ref())
    }

    pub fn scope(&self) -> Option<&str> {
        set(self.scope.as_ref())
    }

    pub fn author(&self) -> Option<&str> {
        set(self.author.as_ref())
    }

    /// Path prefix without surrounding slashes, `None` for the whole tree.
    pub fn path(&self) -> Option<&str> {
        set(self.path.as_ref())
            .map(|p| p.trim_matches('/'))
            .filter(|p| !p.is_empty())
    }

    pub fn is_empty(&self) -> bool {
        self.kind().is_none()
            && self.scope().is_none()
            && self.author().is_none()
            && self.path().is_none()
    }

    fn matches(&self, r: &CommitRecord) -> bool {
        self.kind()
            .is_none_or(|k| r.kind.as_deref().unwrap_or(Self::OTHER) == k)
            && self
                .scope()
                .is_none_or(|s| r.scope.as_deref().unwrap_or(Self::NO_SCOPE) == s)
            && self
                .author()
                .is_none_or(|a| r.email.eq_ignore_ascii_case(a) || r.author == a)
    }

    /// The records the selection keeps. With a path each kept commit lists only the files under
    /// it, so line counts and churn are the path's own. Without a selection the input is borrowed.
    pub fn apply<'a>(&self, records: &'a [CommitRecord]) -> Cow<'a, [CommitRecord]> {
        if self.is_empty() {
            return Cow::Borrowed(records);
        }
        let path = self.path();
        let out = records
            .iter()
            .filter(|r| self.matches(r))
            .filter_map(|r| match path {
                None => Some(r.clone()),
                Some(p) => {
                    let files: Vec<_> = r
                        .files
                        .iter()
                        .filter(|f| under(&f.path, p))
                        .cloned()
                        .collect();
                    (!files.is_empty()).then(|| CommitRecord { files, ..r.clone() })
                }
            })
            .collect();
        Cow::Owned(out)
    }
}

/// True when `path` is `prefix` itself or inside it.
pub fn under(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The counting options as URL parameters. Absent ones keep the defaults they are applied to.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct OptionParams {
    /// `1` counts bot accounts, `0` leaves them out.
    pub bots: Option<String>,
    /// `1` counts lock files and minified output, `0` leaves them out.
    pub generated: Option<String>,
    /// `0` counts paths that `.gitignore` rules out.
    pub ignore: Option<String>,
    /// `1` counts whitespace-only changes.
    pub ws: Option<String>,
    /// Comma separated globs left out on top of the configured ones.
    pub exclude: Option<String>,
}

impl OptionParams {
    /// The URL's own globs, without the configured ones.
    pub fn own_exclude(&self) -> Vec<String> {
        self.exclude
            .as_deref()
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|g| !g.is_empty())
            .map(str::to_owned)
            .collect()
    }

    /// `defaults` (the configuration) overridden by what the URL says.
    pub fn apply(&self, defaults: Options) -> Options {
        let flag = |v: Option<&String>, default: bool| match v.map(String::as_str) {
            Some("1") => true,
            Some("0") => false,
            _ => default,
        };
        let mut exclude = defaults.exclude;
        exclude.extend(self.own_exclude());
        Options {
            exclude,
            include_generated: flag(self.generated.as_ref(), defaults.include_generated),
            respect_ignore: flag(self.ignore.as_ref(), defaults.respect_ignore),
            count_whitespace: flag(self.ws.as_ref(), defaults.count_whitespace),
            include_bots: flag(self.bots.as_ref(), defaults.include_bots),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OptionParams, Selection, under};
    use crate::insights::fixtures::{file, rec};
    use crate::{CommitRecord, Options};

    fn sample() -> Vec<CommitRecord> {
        let mut v = vec![
            rec("a", "ann", 1, vec![file("crates/web/a.rs", 3, 0, 3)]),
            rec("b", "bob", 50, vec![file("crates/git/b.rs", 3, 0, 3)]),
            rec("c", "ann", 99, vec![file("crates/web/c.rs", 3, 0, 3)]),
            rec("d", "ann", 100, vec![file("crates/webby/d.rs", 3, 0, 3)]),
        ];
        for (r, (k, s)) in v.iter_mut().zip([
            (Some("fix"), Some("web")),
            (Some("feat"), None),
            (None, None),
            (Some("fix"), Some("git")),
        ]) {
            r.kind = k.map(Into::into);
            r.scope = s.map(Into::into);
            r.files.push(file("other/x.rs", 1, 0, 1));
        }
        v
    }

    fn ids(s: &Selection) -> Vec<String> {
        s.apply(&sample()).iter().map(|r| r.id.clone()).collect()
    }

    fn sel(kind: Option<&str>, scope: Option<&str>, author: Option<&str>) -> Selection {
        Selection {
            kind: kind.map(Into::into),
            scope: scope.map(Into::into),
            author: author.map(Into::into),
            path: None,
        }
    }

    #[test]
    fn selections_stack() {
        assert_eq!(ids(&Selection::default()), ["a", "b", "c", "d"]);
        assert_eq!(ids(&sel(Some("fix"), None, None)), ["a", "d"]);
        assert_eq!(ids(&sel(Some("fix"), Some("web"), None)), ["a"]);
        assert_eq!(ids(&sel(Some("other"), None, None)), ["c"]);
        assert_eq!(ids(&sel(None, Some("none"), None)), ["b", "c"]);
        assert_eq!(ids(&sel(Some("fix"), None, Some("ANN@x.io"))), ["a", "d"]);
        assert_eq!(ids(&sel(Some(""), None, None)), ["a", "b", "c", "d"]);
    }

    #[test]
    fn path_keeps_only_files_under_the_prefix() {
        let s = Selection {
            path: Some("/crates/web/".into()),
            ..Selection::default()
        };
        let out = s.apply(&sample()).into_owned();
        assert_eq!(
            out.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["a", "c"]
        );
        assert!(out.iter().all(|r| r.files.len() == 1));
        assert!(under("crates/web/a.rs", "crates/web") && !under("crates/webby/d.rs", "crates/web"));
    }

    #[test]
    fn url_options_override_the_defaults() {
        let defaults = Options {
            exclude: vec!["docs/*".into()],
            include_bots: true,
            ..Options::default()
        };
        let p = OptionParams {
            bots: Some("0".into()),
            ignore: Some("0".into()),
            exclude: Some(" *.md, ,x/*".into()),
            ..OptionParams::default()
        };
        let o = p.apply(defaults.clone());
        assert!(!o.include_bots && !o.respect_ignore && !o.include_generated);
        assert_eq!(o.exclude, ["docs/*", "*.md", "x/*"]);
        assert_eq!(OptionParams::default().apply(defaults.clone()), defaults);
    }
}
