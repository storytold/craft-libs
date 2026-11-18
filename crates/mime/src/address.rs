//! Mailbox lists (RFC 5322 §3.4): `Ada <ada@example.org>, "Hopper, Grace" <grace@example.org>`.

use crate::header::clean;
use crate::words::{decode_words, encode_words, is_plain};

/// Most addresses read from one header field.
pub const MAX_ADDRESSES: usize = 10_000;

/// One mailbox of an address list: a display name and an address, either of which may be empty.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mailbox {
    /// The display name, encoded-words decoded, quotes removed.
    pub name: String,
    /// The address (`local@domain`), as written.
    pub email: String,
}

/// What has been read of the mailbox being parsed.
#[derive(Default)]
struct Parts {
    /// The words outside angle brackets, quoted strings unquoted: the display name.
    text: String,
    /// The same as written, quotes kept: the address when there are no angle brackets.
    raw: String,
    /// The last `(comment)`: the name, in the old `address (Name)` form.
    comment: String,
    /// What stood between `<` and `>`, and how long `text` was when it was read.
    angle: Option<(String, usize)>,
}

impl Parts {
    /// White space between words: one space, none at the start.
    fn space(&mut self) {
        for s in [&mut self.text, &mut self.raw] {
            if !s.is_empty() && !s.ends_with(' ') {
                s.push(' ');
            }
        }
    }

    /// End the mailbox: add it to `out` unless it is empty, and start afresh.
    fn finish(&mut self, out: &mut Vec<Mailbox>) {
        let parts = std::mem::take(self);
        let decoded = |s: &str| decode_words(s.trim()).trim().to_string();
        let comment = decoded(&parts.comment);
        let address = match parts.angle {
            Some((angle, _)) => {
                let name = decoded(&parts.text);
                Mailbox { name: if name.is_empty() { comment } else { name }, email: without_route(angle.trim()).to_string() }
            }
            None => {
                let raw = parts.raw.trim();
                if raw.is_empty() {
                    // Nothing, or a comment on its own.
                    return;
                }
                let name = decoded(&parts.text);
                if raw.contains('@') || (name == parts.text.trim() && !name.contains(' ')) {
                    // An address, or one word that can only be a local mailbox (`postmaster`).
                    Mailbox { name: comment, email: raw.to_string() }
                } else {
                    // Words without an address: a name is all there is.
                    Mailbox { name, email: String::new() }
                }
            }
        };
        if (!address.name.is_empty() || !address.email.is_empty()) && out.len() < MAX_ADDRESSES {
            out.push(address);
        }
    }
}

/// `user@host` from the obsolete routed form `@a,@b:user@host`.
fn without_route(angle: &str) -> &str {
    if angle.starts_with('@') { angle.rsplit_once(':').map_or(angle, |(_, address)| address.trim()) } else { angle }
}

/// Parse an address header (`To`, `Cc`, `From`…), leniently.
///
/// Reads display names (quoted or not, with RFC 2047 encoded-words), `<angle>` addresses, bare
/// addresses, comments (`ada@example.org (Ada)` gives the name `Ada`), and groups
/// (`Team: a@example.org, b@example.org;` gives the two members). Mailboxes are separated by `,`
/// or `;`. Never fails: what makes no sense is kept as well as it can be, or dropped when empty.
pub fn parse_address_list(s: &str) -> Vec<Mailbox> {
    let mut out = Vec::new();
    let mut parts = Parts::default();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                parts.raw.push('"');
                while let Some(q) = chars.next() {
                    match q {
                        '"' => break,
                        '\\' => {
                            if let Some(escaped) = chars.next() {
                                parts.text.push(escaped);
                                parts.raw.push('\\');
                                parts.raw.push(escaped);
                            }
                        }
                        q => {
                            parts.text.push(q);
                            parts.raw.push(q);
                        }
                    }
                }
                parts.raw.push('"');
            }
            '(' => {
                let mut depth = 1usize;
                let mut comment = String::new();
                while let Some(q) = chars.next() {
                    match q {
                        '\\' => comment.extend(chars.next()),
                        '(' => {
                            depth = depth.saturating_add(1);
                            comment.push(q);
                        }
                        ')' => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                break;
                            }
                            comment.push(q);
                        }
                        q => comment.push(q),
                    }
                }
                parts.comment = comment;
                parts.space();
            }
            '<' => {
                if let Some((_, at)) = parts.angle {
                    // A second address with no comma before it: the words since the first are its name.
                    let name = parts.text.get(at..).map(|tail| tail.trim_start().to_string());
                    if name.is_some() {
                        parts.text.truncate(at);
                    }
                    parts.finish(&mut out);
                    parts.text = name.unwrap_or_default();
                }
                let mut address = String::new();
                let mut quoted = false;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => {
                            quoted = !quoted;
                            address.push(q);
                        }
                        '\\' if quoted => {
                            address.push(q);
                            address.extend(chars.next());
                        }
                        '>' if !quoted => break,
                        q => address.push(q),
                    }
                }
                parts.angle = Some((address, parts.text.len()));
            }
            ',' | ';' => parts.finish(&mut out),
            // The name of a group ends here; its members follow.
            ':' if parts.angle.is_none() => parts = Parts::default(),
            ' ' | '\t' | '\r' | '\n' => parts.space(),
            c => {
                parts.text.push(c);
                parts.raw.push(c);
            }
        }
        if out.len() >= MAX_ADDRESSES {
            return out;
        }
    }
    parts.finish(&mut out);
    out
}

