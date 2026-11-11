//! The checks that refuse settings a writer can't honour, and the warnings for options it
//! accepts but doesn't apply.
//!
//! Messages are plain English. Each carries a stable `id` an application can use to look up its
//! own wording. Checks that need colour profiles ask a [`ProfileCatalog`].

use std::fmt;

use super::{BleedSettings, ColorConversion, Compatibility, DOCUMENT_CMYK, MarkSettings, PdfSettings, Printing, SecuritySettings, Standard};

/// The colour model of an ICC profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileKind {
    #[allow(missing_docs)]
    Rgb,
    #[allow(missing_docs)]
    Cmyk,
    #[allow(missing_docs)]
    Gray,
}

/// The colour mode of the document being exported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMode {
    #[allow(missing_docs)]
    Rgb,
    #[allow(missing_docs)]
    Cmyk,
}

/// The colour profiles the application knows, for the checks on the Output section.
pub trait ProfileCatalog {
    /// The kind of the profile called `name`, if there is one.
    fn kind(&self, name: &str) -> Option<ProfileKind>;
    /// The name of the RGB profile in effect.
    fn rgb(&self) -> &str;
    /// The name of the CMYK profile in effect.
    fn cmyk(&self) -> &str;
}

/// A catalog without profiles: every profile name is unknown.
impl ProfileCatalog for () {
    fn kind(&self, _name: &str) -> Option<ProfileKind> {
        None
    }
    fn rgb(&self) -> &str {
        ""
    }
    fn cmyk(&self) -> &str {
        ""
    }
}

/// A setting the writer can't honour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsError {
    /// A stable identifier of the rule.
    pub id: &'static str,
    /// The English message.
    pub message: String,
    /// The values the message carries, by the name of their placeholder in an application's own
    /// wording: `standard`, `version`, `name`, `limit`, `field`, `range`, `value`.
    pub values: Vec<(&'static str, String)>,
}

impl SettingsError {
    /// The value of placeholder `key`.
    pub fn value(&self, key: &str) -> Option<&str> {
        self.values.iter().find(|(k, _)| *k == key).map(|(_, v)| v.as_str())
    }

    fn with(mut self, key: &'static str, value: impl Into<String>) -> Self {
        self.values.push((key, value.into()));
        self
    }
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid PDF setting: {}", self.message)
    }
}

impl std::error::Error for SettingsError {}

fn refuse(id: &'static str, message: impl Into<String>) -> SettingsError {
    SettingsError { id, message: message.into(), values: Vec::new() }
}

/// An option the writer accepts but doesn't apply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsWarning {
    /// A stable identifier of the rule.
    pub id: &'static str,
    /// The English message.
    pub message: String,
    /// The values the message carries, as in [`SettingsError::values`].
    pub values: Vec<(&'static str, String)>,
}

impl SettingsWarning {
    /// The value of placeholder `key`.
    pub fn value(&self, key: &str) -> Option<&str> {
        self.values.iter().find(|(k, _)| *k == key).map(|(_, v)| v.as_str())
    }

    fn with(mut self, key: &'static str, value: impl Into<String>) -> Self {
        self.values.push((key, value.into()));
        self
    }
}

fn warn(id: &'static str, message: impl Into<String>) -> SettingsWarning {
    SettingsWarning { id, message: message.into(), values: Vec::new() }
}

/// `v` lies in `lo..=hi` (and is a number).
fn within(id: &'static str, name: &str, v: f64, lo: f64, hi: f64, unit: &str) -> Result<(), SettingsError> {
    if (lo..=hi).contains(&v) {
        Ok(())
    } else {
        Err(refuse(id, format!("{name} must be {lo}–{hi}{unit} (got {v})"))
            .with("field", name)
            .with("range", format!("{lo}–{hi}{unit}"))
            .with("value", v.to_string()))
    }
}

/// The longest password (in bytes) revisions 2–4 use: longer ones are cut to this.
const MAX_LEGACY_PASSWORD: usize = 32;
/// The longest password (UTF-8 bytes) revision 6 uses.
const MAX_PASSWORD: usize = 127;

