//! A footer bar's buttons are Tab stops of their own: Tab moves between them and the arrows leave the focus where it is.

use craft_ui::buttons::Button;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::sections::{DialogRole, FooterBar, FooterButton};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};

/// Which of `labels` holds the focus.
fn focused<'a>(fixture: &Fixture, labels: &[&'a str]) -> Option<&'a str> {
    let focused = craft_ui::testing::access::focused_key(fixture.tree());
    labels.iter().copied().find(|label| fixture.by_label(label).is_some_and(|target| target.node == focused))
}

#[trial]
fn each_footer_button_is_its_own_tab_stop() {
    let mut fixture = Fixture::new(|ui| {
        Stack::new().show(ui, |ui, _| {
            Button::new("Before").show(ui);
            let _ =
                FooterBar::buttons([FooterButton::new("OK", DialogRole::Accept), FooterButton::new("Cancel", DialogRole::Cancel)]).show_footer(ui);
        });
    });
    fixture.idle(4);
    let labels = ["Before", "OK", "Cancel"];
    fixture.key(Key::Tab, Modifiers::NONE).idle(2);
    assert_eq!(focused(&fixture, &labels), Some("Before"), "Tab reaches the first button");
    fixture.key(Key::Tab, Modifiers::NONE).idle(2);
    let first = focused(&fixture, &labels).filter(|label| *label != "Before").expect("Tab reaches a footer button");
    fixture.key(Key::ArrowRight, Modifiers::NONE).idle(2);
    assert_eq!(focused(&fixture, &labels), Some(first), "the arrows do not move between footer buttons");
    fixture.key(Key::Tab, Modifiers::NONE).idle(2);
    let second = focused(&fixture, &labels).expect("Tab stays among the labelled stops");
    assert!(second != first && second != "Before", "Tab moves to the other footer button");
}

#[trial]
fn only_stateful_footer_actions_announce_a_pressed_state() {
    use craft_behavior::{IconId, Mixed};
    use craft_ui::contract::PopupKind;
    use craft_ui::sections::FooterAction;
    use craft_ui::testing::focus::AccessActionKind;
    use std::cell::Cell;

    let pressed = Cell::new(false);
    let mut fixture = Fixture::new(move |ui| {
        let actions = [
            FooterAction::new(IconId::new("trash-2"), "Delete Selected Item"),
            FooterAction::new(IconId::new("list"), "Show Swatch Kinds").pressed(pressed.get()),
            FooterAction::new(IconId::new("plus"), "Add New Effect").popup(PopupKind::Menu),
        ];
        if FooterBar::actions(actions).show_footer(ui).picked == Some(1) {
            pressed.set(!pressed.get());
        }
    });
    fixture.settle();
    let delete = fixture.by_label("Delete Selected Item").expect("the ordinary action");
    let filter = fixture.by_label("Show Swatch Kinds").expect("the stateful action");
    let effect = fixture.by_label("Add New Effect").expect("the menu action");
    assert_eq!(fixture.tree().toggled_of(delete.node), None, "an ordinary action is not announced as a toggle");
    assert_eq!(fixture.tree().has_popup_of(effect.node), Some(PopupKind::Menu), "a menu action still announces its menu");
    assert_eq!(fixture.tree().toggled_of(effect.node), None, "a menu by itself does not make an action a toggle");
    assert_eq!(fixture.tree().toggled_of(filter.node), Some(Mixed::Same(false)), "an explicitly off action is announced as off");
    fixture.click(&filter).settle();
    assert_eq!(fixture.tree().toggled_of(filter.node), Some(Mixed::Same(true)), "the action is announced as on after activation");
    fixture.access(AccessActionKind::Click, &filter).settle();
    assert_eq!(fixture.tree().toggled_of(filter.node), Some(Mixed::Same(false)), "the action is announced as off after the next activation");
}
