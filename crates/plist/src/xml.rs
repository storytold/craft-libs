//! XML property-list reader ported from Craft Anything at 8f3b7751.
use crate::{DateValue, FormatError, Limits, Value};

/// XML-specific representation policy, separate from resource allowances.
#[derive(Clone, Copy, Debug)]
pub struct XmlOptions {
    /// Trim surrounding whitespace in date text, as the original Craft Anything reader does.
    /// Disable for consumers that retain dates as opaque strings, such as Luminar presets.
    pub trim_date_text: bool,
}
impl Default for XmlOptions {
    fn default() -> Self {
        Self { trim_date_text: true }
    }
}
fn decode_base64(text: &str, url_safe: bool) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    let mut digits = 0usize;
    let mut padding = 0usize;
    for c in text.bytes() {
        match c {
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            b'=' => {
                padding += 1;
                if padding > 2 {
                    return None;
                }
                continue;
            }
            _ if padding != 0 => return None,
            _ => {}
        }
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'-' if url_safe => 62,
            b'_' if url_safe => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(value);
        bits += 6;
        digits += 1;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    let remainder = digits % 4;
    if remainder == 1 || (padding != 0 && !matches!((remainder, padding), (2, 2) | (3, 1))) {
        return None;
    }
    Some(out)
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    decode_base64(s, false)
}
#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Open(String),
    Close(String),
    Empty(String),
    Text(String),
}

// Ported from LightCraft f786b875: scan one entity-name run, never the remaining document.
fn entity_end(rest: &str) -> Option<usize> {
    let n = rest.bytes().skip(1).take_while(|b| b.is_ascii_alphanumeric() || *b == b'#').count();
    (rest.as_bytes().get(n + 1) == Some(&b';')).then_some(n + 1)
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(rest.get(..p).unwrap_or(""));
        let after = rest.get(p..).unwrap_or("");
        let Some(end) = entity_end(after) else {
            out.push('&');
            rest = after.get(1..).unwrap_or("");
            continue;
        };
        let ent = after.get(1..end).unwrap_or("");
        let rep = match ent {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e if e.starts_with("#x") || e.starts_with("#X") => {
                let d = e.get(2..).unwrap_or("");
                if d.bytes().all(|b| b.is_ascii_hexdigit()) { u32::from_str_radix(d, 16).ok().and_then(char::from_u32) } else { None }
            }
            e if e.starts_with('#') => {
                let d = e.get(1..).unwrap_or("");
                if d.bytes().all(|b| b.is_ascii_digit()) { d.parse::<u32>().ok().and_then(char::from_u32) } else { None }
            }
            _ => None,
        };
        match rep {
            Some(c) => {
                out.push(c);
                rest = after.get(end + 1..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = after.get(1..).unwrap_or("");
            }
        }
    }
    out.push_str(rest);
    out
}

/// Skip a declaration without interpreting or fetching any entity. A DOCTYPE's
/// internal subset can itself contain declarations, quoted delimiters, comments and PIs.
fn after_declaration(mut rest: &str) -> Result<&str, FormatError> {
    let mut quote = None;
    let mut brackets = 0usize;
    while let Some(ch) = rest.chars().next() {
        if quote.is_none() {
            if let Some(comment) = rest.strip_prefix("<!--") {
                rest = comment.split_once("-->").ok_or_else(|| FormatError::Truncated("comment in declaration".into()))?.1;
                continue;
            }
            if let Some(pi) = rest.strip_prefix("<?") {
                rest = pi.split_once("?>").ok_or_else(|| FormatError::Truncated("processing instruction in declaration".into()))?.1;
                continue;
            }
        }
        rest = rest.get(ch.len_utf8()..).unwrap_or("");
        match (quote, ch) {
            (Some(q), c) if q == c => quote = None,
            (None, '\'' | '"') => quote = Some(ch),
            (None, '[') => brackets += 1,
            (None, ']') => brackets = brackets.saturating_sub(1),
            (None, '>') if brackets == 0 => return Ok(rest),
            _ => {}
        }
    }
    Err(FormatError::Truncated("declaration".into()))
}

