//! The W × H × Resolution rules both crop bars share: the pixel target the fields name, and Front Image with the
//! Perspective limits, and the Crop bar's units menu on W and H.

use craft_behavior::Mixed;
use craft_behavior::kurbo::Size;
use craft_draw::ui::crop::{CropBar, CropDocKey, CropDocument, CropLength, CropRatio, CropSettings, CropSizeFields, ResolutionUnit};
use craft_ui::bars::{Bar, BarKind};
use craft_ui::contract::{Role, ShowContainer as _};
use craft_ui::runtime::env::{ColumnSize, Env, PointSize, UnitKind, Units, with_env};
use craft_ui::testing::{Fixture, Frames, Target, pointer, trial};
use egui::{Key, Modifiers, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

/// The units of a scope whose point is `point_size`.
fn units(point_size: PointSize) -> Units {
    let mut frames = Frames::new();
    let mut units = None;
    frames.frame(vec![], |ui| {
        with_env(
            ui,
            |scope| {
                scope.values.set(point_size);
            },
            |ui| units = Some(Env::of(ui).units()),
        )
    });
    units.expect("the frame ran")
}

fn document(width: f64, height: f64, resolution: f64) -> CropDocument {
    CropDocument::new(CropDocKey::new(1), Size::new(width, height), resolution)
}

fn fields(width: Option<(f64, UnitKind)>, height: Option<(f64, UnitKind)>, resolution: &str, unit: ResolutionUnit) -> CropSizeFields {
    CropSizeFields {
        width: width.map(|(value, unit)| CropLength::new(value, unit)),
        height: height.map(|(value, unit)| CropLength::new(value, unit)),
        resolution: resolution.to_owned(),
        resolution_unit: unit,
        ..Default::default()
    }
}

fn near(value: Option<f64>, expected: f64) -> bool {
    value.is_some_and(|v| (v - expected).abs() < 1e-6)
}

#[trial]
fn size_targets_convert_units_and_perspective_limits_round_inward() {
    let postscript = units(PointSize::PostScript);
    let traditional = units(PointSize::Traditional);
    let doc = document(800.0, 600.0, 72.0);
    let inch = |value: f64| Some((value, UnitKind::Inches));

    // Inches convert at the typed resolution, else at the document's.
    let typed = fields(inch(4.0), inch(5.0), "300", ResolutionUnit::PixelsPerInch).target(&doc, &postscript);
    assert!(near(typed.width, 1200.0) && near(typed.height, 1500.0) && near(typed.resolution, 300.0), "{typed:?}");
    let untyped = fields(inch(2.0), inch(1.0), "", ResolutionUnit::PixelsPerInch).target(&doc, &postscript);
    assert!(near(untyped.width, 144.0) && near(untyped.height, 72.0) && untyped.resolution.is_none(), "{untyped:?}");

    // Pixels ignore the resolution; centimetres and the px/cm resolution convert through 2.54.
    let pixels =
        fields(Some((100.0, UnitKind::Pixels)), Some((50.0, UnitKind::Pixels)), "300", ResolutionUnit::PixelsPerInch).target(&doc, &postscript);
    assert!(near(pixels.width, 100.0) && near(pixels.height, 50.0), "{pixels:?}");
    let centimetres = fields(Some((2.54, UnitKind::Centimetres)), inch(1.0), "10", ResolutionUnit::PixelsPerCentimetre).target(&doc, &postscript);
    assert!(near(centimetres.resolution, 25.4) && near(centimetres.width, 25.4) && near(centimetres.height, 25.4), "{centimetres:?}");

    // A point is 1/72 in for PostScript and 1/72.27 in for Traditional.
    let points = fields(Some((72.0, UnitKind::Points)), None, "100", ResolutionUnit::PixelsPerInch);
    assert!(near(points.target(&doc, &postscript).width, 100.0));
    assert!(near(points.target(&doc, &traditional).width, 100.0 * 72.0 / 72.27));

    // A side that is empty, zero or past the largest length has no target; the other side keeps its own.
    let one_side = fields(inch(4.0), None, "", ResolutionUnit::PixelsPerInch).target(&doc, &postscript);
    assert!(near(one_side.width, 288.0) && one_side.height.is_none(), "{one_side:?}");
    let zero = fields(Some((0.0, UnitKind::Pixels)), Some((1e12, UnitKind::Pixels)), "", ResolutionUnit::PixelsPerInch).target(&doc, &postscript);
    assert!(zero.width.is_none() && zero.height.is_none(), "{zero:?}");

    // The settings' target needs the W × H × Resolution mode and both sides.
    let mut settings = CropSettings { size: fields(inch(4.0), inch(5.0), "300", ResolutionUnit::PixelsPerInch), ..Default::default() };
    assert!(settings.target(&doc, &postscript).is_none(), "a ratio that is not W x H x Resolution has no target");
    settings.ratio = CropRatio::Size;
    let target = settings.target(&doc, &postscript).expect("both sides");
    assert!((target.width - 1200.0).abs() < 1e-6 && (target.height - 1500.0).abs() < 1e-6 && near(target.resolution, 300.0));
    settings.size.height = None;
    assert!(settings.target(&doc, &postscript).is_none(), "one side is a free crop");

    // Front Image with the Perspective limits keeps three decimals and rounds inward.
    let high = document(800.0, 600.0, 30_000.0);
    let mut per_inch = CropSizeFields::default();
    per_inch.front_image(&high, Some((1.0, 29_999.999)));
    assert_eq!(per_inch.resolution, "29999.999");
    assert_eq!(per_inch.width, Some(CropLength::new(800.0, UnitKind::Pixels)));
    assert_eq!(per_inch.height, Some(CropLength::new(600.0, UnitKind::Pixels)));
    let mut per_cm = CropSizeFields { resolution_unit: ResolutionUnit::PixelsPerCentimetre, ..Default::default() };
    per_cm.front_image(&high, Some((0.394, 11_811.023)));
    assert_eq!(per_cm.resolution, "11811.023", "30000 / 2.54 rounds to 11811.024, past the limit");
    assert!(per_cm.resolution_ppi().is_some_and(|ppi| ppi <= 29_999.999), "the stored value converts back inside the limit");
    // Without limits the resolution keeps two decimals.
    let mut plain = CropSizeFields::default();
    plain.front_image(&document(10.0, 10.0, 300.456), None);
    assert_eq!(plain.resolution, "300.46");

    // Clear empties the three fields and puts the unit back to px/in; swap exchanges W and H.
    let mut cleared = fields(inch(4.0), inch(5.0), "300", ResolutionUnit::PixelsPerCentimetre);
    cleared.swap();
    assert_eq!((cleared.width, cleared.height), (Some(CropLength::new(5.0, UnitKind::Inches)), Some(CropLength::new(4.0, UnitKind::Inches))));
    cleared.clear_size();
    assert_eq!(cleared, CropSizeFields::default());
}

/// A Crop bar in W × H × Resolution mode over `start`, on a 72 ppi document with `columns` as its column size.
fn crop_bar(start: CropSizeFields, columns: Option<ColumnSize>) -> (Fixture, Rc<RefCell<CropSettings>>) {
    let settings = Rc::new(RefCell::new(CropSettings { ratio: CropRatio::Size, size: start, ..CropSettings::default() }));
    let seen = Rc::clone(&settings);
    let doc = document(800.0, 600.0, 72.0);
    let doc = columns.map_or(doc, |columns| doc.column_size(columns));
    let mut fixture = Fixture::new(move |ui| {
        let mut settings = seen.borrow_mut();
        Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
            let _ = CropBar::new(&mut settings, &doc).show(ui);
        });
    });
    fixture.room(Vec2::new(1600.0, 80.0)).settle();
    (fixture, settings)
}

