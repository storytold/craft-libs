//! `mailto:` links (RFC 6068): who a new message is for and how it starts.
//!
//! A link is hostile input like any other: it comes from a web page, a document or a message.
//! Only the recipients, the subject and the body are taken from it. Every other header field a
//! link names is ignored (RFC 6068 section 7: a link must not get to set headers of its choice),
//! nothing taken from it can carry a line break into a header, and sizes are capped.

/// What a `mailto:` link asks a new message to start with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mailto {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: String,
}

/// The most of a link that is read.
const MAX_LINK: usize = 64 * 1024;
/// The most recipients taken for one field.
const MAX_ADDRESSES: usize = 200;
/// The longest subject taken (a header line's limit in RFC 5322).
const MAX_SUBJECT: usize = 998;

/// Whether `text` is a `mailto:` link (by its scheme, whatever its case).
pub fn is_mailto(text: &str) -> bool {
    rest_of(text).is_some()
}

/// What follows the scheme, when `text` is a `mailto:` link.
fn rest_of(text: &str) -> Option<&str> {
    let text = text.trim();
    let scheme = text.get(..7)?;
    if scheme.eq_ignore_ascii_case("mailto:") { text.get(7..) } else { None }
}

/// Read a `mailto:` link. `None` when `link` isn't one; a link with nothing after the scheme is
/// an empty message.
pub fn parse_mailto(link: &str) -> Option<Mailto> {
    let rest = cut(rest_of(link)?, MAX_LINK);
    // A fragment is no part of the message.
    let rest = rest.split('#').next().unwrap_or("");
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut m = Mailto::default();
    // Commas separate the addresses before they are decoded: an encoded comma belongs to its address.
    for address in path.split(',') {
        add(&mut m.to, &decode(address));
    }
    let (mut subject, mut body) = (None, None);
    for field in query.split('&') {
        let (name, value) = field.split_once('=').unwrap_or((field, ""));
        let value = decode(value);
        match decode(name).to_ascii_lowercase().as_str() {
            "to" => add_list(&mut m.to, &value),
            "cc" => add_list(&mut m.cc, &value),
            "bcc" => add_list(&mut m.bcc, &value),
            // The first of each counts; a link that repeats one is not to be trusted with the rest.
            "subject" if subject.is_none() => subject = Some(one_line(&value, MAX_SUBJECT)),
            "body" if body.is_none() => body = Some(lines(&value)),
            _ => {}
        }
    }
    m.subject = subject.unwrap_or_default();
    m.body = body.unwrap_or_default();
    Some(m)
}

/// `text` up to `max` bytes, cut where a character ends.
pub(crate) fn cut(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or("")
}

/// Percent-decoding. An escape that isn't one stays as it is written, a plus sign is a plus sign
/// (RFC 6068 section 5), and bytes that aren't UTF-8 become the replacement character.
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let escaped = if b == b'%' { hex(bytes.get(i + 1)).zip(hex(bytes.get(i + 2))) } else { None };
        match escaped {
            Some((high, low)) => {
                out.push((high << 4) | low);
                i += 3;
            }
            None => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: Option<&u8>) -> Option<u8> {
    char::from(*b?).to_digit(16).and_then(|d| u8::try_from(d).ok())
}

/// One header line's worth of text: line breaks and other control characters become spaces.
fn one_line(text: &str, max: usize) -> String {
    let line: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    cut(line.trim(), max).trim_end().to_string()
}

/// Body text: every kind of line break becomes `\n`, and no other control character but the tab
/// is kept.
fn lines(text: &str) -> String {
    text.replace("\r\n", "\n").chars().map(|c| if c == '\r' { '\n' } else { c }).filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).collect()
}

/// One address, if anything is left of it once it is a single clean line.
fn add(list: &mut Vec<String>, address: &str) {
    let address = one_line(address, MAX_SUBJECT);
    if !address.is_empty() && list.len() < MAX_ADDRESSES {
        list.push(address);
    }
}

