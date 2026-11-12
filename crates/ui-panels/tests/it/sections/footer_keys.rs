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
