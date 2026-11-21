//! A stylesheet with the bytes a browser never reads taken out: comments, runs of
//! white space, the space around braces, semicolons and commas, and the last `;` of a block.
//! Strings are copied whole, and white space next to `:`, `+`, `-` and `>` stays, since it
//! can change what a selector or a `calc()` means.

/// `css` without comments and with the least white space that parses to the same rules.
pub fn css(css: &str) -> String {
    let mut out = String::with_capacity(css.len() / 2);
    let (mut depth, mut space) = (0_usize, false);
    let mut chars = css.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = ' ';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
                space = true;
            }
            '"' | '\'' => {
                if space && !out.is_empty() && !out.ends_with(['{', '}', ';', ',', '\n']) {
                    out.push(' ');
                }
                space = false;
                out.push(c);
                for n in chars.by_ref() {
                    out.push(n);
                    if n == c {
                        break;
                    }
                }
            }
            c if c.is_whitespace() => space = true,
            c => {
                let tight = matches!(c, '{' | '}' | ';' | ',')
                    || out.is_empty()
                    || out.ends_with(['{', '}', ';', ',', '\n']);
                if space && !tight {
                    out.push(' ');
                }
                space = false;
                if c == '}' && out.ends_with(';') {
                    out.pop();
                }
                out.push(c);
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            out.push('\n');
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_and_blank_space_go_and_meaning_stays() {
        let src = "/* c */\na , b {\n  color : red ;\n  margin: calc(1px + 2px) ;\n}\n\
                   .x :hover{content:\"a  b;\"}";
        assert_eq!(
            css(src),
            "a,b{color : red;margin: calc(1px + 2px)}\n.x :hover{content:\"a  b;\"}\n"
        );
    }

    #[test]
    fn nested_blocks_keep_their_closing_brace() {
        assert_eq!(css("@media (a) { .x { y: 1; } }"), "@media (a){.x{y: 1}}\n");
    }
}