fn tokenize(text: &str, max_tokens: usize) -> Result<Vec<Tok>, FormatError> {
    let mut toks = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if toks.len() >= max_tokens {
            return Err(FormatError::TooLarge("too many XML tokens".into()));
        }
        match rest.find('<') {
            None => {
                if !rest.is_empty() {
                    toks.push(Tok::Text(decode_entities(rest)));
                }
                break;
            }
            Some(p) => {
                let t = rest.get(..p).unwrap_or("");
                if !t.is_empty() {
                    toks.push(Tok::Text(decode_entities(t)));
                }
                rest = rest.get(p..).unwrap_or("");
            }
        }
        if let Some(r) = rest.strip_prefix("<!--") {
            let e = r.find("-->").ok_or_else(|| FormatError::Truncated("comment".into()))?;
            rest = r.get(e + 3..).unwrap_or("");
            continue;
        }
        if let Some(r) = rest.strip_prefix("<![CDATA[") {
            let e = r.find("]]>").ok_or_else(|| FormatError::Truncated("CDATA".into()))?;
            toks.push(Tok::Text(r.get(..e).unwrap_or("").to_string()));
            rest = r.get(e + 3..).unwrap_or("");
            continue;
        }
        if let Some(r) = rest.strip_prefix("<?") {
            let end = r.find("?>").ok_or_else(|| FormatError::Truncated("processing instruction".into()))?;
            rest = r.get(end + 2..).unwrap_or("");
            continue;
        }
        if let Some(declaration) = rest.strip_prefix("<!") {
            rest = after_declaration(declaration)?;
            continue;
        }
        let mut quote = None;
        let e = rest
            .find(|ch| match (quote, ch) {
                (Some(q), c) if q == c => {
                    quote = None;
                    false
                }
                (None, '\'' | '"') => {
                    quote = Some(ch);
                    false
                }
                (None, '>') => true,
                _ => false,
            })
            .ok_or_else(|| FormatError::Truncated("tag".into()))?;
        let inner = rest.get(1..e).unwrap_or("").trim();
        rest = rest.get(e + 1..).unwrap_or("");
        if let Some(name) = inner.strip_prefix('/') {
            toks.push(Tok::Close(name.trim().to_string()));
        } else if let Some(body) = inner.strip_suffix('/') {
            toks.push(Tok::Empty(body.split_whitespace().next().unwrap_or("").to_string()));
        } else {
            toks.push(Tok::Open(inner.split_whitespace().next().unwrap_or("").to_string()));
        }
    }
    if toks.len() > max_tokens {
        return Err(FormatError::TooLarge("too many XML tokens".into()));
    }
    Ok(toks)
}

struct Xml {
    toks: Vec<Tok>,
    pos: usize,
    nodes: usize,
    limits: Limits,
    bytes_left: usize,
    options: XmlOptions,
}