/// The standard security handler revision written for PDF version `c`: 2 (40-bit RC4), 3 (RC4,
/// metadata always encrypted), 4 (crypt filters: RC4 or AES-128) or 6 (AES-256).
fn revision(c: Compatibility) -> u8 {
    match c {
        Compatibility::Pdf13 => 2,
        Compatibility::Pdf14 => 3,
        Compatibility::Pdf15 | Compatibility::Pdf16 => 4,
        Compatibility::Pdf17 | Compatibility::Pdf20 => 6,
    }
}

impl PdfSettings {
    /// Refuse settings the writer can't honour: what [`Self::check_values`] refuses, a destination
    /// profile that isn't there (named, and in an RGB document; see [`Self::check_destination`]), or a PDF/X
    /// output intent it can't write.
    pub fn check(&self, profiles: &dyn ProfileCatalog) -> Result<(), SettingsError> {
        // A blank destination is the document's own profile, which is always there.
        if !self.output.destination.trim().is_empty() {
            self.check_destination(profiles, ColorMode::Rgb)?;
        }
        self.check_output_intent(profiles)?;
        self.check_values()
    }

    /// The part of [`Self::check`] a preset must pass (without the colour settings' profiles): a
    /// standard with a PDF version it doesn't allow, a standard with editing data, PDF/X-1a or
    /// PDF/X-3 with PDF layers, or an out-of-range value.
    pub fn check_values(&self) -> Result<(), SettingsError> {
        self.security.check(self.standard, self.compatibility)?;
        if !self.standard.allows(self.compatibility) {
            return Err(refuse("standardVersion", format!("{} files can't be {}", self.standard.label(), self.compatibility.label()))
                .with("standard", self.standard.label())
                .with("version", self.compatibility.label()));
        }
        if self.preserve_editing && self.standard != Standard::None {
            return Err(refuse("standardEditing", format!("{} files can't carry editing data: turn off preserveEditing", self.standard.label()))
                .with("standard", self.standard.label()));
        }
        if self.create_layers && !self.standard.allows_layers() {
            return Err(refuse("standardLayers", format!("{} files can't have PDF layers: turn off createLayers", self.standard.label()))
                .with("standard", self.standard.label()));
        }
        let c = &self.compression;
        for (name, ppi, above) in
            [("color", c.color.ppi, c.color.above_ppi), ("gray", c.gray.ppi, c.gray.above_ppi), ("mono", c.mono.ppi, c.mono.above_ppi)]
        {
            within("compressionPpi", &format!("compression.{name}.ppi"), ppi, 9.0, 2400.0, " ppi")?;
            within("compressionAbovePpi", &format!("compression.{name}.abovePpi"), above, 9.0, 2400.0, " ppi")?;
        }
        self.marks.check()?;
        self.bleed.check()?;
        within("fontSubsetPercent", "advanced.fontSubsetPercent", self.advanced.font_subset_percent, 0.0, 100.0, "%")
    }

    /// Refuse a destination profile that isn't there (when converting). A blank destination is the
    /// profile of a document in colour mode `mode`. PDF/X-1a files always convert, to a CMYK
    /// destination: blank, their output intent's profile when it is a CMYK one, else the CMYK
    /// profile in effect.
    pub fn check_destination(&self, profiles: &dyn ProfileCatalog, mode: ColorMode) -> Result<(), SettingsError> {
        let o = &self.output;
        let cmyk_only = self.standard.cmyk_only();
        if o.conversion == ColorConversion::None && !cmyk_only {
            return Ok(());
        }
        let name = o.destination.trim();
        let name = match (name.is_empty(), mode) {
            // Document CMYK: the CMYK profile in effect, in an RGB document too.
            (false, _) if name.eq_ignore_ascii_case(DOCUMENT_CMYK) => profiles.cmyk(),
            (false, _) => name,
            (true, _) if cmyk_only => {
                let intent = o.output_intent.trim();
                if profiles.kind(intent) == Some(ProfileKind::Cmyk) { intent } else { profiles.cmyk() }
            }
            (true, ColorMode::Cmyk) => profiles.cmyk(),
            (true, ColorMode::Rgb) => profiles.rgb(),
        };
        let unknown = || refuse("destinationUnknown", format!("output.destination: no RGB or CMYK profile is called “{name}”")).with("name", name);
        match profiles.kind(name).ok_or_else(unknown)? {
            ProfileKind::Gray => Err(unknown()),
            ProfileKind::Rgb if cmyk_only => {
                Err(refuse("destinationRgb", format!("output.destination: {} files are CMYK, and “{name}” is an RGB profile", self.standard.label()))
                    .with("standard", self.standard.label())
                    .with("name", name))
            }
            ProfileKind::Rgb | ProfileKind::Cmyk => Ok(()),
        }
    }

