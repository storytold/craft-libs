// One character walker, with explicit output policies: ordinary Markdown keeps line
// structure; extracted text flattens it and strips unrepresentable/control characters.
pub(crate) fn escape(s: &str, extracted: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if extracted && !matches!(c, '\t' | '\n' | '\r') && (c.is_ascii_control() || matches!(c, '\u{FFFE}' | '\u{FFFF}')) {
            continue;
        }
        match c {
            '\n' | '\r' | '\t' if extracted => out.push(' '),
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '~' | '|' | '&' => {
                out.push('\\');
                out.push(c);
            }
            '#' | '!' if extracted => {
                out.push('\\');
                out.push(c);
            }
            '\u{000C}' | '\u{000E}' => {}
            _ => out.push(c),
        }
    }
    out
}