fn is_atext(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-/=?^_`{|}~".contains(&b)
}

/// A display name as a header phrase: as it is when it is plain words, quoted when it holds
/// specials, encoded-words when it isn't printable ASCII.
fn phrase(name: &str) -> String {
    if !is_plain(name) {
        encode_words(name)
    } else if name.split(' ').all(|word| !word.is_empty() && word.bytes().all(is_atext)) {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// Whether text can stand as an address as it is: nothing in it could end the address or start another.
fn is_addr_spec(s: &str) -> bool {
    let mut quoted = false;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c.is_control() {
            return false;
        }
        if quoted {
            match c {
                '"' => quoted = false,
                '\\' if chars.next().is_none_or(char::is_control) => return false,
                _ => {}
            }
        } else if c == '"' {
            quoted = true;
        } else if c.is_whitespace() || "<>(),;:\\".contains(c) {
            return false;
        }
    }
    !quoted
}

/// An address that is safe between angle brackets: kept when well formed, else stripped of every
/// character that could break out of it.
fn safe_email(email: &str) -> String {
    let email = email.trim();
    if is_addr_spec(email) {
        return email.to_string();
    }
    email.chars().filter(|c| !c.is_control() && !c.is_whitespace() && !"<>(),;:\"\\".contains(*c)).collect()
}

/// One mailbox for a header: `Name <address>`, or the address alone when there is no name (in
/// angle brackets unless it is a plain `local@domain`, so it can't be read as a name); `None` when
/// both are empty.
pub fn format_address(address: &Mailbox) -> Option<String> {
    let email = safe_email(&address.email);
    let name = clean(&address.name);
    let name = name.trim();
    if !name.is_empty() {
        return Some(format!("{} <{email}>", phrase(name)));
    }
    if email.is_empty() {
        return None;
    }
    let simple = email.contains('@') && !email.contains("=?") && email.bytes().all(|b| is_atext(b) || b == b'.' || b == b'@');
    Some(if simple { email } else { format!("<{email}>") })
}

/// An address list for a header, separated by `, `.
pub fn format_address_list(list: &[Mailbox]) -> String {
    list.iter().filter_map(format_address).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(name: &str, email: &str) -> Mailbox {
        Mailbox { name: name.into(), email: email.into() }
    }

    #[test]
    fn the_usual_forms() {
        assert_eq!(parse_address_list("ada@example.org"), [a("", "ada@example.org")]);
        assert_eq!(parse_address_list("<ada@example.org>"), [a("", "ada@example.org")]);
        assert_eq!(parse_address_list("Ada Lovelace <ada@example.org>"), [a("Ada Lovelace", "ada@example.org")]);
        assert_eq!(parse_address_list("  Ada   Lovelace\t< ada@example.org >  "), [a("Ada Lovelace", "ada@example.org")]);
        assert_eq!(parse_address_list(""), []);
        assert_eq!(parse_address_list(" , ;, "), []);
    }

    #[test]
    fn lists() {
        let got = parse_address_list("Ada <ada@example.org>, bob@example.org,\r\n \"Hopper, Grace\" <grace@example.org>; carol@example.org");
        assert_eq!(got, [a("Ada", "ada@example.org"), a("", "bob@example.org"), a("Hopper, Grace", "grace@example.org"), a("", "carol@example.org")]);
    }

    #[test]
    fn quoted_names() {
        assert_eq!(parse_address_list(r#""Lovelace, Ada" <ada@example.org>"#), [a("Lovelace, Ada", "ada@example.org")]);
        assert_eq!(parse_address_list(r#""Ada \"The Countess\" L." <ada@example.org>"#), [a("Ada \"The Countess\" L.", "ada@example.org")]);
        assert_eq!(parse_address_list(r#""a\\b" <x@example.org>"#), [a("a\\b", "x@example.org")]);
        assert_eq!(parse_address_list(r#""Ada  L" <ada@example.org>"#), [a("Ada  L", "ada@example.org")]);
        assert_eq!(parse_address_list(r#""Dr." Ada "<not an address>" <ada@example.org>"#), [a("Dr. Ada <not an address>", "ada@example.org")]);
        assert_eq!(parse_address_list(r#""semi; colon: here" <s@example.org>"#), [a("semi; colon: here", "s@example.org")]);
    }

    #[test]
    fn comments() {
        assert_eq!(parse_address_list("ada@example.org (Ada Lovelace)"), [a("Ada Lovelace", "ada@example.org")]);
        assert_eq!(parse_address_list("Ada (the first) Lovelace <ada@example.org>"), [a("Ada Lovelace", "ada@example.org")]);
        assert_eq!(parse_address_list("<ada@example.org> (Ada (nested \\) one))"), [a("Ada (nested ) one)", "ada@example.org")]);
        assert_eq!(parse_address_list("(only a comment)"), []);
        assert_eq!(parse_address_list("a@example.org (unterminated"), [a("unterminated", "a@example.org")]);
    }

    #[test]
    fn groups() {
        let got = parse_address_list("Team: ada@example.org, Bob <bob@example.org>;, carol@example.org");
        assert_eq!(got, [a("", "ada@example.org"), a("Bob", "bob@example.org"), a("", "carol@example.org")]);
        assert_eq!(parse_address_list("undisclosed-recipients:;"), []);
        assert_eq!(parse_address_list("Empty group: ; after@example.org"), [a("", "after@example.org")]);
    }

    #[test]
    fn encoded_words_in_names() {
        assert_eq!(parse_address_list("=?UTF-8?B?QWTDoSBMw7N2ZWxhY2U=?= <ada@example.org>"), [a("Adá Lóvelace", "ada@example.org")]);
        assert_eq!(parse_address_list("=?UTF-8?Q?Ad=C3=A1?= =?UTF-8?Q?_L?= <ada@example.org>"), [a("Adá L", "ada@example.org")]);
        assert_eq!(parse_address_list("\"=?ISO-8859-1?Q?Andr=E9?=\" <andre@example.org>"), [a("André", "andre@example.org")]);
        assert_eq!(parse_address_list("andre@example.org (=?ISO-8859-1?Q?Andr=E9?=)"), [a("André", "andre@example.org")]);
        // A decoded name can't smuggle in an address.
        assert_eq!(
            parse_address_list("=?UTF-8?Q?Eve_=3Ceve@example.org=3E=2C?= <ada@example.org>"),
            [a("Eve <eve@example.org>,", "ada@example.org")]
        );
        assert_eq!(parse_address_list("Renée <renee@example.org>"), [a("Renée", "renee@example.org")]);
    }

    #[test]
    fn names_without_addresses_and_local_mailboxes() {
        assert_eq!(parse_address_list("undisclosed recipients"), [a("undisclosed recipients", "")]);
        assert_eq!(parse_address_list("postmaster"), [a("", "postmaster")]);
        assert_eq!(parse_address_list("=?UTF-8?Q?Ren=C3=A9e?="), [a("Renée", "")]);
        assert_eq!(parse_address_list("Ada <>"), [a("Ada", "")]);
    }

    #[test]
    fn odd_addresses() {
        assert_eq!(parse_address_list(r#""john doe"@example.org"#), [a("", r#""john doe"@example.org"#)]);
        assert_eq!(parse_address_list(r#"J <"a>b"@example.org>"#), [a("J", r#""a>b"@example.org"#)]);
        assert_eq!(parse_address_list("<@relay.example.org,@other.example.org:ada@example.org>"), [a("", "ada@example.org")]);
        assert_eq!(parse_address_list("Ada <ada@example.org"), [a("Ada", "ada@example.org")]);
        assert_eq!(parse_address_list("\"Unterminated"), [a("", "\"Unterminated\"")]);
        // A missing comma between two addresses.
        assert_eq!(
            parse_address_list("<a@example.org> Bob <b@example.org><c@example.org>"),
            [a("", "a@example.org"), a("Bob", "b@example.org"), a("", "c@example.org")]
        );
    }

    #[test]
    fn many_addresses_are_capped() {
        let s = "a@example.org,".repeat(MAX_ADDRESSES + 100);
        assert_eq!(parse_address_list(&s).len(), MAX_ADDRESSES);
        let _ = parse_address_list(&"(".repeat(100_000));
        let _ = parse_address_list(&"<".repeat(100_000));
        let _ = parse_address_list(&"\"\\".repeat(100_000));
    }

    #[test]
    fn writes_names_as_needed() {
        assert_eq!(format_address_list(&[a("", "ada@example.org")]), "ada@example.org");
        assert_eq!(format_address_list(&[a("Ada Lovelace", "ada@example.org")]), "Ada Lovelace <ada@example.org>");
        assert_eq!(format_address_list(&[a("Lovelace, Ada", "ada@example.org")]), "\"Lovelace, Ada\" <ada@example.org>");
        assert_eq!(format_address_list(&[a("Ada \"C\" L\\", "ada@example.org")]), r#""Ada \"C\" L\\" <ada@example.org>"#);
        assert_eq!(format_address_list(&[a("J. Doe", "j@example.org")]), "\"J. Doe\" <j@example.org>");
        assert_eq!(format_address_list(&[a("Adá", "ada@example.org")]), "=?UTF-8?B?QWTDoQ==?= <ada@example.org>");
        assert_eq!(format_address_list(&[a("a", "a@example.org"), a("", ""), a("", "b@example.org")]), "a <a@example.org>, b@example.org");
        assert_eq!(format_address_list(&[a("Name only", "")]), "Name only <>");
        assert_eq!(format_address_list(&[]), "");
    }

    #[test]
    fn addresses_cannot_inject_recipients_or_headers() {
        let evil = a("Eve\r\nBcc: x@example.org", "a@example.org>, <evil@example.org");
        let written = format_address_list(&[evil]);
        assert!(!written.contains(['\r', '\n']));
        let back = parse_address_list(&written);
        assert_eq!(back.len(), 1);
        assert_eq!(back.first().map(|x| x.email.as_str()), Some("a@example.orgevil@example.org"));
        assert_eq!(format_address_list(&[a("", "a@example.org, b@example.org")]), "a@example.orgb@example.org");
        assert_eq!(format_address_list(&[a("", "a b\t@example.org\r\n")]), "ab@example.org");
        assert_eq!(format_address_list(&[a("x", "\"unterminated@example.org")]), "x <unterminated@example.org>");
        // A well-formed quoted local part is kept.
        assert_eq!(format_address_list(&[a("", "\"john doe\"@example.org")]), "<\"john doe\"@example.org>");
    }

    #[test]
    fn an_address_alone_reads_back_as_an_address() {
        for email in ["ada@example.org", "postmaster", "=?UTF-8?Q?x?=", "\"a b\"", "\"john doe\"@example.org", "ünï@example.org", "a=?b@example.org"]
        {
            let list = [a("", email)];
            assert_eq!(parse_address_list(&format_address_list(&list)), list, "{email}");
        }
        assert_eq!(format_address_list(&[a("", "postmaster")]), "<postmaster>");
    }

    fn name() -> impl proptest::strategy::Strategy<Value = String> {
        use proptest::prelude::*;
        prop_oneof![any::<String>(), "[ -~]{0,30}", "[a-zA-Z .,\"\\\\()<>:;@=?]{0,20}"]
            .prop_map(|s| s.replace(['\r', '\n', '\0'], "").trim().to_string())
    }

    proptest::proptest! {
        #[test]
        fn written_lists_read_back(names in proptest::collection::vec(name(), 0..5)) {
            let list: Vec<Mailbox> = names.iter().enumerate().map(|(i, n)| a(n, &format!("user{i}@example.org"))).collect();
            proptest::prop_assert_eq!(parse_address_list(&format_address_list(&list)), list);
        }

        #[test]
        fn any_text_parses(s in proptest::prelude::any::<String>()) {
            let _ = parse_address_list(&s);
        }

        #[test]
        fn address_soup_parses(s in "[a-c@<>\"(),;:\\\\ =?.]{0,40}") {
            let _ = parse_address_list(&s);
        }

        #[test]
        fn any_address_writes_one_mailbox(n in proptest::prelude::any::<String>(), e in proptest::prelude::any::<String>()) {
            let written = format_address_list(&[a(&n, &e)]);
            proptest::prop_assert!(!written.contains(['\r', '\n', '\0']));
            proptest::prop_assert!(parse_address_list(&written).len() <= 1);
        }
    }
}