    /// Refuse a PDF/X output intent the file can't have: a grey profile, an RGB one in PDF/X-1a, or
    /// a name that isn't a profile, unless PDF/X-1a and PDF/X-3 name a registered printing
    /// condition (with a registry) without embedding a profile; PDF/X-4 always embeds one.
    pub fn check_output_intent(&self, profiles: &dyn ProfileCatalog) -> Result<(), SettingsError> {
        let o = &self.output;
        let name = o.output_intent.trim();
        if !self.standard.is_pdfx() || name.is_empty() {
            return Ok(());
        }
        let std = self.standard.label();
        match profiles.kind(name) {
            Some(ProfileKind::Cmyk) => Ok(()),
            Some(ProfileKind::Rgb) if !self.standard.cmyk_only() => Ok(()),
            Some(ProfileKind::Rgb) => {
                Err(refuse("intentRgb", format!("output.outputIntent: {std} files print in CMYK, and “{name}” is an RGB profile")).with("standard", std).with("name", name))
            }
            Some(ProfileKind::Gray) => {
                Err(refuse("intentGray", format!("output.outputIntent: “{name}” is a grey profile, which can't be a {std} output intent")).with("standard", std).with("name", name))
            }
            None if !o.registry.trim().is_empty() && self.standard != Standard::PdfX4 => Ok(()),
            None if self.standard == Standard::PdfX4 => Err(refuse(
                "intentUnknownX4",
                format!("output.outputIntent: no profile is called “{name}”, and a {std} output intent embeds its profile"),
            )
            .with("standard", std)
            .with("name", name)),
            None => Err(refuse(
                "intentUnknown",
                format!(
                    "output.outputIntent: no profile is called “{name}”: a {std} output intent embeds a profile, or names a registered printing condition (outputConditionId and registry)"
                ),
            )
            .with("standard", std)
            .with("name", name)),
        }
    }

    /// Options that are accepted but not applied by the writer yet, one warning each.
    pub fn warnings(&self, profiles: &dyn ProfileCatalog) -> Vec<SettingsWarning> {
        let d = Self::default();
        let s = &self.security;
        [
            (
                self.create_layers && !self.compatibility.has_layers(),
                "layersVersion",
                "PDF layers need PDF 1.5 or later: every layer is plain page content",
            ),
            (
                !self.advanced.outline_text && self.advanced.font_subset_percent < 100.0,
                "fontSubsetThreshold",
                "fonts are embedded as subsets of the characters used: a subset threshold below 100% is not applied",
            ),
            (
                !s.protected()
                    && (s.printing != d.security.printing
                        || s.changes != d.security.changes
                        || s.copy != d.security.copy
                        || s.screen_reader != d.security.screen_reader
                        || s.plaintext_metadata != d.security.plaintext_metadata),
                "permissionsNeedPassword",
                "permissions need a password: without one they are not applied",
            ),
        ]
        .into_iter()
        .filter(|(on, ..)| *on)
        .map(|(_, id, message)| warn(id, message))
        .chain(self.encryption_warnings())
        .chain(self.output_warnings(profiles))
        .collect()
    }

    /// The security options that are accepted but have no effect, one warning each.
    fn encryption_warnings(&self) -> Vec<SettingsWarning> {
        let s = &self.security;
        let c = self.compatibility;
        let mut out = vec![];
        if s.protected() && s.plaintext_metadata && revision(c) <= 3 {
            out.push(
                warn(
                    "plaintextMetadataVersion",
                    format!("{} encryption covers the metadata too: plaintext metadata needs PDF 1.5 or later", c.label()),
                )
                .with("version", c.label()),
            );
        }
        if s.protected() && c == Compatibility::Pdf13 {
            out.push(
                warn(
                    "weakKey",
                    format!(
                        "{} encrypts with a 40-bit key, which can be searched in minutes: choose PDF 1.6 or later for stronger protection",
                        c.label()
                    ),
                )
                .with("version", c.label()),
            );
        }
        out
    }

