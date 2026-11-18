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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    SansSerif,
    Serif,
    Display,
    Handwriting,
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

pub fn script_name<'a>(index: &'a Index, id: &str) -> Option<&'a str> {
    index.scripts().iter().find(|s| s.id == id).map(|s| s.name.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_index_reads() {
        let index = bundled().unwrap();
        assert!(index.families().len() > 1000);
        let roboto = index.family("roboto").unwrap();
        assert_eq!(categories(roboto).collect::<Vec<_>>(), [Category::SansSerif]);
        assert_eq!(script_name(&index, "Arab"), Some("Arabic"));
        assert_eq!(source_label(&index, roboto), ("Google Fonts", true));
        let cjk = index.family("Noto Sans CJK JP").unwrap();
        assert_eq!(source_label(&index, cjk), ("Noto", true));
    }
}
