//! Which axis location a subset is pinned to.

use skrifa::raw::TableProvider;
use skrifa::raw::tables::head::MacStyle;
use skrifa::raw::tables::os2::SelectionFlags;
use skrifa::{FontRef, MetadataProvider, Tag};

/// The `{inst}` part of a face URL.
#[derive(Debug, Clone, PartialEq)]
pub enum Instance {
    /// The instance the font list shows: for an italic font the named
    /// instance whose subfamily is "Italic", otherwise (or when it has none)
    /// the one whose subfamily is "Regular", else the default location.
    List,
    /// Every axis kept.
    Var,
    /// One named instance, as axis values sorted by tag. Axes not listed stay
    /// at their default.
    Named(Vec<(Tag, f32)>),
}

/// The axis values to pin, or `None` to keep the font's variations as they
/// are (a `var` subset or a static font). An empty list pins every axis to
/// its default.
pub(crate) fn pins(font: &FontRef, instance: &Instance) -> Option<Vec<(Tag, f32)>> {
    let axes = font.axes();
    if axes.is_empty() {
        return None;
    }
    match instance {
        Instance::Var => None,
        Instance::Named(coords) => Some(coords.clone()),
        Instance::List => {
            let instances = font.named_instances();
            let subfamilies: Vec<String> = instances
                .iter()
                .map(|inst| {
                    font.localized_strings(inst.subfamily_name_id()).english_or_first().map(|s| s.to_string().trim().to_owned()).unwrap_or_default()
                })
                .collect();
            let chosen = list_instance(&subfamilies, italic_flag(font)).and_then(|i| instances.get(i));
            Some(match chosen {
                Some(inst) => axes.iter().map(|a| a.tag()).zip(inst.user_coords()).collect(),
                None => Vec::new(),
            })
        }
    }
}

/// The index of the named instance [`Instance::List`] pins, from the
/// instances' subfamily names. A font is italic when `italic_flag` says so
/// or every named instance's subfamily ends in "Italic".
fn list_instance(subfamilies: &[String], italic_flag: bool) -> Option<usize> {
    let italic = italic_flag || (!subfamilies.is_empty() && subfamilies.iter().all(|s| s.ends_with("Italic")));
    let find = |name: &str| subfamilies.iter().position(|s| s == name);
    italic.then(|| find("Italic")).flatten().or_else(|| find("Regular"))
}

/// The OS/2 fsSelection or head macStyle italic bit.
fn italic_flag(font: &FontRef) -> bool {
    font.os2().is_ok_and(|os2| os2.fs_selection().contains(SelectionFlags::ITALIC))
        || font.head().is_ok_and(|head| head.mac_style().contains(MacStyle::ITALIC))
}

#[cfg(test)]
mod tests {
    use super::list_instance;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn list_instance_prefers_italic_in_italic_fonts() {
        let upright = names(&["Thin", "Regular", "Bold", "Italic"]);
        assert_eq!(list_instance(&upright, false), Some(1));
        assert_eq!(list_instance(&upright, true), Some(3));

        let italic = names(&["Thin Italic", "Italic", "Bold Italic"]);
        assert_eq!(list_instance(&italic, false), Some(1));

        let no_italic = names(&["Light", "Regular"]);
        assert_eq!(list_instance(&no_italic, true), Some(1));
        assert_eq!(list_instance(&names(&["Light", "Bold"]), false), None);
        assert_eq!(list_instance(&[], true), None);
    }
}
