//! Variable font axes (F14.09): the axes and named instances of a face, and the row of sliders
//! that edits them (Photoshop's Properties › Character axis sliders; FigCraft's axis panel). The data types
//! and reading them from a font are always available; the sliders are on `ui`.

#[cfg(feature = "ui")]
pub(crate) mod button;
#[cfg(feature = "ui")]
pub(crate) mod sliders;
#[cfg(feature = "ui")]
pub use button::AxesButton;
#[cfg(feature = "ui")]
pub use sliders::*;

use skrifa::string::StringId;
use skrifa::{FontRef, MetadataProvider as _};

/// One variation axis of a face.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct FontAxis {
    /// The OpenType tag (`wght`, `wdth`, `slnt`, `ital`, `opsz`, or a custom one).
    pub tag: [u8; 4],
    /// The axis's name from the font's name table (`Weight`), English when there is one.
    pub name: String,
    /// Lowest value, in the axis's own units.
    pub min: f32,
    /// Value the face uses when no coordinate is set.
    pub default: f32,
    /// Highest value, in the axis's own units.
    pub max: f32,
    /// Hidden from the user by the font (`HIDDEN_AXIS`).
    pub hidden: bool,
}

impl FontAxis {
    /// The axis `tag` named `name`, from `min` to `max` with the face's `default`; not hidden.
    pub fn new(tag: [u8; 4], name: impl Into<String>, min: f32, default: f32, max: f32) -> Self {
        FontAxis { tag, name: name.into(), min, default, max, hidden: false }
    }

    /// Whether the font hides the axis from the user.
    #[must_use]
    pub fn hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }
}

/// A named instance of a variable face (`Bold Condensed`): its coordinates per axis.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct NamedInstance {
    /// The instance's name from the font's name table.
    pub name: String,
    /// (tag, value) for every axis of the face, in axis order.
    pub coordinates: Vec<([u8; 4], f32)>,
}

impl NamedInstance {
    /// The instance `name` at `coordinates`, as (tag, value) for every axis of the face in axis order.
    pub fn new(name: impl Into<String>, coordinates: Vec<([u8; 4], f32)>) -> Self {
        NamedInstance { name: name.into(), coordinates }
    }
}

/// How close two coordinates are to count as equal: well under the 0.01 the sliders step by.
const SAME: f32 = 0.005;

/// The instance whose coordinates equal `values` on every axis (the Style field shows its name),
/// else `None` (the field shows "Custom").
// pc: crates/text/src/fonts.rs:512 (instance matching by axis values, #8797).
pub fn matching_instance<'a>(instances: &'a [NamedInstance], values: &[([u8; 4], f32)]) -> Option<&'a NamedInstance> {
    instances.iter().find(|instance| {
        !instance.coordinates.is_empty()
            && instance.coordinates.iter().all(|(tag, at)| values.iter().any(|(t, v)| t == tag && (v - at).abs() <= SAME))
    })
}

/// The axes of the face at `index` in `data`, in font order; empty for a static font.
pub fn read_axes(data: &[u8], index: u32) -> Vec<FontAxis> {
    let Ok(font) = FontRef::from_index(data, index) else { return Vec::new() };
    font.axes()
        .iter()
        .map(|axis| {
            let label = name(&font, axis.name_id()).unwrap_or_default();
            FontAxis::new(axis.tag().to_be_bytes(), label, axis.min_value(), axis.default_value(), axis.max_value()).hidden(axis.is_hidden())
        })
        .collect()
}

/// The named instances of the face at `index` in `data`, in font order.
pub fn read_named_instances(data: &[u8], index: u32) -> Vec<NamedInstance> {
    let Ok(font) = FontRef::from_index(data, index) else { return Vec::new() };
    let tags: Vec<[u8; 4]> = font.axes().iter().map(|axis| axis.tag().to_be_bytes()).collect();
    font.named_instances()
        .iter()
        .map(|instance| {
            let label = name(&font, instance.subfamily_name_id()).unwrap_or_default();
            NamedInstance::new(label, tags.iter().copied().zip(instance.user_coords()).collect())
        })
        .collect()
}

fn name(font: &FontRef, id: StringId) -> Option<String> {
    font.localized_strings(id).english_or_first().map(|s| s.to_string()).filter(|s| !s.is_empty())
}
