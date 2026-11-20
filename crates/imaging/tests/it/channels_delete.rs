//! Deleting one process row uses its own key, from the context menu, keyboard and trash button.

use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Event, Key, Modifiers, PointerButton, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::ui::channels::{ChannelKind, ChannelRow, ChannelsCommands, ChannelsEvent, ChannelsFooterButton, ChannelsPanel};

const RGB: u8 = 0;
const GREEN: u8 = 1;
const BLUE: u8 = 2;
type Events = Rc<RefCell<Vec<ChannelsEvent<u8>>>>;

fn panel(targets: &[u8], allow_color_deletion: bool) -> (Fixture, Events) {
    panel_with_permission(targets, Rc::new(Cell::new(allow_color_deletion)))
}

fn panel_with_permission(targets: &[u8], allow_color_deletion: Rc<Cell<bool>>) -> (Fixture, Events) {
    let mut rows =
        [(RGB, ChannelKind::Composite, "RGB"), (GREEN, ChannelKind::Color, "Green"), (BLUE, ChannelKind::Color, "Blue")].map(|(key, kind, name)| {
            let mut row = ChannelRow::new(key, kind, name);
            row.targeted = targets.contains(&key);
            row
        });
    let events = Events::default();
    let reported = Rc::clone(&events);
    let mut fixture = Fixture::new(move |ui| {
        for row in &mut rows {
            row.allow_color_deletion = allow_color_deletion.get();
        }
        reported.borrow_mut().extend(ChannelsPanel::new(&rows).show(ui).events);
    });
    fixture.room(Vec2::new(900.0, 700.0)).idle(6);
    (fixture, events)
}

fn context(fixture: &mut Fixture, name: &str) {
    let pos = fixture.by_label(name).expect("a named channel row").center();
    fixture.frame(vec![pointer::moved(pos)]);
    for pressed in [true, false] {
        fixture.frame(vec![Event::PointerButton { pos, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE }]);
    }
    fixture.idle(2);
}

fn deleted(events: &Events) -> Vec<u8> {
    events
        .borrow()
        .iter()
        .filter_map(|event| match event {
            ChannelsEvent::Command { command: ChannelsCommands::Delete, key: Some(key), .. } => Some(std::slice::from_ref(key)),
            ChannelsEvent::Footer { button: ChannelsFooterButton::Delete, rows, .. }
            | ChannelsEvent::FooterDrop { button: ChannelsFooterButton::Delete, rows, .. } => Some(rows.as_slice()),
            _ => None,
        })
        .flatten()
        .copied()
        .collect()
}

#[trial]
fn process_channel_delete_uses_the_clicked_or_dragged_row() {
    let (mut menu, events) = panel(&[RGB, GREEN, BLUE], true);
    context(&mut menu, "Green");
    assert!(menu.by_label("Duplicate Channel").is_some(), "the process row opens its regular context menu");
    let delete = menu.by_label("Delete Channel").expect("the process row's Delete Channel menu item");
    menu.click(&delete).idle(2);
    assert_eq!(deleted(&events), [GREEN], "the menu deletes the right-clicked row, even with the composite targeted");

    let (mut drag, events) = panel(&[RGB, GREEN, BLUE], true);
    let row = drag.by_label("Green").expect("the Green row");
    let trash = drag.by_label("Delete current channel").expect("the trash button");
    drag.drag(&row, trash.center() - row.center()).idle(3);
    assert_eq!(deleted(&events), [GREEN], "the drop deletes the grabbed row, even with the composite targeted");
}

#[trial]
fn process_channel_delete_from_the_keyboard_and_footer_requires_one_target() {
    let (mut single, events) = panel(&[GREEN], true);
    let row = single.by_label("Green").unwrap();
    single.click(&row);
    assert!(single.focused_role().is_some(), "clicking the process row gives the list focus");
    single.key(Key::Delete, Modifiers::NONE).idle(3);
    assert_eq!(deleted(&events), [GREEN], "Delete on one focused process row");
    events.borrow_mut().clear();
    let trash = single.by_label("Delete current channel").unwrap();
    single.click(&trash).idle(2);
    assert_eq!(deleted(&events), [GREEN], "the trash button deletes a single targeted process row");

    for (targets, opted_in) in [(&[GREEN, BLUE][..], true), (&[GREEN][..], false), (&[RGB, GREEN, BLUE][..], true)] {
        let (mut blocked, events) = panel(targets, opted_in);
        let name = if targets.contains(&RGB) { "RGB" } else { "Green" };
        let row = blocked.by_label(name).unwrap();
        blocked.click(&row).key(Key::Delete, Modifiers::NONE).idle(2);
        let trash = blocked.by_label("Delete current channel").unwrap();
        blocked.click(&trash).idle(2);
        assert!(deleted(&events).is_empty(), "ambiguous, composite and unconfigured targets have no process delete");
        if !opted_in || targets.contains(&RGB) {
            blocked.drag(&row, trash.center() - row.center()).idle(3);
            assert!(deleted(&events).is_empty(), "this row also refuses a trash drop");
            context(&mut blocked, name);
            assert!(blocked.by_label("Delete Channel").is_none(), "this row's context menu keeps the default");
        }
    }
}

#[trial]
fn revoking_process_channel_delete_during_a_drag_blocks_the_drop() {
    let allow = Rc::new(Cell::new(true));
    let (mut fixture, events) = panel_with_permission(&[GREEN], Rc::clone(&allow));
    let row = fixture.by_label("Green").expect("the Green row").center();
    let trash = fixture.by_label("Delete current channel").expect("the trash button").center();
    fixture.frame(vec![pointer::moved(row)]);
    fixture.frame(vec![Event::PointerButton { pos: row, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
    fixture.frame(vec![pointer::moved(row + Vec2::new(16.0, 0.0))]);
    fixture.frame(vec![pointer::moved(trash)]);
    allow.set(false);
    fixture.frame(Vec::new());
    fixture.frame(vec![Event::PointerButton { pos: trash, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
    fixture.idle(3);
    assert!(deleted(&events).is_empty(), "the current row revokes delete before the drop");
}