impl Xml {
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    /// Text up to the closing tag `name`.
    fn text_until(&mut self, name: &str) -> Result<String, FormatError> {
        let mut s = String::new();
        loop {
            match self.next() {
                Some(Tok::Text(t)) => {
                    self.bytes_left = self.bytes_left.checked_sub(t.len()).ok_or_else(|| FormatError::TooLarge("XML text/data bytes".into()))?;
                    s.push_str(&t);
                }
                Some(Tok::Close(n)) if n == name => return Ok(s),
                Some(other) => return Err(FormatError::Malformed(format!("unexpected {other:?} inside <{name}>"))),
                None => return Err(FormatError::Truncated(format!("<{name}> is not closed"))),
            }
        }
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(Tok::Text(t)) if t.bytes().all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))) {
            self.pos += 1;
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, FormatError> {
        self.nodes += 1;
        if depth > self.limits.max_depth.min(256) || self.nodes > self.limits.max_nodes.min(self.limits.max_objects) {
            return Err(FormatError::TooLarge("plist nesting or size limit".into()));
        }
        self.skip_space();
        match self.next() {
            Some(Tok::Empty(n)) => Ok(match n.as_str() {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                "dict" => Value::Dict(Vec::new()),
                "array" => Value::Array(Vec::new()),
                "string" => Value::String(String::new()),
                "data" => Value::Data(Vec::new()),
                other => return Err(FormatError::Malformed(format!("unexpected <{other}/>"))),
            }),
            Some(Tok::Open(n)) => match n.as_str() {
                "string" => Ok(Value::String(self.text_until("string")?)),
                "integer" => {
                    let t = self.text_until("integer")?;
                    let t = t.trim();
                    let v = if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
                        i128::from_str_radix(h, 16).ok()
                    } else {
                        t.parse::<i128>().ok()
                    };
                    v.map(Value::Integer).ok_or_else(|| FormatError::Malformed(format!("bad integer {t:?}")))
                }
                "real" => {
                    let t = self.text_until("real")?;
                    t.trim()
                        .parse::<f64>()
                        .ok()
                        .filter(|v| v.is_finite())
                        .map(Value::Real)
                        .ok_or_else(|| FormatError::Malformed(format!("bad real {t:?}")))
                }
                "date" => {
                    let text = self.text_until("date")?;
                    Ok(Value::Date(DateValue::Text(if self.options.trim_date_text { text.trim().to_string() } else { text })))
                }
                "data" => {
                    let t = self.text_until("data")?;
                    base64_decode(&t).map(Value::Data).ok_or_else(|| FormatError::Malformed("bad base64 in <data>".into()))
                }
                "true" | "false" => {
                    if !self.text_until(&n)?.is_empty() {
                        return Err(FormatError::Malformed(format!("<{n}> must be empty")));
                    }
                    Ok(Value::Bool(n == "true"))
                }
                "array" => {
                    let mut out = Vec::new();
                    loop {
                        self.skip_space();
                        if matches!(self.peek(), Some(Tok::Close(c)) if c == "array") {
                            self.pos += 1;
                            return Ok(Value::Array(out));
                        }
                        if self.peek().is_none() {
                            return Err(FormatError::Truncated("<array> is not closed".into()));
                        }
                        out.push(self.value(depth + 1)?);
                    }
                }
                "dict" => {
                    let mut out = Vec::new();
                    loop {
                        self.skip_space();
                        match self.next() {
                            Some(Tok::Close(c)) if c == "dict" => return Ok(Value::Dict(out)),
                            Some(Tok::Open(k)) if k == "key" => {
                                let key = self.text_until("key")?;
                                let v = self.value(depth + 1)?;
                                out.push((key, v));
                            }
                            Some(Tok::Empty(k)) if k == "key" => {
                                let v = self.value(depth + 1)?;
                                out.push((String::new(), v));
                            }
                            Some(other) => return Err(FormatError::Malformed(format!("expected <key> in <dict>, found {other:?}"))),
                            None => return Err(FormatError::Truncated("<dict> is not closed".into())),
                        }
                    }
                }
                other => Err(FormatError::Malformed(format!("unknown plist element <{other}>"))),
            },
            Some(other) => Err(FormatError::Malformed(format!("expected a value, found {other:?}"))),
            None => Err(FormatError::Truncated("expected a value".into())),
        }
    }
}

/// Decode a UTF-16 XML property list after its byte-order mark into UTF-8 text.
/// Surrogate pairs join normally; a lone surrogate fails as malformed rather
/// than turning into replacement characters that tokenize into garbage element
/// names.
fn decode_utf16_xml(data: &[u8], big_endian: bool) -> Result<String, FormatError> {
    if !data.len().is_multiple_of(2) {
        return Err(FormatError::Truncated("UTF-16 code unit".into()));
    }
    let units: Vec<u16> = data.as_chunks::<2>().0.iter().map(|b| if big_endian { u16::from_be_bytes(*b) } else { u16::from_le_bytes(*b) }).collect();
    String::from_utf16(&units).map_err(|_| FormatError::Malformed("invalid UTF-16 in XML plist".into()))
}

/// Parse an XML property list.
pub fn parse_xml(data: &[u8]) -> Result<Value, FormatError> {
    parse_xml_with_limits(data, Limits::default())
}

/// Parse UTF-8 or BOM-tagged UTF-16 XML with explicit resource allowances.
/// Input and decoded XML text are bounded before tokenization; token count, value count,
/// nesting and cumulative leaf/key text are bounded separately. `max_objects` and
/// `max_nodes` both cap values. Nesting never exceeds 256 even with a larger caller limit.
/// Data text consumes its encoded size, conservatively including whitespace.
pub fn parse_xml_with_limits(data: &[u8], limits: Limits) -> Result<Value, FormatError> {
    parse_xml_with_options(data, limits, XmlOptions::default())
}

/// Parse bounded XML using an explicit date-text representation policy.
pub fn parse_xml_with_options(data: &[u8], limits: Limits, options: XmlOptions) -> Result<Value, FormatError> {
    let max_tokens = limits.max_nodes.min(limits.max_objects).saturating_mul(8).saturating_add(8);
    parse_xml_with_token_limit(data, limits, options, max_tokens)
}

