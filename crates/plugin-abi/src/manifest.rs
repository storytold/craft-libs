//! The `craft.manifest` custom section: types plus the checks that
//! `schemas/plugin-manifest.schema.json` states. Unknown fields survive a parse and a re-serialize
//! so a newer manifest loses nothing when an older host rewrites it.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::permissions::{Permission, PermissionError};

/// Manifest size cap in bytes (`ABI.md` section 1).
pub const MAX_MANIFEST_BYTES: usize = 256 * 1024;
pub const MAX_CONTRIBUTIONS: usize = 256;
pub const MAX_PARAMS: usize = 1024;
/// Prefixes owned by the legacy shims, the suite and the host. Third parties may not use them.
/// `app.` and `plugin.` keep a plugin command id (`plugin.<id>.<contribution>`) out of the host's
/// own `plugin.app.<app>.<verb>` management commands. Compared without regard to ASCII case.
pub const RESERVED_ID_PREFIXES: [&str; 6] = ["ec.", "pc.", "vc.", "craft.", "app.", "plugin."];
/// Ids that are reserved as a whole, because `plugin.app.<contribution>` would sit in the host namespace.
pub const RESERVED_IDS: [&str; 2] = ["app", "plugin"];

/// The reserved prefix or id that `id` falls under, if any. ASCII case is ignored, so `App.x` and
/// `APP` are refused like `app.x`.
pub fn reserved_id_match(id: &str) -> Option<&'static str> {
    let folded = id.to_ascii_lowercase();
    RESERVED_ID_PREFIXES.iter().chain(RESERVED_IDS.iter()).copied().find(|r| if r.ends_with('.') { folded.starts_with(r) } else { folded == *r })
}

type Extra = BTreeMap<String, Value>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    pub abi: Abi,
    /// Supported apps and semver ranges. `None` means any app.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apps: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<Requires>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Permissions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<Limits>,
    pub contributions: Vec<Contribution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy: Option<LegacyTag>,
    /// Reserved. Must be absent or null in v1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Value>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Abi {
    pub craft: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<Codec>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    Json,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Requires {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<Feature>,
    /// Op names. The gate checks them against this host's op table.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ops: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub engine_caps: Vec<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Feature {
    #[serde(rename = "simd128")]
    Simd128,
}

/// Permission strings as written. `Manifest::validate` parses each one.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Permissions {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub optional: Vec<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Limits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack_kb: Option<u32>,
    #[serde(flatten)]
    pub extra: Extra,
}

/// What a contribution is. A kind this release does not know is kept as [`Kind::Unknown`] with its
/// exact name, so a newer manifest loads, the old host skips the entry it cannot run, and a
/// re-serialize writes the name back unchanged (shown as "partly supported").
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Command,
    Hook,
    Effect,
    Filter,
    AudioEffect,
    Format,
    Panel,
    Menu,
    ObjectFilter,
    LiveEffect,
    /// A kind from a newer manifest. `Manifest::validate` checks the name against [`is_kind_name`].
    Unknown(String),
}

impl Kind {
    /// Kinds this release knows, in schema order.
    pub const KNOWN: [Kind; 10] = [
        Kind::Command,
        Kind::Hook,
        Kind::Effect,
        Kind::Filter,
        Kind::AudioEffect,
        Kind::Format,
        Kind::Panel,
        Kind::Menu,
        Kind::ObjectFilter,
        Kind::LiveEffect,
    ];

    /// The manifest spelling.
    pub fn name(&self) -> &str {
        match self {
            Kind::Command => "command",
            Kind::Hook => "hook",
            Kind::Effect => "effect",
            Kind::Filter => "filter",
            Kind::AudioEffect => "audio-effect",
            Kind::Format => "format",
            Kind::Panel => "panel",
            Kind::Menu => "menu",
            Kind::ObjectFilter => "object-filter",
            Kind::LiveEffect => "live-effect",
            Kind::Unknown(name) => name,
        }
    }

    /// Parse a manifest name. A name outside the known set becomes `Kind::Unknown`.
    pub fn from_name(name: &str) -> Kind {
        Kind::KNOWN.into_iter().find(|k| k.name() == name).unwrap_or_else(|| Kind::Unknown(name.to_owned()))
    }

