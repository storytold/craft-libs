//! PDF export settings: the model a PDF export dialog edits and a preset stores, the checks that
//! refuse settings a writer can't honour ([`checks`]) and the built-in presets ([`presets`]).
//!
//! The module needs no feature. Applications save settings with the `serde` feature (VectorCraft's
//! camelCase JSON shape; every field optional, passwords never written) or through the stable `id`
//! (and `from_id`) of every choice enum. Checks and warnings that depend
//! on colour profiles ask the application through [`ProfileCatalog`].

pub mod checks;
pub mod presets;

pub use checks::{ColorMode, ProfileCatalog, ProfileKind, SettingsError, SettingsWarning};
pub use presets::{PdfPreset, builtin_preset, builtin_presets, default_preset_name};

/// A closed list of choices (a settings enum), for UIs that show it as a dropdown.
pub trait Choice: Copy + 'static {
    /// Every choice, in display order.
    const ALL: &'static [Self];
    /// The stable ids, in display order.
    const IDS: &'static [&'static str];
    /// The English labels shown in the PDF dialogs, in display order.
    const LABELS: &'static [&'static str];
}

/// A closed list of choices with a stable id and an English label per variant.
macro_rules! choice {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $v:ident = $id:literal, $label:literal;)+ } default $d:ident) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[derive(serde::Serialize, serde::Deserialize)]
        pub enum $name {
            $($(#[$vm])* #[serde(rename = $id)] $v,)+
        }

        impl Choice for $name {
            const ALL: &'static [Self] = &[$(Self::$v),+];
            const IDS: &'static [&'static str] = &[$($id),+];
            const LABELS: &'static [&'static str] = &[$($label),+];
        }

        impl $name {
            /// The English label shown in the PDF dialogs.
            pub fn label(self) -> &'static str {
                match self {
                    $(Self::$v => $label,)+
                }
            }

            /// The stable id.
            pub fn id(self) -> &'static str {
                match self {
                    $(Self::$v => $id,)+
                }
            }

            /// The choice with this id.
            pub fn from_id(id: &str) -> Option<Self> {
                match id {
                    $($id => Some(Self::$v),)+
                    _ => None,
                }
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::$d
            }
        }
    };
}

choice! {
    /// The PDF standard the file conforms to.
    Standard {
        #[allow(missing_docs)]
        None = "none", "None";
        /// PDF/A-2b (archival), validated by the writer.
        PdfA2b = "pdfA2b", "PDF/A-2b";
        #[allow(missing_docs)]
        PdfX1a = "pdfX1a", "PDF/X-1a:2001";
        #[allow(missing_docs)]
        PdfX3 = "pdfX3", "PDF/X-3:2002";
        #[allow(missing_docs)]
        PdfX4 = "pdfX4", "PDF/X-4:2010";
    } default None
}

impl Standard {
    /// A file of this standard can declare PDF version `c`: PDF/A-2 is based on PDF 1.7 (so it
    /// can't be PDF 2.0, nor PDF 1.3, which has no transparency), PDF/X-4 on PDF 1.6, and PDF/X-1a
    /// and PDF/X-3 on PDF 1.3 (PDF 1.4 is accepted too, and written as 1.3).
    pub fn allows(self, c: Compatibility) -> bool {
        use Compatibility::*;
        match self {
            Self::None => true,
            Self::PdfA2b => !matches!(c, Pdf13 | Pdf20),
            Self::PdfX1a | Self::PdfX3 => matches!(c, Pdf13 | Pdf14),
            Self::PdfX4 => matches!(c, Pdf14 | Pdf15 | Pdf16),
        }
    }

    /// The PDF version a file of this standard is (what choosing it sets).
    pub fn version(self) -> Compatibility {
        match self {
            Self::None | Self::PdfA2b => Compatibility::Pdf17,
            Self::PdfX1a | Self::PdfX3 => Compatibility::Pdf13,
            Self::PdfX4 => Compatibility::Pdf16,
        }
    }

    /// A PDF/X standard.
    pub fn is_pdfx(self) -> bool {
        matches!(self, Self::PdfX1a | Self::PdfX3 | Self::PdfX4)
    }

