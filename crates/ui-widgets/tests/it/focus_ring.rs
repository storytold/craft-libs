//! The focus ring's thickness is a theme role (`stroke.focus`), thicker than the standard line, as production's.

use craft_behavior::{ColumnKey, ColumnWidth, keyed};
use craft_ui::buttons::Button;
use craft_ui::contract::{CollectionEvent, ColumnSpec, Show as _, ShowCollection as _};
use craft_ui::list::{List, ListRow};
use craft_ui::table::Table;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Shape};

/// The widest rectangle stroke in the latest frame.
fn widest_stroke(fixture: &mut Fixture) -> f32 {
    fixture
        .frame_shapes()
        .iter()
        .filter_map(|shape| match shape {
            Shape::Rect(rect) if !rect.stroke.is_empty() => Some(rect.stroke.width),
            _ => None,
        })
        .fold(0.0, f32::max)
}

/// A button with keyboard focus is ringed with a line of at least two points; unfocused, nothing is that thick.
#[trial]
fn the_focus_ring_is_two_points_thick() {
    let mut fixture = Fixture::new(|ui| {
        let _ = Button::new("Go").show(ui);
    });
    fixture.settle();
    assert!(widest_stroke(&mut fixture) < 2.0, "an unfocused button has no thick line");
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.settle();
    assert!(fixture.focused().is_some(), "Tab focused the button");
    let width = widest_stroke(&mut fixture);
    assert!((width - 2.0).abs() < 0.01, "the ring is the theme's focus stroke of two points: {width}");
}

/// A disclosure with keyboard focus is ringed as a button is: a focus stroke around its own rect.
#[trial]
fn a_focused_disclosure_shows_the_focus_ring_around_it() {
    use craft_ui::contract::{Bind, Props as _, Role};
    use craft_ui::controls::disclosure::Disclosure;
    let mut fixture = Fixture::new(|ui| {
        let _ = Disclosure::new(Bind::given(false)).label("Details").show(ui);
    });
    fixture.settle();
    assert!(widest_stroke(&mut fixture) < 2.0, "an unfocused disclosure has no thick line");
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.settle();
    assert!(fixture.focused().is_some(), "Tab focused the disclosure");
    let target = fixture.by_role(Role::BUTTON).into_iter().next().expect("the disclosure shows");
    let ring = fixture.frame_shapes().iter().find_map(|shape| match shape {
        Shape::Rect(rect) if rect.stroke.width >= 2.0 => Some(rect.rect),
        _ => None,
    });
    let ring = ring.expect("the focused disclosure paints a focus ring");
    assert!(ring.expand(1.0).contains_rect(target.rect), "the ring {ring:?} goes around the disclosure {:?}", target.rect);
}