    pub fn is_known(&self) -> bool {
        !matches!(self, Kind::Unknown(_))
    }

    /// Data-plane kinds run through `craft_process`. Known control-plane kinds run through
    /// `craft_invoke`. An unknown kind runs through neither, so this is false for it.
    pub fn is_data_plane(&self) -> bool {
        matches!(self, Kind::Effect | Kind::Filter | Kind::AudioEffect | Kind::ObjectFilter | Kind::LiveEffect)
    }

    /// True when the kind may declare `"budget": "batch"` (`ABI.md` section 8). Kinds that run off the
    /// UI thread may: `format`, `filter`, `object-filter`. A `command` may only when its plugin
    /// declares a `link.call:<app>` permission (`asks_link`), because a link call blocks on another
    /// process. Hooks, panels, menus, the real-time effects and unknown kinds never may.
    pub fn may_run_batch(&self, asks_link: bool) -> bool {
        match self {
            Kind::Format | Kind::Filter | Kind::ObjectFilter => true,
            Kind::Command => asks_link,
            _ => false,
        }
    }

    /// True for a known kind that runs through `craft_invoke`.
    pub fn is_control_plane(&self) -> bool {
        self.is_known() && !self.is_data_plane()
    }
}

/// An unknown kind name must look like a known one: lowercase words joined by `-`, at most 32 characters.
pub fn is_kind_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 32
        && matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

