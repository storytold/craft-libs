//! Stories of `PresetBrowser`.

use super::{PresetBrowser, PresetDrop, PresetReorder};
use crate::choice::SwatchFace;
use crate::contract::{Bind, DropAt, DropPlace, Entry, ListDrag, SelectMode};
use crate::list::ThumbAspect;
use crate::patterns::presets::{
    CellShape, GroupHeads, PresetEvent, PresetFns, PresetFooterOptions, PresetRow, PresetRowMenu, PresetSource, PresetState, PresetTiles, PresetView,
};
use crate::primitives::{FacePaint, IconSpec};
use crate::runtime::env::{Env, Label};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::{IconId, Selection, SelectionModel};
use craft_theme::Srgba;
use egui::Ui;
use egui::ecolor::{Color32, Hsva};

/// The deepest level a group sits at: the Standard folders.
const NESTING: usize = 2;

/// The swatch colour of preset `n`: hues spread round the wheel, three saturations.
fn swatch(n: u32) -> Srgba {
    let saturation = 0.55 + 0.3 * ((n % 3) as f32 / 2.0);
    Srgba::from(Color32::from(Hsva::new((n as f32 * 0.137).fract(), saturation, 0.85, 1.0)))
}

/// A library of 28 swatches: two loose ones, then Warm and Cool, each with three swatches and the folders Pastel and
/// Deep, which hold three swatches and a Standard folder of two. Presets are keyed 1 to 28, groups from 100 on. No
/// preset has a caption, so the grid shows bare tiles.
fn library_rows(_: &(), _: &Env) -> Vec<PresetRow<u32>> {
    let mut rows = Vec::new();
    let mut n = 0;
    let mut group = 100;
    let mut fill = |rows: &mut Vec<PresetRow<u32>>, depth: usize, count: u32| {
        for _ in 0..count {
            n += 1;
            let color = swatch(n);
            rows.push(
                PresetRow::item(n, format!("Preset {n}"), depth)
                    .preview_face(ThumbAspect::SQUARE, FacePaint::new(n.into(), move |cx, rect| SwatchFace::new(color).paint(cx, rect))),
            );
        }
    };
    let mut open = |rows: &mut Vec<PresetRow<u32>>, name: &str, depth: usize, count: usize| {
        group += 1;
        rows.push(PresetRow::group(group, name, depth).count(count));
    };
    fill(&mut rows, 0, 2);
    for top in ["Warm", "Cool"] {
        open(&mut rows, top, 0, 13);
        fill(&mut rows, 1, 3);
        for inner in ["Pastel", "Deep"] {
            open(&mut rows, inner, 1, 5);
            fill(&mut rows, 2, 3);
            open(&mut rows, "Standard", 2, 2);
            fill(&mut rows, 3, 2);
        }
    }
    rows
}

#[story(PresetBrowser<'static, u32>, covers = [], drive = Drive::Never)]
fn preset_browser(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = PresetSource::new((), PresetFns::new(library_rows));
    let slider = *data.value("slider", true);
    let new_group = *data.value("new_group", true);
    let selected = *data.value("selected", None::<u32>);
    let mut browser = PresetBrowser::new("Swatches", data.bind("state", PresetState::<u32>::new()), &rows)
        .toolbar(true)
        .reorder(PresetReorder::items_and_groups().nest(NESTING))
        .footer(PresetFooterOptions::new().slider(slider).new_group(new_group).can_delete(selected.is_some()));
    if let Some(key) = selected {
        browser = browser.selection(Bind::given(Selection::single(key)));
    }
    let out = browser.show(ui);
    let selected = data.value("selected", None::<u32>);
    for event in &out.events {
        if let PresetEvent::Select { key, .. } | PresetEvent::Activate { key, .. } = event {
            *selected = Some(*key);
        }
    }
    let readout = match *selected {
        Some(n) => format!("Selected: Preset {n}"),
        None => String::from("Nothing selected."),
    };
    StoryOutput::from(out).readout(readout)
}

/// Three presets selected under `SelectMode::Multi`, in both views: Shift extends from the anchor, Cmd/Ctrl toggles.
#[story(PresetBrowser<'static, u32>, covers = [], drive = Drive::Never)]
fn preset_browser_multi(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = PresetSource::new((), PresetFns::new(library_rows));
    let shown = data.value("selection", Selection::new(vec![1, 2, 3], Some(3), Some(1))).clone();
    let out = PresetBrowser::new("Swatches", data.bind("state", PresetState::<u32>::new()), &rows)
        .toolbar(true)
        .select(SelectMode::Multi)
        .selection(Bind::given(shown))
        .show(ui);
    let held = data.value("selection", Selection::<u32>::default());
    for event in &out.events {
        if let PresetEvent::Selection(change) = event {
            held.apply(change.clone());
        }
    }
    let readout = format!("Selected: {} presets", held.items.len());
    StoryOutput::from(out).readout(readout)
}

