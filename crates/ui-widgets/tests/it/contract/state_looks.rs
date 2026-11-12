//! Hover and press looks: a part declared `state_layer` takes the theme's hover fill under the pointer, and a check
//! indicator shows the press.

use craft_ui::choice::{Checkbox, Rating};
use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::testing::layout::in_region;
use craft_ui::testing::{Frames, pointer, trial};
use egui::{Rect, Shape, pos2, vec2};
use std::cell::Cell;

/// The filled rects inside `within`.
fn plates(shapes: &[Shape], within: Rect) -> usize {
    shapes.iter().filter(|shape| matches!(shape, Shape::Rect(rect) if rect.fill.a() > 0 && within.contains_rect(rect.rect))).count()
}

#[trial]
fn hovering_a_star_fills_its_cell_and_leaving_it_clears_the_fill() {
    let mut frames = Frames::new();
    let shown = Cell::new(Rect::NOTHING);
    let mut show = |ui: &mut egui::Ui| {
        let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 40.0));
        shown.set(in_region(ui, region, |ui| Rating::new(Bind::given(0u8), 5).id_salt("stars").show(ui).response.rect));
    };
    let rest = frames.frame_shapes(&mut show);
    let rect = shown.get();
    assert_eq!(plates(&rest, rect), 0, "a rating at rest paints no fill");
    frames.frame(vec![pointer::moved(pos2(rect.min.x + 2.0, rect.center().y))], &mut show);
    let hovered = frames.frame_shapes(&mut show);
    assert_eq!(plates(&hovered, rect), 1, "the hovered star's cell, and only it, takes the hover fill");
    frames.frame(vec![pointer::gone()], &mut show);
    let left = frames.frame_shapes(&mut show);
    assert_eq!(plates(&left, rect), 0, "the fill goes with the pointer");
}

/// The fill colours painted inside `within`.
fn fills(shapes: &[Shape], within: Rect) -> Vec<egui::Color32> {
    shapes
        .iter()
        .filter_map(|shape| match shape {
            Shape::Rect(rect) if rect.fill.a() > 0 && within.contains_rect(rect.rect) => Some(rect.fill),
            _ => None,
        })
        .collect()
}

#[trial]
fn pressing_a_check_box_changes_its_fill_whether_off_or_on() {
    for on in [false, true] {
        let mut frames = Frames::new();
        let shown = Cell::new(Rect::NOTHING);
        let mut show = |ui: &mut egui::Ui| {
            let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(40.0, 40.0));
            shown.set(in_region(ui, region, |ui| Checkbox::new(Bind::given(on)).label("Snap").show(ui).response.rect));
        };
        let rest = frames.frame_shapes(&mut show);
        let rect = shown.get();
        frames.frame(vec![pointer::moved(rect.center()), pointer::press(rect.center())], &mut show);
        let pressed = frames.frame_shapes(&mut show);
        assert!(!fills(&rest, rect).is_empty(), "the box paints a fill (on: {on})");
        assert_ne!(fills(&pressed, rect), fills(&rest, rect), "a press shows on the box (on: {on})");
    }
}

/// A press held on a table row shows on the check cell that follows the row's press and on a pressed cell, never on a
/// lock cell the press is not on.
#[trial]
fn a_row_press_shows_on_the_check_cell_that_follows_it_and_a_cell_press_on_its_own_cell() {
    use craft_behavior::{ColumnKey, ColumnWidth, keyed};
    use craft_ui::contract::{ColumnKind, ColumnSpec, ShowCollection as _, ToggleColumn};
    use craft_ui::list::{ListRow, ToggleCell, ToggleState};
    use craft_ui::table::Table;
    const CHECK: ColumnKey = ColumnKey::new("check");
    const NAME: ColumnKey = ColumnKey::new("name");
    const LOCK: ColumnKey = ColumnKey::new("lock");
    let cells = Cell::new([Rect::NOTHING; 3]);
    let mut show = |ui: &mut egui::Ui| {
        let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(240.0, 80.0));
        in_region(ui, region, |ui| {
            let columns = vec![
                ColumnSpec::new(CHECK, "", ColumnWidth::Controls(1.0)).kind(ColumnKind::Toggle(ToggleColumn::default())),
                ColumnSpec::new(NAME, "", ColumnWidth::fill(1.0)),
                ColumnSpec::new(LOCK, "", ColumnWidth::Controls(1.0)).kind(ColumnKind::Toggle(ToggleColumn::default())),
            ];
            let order = [0usize];
            let source = keyed(&order, |&row| row);
            let shown = Table::new("Sections", columns).header(false).show_items(ui, &source, &mut |_, icx| {
                let mut found = [Rect::NOTHING; 3];
                for (at, key) in [CHECK, NAME, LOCK].iter().enumerate() {
                    found[at] = icx.cell(key).unwrap_or(Rect::NOTHING);
                }
                cells.set(found);
                ListRow::new("Shape Dynamics")
                    .cells(icx.cells())
                    .toggle(ToggleCell::check(CHECK, ToggleState::Off, "Enable {row}").row_press())
                    .toggle(ToggleCell::check(LOCK, ToggleState::Off, "Lock {row}"))
            });
            let _ = shown.response;
        });
    };
    let mut held = |at: fn([Rect; 3]) -> egui::Pos2| {
        let mut frames = Frames::new();
        let rest = frames.frame_shapes(&mut show);
        let [check, _, lock] = cells.get();
        let point = at(cells.get());
        frames.frame(vec![pointer::moved(point), pointer::press(point)], &mut show);
        frames.frame(vec![], &mut show);
        let pressed = frames.frame_shapes(&mut show);
        (fills(&rest, check) != fills(&pressed, check), fills(&rest, lock) != fills(&pressed, lock))
    };
    assert_eq!(held(|[_, name, _]| name.center()), (true, false), "a press on the row shows on the check cell, not the lock");
    assert_eq!(held(|[_, _, lock]| lock.center()), (false, true), "a press on the lock shows on the lock only");
    assert_eq!(held(|[check, _, _]| check.center()), (true, false), "a press on the check cell shows on it");
}

/// A highlighted row that is selected shows the selection, as a selected row does; highlighted alone it takes the tint.
#[trial]
fn a_selected_row_shows_its_selection_over_the_highlight() {
    use craft_behavior::{Selection, keyed};
    use craft_ui::contract::{SelectMode, ShowCollection as _};
    use craft_ui::list::{List, ListRow};
    let rows = Cell::new([Rect::NOTHING; 4]);
    let mut show = |ui: &mut egui::Ui| {
        let region = Rect::from_min_size(pos2(0.0, 0.0), vec2(240.0, 200.0));
        in_region(ui, region, |ui| {
            let order = [0usize, 1, 2, 3];
            let source = keyed(&order, |&row| row);
            let selection = Selection::new(vec![0, 1], None, None);
            let _ =
                List::<usize>::new("Styles").select(SelectMode::Multi).selection(Bind::given(selection)).show_items(ui, &source, &mut |key, icx| {
                    let mut found = rows.get();
                    found[*key] = icx.rect;
                    rows.set(found);
                    // Rows 0 and 1 are selected; rows 0 and 2 are highlighted.
                    ListRow::new("Style").highlight(*key % 2 == 0)
                });
        });
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    let shapes = frames.frame_shapes(&mut show);
    let [both, selected, highlighted, plain] = rows.get().map(|rect| fills(&shapes, rect));
    assert_eq!(both, selected, "a selected row shows its selection whether highlighted or not");
    assert_ne!(highlighted, plain, "a highlighted row that is not selected takes the tint");
    assert_ne!(highlighted, both, "the tint is not the selection");
}
