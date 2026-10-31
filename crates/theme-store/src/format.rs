//! Theme file format v1.

use std::{fmt, sync::Arc};

use craft_theme::{
    DefinitionError, IdError, MissingRole, Rgba, Role, RoleMap, SuiteTheme, ThemeId, Variant,
    contrast::{self, Diagnostic},
};
use serde_json::{Map, Value};

/// The `schema` value of a theme file.
pub const THEME_SCHEMA: &str = "craft-theme";
/// The theme file format this crate reads and writes.
pub const THEME_SCHEMA_VERSION: u64 = 1;
/// Larger theme files are refused without being parsed.
pub const MAX_THEME_BYTES: usize = 256 * 1024;

/// A parsed theme file and the bytes it came from.
#[derive(Clone, Debug)]
pub struct ThemeRecord {
    pub theme: SuiteTheme,
    /// Roles the file left out or gave an unreadable colour, filled from Graphite.
    pub missing: Vec<MissingRole>,
    /// Advisory contrast findings; they never block an import or a selection.
    pub diagnostics: Vec<Diagnostic>,
    /// The file exactly as stored, unknown keys included.
    pub bytes: Arc<[u8]>,
}

impl ThemeRecord {
    /// What a theme picker shows as the theme's warnings: filled roles plus contrast findings.
    pub fn diagnostic_count(&self) -> usize {
        self.missing.len() + self.diagnostics.len()
    }
}

/// Why a theme file was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormatError {
    TooLarge {
        len: usize,
    },
    /// Not JSON; the parser's message.
    Json(String),
    NotObject,
    /// `schema` is missing or not `"craft-theme"`.
    WrongSchema,
    /// `schema_version` is missing or not a positive integer.
    BadSchemaVersion,
    /// Written by a newer format than this build reads.
    TooNew {
        schema_version: u64,
    },
    MissingId,
    /// A known key with a value of the wrong type.
    BadField(&'static str),
    Definition(DefinitionError),
    /// A library file whose `id` isn't its file name (`<id>.json`).
    FileName {
        id: String,
    },
}

/// Parses a theme file. Unknown keys are allowed anywhere and kept in [`ThemeRecord::bytes`];
/// a role colour that doesn't parse is treated as missing.
pub fn parse_theme(bytes: &[u8]) -> Result<ThemeRecord, FormatError> {
    if bytes.len() > MAX_THEME_BYTES {
        return Err(FormatError::TooLarge { len: bytes.len() });
    }
    let value: Value = serde_json::from_slice(strip_bom(bytes)).map_err(|e| FormatError::Json(e.to_string()))?;
    let obj = value.as_object().ok_or(FormatError::NotObject)?;
    check_schema(obj)?;
    let id = match obj.get("id") {
        None => return Err(FormatError::MissingId),
        Some(Value::String(s)) => s.as_str(),
        Some(_) => return Err(FormatError::BadField("id")),
    };
    let name = match obj.get("name") {
        None | Some(Value::Null) => "",
        Some(Value::String(s)) => s.as_str(),
        Some(_) => return Err(FormatError::BadField("name")),
    };
    let version = match obj.get("version") {
        None | Some(Value::Null) => 1,
        Some(v) => v.as_u64().and_then(|v| u32::try_from(v).ok()).ok_or(FormatError::BadField("version"))?,
    };
    let light = role_map(obj, "light")?;
    let dark = role_map(obj, "dark")?;
    let (theme, missing) = SuiteTheme::from_maps(id, name, version, light.as_ref(), dark.as_ref()).map_err(FormatError::Definition)?;
    let diagnostics = contrast::diagnose(&theme);
    Ok(ThemeRecord { theme, missing, diagnostics, bytes: Arc::from(bytes) })
}

fn check_schema(obj: &Map<String, Value>) -> Result<(), FormatError> {
    if obj.get("schema").and_then(Value::as_str) != Some(THEME_SCHEMA) {
        return Err(FormatError::WrongSchema);
    }
    match obj.get("schema_version").and_then(Value::as_u64) {
        Some(THEME_SCHEMA_VERSION) => Ok(()),
        Some(v) if v > THEME_SCHEMA_VERSION => Err(FormatError::TooNew { schema_version: v }),
        _ => Err(FormatError::BadSchemaVersion),
    }
}

/// The known roles of one variant. Unknown keys are ignored here; a colour that doesn't parse
/// is left out so that `from_maps` fills and reports it.
fn role_map(obj: &Map<String, Value>, key: &'static str) -> Result<Option<RoleMap>, FormatError> {
    let variant = match obj.get(key) {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Object(v)) => v,
        Some(_) => return Err(FormatError::BadField(key)),
    };
    let map = Role::ALL
        .iter()
        .filter_map(|role| {
            let colour = variant.get(role.key())?.as_str().and_then(Rgba::parse_hex)?;
            Some((role.key().to_owned(), colour))
        })
        .collect();
    Ok(Some(map))
}

fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)
}

/// A theme as a v1 file, roles in [`Role::ALL`] order. Used to export built-ins.
pub fn theme_to_json(theme: &SuiteTheme) -> String {
    let mut out = String::from("{\n");
    out.push_str(&format!("  \"schema\": \"{THEME_SCHEMA}\",\n  \"schema_version\": {THEME_SCHEMA_VERSION},\n"));
    out.push_str(&format!("  \"id\": {},\n", json_string(theme.id.as_str())));
    out.push_str(&format!("  \"name\": {},\n", json_string(&theme.name)));
    out.push_str(&format!("  \"version\": {}", theme.version));
    for (variant, roles) in theme.variants() {
        let key = match variant {
            Variant::Light => "light",
            Variant::Dark => "dark",
        };
        out.push_str(&format!(",\n  \"{key}\": {{\n"));
        let lines: Vec<String> = Role::ALL.iter().map(|r| format!("    \"{}\": \"{}\"", r.key(), roles.get(*r).to_hex())).collect();
        out.push_str(&lines.join(",\n"));
        out.push_str("\n  }");
    }
    out.push_str("\n}\n");
    out
}

fn json_string(s: &str) -> String {
    Value::String(s.to_owned()).to_string()
}

/// The id an "import as copy" gives a theme whose id is taken: `<id>-<n>`, shortening `<id>` so
/// the result keeps the id length limit. The result is checked against the id rules again.
pub(crate) fn copy_id(id: &ThemeId, n: u32) -> Result<ThemeId, IdError> {
    const MAX_ID: usize = 64;
    let suffix = format!("-{n}");
    let keep = MAX_ID.saturating_sub(suffix.len()).min(id.as_str().len());
    // Ids are ASCII, so any byte index is a char boundary.
    let stem = id.as_str().get(..keep).unwrap_or_default();
    ThemeId::parse_user(&format!("{stem}{suffix}"))
}

/// `bytes` with every top-level `"id"` member's value replaced by `id`, everything else byte for
/// byte. `bytes` must be a JSON object that `parse_theme` accepted.
pub(crate) fn replace_top_level_id(bytes: &[u8], id: &ThemeId) -> Option<Vec<u8>> {
    let mut scan = Scanner { bytes, pos: bytes.len() - strip_bom(bytes).len() };
    let mut spans = Vec::new();
    scan.ws();
    scan.expect(b'{')?;
    scan.ws();
    if scan.peek()? != b'}' {
        loop {
            let key_start = scan.pos;
            scan.string()?;
            let key: String = serde_json::from_slice(bytes.get(key_start..scan.pos)?).ok()?;
            scan.ws();
            scan.expect(b':')?;
            scan.ws();
            let value_start = scan.pos;
            scan.value()?;
            if key == "id" {
                spans.push((value_start, scan.pos));
            }
            scan.ws();
            match scan.next()? {
                b',' => scan.ws(),
                b'}' => break,
                _ => return None,
            }
        }
    }
    let replacement = json_string(id.as_str());
    let mut out = Vec::with_capacity(bytes.len() + replacement.len());
    let mut at = 0;
    for (start, end) in spans {
        out.extend_from_slice(bytes.get(at..start)?);
        out.extend_from_slice(replacement.as_bytes());
        at = end;
    }
    out.extend_from_slice(bytes.get(at..)?);
    Some(out)
}

/// Just enough of a JSON scanner to find value spans in text serde_json has already accepted.
struct Scanner<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Scanner<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    fn expect(&mut self, b: u8) -> Option<()> {
        (self.next()? == b).then_some(())
    }

    fn ws(&mut self) {
        while self.peek().is_some_and(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn string(&mut self) -> Option<()> {
        self.expect(b'"')?;
        loop {
            match self.next()? {
                b'\\' => {
                    self.next()?;
                }
                b'"' => return Some(()),
                _ => {}
            }
        }
    }

    fn value(&mut self) -> Option<()> {
        match self.peek()? {
            b'"' => self.string(),
            b'{' | b'[' => self.container(),
            _ => {
                while self.peek().is_some_and(|b| !matches!(b, b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r')) {
                    self.pos += 1;
                }
                Some(())
            }
        }
    }

    /// Skips an object or array; strings inside may contain brackets.
    fn container(&mut self) -> Option<()> {
        let mut depth = 0usize;
        loop {
            match self.peek()? {
                b'"' => self.string()?,
                b'{' | b'[' => {
                    depth += 1;
                    self.pos += 1;
                }
                b'}' | b']' => {
                    depth = depth.saturating_sub(1);
                    self.pos += 1;
                    if depth == 0 {
                        return Some(());
                    }
                }
                _ => self.pos += 1,
            }
        }
    }
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatError::TooLarge { len } => {
                write!(f, "a theme file is at most {} KiB; this one is {} KiB", MAX_THEME_BYTES / 1024, len.div_ceil(1024))
            }
            FormatError::Json(e) => write!(f, "not a JSON file: {e}"),
            FormatError::NotObject => f.write_str("a theme file is a JSON object"),
            FormatError::WrongSchema => write!(f, "not a theme file: \"schema\" is not {THEME_SCHEMA:?}"),
            FormatError::BadSchemaVersion => f.write_str("\"schema_version\" is missing or not a positive whole number"),
            FormatError::TooNew { schema_version } => {
                write!(f, "theme file version {schema_version} is newer than this app reads ({THEME_SCHEMA_VERSION}); update the app")
            }
            FormatError::MissingId => f.write_str("the theme file has no \"id\""),
            FormatError::BadField(key) => write!(f, "\"{key}\" has a value of the wrong type"),
            FormatError::Definition(e) => e.fmt(f),
            FormatError::FileName { id } => write!(f, "the theme's id {id:?} doesn't match its file name; it should be {id}.json"),
        }
    }
}

