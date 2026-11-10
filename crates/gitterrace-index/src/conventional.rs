/// Splits `type(scope)!: description` into type and scope. Subjects that do not follow the
/// convention yield `(None, None)`.
pub fn parse_subject(subject: &str) -> (Option<String>, Option<String>) {
    let Some((head, _)) = subject.split_once(": ") else {
        return (None, None);
    };
    let head = head.strip_suffix('!').unwrap_or(head);
    let (kind, scope) = match head.split_once('(') {
        Some((kind, rest)) => match rest.strip_suffix(')') {
            Some(scope) if !scope.is_empty() && !scope.contains(['(', ')']) => {
                (kind, Some(scope.to_owned()))
            }
            _ => return (None, None),
        },
        None => (head, None),
    };
    if kind.is_empty() || !kind.bytes().all(|b| b.is_ascii_alphabetic()) {
        return (None, None);
    }
    (Some(kind.to_ascii_lowercase()), scope)
}

#[cfg(test)]
mod tests {
    use crate::conventional::parse_subject;

    fn p(s: &str) -> (Option<String>, Option<String>) {
        parse_subject(s)
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "the signature must match the unix variant"
    )]
    fn some(s: &str) -> Option<String> {
        Some(s.to_owned())
    }

    /// A plain type, a scope with a breaking bang, and a type lowercased.
    #[test]
    fn parses_type_and_scope() {
        for (s, want) in [
            ("fix: crash", (some("fix"), None)),
            ("feat(web)!: new api", (some("feat"), some("web"))),
            ("Feat(git): x", (some("feat"), some("git"))),
        ] {
            assert_eq!(p(s), want, "{s}");
        }
    }

    #[test]
    fn rejects_non_conventional() {
        for s in [
            "update readme",
            "fix:no space",
            "fix(: x",
            "fix(): x",
            "two words: x",
            ": x",
            "a1: x",
        ] {
            assert_eq!(p(s), (None, None), "{s}");
        }
    }
}
