/// Splits `type(scope)!: description` into type and scope. Subjects that do not follow the
/// convention yield `(None, None)`.
pub(crate) fn parse_subject(subject: &str) -> (Option<String>, Option<String>) {
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

    fn some(s: &str) -> Option<String> {
        Some(s.to_owned())
    }

    #[test]
    fn plain_type() {
        assert_eq!(p("fix: crash"), (some("fix"), None));
    }

    #[test]
    fn type_with_scope_and_bang() {
        assert_eq!(p("feat(web)!: new api"), (some("feat"), some("web")));
    }

    #[test]
    fn type_is_lowercased() {
        assert_eq!(p("Feat(git): x"), (some("feat"), some("git")));
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
