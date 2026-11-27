//! The font indexes (aiongg/craft-font-index: Google Fonts, Noto, and sources the user adds), as
//! craft-font-store reads them, and the categories the catalogue filters by.
//!
//! [`Index::families`] holds every source's families, sorted by name; a family's source is
//! [`Index::source_of`] (its name, and whether it is curated: [`Family::is_curated`] is false for
//! a source the user added). [`Index::problems`] lists the sources that couldn't be loaded.

pub use craft_font_store::{Family, FontFile, FontSource, GOOGLE_FONTS, Index, IndexError, Language, Script, SourceProblem};

/// The pinned built-in indexes compiled into craft-font-store, read without a request.
pub fn bundled() -> Result<Index, IndexError> {
    Index::bundled()
}

/// The display name of a family's source (`Google Fonts`, `Noto`), and whether it is curated.
pub fn source_label<'a>(index: &'a Index, family: &'a Family) -> (&'a str, bool) {
    index.source_of(family).map_or((family.source.as_str(), family.is_curated()), |s| (s.name.as_str(), s.curated))
}

/// A Google Fonts category that the filter offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Category {
    /// Sans serif faces.
    SansSerif,
    /// Serif faces.
    Serif,
    /// Display faces.
    Display,
    /// Handwriting faces.
    Handwriting,
    /// Monospaced faces.
    Monospace,
}

impl Category {
    /// The categories the filter offers, in Google Fonts' order.
    pub const ALL: [Category; 5] = [Category::SansSerif, Category::Serif, Category::Display, Category::Handwriting, Category::Monospace];

    /// The index's name for it (`SANS_SERIF`).
    pub fn of(name: &str) -> Option<Category> {
        match name {
            "SANS_SERIF" => Some(Category::SansSerif),
            "SERIF" => Some(Category::Serif),
            "DISPLAY" => Some(Category::Display),
            "HANDWRITING" => Some(Category::Handwriting),
            "MONOSPACE" => Some(Category::Monospace),
            _ => None,
        }
    }
}

/// A family's categories that this version knows.
pub fn categories(family: &Family) -> impl Iterator<Item = Category> + '_ {
    family.categories.iter().filter_map(|c| Category::of(c))
}

/// The display name of the script with ISO 15924 code `id`, if the index lists it.
pub fn script_name<'a>(index: &'a Index, id: &str) -> Option<&'a str> {
    index.scripts().iter().find(|s| s.id == id).map(|s| s.name.as_str())
}
