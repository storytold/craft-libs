//! Stories of the paint chip and the paint list.

use super::{GradientPreview, ImagePreview, Paint, PaintChip, PaintChipSize, PaintLink, PaintLinkKind, PaintList, PaintListOptions, PaintRow};
use craft_behavior::Mixed;
use craft_theme::Srgba;
use craft_ui::buttons::PopupMark;
use craft_ui::contract::Role;
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, Validity};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::patterns::presets::PresetState;
use craft_ui::primitives::FacePaint;
use craft_ui::runtime::env::Message;
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::Ui;

use crate::palette::{SwatchColor, SwatchEntry, SwatchGroup};
use crate::ui::SwatchKey;

/// Opens the popover with a click.
fn open_popover(fixture: &mut Fixture) {
    let chip = fixture.by_role(Role::COLOR_WELL);
    if let Some(chip) = chip.first() {
        fixture.click(chip);
    }
    fixture.idle(4);
}

/// Opens the popover and takes the last fill kind it offers.
fn take_last_kind(fixture: &mut Fixture) {
    open_popover(fixture);
    if let Some(kind) = fixture.by_role(Role::RADIO_BUTTON).last().copied() {
        fixture.click(&kind);
    }
    fixture.idle(2);
}

const BLUE: Srgba = Srgba::rgb(0.2, 0.4, 0.8);

fn chip(ui: &mut Ui, data: &mut StoryData, paint: Paint) -> StoryOutput {
    let stops = vec![(0.0, BLUE), (1.0, Srgba::rgb(1.0, 1.0, 1.0))];
    let (held, held_at) = data.value_at("paint", paint);
    PaintChip::new("Fill", held_at.bind(held), BLUE).gradient_editor(GradientPreview::new(stops), |_: &mut Ui| None).show(ui).into()
}

/// A fill, a gradient fill and a stroke with no colour, in one column; the drives use the first.
#[story(PaintChip<'static>, covers = [Default], drive = Drive::Custom(take_last_kind))]
fn paint_chip(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let teal_to_violet = GradientPreview::new(vec![(0.0, Srgba::rgb(0.07, 0.65, 0.58)), (1.0, Srgba::rgb(0.56, 0.31, 0.78))]);
    let paints = data.value("paints", [Paint::Solid(BLUE), Paint::Gradient(teal_to_violet), Paint::None]);
    let mut first = None;
    Stack::new().show(ui, |ui, _| {
        for (index, (label, paint)) in ["Fill", "Gradient fill", "Stroke"].into_iter().zip(paints.iter_mut()).enumerate() {
            let stops = vec![(0.0, BLUE), (1.0, Srgba::rgb(1.0, 1.0, 1.0))];
            let shown = PaintChip::new(label, Bind::bound(paint), BLUE).gradient_editor(GradientPreview::new(stops), |_: &mut Ui| None).show(ui);
            if index == 0 {
                first = Some(StoryOutput::from(shown));
            }
        }
    });
    first.unwrap_or_default()
}

/// Host-picker mode: a click opens nothing and reports the modifiers held.
#[story(PaintChip<'static>, covers = [], drive = Drive::Never)]
fn paint_chip_host_picker(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (held, clicks) = data.value("state", (Paint::Solid(BLUE), 0u32));
    let shown = PaintChip::new("Fill", Bind::bound(held), BLUE).popover(false).show(ui);
    if shown.clicked.is_some() {
        *clicks += 1;
    }
    Text::new(format!("Clicks: {clicks}")).shade(Shade::Muted).show(ui);
    shown.into()
}

/// The chip as a ring round an open middle, for a solid and a gradient paint.
#[story(PaintChip<'static>, covers = [], drive = Drive::Never)]
fn paint_chip_ring(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let gradient = GradientPreview::new(vec![(0.0, BLUE), (1.0, Srgba::rgb(1.0, 1.0, 1.0))]);
    let paints = data.value("paints", [Paint::Solid(BLUE), Paint::Gradient(gradient)]);
    let mut first = None;
    Stack::new().show(ui, |ui, _| {
        for (index, (label, paint)) in ["Stroke", "Gradient stroke"].into_iter().zip(paints.iter_mut()).enumerate() {
            let shown = PaintChip::new(label, Bind::bound(paint), BLUE).ring(true).show(ui);
            if index == 0 {
                first = Some(StoryOutput::from(shown));
            }
        }
    });
    first.unwrap_or_default()
}

