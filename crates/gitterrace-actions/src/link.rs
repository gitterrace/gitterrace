//! `gitterrace://<repo-id>/<sha>[/<path>][#Lx-Ly]`: a permalink that names what, not where.
//! It carries the same tuple as `/go/<id>/<sha>/<path>`, so the two convert both ways. A link
//! arrives from outside (a chat, a terminal, the OS handler) and drives a local lookup, so the
//! parser is strict: anything it does not expect is refused, never repaired.

use gitterrace_git::percent_encode_path;

use crate::payload::{Lines, permalink};

const SCHEME: &str = "gitterrace://";
/// Longest link accepted. A real one is an id, a sha and a path well under this; anything
/// longer is not a link a person handed over.
const MAX_LEN: usize = 4096;
const ID_LEN: usize = 12;
const MIN_SHA: usize = 7;
const MAX_SHA: usize = 40;

/// Which form Copy permalink, Send to, payloads and the MCP hand out (`links.style`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LinkStyle {
    #[default]
    Gitterrace,
    Go,
}

impl LinkStyle {
    /// The style a validated `links.style` names; `go` or the default.
    pub fn from_config(s: &str) -> Self {
        if s == "go" {
            Self::Go
        } else {
            Self::Gitterrace
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gitterrace => "gitterrace",
            Self::Go => "go",
        }
    }
}

/// A parsed link. `sha` may be abbreviated (7 to 40 hex); `/go` needs the full one, so a
/// caller resolves it in a clone before converting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub id: String,
    pub sha: String,
    /// Percent-decoded, relative, without `.`, `..`, empty segments or control characters.
    pub path: Option<String>,
    pub lines: Option<Lines>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    #[error("link too long")]
    TooLong,
    #[error("link holds a character a link never has")]
    BadChar,
    #[error("not a gitterrace:// or /go link")]
    Scheme,
    #[error("repo id must be 12 lowercase hex digits")]
    Id,
    #[error("commit must be 7 to 40 lowercase hex digits")]
    Sha,
    #[error("path is not a plain path inside the repository")]
    Path,
    #[error("line range must be #L<n> or #L<a>-L<b> with 1 <= a <= b")]
    Lines,
    #[error("link has parts gitterrace does not know (a query, a user, a port)")]
    Extra,
}

impl Link {
    /// Parses `gitterrace://...`, `http(s)://<host>/go/...` or `/go/...`.
    pub fn parse(input: &str) -> Result<Self, LinkError> {
        if input.len() > MAX_LEN {
            return Err(LinkError::TooLong);
        }
        if !input.is_ascii()
            || input
                .bytes()
                .any(|b| b.is_ascii_control() || b == b' ' || b == b'\\')
        {
            return Err(LinkError::BadChar);
        }
        let rest = if let Some(r) = input.strip_prefix(SCHEME) {
            r
        } else if let Some(r) = input.strip_prefix("/go/") {
            r
        } else if let Some(r) = input
            .strip_prefix("http://")
            .or_else(|| input.strip_prefix("https://"))
        {
            let (host, path) = r.split_once('/').ok_or(LinkError::Scheme)?;
            // The host is ignored (the link is resolved locally), but a user part is a classic
            // way to dress one host up as another, so it is refused rather than skipped.
            if host.is_empty() || host.contains('@') {
                return Err(LinkError::Extra);
            }
            path.strip_prefix("go/").ok_or(LinkError::Scheme)?
        } else {
            return Err(LinkError::Scheme);
        };
        let (main, frag) = match rest.split_once('#') {
            Some((m, f)) => (m, Some(f)),
            None => (rest, None),
        };
        if main.contains(['?', '@', ':']) {
            return Err(LinkError::Extra);
        }
        let mut parts = main.splitn(3, '/');
        let id = parts.next().unwrap_or_default();
        if id.len() != ID_LEN || !lower_hex(id) {
            return Err(LinkError::Id);
        }
        let sha = parts.next().unwrap_or_default();
        if !(MIN_SHA..=MAX_SHA).contains(&sha.len()) || !lower_hex(sha) {
            return Err(LinkError::Sha);
        }
        let path = parts.next().map(clean_path).transpose()?;
        let lines = frag.map(parse_lines).transpose()?;
        if lines.is_some() && path.is_none() {
            return Err(LinkError::Lines);
        }
        Ok(Self {
            id: id.to_owned(),
            sha: sha.to_owned(),
            path,
            lines,
        })
    }

    /// The `gitterrace://` form.
    pub fn to_gitterrace(&self) -> String {
        format!(
            "{SCHEME}{}{}",
            self.id,
            tail(&self.sha, self.path.as_deref(), self.lines)
        )
    }

