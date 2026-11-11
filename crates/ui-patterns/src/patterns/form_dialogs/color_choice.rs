//! The colour row of the dialogs that offer a named list and Custom…: Guides and Navigator Panel Options.

use super::HostRequests;
use crate::contract::{Bind, Entry, Show as _, ShowContainer as _};
use crate::runtime::requests::{ColorRequest, HostAnswer};

use crate::form::Field;
use crate::pickers::Select;
use crate::runtime::env::Label;
use craft_theme::Srgba;
use egui::Ui;

/// A colour a dialog picks from a named list or sets with Custom…: a small enum whose custom variant carries its colour,
/// so the value alone says which colour is chosen.
pub(super) trait ColorChoice: Copy + PartialEq + Send + Sync + 'static {
    /// The named colours in the order the list shows them.
    const NAMED: &'static [Self];
    /// Whether Custom… leads the list; otherwise it follows the named colours.
    const CUSTOM_FIRST: bool;

    /// The custom colour for a picker's answer.
    fn from_picked(color: Srgba) -> Self;

    /// Whether this is the custom variant.
    fn is_custom(self) -> bool;

    /// The colour shown and the one the picker opens at.
    fn srgba(self) -> Srgba;
}

/// What the colour row shows.
pub(super) struct ColorRowTexts {
    /// The row's label.
    pub(super) label: Label,
    /// The title of the host's colour picker.
    pub(super) picker_title: Label,
}

/// The colour row: the named colours and Custom…, each with a swatch. Picking a named colour sets it. Every pick of
/// Custom…, also while a custom colour is selected, files a colour request under `key` that opens the picker at the
/// colour shown; the colour stays as it is until the host answers, and a cancelled pick changes nothing.
pub(super) fn color_row<C: ColorChoice>(
    ui: &mut Ui,
    texts: &ColorRowTexts,
    color: &mut C,
    key: &str,
    requests: &mut HostRequests,
    name: impl Fn(C) -> Label,
) {
    let current = *color;
    let custom = if current.is_custom() { current } else { C::from_picked(current.srgba()) };
    let entry = |value: C| Entry::new(value, name(value)).swatch(value.srgba());
    let mut entries: Vec<Entry<C>> = C::NAMED.iter().copied().map(&entry).collect();
    if C::CUSTOM_FIRST {
        entries.insert(0, entry(custom));
    } else {
        entries.push(entry(custom));
    }
    Field::new(texts.label.clone()).show(ui, |ui, _| {
        match Select::new(texts.label.clone(), Bind::given(current), &entries).show(ui).value().copied() {
            Some(chosen) if chosen.is_custom() => {
                requests.request_color(ColorRequest::new(key, chosen.srgba()).title(texts.picker_title.clone()));
            }
            Some(named) => *color = named,
            None => {}
        }
    });
}

/// Sets `color` from the host's answers to the colour request `key`.
pub(super) fn apply_color_answers<C: ColorChoice>(answers: impl IntoIterator<Item = HostAnswer>, key: &str, color: &mut C) {
    for answer in answers {
        if let HostAnswer::Color { key: answered, color: picked } = answer
            && answered == key
        {
            *color = C::from_picked(picked);
        }
    }
}
