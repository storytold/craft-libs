//! The Variants view: the target story repeated over one axis of the appearance, a card for each value, in place of the
//! front centre tab's content.

use super::card::Look;
use super::overview::{Spec, SpecStory, apply, grid, on_background, selected_type, target_story};
use super::panels::Panels;
use crate::viewer::{DIRECTIONS, MODES, PanelId, VariantAxis, entry_of};
use craft_i18n::Direction;
use craft_theme::variant::Tone;
use craft_theme::{ControlSize, Mode, ModeChoice};
use craft_ui::buttons::Button;
use craft_ui::contract::{Show as _, ShowContainer as _, StoryLayout};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::Row;
use craft_ui::notices::Notice;
use craft_ui::style::TypeRole;
use egui::Ui;

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

/// Whether the Variants view is open in `tab`: it opens in the tab that is in front when it is asked for and closes
/// when another one comes to the front.
fn open_in(p: &mut Panels<'_>, tab: PanelId) -> bool {
    if p.variants.is_none() {
        *p.variants_tab = None;
        return false;
    }
    match *p.variants_tab {
        None => {
            *p.variants_tab = Some(tab);
            true
        }
        Some(owner) if owner == tab => true,
        Some(_) => {
            *p.variants = None;
            *p.variants_tab = None;
            false
        }
    }
}

/// Shows the Variants view in `tab` when one is open there. Returns whether it did.
pub(super) fn show(ui: &mut Ui, p: &mut Panels<'_>, tab: PanelId) -> bool {
    if !open_in(p, tab) {
        return false;
    }
    let Some(axis) = *p.variants else { return false };
    let mut closed = false;
    let target = selected_type(p).and_then(|type_name| target_story(p, type_name).map(|index| (type_name, index)));
    let cards: Vec<Spec> = match (target, values(axis)) {
        (Some((type_name, index)), Some(values)) => {
            let Some(entry) = entry_of(type_name) else { return false };
            let (mode, direction) = (p.env.appearance(ui.ctx()).mode, p.env.direction);
            let shown = |look: &Look| match axis {
                VariantAxis::Mode => look.mode == Some(mode),
                VariantAxis::Direction => look.direction == Some(direction),
                _ => false,
            };
            values
                .iter()
                .map(|value| Spec {
                    key: format!("variant/{}/{}", noun(axis), value.label),
                    type_name,
                    title: value.label.to_owned(),
                    note: shown(&value.look).then(|| "shown now".to_owned()),
                    stories: vec![SpecStory::of(p, index, &entry.name)],
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
        Some(format!("{} · {} in every {}", entry.name, p.catalog.entries[index].registration.story.caption(&entry.name), noun(axis)))
    });
    let mut reported = Vec::new();
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
            reported = grid(ui, "variants", &cards, p, false);
        }
        closed |= cx.take_cancel();
    });
    let used = reported.iter().find(|reported| reported.events.used).map(|reported| reported.spec);
    apply(ui, p, &cards, reported);
    if let Some(card) = used.and_then(|spec| values(axis).and_then(|values| values.into_iter().nth(spec))) {
        if let Some(mode) = card.look.mode {
            p.env.mode = mode.into();
        }
        if let Some(direction) = card.look.direction {
            p.env.direction = direction;
        }
        closed = true;
    }
    if closed {
        *p.variants = None;
    }
    true
}
