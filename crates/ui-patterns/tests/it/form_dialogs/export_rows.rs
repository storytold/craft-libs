//! The Export As dialog opened over several assets: the row list on its left, the settings of the selected row, and
//! the rows the answer carries.

use craft_behavior::PixelSize;
use craft_ui::contract::Role;
use craft_ui::dialog::{DialogHost, DialogResult, StackDialog};
use craft_ui::patterns::form_dialogs::{ExportAsAsset, ExportAsResult, ExportOptions, ExportSize, HostedExportAs, SizeEstimate};
use craft_ui::testing::pointer;
use craft_ui::testing::{Fixture, Target, trial};
use egui::{Key, Modifiers, Vec2};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// The document's size in pixels.
fn document() -> PixelSize {
    PixelSize::new(1200, 800)
}

/// The settings of a row: `format` at full scale.
fn settings(format: &str) -> ExportOptions {
    let mut options = ExportOptions::default();
    options.format = format.to_owned();
    options
}

/// The rows as an app would hand them over: two assets with pixels and their own formats, and one without pixels.
fn assets() -> Vec<ExportAsAsset> {
    vec![
        ExportAsAsset::new("big", "Big mark", PixelSize::new(400, 300)).options(settings("jpg")),
        ExportAsAsset::new("small", "Small mark", PixelSize::new(40, 30)).options(settings("png")),
        ExportAsAsset::new("fill", "Fill layer", document()).options(settings("webp")).empty(true),
    ]
}

/// A hosted Export As over the three assets, and the fixture that runs the dialog host.
fn open(host: &Rc<RefCell<DialogHost>>, fixture: &mut Fixture) {
    host.borrow_mut().open(HostedExportAs::new("Poster", document(), ExportOptions::default()).assets(assets()));
    fixture.idle(10);
}

/// A host running one Export As, opened over `assets` when given, else over one file.
fn dialog(with_assets: bool) -> (Rc<RefCell<DialogHost>>, Fixture) {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    let mut fixture = fixture;
    fixture.room(Vec2::new(1500.0, 1000.0));
    if with_assets {
        open(&host, &mut fixture);
    } else {
        host.borrow_mut().open(HostedExportAs::new("Poster", document(), ExportOptions::default()));
        fixture.idle(10);
    }
    (host, fixture)
}

/// The rows of the list, top row first.
fn rows(fixture: &Fixture) -> Vec<String> {
    let mut rows = fixture.by_role(Role::LIST_ITEM);
    rows.sort_by(|a, b| a.center().y.total_cmp(&b.center().y));
    rows.iter().map(|row| fixture.tree().name_of(row.node).unwrap_or_default().to_owned()).collect()
}

/// The names the tree gives the cells inside the row `wanted`, in tree order: its own text, its format, its size and
/// its estimate.
fn cells_of(fixture: &Fixture, wanted: usize) -> Vec<String> {
    let mut list = fixture.by_role(Role::LIST_ITEM);
    list.sort_by(|a, b| a.center().y.total_cmp(&b.center().y));
    let row = &list[wanted];
    fixture
        .by_role(Role::CELL)
        .into_iter()
        .filter(|cell| row.rect.contains(cell.rect.center()))
        .map(|cell| fixture.tree().name_of(cell.node).unwrap_or_default().to_owned())
        .collect()
}

/// The tick boxes of the rows, top row first.
fn ticks(fixture: &Fixture) -> Vec<craft_ui::testing::Target> {
    let mut ticks = fixture.by_role(Role::CHECK_BOX);
    ticks.sort_by(|a, b| a.center().y.total_cmp(&b.center().y));
    ticks
}

/// A host running one Export As over `list`, in a room of `room`. The dialog's own opening settings are what every
/// trial here uses: PNG at the document's full size.
fn over(list: Vec<ExportAsAsset>, room: Vec2) -> (Rc<RefCell<DialogHost>>, Fixture) {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(room);
    host.borrow_mut().open(HostedExportAs::new("Poster", document(), ExportOptions::default()).assets(list));
    fixture.idle(10);
    (host, fixture)
}

/// The rows as the Reset trial opens with: the first as JPG, which the dialog's own opening value is not, and the
/// second at a fixed pixel size of its own, which only fits its own 40 x 30 asset.
fn opening_assets() -> Vec<ExportAsAsset> {
    let mut thumb = settings("webp");
    thumb.size = ExportSize::pixels(20, 15);
    vec![
        ExportAsAsset::new("big", "Big mark", PixelSize::new(400, 300)).options(settings("jpg")),
        ExportAsAsset::new("thumb", "Thumb", PixelSize::new(40, 30)).options(thumb),
    ]
}

