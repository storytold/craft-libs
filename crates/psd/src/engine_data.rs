//! Photoshop's `EngineData`: the PostScript-like text engine document stored in `TySh`.
//!
//! Syntax (from observation of real files and public notes): dictionaries `<< /Key value … >>`,
//! arrays `[ … ]`, strings `( … )` holding UTF-16BE with a `FE FF` byte-order mark and `\`
//! escapes, numbers (`12`, `-1.5`, `.5`), `true`/`false`, and `/Name` values.
//!
//! [`parse`] and [`write`] keep the document as a tree of [`Value`]s; what was parsed is written
//! back in Photoshop's layout, and strings stored as plain bytes stay plain bytes.
//! [`first_style`] is a typed view of the first style run, for readers that show a type layer
//! without editing it; [`parse_lossy`] reads what it can of a damaged document for them.
//! Nesting is limited to 64 levels.

use std::fmt::Write as _;

/// One EngineData value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// `<< /Key value … >>`, in file order.
    Dict(Vec<(String, Value)>),
    /// `[ … ]`.
    Array(Vec<Value>),
    /// A string stored as UTF-16BE after the `FE FF` mark.
    String(String),
    /// A string the file stored as plain bytes, without the `FE FF` mark (one char per byte).
    /// Kept apart from [`Value::String`] so it is written back as the same bytes: Photoshop
    /// rejects a `Txt2` whose raw strings come back as UTF-16 (photocraft#9091).
    RawString(String),
    /// A number written without a fraction or exponent.
    Int(i64),
    /// A number written with a fraction or exponent.
    Real(f64),
    /// `true` or `false`.
    Bool(bool),
    /// `/Name` in value position.
    Name(String),
}

impl Value {
    /// The entry `key` of a dictionary.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Dict(items) => items.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    /// Item `i` of an array.
    pub fn index(&self, i: usize) -> Option<&Value> {
        self.as_array()?.get(i)
    }
    /// Follows a path of dictionary keys.
    pub fn path(&self, keys: &[&str]) -> Option<&Value> {
        keys.iter().try_fold(self, |v, k| v.get(k))
    }
    /// The entry `key` of a dictionary, mutably.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        match self {
            Value::Dict(items) => items.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    /// Sets (or appends) a dictionary entry.
    pub fn set(&mut self, key: &str, v: Value) {
        if let Value::Dict(items) = self {
            match items.iter_mut().find(|(k, _)| k == key) {
                Some(e) => e.1 = v,
                None => items.push((key.to_string(), v)),
            }
        }
    }
    /// A finite number.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            // `inf`, `nan` and `1e999` parse as reals but are never a valid style number.
            Value::Real(r) if r.is_finite() => Some(*r),
            _ => None,
        }
    }
    /// A number as an integer (a real is truncated).
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Real(r) => Some(*r as i64),
            _ => None,
        }
    }
    /// A boolean; an integer counts as `true` when it is not zero.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            Value::Int(i) => Some(*i != 0),
            _ => None,
        }
    }
    /// The text of a string or a name.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) | Value::RawString(s) | Value::Name(s) => Some(s),
            _ => None,
        }
    }
    /// The items of an array.
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }
    /// An empty dictionary.
    pub fn dict() -> Value {
        Value::Dict(Vec::new())
    }
}

/// Why an EngineData document did not parse: a message with the byte offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EngineData: {}", self.0)
    }
}
impl std::error::Error for ParseError {}

const MAX_DEPTH: usize = 64;

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
    /// Best effort: damage ends the value being read instead of failing the document.
    lenient: bool,
}

