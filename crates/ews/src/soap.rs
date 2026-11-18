//! The SOAP envelope of a request, and the elements it carries.
//!
//! Everything written here is escaped: attribute values pass through [`escape`] and
//! lose every control character (CR, LF and NUL among them); text passes through
//! [`escape_text`], which keeps its tabs and line feeds (a body of text keeps its
//! lines), writes a carriage return as a character reference and drops NUL and the other
//! controls XML forbids; and a name keeps only the characters a qualified name is made of.
//! Nothing a caller hands in can therefore end an element, open another, or break a line of the
//! protocol.

/// The SOAP 1.1 envelope's namespace.
pub const SOAP: &str = "http://schemas.xmlsoap.org/soap/envelope/";
/// The namespace of the types ([MS-OXWSCDATA]).
pub const TYPES: &str = "http://schemas.microsoft.com/exchange/services/2006/types";
/// The namespace of the messages (the operations and their answers).
pub const MESSAGES: &str = "http://schemas.microsoft.com/exchange/services/2006/messages";
/// The namespace of the errors a fault carries.
pub const ERRORS: &str = "http://schemas.microsoft.com/exchange/services/2006/errors";

/// The version of the schema a request asks the server to answer in (`RequestServerVersion`,
/// [MS-OXWSCDATA] `ExchangeVersionType`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Version {
    /// Exchange 2007.
    Exchange2007,
    /// Exchange 2007 with service pack 1.
    Exchange2007Sp1,
    /// Exchange 2010.
    Exchange2010,
    /// Exchange 2010 with service pack 1.
    Exchange2010Sp1,
    /// Exchange 2010 with service pack 2.
    Exchange2010Sp2,
    /// Exchange 2013: what a request asks for unless told otherwise, as every server that
    /// speaks the service today understands it.
    #[default]
    Exchange2013,
    /// Exchange 2013 with service pack 1.
    Exchange2013Sp1,
    /// Exchange 2015.
    Exchange2015,
    /// Exchange 2016.
    Exchange2016,
}

impl Version {
    /// Every name the schema has for a version.
    pub const NAMES: [&'static str; 9] = [
        "Exchange2007",
        "Exchange2007_SP1",
        "Exchange2010",
        "Exchange2010_SP1",
        "Exchange2010_SP2",
        "Exchange2013",
        "Exchange2013_SP1",
        "Exchange2015",
        "Exchange2016",
    ];

    /// The name the schema gives it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exchange2007 => "Exchange2007",
            Self::Exchange2007Sp1 => "Exchange2007_SP1",
            Self::Exchange2010 => "Exchange2010",
            Self::Exchange2010Sp1 => "Exchange2010_SP1",
            Self::Exchange2010Sp2 => "Exchange2010_SP2",
            Self::Exchange2013 => "Exchange2013",
            Self::Exchange2013Sp1 => "Exchange2013_SP1",
            Self::Exchange2015 => "Exchange2015",
            Self::Exchange2016 => "Exchange2016",
        }
    }
}

/// A name as a qualified name may be written: letters, digits, `_`, `-`, `.` and one `:`
/// between the prefix and the local name; anything else is dropped.
fn clean_name(name: &str) -> String {
    let mut colon = false;
    name.chars()
        .filter(|c| match c {
            ':' if !colon => {
                colon = true;
                true
            }
            c => c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'),
        })
        .collect()
}

/// One element of a request: a name (with its prefix, `m:GetFolder`), attributes, and text and
/// children.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Element {
    name: String,
    attributes: Vec<(String, String)>,
    text: String,
    children: Vec<Element>,
}

impl std::fmt::Debug for Element {
    /// The name and how much is inside: the text and the attributes may be somebody's mail.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Element")
            .field("name", &self.name)
            .field("attributes", &self.attributes.len())
            .field("children", &self.children.len())
            .finish()
    }
}

impl Element {
    /// An element called `name`.
    pub fn new(name: &str) -> Self {
        Self { name: clean_name(name), ..Self::default() }
    }

    /// With the attribute `name` set to `value`.
    #[must_use]
    pub fn attr(mut self, name: &str, value: &str) -> Self {
        self.attributes.push((clean_name(name), escape(value)));
        self
    }

    /// With `text` inside it.
    #[must_use]
    pub fn text(mut self, text: &str) -> Self {
        self.text = escape_text(text);
        self
    }

    /// With `child` inside it, after the ones before.
    #[must_use]
    pub fn child(mut self, child: Element) -> Self {
        self.children.push(child);
        self
    }

    /// The element as text.
    pub fn write(&self) -> String {
        let mut out = String::new();
        self.write_into(&mut out);
        out
    }

    fn write_into(&self, out: &mut String) {
        out.push('<');
        out.push_str(&self.name);
        for (name, value) in &self.attributes {
            out.push(' ');
            out.push_str(name);
            out.push_str("=\"");
            out.push_str(value);
            out.push('"');
        }
        if self.text.is_empty() && self.children.is_empty() {
            out.push_str("/>");
            return;
        }
        out.push('>');
        out.push_str(&self.text);
        for child in &self.children {
            child.write_into(out);
        }
        out.push_str("</");
        out.push_str(&self.name);
        out.push('>');
    }
}

/// The whole request: a SOAP envelope whose header names `version` and whose body holds
/// `operation` (an element of the messages' namespace, `m:GetFolder`).
pub fn envelope(version: Version, operation: &Element) -> String {
    let header = Element::new("t:RequestServerVersion").attr("Version", version.as_str());
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><s:Envelope xmlns:s=\"{SOAP}\" xmlns:m=\"{MESSAGES}\" xmlns:t=\"{TYPES}\">\
         <s:Header>{}</s:Header><s:Body>{}</s:Body></s:Envelope>",
        header.write(),
        operation.write()
    )
}

/// `text` as it is written inside an attribute (or anywhere a line of a protocol could end): the
/// five characters XML has names for by those names, and nothing that could end a line or a
/// header: every control character (CR, LF, NUL and the tab too), and the characters XML 1.0
/// forbids (U+FFFE, U+FFFF).
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() || c == '\u{FFFE}' || c == '\u{FFFF}' => {}
            c => out.push(c),
        }
    }
    out
}

/// `text` as it is written inside an element: as [`escape`], except that the tab and the line
/// feed stay (a body of text keeps its lines) and a carriage return is written as `&#13;`, which
/// is not one on the wire. NUL and the other C0 controls, U+FFFE and U+FFFF are dropped.
/// (A `&str` holds no unpaired surrogate.)
pub fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            c if allowed(c) => out.push(c),
            _ => {}
        }
    }
    out
}

/// Whether XML 1.0 can carry `c` in a text.
fn allowed(c: char) -> bool {
    !((c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r')) && c != '\u{FFFE}' && c != '\u{FFFF}'
}