/// Twelve assets with pixels, as a panel of twelve layers hands them over.
fn long_list(count: usize) -> Vec<ExportAsAsset> {
    (1..=count).map(|at| ExportAsAsset::new(format!("layer{at}"), format!("Layer {at}"), PixelSize::new(400, 300))).collect()
}

/// The rows of the list as the tree bounds them, top row first.
fn row_targets(fixture: &Fixture) -> Vec<Target> {
    let mut rows = fixture.by_role(Role::LIST_ITEM);
    rows.sort_by(|a, b| a.center().y.total_cmp(&b.center().y));
    rows
}

/// The number an Image Size row shows, without its unit.
fn size_row(fixture: &Fixture, label: &str) -> String {
    let row = fixture.by_label(label).unwrap_or_else(|| panic!("the {label} row"));
    let value = fixture.tree().value_of(row.node).unwrap_or_default();
    value.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect()
}

/// Presses Export and answers the path it asks for.
fn export(host: &Rc<RefCell<DialogHost>>, fixture: &mut Fixture) -> ExportAsResult {
    let button = fixture.by_label("Export…").expect("the Export button");
    fixture.click(&button).idle(4);
    let request = host.borrow_mut().get_mut::<HostedExportAs>().and_then(HostedExportAs::pick_path).expect("Export asks for a path");
    if let Some(dialog) = host.borrow_mut().get_mut::<HostedExportAs>() {
        dialog.answer_path(request.key.clone(), PathBuf::from("/tmp/poster"));
    }
    fixture.idle(6);
    host.borrow_mut().take::<HostedExportAs>().and_then(DialogResult::ok).expect("the answered path finishes the dialog")
}

/// The rows the dialog holds, as the user left them.
fn held(host: &Rc<RefCell<DialogHost>>) -> Vec<(String, String, bool)> {
    host.borrow()
        .get::<HostedExportAs>()
        .and_then(HostedExportAs::rows)
        .map(|rows| rows.rows.iter().map(|row| (row.asset.id.clone(), row.options.format.clone(), row.ticked)).collect())
        .unwrap_or_default()
}

/// The list draws one row per asset, in the order the app gave them, and each row carries its own format and its own
/// size: the assets do not all read the document's settings.
#[trial]
fn the_list_draws_one_row_per_asset_with_its_own_format_and_size() {
    let (host, fixture) = dialog(true);
    assert_eq!(rows(&fixture), ["Big mark", "Small mark", "Fill layer"], "one row per asset, in the app's order");
    let big = cells_of(&fixture, 0);
    assert!(big.iter().any(|cell| cell == "JPG"), "the first row names its own format: {big:?}");
    assert!(big.iter().any(|cell| cell.contains("400") && cell.contains("300")), "the first row shows its own sides: {big:?}");
    let small = cells_of(&fixture, 1);
    assert!(small.iter().any(|cell| cell == "PNG"), "the second row names its own format: {small:?}");
    assert!(small.iter().any(|cell| cell.contains("40") && cell.contains("30")), "the second row shows its own sides: {small:?}");
    assert_eq!(host.borrow().get::<HostedExportAs>().and_then(HostedExportAs::rows).map(|rows| rows.selected), Some(0), "the first row is selected");
}

/// The asset with no pixels has no thumbnail, format, size or estimate in its row, and carries the mark that says the
/// image will not be exported. Selecting its row says so where the preview is.
#[trial]
fn an_asset_with_no_pixels_says_so_in_its_row_and_in_the_preview() {
    let (host, mut fixture) = dialog(true);
    let fill = cells_of(&fixture, 2);
    assert!(!fill.iter().any(|cell| cell == "WEBP"), "the empty asset's row names no format: {fill:?}");
    assert!(!fill.iter().any(|cell| cell.contains('×')), "the empty asset's row shows no size: {fill:?}");
    let marks = fixture.by_role(Role::IMAGE);
    let warned =
        marks.iter().filter(|mark| fixture.tree().name_of(mark.node).is_some_and(|name| name.starts_with("Image will not be exported"))).count();
    assert_eq!(warned, 1, "the row that is not ticked carries the mark that says the image will not be exported");
    assert!(!fixture.texts().iter().any(|run| run.text == "Image is empty."), "the notice shows only once its row is selected");
    let list = {
        let mut list = fixture.by_role(Role::LIST_ITEM);
        list.sort_by(|a, b| a.center().y.total_cmp(&b.center().y));
        list[2]
    };
    fixture.click(&list).idle(4);
    assert_eq!(host.borrow().get::<HostedExportAs>().and_then(HostedExportAs::rows).map(|rows| rows.selected), Some(2), "the row is selected");
    assert!(fixture.texts().iter().any(|run| run.text == "Image is empty."), "the preview's place says the image is empty");
}

