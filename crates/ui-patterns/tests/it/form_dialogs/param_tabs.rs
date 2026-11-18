//! A tabbed `ParamDialog` hosted in a `DialogHost`: every tab's values open, validate and answer together, the hosted
//! dialog opens on its tab and the host moves it, and a read-only tab shows its values without a control.

use craft_ui::contract::Bind;
use craft_ui::dialog::{DialogHost, DialogResult, dialog_id};
use craft_ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind, ParamTab};
use craft_ui::runtime::env::Label;
use craft_ui::testing::{Fixture, trial};
use egui::Vec2;
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The note above the read-only tab's rows.
const NOTE: &str = "Fonts are listed as the file embeds them.";

/// A Document Properties dialog with three tabs; the last is read-only.
fn properties(fields: &mut DialogFields) -> ParamDialog<'_> {
    let tabs = vec![
        ParamTab::new("Description", vec![Param::new("title", "Title", ParamKind::text(""))]),
        ParamTab::new("Security", vec![Param::new("level", "Level", ParamKind::int(Some(0), Some(3), 1))]),
        ParamTab::new("Fonts", vec![Param::new("fonts", "Embedded Fonts", ParamKind::text("Serif"))]).read_only(Some(Label::from(NOTE))),
    ];
    ParamDialog::new("properties", "Document Properties", Vec::<Param>::new()).tabs(tabs).values(Bind::bound(fields))
}

/// `act` on the open `HostedParamDialog`; `None` when none is open.
fn on_dialog<T>(host: &Rc<RefCell<DialogHost>>, act: impl FnOnce(&mut HostedParamDialog) -> T) -> Option<T> {
    host.borrow_mut().get_mut::<HostedParamDialog>().map(act)
}

// A value in a tab that is not shown opens with its default, is checked by its row when the host sets it and comes
// back at OK; the hosted dialog opens on its `tab`, `set_tab` and a press on the strip move it (once, in a frame that
// runs twice), and the read-only tab shows its note and its value and takes no typing.
#[trial]
fn a_tabbed_dialog_answers_every_tabs_values_and_opens_on_its_tab() {
    let (host, twice) = (Rc::new(RefCell::new(DialogHost::new())), Rc::new(Cell::new(false)));
    let (shown, again) = (Rc::clone(&host), Rc::clone(&twice));
    let mut fixture = Fixture::new(move |ui| {
        shown.borrow_mut().run(ui);
        if again.get() && ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("the frame runs twice");
        }
    });
    fixture.room(Vec2::new(1200.0, 900.0));
    let ctx = fixture.ctx().clone();
    let opening = DialogFields::from_pairs([("level", FieldValue::Int(2))]);
    host.borrow_mut().open(HostedParamDialog::new(&ctx, "properties", opening, properties).tab(1));
    assert_eq!(on_dialog(&host, |dialog| dialog.shown_tab()), Some(1), "the dialog opens on its tab");

    // Before any pass: the hidden tabs' rows opened with their defaults and take or refuse a set as shown rows do.
    let id = dialog_id("properties");
    let listed = host.borrow().open_dialogs().into_iter().find(|dialog| dialog.id == id).and_then(|dialog| dialog.fields);
    let listed = listed.expect("the dialog lists its fields");
    assert_eq!((listed["level"].as_i64(), listed["fonts"].clone()), (Some(2), json!("Serif")), "every tab's values open: {listed}");
    assert_eq!(host.borrow_mut().set_json_field(&ctx, id, "title", json!("Report")), Ok(()), "a hidden tab's row takes a value");
    assert!(host.borrow_mut().set_json_field(&ctx, id, "level", json!(9)).is_err(), "a row checks a value past its range");
    assert!(host.borrow_mut().set_json_field(&ctx, id, "fonts", json!("Sans")).is_err(), "automation is refused a read-only tab's row");

    fixture.idle(6);
    assert!(fixture.by_label("Level").is_some(), "the opening tab's rows show");
    assert!(fixture.by_label("Title").is_none(), "another tab's rows do not");

    on_dialog(&host, |dialog| dialog.set_tab(0));
    fixture.idle(4);
    let title = fixture.by_label("Title").expect("set_tab shows the first tab's rows");
    assert_eq!(fixture.tree().value_of(title.node), Some("Report"), "the value set while hidden shows");
    assert_eq!(on_dialog(&host, |dialog| dialog.shown_tab()), Some(0));

    let fonts = fixture.by_label("Fonts").expect("the strip has the Fonts tab");
    twice.set(true);
    fixture.click(&fonts);
    assert_eq!(fixture.passes(), 2, "the frame ran a second pass");
    twice.set(false);
    fixture.idle(4);
    assert_eq!(on_dialog(&host, |dialog| dialog.shown_tab()), Some(2), "a press on the strip shows its tab");
    assert!(fixture.by_label(NOTE).is_some(), "the read-only tab shows its note");
    let field = fixture.by_label("Embedded Fonts").expect("the read-only row shows");
    assert_eq!(fixture.tree().value_of(field.node), Some("Serif"), "the read-only row shows its value");
    fixture.click(&field).settle();
    fixture.type_text("Sans").settle();
    let held = on_dialog(&host, |dialog| dialog.fields().text("fonts").map(str::to_owned)).flatten();
    assert_eq!(held.as_deref(), Some("Serif"), "the read-only row takes no typing");

    let ok = fixture.by_label("OK").expect("the OK button");
    fixture.click(&ok).idle(4);
    let answered = host.borrow_mut().take_keyed::<HostedParamDialog>().map(|(result, _)| result).and_then(DialogResult::ok);
    let fields = answered.expect("OK answers").fields;
    assert_eq!(fields.text("title"), Some("Report"), "a tab not shown at OK answers its value");
    assert_eq!(fields.int("level"), Some(2), "the opening value of another tab answers");
    assert_eq!(fields.text("fonts"), Some("Serif"), "the read-only tab answers its value");
}
