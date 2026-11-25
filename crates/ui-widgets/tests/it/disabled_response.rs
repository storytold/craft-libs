//! A disabled component reports its effective state on the response it returns, without disabling its native parent.

use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Props as _, Role, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::popover::Popover;
use craft_ui::runtime::overlay::CloseReason;
use craft_ui::testing::{Fixture, trial};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[trial]
fn a_button_reports_explicit_and_inherited_disabled_state_on_its_response() {
    let enabled = Rc::new(Cell::new(false));
    let parent_enabled = Rc::new(Cell::new(true));
    let clicks = Rc::new(Cell::new(0usize));
    let observed = Rc::new(RefCell::new(None));
    let (shown_enabled, shown_parent, seen_clicks, seen) = (enabled.clone(), parent_enabled.clone(), clicks.clone(), observed.clone());
    let mut fixture = Fixture::new(move |ui| {
        ui.add_enabled_ui(shown_parent.get(), |ui| {
            let parent = ui.is_enabled();
            let out = Button::new("Use Library Folder").enabled(shown_enabled.get()).show(ui);
            if out.clicked() {
                seen_clicks.set(seen_clicks.get() + 1);
            }
            *seen.borrow_mut() = Some((out.response.id, out.response.enabled(), parent));
        });
    });
    fixture.settle();
    let (id, reported, parent) = observed.borrow().as_ref().copied().expect("the button's output");
    assert!(parent, "the native parent is enabled");
    assert_eq!(fixture.tree().role(id), Some(Role::BUTTON), "the response addresses the button's own node");
    assert_eq!(fixture.tree().name(id), Some("Use Library Folder"));
    assert!(fixture.tree().is_disabled(id));
    assert!(!reported, "Button::enabled(false) reports false on its response under an enabled parent");

    enabled.set(true);
    fixture.idle(2);
    let (current, reported, parent) = observed.borrow().as_ref().copied().expect("the enabled output");
    assert_eq!(current, id);
    assert!(parent && reported && !fixture.tree().is_disabled(id), "an enabled button reports enabled");

    parent_enabled.set(false);
    fixture.idle(2);
    let (current, reported, parent) = observed.borrow().as_ref().copied().expect("the inherited-disabled output");
    assert_eq!(current, id);
    assert!(!parent && !reported && fixture.tree().is_disabled(id), "a disabled parent disables the button");

    parent_enabled.set(true);
    enabled.set(false);
    fixture.idle(2);
    let (current, reported, parent) = observed.borrow().as_ref().copied().expect("the disabled output again");
    assert_eq!(current, id);
    assert!(parent && !reported && fixture.tree().is_disabled(id));
    let target = *fixture.by_role(Role::BUTTON).first().expect("the disabled button");
    fixture.click(&target).settle();
    assert_eq!(clicks.get(), 0, "a disabled button does not activate");

    enabled.set(true);
    fixture.idle(2);
    let target = *fixture.by_role(Role::BUTTON).first().expect("the re-enabled button");
    fixture.click(&target).settle();
    assert_eq!(clicks.get(), 1, "the re-enabled button activates once");
}

#[trial]
fn a_disabled_opener_closes_its_popover_by_the_anchor_gone_rule() {
    let enabled = Rc::new(Cell::new(true));
    let closes = Rc::new(RefCell::new(Vec::new()));
    let opens = Rc::new(Cell::new(false));
    let (opener_enabled, seen_closes, seen_open) = (enabled.clone(), closes.clone(), opens.clone());
    let mut fixture = Fixture::new(move |ui| {
        let opener = Button::new("Options").enabled(opener_enabled.get()).show(ui);
        let mut is_open = seen_open.get();
        let shown = Popover::new("Options card", Bind::bound(&mut is_open)).opened_by(&opener.response).show(ui, |ui, _| {
            Text::new("Body").show(ui);
        });
        seen_open.set(is_open);
        if let Some(why) = shown.output.closed {
            seen_closes.borrow_mut().push(why);
        }
    });
    fixture.settle();
    opens.set(true);
    fixture.idle(3);
    assert!(opens.get(), "the popover stays open while its opener is enabled");
    assert!(closes.borrow().is_empty(), "nothing closes it: {:?}", closes.borrow());
    enabled.set(false);
    fixture.idle(3);
    assert_eq!(closes.borrow().first(), Some(&CloseReason::AnchorGone), "the disabled opener's popover closes: {:?}", closes.borrow());
}