    /// Files of this standard have no transparency (PDF/X-1a and PDF/X-3): it is flattened.
    pub fn flattens(self) -> bool {
        matches!(self, Self::PdfX1a | Self::PdfX3)
    }

    /// Every colour is CMYK, grey or a spot ink (PDF/X-1a).
    pub fn cmyk_only(self) -> bool {
        self == Self::PdfX1a
    }

    /// Files of this standard may have PDF layers (PDF/X-1a and PDF/X-3 are PDF 1.3, which has
    /// none).
    pub fn allows_layers(self) -> bool {
        !self.flattens()
    }
}

choice! {
    /// The PDF version written in the file header.
    Compatibility {
        /// No transparency: it is flattened (see [`PdfSettings::pdf13`]).
        Pdf13 = "1.3", "PDF 1.3";
        #[allow(missing_docs)]
        Pdf14 = "1.4", "PDF 1.4";
        #[allow(missing_docs)]
        Pdf15 = "1.5", "PDF 1.5";
        #[allow(missing_docs)]
        Pdf16 = "1.6", "PDF 1.6";
        #[allow(missing_docs)]
        Pdf17 = "1.7", "PDF 1.7";
        #[allow(missing_docs)]
        Pdf20 = "2.0", "PDF 2.0";
    } default Pdf17
}

impl Compatibility {
    /// The version has PDF layers (optional content, PDF 1.5).
    pub fn has_layers(self) -> bool {
        matches!(self, Self::Pdf15 | Self::Pdf16 | Self::Pdf17 | Self::Pdf20)
    }

    /// The version has a page's `UserUnit` (PDF 1.6).
    pub fn has_user_unit(self) -> bool {
        matches!(self, Self::Pdf16 | Self::Pdf17 | Self::Pdf20)
    }
}

choice! {
    /// How images above the threshold resolution are resampled.
    Downsample {
        #[allow(missing_docs)]
        None = "none", "No downsampling";
        #[allow(missing_docs)]
        Average = "average", "Average";
        #[allow(missing_docs)]
        Subsample = "subsample", "Subsample";
        #[allow(missing_docs)]
        Bicubic = "bicubic", "Bicubic";
    } default None
}

choice! {
    /// Compression of colour and greyscale images.
    ImageCodec {
        #[allow(missing_docs)]
        None = "none", "None";
        #[allow(missing_docs)]
        Zip = "zip", "ZIP";
        #[allow(missing_docs)]
        Jpeg = "jpeg", "JPEG";
        #[allow(missing_docs)]
        Jpeg2000 = "jpeg2000", "JPEG 2000";
        /// JPEG images stay JPEG, the others are compressed losslessly.
        Auto = "auto", "Automatic";
    } default Auto
}

choice! {
    /// JPEG image quality.
    JpegQuality {
        #[allow(missing_docs)]
        Minimum = "minimum", "Minimum";
        #[allow(missing_docs)]
        Low = "low", "Low";
        #[allow(missing_docs)]
        Medium = "medium", "Medium";
        #[allow(missing_docs)]
        High = "high", "High";
        #[allow(missing_docs)]
        Maximum = "maximum", "Maximum";
    } default Maximum
}

choice! {
    /// Compression of 1-bit (monochrome) images.
    MonoCodec {
        #[allow(missing_docs)]
        None = "none", "None";
        #[allow(missing_docs)]
        CcittG3 = "ccittG3", "CCITT Group 3";
        #[allow(missing_docs)]
        CcittG4 = "ccittG4", "CCITT Group 4";
        #[allow(missing_docs)]
        Zip = "zip", "ZIP";
        #[allow(missing_docs)]
        RunLength = "runLength", "Run Length";
    } default Zip
}

choice! {
    /// The style of printer's marks.
    MarkKind {
        #[allow(missing_docs)]
        Roman = "roman", "Roman";
        #[allow(missing_docs)]
        Japanese = "japanese", "Japanese";
    } default Roman
}

choice! {
    /// Colour conversion on output.
    ColorConversion {
        #[allow(missing_docs)]
        None = "none", "No conversion";
        #[allow(missing_docs)]
        Destination = "destination", "Convert to destination";
        /// Convert only colours whose profile differs from the destination's.
        PreserveNumbers = "preserveNumbers", "Convert to destination (preserve numbers)";
    } default None
}

