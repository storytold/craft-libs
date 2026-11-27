//! The Glyphs panel's flows over a source of plain cells: a marked cell far down the grid, and a pick in a cell's
//! alternates popup.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use craft_font_manager::glyphs::{CellInfo, GlyphCell, GlyphSource, GlyphsPanel, GlyphsPanelSettings};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::testing::pointer;
use craft_ui::testing::{Fixture, Room, trial};
use egui::{ColorImage, Event};

/// The first cell, which has alternates.
const FIRST: char = 'Ā';
/// A cell several screens below the first.
const FAR: char = 'Ǻ';

/// Three hundred characters from U+0100 on, no pictures; the first has three alternates.
struct Plain;

impl GlyphSource for Plain {
    fn cells(&self, _: &str, _: &str, _: &str) -> Vec<GlyphCell> {
        (0..300).filter_map(|i| char::from_u32(0x100 + i)).map(GlyphCell::Char).collect()
    }

    fn glyph(&self, _: &str, _: &str, _: GlyphCell, _: u32) -> Option<ColorImage> {
        None
    }

    fn describe(&self, _: &str, _: &str, cell: GlyphCell) -> CellInfo {
        let mut info = CellInfo::default();
        info.has_alternates = cell == GlyphCell::Char(FIRST);
        info
    }

    fn alternates(&self, _: &str, _: &str, cell: GlyphCell) -> Vec<GlyphCell> {
        if cell == GlyphCell::Char(FIRST) { ['Ⓐ', 'Ⓑ', 'Ⓒ'].map(GlyphCell::Char).to_vec() } else { Vec::new() }
    }
}

/// What the host passes and what the panel reported over the passes.
#[derive(Default)]
struct Seen {
    marked: Option<GlyphCell>,
    reveal: Option<GlyphCell>,
    picks: Vec<(GlyphCell, GlyphCell)>,
    /// Cells a secondary click asked for a menu on, in order.
    menu: Vec<GlyphCell>,
}

fn rig() -> (Fixture, Rc<RefCell<Seen>>) {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let shown = Rc::clone(&seen);
    let (mut family, mut style, mut settings) = ("Plain".to_owned(), "Regular".to_owned(), GlyphsPanelSettings::default());
    let mut fx = Fixture::new(move |ui| {
        let (marked, reveal) = {
            let mut seen = shown.borrow_mut();
            (seen.marked, seen.reveal.take())
        };
        let mut panel =
            GlyphsPanel::new("Glyphs", &Plain, Bind::bound(&mut family), Bind::bound(&mut style), Bind::bound(&mut settings)).can_insert(true);
        if let Some(cell) = marked {
            panel = panel.marked(cell);
        }
        if let Some(cell) = reveal {
            panel = panel.reveal(cell);
        }
        let out = panel.show(ui);
        if let Some(pick) = out.alternate {
            shown.borrow_mut().picks.push((pick.cell, pick.glyph));
        }
        if let Some(menu) = out.menu {
            shown.borrow_mut().menu.push(menu.cell);
        }
    });
    fx.in_room(Room::Window);
    fx.settle();
    (fx, seen)
}

fn name(c: char) -> String {
    format!("{c}, U+{:04X}", c as u32)
}

fn visible(fx: &Fixture, c: char) -> bool {
    fx.by_label(&name(c)).and_then(|cell| fx.visible(&cell)).is_some()
}

fn selected(fx: &Fixture, c: char) -> bool {
    fx.by_label(&name(c)).is_some_and(|cell| fx.tree().selected_of(cell.node).unwrap_or(false))
}

