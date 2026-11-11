//! Roving groups, arena members and part stops: what Tab reaches and what a component reads as focused.

use super::{control, focused, id, tap};
use craft_behavior::{Input, Intent, NavShape, Roving};
use craft_ui::runtime::focus::KeyUse;
use craft_ui::testing::focus::{Group, Stop, activated_member, intents_of, is_focused, register_group, register_stop, widget};
use craft_ui::testing::{Frames, trial};
use egui::{Key, Modifiers, Ui};
use std::cell::{Cell, RefCell};

/// Skeleton `r8_member_focus::member_focus_moves_with_arrows_and_reads_through_the_focus_service`: a radio
/// group is one Tab stop that arrives at the selected member; arrows move focus between members and leave the
/// value; Space activates the focused member through the group; egui focus stays on the root.
#[trial]
fn a_roving_group_is_one_tab_stop_and_its_members_read_their_own_focus() {
    let mut frames = Frames::new();
    let reads = RefCell::new([false; 4]);
    let activated = Cell::new((false, None));
    let mut show = |ui: &mut Ui| {
        control(ui, "before");
        widget(ui, id("group"));
        for member in ["m1", "m2", "m3"] {
            register_stop(ui.ctx(), Stop::new(id(member), KeyUse::of(Input::Pointer)).root(id("group")).in_group(id("group")));
        }
        register_group(ui.ctx(), Group::new(id("group"), Roving::new(NavShape::Flat)).selected((id("m2"), None)));
        let ctx = ui.ctx();
        *reads.borrow_mut() = ["m1", "m2", "m3", "group"].map(|name| is_focused(ctx, id(name)));
        let mut group = intents_of(ctx, Some(id("group")), false);
        if group.take(Intent::Activate) {
            activated.set((true, activated_member(ctx, id("group"))));
        }
        let member_moves = intents_of(ctx, Some(id("m3")), false).take_move();
        assert!(member_moves.is_none(), "a member gets no move: the group's roving step takes it");
    };
    frames.idle(2, &mut show);
    tap(&mut frames, Key::Tab, Modifiers::NONE, &mut show);
    assert_eq!(focused(&frames.ctx), Some(id("before")));
    tap(&mut frames, Key::Tab, Modifiers::NONE, &mut show);
    assert_eq!(focused(&frames.ctx), Some(id("group")), "Tab reaches the group as one stop");
    assert_eq!(*reads.borrow(), [false, true, false, false], "Tab lands on the selected member, not the root");
    tap(&mut frames, Key::ArrowRight, Modifiers::NONE, &mut show);
    assert_eq!(*reads.borrow(), [false, false, true, false]);
    assert_eq!(focused(&frames.ctx), Some(id("group")), "egui focus stays on the arena root");
    tap(&mut frames, Key::Home, Modifiers::NONE, &mut show);
    assert_eq!(*reads.borrow(), [true, false, false, false], "Home goes to the first member");
    tap(&mut frames, Key::ArrowRight, Modifiers::NONE, &mut show);
    tap(&mut frames, Key::ArrowRight, Modifiers::NONE, &mut show);
    tap(&mut frames, Key::Space, Modifiers::NONE, &mut show);
    assert_eq!(activated.get(), (true, Some(id("m3"))), "Space activates the focused member through the group");
    tap(&mut frames, Key::Tab, Modifiers::NONE, &mut show);
    assert_eq!(focused(&frames.ctx), Some(id("before")), "Tab leaves the group, wrapping to the first stop");
    assert_eq!(*reads.borrow(), [false; 4], "focus leaving the group clears every member's focus");
}