choice! {
    /// Which ICC profiles the file embeds.
    ProfileInclusion {
        #[allow(missing_docs)]
        None = "none", "Don't include profiles";
        #[allow(missing_docs)]
        All = "all", "Include all profiles";
        #[allow(missing_docs)]
        Destination = "destination", "Include destination profile";
        #[allow(missing_docs)]
        TaggedSource = "taggedSource", "Include tagged source profiles";
    } default None
}

choice! {
    /// What happens to overprinting fills and strokes.
    Overprint {
        #[allow(missing_docs)]
        Preserve = "preserve", "Preserve";
        #[allow(missing_docs)]
        Discard = "discard", "Discard";
    } default Preserve
}

choice! {
    /// Printing allowed by the permissions.
    Printing {
        #[allow(missing_docs)]
        None = "none", "None";
        #[allow(missing_docs)]
        Low = "low", "Low resolution (150 ppi)";
        #[allow(missing_docs)]
        High = "high", "High resolution";
    } default High
}

choice! {
    /// Changes allowed by the permissions.
    Changes {
        #[allow(missing_docs)]
        None = "none", "None";
        #[allow(missing_docs)]
        Pages = "pages", "Inserting, deleting and rotating pages";
        #[allow(missing_docs)]
        Forms = "forms", "Filling in form fields and signing";
        #[allow(missing_docs)]
        Comments = "comments", "Commenting, filling in form fields and signing";
        #[allow(missing_docs)]
        Any = "any", "Any except extracting pages";
    } default Any
}

choice! {
    /// The encryption algorithm, which follows the PDF version ([`Compatibility`]).
    Encryption {
        #[allow(missing_docs)]
        Rc440 = "rc440", "40-bit RC4";
        #[allow(missing_docs)]
        Rc4 = "rc4", "128-bit RC4";
        #[allow(missing_docs)]
        Aes128 = "aes128", "128-bit AES";
        #[allow(missing_docs)]
        Aes256 = "aes256", "256-bit AES";
    } default Aes256
}

impl Encryption {
    /// The algorithm a file of PDF version `c` is encrypted with.
    pub fn for_compatibility(c: Compatibility) -> Self {
        match c {
            Compatibility::Pdf13 => Self::Rc440,
            Compatibility::Pdf14 | Compatibility::Pdf15 => Self::Rc4,
            Compatibility::Pdf16 => Self::Aes128,
            Compatibility::Pdf17 | Compatibility::Pdf20 => Self::Aes256,
        }
    }
}

/// PDF export settings (the dialog's sections; what a preset stores).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PdfSettings {
    #[allow(missing_docs)]
    pub standard: Standard,
    #[allow(missing_docs)]
    pub compatibility: Compatibility,
    // General.
    /// Embed the native document so the PDF reopens editable.
    pub preserve_editing: bool,
    /// Embed a thumbnail image per page.
    pub thumbnails: bool,
    /// Write a linearised file that displays page by page while it downloads.
    pub fast_web_view: bool,
    /// The app opens the written file (frontends only; the writer ignores it).
    pub view_after_saving: bool,
    /// Write layers and sublayers as PDF layers (optional content).
    pub create_layers: bool,
    /// Keep the layers whose Print option is off (they are left out otherwise, unless
    /// `create_layers` is on).
    pub include_non_printing: bool,
    #[allow(missing_docs)]
    pub compression: CompressionSettings,
    #[allow(missing_docs)]
    pub marks: MarkSettings,
    #[allow(missing_docs)]
    pub bleed: BleedSettings,
    #[allow(missing_docs)]
    pub output: OutputSettings,
    #[allow(missing_docs)]
    pub advanced: AdvancedSettings,
    #[allow(missing_docs)]
    pub security: SecuritySettings,
    /// The transparency flattener preset PDF 1.3 files (PDF/X-1a and PDF/X-3 ones too) are
    /// flattened with: a built-in or saved preset's name; empty, High Resolution. The app flattens
    /// the document before writing; the writer doesn't read it.
    pub flattener_preset: String,
}

