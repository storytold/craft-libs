//! `ColorPicker`, its hex pair field, `ColorReadout` and `ColorChoice`: the edit rules the picker owns.

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::Mixed;
use craft_color::ui::{ColorPickerDialog, ColorReadout, PickedColor};
use craft_host::{AppInfo, Host};
use craft_theme::{ModelValues, Rgba, Srgba};
use craft_ui::contract::Bind;
use craft_ui::dialog::{DialogHost, DialogResult, dialog_id};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, vec2};

fn type_into(fixture: &mut Fixture, label: &str, text: &str) {
    let field = fixture.by_label(label).unwrap_or_else(|| panic!("the picker shows a {label} field"));
    fixture.click(&field);
    fixture.key(Key::A, Modifiers::COMMAND);
    fixture.type_text(text);
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
}

/// Two items with different G, B and alpha, shown by a readout the way a multi-selection host does.
fn mixed_readout(items: Rc<RefCell<[Rgba; 2]>>) -> Fixture {
    let mut fixture = Fixture::new(move |ui| {
        let mut items = items.borrow_mut();
        let out = ColorReadout::new("Color", Bind::given(Mixed::<Rgba>::Mixed)).show(ui);
        for edit in &out.channel {
            if let Some(Some(byte)) = edit.event.value() {
                for item in items.iter_mut() {
                    item.0[edit.channel.index()] = *byte;
                }
            }
        }
        if let Some(color) = out.events.value() {
            items.fill(*color);
        }
    });
    fixture.room(vec2(480.0, 60.0));
    fixture.settle();
    fixture
}

/// A channel typed while the colour is mixed sets that channel of every item and keeps each item's other channels
/// and alpha; a hex value sets the whole colour of both.
#[trial]
fn a_channel_typed_into_a_mixed_readout_keeps_each_items_other_channels() {
    let items = Rc::new(RefCell::new([Rgba([10, 20, 30, 100]), Rgba([40, 50, 60, 200])]));
    let mut fixture = mixed_readout(items.clone());
    type_into(&mut fixture, "Red", "255");
    assert_eq!(*items.borrow(), [Rgba([255, 20, 30, 100]), Rgba([255, 50, 60, 200])]);
    type_into(&mut fixture, "Hex color", "336699");
    let whole = items.borrow()[0];
    assert_eq!(&whole.0[..3], &[0x33, 0x66, 0x99], "the hex sets the colour");
    assert_eq!(items.borrow()[1], whole, "and sets it for both items");
}

/// What the dialog recalls on open: Only Web Colors snaps the opening colour and the compare swatch, so OK without an
/// edit changes nothing; with it off, the model values `kept` gives stay on the current and the opening colour.
#[trial]
fn a_dialog_reopened_keeps_only_web_colors_and_recalls_the_values_kept_on_ok() {
    let dialogs = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&dialogs);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(vec2(900.0, 700.0));
    fixture.host(std::sync::Arc::new(Host::builder(AppInfo { id: "picker-test", name: "Picker test", version: "0" }).build()));
    let ctx = fixture.ctx().clone();
    let id = dialog_id(("color-picker", "fill"));
    let opened = Srgba::from_hex("#123456").expect("a colour");

    dialogs.borrow_mut().open(ColorPickerDialog::new("fill", "Fill", opened));
    fixture.idle(6);
    let set = dialogs.borrow_mut().set_json_field(&ctx, id, "webColors", serde_json::json!(true));
    assert_eq!(set, Ok(()), "Only Web Colors is a field of the dialog");
    assert!(dialogs.borrow_mut().confirm::<ColorPickerDialog>(&ctx), "OK answers");
    assert!(dialogs.borrow_mut().take::<ColorPickerDialog>().is_some());

    dialogs.borrow_mut().open(ColorPickerDialog::new("fill", "Fill", opened));
    fixture.idle(6);
    let hex = |picked: Option<PickedColor>| picked.map(|picked| picked.color.to_hex());
    let (opening, current) = dialogs.borrow().get::<ColorPickerDialog>().map(|d| (hex(d.opening()), hex(d.current()))).unwrap_or_default();
    assert_eq!(opening.as_deref(), Some("#003366"), "the recalled Only Web Colors snaps the opening colour");
    assert_eq!(current, opening, "and the colour OK would answer");
    // Turn it off so the next open starts plain; OK stores that.
    let set = dialogs.borrow_mut().set_json_field(&ctx, id, "webColors", serde_json::json!(false));
    assert_eq!(set, Ok(()));
    assert!(dialogs.borrow_mut().confirm::<ColorPickerDialog>(&ctx));
    let answered = dialogs.borrow_mut().take::<ColorPickerDialog>().and_then(DialogResult::ok).map(|result| hex(Some(result.picked)));
    assert_eq!(answered, Some(opening), "OK without an edit answers the snapped colour");

    let values = ModelValues::new("cmyk", &[10.0, 20.0, 30.0, 40.0]);
    dialogs.borrow_mut().open(ColorPickerDialog::new("fill", "Fill", opened).kept(values));
    fixture.idle(6);
    let (opening, current) = dialogs.borrow().get::<ColorPickerDialog>().map(|d| (d.opening(), d.current())).unwrap_or_default();
    assert_eq!(opening.and_then(|picked| picked.model), Some(values), "the compare swatch's colour keeps the typed values");
    assert_eq!(current.and_then(|picked| picked.model), Some(values), "and so does the current colour");

    // Current (the compare swatch's current half) and Reset return to the opening colour with its typed values.
    let edited =
        |dialogs: &Rc<RefCell<DialogHost>>| dialogs.borrow().get::<ColorPickerDialog>().and_then(|d| d.current()).and_then(|picked| picked.model);
    let set = dialogs.borrow_mut().set_json_field(&ctx, id, "color", serde_json::json!("#ff0000"));
    assert_eq!(set, Ok(()), "the colour is a field of the dialog");
    fixture.idle(4);
    assert_ne!(edited(&dialogs), Some(values), "the edit replaces the typed values");
    let restore = fixture.by_label("Restore current color").expect("the compare swatch's current half");
    fixture.click(&restore).idle(4);
    assert_eq!(edited(&dialogs), Some(values), "Current returns the typed values");

    let set = dialogs.borrow_mut().set_json_field(&ctx, id, "color", serde_json::json!("#ff0000"));
    assert_eq!(set, Ok(()));
    fixture.idle(4);
    assert_ne!(edited(&dialogs), Some(values));
    fixture.hold(Modifiers::ALT).idle(2);
    let reset = fixture.by_label("Reset").expect("Alt turns Cancel into Reset");
    fixture.click(&reset).idle(4);
    fixture.hold(Modifiers::NONE).idle(2);
    assert_eq!(edited(&dialogs), Some(values), "Reset returns the typed values");
}