impl std::error::Error for FormatError {}

impl From<IdError> for FormatError {
    fn from(e: IdError) -> Self {
        FormatError::Definition(DefinitionError::Id(e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreadable_colours_become_missing_roles() {
        let bytes = br##"{"schema":"craft-theme","schema_version":1,"id":"odd","light":{"text":"#12","surface":7,"line":"#123456"}}"##;
        let record = parse_theme(bytes).unwrap();
        assert_eq!(record.missing.len(), 12);
        assert!(record.missing.iter().any(|m| m.role == Role::Text));
        assert_eq!(record.theme.light.unwrap().line, Rgba::from_rgb_u32(0x123456));
        assert_eq!(record.diagnostic_count(), record.missing.len() + record.diagnostics.len());
    }

    #[test]
    fn refusals() {
        let refuse = |s: &str| parse_theme(s.as_bytes()).unwrap_err();
        assert_eq!(refuse("[]"), FormatError::NotObject);
        assert!(matches!(refuse("{"), FormatError::Json(_)));
        assert_eq!(refuse(r#"{"schema":"other","schema_version":1,"id":"ab","light":{}}"#), FormatError::WrongSchema);
        assert_eq!(refuse(r#"{"schema":"craft-theme","schema_version":2,"id":"ab","light":{}}"#), FormatError::TooNew { schema_version: 2 });
        assert_eq!(refuse(r#"{"schema":"craft-theme","schema_version":1,"light":{}}"#), FormatError::MissingId);
        assert_eq!(refuse(r#"{"schema":"craft-theme","schema_version":1,"id":"ab","light":[]}"#), FormatError::BadField("light"));
        assert_eq!(refuse(r#"{"schema":"craft-theme","schema_version":1,"id":"ab"}"#), FormatError::Definition(DefinitionError::NoVariant));
        assert_eq!(refuse(r#"{"schema":"craft-theme","schema_version":1,"id":"craft.x","dark":{}}"#), FormatError::from(IdError::Reserved));
        let big = format!(r#"{{"pad":"{}"}}"#, "x".repeat(MAX_THEME_BYTES));
        assert!(matches!(refuse(&big), FormatError::TooLarge { .. }));
    }

    #[test]
    fn replacing_the_id_keeps_every_other_byte() {
        let src =
            "\u{feff}{ \"note\": \"\\\"id\\\": {x}\", \"id\" : \"mine\",\n \"nested\": {\"id\": \"inner\", \"a\": [1, {\"id\": 2}]}, \"n\": -1.5e3 }";
        let id = ThemeId::parse_user("mine-2").unwrap();
        let out = String::from_utf8(replace_top_level_id(src.as_bytes(), &id).unwrap()).unwrap();
        assert_eq!(out, src.replace("\"id\" : \"mine\"", "\"id\" : \"mine-2\""));
    }

    #[test]
    fn copy_ids_keep_the_length_limit() {
        let short = ThemeId::parse_user("navy").unwrap();
        assert_eq!(copy_id(&short, 2).unwrap().as_str(), "navy-2");
        let long = ThemeId::parse_user(&"a".repeat(64)).unwrap();
        let copy = copy_id(&long, 12).unwrap();
        assert_eq!(copy.as_str().len(), 64);
        assert!(copy.as_str().ends_with("a-12"));
    }

    #[test]
    fn builtins_export_and_parse_back() {
        for builtin in craft_theme::builtins() {
            let json = theme_to_json(builtin);
            // Built-in ids are reserved, so a re-import needs a user id.
            let renamed = replace_top_level_id(json.as_bytes(), &ThemeId::parse_user("copy").unwrap()).unwrap();
            let record = parse_theme(&renamed).unwrap();
            assert!(record.missing.is_empty());
            assert_eq!((record.theme.light, record.theme.dark), (builtin.light, builtin.dark));
            assert_eq!(record.theme.name, builtin.name);
        }
    }
}