/// Bytes that end a bare token or a `/Name`.
fn is_delimiter(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n' | b'[' | b']' | b'<' | b'>' | b'(' | b'/')
}

impl Parser<'_> {
    fn ws(&mut self) {
        while matches!(self.b.get(self.i), Some(b' ' | b'\t' | b'\r' | b'\n' | 0)) {
            self.i += 1;
        }
    }
    fn err<T>(&self, m: &str) -> Result<T, ParseError> {
        Err(ParseError(format!("{m} at byte {}", self.i)))
    }
    fn value(&mut self, depth: usize) -> Result<Value, ParseError> {
        if depth > MAX_DEPTH {
            return self.err("nesting too deep");
        }
        self.ws();
        match self.b.get(self.i) {
            None => self.err("unexpected end"),
            Some(b'<') if self.b.get(self.i + 1) == Some(&b'<') => {
                self.i += 2;
                self.dict(depth)
            }
            Some(b'[') => {
                self.i += 1;
                self.array(depth)
            }
            Some(b'(') => {
                self.i += 1;
                self.string()
            }
            Some(b'/') => Ok(Value::Name(self.name())),
            Some(_) => self.token(),
        }
    }
    /// The entries after `<<`.
    fn dict(&mut self, depth: usize) -> Result<Value, ParseError> {
        let mut items = Vec::new();
        loop {
            self.ws();
            match self.b.get(self.i) {
                Some(b'>') if self.b.get(self.i + 1) == Some(&b'>') => {
                    self.i += 2;
                    return Ok(Value::Dict(items));
                }
                Some(b'/') => {
                    let k = self.name();
                    match self.value(depth + 1) {
                        Ok(v) => items.push((k, v)),
                        Err(_) if self.lenient => return Ok(Value::Dict(items)),
                        Err(e) => return Err(e),
                    }
                }
                None if self.lenient => return Ok(Value::Dict(items)),
                None => return self.err("unterminated dictionary"),
                // Lenient: skip the byte that is not a key.
                _ if self.lenient => self.i += 1,
                _ => return self.err("expected key"),
            }
        }
    }
    /// The items after `[`.
    fn array(&mut self, depth: usize) -> Result<Value, ParseError> {
        let mut items = Vec::new();
        loop {
            self.ws();
            match self.b.get(self.i) {
                Some(b']') => {
                    self.i += 1;
                    return Ok(Value::Array(items));
                }
                None if self.lenient => return Ok(Value::Array(items)),
                None => return self.err("unterminated array"),
                _ => match self.value(depth + 1) {
                    Ok(v) => items.push(v),
                    Err(_) if self.lenient => return Ok(Value::Array(items)),
                    Err(e) => return Err(e),
                },
            }
        }
    }
    /// The string after `(`.
    fn string(&mut self) -> Result<Value, ParseError> {
        let mut raw = Vec::new();
        loop {
            match self.b.get(self.i) {
                None if self.lenient => break,
                None => return self.err("unterminated string"),
                Some(b'\\') => {
                    if let Some(&c) = self.b.get(self.i + 1) {
                        raw.push(c);
                    }
                    self.i += 2;
                }
                Some(b')') => {
                    self.i += 1;
                    break;
                }
                Some(&c) => {
                    raw.push(c);
                    self.i += 1;
                }
            }
        }
        Ok(decode_string(&raw))
    }
    /// A number, `true` or `false`.
    fn token(&mut self) -> Result<Value, ParseError> {
        let start = self.i;
        while self.b.get(self.i).is_some_and(|&c| !is_delimiter(c)) {
            self.i += 1;
        }
        let tok = self.b.get(start..self.i).and_then(|t| std::str::from_utf8(t).ok()).unwrap_or("");
        let number = match tok {
            "true" => return Ok(Value::Bool(true)),
            "false" => return Ok(Value::Bool(false)),
            "" => {
                // A delimiter that starts no value (`>`, `]`), or bytes that are not UTF-8.
                self.i = self.i.max(start + 1);
                None
            }
            t if t.contains(['.', 'e', 'E']) => t.parse().ok().map(Value::Real),
            t => t.parse().map(Value::Int).ok().or_else(|| t.parse().ok().map(Value::Real)),
        };
        match number {
            Some(v) => Ok(v),
            None if self.lenient => Ok(Value::Name(tok.to_string())),
            None => self.err("bad token"),
        }
    }
    fn name(&mut self) -> String {
        self.i += 1; // '/'
        let start = self.i;
        while self.b.get(self.i).is_some_and(|&c| !is_delimiter(c)) {
            self.i += 1;
        }
        String::from_utf8_lossy(self.b.get(start..self.i).unwrap_or_default()).into_owned()
    }
}

