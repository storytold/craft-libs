//! The Variants view: the target story repeated over one axis of the appearance, a card for each value, in place of the
//! front centre tab's content.

use super::card::Look;
use super::overview::{Centre, Spec, SpecStory, apply, grid, on_background};
use super::panels::{selected_of, target_story};
use crate::viewer::{DIRECTIONS, MODES, SETS, VariantAxis, entry_of};
use craft_i18n::Direction;
use craft_theme::variant::Tone;
use craft_theme::{ControlSize, Mode, ModeChoice};
use craft_ui::buttons::Button;
use craft_ui::contract::{MemoProps, Show as _, ShowContainer as _, StoryLayout};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::Row;
use craft_ui::notices::Notice;
use craft_ui::style::TypeRole;
use egui::Ui;

/// The Variants view open in a centre tab: its axis, and the window's look, which the card that shows it now notes.
#[derive(Clone, Copy, Debug, PartialEq, MemoProps)]
#[non_exhaustive]
pub struct VariantsIn {
    /// What the view repeats the story over.
    pub axis: VariantAxis,
    /// The window's mode, direction and component set.
    pub shown: Look,
}

impl VariantsIn {
    /// The view of `axis` under the window's `shown` look.
    pub fn new(axis: VariantAxis, shown: Look) -> Self {
        VariantsIn { axis, shown }
    }
}

/// What the Variants view asks of the viewer.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum VariantsEvent {
    /// Close the view.
    Close,
    /// Make a card's look the window's, and close the view.
    Use(Look),
}

/// One value of an axis: its name, the look a card shows it with, and what picking it sets.
struct Value {
    label: &'static str,
    look: Look,
}

/// The words after "in every" in the view's heading.
fn noun(axis: VariantAxis) -> &'static str {
    match axis {
        VariantAxis::Theme => "theme",
        VariantAxis::Mode => "mode",
        VariantAxis::Contrast => "contrast",
        VariantAxis::Size => "size",
        VariantAxis::Set => "set",
        VariantAxis::Direction => "direction",
        VariantAxis::ControlSize => "control size",
        VariantAxis::DirectionAndControlSize => "direction and control size",
    }
}

/// The values of `axis`, or `None` for an axis a card cannot be shown under: the theme, contrast and density of a
/// region are the window's (a theme scope sets the mode only).
fn values(axis: VariantAxis) -> Option<Vec<Value>> {
    let direction = |direction: Direction| Some(direction);
    let sizes = [(ControlSize::Sm, "Sm"), (ControlSize::Md, "Md")];
    match axis {
        VariantAxis::Mode => {
            // The two modes a card can show; System is the window's own choice.
            let mode_of = |choice: ModeChoice| match choice {
                ModeChoice::Light => Some(Mode::Light),
                ModeChoice::Dark => Some(Mode::Dark),
                _ => None,
            };
            Some(
                MODES
                    .0
                    .iter()
                    .filter_map(|choice| {
                        mode_of(choice.value).map(|mode| Value { label: choice.label, look: Look { mode: Some(mode), ..Look::default() } })
                    })
                    .collect(),
            )
        }
        VariantAxis::Direction => Some(
            DIRECTIONS
                .0
                .iter()
                .map(|choice| Value { label: choice.label, look: Look { direction: direction(choice.value), ..Look::default() } })
                .collect(),
        ),
        VariantAxis::Set => {
            Some(SETS.0.iter().map(|choice| Value { label: choice.label, look: Look { set: Some(choice.value), ..Look::default() } }).collect())
        }
        VariantAxis::ControlSize => {
            Some(sizes.iter().map(|(size, label)| Value { label, look: Look { size: Some(*size), ..Look::default() } }).collect())
        }
        VariantAxis::DirectionAndControlSize => Some(
            DIRECTIONS
                .0
                .iter()
                .flat_map(|choice| {
                    sizes.iter().map(move |(size, _)| Value {
                        label: if choice.value.is_rtl() {
                            if *size == ControlSize::Sm { "RTL, Sm" } else { "RTL, Md" }
                        } else if *size == ControlSize::Sm {
                            "LTR, Sm"
                        } else {
                            "LTR, Md"
                        },
                        look: Look { direction: Some(choice.value), size: Some(*size), ..Look::default() },
                    })
                })
                .collect(),
        ),
        _ => None,
    }
}

/// Shows the Variants view when one is open in the tab `c` shows. Returns whether it did. Close and Use leave through
/// `c.variants_out`.
pub(super) fn show(ui: &mut Ui, c: &mut Centre<'_>) -> bool {
    let Some(VariantsIn { axis, shown: window }) = c.variants else { return false };
    let mut closed = false;
    let target = selected_of(c.selection).and_then(|type_name| target_story(c.catalog, c.story_of, type_name).map(|index| (type_name, index)));
    let cards: Vec<Spec> = match (target, values(axis)) {
        (Some((type_name, index)), Some(values)) => {
            let Some(entry) = entry_of(type_name) else { return false };
            let shown = |look: &Look| match axis {
                VariantAxis::Mode => look.mode == window.mode,
                VariantAxis::Direction => look.direction == window.direction,
                VariantAxis::Set => look.set == window.set,
                _ => false,
            };
            values
                .iter()
                .map(|value| Spec {
                    key: format!("variant/{}/{}", noun(axis), value.label),
                    type_name,
                    title: value.label.to_owned(),
                    note: shown(&value.look).then(|| "shown now".to_owned()),
                    stories: vec![SpecStory::of(c.catalog, index, &entry.name)],
                    layout: StoryLayout::Stacked,
                    surface: entry.surface,
                    open: false,
                    usable: true,
                    selected: false,
                    look: value.look,
                    gated: false,
                })
                .collect()
        }
        _ => Vec::new(),
    };
    let heading = target.and_then(|(type_name, index)| {
        let entry = entry_of(type_name)?;
        Some(format!("{} · {} in every {}", entry.name, c.catalog.entries[index].registration.story.caption(&entry.name), noun(axis)))
    });
    let mut reported = Vec::new();
    let generation = c.generation;
    on_background(ui, "variants", |ui, cx| {
        Row::new().show(ui, |ui, _| {
            Text::new(heading.clone().unwrap_or_else(|| "Variants".to_owned())).role(TypeRole::Heading).show(ui);
            Text::new("Use one to set it").shade(Shade::Muted).show(ui);
            closed |= Button::new("Close").show(ui).clicks > 0;
        });
        if cards.is_empty() {
            let text = if target.is_none() {
                "Select a component to see its variants."
            } else {
                "Cards under this axis need a theme scope the framework does not have yet."
            };
            let _ = Notice::new(Tone::Info, text).show(ui);
        } else {
            reported = grid(ui, "variants", None, &cards, c.cells, generation, false, None);
        }
        closed |= cx.take_cancel();
    });
    let used = reported.iter().find(|reported| reported.events.used).map(|reported| reported.spec);
    apply(c, &cards, reported);
    if let Some(card) = used.and_then(|spec| values(axis).and_then(|values| values.into_iter().nth(spec))) {
        c.variants_out.send(VariantsEvent::Use(card.look));
    } else if closed {
        c.variants_out.send(VariantsEvent::Close);
    }
    true
}