#[trial]
fn keyboard_focus_rings_the_lists_or_tables_current_target() {
    for table in [false, true] {
        let names = ["First file", "Second file"];
        let mut fixture = Fixture::new(move |ui| {
            let source = keyed(&names, |name: &&str| *name);
            if table {
                Table::new("Files", vec![ColumnSpec::new(ColumnKey::new("name"), "Name", ColumnWidth::fill(1.0))]).show_items(
                    ui,
                    &source,
                    &mut |name, icx| ListRow::new(*name).cells(icx.cells()),
                );
            } else {
                List::new("Files").show_items(ui, &source, &mut |name, _| ListRow::new(*name));
            }
        });
        fixture.settle();
        let collection = fixture.by_label("Files").expect("the collection");
        let first = fixture.by_label(names[0]).expect("the first row");
        let next = fixture.by_label(names[1]).expect("the second row");

        fixture.key(Key::Tab, Modifiers::NONE).settle();
        assert_eq!(fixture.tree().focused_key(), collection.node, "Tab reaches the collection (table: {table})");
        let viewport_ring = fixture
            .frame_shapes()
            .iter()
            .find_map(|shape| match shape {
                Shape::Rect(rect) if rect.stroke.width >= 2.0 && rect.rect.contains(first.center()) && rect.rect.contains(next.center()) => {
                    Some(rect.rect)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("the rows' viewport is ringed (table: {table})"));
        if table {
            let header = fixture.by_label("Name").expect("the column header");
            assert!(viewport_ring.top() >= header.rect.bottom() - 1.0, "the table ring starts below the column header");
        }
        assert!(!row_is_ringed(&mut fixture, first.rect), "Tab does not target the first row (table: {table})");

        fixture.key(Key::ArrowDown, Modifiers::NONE).settle();
        assert_eq!(fixture.tree().focused_key(), first.node, "Down reaches the first row (table: {table})");
        assert!(row_is_ringed(&mut fixture, first.rect), "the first row is ringed (table: {table})");

        fixture.key(Key::ArrowDown, Modifiers::NONE).settle();
        assert_eq!(fixture.tree().focused_key(), next.node, "Down moves the row lead (table: {table})");
        assert!(row_is_ringed(&mut fixture, next.rect), "the new lead is ringed (table: {table})");
        assert!(!row_is_ringed(&mut fixture, first.rect), "the old lead is not ringed (table: {table})");
    }
}

fn row_is_ringed(fixture: &mut Fixture, row: egui::Rect) -> bool {
    fixture.frame_shapes().iter().any(|shape| match shape {
        Shape::Rect(rect) => rect.stroke.width >= 2.0 && (rect.rect.center().y - row.center().y).abs() < 1.0,
        _ => false,
    })
}

#[trial]
fn delete_after_a_click_in_blank_list_space_does_not_choose_a_row() {
    use std::{cell::Cell, rc::Rc};

    for tab_first in [false, true] {
        let deleted = Rc::new(Cell::new(false));
        let seen = deleted.clone();
        let mut fixture = Fixture::new(move |ui| {
            let source = keyed(&[0], |key: &usize| *key);
            let out = List::new("Files").deletable(true).show_items(ui, &source, &mut |_, _| ListRow::new("A file"));
            if out.events.iter().any(|event| matches!(event, CollectionEvent::Delete(_))) {
                seen.set(true);
            }
        });
        fixture.room(egui::vec2(300.0, 180.0)).settle();
        let list = fixture.by_label("Files").expect("list");
        let row = fixture.by_label("A file").expect("row");
        if tab_first {
            fixture.key(Key::Tab, Modifiers::NONE).settle();
            assert_eq!(fixture.tree().focused_key(), list.node);
        }
        let point = egui::pos2(row.rect.center().x, (row.rect.bottom() + list.rect.bottom()) / 2.0);
        assert!(list.rect.contains(point) && !row.rect.contains(point), "the click is below the row");
        fixture.click_at(point).settle();
        assert_eq!(fixture.tree().focused_key(), list.node, "the collection is focused without a row");
        fixture.key(Key::Delete, Modifiers::NONE).settle();
        assert!(!deleted.get(), "Delete must leave the row alone");
    }
}

#[trial]
fn a_focused_list_shows_a_ring_when_rows_arrive() {
    use std::{cell::Cell, rc::Rc};

    let loaded = Rc::new(Cell::new(false));
    let state = loaded.clone();
    let mut fixture = Fixture::new(move |ui| {
        let keys: &[usize] = if state.get() { &[0] } else { &[] };
        let source = keyed(keys, |key: &usize| *key);
        List::new("Files").rows_stamp(keys.len() as u64).show_items(ui, &source, &mut |_, _| ListRow::new("A file"));
    });
    fixture.room(egui::vec2(300.0, 180.0)).settle();
    let list = fixture.by_label("Files").expect("list");
    fixture.key(Key::Tab, Modifiers::NONE).settle();
    assert_eq!(fixture.tree().focused_key(), list.node, "the empty list holds focus");
    loaded.set(true);
    fixture.settle();
    assert!(fixture.by_label("A file").is_some());
    assert_eq!(fixture.tree().focused_key(), list.node);
    assert!(widest_stroke(&mut fixture) >= 2.0, "the loaded rows' viewport is ringed");
}

#[trial]
fn a_lead_scrolled_out_of_view_marks_the_edge_it_passed() {
    let mut fixture = Fixture::new(move |ui| {
        let keys: Vec<usize> = (0..48).collect();
        let source = keyed(&keys, |key: &usize| *key);
        List::new("Files").show_items(ui, &source, &mut |key, _| ListRow::new(format!("File {key}")));
    });
    fixture.room(egui::vec2(300.0, 180.0)).settle();
    let list = fixture.by_label("Files").expect("list");
    let first = fixture.by_label("File 0").expect("first row");
    fixture.key(Key::Tab, Modifiers::NONE).key(Key::ArrowDown, Modifiers::NONE).settle();
    assert_eq!(fixture.tree().focused_key(), first.node);
    fixture.frame(vec![craft_ui::testing::pointer::moved(list.center())]);
    fixture.frame(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -360.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
        source: egui::MouseWheelSource::Wheel,
    }]);
    fixture.settle();
    assert!(fixture.frame_shapes().iter().any(|shape| match shape {
        Shape::Rect(rect) => {
            rect.stroke.width >= 2.0
                && (rect.rect.top() - list.rect.top()).abs() <= 1.0
                && rect.rect.width() > list.rect.width() / 2.0
                && rect.rect.height() < first.rect.height() / 2.0
        }
        _ => false,
    }));
}

#[trial]
fn a_focus_ring_contrasts_with_the_lead_rows_fill() {
    use craft_theme::{Contrast, Mode, Rgba, ThemeId, contrast_ratio};

    for (theme, mode, select) in [("craft.classic", Mode::Light, true), ("craft.adwaita", Mode::Dark, false)] {
        let mut fixture = Fixture::new(move |ui| {
            let source = keyed(&[0], |key: &usize| *key);
            List::new("Files").arrows_select(select).show_items(ui, &source, &mut |_, _| ListRow::new("A file"));
        });
        fixture.theme_id(ThemeId::parse(theme).unwrap()).theme(mode, Contrast::Standard).settle();
        let row = fixture.by_label("A file").expect("row");
        fixture.key(Key::Tab, Modifiers::NONE).key(Key::ArrowDown, Modifiers::NONE).settle();
        assert_eq!(fixture.tree().focused_key(), row.node);

        let shapes = fixture.frame_shapes();
        let row_shapes: Vec<_> = shapes
            .iter()
            .filter_map(|shape| match shape {
                Shape::Rect(rect) if (rect.rect.center().y - row.center().y).abs() < 1.0 => Some(rect),
                _ => None,
            })
            .collect();
        let fill = row_shapes.iter().rev().find(|shape| shape.fill.a() > 0).expect("the lead row fill").fill;
        let stroke = row_shapes.iter().rev().find(|shape| !shape.stroke.is_empty()).expect("the focus stroke").stroke.color;
        let ratio = contrast_ratio(Rgba(stroke.to_srgba_unmultiplied()), Rgba(fill.to_srgba_unmultiplied()));
        assert!(ratio >= 3.0, "{theme}: the row focus ring contrast is {ratio}");
    }
}