/// The settings beside the list belong to one row: a format picked while the second row is selected leaves the first
/// row's format as it was, and the row text follows its own settings.
#[trial]
fn a_setting_changed_on_one_row_leaves_the_other_rows() {
    let (host, mut fixture) = dialog(true);
    let row = {
        let mut list = fixture.by_role(Role::LIST_ITEM);
        list.sort_by(|a, b| a.center().y.total_cmp(&b.center().y));
        list[1]
    };
    fixture.click(&row).idle(4);
    let format = fixture.by_label("Format").expect("the format select");
    fixture.click(&format).idle(3);
    let webp = fixture.by_label("WebP").expect("the select offers WebP");
    fixture.click(&webp).idle(4);
    let held = held(&host);
    assert_eq!(held[0].1, "jpg", "the first row keeps its format");
    assert_eq!(held[1].1, "webp", "the selected row takes the format");
    assert_eq!(held[2].1, "webp", "and the third row is untouched");
    assert!(cells_of(&fixture, 1).iter().any(|cell| cell == "WEBP"), "the row shows the format it now exports with");
    assert!(cells_of(&fixture, 0).iter().any(|cell| cell == "JPG"), "and the first row still shows its own");
}

/// The arrow keys move which row is selected and Space flips that row's tick box; a tick box left off takes no part in
/// the export.
#[trial]
fn the_arrow_keys_move_the_row_and_space_flips_the_tick() {
    let (host, mut fixture) = dialog(true);
    let first = {
        let mut list = fixture.by_role(Role::LIST_ITEM);
        list.sort_by(|a, b| a.center().y.total_cmp(&b.center().y));
        list[0]
    };
    fixture.click(&first).idle(3);
    fixture.key(Key::ArrowDown, Modifiers::NONE).idle(3);
    assert_eq!(host.borrow().get::<HostedExportAs>().and_then(HostedExportAs::rows).map(|rows| rows.selected), Some(1), "Down moves the row");
    assert_eq!(
        held(&host),
        vec![("big".to_owned(), "jpg".to_owned(), true), ("small".to_owned(), "png".to_owned(), true), ("fill".to_owned(), "webp".to_owned(), false)]
    );
    fixture.key(Key::Space, Modifiers::NONE).idle(3);
    assert!(!held(&host)[1].2, "Space flips the selected row's tick box");
    assert!(held(&host)[0].2, "and no other row's");
    fixture.key(Key::ArrowDown, Modifiers::NONE).idle(3);
    assert_eq!(host.borrow().get::<HostedExportAs>().and_then(HostedExportAs::rows).map(|rows| rows.selected), Some(2), "Down moves on");
    fixture.key(Key::Space, Modifiers::NONE).idle(3);
    assert!(held(&host)[2].2, "an asset with no pixels can be ticked too");
    let names: Vec<String> = ticks(&fixture).iter().map(|tick| fixture.tree().name_of(tick.node).unwrap_or_default().to_owned()).collect();
    for name in ["Big mark", "Small mark", "Fill layer"] {
        assert!(names.iter().any(|named| named == name), "the row named {name} has its own check box: {names:?}");
    }
}