/// Fill and Stroke chips with the menu mark: the dropdown arrow after the well, inside the chip.
#[story(PaintChip<'static>, covers = [], drive = Drive::Never)]
fn paint_chip_menu_mark(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let paints = data.value("paints", [Paint::Solid(BLUE), Paint::Solid(Srgba::rgb(0.1, 0.1, 0.1))]);
    let mut first = None;
    Stack::new().show(ui, |ui, _| {
        for (index, (label, paint)) in ["Fill", "Stroke"].into_iter().zip(paints.iter_mut()).enumerate() {
            let shown = PaintChip::new(label, Bind::bound(paint), BLUE).ring(index == 1).popup_mark(PopupMark::Menu).show(ui);
            if index == 0 {
                first = Some(StoryOutput::from(shown));
            }
        }
    });
    first.unwrap_or_default()
}

/// Compact Fill and Stroke chips in a row, as among a task bar's buttons: a small well in a control-high slot.
#[story(PaintChip<'static>, covers = [], drive = Drive::Never)]
fn paint_chip_compact(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let paints = data.value("paints", [Paint::Solid(BLUE), Paint::Solid(Srgba::rgb(0.1, 0.1, 0.1))]);
    let mut first = None;
    Row::new().show(ui, |ui, _| {
        for (index, (label, paint)) in ["Fill", "Stroke"].into_iter().zip(paints.iter_mut()).enumerate() {
            let shown = PaintChip::new(label, Bind::bound(paint), BLUE).ring(index == 1).chip_size(PaintChipSize::Compact).popover(false).show(ui);
            if index == 0 {
                first = Some(StoryOutput::from(shown));
            }
        }
    });
    first.unwrap_or_default()
}

/// A tool rail's Color, Gradient and None as swatch chips, three in a row; a click opens nothing.
#[story(PaintChip<'static>, covers = [], drive = Drive::Never)]
fn paint_chip_swatches(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let gradient = GradientPreview::new(vec![(0.0, Srgba::rgb(1.0, 1.0, 1.0)), (1.0, Srgba::rgb(0.0, 0.0, 0.0))]);
    let paints = data.value("paints", [Paint::Solid(BLUE), Paint::Gradient(gradient), Paint::None]);
    let mut first = None;
    Row::new().show(ui, |ui, _| {
        for (index, (label, paint)) in ["Color", "Gradient", "None"].into_iter().zip(paints.iter_mut()).enumerate() {
            let shown = PaintChip::new(label, Bind::bound(paint), BLUE).chip_size(PaintChipSize::Swatch).show(ui);
            if index == 0 {
                first = Some(StoryOutput::from(shown));
            }
        }
    });
    first.unwrap_or_default()
}

/// A gradient whose face is the host's painter: one coloured rect over the checker.
#[story(PaintChip<'static>, covers = [], drive = Drive::Never)]
fn paint_chip_gradient_face(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let face = FacePaint::new(1, |painter, rect| {
        painter.painter.rect_filled(rect.shrink2(egui::vec2(rect.width() / 4.0, 0.0)), 0.0, egui::Color32::from_rgb(230, 120, 30));
    });
    let preview = GradientPreview::new(vec![(0.0, BLUE), (1.0, Srgba::rgb(1.0, 1.0, 1.0))]).face(face);
    chip(ui, data, Paint::Gradient(preview))
}

#[story(PaintChip<'static>, covers = [Mixed], drive = Drive::Never)]
fn paint_chip_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (held, held_at) = data.value_at("mixed", Mixed::<Paint>::Mixed);
    PaintChip::new("Fill", held_at.bind_mixed(held), BLUE).show(ui).into()
}

#[story(PaintChip<'static>, covers = [Open], drive = Drive::Custom(open_popover))]
fn paint_chip_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    chip(ui, data, Paint::Solid(BLUE))
}

