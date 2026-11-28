//! Agent hints a commit message carries itself: an `Agent-Session:` trailer or a
//! `Co-authored-by:` naming a coding agent. Nothing outside the repository is read.
//! Git has no trailer parser in the other crates, so the last paragraph is read here.

/// Agent names matched as whole words in a co-author name, lowercase.
const NAMES: [&str; 6] = ["claude", "codex", "gemini", "copilot", "cursor", "aider"];

/// Email domains of agent noreply addresses, matched exactly, with the agent each stands for.
const DOMAINS: [(&str, &str); 2] = [("anthropic.com", "claude"), ("openai.com", "codex")];

/// Agents with a known way to resume a session. Others only show their id.
const RESUME: [(&str, &str); 1] = [("claude", "claude --resume")];

/// One agent hint for the commit page.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AgentLink {
    pub(crate) label: String,
    /// The session id to copy, when the trailer gave one.
    pub(crate) session: Option<String>,
    /// A command that resumes the session, only when the agent is named and known.
    pub(crate) resume: Option<String>,
}

fn value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let (k, v) = line.split_once(':')?;
    k.trim().eq_ignore_ascii_case(key).then(|| v.trim())
}

fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The agent a name or `Name <email>` value stands for, if any.
fn agent_of(who: &str) -> Option<&'static str> {
    let (name, mail) = who.split_once('<').unwrap_or((who, ""));
    let lower = name.to_ascii_lowercase();
    let word =
        NAMES.iter().find(|n| lower.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w == **n));
    let domain = mail.trim_end_matches('>').rsplit('@').next().unwrap_or("").to_ascii_lowercase();
    word.copied().or_else(|| DOMAINS.iter().find(|(d, _)| *d == domain).map(|(_, n)| *n))
}

/// The hints in the trailer block, the last paragraph of the message, in message order.
pub(crate) fn links(message: &str) -> Vec<AgentLink> {
    let block = message.trim_end().rsplit("\n\n").next().unwrap_or("");
    let mut named: Option<String> = None;
    let mut sessions = Vec::new();
    let mut out = Vec::new();
    for line in block.lines() {
        if let Some(a) = value(line, "Agent") {
            named = Some(a.to_ascii_lowercase());
        } else if let Some(id) = value(line, "Agent-Session") {
            if safe_id(id) {
                sessions.push(id.to_owned());
            }
        } else if let Some((who, a)) =
            value(line, "Co-authored-by").and_then(|w| agent_of(w).map(|a| (w, a)))
        {
            named.get_or_insert_with(|| a.to_owned());
            let name = who.split('<').next().unwrap_or("").trim();
            out.push(AgentLink {
                label: format!("Co-authored by {name}"),
                session: None,
                resume: None,
            });
        }
    }
    for id in sessions {
        let resume = named.as_deref().and_then(|n| {
            RESUME.iter().find(|(k, _)| *k == n).map(|(_, cmd)| format!("{cmd} {id}"))
        });
        out.push(AgentLink { label: "Agent session".to_owned(), session: Some(id), resume });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::links;

    #[test]
    fn a_session_trailer_alone_offers_the_id_but_no_command() {
        let got = links("feat: x\n\nbody\n\nAgent-Session: 3f2a-bc_9\n");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].session.as_deref(), Some("3f2a-bc_9"));
        assert_eq!(got[0].resume, None);
    }

    #[test]
    fn a_claude_session_gets_a_resume_command() {
        let got = links("x\n\nAgent: claude\nAgent-Session: abc\n");
        assert_eq!(got[0].resume.as_deref(), Some("claude --resume abc"));
        let got =
            links("x\n\nCo-authored-by: Claude <noreply@anthropic.com>\nAgent-Session: abc\n");
        assert_eq!(got.last().and_then(|l| l.resume.as_deref()), Some("claude --resume abc"));
    }

    #[test]
    fn a_codex_session_gets_no_claude_command() {
        let got = links("x\n\nAgent: codex\nAgent-Session: abc\n");
        assert_eq!(got[0].resume, None);
    }

    #[test]
    fn an_unsafe_session_id_is_dropped() {
        assert_eq!(links("x\n\nAgent-Session: a; rm -rf ~\n"), []);
    }

    #[test]
    fn only_agent_co_authors_are_listed() {
        let msg = "x\n\nCo-authored-by: Claude <noreply@anthropic.com>\n\
                   Co-authored-by: Ana Dev <ana@example.com>\n\
                   Co-authored-by: Cursorini Rossi <c@example.com>\n";
        let got = links(msg);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].label, "Co-authored by Claude");
    }

    #[test]
    fn a_trailer_outside_the_last_paragraph_is_ignored() {
        assert_eq!(links("x\n\nAgent-Session: abc\n\nmore prose"), []);
    }
}
