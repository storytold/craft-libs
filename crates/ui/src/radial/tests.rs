//! Behavior tests for the radial menu: its geometry, and the ring driven by egui pointer and key
//! events in an 800 by 600 pt window.

use egui::accesskit::Role as AkRole;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use egui_kittest::{Harness, kittest::NodeT, kittest::Queryable};

use super::*;

const LABELS: [&str; 8] = ["Copy", "Paste", "Group", "Ungroup", "Lock", "Hide", "Undo", "Redo"];
const MIDDLE: Pos2 = pos2(400.0, 300.0);

struct App {
    items: Vec<RadialItem>,
    events: Vec<RadialEvent>,
    other: Option<Id>,
    other_rect: Rect,
    other_clicks: usize,
}

fn menu() -> RadialMenu {
    RadialMenu::new(Id::unique("test-ring"))
}

fn items(count: usize) -> Vec<RadialItem> {
    LABELS.iter().take(count).map(|l| RadialItem::new(l.to_lowercase(), *l)).collect()
}

fn harness(items: Vec<RadialItem>) -> Harness<'static, App> {
    let mut h = Harness::builder().with_size(vec2(800.0, 600.0)).build_ui_state(
        |ui, app: &mut App| {
            if let Some(event) = menu().show(ui.ctx(), &app.items, &RadialLabels::default()) {
                app.events.push(event);
            }
            // A panel drawn after the ring that also closes on Esc.
            ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
            let other = ui.button("Other");
            app.other = Some(other.id);
            app.other_rect = other.rect;
            app.other_clicks += usize::from(other.clicked());
        },
        App { items, events: Vec::new(), other: None, other_rect: Rect::NOTHING, other_clicks: 0 },
    );
    h.step();
    h
}

fn open(h: &mut Harness<'_, App>, at: Pos2, held: bool) {
    menu().open(&h.ctx, at, held);
    h.step();
    h.step();
}

fn key(h: &mut Harness<'_, App>, key: Key, modifiers: Modifiers) {
    h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    h.step();
    h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
    h.step();
}

fn move_to(h: &mut Harness<'_, App>, at: Pos2) {
    h.input_mut().events.push(Event::PointerMoved(at));
    h.step();
}

fn click(h: &mut Harness<'_, App>, at: Pos2) {
    move_to(h, at);
    for pressed in [true, false] {
        h.input_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
}

fn focused(h: &Harness<'_, App>) -> Option<Id> {
    h.ctx.memory(|m| m.focused())
}

/// Where item `index` of `count` sits around the middle of the window.
fn slot(index: usize, count: usize) -> Pos2 {
    MIDDLE + slot_direction(index, count) * RADIUS
}

#[test]
fn items_go_clockwise_from_the_top_evenly_spaced() {
    let up = slot_direction(0, 8);
    let right = slot_direction(2, 8);
    let down = slot_direction(4, 8);
    let left = slot_direction(6, 8);
    assert!(up.y < -0.99 && right.x > 0.99 && down.y > 0.99 && left.x < -0.99, "{up:?} {right:?} {down:?} {left:?}");
    for count in 1..=MAX_ITEMS {
        let angles: Vec<f32> = (0..count).map(|i| slot_angle(i, count)).collect();
        assert!(angles.windows(2).all(|w| w[0] < w[1]), "{count} items: {angles:?}");
        let gaps: Vec<f32> = angles.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps.iter().all(|g| (g - gaps[0]).abs() < 1e-4), "{count} items, even gaps: {gaps:?}");
    }
}

#[test]
fn a_ring_opened_near_an_edge_stays_on_screen() {
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let reach = menu().reach();
    for at in [pos2(0.0, 0.0), pos2(800.0, 0.0), pos2(0.0, 600.0), pos2(800.0, 600.0), pos2(-50.0, 300.0), pos2(400.0, 900.0)] {
        let center = fit_center(at, reach, screen);
        assert!(screen.contains_rect(Rect::from_center_size(center, Vec2::splat(2.0 * reach))), "opened at {at:?}, centered at {center:?}");
    }
    assert_eq!(fit_center(MIDDLE, reach, screen), MIDDLE, "a ring with room opens where it was asked");
    let small = Rect::from_min_size(Pos2::ZERO, vec2(100.0, 100.0));
    assert_eq!(fit_center(pos2(10.0, 90.0), reach, small), small.center(), "too small a screen centers the ring");
    assert_eq!(fit_center(pos2(f32::NAN, 0.0), reach, screen), screen.center());
}