impl Serialize for Kind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for Kind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Ok(Kind::from_name(&name))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Instance {
    Fresh,
    Session,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Budget {
    Interactive,
    Render,
    Batch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Enabled {
    Always,
    HasDocument,
    HasSelection,
    Cached,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    Before,
    After,
    Change,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Dock {
    Left,
    Right,
    Bottom,
    Floating,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SampleType {
    U8,
    U16,
    F16,
    F32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AlphaMode {
    None,
    Straight,
    Premultiplied,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Contribution {
    pub kind: Kind,
    pub id: String,
    /// Index passed to `craft_invoke` and `craft_process`.
    pub entry: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Absent means `fresh`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<Instance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<Budget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<String>,
    /// Absent means `has-document`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<Enabled>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<Phase>,
    #[serde(default, rename = "match", skip_serializing_if = "Option::is_none")]
    pub match_globs: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dock: Option<Dock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<Process>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<BTreeMap<String, ParamSpec>>,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Contribution {
    pub fn instance(&self) -> Instance {
        self.instance.unwrap_or(Instance::Fresh)
    }

    pub fn enabled(&self) -> Enabled {
        self.enabled.unwrap_or(Enabled::HasDocument)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Process {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlap: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub band_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample: Option<Vec<SampleType>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha: Option<AlphaMode>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamType {
    Number,
    Int,
    Bool,
    Choice,
    Text,
    Color,
    Angle,
    Point2,
    Point3,
    Layer,
    File,
    Json,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParamSpec {
    #[serde(rename = "type")]
    pub ty: ParamType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegacyTag {
    #[serde(rename = "effectcraft-v1")]
    EffectcraftV1,
    #[serde(rename = "photocraft-v1")]
    PhotocraftV1,
    #[serde(rename = "vectorcraft-v1")]
    VectorcraftV1,
}

/// A manifest that failed to parse or to validate.
#[derive(Clone, Debug, PartialEq)]
pub enum ManifestError {
    TooLarge {
        bytes: usize,
    },
    /// Not UTF-8 JSON, wrong shape, or an enum value outside the schema.
    Json(String),
    Invalid {
        path: String,
        reason: String,
    },
    Permission {
        path: String,
        error: PermissionError,
    },
}

impl ManifestError {
    fn invalid(path: impl Into<String>, reason: impl Into<String>) -> Self {
        ManifestError::Invalid { path: path.into(), reason: reason.into() }
    }
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestError::TooLarge { bytes } => write!(f, "manifest is {bytes} bytes, cap is {MAX_MANIFEST_BYTES}"),
            ManifestError::Json(m) => write!(f, "manifest JSON: {m}"),
            ManifestError::Invalid { path, reason } => write!(f, "manifest {path}: {reason}"),
            ManifestError::Permission { path, error } => write!(f, "manifest {path}: {error}"),
        }
    }
}

impl std::error::Error for ManifestError {}

fn is_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

impl Manifest {
    /// Parse and validate the bytes of a `craft.manifest` section.
    pub fn parse(bytes: &[u8]) -> Result<Manifest, ManifestError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(ManifestError::TooLarge { bytes: bytes.len() });
        }
        let manifest: Manifest = serde_json::from_slice(bytes).map_err(|e| ManifestError::Json(e.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Check every rule of the schema that serde types cannot state.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.schema != 1 {
            return Err(ManifestError::invalid("schema", format!("must be 1, got {}", self.schema)));
        }
        if self.id.is_empty() || self.id.chars().count() > 64 || !self.id.chars().all(is_id_char) {
            return Err(ManifestError::invalid("id", "must match [A-Za-z0-9._-]{1,64}"));
        }
        if let Some(r) = reserved_id_match(&self.id) {
            return Err(ManifestError::invalid("id", format!("{r:?} is reserved for the host and the suite")));
        }
        // Text the host shows must not hide or forge anything.
        check_text("name", &self.name, false)?;
        check_text("version", &self.version, false)?;
        if let Some(d) = &self.description {
            check_text("description", d, true)?;
        }
        for (field, value) in [("author", &self.author), ("license", &self.license), ("homepage", &self.homepage)] {
            if let Some(v) = value {
                check_text(field, v, false)?;
            }
        }
        if self.name.chars().count() > 80 {
            return Err(ManifestError::invalid("name", "longer than 80 characters"));
        }
        if self.description.as_ref().is_some_and(|d| d.chars().count() > 2000) {
            return Err(ManifestError::invalid("description", "longer than 2000 characters"));
        }
        if self.abi.craft != 1 {
            return Err(ManifestError::invalid("abi.craft", format!("must be 1, got {}", self.abi.craft)));
        }
        if self.signature.is_some() {
            return Err(ManifestError::invalid("signature", "reserved, must be null"));
        }
        if let Some(l) = &self.limits {
            if l.memory_mb.is_some_and(|m| !(1..=4096).contains(&m)) {
                return Err(ManifestError::invalid("limits.memory_mb", "must be 1 to 4096"));
            }
            if l.stack_kb.is_some_and(|s| !(64..=8192).contains(&s)) {
                return Err(ManifestError::invalid("limits.stack_kb", "must be 64 to 8192"));
            }
        }
        if let Some(p) = &self.permissions {
            for (list, group) in [(&p.required, "required"), (&p.optional, "optional")] {
                for (i, s) in list.iter().enumerate() {
                    s.parse::<Permission>().map_err(|error| ManifestError::Permission { path: format!("permissions.{group}[{i}]"), error })?;
                }
            }
        }
        if self.contributions.is_empty() || self.contributions.len() > MAX_CONTRIBUTIONS {
            return Err(ManifestError::invalid("contributions", format!("must hold 1 to {MAX_CONTRIBUTIONS} items")));
        }
        let asks_link = self.permissions.as_ref().is_some_and(|p| p.required.iter().chain(&p.optional).any(|s| s.starts_with("link.call:")));
        for (i, c) in self.contributions.iter().enumerate() {
            let at = format!("contributions[{i}]");
            c.validate(&at)?;
            if c.budget == Some(Budget::Batch) && !c.kind.may_run_batch(asks_link) {
                return Err(ManifestError::invalid(
                    format!("{at}.budget"),
                    format!(
                        "\"batch\" is not allowed on kind {:?} here: it runs on the UI thread (a command needs a link.call permission)",
                        c.kind.name()
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Contributions whose kind this release does not know. The host loads the rest and marks the
    /// plugin "partly supported" when this is non-empty.
    pub fn unsupported_contributions(&self) -> impl Iterator<Item = &Contribution> {
        self.contributions.iter().filter(|c| !c.kind.is_known())
    }

    /// Every permission the manifest asks for, required first. Call after `validate`.
    pub fn parsed_permissions(&self) -> Result<(Vec<Permission>, Vec<Permission>), ManifestError> {
        let parse_all = |list: &[String], group: &str| -> Result<Vec<Permission>, ManifestError> {
            list.iter()
                .enumerate()
                .map(|(i, s)| s.parse().map_err(|error| ManifestError::Permission { path: format!("permissions.{group}[{i}]"), error }))
                .collect()
        };
        match &self.permissions {
            Some(p) => Ok((parse_all(&p.required, "required")?, parse_all(&p.optional, "optional")?)),
            None => Ok((Vec::new(), Vec::new())),
        }
    }
}

/// A string the host shows to the user holds no control, bidi or zero-width character.
/// `multiline` keeps newline and tab.
fn check_text(at: impl Into<String>, s: &str, multiline: bool) -> Result<(), ManifestError> {
    let bad = if multiline { crate::text::has_unsafe_multiline(s) } else { crate::text::has_unsafe(s) };
    if bad {
        return Err(ManifestError::invalid(at, "holds a control, bidi or zero-width character"));
    }
    Ok(())
}

impl Contribution {
    fn validate(&self, at: &str) -> Result<(), ManifestError> {
        for (field, value) in [("label", &self.label), ("category", &self.category), ("shortcut", &self.shortcut), ("title", &self.title)] {
            if let Some(v) = value {
                check_text(format!("{at}.{field}"), v, false)?;
            }
        }
        for entry in self.menu.iter().flatten() {
            check_text(format!("{at}.menu"), entry, false)?;
        }
        if self.id.is_empty() || self.id.chars().count() > 64 || !self.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err(ManifestError::invalid(format!("{at}.id"), "must match [A-Za-z0-9_-]{1,64}"));
        }
        if let Kind::Unknown(name) = &self.kind
            && !is_kind_name(name)
        {
            return Err(ManifestError::invalid(format!("{at}.kind"), "an unknown kind must match [a-z][a-z0-9-]{0,31}"));
        }
        if self.kind == Kind::Hook {
            let Some(phase) = self.phase else {
                return Err(ManifestError::invalid(format!("{at}.phase"), "a hook needs a phase"));
            };
            if phase == Phase::Before && self.match_globs.is_none() {
                return Err(ManifestError::invalid(format!("{at}.match"), "a before hook needs match globs"));
            }
        }
        if self.kind == Kind::Format {
            let ok = |e: &String| (1..=16).contains(&e.len()) && e.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
            match &self.extensions {
                Some(list) if !list.is_empty() && list.iter().all(ok) => {}
                _ => {
                    return Err(ManifestError::invalid(
                        format!("{at}.extensions"),
                        "a format needs extensions: 1 to 16 lower case letters or digits each, no dot",
                    ));
                }
            }
        }
        if let Some(p) = &self.process {
            if p.overlap.is_some_and(|o| o > 256) {
                return Err(ManifestError::invalid(format!("{at}.process.overlap"), "must be 0 to 256"));
            }
            if p.band_bytes.is_some_and(|b| b < 65536) {
                return Err(ManifestError::invalid(format!("{at}.process.band_bytes"), "must be at least 65536"));
            }
        }
        if let Some(params) = &self.params {
            if params.len() > MAX_PARAMS {
                return Err(ManifestError::invalid(format!("{at}.params"), format!("more than {MAX_PARAMS} params")));
            }
            for (name, spec) in params {
                for (field, value) in [("label", &spec.label), ("group", &spec.group)] {
                    if let Some(v) = value {
                        check_text(format!("{at}.params.{name}.{field}"), v, false)?;
                    }
                }
                for option in spec.options.iter().flatten() {
                    check_text(format!("{at}.params.{name}.options"), option, false)?;
                }
            }
            for name in params.keys() {
                let mut chars = name.chars();
                let ok = matches!(chars.next(), Some(c) if c.is_ascii_alphanumeric()) && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
                if !ok {
                    return Err(ManifestError::invalid(format!("{at}.params.{name}"), "name must match [A-Za-z0-9][A-Za-z0-9_]*"));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The same file the schema test validates, so the two cannot drift.
    fn good() -> Value {
        serde_json::from_str(include_str!("../tests/fixtures/manifest_good.json")).unwrap()
    }

    fn parse(v: &Value) -> Result<Manifest, ManifestError> {
        Manifest::parse(&serde_json::to_vec(v).unwrap())
    }

    #[test]
    fn hostile_text_in_the_manifest_is_refused() {
        for (path, value) in [
            ("name", json!("Gain\u{202E}gpj")),
            ("author", json!("Acme\u{200B} Tools")),
            ("version", json!("1.0\n2.0")),
            ("description", json!("fine\u{1b}[2J")),
        ] {
            let mut m = good();
            m[path] = value;
            assert!(parse(&m).is_err(), "{path}");
        }
        let mut m = good();
        m["contributions"][0]["label"] = json!("Free\u{2066}Money");
        assert!(parse(&m).is_err());
        let mut m = good();
        m["contributions"][0]["menu"] = json!(["Plug-ins\nSave"]);
        assert!(parse(&m).is_err());
        // Multi-line descriptions stay allowed.
        let mut m = good();
        m["description"] = json!("two\nlines");
        assert!(parse(&m).is_ok());
    }

    #[test]
    fn batch_is_refused_on_ui_thread_kinds() {
        let with = |kind: &str, perms: Option<Value>| {
            let mut m = good();
            m["contributions"] = json!([{ "kind": kind, "id": "x", "entry": 0, "budget": "batch",
                "phase": "after", "extensions": ["abc"] }]);
            if let Some(p) = perms {
                m["permissions"] = p;
            }
            parse(&m)
        };
        for kind in ["hook", "panel", "menu", "effect", "audio-effect", "live-effect"] {
            let err = with(kind, None).expect_err(kind).to_string();
            assert!(err.contains("batch"), "{kind}: {err}");
        }
        // Off the UI thread, or a link call: allowed.
        for kind in ["format", "filter", "object-filter"] {
            assert!(with(kind, None).is_ok(), "{kind}: {:?}", with(kind, None));
        }
        assert!(with("command", None).expect_err("command").to_string().contains("batch"), "a command needs link.call");
        assert!(with("command", Some(json!({ "required": ["link.call:filmcraft"] }))).is_ok());
        assert!(with("command", Some(json!({ "optional": ["link.call:filmcraft"] }))).is_ok());
    }

    #[test]
    fn good_example_deserializes_with_every_field() {
        let m = parse(&good()).unwrap();
        assert_eq!(m.id, "org.example.posterize");
        assert_eq!(m.abi, Abi { craft: 1, codec: Some(Codec::Json), extra: Extra::new() });
        assert_eq!(m.contributions.len(), 5);
        assert_eq!(m.contributions[0].kind, Kind::Effect);
        assert!(m.contributions[0].kind.is_data_plane());
        assert_eq!(m.unsupported_contributions().count(), 0);
        assert_eq!(m.contributions[0].params.as_ref().unwrap()["bands"].ty, ParamType::Int);
        assert_eq!(m.contributions[1].instance(), Instance::Session);
        assert_eq!(m.contributions[3].match_globs.as_deref(), Some(&["effect.*".to_owned()][..]));
        assert_eq!(m.contributions[4].dock, Some(Dock::Right));
        assert_eq!(m.limits.as_ref().unwrap().memory_mb, Some(256));
        let (req, opt) = m.parsed_permissions().unwrap();
        assert_eq!(req, vec![Permission::CommandsRun(Some("effect.*".into()))]);
        assert_eq!(opt, vec![Permission::NetFetch("https://example.org".into())]);
    }

    #[test]
    fn defaults_apply_when_fields_are_absent() {
        let mut v = good();
        v["contributions"] = json!([{ "kind": "command", "id": "x", "entry": 0 }]);
        let m = parse(&v).unwrap();
        assert_eq!(m.contributions[0].instance(), Instance::Fresh);
        assert_eq!(m.contributions[0].enabled(), Enabled::HasDocument);
    }

    #[test]
    fn unknown_fields_are_kept_at_every_level_through_a_round_trip() {
        let mut v = good();
        v["future_top"] = json!({"a": [1, 2]});
        v["abi"]["future_abi"] = json!(true);
        v["requires"]["future_req"] = json!("x");
        v["permissions"]["future_perm"] = json!(1);
        v["limits"]["future_limit"] = json!(2);
        v["contributions"][1]["future_c"] = json!("kept");
        v["contributions"][0]["process"]["future_p"] = json!(null);
        v["contributions"][0]["params"]["bands"]["future_param"] = json!([true]);
        let m = parse(&v).unwrap();
        assert_eq!(m.extra["future_top"], json!({"a": [1, 2]}));
        assert_eq!(m.contributions[1].extra["future_c"], json!("kept"));
        let again: Value = serde_json::to_value(&m).unwrap();
        for ptr in [
            "/future_top",
            "/abi/future_abi",
            "/requires/future_req",
            "/permissions/future_perm",
            "/limits/future_limit",
            "/contributions/1/future_c",
            "/contributions/0/process/future_p",
            "/contributions/0/process/area",
            "/contributions/0/params/bands/future_param",
        ] {
            assert_eq!(again.pointer(ptr), v.pointer(ptr), "{ptr} lost");
        }
    }

    #[test]
    fn reserved_id_prefixes_are_rejected() {
        for id in [
            "ec.blur",
            "pc.blur",
            "vc.blur",
            "craft.blur",
            "craft.",
            "app.filmcraft",
            "app.",
            "app",
            "plugin.x",
            "plugin",
            "App.filmcraft",
            "APP",
            "Plugin.X",
            "EC.blur",
            "Craft.blur",
        ] {
            let mut v = good();
            v["id"] = json!(id);
            let err = parse(&v).unwrap_err();
            assert!(matches!(&err, ManifestError::Invalid { path, .. } if path == "id"), "{id}: {err:?}");
        }
        // Near misses stay legal: the prefix test needs the dot.
        for id in ["ec", "ecx.blur", "crafty.x", "my.ec.blur", "apps.x", "application", "my.app.x", "plugins.x", "app-x", "app_x.y"] {
            let mut v = good();
            v["id"] = json!(id);
            parse(&v).unwrap_or_else(|e| panic!("{id} should pass: {e}"));
        }
    }

    #[test]
    fn bad_ids_are_rejected() {
        for id in json!(["", "has space", "a/b", "x".repeat(65), "caf\u{e9}", "\u{ff41}pp.x", "app\u{200b}.x"]).as_array().unwrap() {
            let mut v = good();
            v["id"] = id.clone();
            assert!(parse(&v).is_err(), "{id}");
        }
        let mut v = good();
        v["id"] = json!("x".repeat(64));
        parse(&v).unwrap();
    }

    #[test]
    fn unknown_permission_names_are_rejected_with_their_location() {
        for (group, idx, p) in [("required", 0, "commands.exec"), ("optional", 0, "net.fetch:http://example.org"), ("required", 1, "doc.read:all")] {
            let mut v = good();
            let list = v["permissions"][group].as_array_mut().unwrap();
            if idx == 0 {
                list[0] = json!(p);
            } else {
                list.push(json!(p));
            }
            match parse(&v).unwrap_err() {
                ManifestError::Permission { path, error } => {
                    assert_eq!(path, format!("permissions.{group}[{idx}]"));
                    assert_eq!(error.input, p);
                }
                other => panic!("{p}: {other:?}"),
            }
        }
    }

    #[test]
    fn before_hook_without_match_is_rejected_but_after_hook_without_match_is_fine() {
        let mut v = good();
        v["contributions"] = json!([{ "kind": "hook", "id": "h", "entry": 0, "phase": "before" }]);
        let err = parse(&v).unwrap_err();
        assert!(matches!(&err, ManifestError::Invalid { path, .. } if path == "contributions[0].match"), "{err:?}");
        v["contributions"] = json!([{ "kind": "hook", "id": "h", "entry": 0, "phase": "after" }]);
        parse(&v).unwrap();
        v["contributions"] = json!([{ "kind": "hook", "id": "h", "entry": 0, "phase": "before", "match": [] }]);
        parse(&v).unwrap();
    }

    #[test]
    fn a_format_needs_plain_lower_case_extensions() {
        let mut v = good();
        let at = |v: &Value| parse(v).unwrap_err();
        for ext in [json!(null), json!([]), json!([""]), json!(["Xyz"]), json!([".xyz"]), json!(["a b"]), json!(["x".repeat(17)])] {
            v["contributions"] = json!([{ "kind": "format", "id": "f", "entry": 0, "extensions": ext }]);
            if ext.is_null() {
                v["contributions"][0].as_object_mut().unwrap().remove("extensions");
            }
            let err = at(&v);
            assert!(matches!(&err, ManifestError::Invalid { path, .. } if path == "contributions[0].extensions"), "{ext}: {err:?}");
        }
        v["contributions"] = json!([{ "kind": "format", "id": "f", "entry": 0, "extensions": ["xyz", "x1"] }]);
        parse(&v).unwrap();
    }

    #[test]
    fn hook_without_phase_is_rejected_but_command_without_phase_is_fine() {
        let mut v = good();
        v["contributions"] = json!([{ "kind": "hook", "id": "h", "entry": 0, "match": ["*"] }]);
        let err = parse(&v).unwrap_err();
        assert!(matches!(&err, ManifestError::Invalid { path, .. } if path == "contributions[0].phase"), "{err:?}");
        v["contributions"] = json!([{ "kind": "command", "id": "h", "entry": 0 }]);
        parse(&v).unwrap();
    }

    #[test]
    fn schema_const_and_abi_const_and_signature_are_enforced() {
        for (ptr, val) in [("/schema", json!(2)), ("/abi/craft", json!(2)), ("/signature", json!({"alg": "ed25519"}))] {
            let mut v = good();
            *v.pointer_mut(ptr).unwrap() = val;
            assert!(parse(&v).is_err(), "{ptr}");
        }
        let mut v = good();
        v["abi"]["codec"] = json!("cbor");
        assert!(matches!(parse(&v), Err(ManifestError::Json(_))));
    }

    #[test]
    fn required_fields_and_enum_values_are_enforced() {
        for key in ["schema", "id", "name", "version", "abi", "contributions"] {
            let mut v = good();
            v.as_object_mut().unwrap().remove(key);
            assert!(parse(&v).is_err(), "missing {key}");
        }
        for key in ["kind", "id", "entry"] {
            let mut v = good();
            v["contributions"][1].as_object_mut().unwrap().remove(key);
            assert!(parse(&v).is_err(), "missing contribution {key}");
        }
        let mut v = good();
        v["requires"]["features"] = json!(["relaxed-simd"]);
        assert!(parse(&v).is_err());
        let mut v = good();
        v["legacy"] = json!("gridcraft-v1");
        assert!(parse(&v).is_err());
        let mut v = good();
        v["contributions"] = json!([]);
        assert!(parse(&v).is_err());
    }

    #[test]
    fn unknown_kind_loads_with_its_name_and_the_rest_of_the_manifest_intact() {
        let mut v = good();
        v["contributions"][1]["kind"] = json!("teleport");
        v["contributions"][1]["future_c"] = json!({"k": 1});
        let m = parse(&v).unwrap();
        assert_eq!(m.contributions[1].kind, Kind::Unknown("teleport".into()));
        assert!(!m.contributions[1].kind.is_known());
        assert!(!m.contributions[1].kind.is_data_plane());
        assert!(!m.contributions[1].kind.is_control_plane());
        assert_eq!(m.contributions[0].kind, Kind::Effect, "known entries are unchanged");
        let skipped: Vec<_> = m.unsupported_contributions().map(|c| (c.id.as_str(), c.entry)).collect();
        assert_eq!(skipped, vec![("nudge", 1)]);
        let again = serde_json::to_value(&m).unwrap();
        assert_eq!(again.pointer("/contributions/1/kind"), Some(&json!("teleport")), "the name survives a rewrite");
        assert_eq!(again.pointer("/contributions/1/future_c"), Some(&json!({"k": 1})));
    }

    #[test]
    fn an_unknown_kind_still_gets_every_common_check() {
        let bad: [(&str, Value); 8] = [
            ("kind", json!("Teleport")),
            ("kind", json!("")),
            ("kind", json!("a b")),
            ("kind", json!("x".repeat(33))),
            ("kind", json!("9lives")),
            ("id", json!("has.dot")),
            ("entry", json!(-1)),
            ("phase", json!("sideways")),
        ];
        for (key, val) in bad {
            let mut v = good();
            v["contributions"][1]["kind"] = json!("teleport");
            v["contributions"][1][key] = val.clone();
            assert!(parse(&v).is_err(), "{key} = {val}");
        }
        let mut v = good();
        v["contributions"][1]["kind"] = json!("x".repeat(32));
        parse(&v).unwrap();
        // The hook rules bind the hook kind only, never a lookalike.
        let mut v = good();
        v["contributions"] = json!([{ "kind": "hook2", "id": "h", "entry": 0 }, { "kind": "hook", "id": "g", "entry": 1 }]);
        assert!(parse(&v).is_err(), "a real hook with no phase still fails beside an unknown kind");
    }

    #[test]
    fn numeric_ranges_are_enforced() {
        let cases: [(&str, Value); 8] = [
            ("/limits/memory_mb", json!(0)),
            ("/limits/memory_mb", json!(4097)),
            ("/limits/stack_kb", json!(63)),
            ("/limits/stack_kb", json!(8193)),
            ("/contributions/0/process/overlap", json!(257)),
            ("/contributions/0/process/band_bytes", json!(65535)),
            ("/contributions/0/entry", json!(-1)),
            ("/contributions/0/params/bands/type", json!("matrix")),
        ];
        for (ptr, val) in cases {
            let mut v = good();
            *v.pointer_mut(ptr).unwrap() = val.clone();
            assert!(parse(&v).is_err(), "{ptr} = {val}");
        }
        for (ptr, val) in [("/limits/memory_mb", json!(4096)), ("/limits/stack_kb", json!(64)), ("/contributions/0/process/overlap", json!(256))] {
            let mut v = good();
            *v.pointer_mut(ptr).unwrap() = val.clone();
            parse(&v).unwrap_or_else(|e| panic!("{ptr} = {val}: {e}"));
        }
    }

    #[test]
    fn param_names_and_counts_are_enforced() {
        for name in ["_hidden", "has-dash", "", "a b"] {
            let mut v = good();
            v["contributions"][0]["params"] = json!({ name: { "type": "bool" } });
            assert!(parse(&v).is_err(), "{name:?}");
        }
        let many: serde_json::Map<String, Value> = (0..=MAX_PARAMS).map(|i| (format!("p{i}"), json!({"type": "bool"}))).collect();
        let mut v = good();
        v["contributions"][0]["params"] = Value::Object(many);
        assert!(parse(&v).is_err());
    }

    #[test]
    fn size_cap_is_enforced() {
        let mut v = good();
        v["description"] = json!("x".repeat(MAX_MANIFEST_BYTES));
        assert!(matches!(parse(&v), Err(ManifestError::TooLarge { .. })));
    }

    #[test]
    fn contribution_cap_is_enforced() {
        let mk = |n: usize| -> Value {
            let mut v = good();
            v["contributions"] = Value::Array((0..n).map(|i| json!({"kind": "command", "id": format!("c{i}"), "entry": i})).collect());
            v
        };
        parse(&mk(MAX_CONTRIBUTIONS)).unwrap();
        assert!(parse(&mk(MAX_CONTRIBUTIONS + 1)).is_err());
    }

    #[test]
    fn garbage_is_a_json_error_not_a_panic() {
        for bytes in [&b""[..], b"{", b"[]", b"null", &[0xFF, 0xFE]] {
            assert!(matches!(Manifest::parse(bytes), Err(ManifestError::Json(_))), "{bytes:?}");
        }
    }
}
