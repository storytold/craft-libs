//! `PropertiesDialog` and `HostedPropertiesDialog`: Reset Panel next to the form's own Reset, and the shown tab and the
//! values as the fields automation reads and sets.

use craft_ui::contract::Bind;
use craft_ui::dialog::{DialogHost, DialogId};
use craft_ui::patterns::form_dialogs::{DialogFields, HostedPropertiesDialog, Param, ParamKind, PropertiesDialog, PropertiesTab, PropertyRow};
use craft_ui::testing::{Fixture, trial};
use egui::{Modifiers, Vec2};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Two tabs: Character resets and holds a row without a default (`extra`), a check box and the row it enables;
/// Paragraph does not reset.
fn tabs() -> Vec<PropertiesTab> {
    vec![
        PropertiesTab::new(
            "Character",
            vec![
                PropertyRow::field(Param::number("size", "Size", 12.0, 1.0, 0)),
                PropertyRow::field(Param::new("extra", "Extra", ParamKind::optional_number(0.0, 100.0, None, 0.0, 0))),
                PropertyRow::field(Param::new("title", "Title", ParamKind::text(""))),
                PropertyRow::field(Param::new("locked", "Locked", ParamKind::bool(false))),
                PropertyRow::field(Param::new("detail", "Detail", ParamKind::text("")).enabled_when("locked", true)),
            ],
        )
        .key("character")
        .resettable(),
        PropertiesTab::new("Paragraph", vec![PropertyRow::field(Param::number("indent", "Indent", 0.0, 1.0, 0))]).key("paragraph"),
    ]
}

#[trial]
fn reset_panel_puts_the_shown_tabs_keys_back_to_their_defaults() {
    let values = Rc::new(RefCell::new(DialogFields::new()));
    let (shown, pressed) = (Rc::clone(&values), Rc::new(Cell::new(false)));
    let seen = Rc::clone(&pressed);
    let mut fixture = Fixture::new(move |ui| {
        let out = PropertiesDialog::new("properties", "Properties", tabs(), Bind::bound(&mut *shown.borrow_mut())).show(ui);
        if out.tab_reset {
            seen.set(true);
        }
    });
    fixture.room(Vec2::new(900.0, 700.0));
    // The first pass opens the dialog with every row's default; `extra` has none.
    fixture.settle();
    assert!(!values.borrow().contains("extra"), "a row without a default opens without the key");
    {
        let mut edited = values.borrow_mut();
        edited.set("size", 20.0);
        edited.set("extra", 5.0);
        edited.set("title", "hello");
        edited.set("indent", 7.0);
    }
    fixture.settle();

    let reset_panel = fixture.by_label("Reset Panel").expect("a resettable tab shows the Reset Panel button");
    fixture.click(&reset_panel).settle();
    assert!(pressed.get(), "the press is reported in the output");
    {
        let after = values.borrow();
        assert_eq!(after.number("size"), Some(12.0), "a keyed row goes back to its default");
        assert_eq!(after.text("title"), Some(""), "a text row goes back to its default");
        assert!(!after.contains("extra"), "a row without a default loses its key");
        assert_eq!(after.number("indent"), Some(7.0), "the other tab's keys keep their values");
        assert!(after.was_reset("character"), "the reset tab is marked by its key");
    }

    // The form's own Reset still restores everything from when the dialog opened, the other tab included.
    fixture.hold(Modifiers::ALT).idle(2);
    let reset = fixture.by_label("Reset").expect("Alt turns Cancel into Reset");
    fixture.click(&reset).idle(2);
    fixture.hold(Modifiers::NONE).settle();
    assert_eq!(values.borrow().number("indent"), Some(0.0), "Alt-Reset puts the other tab's keys back to the opening values");
}

/// The fields of the open dialog `id`, as automation lists them.
fn listed(host: &Rc<RefCell<DialogHost>>, id: DialogId) -> Value {
    host.borrow().open_dialogs().into_iter().find(|open| open.id == id).and_then(|open| open.fields).unwrap_or(Value::Null)
}

#[trial]
fn the_shown_tab_and_the_values_are_json_fields() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(Vec2::new(900.0, 700.0));
    let ctx = fixture.ctx().clone();
    let dialog =
        HostedPropertiesDialog::new("properties", DialogFields::new(), |values| PropertiesDialog::new("properties", "Properties", tabs(), values));
    let id = host.borrow_mut().open(dialog.previews());
    fixture.idle(10);
    let set = |field: &str, value: Value| host.borrow_mut().set_json_field(&ctx, id, field, value);

    let before = listed(&host, id);
    assert_eq!(before.get("tab"), Some(&json!("character")), "the shown tab is listed by its key: {before}");
    assert_eq!(before.get("preview"), Some(&json!(true)), "the Preview check box starts on: {before}");

    assert_eq!(set("tab", json!("paragraph")), Ok(()), "a tab key switches the tab");
    fixture.idle(4);
    assert_eq!(listed(&host, id).get("tab"), Some(&json!("paragraph")));
    assert!(fixture.by_label("Indent").is_some(), "the dialog shows the tab the field named");
    assert!(set("tab", json!("nope")).is_err(), "an unknown tab key is refused");
    assert_eq!(listed(&host, id).get("tab"), Some(&json!("paragraph")), "a refused tab changes nothing");

    assert_eq!(set("size", json!(30.0)), Ok(()), "a field of another tab is set");
    assert_eq!(listed(&host, id).get("size"), Some(&json!(30.0)));
    assert!(set("detail", json!("x")).is_err(), "a row its check box disables is refused");
    assert!(set("noSuchField", json!(1)).is_err(), "an unknown field is refused");

    assert_eq!(set("preview", json!(false)), Ok(()), "preview takes a bool");
    fixture.idle(4);
    let preview = fixture.by_label("Preview").expect("the dialog has the Preview check box");
    fixture.click(&preview).idle(4);
    assert_eq!(listed(&host, id).get("preview"), Some(&json!(true)), "the check box writes the field the host lists");
}