fn decode_string(raw: &[u8]) -> Value {
    if raw.len() >= 2 && raw[0] == 0xFE && raw[1] == 0xFF {
        let u: Vec<u16> = raw[2..].as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        Value::String(String::from_utf16_lossy(&u))
    } else {
        Value::RawString(raw.iter().map(|&c| c as char).collect())
    }
}

/// Parses an EngineData document (the top-level dictionary).
pub fn parse(data: &[u8]) -> Result<Value, ParseError> {
    let mut p = Parser { b: data, i: 0, lenient: false };
    p.value(0)
}

/// Parses as much of a damaged EngineData document as can be read, for readers that only show
/// the text: a truncated dictionary, array or string keeps what it has, a byte that starts no
/// key is skipped, and a token that is no number becomes a [`Value::Name`]. Garbage gives an
/// empty dictionary. What [`parse`] accepts comes out the same. The result is not meant to be
/// written back.
pub fn parse_lossy(data: &[u8]) -> Value {
    let mut p = Parser { b: data, i: 0, lenient: true };
    p.value(0).unwrap_or_else(|_| Value::dict())
}

/// Serializes in Photoshop's layout (tab-indented, one key per line).
pub fn write(v: &Value) -> Vec<u8> {
    let mut out = b"\n\n".to_vec();
    write_value(v, 0, &mut out);
    out.push(b'\n');
    out
}

/// Serializes `/key value` pairs without an enclosing dictionary: the layout of the document's
/// `Txt2` block, which a reader wraps in `<< >>` to parse.
pub fn write_bare(items: &[(String, Value)]) -> Vec<u8> {
    let mut out = b"\n\n".to_vec();
    for (k, v) in items {
        out.push(b'/');
        out.extend_from_slice(k.as_bytes());
        if is_container(v) {
            out.push(b'\n');
            indent(&mut out, 1);
        } else {
            out.push(b' ');
        }
        write_value(v, 1, &mut out);
        out.push(b'\n');
    }
    out
}

fn indent(out: &mut Vec<u8>, n: usize) {
    out.extend(std::iter::repeat_n(b'\t', n));
}

fn is_container(v: &Value) -> bool {
    matches!(v, Value::Dict(_)) || matches!(v, Value::Array(a) if a.iter().any(|x| matches!(x, Value::Dict(_) | Value::Array(_))))
}

fn write_value(v: &Value, depth: usize, out: &mut Vec<u8>) {
    match v {
        Value::Dict(items) => {
            out.extend_from_slice(b"<<\n");
            for (k, v) in items {
                indent(out, depth + 1);
                out.push(b'/');
                out.extend_from_slice(k.as_bytes());
                if is_container(v) {
                    out.push(b'\n');
                    indent(out, depth + 1);
                } else {
                    out.push(b' ');
                }
                write_value(v, depth + 1, out);
                out.push(b'\n');
            }
            indent(out, depth);
            out.extend_from_slice(b">>");
        }
        Value::Array(items) if is_container(v) => {
            out.extend_from_slice(b"[\n");
            for x in items {
                indent(out, depth);
                write_value(x, depth, out);
                out.push(b'\n');
            }
            indent(out, depth);
            out.push(b']');
        }
        Value::Array(items) => {
            out.push(b'[');
            for x in items {
                out.push(b' ');
                write_value(x, depth, out);
            }
            out.extend_from_slice(b" ]");
        }
        Value::RawString(s) if s.chars().all(|c| u32::from(c) <= 0xFF) => {
            out.push(b'(');
            for c in s.chars() {
                let c = u32::from(c) as u8;
                if matches!(c, b'(' | b')' | b'\\') {
                    out.push(b'\\');
                }
                out.push(c);
            }
            out.push(b')');
        }
        Value::String(s) | Value::RawString(s) => {
            out.push(b'(');
            let mut raw = vec![0xFE, 0xFF];
            for u in s.encode_utf16() {
                raw.extend_from_slice(&u.to_be_bytes());
            }
            for c in raw {
                if matches!(c, b'(' | b')' | b'\\') {
                    out.push(b'\\');
                }
                out.push(c);
            }
            out.push(b')');
        }
        Value::Int(i) => {
            let _ = write!(Wrap(out), "{i}");
        }
        Value::Real(r) => out.extend_from_slice(format_real(*r).as_bytes()),
        Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Value::Name(n) => {
            out.push(b'/');
            out.extend_from_slice(n.as_bytes());
        }
    }
}