#[test]
fn a_long_caption_near_an_edge_stays_on_screen() {
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let size = vec2(400.0, 20.0);
    for anchor in [pos2(116.0, 300.0), pos2(684.0, 300.0), pos2(400.0, 595.0)] {
        let rect = caption_rect(anchor, size, screen);
        assert!(screen.contains_rect(rect), "under {anchor:?}: {rect:?}");
        assert_eq!(rect.size(), size);
    }
    assert_eq!(caption_rect(MIDDLE, size, screen).center(), MIDDLE, "a caption with room stays centered");
}

#[test]
fn a_point_hits_the_item_in_its_direction() {
    let m = menu();
    let (inner, outer) = (m.inner_radius(), m.outer_radius());
    for count in 1..=MAX_ITEMS {
        for index in 0..count {
            assert_eq!(hit(MIDDLE, slot(index, count), count, inner, outer), RingHit::Slot(index), "item {index} of {count}");
        }
        assert_eq!(hit(MIDDLE, MIDDLE, count, inner, outer), RingHit::Center);
        assert_eq!(hit(MIDDLE, MIDDLE + vec2(outer + 1.0, 0.0), count, inner, outer), RingHit::Outside);
    }
    let between = MIDDLE + (slot_direction(0, 8) + slot_direction(1, 8)).normalized() * RADIUS;
    assert!(matches!(hit(MIDDLE, between, 8, inner, outer), RingHit::Slot(0 | 1)), "on the boundary of items 0 and 1");
    assert_eq!(hit(MIDDLE, slot(0, 4), 0, inner, outer), RingHit::Outside, "an empty ring has no sectors");
}