/// Resampling and compression of one kind of image.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ImageCompression {
    #[allow(missing_docs)]
    pub downsample: Downsample,
    /// Target resolution (ppi).
    pub ppi: f64,
    /// Only images above this resolution (ppi) are downsampled.
    pub above_ppi: f64,
    #[allow(missing_docs)]
    pub compression: ImageCodec,
    #[allow(missing_docs)]
    pub quality: JpegQuality,
}

impl Default for ImageCompression {
    fn default() -> Self {
        Self { downsample: Downsample::None, ppi: 300.0, above_ppi: 450.0, compression: ImageCodec::Auto, quality: JpegQuality::Maximum }
    }
}

/// Resampling and compression of monochrome images.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MonoCompression {
    #[allow(missing_docs)]
    pub downsample: Downsample,
    /// Target resolution (ppi).
    pub ppi: f64,
    /// Only images above this resolution (ppi) are downsampled.
    pub above_ppi: f64,
    #[allow(missing_docs)]
    pub compression: MonoCodec,
}

impl Default for MonoCompression {
    fn default() -> Self {
        Self { downsample: Downsample::None, ppi: 1200.0, above_ppi: 1800.0, compression: MonoCodec::Zip }
    }
}

/// The Compression section.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CompressionSettings {
    /// Settings for colour images.
    pub color: ImageCompression,
    /// Settings for greyscale images.
    pub gray: ImageCompression,
    /// Settings for monochrome images.
    pub mono: MonoCompression,
    /// Compress text and line art (Flate content streams).
    pub compress_text: bool,
}

impl Default for CompressionSettings {
    fn default() -> Self {
        Self { color: ImageCompression::default(), gray: ImageCompression::default(), mono: MonoCompression::default(), compress_text: true }
    }
}

/// Printer's marks (Marks and Bleeds section).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MarkSettings {
    #[allow(missing_docs)]
    pub trim: bool,
    #[allow(missing_docs)]
    pub registration: bool,
    #[allow(missing_docs)]
    pub color_bars: bool,
    #[allow(missing_docs)]
    pub page_info: bool,
    #[allow(missing_docs)]
    pub kind: MarkKind,
    /// Trim mark weight (pt).
    pub weight: f64,
    /// Distance of the marks from the artboard (pt).
    pub offset: f64,
}

impl Default for MarkSettings {
    fn default() -> Self {
        Self { trim: false, registration: false, color_bars: false, page_info: false, kind: MarkKind::Roman, weight: 0.25, offset: 6.0 }
    }
}

impl MarkSettings {
    /// Any mark is on.
    pub fn any(&self) -> bool {
        self.trim || self.registration || self.color_bars || self.page_info
    }
}

/// Bleed (Marks and Bleeds section), in points.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BleedSettings {
    /// Use the document's bleed instead of the values below.
    pub use_document: bool,
    #[allow(missing_docs)]
    pub top: f64,
    #[allow(missing_docs)]
    pub bottom: f64,
    #[allow(missing_docs)]
    pub left: f64,
    #[allow(missing_docs)]
    pub right: f64,
}

impl BleedSettings {
    /// `[top, bottom, left, right]`.
    pub fn values(&self) -> [f64; 4] {
        [self.top, self.bottom, self.left, self.right]
    }
}

/// The Output destination that names the CMYK profile of the colour settings in effect, in an RGB
/// document as in a CMYK one (Illustrator's "Document CMYK").
pub const DOCUMENT_CMYK: &str = "Document CMYK";

/// The Output section.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OutputSettings {
    #[allow(missing_docs)]
    pub conversion: ColorConversion,
    /// Destination profile name (empty: the document's profile; [`DOCUMENT_CMYK`]: the CMYK
    /// profile in effect, whatever the document's colour mode).
    pub destination: String,
    #[allow(missing_docs)]
    pub profiles: ProfileInclusion,
    /// PDF/X output intent profile name.
    pub output_intent: String,
    /// Output condition text, as entered.
    pub output_condition: String,
    /// Output condition identifier, as entered.
    pub output_condition_id: String,
    /// Registry name, as entered.
    pub registry: String,
    /// Mark the file as trapped.
    pub trapped: bool,
}