/// W, H and Resolution, left to right.
fn size_fields(fixture: &Fixture) -> [Target; 3] {
    let mut spins = fixture.by_role(Role::SPIN_BUTTON);
    spins.sort_by(|a, b| a.rect.min.x.total_cmp(&b.rect.min.x));
    <[Target; 3]>::try_from(spins).expect("W, H and Resolution are number fields")
}

fn right_click(fixture: &mut Fixture, target: &Target) {
    let at = target.center();
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::secondary(at, true)]);
    fixture.frame(vec![pointer::secondary(at, false)]);
    fixture.settle();
}

/// The open menu's rows, top to bottom: name, checked, target.
fn menu_rows(fixture: &Fixture) -> Vec<(String, bool, Target)> {
    let roles = [Role::MENU_ITEM, Role::MENU_ITEM_CHECK_BOX, Role::MENU_ITEM_RADIO];
    let mut rows: Vec<Target> = roles.into_iter().flat_map(|role| fixture.by_role(role)).collect();
    rows.sort_by(|a, b| a.rect.min.y.total_cmp(&b.rect.min.y));
    let tree = fixture.tree();
    rows.into_iter()
        .map(|row| {
            let checked = tree.toggled_of(row.node) == Some(Mixed::Same(true));
            (tree.name_of(row.node).unwrap_or_default().to_owned(), checked, row)
        })
        .collect()
}

fn pick(fixture: &mut Fixture, field: &Target, name: &str) {
    right_click(fixture, field);
    let rows = menu_rows(fixture);
    let names: Vec<&str> = rows.iter().map(|(row, ..)| row.as_str()).collect();
    let row = rows.iter().find(|(row, ..)| row == name).map(|(.., row)| *row);
    let row = row.unwrap_or_else(|| panic!("{name} is in the units menu: {names:?}"));
    fixture.click(&row);
    fixture.settle();
}

fn type_into(fixture: &mut Fixture, field: &Target, text: &str) {
    fixture.click(field);
    fixture.key(Key::A, Modifiers::COMMAND);
    if text.is_empty() {
        fixture.key(Key::Backspace, Modifiers::NONE);
    } else {
        fixture.type_text(text);
    }
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
}