#[test]
fn arrows_and_tab_move_focus_around_the_ring() {
    let m = menu();
    let mut list = items(8);
    list[3].enabled = false;
    let mut h = harness(list);
    open(&mut h, MIDDLE, false);
    assert_eq!(focused(&h), Some(m.item_id(0)), "the first item takes focus");

    key(&mut h, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(focused(&h), Some(m.item_id(2)), "Right goes to the item on the right");
    key(&mut h, Key::ArrowDown, Modifiers::NONE);
    assert_eq!(focused(&h), Some(m.item_id(4)));

    key(&mut h, Key::Tab, Modifiers::SHIFT);
    assert_eq!(focused(&h), Some(m.item_id(2)), "Shift+Tab skips the disabled item 3");
    for _ in 0..4 {
        key(&mut h, Key::Tab, Modifiers::NONE);
    }
    assert_eq!(focused(&h), Some(m.item_id(7)));
    key(&mut h, Key::Tab, Modifiers::NONE);
    assert_eq!(focused(&h), Some(m.close_id()), "after the last item, the close button");
    key(&mut h, Key::Tab, Modifiers::NONE);
    assert_eq!(focused(&h), Some(m.item_id(0)), "Tab wraps");
    assert!(h.state().events.is_empty());
}

#[test]
fn an_arrow_on_the_opening_frame_stays_in_the_ring() {
    let m = menu();
    let mut h = harness(items(8));
    m.open(&h.ctx, MIDDLE, false);
    h.input_mut().events.push(Event::Key { key: Key::ArrowDown, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.step();
    h.step();
    assert_eq!(focused(&h), Some(m.item_id(4)));
}

#[test]
fn an_arrow_pressed_and_released_in_one_frame_still_moves() {
    let m = menu();
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    for pressed in [true, false] {
        h.input_mut().events.push(Event::Key { key: Key::ArrowRight, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
    }
    h.step();
    h.step();
    assert_eq!(focused(&h), Some(m.item_id(2)));
}

#[test]
fn the_key_that_opens_the_ring_does_not_pick() {
    let m = menu();
    let mut h = Harness::builder().with_size(vec2(800.0, 600.0)).build_ui_state(
        |ui, app: &mut App| {
            if !menu().is_open(ui.ctx()) && ui.input(|i| i.key_pressed(Key::Enter)) {
                menu().open(ui.ctx(), MIDDLE, false);
            }
            if let Some(event) = menu().show(ui.ctx(), &app.items, &RadialLabels::default()) {
                app.events.push(event);
            }
        },
        App { items: items(8), events: Vec::new(), other: None, other_rect: Rect::NOTHING, other_clicks: 0 },
    );
    h.step();
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert!(h.state().events.is_empty());
    assert!(m.is_open(&h.ctx), "Enter opened the ring and picked nothing");
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert_eq!(h.state().events, vec![RadialEvent::Picked("copy".into())], "the next Enter picks the first item");
}

#[test]
fn a_held_opening_key_repeating_does_not_pick() {
    let m = menu();
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, true);
    for _ in 0..3 {
        h.input_mut().events.push(Event::Key { key: Key::Space, physical_key: None, pressed: true, repeat: true, modifiers: Modifiers::NONE });
        h.step();
    }
    assert!(h.state().events.is_empty());
    assert!(m.is_open(&h.ctx));
}

#[test]
fn shift_enter_picks_like_enter() {
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    key(&mut h, Key::Enter, Modifiers::SHIFT);
    assert_eq!(h.state().events, vec![RadialEvent::Picked("copy".into())]);
}

#[test]
fn enter_picks_the_focused_item_and_escape_closes() {
    let m = menu();
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    key(&mut h, Key::ArrowLeft, Modifiers::NONE);
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert_eq!(h.state().events, vec![RadialEvent::Picked("undo".into())], "Left is item 6 of 8");
    assert!(!m.is_open(&h.ctx));

    open(&mut h, MIDDLE, false);
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert_eq!(h.state().events.last(), Some(&RadialEvent::Closed));
    assert!(!m.is_open(&h.ctx));
}

#[test]
fn a_click_on_an_item_returns_it_once() {
    let m = menu();
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    click(&mut h, slot(2, 8));
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Picked("group".into())]);
    assert!(!m.is_open(&h.ctx));

    open(&mut h, MIDDLE, false);
    let between = MIDDLE + (slot_direction(5, 8) + slot_direction(6, 8) * 0.2).normalized() * RADIUS;
    click(&mut h, between);
    assert_eq!(h.state().events.last(), Some(&RadialEvent::Picked("hide".into())), "a click in an item's sector picks it");
}

#[test]
fn a_click_outside_or_on_the_middle_closes_without_a_pick() {
    let m = menu();
    let mut h = harness(items(4));
    open(&mut h, MIDDLE, false);
    let under = h.state().other_rect.center();
    move_to(&mut h, under);
    h.step();
    assert_eq!(h.ctx.layer_id_at(under).map(|l| l.order), Some(egui::Order::Foreground), "the open ring covers the window");
    click(&mut h, under);
    assert_eq!(h.state().events, vec![RadialEvent::Closed]);
    assert_eq!(h.state().other_clicks, 0, "the button under the click does not see it");
    assert!(!m.is_open(&h.ctx));

    open(&mut h, MIDDLE, false);
    click(&mut h, MIDDLE);
    assert_eq!(h.state().events, vec![RadialEvent::Closed, RadialEvent::Closed]);
}

#[test]
fn closing_asks_for_another_pass() {
    let m = menu();
    let mut h = harness(items(4));
    open(&mut h, MIDDLE, false);
    h.run_ok();
    assert!(!h.ctx.has_requested_repaint(), "an open ring at rest asks for nothing");
    m.close(&h.ctx);
    assert!(h.ctx.has_requested_repaint(), "the pass that drops the ring's layer runs at once");
}

#[test]
fn a_click_on_a_disabled_item_picks_nothing() {
    let m = menu();
    let mut list = items(4);
    list[1].enabled = false;
    let mut h = harness(list);
    open(&mut h, MIDDLE, false);
    click(&mut h, slot(1, 4));
    assert!(h.state().events.is_empty());
    assert!(m.is_open(&h.ctx));
}

#[test]
fn releasing_the_key_picks_the_item_the_pointer_moved_toward() {
    let m = menu();
    let mut h = harness(items(8));
    move_to(&mut h, MIDDLE);
    open(&mut h, MIDDLE, true);
    // A flick past the ring still counts while the key is held.
    move_to(&mut h, MIDDLE + vec2(RADIUS * 3.0, 0.0));
    m.release(&h.ctx);
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Picked("group".into())]);
    assert!(!m.is_open(&h.ctx));
}

#[test]
fn a_flick_past_a_ring_moved_on_screen_aims_from_where_the_pointer_started() {
    let m = menu();
    let corner = pos2(10.0, 10.0);
    // From the corner to the right, over the ring and past it; from the ring's moved center
    // these points lie up, and up and to the right.
    for to in [pos2(150.0, 30.0), pos2(250.0, 10.0)] {
        let mut h = harness(items(8));
        move_to(&mut h, corner);
        open(&mut h, corner, true);
        move_to(&mut h, to);
        m.release(&h.ctx);
        h.step();
        assert_eq!(h.state().events, vec![RadialEvent::Picked("group".into())], "the item on the right, released at {to:?}");
    }
}