/// The Advanced section.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AdvancedSettings {
    /// Embed whole fonts when more than this share (%) of their characters is used.
    pub font_subset_percent: f64,
    /// Text as glyph outlines; off, text is real (selectable, searchable) text in embedded subset
    /// fonts.
    pub outline_text: bool,
    #[allow(missing_docs)]
    pub overprint: Overprint,
}

impl Default for AdvancedSettings {
    fn default() -> Self {
        Self { font_subset_percent: 100.0, outline_text: false, overprint: Overprint::Preserve }
    }
}

/// The Security section. Passwords are never part of a saved preset: save with
/// [`PdfSettings::clear_passwords`] applied.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SecuritySettings {
    /// Password required to open the file; empty for none. Never serialised.
    #[serde(skip_serializing)]
    pub open_password: String,
    /// Password that lifts the permission limits; empty for none. Never serialised.
    #[serde(skip_serializing)]
    pub permissions_password: String,
    #[allow(missing_docs)]
    pub printing: Printing,
    #[allow(missing_docs)]
    pub changes: Changes,
    /// Allow copying text, images and other content.
    pub copy: bool,
    /// Allow screen readers to read the text.
    pub screen_reader: bool,
    /// Leave the metadata unencrypted.
    pub plaintext_metadata: bool,
}

impl Default for SecuritySettings {
    fn default() -> Self {
        Self {
            open_password: String::new(),
            permissions_password: String::new(),
            printing: Printing::High,
            changes: Changes::Any,
            copy: true,
            screen_reader: true,
            plaintext_metadata: true,
        }
    }
}

impl SecuritySettings {
    /// A password is set: the file is encrypted.
    pub fn protected(&self) -> bool {
        !self.open_password.is_empty() || !self.permissions_password.is_empty()
    }

    /// Some permission is withheld (only a permissions password applies them).
    pub fn restricts(&self) -> bool {
        self.printing != Printing::High || self.changes != Changes::Any || !self.copy || !self.screen_reader
    }

    /// The encryption dictionary's `P` entry: the permission bits (1-based bit numbers of the PDF
    /// reference's user access permissions). Everything is allowed without a permissions password.
    pub fn permission_bits(&self) -> i32 {
        if self.permissions_password.is_empty() {
            return !3;
        }
        let bit = |n: u32| 1u32 << (n - 1);
        // Bits 1–2 are clear; the reserved bits 7–8 and 13–32 are set.
        let mut p = 0xFFFF_F0C0u32;
        p |= match self.printing {
            Printing::None => 0,
            Printing::Low => bit(3),
            Printing::High => bit(3) | bit(12),
        };
        p |= match self.changes {
            Changes::None => 0,
            Changes::Pages => bit(11),
            Changes::Forms => bit(9),
            Changes::Comments => bit(6) | bit(9),
            Changes::Any => bit(4) | bit(6) | bit(9) | bit(11),
        };
        if self.copy {
            p |= bit(5);
        }
        // Content copied may always be read out by screen readers.
        if self.copy || self.screen_reader {
            p |= bit(10);
        }
        p as i32
    }
}

impl PdfSettings {
    /// Layers and sublayers are written as PDF layers: asked for, at a version that has them.
    pub fn writes_layers(&self) -> bool {
        self.create_layers && self.compatibility.has_layers()
    }

    /// The file is PDF 1.3 (asked for, or a PDF/X-1a or PDF/X-3 file): it has no transparency (the
    /// app flattens it, see [`Self::flattener_preset`]), is written with the PDF 1.4 settings, and
    /// its header and metadata say 1.3.
    pub fn pdf13(&self) -> bool {
        self.compatibility == Compatibility::Pdf13 || self.standard.flattens()
    }

    /// Forget the passwords (presets never store them).
    pub fn clear_passwords(&mut self) {
        self.security.open_password.clear();
        self.security.permissions_password.clear();
    }

    /// The algorithm the file is encrypted with; `None` without a password.
    pub fn encryption(&self) -> Option<Encryption> {
        self.security.protected().then(|| Encryption::for_compatibility(self.compatibility))
    }
}