/// The answer carries one entry per ticked row, in the list's order, each with the settings the user confirmed for it
/// and its own output size, so one run writes mixed formats. Where more than one row is ticked, Export asks for a
/// folder.
#[trial]
fn the_answer_carries_each_ticked_row_with_its_own_settings() {
    let (host, mut fixture) = dialog(true);
    let result = export(&host, &mut fixture);
    let ids: Vec<&str> = result.rows.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(ids, ["big", "small"], "the ticked rows, in the list's order");
    let formats: Vec<&str> = result.rows.iter().map(|row| row.options.format.as_str()).collect();
    assert_eq!(formats, ["jpg", "png"], "each row exports with its own settings");
    let sizes: Vec<PixelSize> = result.rows.iter().map(|row| row.size).collect();
    assert_eq!(sizes, [PixelSize::new(400, 300), PixelSize::new(40, 30)], "each row's size comes from its own asset");
    assert_eq!(result.rows.iter().map(|row| row.outputs.len()).collect::<Vec<_>>(), [1, 1], "one file per row before Scale All");
    // The single-file fields describe the row the settings edit: the first row here, a 400 x 300 asset exported at its
    // full size in JPG. The rows carry every row's own settings and size.
    assert_eq!(result.options.format, "jpg", "the single-file options are the selected row's");
    assert_eq!(result.size, PixelSize::new(400, 300), "and the single-file size comes from that row's asset");
    assert_eq!(result.outputs.len(), 1, "one file for that row before Scale All");
}

/// A row whose tick box is off leaves the answer, and a row ticked again comes back: the tick box, not the selection,
/// decides what is written.
#[trial]
fn a_row_left_unticked_is_no_file_in_the_answer() {
    let (host, mut fixture) = dialog(true);
    let tick = ticks(&fixture)[0];
    fixture.click(&tick).idle(4);
    assert!(!held(&host)[0].2, "a press on the box turns the row off");
    let result = export(&host, &mut fixture);
    let ids: Vec<&str> = result.rows.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(ids, ["small"], "the unticked row takes no part");
}

/// The dialog asks the app for an estimate of each row that has pixels, once per change of its settings; the answer
/// shows in the row, and a setting changed after it drops the answer until the app measures the new settings.
#[trial]
fn each_row_asks_the_app_for_its_estimate_once() {
    let (host, mut fixture) = dialog(true);
    let mut asked = host.borrow_mut().get_mut::<HostedExportAs>().map(HostedExportAs::wanted_estimates).unwrap_or_default();
    let ids: Vec<&str> = asked.iter().map(|asked| asked.id.as_str()).collect();
    assert_eq!(ids, ["big", "small"], "the rows with pixels are asked for, the empty one not");
    fixture.idle(6);
    let again = host.borrow_mut().get_mut::<HostedExportAs>().map(HostedExportAs::wanted_estimates).unwrap_or_default();
    assert!(again.is_empty(), "the same settings are asked for once");
    let waiting = cells_of(&fixture, 0);
    assert!(waiting.iter().any(|cell| cell == "…"), "a row waits for the app's estimate: {waiting:?}");
    let first = asked.remove(0);
    if let Some(dialog) = host.borrow_mut().get_mut::<HostedExportAs>() {
        dialog.set_row_estimate(&first.id, &first.asked, Some(SizeEstimate::Exact(2_048)));
    }
    fixture.idle(6);
    let answered = cells_of(&fixture, 0);
    assert_ne!(answered, waiting, "the row shows the size the app answered instead of waiting: {answered:?}");
    assert!(!answered.iter().any(|cell| cell == "…"), "the waiting mark is gone once the answer is in: {answered:?}");
    if let Some(dialog) = host.borrow_mut().get_mut::<HostedExportAs>() {
        dialog.set_json_field(fixture.ctx(), "format", "tif".into()).expect("TIFF is an offered format");
    }
    fixture.idle(6);
    let asked = host.borrow_mut().get_mut::<HostedExportAs>().map(HostedExportAs::wanted_estimates).unwrap_or_default();
    assert_eq!(asked.iter().map(|asked| asked.id.as_str()).collect::<Vec<_>>(), ["big"], "the changed row is asked again");
    let cell = cells_of(&fixture, 0);
    assert!(cell.iter().any(|cell| cell == "…"), "while the new settings wait, the row says so: {cell:?}");
}

/// Without an asset list nothing of the single-file dialog changes: no list is drawn, Export asks for one file named
/// by the settings' format, and the answer carries no rows.
#[trial]
fn a_dialog_over_one_file_draws_no_rows_and_answers_no_rows() {
    let (host, mut fixture) = dialog(false);
    assert!(fixture.by_role(Role::LIST_ITEM).is_empty(), "no asset list is drawn");
    assert!(!fixture.texts().iter().any(|run| run.text == "Image is empty."), "and no asset notice");
    let result = export(&host, &mut fixture);
    assert!(result.rows.is_empty(), "the single-file answer has no rows");
    assert_eq!(result.path, PathBuf::from("/tmp/poster"), "one file, the chosen path");
    assert_eq!(result.size, PixelSize::new(1200, 800));
    assert_eq!(result.outputs.len(), 1, "one file before Scale All");
    assert_eq!(
        host.borrow_mut().get_mut::<HostedExportAs>().map(HostedExportAs::wanted_estimates).unwrap_or_default().len(),
        0,
        "no row estimates are asked for"
    );
}