#[trial]
fn the_marked_cell_is_selected_and_revealed_only_in_the_pass_that_carries_reveal() {
    let (mut fx, seen) = rig();
    assert!(visible(&fx, FIRST), "the grid starts at its first cell");
    assert!(!visible(&fx, FAR), "a cell several screens down is out of view");

    {
        let mut seen = seen.borrow_mut();
        seen.marked = Some(GlyphCell::Char(FAR));
        seen.reveal = Some(GlyphCell::Char(FAR));
    }
    fx.frame(Vec::new());
    fx.settle();
    assert!(visible(&fx, FAR), "the pass with reveal scrolls the marked cell into view");
    assert!(selected(&fx, FAR), "the marked cell is the selected one, not a position");

    fx.frame(Vec::new());
    fx.settle();
    assert!(visible(&fx, FAR), "a pass without reveal keeps the scroll");
    assert!(!visible(&fx, FIRST), "the grid does not jump back to its start");
}

#[trial]
fn a_corner_pick_reports_the_cell_and_the_glyph() {
    let (mut fx, seen) = rig();
    let cell = fx.by_label(&name(FIRST)).expect("the first cell");
    fx.frame(vec![Event::PointerMoved(cell.center())]);
    // A tile declares its hover-revealed parts from the last pass's hover.
    fx.settle();
    let action = fx.by_label("Show Alternates").expect("the corner action shows while the pointer is over the cell");
    fx.click(&action);
    fx.settle();
    assert!(visible(&fx, 'Ⓑ'), "the corner action opens the popup of alternates");
    assert!(seen.borrow().picks.is_empty(), "opening the popup picks nothing");

    let second = fx.by_label(&name('Ⓑ')).expect("the second alternate");
    fx.click(&second);
    fx.settle();
    assert_eq!(seen.borrow().picks, vec![(GlyphCell::Char(FIRST), GlyphCell::Char('Ⓑ'))], "one pick, with the cell the popup was opened for");
    assert!(!visible(&fx, 'Ⓑ'), "a pick closes the popup");
}

#[trial]
fn a_secondary_click_reports_the_cell_the_menu_is_for() {
    let (mut fx, seen) = rig();
    let cell = fx.by_label(&name(FIRST)).expect("the first cell");
    let at = cell.center();
    fx.frame(vec![pointer::moved(at), pointer::secondary(at, true)]);
    fx.frame(vec![pointer::secondary(at, false)]);
    fx.settle();
    let menu = seen.borrow().menu.clone();
    assert!(!menu.is_empty(), "the secondary click names the cell");
    assert!(menu.iter().all(|cell| *cell == GlyphCell::Char(FIRST)), "the menu is for the clicked cell: {menu:?}");
}

/// A list whose first cell changes, at the same length, when `swapped` is set. The revision changes with it, so the
/// panel reads the list again.
struct Swap(Arc<AtomicBool>);

impl GlyphSource for Swap {
    fn cells(&self, _: &str, _: &str, _: &str) -> Vec<GlyphCell> {
        let first = if self.0.load(Ordering::Relaxed) { '\u{0102}' } else { FIRST };
        std::iter::once(GlyphCell::Char(first)).chain((1..300).filter_map(|i| char::from_u32(0x100 + i).map(GlyphCell::Char))).collect()
    }

    fn glyph(&self, _: &str, _: &str, _: GlyphCell, _: u32) -> Option<egui::ColorImage> {
        None
    }

    fn revision(&self) -> u64 {
        u64::from(self.0.load(Ordering::Relaxed))
    }
}