struct Wrap<'a>(&'a mut Vec<u8>);
impl std::fmt::Write for Wrap<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

/// Photoshop style: `1.0`, `.5`, `-.25`.
fn format_real(r: f64) -> String {
    if !r.is_finite() {
        return "0.0".into();
    }
    if r.fract() == 0.0 && r.abs() < 1e15 {
        return format!("{r:.1}");
    }
    let mut s = format!("{r:.5}");
    while s.ends_with('0') {
        s.pop();
    }
    if matches!(s.as_str(), "0." | "-0.") {
        return "0.0".into();
    }
    if s.ends_with('.') {
        s.push('0');
    }
    if let Some(rest) = s.strip_prefix("0.") {
        s = format!(".{rest}");
    } else if let Some(rest) = s.strip_prefix("-0.") {
        s = format!("-.{rest}");
    }
    s
}

/// The character and paragraph attributes of a type layer's first style run: what a reader that
/// shows the text in one style needs. Attributes the run leaves out come from the document's
/// first (normal) style sheet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextStyle {
    /// PostScript name of the font.
    pub font: Option<String>,
    /// Font size in points, at most 10 000.
    pub size: Option<f64>,
    /// Fill colour, RGB in 0..1.
    pub color: Option<[f64; 3]>,
    /// 0 left, 1 right, 2 centre, 3 and above the justified variants.
    pub justification: Option<u8>,
    /// Tracking in 1/1000 em, within ±100 000.
    pub tracking: Option<f64>,
    /// Leading in points, within ±100 000; `None` for automatic leading.
    pub leading: Option<f64>,
    /// Faux bold.
    pub faux_bold: bool,
    /// Faux italic.
    pub faux_italic: bool,
    /// Paragraph (box) text rather than point text.
    pub box_text: bool,
}

/// Upper bound for a font size in points (Photoshop's own limit is 1296 pt).
const MAX_FONT_SIZE: f64 = 10_000.0;
/// Bound for tracking (1/1000 em) and leading (points).
const MAX_SPACING: f64 = 100_000.0;

/// The first style run of a parsed EngineData document (see [`TextStyle`]). Missing and
/// non-finite values are `None`; sizes and spacings are clamped to sane bounds.
pub fn first_style(ed: &Value) -> TextStyle {
    let engine = ed.get("EngineDict").unwrap_or(ed);
    let resources = ed.get("ResourceDict").or_else(|| ed.get("DocumentResources"));
    let run = engine.path(&["StyleRun", "RunArray"]).and_then(|a| a.index(0)).and_then(|r| r.path(&["StyleSheet", "StyleSheetData"]));
    let normal = resources.and_then(|r| r.get("StyleSheetSet")).and_then(|a| a.index(0)).and_then(|s| s.get("StyleSheetData"));
    let attr = |k: &str| run.and_then(|r| r.get(k)).or_else(|| normal.and_then(|n| n.get(k)));
    let num = |k: &str| attr(k).and_then(Value::as_f64);
    let font = num("Font")
        .and_then(|i| resources?.get("FontSet")?.index(i as usize)?.get("Name"))
        .and_then(Value::as_str)
        .map(|s| s.trim_end_matches('\0').to_string());
    let color = attr("FillColor").and_then(|c| c.get("Values")).and_then(Value::as_array).and_then(|a| match a {
        [_, r, g, b, ..] => Some([r.as_f64()?, g.as_f64()?, b.as_f64()?]),
        _ => None,
    });
    let para = engine.path(&["ParagraphRun", "RunArray"]).and_then(|a| a.index(0)).and_then(|r| r.path(&["ParagraphSheet", "Properties"]));
    let para_normal = resources.and_then(|r| r.get("ParagraphSheetSet")).and_then(|a| a.index(0)).and_then(|s| s.get("Properties"));
    let justification = para
        .and_then(|p| p.get("Justification"))
        .or_else(|| para_normal.and_then(|p| p.get("Justification")))
        .and_then(Value::as_f64)
        .map(|n| n as u8);
    let auto_leading = attr("AutoLeading").is_none_or(|v| *v != Value::Bool(false));
    let shape = engine.path(&["Rendered", "Shapes", "Children"]).and_then(|a| a.index(0)).and_then(|s| s.get("ShapeType"));
    TextStyle {
        font,
        size: num("FontSize").filter(|s| *s > 0.0).map(|s| s.min(MAX_FONT_SIZE)),
        color,
        justification,
        tracking: num("Tracking").map(|t| t.clamp(-MAX_SPACING, MAX_SPACING)),
        leading: if auto_leading { None } else { num("Leading").map(|l| l.clamp(-MAX_SPACING, MAX_SPACING)) },
        faux_bold: attr("FauxBold") == Some(&Value::Bool(true)),
        faux_italic: attr("FauxItalic") == Some(&Value::Bool(true)),
        box_text: shape.and_then(Value::as_f64) == Some(1.0),
    }
}