/// A right click on W or H opens its units menu with the field's unit checked; a pick converts that field alone and
/// keeps its length, a typed count of columns is kept in Columns, and an empty field keeps the picked unit for the
/// number typed next.
#[trial]
fn the_units_menu_of_w_and_h_converts_one_field_and_keeps_the_length() {
    let postscript = units(PointSize::PostScript);
    let doc = document(800.0, 600.0, 72.0);
    let inches = fields(Some((4.0, UnitKind::Inches)), Some((6.0, UnitKind::Inches)), "", ResolutionUnit::PixelsPerInch);
    let before = inches.target(&doc, &postscript);
    let (mut fixture, settings) = crop_bar(inches, Some(ColumnSize::new(180.0, 12.0)));
    let [w, h, _] = size_fields(&fixture);

    right_click(&mut fixture, &w);
    let rows = menu_rows(&fixture);
    let names: Vec<&str> = rows.iter().map(|(name, ..)| name.as_str()).collect();
    assert_eq!(names.len(), 7, "six lengths and Columns: {names:?}");
    assert_eq!((names.first().copied(), names.last().copied()), (Some("Pixels"), Some("Columns")), "{names:?}");
    let checked: Vec<&str> = rows.iter().filter(|(_, checked, _)| *checked).map(|(name, ..)| name.as_str()).collect();
    assert_eq!(checked, ["Inches"], "the field's unit is checked");
    fixture.key(Key::Escape, Modifiers::NONE).settle();

    pick(&mut fixture, &w, "Centimetres");
    let size = settings.borrow().size.clone();
    assert_eq!(size.width.map(|w| w.unit), Some(UnitKind::Centimetres), "W takes the picked unit");
    assert_eq!(size.height, Some(CropLength::new(6.0, UnitKind::Inches)), "H keeps its own unit");
    let after = size.target(&doc, &postscript);
    assert!(near(after.width, before.width.unwrap_or_default()), "the same length in centimetres: {before:?} {after:?}");

    type_into(&mut fixture, &h, "2 col");
    assert_eq!(settings.borrow().size.height, Some(CropLength::new(2.0, UnitKind::Columns)), "a typed count of columns");
    type_into(&mut fixture, &h, "3 in");
    assert_eq!(settings.borrow().size.height, Some(CropLength::new(3.0, UnitKind::Inches)), "a typed length in a Columns field");

    type_into(&mut fixture, &w, "");
    assert_eq!(settings.borrow().size.width, None, "a cleared field is unset");
    pick(&mut fixture, &w, "Millimetres");
    type_into(&mut fixture, &w, "5");
    assert_eq!(settings.borrow().size.width, Some(CropLength::new(5.0, UnitKind::Millimetres)), "a bare number in the picked unit");
}

/// A length smaller than the least value a field takes in the picked unit (1 px at 300 px/in is a third of a
/// hundredth of an inch) keeps its size when its unit changes.
#[trial]
fn a_picked_unit_keeps_a_length_below_the_fields_least_value() {
    let postscript = units(PointSize::PostScript);
    let doc = document(800.0, 600.0, 72.0);
    let pixel = fields(Some((1.0, UnitKind::Pixels)), None, "300", ResolutionUnit::PixelsPerInch);
    let before = pixel.target(&doc, &postscript);
    let (mut fixture, settings) = crop_bar(pixel, None);
    let [w, ..] = size_fields(&fixture);

    pick(&mut fixture, &w, "Inches");
    let size = settings.borrow().size.clone();
    assert_eq!(size.width.map(|w| w.unit), Some(UnitKind::Inches), "W takes the picked unit");
    let after = size.target(&doc, &postscript);
    assert!(near(after.width, before.width.unwrap_or_default()), "the same width in inches: {before:?} {after:?}");

    // Entering the field and committing it unedited keeps the length too.
    let [w, ..] = size_fields(&fixture);
    fixture.click(&w);
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
    let kept = settings.borrow().size.target(&doc, &postscript);
    assert!(near(kept.width, before.width.unwrap_or_default()), "an unedited commit keeps the width: {before:?} {kept:?}");
}

/// Without a column size the menu offers no Columns and a length in columns names no size; with one, two columns
/// are wider than one.
#[trial]
fn columns_need_a_column_size() {
    let (mut fixture, _) = crop_bar(CropSizeFields::default(), None);
    let [w, ..] = size_fields(&fixture);
    right_click(&mut fixture, &w);
    let names: Vec<String> = menu_rows(&fixture).into_iter().map(|(name, ..)| name).collect();
    assert_eq!(names.len(), 6, "{names:?}");
    assert!(!names.iter().any(|name| name == "Columns"), "{names:?}");

    let postscript = units(PointSize::PostScript);
    let columns = fields(Some((1.0, UnitKind::Columns)), Some((2.0, UnitKind::Columns)), "", ResolutionUnit::PixelsPerInch);
    let with = columns.target(&document(800.0, 600.0, 72.0).column_size(ColumnSize::new(180.0, 12.0)), &postscript);
    assert!(with.width.zip(with.height).is_some_and(|(one, two)| two > one), "{with:?}");
    let without = columns.target(&document(800.0, 600.0, 72.0), &postscript);
    assert!(without.width.is_none() && without.height.is_none(), "{without:?}");
}