/// Group heads as full-width rules between the groups, over wide chips, with no row menu.
#[story(PresetBrowser<'static, u32>, covers = [], drive = Drive::Never)]
fn preset_browser_rule_groups(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = PresetSource::new((), PresetFns::new(library_rows));
    PresetBrowser::new("Gradients", data.bind("state", PresetState::<u32>::new()), &rows)
        .tiles(PresetTiles::Chips)
        .cell_shape(CellShape::ratio(64, 28))
        .group_heads(GroupHeads::Rule)
        .row_menu(PresetRowMenu::Off)
        .show(ui)
        .into()
}

/// A drop target on the presets (a preset dragged onto another), and a row menu of the host's own rows.
// PLAN: the sheet's `DropAction` around the browser and `FooterBar` in a `Panel` footer are not shown: their builders are not in this set's bundle.
#[story(PresetBrowser<'static, u32>, covers = [], drive = Drive::Never)]
fn preset_browser_drops(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = PresetSource::new((), PresetFns::new(library_rows));
    let out = PresetBrowser::new("Swatches", data.bind("state", PresetState::<u32>::new()), &rows)
        .view_lock(PresetView::List)
        .drags(true)
        .takes_drop(PresetDrop::new::<ListDrag<u32>>(DropPlace::Onto, |drag, at| matches!(at, DropAt::Onto(key) if *key != drag.grabbed)))
        .row_menu(PresetRowMenu::Host)
        .row_items(|key| {
            vec![
                Entry::new(String::from("use"), Label::from(format!("Use Preset {key}"))).checked(*key == 1),
                Entry::new(String::from("lock"), Label::from(String::from("Lock")))
                    .enabled(false)
                    .disabled_reason(Label::from(String::from("Already locked"))),
            ]
        })
        .show(ui);
    let dropped = data.value("dropped", String::new());
    for event in &out.events {
        match event {
            PresetEvent::Dropped { at, .. } => *dropped = format!("Dropped {at:?}"),
            PresetEvent::Menu { target, command } => *dropped = format!("Menu {command} on Preset {target}"),
            _ => {}
        }
    }
    let readout = if dropped.is_empty() { String::from("Nothing dropped.") } else { dropped.clone() };
    StoryOutput::from(out).readout(readout)
}

/// The library with a lock icon after every fifth preset's label.
fn locked_rows(inputs: &(), env: &Env) -> Vec<PresetRow<u32>> {
    let mut rows = library_rows(inputs, env);
    for row in rows.iter_mut().filter(|row| !row.is_group() && row.key % 5 == 0) {
        row.trailing.push((IconSpec::svg(IconId::new("lock")), String::from("Locked")));
    }
    rows
}

/// Trailing icons after a row's label in the list view, and a preset scrolled into view once.
#[story(PresetBrowser<'static, u32>, covers = [], drive = Drive::Never)]
fn preset_browser_trailing_and_reveal(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = PresetSource::new((), PresetFns::new(locked_rows));
    PresetBrowser::new("Swatches", data.bind("state", PresetState::<u32>::new()), &rows)
        .view_lock(PresetView::List)
        .max_rows(8)
        .reveal(28)
        .show(ui)
        .into()
}

/// The chips variant: the same library as small bare chips under text-high group names, as a palette in a popover.
#[story(PresetBrowser<'static, u32>, covers = [], drive = Drive::Never)]
fn preset_browser_chips(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = PresetSource::new((), PresetFns::new(library_rows));
    let selected = *data.value("selected", None::<u32>);
    let mut browser = PresetBrowser::new("Swatches", data.bind("state", PresetState::<u32>::new()), &rows).tiles(PresetTiles::Chips);
    if let Some(key) = selected {
        browser = browser.selection(Bind::given(Selection::single(key)));
    }
    let out = browser.show(ui);
    let selected = data.value("selected", None::<u32>);
    for event in &out.events {
        if let PresetEvent::Select { key, .. } | PresetEvent::Activate { key, .. } = event {
            *selected = Some(*key);
        }
    }
    let readout = match *selected {
        Some(n) => format!("Selected: Preset {n}"),
        None => String::from("Nothing selected."),
    };
    StoryOutput::from(out).readout(readout)
}
