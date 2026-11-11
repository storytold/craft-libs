//! Page numbering (N3.29): a page's number in its section's style, start and prefix (InDesign's
//! Numbering & Section Options, Word's Page Number Format). The styles are craft-text's list
//! number styles, so a page number and a list number written "iv" look the same.

pub use craft_text::lists::NumberStyle;

/// The styles a page numbering Style menu offers, in DesignCraft's order; each item reads its
/// [`NumberStyle::sample`] ("1, 2, 3, 4…").
// designcraft: crates/ui-egui/src/section_options.rs:14 `STYLES`.
pub const PAGE_NUMBER_STYLES: [NumberStyle; 8] = [
    NumberStyle::Arabic,
    NumberStyle::UpperRoman,
    NumberStyle::LowerRoman,
    NumberStyle::UpperLetters,
    NumberStyle::LowerLetters,
    NumberStyle::ArabicLeadingZero,
    NumberStyle::ArabicThreeDigits,
    NumberStyle::ArabicFourDigits,
];

/// A section's page numbering: the number its first page gets, the style, and a prefix shown with
/// the number where `include_prefix` says (InDesign's Section Prefix, "Include Prefix when
/// Numbering Pages").
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PageNumbering {
    #[allow(missing_docs)]
    pub style: NumberStyle,
    /// The first page's number. 0 is written in each style's zero form, as DesignCraft's
    /// `NumberStyle::format` does: Roman "0", letters "", symbols "*", kanji "〇", padded
    /// Arabic "00", "000", "0000".
    pub start: u32,
    /// The text put before the number when `include_prefix` is on.
    pub prefix: String,
    /// Whether the prefix is shown in front of the number.
    pub include_prefix: bool,
}

impl Default for PageNumbering {
    fn default() -> Self {
        PageNumbering { style: NumberStyle::Arabic, start: 1, prefix: String::new(), include_prefix: false }
    }
}

impl PageNumbering {
    /// The label of the section's page `index` (0-based within the section): the prefix when
    /// included, then `start + index` in the style ("A-iv"); a number of 0 is written in the
    /// style's zero form (see [`start`](Self::start)). Saturates at `u32::MAX`.
    // designcraft: crates/ui-egui/src/section_options.rs:32 `open` (start number and prefix fields);
    // crates/doc/src/attrs/rules_lists.rs:144 `NumberStyle::format`.
    pub fn label(&self, index: usize) -> String {
        let number = u32::try_from(index).unwrap_or(u32::MAX).saturating_add(self.start);
        let prefix = if self.include_prefix { self.prefix.as_str() } else { "" };
        format!("{prefix}{}", self.style.format(number))
    }
}