/// The addresses of a decoded header value: separated by commas, except inside a quoted name or
/// an angle-bracketed address.
fn add_list(list: &mut Vec<String>, value: &str) {
    let (mut start, mut quoted, mut angle, mut escaped) = (0, false, false, false);
    for (i, c) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '<' if !quoted => angle = true,
            '>' if !quoted => angle = false,
            ',' if !quoted && !angle => {
                add(list, value.get(start..i).unwrap_or(""));
                start = i + 1;
            }
            _ => {}
        }
    }
    add(list, value.get(start..).unwrap_or(""));
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn s(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn reads_the_examples_of_rfc_6068() {
        assert_eq!(parse_mailto("mailto:chris@example.org"), Some(Mailto { to: s(&["chris@example.org"]), ..Default::default() }));
        let m = parse_mailto("mailto:infobot@example.org?subject=current-issue").unwrap();
        assert_eq!((m.to, m.subject.as_str()), (s(&["infobot@example.org"]), "current-issue"));
        let m = parse_mailto("mailto:infobot@example.org?body=send%20current-issue%0D%0Asend%20index").unwrap();
        assert_eq!(m.body, "send current-issue\nsend index");
        let m = parse_mailto("mailto:joe@example.org?cc=bob@example.org&body=hello").unwrap();
        assert_eq!((m.to, m.cc, m.body.as_str()), (s(&["joe@example.org"]), s(&["bob@example.org"]), "hello"));
        // Addresses in the path, in `to` fields, or both.
        let m = parse_mailto("mailto:addr1@an.example,addr2@an.example").unwrap();
        assert_eq!(m.to, s(&["addr1@an.example", "addr2@an.example"]));
        let m = parse_mailto("mailto:?to=addr1@an.example,addr2@an.example").unwrap();
        assert_eq!(m.to, s(&["addr1@an.example", "addr2@an.example"]));
        let m = parse_mailto("mailto:addr1@an.example?to=addr2@an.example").unwrap();
        assert_eq!(m.to, s(&["addr1@an.example", "addr2@an.example"]));
        // Encoded characters in an address, and UTF-8 in a subject.
        assert_eq!(parse_mailto("mailto:gorby%25kremvax@example.org").unwrap().to, s(&["gorby%kremvax@example.org"]));
        assert_eq!(parse_mailto("mailto:user@example.org?subject=caf%C3%A9").unwrap().subject, "café");
        assert_eq!(parse_mailto("mailto:%22not%40me%22@example.org").unwrap().to, s(&["\"not@me\"@example.org"]));
    }

    #[test]
    fn knows_a_link_from_anything_else() {
        assert!(is_mailto("  MailTo:someone@example.org "));
        assert!(!is_mailto("mailbox.example"));
        assert!(!is_mailto("mail"));
        assert!(!is_mailto("https://example.org/?mailto:x"));
        assert_eq!(parse_mailto("C:\\mail\\mailto.eml"), None);
        assert_eq!(parse_mailto("mailto:"), Some(Mailto::default()));
        assert_eq!(parse_mailto("MAILTO:a@example.org#top").unwrap().to, s(&["a@example.org"]));
    }

    #[test]
    fn a_link_sets_nothing_but_recipients_subject_and_body() {
        let m =
            parse_mailto("mailto:a@example.org?from=boss@example.org&reply-to=x@evil.example&attach=C:%5Csecrets.txt&X-Mailer=m&subject=Hi").unwrap();
        assert_eq!(m, Mailto { to: s(&["a@example.org"]), subject: "Hi".into(), ..Default::default() });
        // No line break reaches a header, whatever the link encodes.
        let m = parse_mailto(
            "mailto:a@example.org%0D%0ABcc:%20x@evil.example?subject=Hi%0D%0ABcc:%20y@evil.example&cc=c@example.org%0Abcc:z@evil.example",
        )
        .unwrap();
        assert!(m.bcc.is_empty());
        for text in m.to.iter().chain(&m.cc).chain([&m.subject]) {
            assert!(!text.contains(['\r', '\n', '\0']), "{text:?}");
        }
        // The first subject and body count.
        let m = parse_mailto("mailto:?subject=one&subject=two&body=first&body=second").unwrap();
        assert_eq!((m.subject.as_str(), m.body.as_str()), ("one", "first"));
        // A plus sign is not a space, a broken escape is what it looks like.
        let m = parse_mailto("mailto:a+b@example.org?subject=1+1%3D2%&body=100%25%zz").unwrap();
        assert_eq!((m.to, m.subject.as_str(), m.body.as_str()), (s(&["a+b@example.org"]), "1+1=2%", "100%%zz"));
    }

    #[test]
    fn splits_recipient_lists_the_way_addresses_are_written() {
        let m = parse_mailto("mailto:?cc=%22Rivera%2C%20Tom%C3%A1s%22%20%3Ctomas@example.org%3E,%20lin@example.org&bcc=a@example.org,,b@example.org")
            .unwrap();
        assert_eq!(m.cc, s(&["\"Rivera, Tomás\" <tomas@example.org>", "lin@example.org"]));
        assert_eq!(m.bcc, s(&["a@example.org", "b@example.org"]));
    }

    #[test]
    fn sizes_are_capped() {
        let many = format!("mailto:{}", "a@example.org,".repeat(1000));
        assert_eq!(parse_mailto(&many).unwrap().to.len(), MAX_ADDRESSES);
        let long = format!("mailto:a@example.org?subject={}&body={}", "é".repeat(5000), "x".repeat(200_000));
        let m = parse_mailto(&long).unwrap();
        assert!(m.subject.len() <= MAX_SUBJECT && m.subject.chars().all(|c| c == 'é'));
        assert!(m.body.len() <= MAX_LINK);
    }

    proptest! {
        #[test]
        fn any_text_is_read_without_trouble(text in ".{0,300}", tail in proptest::collection::vec(any::<u8>(), 0..300)) {
            let _ = parse_mailto(&text);
            let link = format!("mailto:{text}?{}", String::from_utf8_lossy(&tail));
            let m = parse_mailto(&link).unwrap();
            for line in m.to.iter().chain(&m.cc).chain(&m.bcc).chain([&m.subject]) {
                prop_assert!(!line.chars().any(char::is_control));
            }
            prop_assert!(!m.body.contains('\r'));
        }
    }
}