    /// What the Output section leaves out, one warning each.
    fn output_warnings(&self, profiles: &dyn ProfileCatalog) -> Vec<SettingsWarning> {
        let o = &self.output;
        let asks = !o.output_intent.trim().is_empty() || !o.output_condition_id.trim().is_empty();
        let mut out = vec![];
        if self.standard == Standard::PdfA2b {
            if asks || o.trapped {
                out.push(warn("pdfaOutputIntent", "PDF/A files carry their own output intent: the output intent and Trapped entries are left out"));
            }
            return out;
        }
        let name = o.output_intent.trim();
        if !name.is_empty() && profiles.kind(name).is_none() {
            out.push(
                warn("intentNotEmbedded", format!("no profile is called “{name}”: the output intent names it without embedding it"))
                    .with("name", name),
            );
        }
        // PDF/X files always have an output intent.
        if !asks && !self.standard.is_pdfx() && (!o.output_condition.trim().is_empty() || !o.registry.trim().is_empty()) {
            out.push(warn(
                "conditionNeedsIntent",
                "an output intent needs a profile or a condition identifier: the output condition and registry are left out",
            ));
        }
        out
    }
}

impl MarkSettings {
    /// Refuse a weight or offset out of range.
    pub fn check(&self) -> Result<(), SettingsError> {
        within("marksWeight", "marks.weight", self.weight, 0.05, 2.0, " pt")?;
        within("marksOffset", "marks.offset", self.offset, 0.0, 72.0, " pt")
    }
}

impl BleedSettings {
    /// Refuse a side out of range.
    pub fn check(&self) -> Result<(), SettingsError> {
        for (side, v) in ["top", "bottom", "left", "right"].iter().zip(self.values()) {
            within("bleedSide", &format!("bleed.{side}"), v, 0.0, 72.0, " pt")?;
        }
        Ok(())
    }
}

impl SecuritySettings {
    /// Refuse passwords the file can't carry: with a PDF standard (PDF/A and PDF/X forbid
    /// encryption), two equal passwords, or passwords the algorithm of `c` can't take.
    pub fn check(&self, standard: Standard, c: Compatibility) -> Result<(), SettingsError> {
        if !self.protected() {
            return Ok(());
        }
        if standard != Standard::None {
            return Err(refuse(
                "standardPassword",
                format!("{} files can't be password-protected: remove the passwords or choose no standard", standard.label()),
            )
            .with("standard", standard.label()));
        }
        // Revision 2 has one printing bit: it can't say "low resolution only".
        if revision(c) == 2 && self.printing == Printing::Low && !self.permissions_password.is_empty() {
            return Err(refuse(
                "lowPrintingVersion",
                format!("{} can't limit printing to low resolution: choose PDF 1.4 or later, or allow high-resolution printing", c.label()),
            )
            .with("version", c.label()));
        }
        if self.open_password == self.permissions_password {
            return Err(refuse("passwordsEqual", "the open and permissions passwords must differ"));
        }
        for pw in [&self.open_password, &self.permissions_password] {
            if revision(c) < 6 {
                if !pw.is_ascii() || pw.len() > MAX_LEGACY_PASSWORD {
                    return Err(refuse(
                        "passwordLegacy",
                        format!("{} passwords can have up to {MAX_LEGACY_PASSWORD} ASCII characters (PDF 1.7 and later take any)", c.label()),
                    )
                    .with("version", c.label())
                    .with("limit", MAX_LEGACY_PASSWORD.to_string()));
                }
            } else if pw.len() > MAX_PASSWORD {
                return Err(refuse("passwordLong", format!("passwords can have up to {MAX_PASSWORD} bytes")).with("limit", MAX_PASSWORD.to_string()));
            }
        }
        Ok(())
    }
}