/// Parse XML with an independent token allowance, without reducing the value allowance.
/// This bounds token-heavy input (for example many CDATA chunks in one string).
/// Zero tokens refuses nonempty XML. Existing convenience APIs retain their default budget.
pub fn parse_xml_with_token_limit(data: &[u8], limits: Limits, options: XmlOptions, max_tokens: usize) -> Result<Value, FormatError> {
    if data.len() > limits.max_input_bytes {
        return Err(FormatError::TooLarge("input bytes".into()));
    }
    // Apple's Core Foundation reader accepts XML plists in UTF-16 as well as
    // UTF-8: it recognises both byte-order marks. `from_utf8_lossy` turns a
    // UTF-16 byte stream into NUL-separated text, so the encoding is settled
    // here, before tokenizing.
    let text = match data {
        [0xFF, 0xFE, rest @ ..] => decode_utf16_xml(rest, false)?,
        [0xFE, 0xFF, rest @ ..] => decode_utf16_xml(rest, true)?,
        _ => std::str::from_utf8(data.strip_prefix(b"\xef\xbb\xbf").unwrap_or(data))
            .map_err(|_| FormatError::Malformed("invalid UTF-8 in XML plist".into()))?
            .to_owned(),
    };
    if text.len() > limits.max_expanded_bytes {
        return Err(FormatError::TooLarge("decoded XML bytes".into()));
    }
    // XML normalizes literal line endings before parsing; character references such as
    // &#13; must still decode to a carriage return afterwards.
    let text = if text.contains('\r') { text.replace("\r\n", "\n").replace('\r', "\n") } else { text };
    let toks = tokenize(&text, max_tokens)?;
    let mut x = Xml { toks, pos: 0, nodes: 0, limits, bytes_left: limits.max_expanded_bytes, options };
    x.skip_space();
    let wrapped = matches!(x.peek(), Some(Tok::Open(n)) if n == "plist");
    if wrapped {
        x.pos += 1;
    }
    let value = x.value(0)?;
    x.skip_space();
    if wrapped {
        if !matches!(x.next(), Some(Tok::Close(n)) if n == "plist") {
            return Err(FormatError::Malformed("expected </plist> after its value".into()));
        }
        x.skip_space();
    }
    if x.peek().is_some() {
        return Err(FormatError::Malformed("unexpected content after the plist value".into()));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    #[test]
    fn explicit_token_budget_is_independent_of_value_budget() {
        let limits = Limits { max_nodes: 100, max_objects: 100, ..Limits::default() };
        let xml = b"<string><![CDATA[a]]><![CDATA[b]]></string>";
        assert!(matches!(parse_xml_with_token_limit(xml, limits, XmlOptions::default(), 3), Err(FormatError::TooLarge(_))));
        assert_eq!(parse_xml_with_token_limit(xml, limits, XmlOptions::default(), 4).unwrap(), Value::String("ab".into()));
        assert!(matches!(parse_xml_with_token_limit(b"<true/>", limits, XmlOptions::default(), 0), Err(FormatError::TooLarge(_))));
    }

    #[test]
    fn xml_has_one_complete_root_with_only_markup_space_around_values() {
        for invalid in [
            "<plist><true/>",
            "<plist><true/></dict>",
            "<plist><true/><false/></plist>",
            "<plist><true/></plist><dict/>",
            "<false/>junk",
            "text<true/>",
            "<array>junk<true/></array>",
            "<dict><key>x</key>garbage<true/></dict>",
            "<true>no</true>",
            "<false>yes</false>",
            "<array><key/></array>",
        ] {
            assert!(parse_xml(invalid.as_bytes()).is_err(), "{invalid}");
        }
        let xml = b"<?example content > <false/> ?>\n<plist><dict><key/><string>  </string></dict></plist><!-- end -->";
        assert_eq!(parse_xml(xml).unwrap().get("").and_then(Value::as_str), Some("  "));
        assert_eq!(parse_xml(b" <true></true> ").unwrap(), Value::Bool(true));
    }

    #[test]
    fn xml_rejects_partial_code_units_and_preserves_the_utf8_bom() {
        assert_eq!(parse_xml(b"\xef\xbb\xbf<string>ok</string>").unwrap().as_str(), Some("ok"));
        assert!(parse_xml(b"<string>\xff</string>").is_err());
        for be in [false, true] {
            let mut bytes = if be { vec![0xfe, 0xff] } else { vec![0xff, 0xfe] };
            for unit in "<true/>".encode_utf16() {
                bytes.extend(if be { unit.to_be_bytes() } else { unit.to_le_bytes() });
            }
            bytes.push(0);
            assert!(parse_xml(&bytes).is_err());
        }
    }

    #[test]
    fn a_doctype_internal_subset_is_skipped_as_one_declaration() {
        let doc = br#"<!DOCTYPE plist [<!ELEMENT plist (string)><!ENTITY text "x]>y"><!-- ]> --><?note ]> ?>]><plist><string>ok</string></plist>"#;
        assert_eq!(parse_xml(doc).unwrap().as_str(), Some("ok"));
        assert!(parse_xml(b"<!DOCTYPE plist [<!ELEMENT plist ANY><plist><true/></plist>").is_err());
    }

    #[test]
    fn xml_char_refs_require_plain_digits() {
        let text = |s: &str| {
            let doc = format!("<plist version=\"1.0\"><string>{s}</string></plist>");
            parse_xml(doc.as_bytes()).unwrap().as_str().map(str::to_string)
        };
        assert_eq!(text("a&#+65;b&#x+41;").as_deref(), Some("a&#+65;b&#x+41;"));
        assert_eq!(text("&#65;&#x41;").as_deref(), Some("AA"));
        assert_eq!(text("&#;&#x;").as_deref(), Some("&#;&#x;"));
    }

    #[test]
    fn xml_real_must_be_finite() {
        for bad in ["NaN", "1e999"] {
            let doc = format!("<dict><key>k</key><real>{bad}</real></dict>");
            assert!(parse_xml(doc.as_bytes()).is_err(), "{bad}");
        }
        for (text, want) in [("1.5", 1.5), ("-2", -2.0), ("0", 0.0)] {
            let doc = format!("<dict><key>k</key><real>{text}</real></dict>");
            let v = parse_xml(doc.as_bytes()).unwrap_or_else(|e| panic!("{text} should parse: {e}"));
            assert_eq!(v.get("k"), Some(&Value::Real(want)), "{text}");
        }
    }

    #[test]
    fn malformed_entity_scan_is_bounded_and_preserves_text() {
        for n in [4096, 65536, 262144] {
            let text = "&".repeat(n);
            assert_eq!(decode_entities(&text), text);
            assert_eq!(decode_entities(&(text.clone() + ";")), text + ";");
        }
        assert_eq!(decode_entities("é&missing; &amp; &#13; &#x41;"), "é&missing; & \r A");
        assert_eq!(decode_entities("C&#0000000097;non &#x0000041;"), "Canon A");
    }
    #[test]
    fn caller_limits_refuse_before_unbounded_tree_growth() {
        let limits = Limits { max_input_bytes: 3, ..Limits::default() };
        assert!(matches!(parse_xml_with_limits(b"<true/>", limits), Err(FormatError::TooLarge(_))));
        let limits = Limits { max_nodes: 1, ..Limits::default() };
        assert!(matches!(parse_xml_with_limits(b"<array><true/></array>", limits), Err(FormatError::TooLarge(_))));
        let limits = Limits { max_depth: 0, ..Limits::default() };
        assert!(matches!(parse_xml_with_limits(b"<array><true/></array>", limits), Err(FormatError::TooLarge(_))));
        let limits = Limits { max_expanded_bytes: 8, ..Limits::default() };
        assert!(matches!(parse_xml_with_limits(b"<string>abc</string>", limits), Err(FormatError::TooLarge(_))));
        assert!(parse_xml_with_limits(b"<true/>", Limits::default()).is_ok());
    }
    #[test]
    fn data_preserves_existing_whitespace_padding_and_alphabet_rules() {
        for (s, expected) in [(" Z g == ", Some(vec![102])), ("Zg", Some(vec![102])), ("Zg=", None), ("Zg==junk", None), ("_w==", None)] {
            let xml = format!("<data>{s}</data>");
            assert_eq!(parse_xml(xml.as_bytes()).ok(), expected.map(Value::Data));
        }
    }
    #[test]
    fn small_node_allowance_accepts_one_dictionary_entry() {
        let limits = Limits { max_nodes: 2, max_objects: 2, ..Limits::default() };
        assert!(parse_xml_with_limits(b"<dict><key>k</key><string>v</string></dict>", limits).is_ok());
    }
    #[test]
    fn callers_can_preserve_opaque_date_whitespace() {
        let xml = b"<date> 2026-01-01T00:00:00Z </date>";
        assert_eq!(parse_xml(xml).unwrap(), Value::Date(DateValue::Text("2026-01-01T00:00:00Z".into())));
        assert_eq!(
            parse_xml_with_options(xml, Limits::default(), XmlOptions { trim_date_text: false }).unwrap(),
            Value::Date(DateValue::Text(" 2026-01-01T00:00:00Z ".into()))
        );
    }
}
