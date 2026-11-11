//! The rows of the style picker: the document's named styles as the rows of a `PresetPicker`.

use crate::styles::{group_by_folder, style_leaf};
use craft_behavior::IconId;
use craft_theme::Rgba;
use craft_ui::patterns::presets::{PresetKey, PresetRow};
use craft_ui::style::TypeRole;

/// What a style row shows before its name.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum StylePreview {
    /// A swatch (colour and fill styles).
    Color(Rgba),
    /// A short sample drawn in the style's type role.
    #[non_exhaustive]
    Sample {
        /// The sample text ("Ag").
        text: String,
        /// The type role it is drawn in.
        role: TypeRole,
    },
    /// An icon from the icon set (effect and grid styles).
    Icon(IconId),
}

/// One style in the picker; its folder is the name's ([`crate::styles::style_folder`]).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct NamedStyleRow {
    /// The style's id.
    pub id: u64,
    /// The style's name; folders are split off it.
    pub name: String,
    /// The second line ("Inter 16/24", "#1473E6").
    pub meta: String,
    /// What the row shows before its name.
    pub preview: StylePreview,
}

impl StylePreview {
    /// A sample `text` drawn in `role`.
    pub fn sample(text: impl Into<String>, role: TypeRole) -> Self {
        StylePreview::Sample { text: text.into(), role }
    }
}

impl NamedStyleRow {
    /// A style `name` with the id `id`, showing `preview` and with no second line.
    pub fn new(id: u64, name: impl Into<String>, preview: StylePreview) -> Self {
        NamedStyleRow { id, name: name.into(), meta: String::new(), preview }
    }

    /// The second line under the name.
    #[must_use]
    pub fn meta(mut self, meta: impl Into<String>) -> Self {
        self.meta = meta.into();
        self
    }
}

/// The picker's rows: styles with no folder first, then each folder as a group row over its styles, in order of
/// first appearance. A style's key is `PresetKey::Item(id)`.
pub fn style_rows(styles: &[NamedStyleRow]) -> Vec<PresetRow<PresetKey<u64>>> {
    let mut rows = Vec::new();
    for (folder, group) in group_by_folder(styles, |style: &NamedStyleRow| style.name.as_str()) {
        let depth = usize::from(!folder.is_empty());
        if depth == 1 {
            rows.push(PresetRow::group(PresetKey::group([folder.clone()]), folder.clone(), 0).count(group.len()));
        }
        for style in group {
            let leaf = style_leaf(&style.name);
            let mut row = PresetRow::item(PresetKey::Item(style.id), leaf, depth).tooltip(style.name.clone());
            if !style.meta.is_empty() {
                row = row.caption(style.meta.clone());
            }
            if let StylePreview::Icon(icon) = &style.preview {
                row = row.icon(icon.clone());
            }
            rows.push(row);
        }
    }
    rows
}
