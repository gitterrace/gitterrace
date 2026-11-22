//! Checks over the safety section of the built landing page.

use crate::site::tests::generated;

/// Whether the landing page has the trust strip, with the default address the code uses and
/// a link to the security model page.
fn safety_problems(landing: &str, addr: &str) -> Vec<String> {
    let mut bad = Vec::new();
    let Some((_, rest)) = landing.split_once("id=\"your-code\">") else {
        return vec!["the landing page has no trust strip (#your-code)".to_owned()];
    };
    let section = rest.split("</div>").next().unwrap_or("");
    if !section.contains(&format!("<code>{addr}</code>")) {
        bad.push(format!("the safety section does not name {addr}"));
    }
    if !section.contains("href=\"/docs/security/\"") {
        bad.push("the safety section does not link the security model page".to_owned());
    }
    bad
}

/// The class this guards: the landing page losing the promise section, or stating an address
/// other than the one the code serves on. The prose stays in docs/landing.md, short beside
/// SECURITY.md's long list, and `landing_prose_that_names_a_wrong_address_or_cache_path_fails`
/// ties its facts to the code.
#[test]
fn the_built_landing_page_has_the_safety_section_with_the_real_address() {
    let (pages, _) = generated();
    let landing = &pages.iter().find(|p| p.0 == "index.html").expect("landing").1;
    let bad = safety_problems(landing, gitterrace_config::DEFAULT_ADDR);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn a_landing_page_without_the_section_or_with_another_address_is_reported() {
    let (pages, _) = generated();
    let landing = &pages.iter().find(|p| p.0 == "index.html").expect("landing").1;
    let gone = landing.replace("id=\"your-code\"", "id=\"safety\"");
    assert!(safety_problems(&gone, "127.0.0.1:4242")[0].contains("has no"));
    let bad = safety_problems(landing, "127.0.0.1:9");
    assert!(bad[0].contains("does not name 127.0.0.1:9"), "{bad:?}");
}
