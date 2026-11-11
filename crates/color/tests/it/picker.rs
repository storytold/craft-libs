//! `ColorPicker`, its hex pair field, `ColorReadout` and `ColorChoice`: the edit rules the picker owns.

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::Mixed;
use craft_color::ui::ColorReadout;
use craft_theme::Rgba;
use craft_ui::contract::Bind;
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
