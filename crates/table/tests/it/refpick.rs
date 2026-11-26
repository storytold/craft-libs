//! `ReferencePicker`: point mode writes absolute references at the caret.

use craft_behavior::CellAt;
use craft_table::ui::ReferencePicker;
use craft_ui::testing::trial;

fn cell(row: u32, column: u32) -> CellAt<u32> {
    CellAt::new(row, column)
}

/// A reference may follow `=` or an operator.
fn can_point(text: &str, caret: usize) -> bool {
    let before: String = text.chars().take(caret).collect();
    before.trim_end().ends_with(['=', '(', ',', '+', '-'])
}

fn quote(name: &str) -> String {
    format!("'{name}'")
}

/// Extending makes `$B$3:$C$5`; the next click replaces it.
#[trial]
fn a_drag_grows_a_range_and_another_click_replaces_it() {
    let mut picker = ReferencePicker::new("Sheet1");
    let mut text = "=SUM(".to_owned();
    picker.set_caret(5, false);
    assert!(picker.pick(&mut text, cell(2, 1), "Sheet1", false, can_point, quote));
    assert!(picker.pick(&mut text, cell(4, 2), "Sheet1", true, can_point, quote));
    assert_eq!(text, "=SUM($B$3:$C$5");
    assert!(picker.pick(&mut text, cell(0, 3), "Sheet1", false, can_point, quote));
    assert_eq!(text, "=SUM($D$1");
    picker.set_caret(text.chars().count(), true);
    assert!(!picker.pick(&mut text, cell(0, 0), "Sheet1", false, can_point, quote), "typing forgets the last reference");
}

/// A stored reference that is no longer at the caret does not anchor a drag: the range grows from the press's cell.
#[trial]
fn a_drag_from_elsewhere_ignores_the_stale_reference() {
    let mut picker = ReferencePicker::new("Sheet1");
    let mut text = "=SUM(".to_owned();
    picker.set_caret(5, false);
    assert!(picker.pick(&mut text, cell(2, 1), "Sheet1", false, can_point, quote));
    text.push(',');
    picker.set_caret(text.chars().count(), false);
    assert!(picker.pick(&mut text, cell(4, 2), "Sheet1", true, can_point, quote));
    assert_eq!(text, "=SUM($B$3,$C$5");
}
