//! The inspector: while the Inspect panel is in front, the selected card's stories are shown as inspected regions
//! with the inspector's drawing over each, and the panel lists what each region showed.

use super::overview::{SpecStory, selected_type, target_story};
use super::panels::Panels;
use crate::viewer::PanelId;
use craft_theme::variant::Tone;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Pad, Props as _, Scroll, Show as _, ShowContainer as _};
use craft_ui::display::{InspectLayers, Shade, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::runtime::env::Env;
use craft_ui::runtime::inspect::{self, Element, Recorded};
use craft_ui::sections::Section;
use craft_ui::style::TypeRole;
use egui::{Id, Ui};

/// What the inspector shows in a pass.
#[non_exhaustive]
pub struct Inspect<'a> {
    /// The Inspect panel is in front in the components workspace, with no Variants view open: the selected card's
    /// stories are recorded and drawn over.
    pub on: bool,
    /// The centre tabs in front whose selected card is inspected: the Overview, the selected component's tab.
    pub front: Vec<PanelId>,
    /// The drawings shown.
    pub layers: &'a mut InspectLayers,
}

impl Inspect<'_> {
    /// What the selected card of `tab` inspects, when the inspector is on and the tab is in front.
    pub fn of(&self, tab: Option<PanelId>, selected: bool) -> Option<Inspected> {
        let tab = tab.filter(|tab| self.on && selected && self.front.contains(tab))?;
        Some(Inspected { tab, layers: *self.layers })
    }
}

/// How a card's stories are inspected: the tab the card is in, which keys their regions, and the drawings shown.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Inspected {
    /// The centre tab the card is in.
    pub tab: PanelId,
    /// The drawings shown over each story.
    pub layers: InspectLayers,
}

/// The inspected region of the story at catalog index `story` on the selected card of `tab`.
pub fn region_key(tab: PanelId, story: usize) -> Id {
    Id::NULL.with("gallery.inspect").with(tab).with(story)
}

/// The Inspect panel: the drawing switches, then for each front tab one section per story of the selected card with the
/// elements its region showed, then the selected component's declared facts.
pub(super) fn panel(ui: &mut Ui, p: &mut Panels<'_>) {
    let sections = regions(p);
    let least = Env::of(ui).tokens.size.control.sm;
    let (catalog, selected) = (p.catalog, selected_type(p));
    let layers = &mut *p.inspect.layers;
    Stack::new().fill().pad(Pad::Regular).scroll(Scroll::vertical()).show(ui, |ui, _| {
        Checkbox::new(Bind::bound(&mut layers.padding)).label("Padding").show(ui);
        Checkbox::new(Bind::bound(&mut layers.hits)).label("Hit areas").show(ui);
        Checkbox::new(Bind::bound(&mut layers.focus)).label("Focus order").show(ui);
        Text::new(format!("Hit areas under {least} pt (size.control.sm) are flagged.")).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
        for (at, (key, caption)) in sections.iter().enumerate() {
            Section::new(caption.clone()).id_salt(*key).initially_open(at == 0).show(ui, |ui, _| {
                Stack::new().fill_width().show(ui, |ui, _| match inspect::recorded(ui.ctx(), *key) {
                    Some(recorded) => rows(ui, *key, &recorded),
                    None => {
                        Text::new("The story has not run yet.").shade(Shade::Faint).show(ui);
                    }
                });
            });
        }
        super::panels::declared(ui, catalog, selected);
    });
}

/// The regions the panel lists, with their captions: the stories of the selected card of each front tab.
fn regions(p: &Panels<'_>) -> Vec<(Id, String)> {
    let Some(type_name) = selected_type(p).filter(|_| p.inspect.on) else { return Vec::new() };
    let (Some(component), Some(target)) = (p.catalog.component(type_name), target_story(p, type_name)) else { return Vec::new() };
    let Some(group) = p.catalog.group_at(type_name, target) else { return Vec::new() };
    let stories = p.catalog.group_stories(type_name, group);
    let tabs = p.inspect.front.iter().filter(|tab| matches!(tab, PanelId::Overview) || **tab == PanelId::Component(type_name));
    tabs.flat_map(|tab| {
        stories.iter().map(|&index| (region_key(*tab, index), SpecStory::of(p.catalog, index, &component.name).caption)).collect::<Vec<_>>()
    })
    .collect()
}

/// One row per element of `recorded` with a node or a focus stop, indented by the shown elements it sits in; an element
/// with neither is left out and what it shows takes its place. The row under the pointer points the region's drawing at
/// its element.
fn rows(ui: &mut Ui, key: Id, recorded: &Recorded) {
    let mut pointed = None;
    // Whether the element at each depth of the current path has a row.
    let mut path: Vec<bool> = Vec::new();
    for (index, element) in recorded.elements.iter().enumerate() {
        path.truncate(element.depth);
        let shown = element.facts.is_some() || element.stop.is_some();
        let indent = path.iter().filter(|shown| **shown).count();
        path.push(shown);
        if !shown {
            continue;
        }
        let row = Row::new().wrap(true).hover().indent(u8::try_from(indent).unwrap_or(u8::MAX)).show(ui, |ui, _| element_row(ui, element));
        if row.output.hovered() {
            pointed = Some(index);
        }
    }
    inspect::point_at(ui.ctx(), key, pointed);
}

/// The words of one element's row, as the old inspector wrote a node: its number in Tab order, role, name, value,
/// actions, state, and a warning for an interactive node without a name.
fn element_row(ui: &mut Ui, element: &Element) {
    if let Some(stop) = element.stop {
        Text::new(format!("{stop}.")).show(ui);
    }
    let Some(facts) = &element.facts else {
        Text::new(element.component).show(ui);
        return;
    };
    Text::new(facts.role.clone()).show(ui);
    // A text's name is its text, which reads as its value.
    let label = facts.role == "Label";
    if let Some(name) = facts.name.as_ref().filter(|_| !label) {
        Text::new(format!("\"{name}\"")).shade(Shade::Muted).show(ui);
    }
    if let Some(value) = facts.value.as_ref().or(facts.name.as_ref().filter(|_| label)) {
        Text::new(format!("= {value}")).shade(Shade::Muted).italic().show(ui);
    }
    if !facts.actions.is_empty() {
        Text::new(facts.actions.join(", ")).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
    }
    if facts.disabled {
        Text::new("disabled").role(TypeRole::Caption).shade(Shade::Muted).show(ui);
    }
    if facts.interactive && facts.name.is_none() {
        Text::new("no label").tone(Tone::Warn).show(ui);
    }
}
