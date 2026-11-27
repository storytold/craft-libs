//! Text AccessKit and the field's visible states: value, `SetValue`, selection, INVALID, selectable text.

use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::TextField;
use std::cell::Cell;
use std::rc::Rc;

/// A masked field's node carries no value, where the same field unmasked carries its text.
#[trial]
fn a_masked_field_exposes_no_value() {
    let (open, masked) = (Rc::new(Cell::new(egui::Id::NULL)), Rc::new(Cell::new(egui::Id::NULL)));
    let (plain, hidden) = (open.clone(), masked.clone());
    let mut fixture = Fixture::new(move |ui| {
        plain.set(TextField::new("Name", Bind::given("secret".to_owned())).id_salt("name").show(ui).response.id);
        hidden.set(TextField::new("Password", Bind::given("secret".to_owned())).id_salt("password").masked().show(ui).response.id);
    });
    fixture.settle();
    assert_eq!(fixture.tree().value(open.get()), Some("secret"));
    assert_eq!(fixture.tree().value(masked.get()), None, "a masked node has no value to read out");
}

/// A number field in a side-labelled field that keeps its own name is named by it and described by the caption; without
/// `keeps_name` the caption names it.
#[trial]
fn a_number_field_that_keeps_its_name_is_described_by_its_caption() {
    use craft_ui::contract::ShowContainer as _;
    use craft_ui::form::Field;
    use craft_ui::testing::access::{NodeKey, name};
    use craft_ui::text::NumberField;

    for keeps in [true, false] {
        let field = Rc::new(Cell::new(egui::Id::NULL));
        let seen = field.clone();
        let mut fixture = Fixture::new(move |ui| {
            Field::new("X").show(ui, |ui, _| {
                let mut value = Some(12.0);
                let out = NumberField::new("Clone Source X Offset", Bind::bound(&mut value)).id_salt("x").keeps_name(keeps).show(ui);
                seen.set(out.response.id);
            });
        });
        fixture.settle();
        let tree = fixture.tree();
        let node = NodeKey::of(field.get());
        let described: Vec<Option<&str>> = tree.described_by(field.get()).into_iter().map(|key| tree.name_of(key)).collect();
        if keeps {
            assert_eq!(name(tree, node), Some("Clone Source X Offset"), "the field keeps its own name");
            assert!(described.contains(&Some("X")), "the caption describes the field: {described:?}");
        } else {
            assert_eq!(name(tree, node), Some("X"), "the caption names the field");
        }
    }
}

/// A number field whose value is not a number shows the dash and its node carries no numeric value, only the range;
/// with a number it carries that number.
#[trial]
fn a_number_field_with_no_number_has_no_numeric_value() {
    use craft_ui::text::NumberField;

    for (value, read) in [(f64::NAN, None), (12.0, Some(12.0))] {
        let field = Rc::new(Cell::new(egui::Id::NULL));
        let seen = field.clone();
        let mut fixture = Fixture::new(move |ui| {
            let mut shown = Some(value);
            seen.set(NumberField::new("Offset", Bind::bound(&mut shown)).id_salt("offset").show(ui).response.id);
        });
        fixture.settle();
        let numeric = fixture.tree().numeric(field.get()).expect("the field's node is numeric");
        assert_eq!(numeric.value, read, "the numeric value of a field holding {value}");
    }
}

/// A number field's value reads its number, then the unit it draws after it ("160 pt"), as the field shows them; a
/// value that is not a number shows the dash alone, draws no unit and reads its text alone. Guards against the text
/// part's value, which is the editable text, leaving out the unit.
#[trial]
fn a_number_field_reads_its_unit_after_its_number() {
    use craft_ui::runtime::env::UnitKind;
    use craft_ui::text::NumberField;

    for value in [160.0, f64::NAN] {
        let field = Rc::new(Cell::new((egui::Id::NULL, egui::Rect::NOTHING)));
        let seen = field.clone();
        let mut fixture = Fixture::new(move |ui| {
            let mut shown = Some(value);
            let out = NumberField::new("Radius", Bind::bound(&mut shown)).unit_kind(UnitKind::Points).id_salt("radius").show(ui);
            seen.set((out.response.id, out.response.rect));
        });
        fixture.settle();
        let (id, rect) = field.get();
        let tree = fixture.tree();
        let read = tree.value(id).unwrap_or_default();
        if value.is_finite() {
            let mut painted: Vec<_> = fixture.texts().iter().filter(|text| !text.text.is_empty() && rect.contains_rect(text.rect)).collect();
            painted.sort_by(|a, b| a.rect.left().total_cmp(&b.rect.left()));
            let shown: Vec<&str> = painted.iter().map(|text| text.text.as_str()).collect();
            assert_eq!(shown.len(), 2, "the number and its unit: {shown:?}");
            assert_eq!(read, shown.join(" "), "the value reads the number, then the unit");
        } else {
            assert_eq!(read, tree.run_texts(id).concat(), "a field that draws no unit reads its text alone");
        }
    }
}