/// An asset list with nothing in it is the same dialog: no row is drawn, nothing is picked, the settings stay on the
/// dialog's own value and Export still writes one file.
#[trial]
fn an_empty_asset_list_draws_no_rows_and_exports_one_file() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(Vec2::new(1500.0, 1000.0));
    host.borrow_mut().open(HostedExportAs::new("Poster", document(), ExportOptions::default()).assets(Vec::<ExportAsAsset>::new()));
    fixture.idle(10);
    assert!(fixture.by_role(Role::LIST_ITEM).is_empty(), "no rows to pick from");
    assert_eq!(host.borrow().get::<HostedExportAs>().and_then(HostedExportAs::rows).map(|rows| rows.rows.len()), Some(0), "the list is empty");
    let result = export(&host, &mut fixture);
    assert!(result.rows.is_empty(), "and the answer carries none");
    assert_eq!(result.size, PixelSize::new(1200, 800), "the document's own size");
}

/// Automation reads and moves the selection: `row` names the selected asset and the option fields edit that row, so a
/// test or a script sets one row's settings without the pointer.
#[trial]
fn automation_edits_the_row_it_selects() {
    let (host, mut fixture) = dialog(true);
    let shown = {
        let host = host.borrow();
        let dialog = host.get::<HostedExportAs>().expect("the dialog is open");
        StackDialog::json_fields(dialog).expect("the form's fields")
    };
    assert_eq!(shown.get("row").and_then(|row| row.as_str()), Some("big"), "the selected row is a field of the form");
    let set = host.borrow_mut().get_mut::<HostedExportAs>().map(|dialog| dialog.set_json_field(fixture.ctx(), "row", "small".into()));
    assert_eq!(set, Some(Ok(())), "a row of the list is named by its id");
    if let Some(dialog) = host.borrow_mut().get_mut::<HostedExportAs>() {
        dialog.set_json_field(fixture.ctx(), "format", "tif".into()).expect("the format field edits the selected row");
    }
    fixture.idle(6);
    let held = held(&host);
    assert_eq!((held[1].0.as_str(), held[1].1.as_str()), ("small", "tif"), "the second row took the settings");
    assert_eq!(held[0].1, "jpg", "the first row kept its own");
    let refused = host.borrow_mut().get_mut::<HostedExportAs>().map(|dialog| dialog.set_json_field(fixture.ctx(), "row", "nope".into()));
    assert!(refused.is_some_and(|set| set.is_err()), "an id the list has no row for is refused");
}

/// Alt-Reset puts the settings the SELECTED ROW opened with back into that row: its own format and its own size, not
/// the settings the dialog as a whole opened with. The other rows keep their own.
#[trial]
fn reset_puts_the_opening_settings_back_into_the_selected_row() {
    let (host, mut fixture) = over(opening_assets(), Vec2::new(1500.0, 1000.0));
    let rows = row_targets(&fixture);
    fixture.click(&rows[1]).idle(4);
    let format = fixture.by_label("Format").expect("the format select");
    fixture.click(&format).idle(3);
    let tiff = fixture.by_label("TIFF").expect("the select offers TIFF");
    fixture.click(&tiff).idle(4);
    assert_eq!(held(&host)[1].1, "tif", "the selected row took the format");
    assert_eq!(size_row(&fixture, "Width"), "20", "and keeps the pixel size it opened with");
    fixture.hold(Modifiers::ALT).idle(2);
    let reset = fixture.by_label("Reset").expect("Alt turns Cancel into Reset");
    fixture.click(&reset).idle(4);
    let held = held(&host);
    assert_eq!(held[1].1, "webp", "Reset puts the format THIS row opened with back, not the dialog's");
    assert_eq!(size_row(&fixture, "Width"), "20", "and the pixel size this row opened with, fitted to its own asset");
    let thumb = cells_of(&fixture, 1);
    assert!(thumb.iter().any(|cell| cell.contains("20") && cell.contains("15")), "its row shows that size again: {thumb:?}");
    assert_eq!(held[0].1, "jpg", "the other row keeps its own settings");
}

