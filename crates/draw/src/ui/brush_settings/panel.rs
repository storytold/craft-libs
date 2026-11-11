//! The Brush Settings panel frame ([`BrushSettingsPanel`]).

use craft_behavior::{ColumnWidth, IconId, Mixed, Value};
use craft_ui::buttons::IconButton;
use craft_ui::contract::{Bind, BodyCx, Outcome, Props as _, Scroll, Show as _, ShowContainer as _, clicked};
use craft_ui::display::Divider;
use craft_ui::layout_components::{Grid, Row, Stack};
use craft_ui::runtime::env::{Env, Label};
use egui::{Response, Ui};
use std::hash::Hash;

use super::preview::BrushStrokeStrip;
use super::sections::{BrushSection, BrushSectionEvent, BrushSectionKind, BrushSectionList};
use super::words::BrushSettingsWords;

/// The strip takes the room the buttons leave.
const BOTTOM: &[ColumnWidth] = &[ColumnWidth::fill(1.0), ColumnWidth::Content];

/// The buttons beside the stroke strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BrushPanelAction {
    /// "Create new brush from these settings": the app opens its New Brush Preset dialog.
    NewPreset,
    /// "Reset the brush to the defaults": the app resets the brush, keeping its colours,
    /// smoothing and locks (PhotoCraft), as one preset command.
    Reset,
    /// "Update the current brush with these settings" (enabled with
    /// [`can_update`](BrushSettingsPanel::can_update)): the app overwrites the current preset.
    UpdatePreset,
}

/// What [`BrushSettingsPanel::show`] returns.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the panel's events"]
pub struct BrushSettingsPanelOutput<K, R> {
    /// The panel.
    pub response: Response,
    /// The section list's events, in order.
    pub sections: Vec<BrushSectionEvent<K>>,
    /// A button beside the strip, if one was pressed.
    pub action: Option<BrushPanelAction>,
    /// What the pane closure returned; `None` when no pane section exists.
    pub body: Option<R>,
}

impl<K, R> Outcome for BrushSettingsPanelOutput<K, R> {
    type Value = craft_ui::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Photoshop's Brush Settings panel: the [`BrushSectionList`] at the reading start, a vertical line, the shown
/// section's pane (the app's content) in a scrolling body, a hairline, then the [`BrushStrokeStrip`] with the New,
/// Reset and Update icon buttons at the reading end. An ordinary dockable panel.
///
/// - **Pane.** The `pane` closure (a body closure that also gets the key of the section shown) draws the shown pane section's controls, with the shared rows
///   ([`BrushSizeField`](super::BrushSizeField), [`TipEllipse`](super::TipEllipse),
///   [`DynamicControlRow`](super::DynamicControlRow)) and the library's controls. A section that is off
///   ([`BrushSection::is_off`]) runs it disabled (greyed). One scroll position per section. No title: the selected
///   row names the pane.
/// - **Edits.** The panel makes none. The list's events and the actions go to the app; the controls in the pane
///   report their own edits, which the app sends to its tool options host: brush settings never become history
///   states.
/// - **Keys.** The list, the pane's controls, the buttons, in Tab order.
/// - **Height.** The pane takes the height the container offers less the bottom row.
/// - **RTL.** Mirrors: the list at the right, the buttons left of the strip.
#[must_use = "show the panel with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
#[pattern(probe(BrushSettingsPanel<'static, u8>))]
pub struct BrushSettingsPanel<'a, K: Value + Eq + Hash + std::fmt::Debug> {
    label: Label,
    sections: &'a [BrushSection<K>],
    shown: Bind<'a, K>,
    /// Shows the list's "Brushes" link row.
    #[prop(flag)]
    link_row: bool,
    /// Enables Update (the brush was picked from a preset that still exists).
    #[prop(flag)]
    can_update: bool,
    /// The stroke strip beside the buttons; without one the bottom row has the buttons only.
    #[prop]
    strip: Option<BrushStrokeStrip<'a>>,
}

impl<'a, K: Value + Eq + Hash + std::fmt::Debug> BrushSettingsPanel<'a, K> {
    /// A panel named `label` over `sections`, showing the pane of the section in `shown` (the first pane section
    /// when it holds none).
    pub fn new(label: impl Into<Label>, sections: &'a [BrushSection<K>], shown: Bind<'a, K>) -> Self {
        Self { label: label.into(), sections, shown, link_row: false, can_update: false, strip: None }
    }

    /// Shows the panel; `pane` draws the pane of the section it is given and its result comes back in
    /// [`BrushSettingsPanelOutput::body`]. A pass may run it more than once.
    pub fn show<R>(self, ui: &mut Ui, mut pane: impl FnMut(&mut Ui, &mut BodyCx<'_>, &K) -> R) -> BrushSettingsPanelOutput<K, R> {
        let BrushSettingsPanel { label, sections, mut shown, link_row, can_update, strip } = self;
        let words = Env::of(ui).words::<BrushSettingsWords>();
        let mut section_events = Vec::new();
        let mut action = None;
        let mut result = None;
        let bottom = |ui: &mut Ui, _: &mut BodyCx<'_>| {
            Divider::horizontal().show(ui);
            Grid::new(BOTTOM).show(ui, |ui, _| {
                if let Some(strip) = strip.clone() {
                    strip.show(ui);
                }
                Stack::new().show(ui, |ui, _| {
                    let button = |ui: &mut Ui, name: &Label, icon: &'static str, enabled: bool, which: BrushPanelAction| {
                        let response = IconButton::new(IconId::new(icon)).label(name.clone()).tooltip(name.clone()).enabled(enabled).show(ui);
                        clicked(&response).then_some(which)
                    };
                    let pressed = [
                        button(ui, &words.new_preset, "square_plus", true, BrushPanelAction::NewPreset),
                        button(ui, &words.reset, "undo_2", true, BrushPanelAction::Reset),
                        button(ui, &words.update, "check", can_update, BrushPanelAction::UpdatePreset),
                    ];
                    action = pressed.into_iter().flatten().next();
                });
            });
        };
        let out = Stack::new().label(label).fill().footer(bottom).show(ui, |ui, cx| {
            let held = match shown.reborrow().read(cx) {
                Some(Mixed::Same(held)) => Some(held),
                _ => None,
            };
            let is_pane = |section: &&BrushSection<K>| section.kind == BrushSectionKind::Pane;
            let current =
                held.and_then(|held| sections.iter().filter(is_pane).find(|section| section.key == held)).or_else(|| sections.iter().find(is_pane));
            let Some(first) = sections.first() else { return };
            Row::new().fill().show(ui, |ui, _| {
                let key = current.unwrap_or(first).key.clone();
                let mut list = BrushSectionList::new(words.sections.clone(), sections, Bind::given(key));
                if link_row {
                    list = list.link_row();
                }
                section_events = list.show(ui).events;
                Divider::vertical().show(ui);
                let Some(section) = current else { return };
                result = Stack::new()
                    .id_salt(&section.key)
                    .fill()
                    .scroll(Scroll::vertical())
                    .enabled(!section.is_off())
                    .show(ui, |ui, cx| pane(ui, cx, &section.key))
                    .inner;
            });
        });
        BrushSettingsPanelOutput { response: out.output, sections: section_events, action, body: result }
    }
}