    /// The `/go` form rooted at `origin` (empty for the path form).
    pub fn go(&self, origin: &str) -> String {
        permalink(
            origin,
            Some(&self.id),
            "",
            &self.sha,
            self.path.as_deref(),
            self.lines,
        )
    }
}

/// The link Copy permalink and friends hand out in `style`. A repository without an id (no
/// commits) has no `gitterrace://` form, so it falls back to `/go`.
pub fn styled(
    style: LinkStyle,
    origin: &str,
    id: Option<&str>,
    name: &str,
    sha: &str,
    path: Option<&str>,
    lines: Option<Lines>,
) -> String {
    match (style, id) {
        (LinkStyle::Gitterrace, Some(id)) => format!("{SCHEME}{id}{}", tail(sha, path, lines)),
        _ => permalink(origin, id, name, sha, path, lines),
    }
}

/// `/<sha>[/<path>][#L..]`, shared by both forms.
pub(crate) fn tail(sha: &str, path: Option<&str>, lines: Option<Lines>) -> String {
    let mut s = format!("/{sha}");
    if let Some(p) = path {
        s.push('/');
        s.push_str(&percent_encode_path(p));
    }
    match lines {
        Some(l) if l.start == l.end => {
            s += "#L";
            s += &l.start.to_string();
        }
        Some(l) => {
            s += "#L";
            s += &l.start.to_string();
            s += "-L";
            s += &l.end.to_string();
        }
        None => {}
    }
    s
}