/// The Image Size rows beside the list work on the selected row's asset: they show that asset's sides, and they change
/// with the row the list selects — the document's sides are not what the settings edit.
#[trial]
fn the_size_rows_show_the_selected_rows_asset() {
    let (_host, mut fixture) = dialog(true);
    assert_eq!(size_row(&fixture, "Width"), "400", "the first row's asset is 400 wide");
    assert_eq!(size_row(&fixture, "Height"), "300", "and 300 tall");
    let rows = row_targets(&fixture);
    fixture.click(&rows[1]).idle(4);
    assert_eq!(size_row(&fixture, "Width"), "40", "the size follows the row the list selects");
    assert_eq!(size_row(&fixture, "Height"), "30", "on both of its sides");
}

/// The list measures its columns from the text its rows hold, not from the text they held when the dialog opened: an
/// estimate several characters wide takes the width it needs, where the waiting mark is one character.
#[trial]
fn the_estimate_column_widens_for_the_answer() {
    const SLACK: f32 = 1.0;
    let (host, mut fixture) = dialog(true);
    let cells = cells_of(&fixture, 0);
    assert!(cells.iter().any(|cell| cell == "…"), "the first row waits for its estimate: {cells:?}");
    let before = row_targets(&fixture).remove(0);
    let asked = host.borrow_mut().get_mut::<HostedExportAs>().map(HostedExportAs::wanted_estimates).unwrap_or_default();
    let first = asked.into_iter().find(|asked| asked.id == "big").expect("the first row is asked for its estimate");
    if let Some(dialog) = host.borrow_mut().get_mut::<HostedExportAs>() {
        dialog.set_row_estimate(&first.id, &first.asked, Some(SizeEstimate::Proxy(12_345_678_901)));
    }
    fixture.idle(6);
    let row = row_targets(&fixture).remove(0);
    assert!(row.rect.width() > before.rect.width(), "the row grew by the width the answer needs: {before:?} then {row:?}");
    let answer = fixture
        .texts()
        .iter()
        .filter(|run| row.rect.contains(run.rect.center()))
        .max_by(|a, b| a.rect.center().x.total_cmp(&b.rect.center().x))
        .expect("a row drawn in the list");
    assert!(!answer.text.starts_with('…'), "the row shows the size the app answered: {answer:?}");
    assert!(answer.rect.max.x <= row.rect.max.x + SLACK, "and inside the column it was measured for: {answer:?} in {row:?}");
}

/// A long asset list does not lay all of its rows out inside the dialog's scrolling body: it bounds itself and scrolls
/// its own rows, so the last row is reachable (the collection rule for a long list in a scrolling body).
#[trial]
fn a_long_asset_list_scrolls_its_own_rows_and_reaches_the_last_one() {
    let room = Vec2::new(1200.0, 500.0);
    let (_host, mut fixture) = over(long_list(12), room);
    let list = fixture.by_role(Role::LIST).into_iter().next().expect("the asset list");
    let drawn = row_targets(&fixture).len();
    assert!(drawn < 12, "twelve rows in a {}-point room: the list lays out all {drawn} instead of scrolling them", room.y);
    assert!(list.rect.height() <= room.y, "the list is no taller than the room it stands in: {list:?} in a room of {room:?}");
    let names = rows(&fixture);
    assert!(!names.iter().any(|name| name == "Layer 12"), "the last row starts out of sight: {names:?}");
    // A row of the list is picked, which gives the list its keys and its wheel; the wheel then turns over the rows.
    let first = row_targets(&fixture).remove(0);
    fixture.click(&first).idle(2);
    fixture.frame(vec![pointer::moved(first.center())]).idle(2);
    fixture.frame(vec![pointer::wheel_lines(-10.0)]).idle(4);
    let names = rows(&fixture);
    assert!(names.iter().any(|name| name == "Layer 12"), "the wheel over the list brings its last row into view: {names:?}");
    assert!(row_targets(&fixture).len() < 12, "the well moved, it did not grow to hold every row: {names:?}");
    let last = row_targets(&fixture).pop().expect("a row of the list");
    assert!(last.rect.max.y <= room.y, "and that row lies inside the room the dialog stands in: {last:?} of a {room:?} room");
    for _ in 0..11 {
        fixture.key(Key::ArrowDown, Modifiers::NONE).idle(1);
    }
    let names = rows(&fixture);
    assert!(names.iter().any(|name| name == "Layer 12"), "and the arrow keys reach it without the pointer: {names:?}");
}