#[test]
fn a_repeating_open_keeps_the_ring_and_its_gesture() {
    let m = menu();
    let mut h = harness(items(8));
    if let Some(other) = h.state().other {
        h.ctx.memory_mut(|mem| mem.request_focus(other));
    }
    move_to(&mut h, MIDDLE);
    open(&mut h, MIDDLE, true);
    move_to(&mut h, MIDDLE + vec2(RADIUS * 3.0, 0.0));
    open(&mut h, MIDDLE + vec2(RADIUS * 3.0, 0.0), true);
    m.release(&h.ctx);
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Picked("group".into())], "the flick still counts from the first open");
    assert_eq!(focused(&h), h.state().other, "focus goes back to the widget focused before the first open");
}

#[test]
fn releasing_without_moving_leaves_the_ring_open() {
    let m = menu();
    let mut h = harness(items(8));
    move_to(&mut h, MIDDLE);
    open(&mut h, MIDDLE, true);
    m.release(&h.ctx);
    h.step();
    assert!(h.state().events.is_empty());
    assert!(m.is_open(&h.ctx), "a tap opens the ring for a click");

    // Opened from the keyboard away from a pointer that has not moved.
    m.close(&h.ctx);
    move_to(&mut h, slot(2, 8));
    let away = pos2(200.0, 300.0);
    open(&mut h, away, true);
    m.release(&h.ctx);
    h.step();
    assert!(h.state().events.is_empty(), "a still pointer picks nothing");
    assert!(m.is_open(&h.ctx));
}

#[test]
fn focus_goes_to_the_first_enabled_item_and_back_on_close() {
    let m = menu();
    let mut list = items(3);
    list[0].enabled = false;
    let mut h = harness(list);
    let other = h.state().other;
    if let Some(other) = other {
        h.ctx.memory_mut(|mem| mem.request_focus(other));
    }
    h.step();
    open(&mut h, MIDDLE, false);
    assert_eq!(focused(&h), Some(m.item_id(1)), "item 0 is disabled");
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert_eq!(focused(&h), other, "focus goes back to the widget that had it");

    let mut empty = harness(Vec::new());
    open(&mut empty, MIDDLE, false);
    assert_eq!(focused(&empty), Some(m.close_id()), "an empty ring focuses its close button");
}

#[test]
fn focus_moves_on_when_the_focused_item_is_disabled() {
    let m = menu();
    let mut h = harness(items(3));
    open(&mut h, MIDDLE, false);
    assert_eq!(focused(&h), Some(m.item_id(0)));
    h.state_mut().items[0].enabled = false;
    h.step();
    h.step();
    assert_eq!(focused(&h), Some(m.item_id(1)));
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert_eq!(h.state().events, vec![RadialEvent::Picked("paste".into())]);

    // Disabled in the same pass as an Enter: focus moves, and nothing is picked.
    let mut h = harness(items(3));
    open(&mut h, MIDDLE, false);
    h.state_mut().items[0].enabled = false;
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert!(h.state().events.is_empty());
    assert_eq!(focused(&h), Some(m.item_id(1)));

    // Disabled before the ring was first drawn, while focus waits to be placed on it.
    let mut h = harness(items(3));
    m.open(&h.ctx, MIDDLE, false);
    h.step();
    h.state_mut().items[0].enabled = false;
    h.step();
    assert_eq!(focused(&h), Some(m.item_id(1)), "on the first pass the ring is shown");
}

#[test]
fn screen_readers_see_a_named_menu_of_items() {
    let mut list = items(3);
    list[2].enabled = false;
    let mut h = harness(list);
    open(&mut h, MIDDLE, false);
    let copy = h.get_by_label("Copy").accesskit_node();
    assert_eq!(copy.role(), AkRole::MenuItem);
    assert!(h.get_by_label("Group").accesskit_node().is_disabled());
    assert_eq!(h.get_by_label("Commands").accesskit_node().role(), AkRole::Menu);
    assert_eq!(h.get_by_label("Close").accesskit_node().role(), AkRole::Button);
    let parent = h.get_by_label("Copy").parent().map(|p| p.accesskit_node().role());
    assert_eq!(parent, Some(AkRole::Menu), "items sit in the menu");
}