fn lower_hex(s: &str) -> bool {
    s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Decodes strictly (a `%` must start two hex digits, the result must be UTF-8), then refuses
/// anything but a relative path of real segments. Decoding comes first so `%2e%2e` is `..`.
fn clean_path(raw: &str) -> Result<String, LinkError> {
    let b = raw.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = |j: usize| b.get(j).and_then(|c| char::from(*c).to_digit(16));
            let (Some(h), Some(l)) = (hex(i + 1), hex(i + 2)) else {
                return Err(LinkError::Path);
            };
            out.push(u8::try_from(h * 16 + l).map_err(|_| LinkError::Path)?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    let p = String::from_utf8(out).map_err(|_| LinkError::Path)?;
    let bad = p.is_empty()
        || p.chars().any(|c| c.is_control() || c == '\\')
        || p.split('/').any(|s| s.is_empty() || s == "." || s == "..");
    if bad { Err(LinkError::Path) } else { Ok(p) }
}

/// `L<n>` or `L<a>-L<b>`. Digits only: `u32::from_str` would take a `+`.
fn parse_lines(f: &str) -> Result<Lines, LinkError> {
    let num = |s: &str| {
        let s = s.strip_prefix('L')?;
        if s.is_empty() || s.len() > 9 || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        s.parse::<u32>().ok().filter(|n| *n > 0)
    };
    let (a, b) = match f.split_once('-') {
        Some((a, b)) => (num(a), num(b)),
        None => (num(f), num(f)),
    };
    match (a, b) {
        (Some(start), Some(end)) if start <= end => Ok(Lines { start, end }),
        _ => Err(LinkError::Lines),
    }
}

#[cfg(test)]
mod tests {
    use crate::link::{Link, LinkError, LinkStyle, styled};
    use crate::payload::Lines;

    const ID: &str = "0123456789ab";
    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn accepts_the_three_forms_and_round_trips() {
        let want = Link {
            id: ID.into(),
            sha: SHA.into(),
            path: Some("src/a b.rs".into()),
            lines: Some(Lines { start: 3, end: 7 }),
        };
        for s in [
            format!("gitterrace://{ID}/{SHA}/src/a%20b.rs#L3-L7"),
            format!(
                "{}/go/{ID}/{SHA}/src/a%20b.rs#L3-L7",
                gitterrace_testkit::base_url()
            ),
            format!("https://example.org/go/{ID}/{SHA}/src/a%20b.rs#L3-L7"),
            format!("/go/{ID}/{SHA}/src/a%20b.rs#L3-L7"),
        ] {
            assert_eq!(Link::parse(&s), Ok(want.clone()), "{s}");
        }
        assert_eq!(
            want.to_gitterrace(),
            format!("gitterrace://{ID}/{SHA}/src/a%20b.rs#L3-L7")
        );
        assert_eq!(
            Link::parse(&want.to_gitterrace()).map(|l| l.go("")),
            Ok(format!("/go/{ID}/{SHA}/src/a%20b.rs#L3-L7"))
        );
        let one = Link::parse(&format!("gitterrace://{ID}/abc1234/x#L5")).expect("short");
        assert_eq!(one.lines, Some(Lines { start: 5, end: 5 }));
        assert_eq!(one.sha, "abc1234");
        let bare = Link::parse(&format!("gitterrace://{ID}/{SHA}")).expect("bare");
        assert_eq!((bare.path, bare.lines), (None, None));
        let uni = Link::parse(&format!("gitterrace://{ID}/{SHA}/%C3%A9t%C3%A9.md")).expect("u");
        assert_eq!(uni.path.as_deref(), Some("\u{e9}t\u{e9}.md"));
    }

    #[test]
    fn refuses_hostile_input() {
        use LinkError::{BadChar, Extra, Id, Lines as L, Path, Scheme, Sha, TooLong};
        let g = |rest: &str| format!("gitterrace://{ID}/{SHA}{rest}");
        let cases: Vec<(String, LinkError)> = vec![
            (g("/../etc/passwd"), Path),
            (g("/a/../../b"), Path),
            (g("/%2e%2e/x"), Path),
            (g("/a/%2E%2E"), Path),
            (g("/%2e"), Path),
            (g("/a%2f..%2fb"), Path),
            (g("//etc/passwd"), Path),
            (g("/%2Fetc"), Path),
            (g("/a%00b"), Path),
            (g("/a%0ab"), Path),
            (g("/a%5cb"), Path),
            (g("/a%zz"), Path),
            (g("/a%2"), Path),
            (g("/a%ff"), Path),
            (g("/"), Path),
            (g("/a//b"), Path),
            (g("/a\\b"), BadChar),
            (g("/\u{e9}"), BadChar),
            (g("/a b"), BadChar),
            (g("/a\nb"), BadChar),
            (g("/a?x=1"), Extra),
            (g("?x=1"), Extra),
            (g("/a#L0"), L),
            (g("/a#L5-L3"), L),
            (g("/a#L+5"), L),
            (g("/a#L"), L),
            (g("/a#5"), L),
            (g("/a#L1-L2-L3"), L),
            (g("/a#L99999999999"), L),
            (g("#L1"), L),
            (g(&format!("/{}", "a".repeat(5000))), TooLong),
            ("gitterrace://".into(), Id),
            (format!("gitterrace:///{SHA}"), Id),
            (format!("gitterrace://a@b/{SHA}"), Extra),
            (format!("gitterrace://{ID}:80/{SHA}"), Extra),
            (format!("gitterrace://user@{ID}/{SHA}"), Extra),
            (format!("gitterrace://0123456789AB/{SHA}"), Id),
            (format!("gitterrace://0123456789a/{SHA}"), Id),
            (format!("gitterrace://0123456789abc/{SHA}"), Id),
            (format!("gitterrace://{ID}"), Sha),
            (format!("gitterrace://{ID}/abc12"), Sha),
            (format!("gitterrace://{ID}/{SHA}0"), Sha),
            (format!("gitterrace://{ID}/ABC1234"), Sha),
            (format!("gitterrace://{ID}/HEAD"), Sha),
            (format!("gitterrace://{ID}/-abcdef0"), Sha),
            (format!("GITTERRACE://{ID}/{SHA}"), Scheme),
            (format!("gitterrace:{ID}/{SHA}"), Scheme),
            (format!("file:///go/{ID}/{SHA}"), Scheme),
            (format!("javascript://{ID}/{SHA}"), Scheme),
            (format!("http://h/r/x/go/{ID}/{SHA}"), Scheme),
            (format!("http://u@h/go/{ID}/{SHA}"), Extra),
            (format!("http:///go/{ID}/{SHA}"), Extra),
            (format!("go/{ID}/{SHA}"), Scheme),
            (String::new(), Scheme),
        ];
        for (s, want) in cases {
            assert_eq!(Link::parse(&s), Err(want), "{s:?}");
        }
    }

    #[test]
    fn styled_switches_form() {
        let l = Some(Lines { start: 2, end: 2 });
        let gt = styled(
            LinkStyle::Gitterrace,
            "http://h",
            Some(ID),
            "n",
            SHA,
            Some("a"),
            l,
        );
        assert_eq!(gt, format!("gitterrace://{ID}/{SHA}/a#L2"));
        let go = styled(LinkStyle::Go, "http://h", Some(ID), "n", SHA, Some("a"), l);
        assert_eq!(go, format!("http://h/go/{ID}/{SHA}/a#L2"));
        let none = styled(LinkStyle::Gitterrace, "", None, "n", SHA, None, None);
        assert_eq!(none, format!("/go/n/{SHA}"), "no id, no gitterrace:// form");
    }
}