/// The Solid popover with the recent colours, two swatch folders and the Color Picker… button.
#[story(PaintChip<'static>, covers = [Open], drive = Drive::Custom(open_popover))]
fn paint_chip_open_with_recents_and_swatches(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let entry = |name: &str, rgb: [f32; 3]| SwatchEntry::new(name, SwatchColor::Rgb(rgb));
    let groups = vec![
        SwatchGroup {
            name: "Basics".to_owned(),
            entries: vec![entry("Red", [1.0, 0.0, 0.0]), entry("Green", [0.0, 0.7, 0.2]), entry("Blue", [0.1, 0.2, 0.9])],
        },
        SwatchGroup { name: "Pastels".to_owned(), entries: vec![entry("Rose", [1.0, 0.8, 0.85]), entry("Mint", [0.75, 1.0, 0.85])] },
    ];
    let recent = [BLUE, Srgba::rgb(0.9, 0.5, 0.1), Srgba::rgb(0.1, 0.6, 0.3)];
    let ((held, folders), state_at) = data.value_at("state", (Paint::Solid(BLUE), PresetState::<SwatchKey>::new()));
    PaintChip::new("Fill", state_at.bind(held), BLUE).recent(&recent).swatches(&groups, state_at.bind(folders)).full_picker(true).show(ui).into()
}

#[story(PaintChip<'static>, covers = [], drive = Drive::Never)]
fn paint_chip_no_color(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    chip(ui, data, Paint::None)
}

#[story(PaintChip<'static>, covers = [], drive = Drive::Never)]
fn paint_chip_gradient(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    chip(ui, data, Paint::Gradient(GradientPreview::new(vec![(0.0, BLUE), (0.5, Srgba::rgb(0.9, 0.8, 0.1)), (1.0, Srgba::rgb(0.9, 0.2, 0.2))])))
}

#[story(PaintChip<'static>, covers = [], drive = Drive::Never)]
fn paint_chip_pattern(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    chip(ui, data, Paint::Pattern(ImagePreview::new("Bubbles", None)))
}

#[story(PaintChip<'static>, covers = [], drive = Drive::Never)]
fn paint_chip_disabled(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (held, held_at) = data.value_at("paint", Paint::Solid(Srgba { r: 0.9, g: 0.2, b: 0.2, a: 0.6 }));
    PaintChip::new("Fill", held_at.bind(held), BLUE).enabled(false).show(ui).into()
}

fn paints() -> Vec<(String, Paint)> {
    vec![
        ("Brand blue".to_owned(), Paint::Solid(BLUE)),
        ("Sky".to_owned(), Paint::Solid(Srgba { r: 0.5, g: 0.7, b: 1.0, a: 0.5 })),
        ("Sunset".to_owned(), Paint::Gradient(GradientPreview::new(vec![(0.0, Srgba::rgb(0.9, 0.5, 0.1)), (1.0, Srgba::rgb(0.6, 0.1, 0.4))]))),
        ("Nothing".to_owned(), Paint::None),
    ]
}

#[story(PaintList<'static>, covers = [Default], drive = Drive::Never)]
fn paint_list(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let paints = paints();
    PaintList::new("Paints", &paints, data.bind("selected", None::<usize>)).show(ui).into()
}

#[story(PaintList<'static>, covers = [], drive = Drive::Never)]
fn paint_list_selection_colors(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let paints = paints();
    let rows = [PaintRow::linked(PaintLink::new("Primary / Blue", PaintLinkKind::Style))];
    PaintList::new("Selection colors", &paints, data.bind("selected", Some(0usize)))
        .options(PaintListOptions { select_users: true, bind: true, remove: true, add: true, ..PaintListOptions::SELECTION_COLORS })
        .rows(&rows)
        .show(ui)
        .into()
}

#[story(PaintChip<'static>, covers = [Invalid], drive = Drive::Never)]
fn paint_chip_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_none = |paint: &Paint| if matches!(paint, Paint::None) { Validity::Refuse(Message::new("A fill is required")) } else { Validity::Ok };
    let held = data.bind("paint", Paint::None).check(refuse_none);
    PaintChip::new("Fill", held, BLUE).show(ui).into()
}