/// A minimal EngineData document for a type layer with one style run: the text, one font, its
/// size in points, an RGB fill in 0..1 and the justification ([`TextStyle::justification`]).
pub fn minimal(text: &str, font: &str, size: f64, color: [f64; 3], justification: u8) -> Vec<u8> {
    let dict = |items: Vec<(&str, Value)>| Value::Dict(items.into_iter().map(|(k, v)| (k.to_string(), v)).collect());
    let run = |sheet: Value| dict(vec![("RunArray", Value::Array(vec![sheet]))]);
    let fill = Value::Array(vec![Value::Real(1.0), Value::Real(color[0]), Value::Real(color[1]), Value::Real(color[2])]);
    let style = dict(vec![
        ("Font", Value::Int(0)),
        ("FontSize", Value::Real(size)),
        ("FillColor", dict(vec![("Type", Value::Int(1)), ("Values", fill)])),
        ("AutoLeading", Value::Bool(true)),
    ]);
    let paragraph = dict(vec![("Properties", dict(vec![("Justification", Value::Int(i64::from(justification)))]))]);
    let engine = dict(vec![
        ("Editor", dict(vec![("Text", Value::String(format!("{text}\r")))])),
        ("ParagraphRun", run(dict(vec![("ParagraphSheet", paragraph)]))),
        ("StyleRun", run(dict(vec![("StyleSheet", dict(vec![("StyleSheetData", style)]))]))),
    ]);
    let fonts = Value::Array(vec![dict(vec![("Name", Value::String(font.to_string())), ("Type", Value::Int(0))])]);
    write(&dict(vec![("EngineDict", engine), ("ResourceDict", dict(vec![("FontSet", fonts)]))]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = b"\n\n<<\n\t/EngineDict\n\t<<\n\t\t/Editor\n\t\t<<\n\t\t\t/Text (\xFE\xFF\x00H\x00i\x00\\)\x00\r)\n\t\t>>\n\t\t/Nums [ 1.0 .5 -2 0 ]\n\t\t/Flag true\n\t\t/Kind /Roman\n\t\t/Runs [\n\t\t<<\n\t\t\t/A 1\n\t\t>>\n\t\t]\n\t>>\n>>";

    #[test]
    fn format_real_never_ends_with_bare_dot() {
        assert_eq!(format_real(5.000001), "5.0");
        assert_eq!(format_real(12.000004), "12.0");
        assert_eq!(format_real(-5.000001), "-5.0");
        assert_eq!(format_real(1e20), "100000000000000000000.0");
        assert_eq!(format_real(0.5), ".5");
        assert_eq!(format_real(-0.25), "-.25");
        assert_eq!(format_real(1e-9), "0.0");
        assert_eq!(format_real(3.0), "3.0");
    }

    #[test]
    fn parses_sample() {
        let v = parse(SAMPLE).unwrap();
        assert_eq!(v.path(&["EngineDict", "Editor", "Text"]).and_then(Value::as_str), Some("Hi)\r"));
        let nums: Vec<f64> = v.path(&["EngineDict", "Nums"]).unwrap().as_array().unwrap().iter().filter_map(Value::as_f64).collect();
        assert_eq!(nums, [1.0, 0.5, -2.0, 0.0]);
        assert_eq!(v.path(&["EngineDict", "Flag"]).and_then(Value::as_bool), Some(true));
        assert_eq!(v.path(&["EngineDict", "Kind"]), Some(&Value::Name("Roman".into())));
        assert_eq!(v.path(&["EngineDict", "Runs"]).unwrap().as_array().unwrap()[0].get("A").and_then(Value::as_i64), Some(1));
    }

    #[test]
    fn raw_strings_are_written_back_as_the_same_bytes() {
        let v = parse(b"<< /2 (0100\\)) /3 (\xFE\xFF\x00h) >>").unwrap();
        assert_eq!(v.get("2"), Some(&Value::RawString("0100)".into())));
        assert_eq!(v.get("2").and_then(Value::as_str), Some("0100)"));
        assert_eq!(v.get("3"), Some(&Value::String("h".into())));
        let bytes = write(&v);
        assert!(bytes.windows(8).any(|w| w == b"(0100\\))"), "{:?}", String::from_utf8_lossy(&bytes));
        assert_eq!(parse(&bytes).unwrap(), v);
    }

    #[test]
    fn write_parse_roundtrip() {
        let v = parse(SAMPLE).unwrap();
        let bytes = write(&v);
        assert_eq!(parse(&bytes).unwrap(), v);
        // Stable: writing twice gives identical bytes.
        assert_eq!(write(&parse(&bytes).unwrap()), bytes);
    }

    #[test]
    fn reals_format_like_photoshop() {
        assert_eq!(format_real(1.0), "1.0");
        assert_eq!(format_real(0.5), ".5");
        assert_eq!(format_real(-0.25), "-.25");
        assert_eq!(format_real(12.125), "12.125");
    }

    #[test]
    fn tiny_reals_round_trip_as_numbers() {
        for value in [0.000004, -0.000004] {
            assert_eq!(format_real(value), "0.0");
            let source = Value::Dict(vec![(
                "EngineDict".into(),
                Value::Dict(vec![("StyleRun".into(), Value::Array(vec![Value::Dict(vec![("BaselineShift".into(), Value::Real(value))])]))]),
            )]);
            let encoded = write(&source);
            let parsed = parse(&encoded).unwrap();
            let baseline_shift = parsed
                .path(&["EngineDict", "StyleRun"])
                .and_then(Value::as_array)
                .and_then(|runs| runs.first())
                .and_then(|run| run.get("BaselineShift"))
                .and_then(Value::as_f64);
            assert_eq!(baseline_shift, Some(0.0));
        }
    }

    #[test]
    fn first_style_reads_a_minimal_document() {
        let b = minimal("Hi (there)", "Arial-BoldMT", 36.0, [1.0, 0.5, 0.0], 2);
        let v = parse(&b).unwrap();
        assert_eq!(v.path(&["EngineDict", "Editor", "Text"]).and_then(Value::as_str), Some("Hi (there)\r"));
        let st = first_style(&v);
        assert_eq!(st.font.as_deref(), Some("Arial-BoldMT"));
        assert_eq!(st.size, Some(36.0));
        assert_eq!(st.color, Some([1.0, 0.5, 0.0]));
        assert_eq!(st.justification, Some(2));
        assert_eq!(st.leading, None);
        assert!(!st.box_text && !st.faux_bold && !st.faux_italic);
    }

    #[test]
    fn first_style_falls_back_to_the_normal_style_sheet_and_reads_box_text() {
        let v = parse(
            b"<< /EngineDict << /StyleRun << /RunArray [ << /StyleSheet << /StyleSheetData << /FauxBold true >> >> >> ] >> \
              /Rendered << /Shapes << /Children [ << /ShapeType 1 >> ] >> >> >> \
              /ResourceDict << /FontSet [ << /Name (A) >> << /Name (B) >> ] \
              /StyleSheetSet [ << /StyleSheetData << /Font 1 /FontSize 12.0 /Tracking 50 /AutoLeading false /Leading 14.5 >> >> ] \
              /ParagraphSheetSet [ << /Properties << /Justification 1 >> >> ] >> >>",
        )
        .unwrap();
        let st = first_style(&v);
        assert_eq!(st.font.as_deref(), Some("B"));
        assert_eq!((st.size, st.tracking, st.leading), (Some(12.0), Some(50.0), Some(14.5)));
        assert_eq!(st.justification, Some(1));
        assert!(st.faux_bold && !st.faux_italic && st.box_text);
    }

    /// Sizes and spacings are untrusted: non-finite values are dropped and huge ones clamped.
    #[test]
    fn first_style_drops_non_finite_numbers_and_clamps_the_rest() {
        let sheet = |attrs: &str| {
            let doc = format!("<< /EngineDict << /StyleRun << /RunArray [ << /StyleSheet << /StyleSheetData << {attrs} >> >> >> ] >> >> >>");
            first_style(&parse(doc.as_bytes()).unwrap())
        };
        let st = sheet("/FontSize nan /Tracking inf /Leading -inf /AutoLeading false");
        assert_eq!((st.size, st.tracking, st.leading), (None, None, None));
        let st = sheet("/FontSize 1e300 /Tracking -1e300 /AutoLeading false /Leading 1e300");
        assert_eq!((st.size, st.tracking, st.leading), (Some(MAX_FONT_SIZE), Some(-MAX_SPACING), Some(MAX_SPACING)));
        assert_eq!(sheet("/FontSize -4").size, None);
        assert_eq!(first_style(&Value::dict()), TextStyle::default());
    }

    #[test]
    fn nesting_deeper_than_the_limit_is_an_error() {
        let nested = |n: usize| [vec![b'['; n], vec![b']'; n]].concat();
        assert!(parse(&nested(MAX_DEPTH)).is_ok());
        assert!(parse(&nested(MAX_DEPTH + 2)).is_err());
    }

    #[test]
    fn parse_lossy_keeps_what_a_damaged_document_holds() {
        // What the strict parser accepts comes out the same.
        assert_eq!(parse_lossy(SAMPLE), parse(SAMPLE).unwrap());
        // Truncated inside a string inside an array inside a dictionary.
        let v = parse_lossy(b"<< /A [ 1 2 << /B (unterminated");
        assert_eq!(v.get("A").and_then(|a| a.index(1)).and_then(Value::as_i64), Some(2));
        assert_eq!(v.get("A").and_then(|a| a.index(2)).and_then(|d| d.get("B")).and_then(Value::as_str), Some("unterminated"));
        // A byte that starts no key is skipped; a bad token is a name, not a number.
        let v = parse_lossy(b"<< ] /Size 12.0 /Bad 1.2.3 /After true >>");
        assert_eq!(v.get("Size").and_then(Value::as_f64), Some(12.0));
        assert_eq!(v.get("Bad"), Some(&Value::Name("1.2.3".into())));
        assert_eq!(v.get("After"), Some(&Value::Bool(true)));
        // Garbage and every truncation: no panic, no hang.
        assert_eq!(parse_lossy(b""), Value::dict());
        let _ = parse_lossy(b"]]]>>>> /x");
        let _ = parse_lossy(&[0xFF; 64]);
        for cut in 0..SAMPLE.len() {
            let _ = parse_lossy(&SAMPLE[..cut]);
        }
        // Nesting past the limit ends there; the outer levels stay.
        let deep = [b"<< /K ".as_slice(), &[b'['; 200]].concat();
        assert!(matches!(parse_lossy(&deep).get("K"), Some(Value::Array(_))));
    }

    #[test]
    fn malformed_input_never_panics() {
        for cut in 0..SAMPLE.len() {
            let _ = parse(&SAMPLE[..cut]);
        }
        assert!(parse(b"<< /A").is_err());
        assert!(parse(&[b'['; 200]).is_err());
    }
}
