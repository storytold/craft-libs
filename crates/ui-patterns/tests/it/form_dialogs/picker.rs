//! A picker form dialog: a category list, a grid of tiles a few rows high and a large preview with its name and a
//! sentence about it, side by side (WordCraft's Insert SmartArt). Opening it settles in a frame or two.

use craft_behavior::{Selection, keyed};
use craft_ui::contract::{Bind, BodyCx, PaneEdge, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft_ui::display::{Picture, PictureSize, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::{List, ListRow, ThumbAspect, ThumbSize};
use craft_ui::patterns::dock::{PaneArea, PaneContent, PaneSpec, PaneState};
use craft_ui::patterns::form_dialogs::FormDialog;
use craft_ui::testing::{Fixture, trial};
use craft_ui::tile_grid::{Tile, TileGrid};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const CATEGORIES: [&str; 8] = ["All", "List", "Process", "Cycle", "Hierarchy", "Relationship", "Matrix", "Pyramid"];
const LAYOUTS: [&str; 8] = ["Basic List", "Basic Process", "Basic Cycle", "Hierarchy", "Basic Pyramid", "Basic Radial", "Basic Matrix", "Basic Venn"];
const ABOUT: &str = "Shows items of equal weight as blocks in rows.";

/// The picker's value: the category shown and the layout picked.
#[derive(Clone, Debug, Default, PartialEq)]
struct Picked {
    category: usize,
    layout: usize,
}

fn picker(ui: &mut egui::Ui, picked: &Picked, say: fn(&str) -> String) {
    Row::new().show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            let positions: Vec<usize> = (0..CATEGORIES.len()).collect();
            List::<usize>::new("Categories")
                .select(SelectMode::Single)
                .natural_height(true)
                .selection(Bind::given(Selection::single(picked.category)))
                .show_items(ui, &keyed(&positions, |i| *i), &mut |key, _| ListRow::new(say(CATEGORIES[*key])));
        });
        Stack::new().show(ui, |ui, _| {
            let positions: Vec<usize> = (0..LAYOUTS.len()).collect();
            let _ = TileGrid::<usize>::new("Layouts")
                .select(SelectMode::Single)
                .card_columns(3)
                .max_rows(3)
                .selection(Bind::given(Selection::single(picked.layout)))
                .show_items(ui, &keyed(&positions, |i| *i), &mut |key, _| Tile::new(say(LAYOUTS[*key])).thumb(ThumbAspect::SQUARE, |_, _| ()));
        });
        Stack::new().show(ui, |ui, _| {
            Picture::painter(say(LAYOUTS[picked.layout]), PictureSize::Sample(ThumbSize::Large), ThumbAspect::new(4, 3), |_, _| ()).show(ui);
            Text::new(say(LAYOUTS[picked.layout])).show(ui);
            Text::new(say(ABOUT)).show(ui);
        });
    });
}

/// The passes each of the first six frames of the picker ran, and egui's warning drawn in the last, if any.
fn opening(say: fn(&str) -> String) -> (Vec<usize>, Vec<String>) {
    let value = Rc::new(RefCell::new(Picked::default()));
    let shown = Rc::clone(&value);
    let mut fixture = Fixture::new(move |ui| {
        let mut picked = shown.borrow_mut();
        FormDialog::new(say("Insert Diagram"), Bind::bound(&mut *picked)).show(ui, |ui, _, picked| {
            picker(ui, picked, say);
            true
        });
    });
    fixture.room(egui::vec2(2560.0, 900.0));
    let mut passes = Vec::new();
    for _ in 0..6 {
        fixture.frame(Vec::new());
        passes.push(fixture.passes());
    }
    let warned = fixture.texts().iter().map(|t| t.text.clone()).filter(|t| t.contains("PERF WARNING")).collect();
    (passes, warned)
}

/// The app's one pane, docked at the right with a few rows of text.
struct TextPane;

impl PaneContent<u32> for TextPane {
    fn spec(&self, _: &u32) -> PaneSpec {
        PaneSpec::new("Text Pane", PaneState::docked(PaneEdge::End, 260.0), egui::vec2(260.0, 300.0))
    }

    fn body(&mut self, _: &u32, ui: &mut egui::Ui, _: &mut BodyCx<'_>) {
        for line in ["First item", "Second item", "Third item"] {
            Text::new(line).show(ui);
        }
    }
}

/// WordCraft's audit: the SmartArt text pane shows beside the page for a few frames, then closes in the frame the
/// picker opens. Every frame from the third after that runs one pass.
#[trial]
fn a_picker_dialog_opened_as_a_pane_closes_settles() {
    let open = Rc::new(Cell::new(false));
    let value = Rc::new(RefCell::new(Picked::default()));
    let (dialog, shown) = (Rc::clone(&open), Rc::clone(&value));
    let mut fixture = Fixture::new(move |ui| {
        let panes: &[u32] = if dialog.get() { &[] } else { &[0] };
        PaneArea::new("Panes", panes, &mut TextPane).show(ui, |ui, _| {
            Text::new("The page").show(ui);
        });
        if dialog.get() {
            let mut picked = shown.borrow_mut();
            FormDialog::new("Insert Diagram", Bind::bound(&mut *picked)).show(ui, |ui, _, picked| {
                picker(ui, picked, str::to_owned);
                true
            });
        }
    });
    fixture.room(egui::vec2(2560.0, 900.0)).settle();
    open.set(true);
    let mut passes = Vec::new();
    for _ in 0..6 {
        fixture.frame(Vec::new());
        passes.push(fixture.passes());
    }
    let warned: Vec<String> = fixture.texts().iter().map(|t| t.text.clone()).filter(|t| t.contains("PERF WARNING")).collect();
    assert!(passes[2..].iter().all(|&n| n == 1), "every frame from the third runs one pass: {passes:?}; {warned:?}");
}

/// egui warns (and draws the warning) when three frames in a row each run a second pass: a layout that never settles.
/// The picker opens in a frame or two, and every frame after runs once, in English and in the pseudo locale's longer,
/// bracketed words.
#[trial]
fn a_picker_dialog_settles_after_opening() {
    let pseudo: fn(&str) -> String =
        |text| format!("⟦{}{}⟧", text.replace('a', "á").replace('e', "é").replace('o', "ö"), "~".repeat(text.len() * 2 / 5));
    for (name, say) in [("English", (|text: &str| text.to_owned()) as fn(&str) -> String), ("pseudo", pseudo)] {
        let (passes, warned) = opening(say);
        assert!(passes[2..].iter().all(|&n| n == 1), "{name}: every frame from the third runs one pass: {passes:?}; {warned:?}");
    }
}