#[trial]
fn an_open_menu_keeps_the_clicked_cell_when_that_row_changes() {
    let swapped = Arc::new(AtomicBool::new(false));
    let source = Swap(Arc::clone(&swapped));
    let seen = Rc::new(RefCell::new(Vec::<GlyphCell>::new()));
    let shown = Rc::clone(&seen);
    let (mut family, mut style, mut settings) = ("Plain".to_owned(), "Regular".to_owned(), GlyphsPanelSettings::default());
    let mut fx = Fixture::new(move |ui| {
        let out = GlyphsPanel::new("Glyphs", &source, Bind::bound(&mut family), Bind::bound(&mut style), Bind::bound(&mut settings))
            .can_insert(true)
            .show(ui);
        if let Some(menu) = out.menu {
            shown.borrow_mut().push(menu.cell);
            Menu::context("Glyph", &menu.opener).show(ui, |ui, _| {
                MenuItem::new("Insert Glyph").show(ui);
            });
        }
    });
    fx.in_room(Room::Window);
    fx.settle();
    let cell = fx.by_label(&name(FIRST)).expect("the first cell");
    let at = cell.center();
    fx.frame(vec![pointer::moved(at), pointer::secondary(at, true)]);
    fx.frame(vec![pointer::secondary(at, false)]);
    fx.settle();
    assert!(seen.borrow().contains(&GlyphCell::Char(FIRST)), "the click names the cell");

    seen.borrow_mut().clear();
    swapped.store(true, Ordering::Relaxed);
    fx.frame(Vec::new());
    fx.settle();
    let menu = seen.borrow().clone();
    assert!(!menu.is_empty(), "the menu stays open");
    assert!(menu.iter().all(|cell| *cell == GlyphCell::Char(FIRST)), "the row's new glyph does not take the menu: {menu:?}");
}

/// Unencoded glyphs 1 to 300 in every face: the same id names another glyph in another face.
struct Unencoded;

impl GlyphSource for Unencoded {
    fn cells(&self, _: &str, _: &str, _: &str) -> Vec<GlyphCell> {
        (1..=300).map(GlyphCell::Glyph).collect()
    }

    fn glyph(&self, _: &str, _: &str, _: GlyphCell, _: u32) -> Option<ColorImage> {
        None
    }
}

#[trial]
fn an_open_menu_does_not_report_a_glyph_id_from_another_face() {
    let family = Rc::new(RefCell::new("Plain".to_owned()));
    let host_family = Rc::clone(&family);
    let seen = Rc::new(RefCell::new(Vec::<GlyphCell>::new()));
    let shown = Rc::clone(&seen);
    let (mut style, mut settings) = ("Regular".to_owned(), GlyphsPanelSettings::default());
    let mut fx = Fixture::new(move |ui| {
        let out = {
            let mut family = host_family.borrow_mut();
            GlyphsPanel::new("Glyphs", &Unencoded, Bind::bound(&mut *family), Bind::bound(&mut style), Bind::bound(&mut settings))
                .can_insert(true)
                .show(ui)
        };
        if let Some(menu) = out.menu {
            shown.borrow_mut().push(menu.cell);
            Menu::context("Glyph", &menu.opener).show(ui, |ui, _| {
                MenuItem::new("Insert Glyph").show(ui);
            });
        }
    });
    fx.in_room(Room::Window);
    fx.settle();
    let cell = fx.by_label("1, glyph 1").expect("the first glyph");
    let at = cell.center();
    fx.frame(vec![pointer::moved(at), pointer::secondary(at, true)]);
    fx.frame(vec![pointer::secondary(at, false)]);
    fx.settle();
    assert!(seen.borrow().contains(&GlyphCell::Glyph(1)), "the click names the glyph");

    seen.borrow_mut().clear();
    *family.borrow_mut() = "Other".to_owned();
    fx.frame(Vec::new());
    fx.settle();
    let menu = seen.borrow().clone();
    assert!(menu.is_empty(), "the clicked id names another glyph in the new face, so the menu reports none: {menu:?}");
}

#[trial]
fn glyphs_panel_alternates() {
    let (mut fx, _seen) = rig();
    let cell = fx.by_label(&name(FIRST)).expect("the first cell");
    assert!(fx.by_label("Show Alternates").is_none(), "the corner action stays hidden while the pointer is elsewhere");
    fx.frame(vec![Event::PointerMoved(cell.center())]);
    fx.settle();
    let action = fx.by_label("Show Alternates").expect("the corner action shows while the pointer is over the cell");
    fx.click(&action);
    fx.settle();
    for alternate in ['Ⓐ', 'Ⓑ', 'Ⓒ'] {
        assert!(visible(&fx, alternate), "the popup lists every alternate of the cell");
    }
}
