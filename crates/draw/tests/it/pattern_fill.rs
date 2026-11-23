//! The Pattern Fill dialog's automation and the Angle row of its editor.

use std::cell::RefCell;
use std::rc::Rc;

use craft_draw::ui::pattern::{PatternColors, PatternFillDialog, PatternFillEditor, PatternFillOptions, PatternSwatch};
use craft_theme::Srgba;
use craft_ui::contract::{Bind, Role};
use craft_ui::dialog::{DialogHost, DialogId, DialogResult};
use craft_ui::testing::{Fixture, Target, trial};
use egui::{Key, Modifiers, vec2};
use serde_json::{Value, json};

fn swatches() -> Vec<PatternSwatch> {
    vec![PatternSwatch::new("bubbles", "Bubbles"), PatternSwatch::new("dots", "Dots")]
}

fn fields(host: &DialogHost, id: DialogId) -> Value {
    host.open_dialogs().into_iter().find(|open| open.id == id).and_then(|open| open.fields).unwrap_or(Value::Null)
}

/// A bitmap pattern lists its fields, refuses an unknown or malformed one, takes a scale held to its range, and OK
/// answers the set values; a preset pattern refuses the bitmap rows as inactive.
#[trial]
fn pattern_fill_automation_lists_sets_and_confirms_its_fields() {
    let ctx = egui::Context::default();
    let mut host = DialogHost::new();

    let id = host.open(PatternFillDialog::new(PatternFillOptions::bitmap("bubbles"), swatches()));
    assert_eq!(fields(&host, id)["scale"], json!(100.0), "the opening scale is listed");
    assert!(host.set_json_field(&ctx, id, "depth", json!(3)).is_err(), "an unknown field is refused");
    assert!(host.set_json_field(&ctx, id, "pattern", json!("missing")).is_err(), "a pattern the app does not offer is refused");
    assert_eq!(host.set_json_field(&ctx, id, "scale", json!(250)), Ok(()));
    assert_eq!(host.set_json_field(&ctx, id, "scale", json!(5000)), Ok(()), "a scale past the range is held to it");
    assert_eq!(fields(&host, id)["scale"], json!(*PatternFillOptions::SCALE.end()));
    assert_eq!(host.set_json_field(&ctx, id, "scale", json!(250)), Ok(()));
    assert_eq!(host.set_json_field(&ctx, id, "pattern", json!("dots")), Ok(()));
    assert!(host.confirm::<PatternFillDialog>(&ctx), "OK is enabled");
    let options = host.take::<PatternFillDialog>().and_then(DialogResult::ok).expect("OK answers the options");
    assert_eq!((options.pattern.as_str(), options.scale), ("dots", 250.0), "the answer carries the set values");

    let colors = PatternColors::new(Srgba::BLACK, Srgba::WHITE);
    let id = host.open(PatternFillDialog::new(PatternFillOptions::preset("bubbles", colors), swatches()));
    assert!(host.set_json_field(&ctx, id, "scale", json!(200)).is_err(), "a bitmap row is inactive in preset mode");
    assert_eq!(host.set_json_field(&ctx, id, "colors", json!({ "foreground": Srgba::WHITE, "background": Srgba::BLACK })), Ok(()));
}

/// The editor over `held`, writing each reported edit back into it.
fn editor(held: &Rc<RefCell<PatternFillOptions>>) -> Fixture {
    let value = Rc::clone(held);
    let mut fixture = Fixture::new(move |ui| {
        let list = swatches();
        let given = value.borrow().clone();
        let out = PatternFillEditor::new("Pattern", Bind::given(given), &list).show(ui);
        for event in out.events.iter() {
            event.apply(&mut *value.borrow_mut());
        }
    });
    fixture.settle();
    fixture
}

/// The one control of `role` named `name`.
fn named(fixture: &Fixture, role: Role, name: &str) -> Target {
    let mut found: Vec<Target> = fixture.by_role(role).into_iter().filter(|t| fixture.tree().name_of(t.node) == Some(name)).collect();
    assert_eq!(found.len(), 1, "one {role:?} named {name}");
    found.remove(0)
}

/// The text the control shows as its value.
fn value_of(fixture: &Fixture, target: &Target) -> String {
    fixture.tree().value_of(target.node).unwrap_or_default().to_owned()
}

/// The Angle row has a degree field beside the dial (#16346): a typed angle is held and the dial follows it, a typed
/// angle past the range is held to it, and a dial drag shows in the field.
#[trial]
fn pattern_fill_angle_takes_a_typed_degree_and_keeps_the_dial_in_step() {
    let held = Rc::new(RefCell::new(PatternFillOptions::bitmap("bubbles")));
    let mut fixture = editor(&held);
    let field = named(&fixture, Role::SPIN_BUTTON, "Angle");
    let dial_before = value_of(&fixture, &fixture.by_role(Role::SLIDER).remove(0));

    let type_angle = |fixture: &mut Fixture, text: &str| {
        fixture.click(&field).settle();
        fixture.key(Key::A, Modifiers::COMMAND).type_text(text);
        fixture.key(Key::Tab, Modifiers::NONE).settle();
    };
    type_angle(&mut fixture, "45");
    assert_eq!(held.borrow().angle, 45.0, "the typed angle is held");
    let dial = fixture.by_role(Role::SLIDER).remove(0);
    assert_ne!(value_of(&fixture, &dial), dial_before, "the dial follows the typed angle");

    type_angle(&mut fixture, "500");
    assert_eq!(held.borrow().angle, 180.0, "a typed angle past the range is held to its end");

    let field_before = value_of(&fixture, &field);
    let dial = fixture.by_role(Role::SLIDER).remove(0);
    fixture.drag(&dial, vec2(0.0, 30.0));
    fixture.settle();
    assert_ne!(held.borrow().angle, 180.0, "the dial drag changed the angle");
    let field = named(&fixture, Role::SPIN_BUTTON, "Angle");
    assert_ne!(value_of(&fixture, &field), field_before, "the field follows the dial");
}
