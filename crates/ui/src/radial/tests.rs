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
    /// Key presses and typed text a panel drawn after the ring saw.
    later_keys: usize,
    /// The radius the app asks the ring for.
    radius: f32,
}

fn app(items: Vec<RadialItem>) -> App {
    App { items, events: Vec::new(), other: None, other_rect: Rect::NOTHING, other_clicks: 0, later_keys: 0, radius: RADIUS }
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
            if let Some(event) = menu().radius(app.radius).show(ui.ctx(), &app.items, &RadialLabels::default()) {
                app.events.push(event);
            }
            // A panel drawn after the ring that also closes on Esc.
            ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
            let typed = |e: &&Event| {
                matches!(
                    e,
                    Event::Key { pressed: true, .. }
                        | Event::Text(_)
                        | Event::Ime(_)
                        | Event::Paste(_)
                        | Event::PasteImage(_)
                        | Event::MiddleClickPaste { .. }
                )
            };
            app.later_keys += ui.input(|i| i.events.iter().filter(typed).count() + i.raw.events.iter().filter(typed).count());
            let other = ui.button("Other");
            app.other = Some(other.id);
            app.other_rect = other.rect;
            app.other_clicks += usize::from(other.clicked());
        },
        app(items),
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
        app(items(8)),
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
    list[2] = list[2].clone().disabled("Select two objects");
    let mut h = harness(list);
    open(&mut h, MIDDLE, false);
    let copy = h.get_by_label("Copy").accesskit_node();
    assert_eq!(copy.role(), AkRole::MenuItem);
    assert!(h.get_by_label("Group").accesskit_node().is_disabled());
    assert_eq!(h.get_by_label("Group").accesskit_node().description().as_deref(), Some("Select two objects"), "the reason it is disabled");
    assert_eq!(h.get_by_label("Commands").accesskit_node().role(), AkRole::Menu);
    assert_eq!(h.get_by_label("Close").accesskit_node().role(), AkRole::Button);
    let parent = h.get_by_label("Copy").parent().map(|p| p.accesskit_node().role());
    assert_eq!(parent, Some(AkRole::Menu), "items sit in the menu");
}

#[test]
fn space_picks_like_enter() {
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    key(&mut h, Key::ArrowRight, Modifiers::NONE);
    key(&mut h, Key::Space, Modifiers::NONE);
    assert_eq!(h.state().events, vec![RadialEvent::Picked("group".into())]);
}

#[test]
fn a_repeating_enter_does_not_pick() {
    let m = menu();
    let mut h = harness(items(8));
    // Enter goes down before the ring opens and repeats while it is open. egui marks a press of
    // a key that is already down as a repeat.
    let press = |pressed| Event::Key { key: Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    h.input_mut().events.push(press(true));
    h.step();
    open(&mut h, MIDDLE, false);
    for _ in 0..3 {
        h.input_mut().events.push(press(true));
        h.step();
    }
    assert!(h.state().events.is_empty(), "a key repeat is a key held from before: {:?}", h.state().events);
    assert!(m.is_open(&h.ctx));
    h.input_mut().events.push(press(false));
    h.step();
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert_eq!(h.state().events, vec![RadialEvent::Picked("copy".into())], "a fresh press picks");
}

#[test]
fn two_arrows_held_together_give_a_diagonal() {
    let m = menu();
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    h.input_mut().events.push(Event::Key { key: Key::ArrowUp, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.step();
    assert_eq!(focused(&h), Some(m.item_id(0)));
    key(&mut h, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(focused(&h), Some(m.item_id(1)), "Up held with Right is up and to the right");
}

#[test]
fn the_window_losing_focus_closes_the_ring() {
    let m = menu();
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    h.input_mut().focused = false;
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Closed]);
    assert!(!m.is_open(&h.ctx));

    // Lost and regained between two passes.
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    h.input_mut().events.push(Event::WindowFocused(false));
    h.input_mut().events.push(Event::WindowFocused(true));
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Closed]);
}

#[test]
fn a_click_that_closes_the_ring_gives_focus_back() {
    let m = menu();
    let mut h = harness(items(8));
    let other = h.state().other;
    if let Some(other) = other {
        h.ctx.memory_mut(|mem| mem.request_focus(other));
    }
    h.step();
    open(&mut h, MIDDLE, false);
    click(&mut h, slot(2, 8));
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Picked("group".into())]);
    assert!(!m.is_open(&h.ctx));
    assert_eq!(focused(&h), other, "focus goes back after a pointer pick");
}

#[test]
fn a_flick_or_click_at_a_command_the_app_just_replaced_picks_nothing() {
    let m = menu();
    let mut h = harness(items(8));
    move_to(&mut h, MIDDLE);
    open(&mut h, MIDDLE, true);
    move_to(&mut h, MIDDLE + vec2(0.0, RADIUS * 3.0));
    h.state_mut().items[4] = RadialItem::new("other", "Other");
    m.release(&h.ctx);
    h.step();
    assert!(h.state().events.is_empty(), "{:?}", h.state().events);

    // A click in the pass the app replaced the command.
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    move_to(&mut h, slot(4, 8));
    h.input_mut().events.push(Event::PointerButton { pos: slot(4, 8), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.state_mut().items[4] = RadialItem::new("other", "Other");
    h.input_mut().events.push(Event::PointerButton { pos: slot(4, 8), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.step();
    assert!(h.state().events.is_empty(), "{:?}", h.state().events);
}

#[test]
fn after_the_arrows_releasing_the_key_leaves_the_ring_open_for_enter() {
    let m = menu();
    // The pointer stays still, or flicks toward the item below after the arrows.
    for (flick, highlighted) in [(Vec2::ZERO, "group"), (vec2(0.0, RADIUS * 3.0), "lock")] {
        let mut h = harness(items(8));
        move_to(&mut h, MIDDLE);
        open(&mut h, MIDDLE, true);
        key(&mut h, Key::ArrowRight, Modifiers::NONE);
        if flick != Vec2::ZERO {
            move_to(&mut h, MIDDLE + flick);
        }
        m.release(&h.ctx);
        h.step();
        assert!(h.state().events.is_empty(), "flick {flick:?}: {:?}", h.state().events);
        assert!(m.is_open(&h.ctx));
        key(&mut h, Key::Enter, Modifiers::NONE);
        assert_eq!(h.state().events, vec![RadialEvent::Picked(highlighted.into())], "Enter picks the highlighted item");
    }
}

#[test]
fn an_arrow_that_opens_the_ring_is_not_picked_on_release() {
    let m = menu();
    let mut h = Harness::builder().with_size(vec2(800.0, 600.0)).build_ui_state(
        |ui, app: &mut App| {
            // Opened on any press, its repeats included, and not consumed.
            if ui.input(|i| i.key_pressed(Key::ArrowRight)) {
                menu().open_held_key(ui.ctx(), MIDDLE, Key::ArrowRight);
            }
            if let Some(event) = menu().show(ui.ctx(), &app.items, &RadialLabels::default()) {
                app.events.push(event);
            }
        },
        app(items(8)),
    );
    h.input_mut().events.push(Event::PointerMoved(MIDDLE));
    h.step();
    key(&mut h, Key::ArrowRight, Modifiers::NONE);
    h.step();
    assert!(h.state().events.is_empty(), "a tap of the opening key picks nothing: {:?}", h.state().events);
    assert!(m.is_open(&h.ctx));

    // Held long enough to repeat: egui marks a press of a key that is already down as a repeat.
    m.close(&h.ctx);
    h.step();
    let arrow = |pressed| Event::Key { key: Key::ArrowRight, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    for _ in 0..4 {
        h.input_mut().events.push(arrow(true));
        h.step();
    }
    h.input_mut().events.push(arrow(false));
    h.step();
    h.step();
    assert!(h.state().events.is_empty(), "repeats of the opening key pick nothing: {:?}", h.state().events);
    assert!(m.is_open(&h.ctx));
}

#[test]
fn an_arrow_in_the_pass_the_key_goes_up_only_moves_focus() {
    let m = menu();
    let mut h = harness(items(8));
    move_to(&mut h, MIDDLE);
    open(&mut h, MIDDLE, true);
    // A flick toward the item below, then the release and an arrow in one pass.
    move_to(&mut h, MIDDLE + vec2(0.0, RADIUS * 3.0));
    m.release(&h.ctx);
    h.input_mut().events.push(Event::Key { key: Key::ArrowRight, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.step();
    h.step();
    assert!(h.state().events.is_empty(), "{:?}", h.state().events);
    assert_eq!(focused(&h), Some(m.item_id(2)));
}

/// A window whose ring opens on `opener`, consumed as the README shows.
fn opened_by(opener: Key) -> Harness<'static, App> {
    Harness::builder().with_size(vec2(800.0, 600.0)).build_ui_state(
        move |ui, app: &mut App| {
            let fresh =
                |i: &egui::InputState| i.events.iter().any(|e| matches!(e, Event::Key { key, pressed: true, repeat: false, .. } if *key == opener));
            if !menu().is_open(ui.ctx()) && ui.input(fresh) {
                ui.input_mut(|i| i.consume_key(Modifiers::NONE, opener));
                menu().open_held_key(ui.ctx(), MIDDLE, opener);
            }
            if let Some(event) = menu().show(ui.ctx(), &app.items, &RadialLabels::default()) {
                app.events.push(event);
            }
        },
        app(items(8)),
    )
}

#[test]
fn a_flick_still_picks_when_an_arrow_held_down_opened_the_ring() {
    let m = menu();
    let mut h = opened_by(Key::ArrowRight);
    h.input_mut().events.push(Event::PointerMoved(MIDDLE));
    h.step();
    // The first press opens the ring; egui marks the presses after it as repeats.
    let arrow = |pressed| Event::Key { key: Key::ArrowRight, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    for _ in 0..4 {
        h.input_mut().events.push(arrow(true));
        h.step();
    }
    move_to(&mut h, MIDDLE + vec2(0.0, RADIUS * 3.0));
    // The last repeat and the key going up in one pass; the ring reads the release itself.
    h.input_mut().events.push(arrow(true));
    h.input_mut().events.push(arrow(false));
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Picked("lock".into())]);
    assert!(!m.is_open(&h.ctx));
}

#[test]
fn an_arrow_held_as_the_opening_key_is_not_part_of_a_diagonal() {
    let m = menu();
    let mut h = opened_by(Key::ArrowRight);
    h.input_mut().events.push(Event::Key { key: Key::ArrowRight, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.step();
    h.step();
    key(&mut h, Key::ArrowUp, Modifiers::NONE);
    assert_eq!(focused(&h), Some(m.item_id(0)), "Up alone");
}

#[test]
fn repeats_of_an_escape_that_opened_the_ring_do_not_close_it() {
    let m = menu();
    let mut h = opened_by(Key::Escape);
    for _ in 0..3 {
        h.input_mut().events.push(Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        h.step();
    }
    assert!(m.is_open(&h.ctx));
    assert!(h.state().events.is_empty());
}

#[test]
fn a_release_from_before_the_opening_press_does_not_end_the_new_hold() {
    let m = menu();
    let mut h = opened_by(Key::Space);
    let space = |pressed| Event::Key { key: Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    h.input_mut().events.push(Event::PointerMoved(MIDDLE));
    h.input_mut().events.push(space(true));
    h.step();
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert!(!m.is_open(&h.ctx));
    // The old hold ends and a new one starts in one pass.
    h.input_mut().events.push(space(false));
    h.input_mut().events.push(space(true));
    h.step();
    assert!(m.is_open(&h.ctx));
    move_to(&mut h, MIDDLE + vec2(0.0, RADIUS * 3.0));
    h.input_mut().events.push(space(false));
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Closed, RadialEvent::Picked("lock".into())]);
}

#[test]
fn input_after_the_opening_key_goes_up_in_the_same_pass_does_not_count_as_held() {
    let m = menu();
    let space = |pressed| Event::Key { key: Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    let enter = Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };

    // A tap, then a pointer move: the tap leaves the ring open.
    let mut h = opened_by(Key::Space);
    h.input_mut().events.push(Event::PointerMoved(MIDDLE));
    h.input_mut().events.push(space(true));
    h.step();
    h.step();
    h.input_mut().events.push(space(false));
    h.input_mut().events.push(Event::PointerMoved(MIDDLE + vec2(0.0, RADIUS * 3.0)));
    h.step();
    assert!(h.state().events.is_empty(), "{:?}", h.state().events);
    assert!(m.is_open(&h.ctx));

    // Enter while the key is still held, then the release.
    let mut h = opened_by(Key::Space);
    h.input_mut().events.push(Event::PointerMoved(MIDDLE));
    h.input_mut().events.push(space(true));
    h.step();
    h.step();
    h.input_mut().events.push(enter);
    h.input_mut().events.push(space(false));
    h.step();
    assert!(h.state().events.is_empty(), "{:?}", h.state().events);
    assert!(m.is_open(&h.ctx));

    // A flick, then the pointer leaves the window before the release.
    let mut h = opened_by(Key::Space);
    h.input_mut().events.push(Event::PointerMoved(MIDDLE));
    h.input_mut().events.push(space(true));
    h.step();
    h.step();
    h.input_mut().events.push(Event::PointerMoved(MIDDLE + vec2(0.0, RADIUS * 3.0)));
    h.input_mut().events.push(Event::PointerGone);
    h.input_mut().events.push(space(false));
    h.step();
    assert!(h.state().events.is_empty(), "no pointer at the release: {:?}", h.state().events);
}

#[test]
fn a_held_key_gesture_reads_the_pointer_at_its_press_and_release() {
    let m = menu();
    let space = |pressed| Event::Key { key: Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };

    // A flick, the release, then Enter, in one pass: the flick picks.
    let mut h = opened_by(Key::Space);
    h.input_mut().events.push(Event::PointerMoved(MIDDLE));
    h.input_mut().events.push(space(true));
    h.step();
    h.step();
    h.input_mut().events.push(Event::PointerMoved(MIDDLE + vec2(0.0, RADIUS * 3.0)));
    h.input_mut().events.push(space(false));
    h.input_mut().events.push(Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Picked("lock".into())]);

    // A flick, then Esc before the release: Esc closes.
    let mut h = opened_by(Key::Space);
    h.input_mut().events.push(Event::PointerMoved(MIDDLE));
    h.input_mut().events.push(space(true));
    h.step();
    h.step();
    h.input_mut().events.push(Event::PointerMoved(MIDDLE + vec2(0.0, RADIUS * 3.0)));
    h.input_mut().events.push(Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.input_mut().events.push(space(false));
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Closed]);

    // A tap with the pointer still, then a move, all in the pass that opens the ring.
    let mut h = opened_by(Key::Space);
    h.input_mut().events.push(Event::PointerMoved(MIDDLE));
    h.step();
    h.input_mut().events.push(space(true));
    h.input_mut().events.push(space(false));
    h.input_mut().events.push(Event::PointerMoved(MIDDLE + vec2(0.0, RADIUS * 3.0)));
    h.step();
    assert!(h.state().events.is_empty(), "{:?}", h.state().events);
    assert!(m.is_open(&h.ctx));
}

#[test]
fn a_new_press_after_the_hold_ends_in_the_same_pass_activates() {
    let m = menu();
    let space = |pressed| Event::Key { key: Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    let mut h = opened_by(Key::Space);
    h.input_mut().events.push(Event::PointerMoved(MIDDLE));
    h.input_mut().events.push(space(true));
    h.step();
    h.step();
    assert!(m.is_open(&h.ctx));
    for pressed in [false, true, false] {
        h.input_mut().events.push(space(pressed));
    }
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Picked("copy".into())], "the second Space picks the focused item");
}

#[test]
fn a_held_opening_key_does_not_reopen_the_ring_after_escape() {
    let m = menu();
    let mut h = opened_by(Key::Space);
    let space = Event::Key { key: Key::Space, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    h.input_mut().events.push(space.clone());
    h.step();
    assert!(m.is_open(&h.ctx));
    key(&mut h, Key::Escape, Modifiers::NONE);
    assert!(!m.is_open(&h.ctx));
    for _ in 0..3 {
        h.input_mut().events.push(space.clone());
        h.step();
    }
    assert!(!m.is_open(&h.ctx), "the held key's repeats do not open it again");
}

#[test]
fn another_key_held_with_the_opening_key_still_repeats() {
    let m = menu();
    let mut h = opened_by(Key::Space);
    let press = |key| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    h.input_mut().events.push(press(Key::Space));
    h.step();
    h.step();
    for _ in 0..2 {
        h.input_mut().events.push(press(Key::Tab));
        h.step();
    }
    h.step();
    assert_eq!(focused(&h), Some(m.item_id(2)), "Tab and its repeat move twice");
}

#[test]
fn another_arrow_in_the_opening_pass_leaves_the_release_to_enter() {
    let m = menu();
    let mut h = opened_by(Key::Space);
    h.input_mut().events.push(Event::PointerMoved(MIDDLE));
    h.step();
    for key in [Key::Space, Key::ArrowRight] {
        h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    }
    h.step();
    move_to(&mut h, MIDDLE + vec2(0.0, RADIUS * 3.0));
    m.release(&h.ctx);
    h.step();
    assert!(h.state().events.is_empty(), "{:?}", h.state().events);
    assert!(m.is_open(&h.ctx));
}

#[test]
fn an_arrow_keeps_the_highlight_over_a_pointer_move_in_the_same_pass() {
    let m = menu();
    let mut h = harness(items(8));
    move_to(&mut h, MIDDLE);
    open(&mut h, MIDDLE, false);
    h.input_mut().events.push(Event::PointerMoved(slot(4, 8)));
    h.input_mut().events.push(Event::Key { key: Key::ArrowRight, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.step();
    h.step();
    assert_eq!(focused(&h), Some(m.item_id(2)));
}

#[test]
fn an_assistive_click_on_a_control_under_the_ring_does_nothing() {
    let m = menu();
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    let click = |id: Id| {
        Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Click,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node: id.accesskit_id(),
            data: None,
        })
    };
    let other = h.state().other.unwrap_or(Id::NULL);
    h.input_mut().events.push(click(other));
    h.step();
    assert_eq!(h.state().other_clicks, 0);
    assert!(m.is_open(&h.ctx));
    // An assistive focus request for the control under the ring: focus goes back to the ring's
    // item.
    key(&mut h, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(focused(&h), Some(m.item_id(2)));
    h.input_mut().events.push(Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
        action: egui::accesskit::Action::Focus,
        target_tree: egui::accesskit::TreeId::ROOT,
        target_node: other.accesskit_id(),
        data: None,
    }));
    h.step();
    h.step();
    assert_eq!(focused(&h), Some(m.item_id(2)));
    // A click for an item whose command the app just replaced does nothing.
    h.state_mut().items.remove(0);
    h.input_mut().events.push(click(m.item_id(0)));
    h.step();
    assert!(h.state().events.is_empty(), "{:?}", h.state().events);
    h.input_mut().events.push(click(m.item_id(2)));
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Picked("ungroup".into())], "the ring's own items still act");
}

#[test]
fn an_arrow_and_enter_in_one_pass_pick_where_the_arrow_moved() {
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    for key in [Key::ArrowRight, Key::Enter] {
        h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    }
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Picked("group".into())]);
}

#[test]
fn enter_acts_on_the_highlight_as_it_was_at_its_press() {
    let press = |key| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    // Tab or an arrow after Enter in the same pass does not move what Enter picks.
    for after in [Key::ArrowUp, Key::Tab] {
        let mut h = harness(items(8));
        open(&mut h, MIDDLE, false);
        for key in [Key::ArrowRight, Key::Enter, after] {
            h.input_mut().events.push(press(key));
        }
        h.step();
        assert_eq!(h.state().events, vec![RadialEvent::Picked("group".into())], "{after:?} after Enter");
    }

    // Up held, then Right and Enter, then Up goes up, in one pass: Up was still held at Enter.
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    h.input_mut().events.push(press(Key::ArrowUp));
    h.step();
    h.input_mut().events.push(press(Key::ArrowRight));
    h.input_mut().events.push(press(Key::Enter));
    h.input_mut().events.push(Event::Key { key: Key::ArrowUp, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Picked("paste".into())]);
}

#[test]
fn the_first_key_in_a_pass_that_ends_the_ring_acts() {
    let press = |key| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    let up = |key| Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE };
    let run = |events: Vec<Event>| {
        let mut h = harness(items(8));
        open(&mut h, MIDDLE, false);
        h.step();
        h.input_mut().events.extend(events);
        h.step();
        h.state().events.clone()
    };
    // With Copy focused, Enter picks Copy before Right moves the highlight and Space acts.
    assert_eq!(run(vec![press(Key::Enter), press(Key::ArrowRight), press(Key::Space)]), vec![RadialEvent::Picked("copy".into())]);
    // The first Enter picks Group, before Up and the second Enter.
    assert_eq!(
        run(vec![press(Key::ArrowRight), press(Key::Enter), up(Key::Enter), press(Key::ArrowUp), press(Key::Enter)]),
        vec![RadialEvent::Picked("group".into())]
    );
    // Enter before Esc picks; Esc before Enter closes.
    assert_eq!(run(vec![press(Key::Enter), press(Key::Escape)]), vec![RadialEvent::Picked("copy".into())]);
    assert_eq!(run(vec![press(Key::Escape), press(Key::Enter)]), vec![RadialEvent::Closed]);
}

#[test]
fn an_enter_that_cannot_act_leaves_the_keys_after_it_to_act() {
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    // The app replaces the commands: Enter on the focused item does nothing, and Shift+Tab and
    // Enter after it reach the close button.
    h.state_mut().items.remove(0);
    for (key, pressed, modifiers) in [
        (Key::Enter, true, Modifiers::NONE),
        (Key::Enter, false, Modifiers::NONE),
        (Key::Tab, true, Modifiers::SHIFT),
        (Key::Enter, true, Modifiers::NONE),
    ] {
        h.input_mut().events.push(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
    }
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Closed]);
}

#[test]
fn a_repeat_of_a_key_the_ring_did_not_take_does_not_move_the_highlight() {
    let m = menu();
    // egui marks a press of a key it already has down as a repeat.
    let repeat = |key, modifiers| Event::Key { key, physical_key: None, pressed: true, repeat: true, modifiers };
    let down_before_open = |h: &mut Harness<'_, App>, key, modifiers| {
        h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
        h.step();
        open(h, MIDDLE, false);
    };
    // Right, Tab and Shift+Tab repeating from before the ring opened.
    for (key, modifiers) in [(Key::ArrowRight, Modifiers::NONE), (Key::Tab, Modifiers::NONE), (Key::Tab, Modifiers::SHIFT)] {
        let mut h = harness(items(8));
        down_before_open(&mut h, key, modifiers);
        h.input_mut().events.push(repeat(key, modifiers));
        h.step();
        h.step();
        assert_eq!(focused(&h), Some(m.item_id(0)), "{key:?} with {modifiers:?}");
    }
    // A repeat of Right from before the ring opened is not part of a diagonal.
    let mut h = harness(items(8));
    down_before_open(&mut h, Key::ArrowRight, Modifiers::NONE);
    h.input_mut().events.push(repeat(Key::ArrowRight, Modifiers::NONE));
    h.step();
    key(&mut h, Key::ArrowUp, Modifiers::NONE);
    assert_eq!(focused(&h), Some(m.item_id(0)), "Up alone");
    // Right pressed with Alt, then repeating once Alt goes up, is not part of one either.
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    h.input_mut().events.push(Event::Key { key: Key::ArrowRight, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::ALT });
    h.step();
    h.input_mut().events.push(repeat(Key::ArrowRight, Modifiers::NONE));
    h.step();
    key(&mut h, Key::ArrowUp, Modifiers::NONE);
    assert_eq!(focused(&h), Some(m.item_id(0)), "Up alone after Alt went up");
    // Repeats of a press the ring took still move the highlight.
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    h.input_mut().events.push(Event::Key { key: Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.input_mut().events.push(repeat(Key::Tab, Modifiers::NONE));
    h.step();
    h.input_mut().events.push(repeat(Key::Tab, Modifiers::NONE));
    h.step();
    h.step();
    assert_eq!(focused(&h), Some(m.item_id(3)), "a press and two repeats");
}

#[test]
fn a_repeat_of_an_arrow_from_before_the_ring_opened_does_not_stop_the_flick() {
    let space = |pressed| Event::Key { key: Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    let mut h = opened_by(Key::Space);
    h.input_mut().events.push(Event::PointerMoved(MIDDLE));
    h.input_mut().events.push(Event::Key { key: Key::ArrowRight, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.step();
    h.input_mut().events.push(space(true));
    h.step();
    move_to(&mut h, MIDDLE + vec2(0.0, RADIUS * 3.0));
    h.input_mut().events.push(Event::Key { key: Key::ArrowRight, physical_key: None, pressed: true, repeat: true, modifiers: Modifiers::NONE });
    h.input_mut().events.push(space(false));
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Picked("lock".into())]);
}

#[test]
fn an_arrow_pressed_before_the_opening_key_in_its_pass_does_not_stop_the_flick() {
    let m = menu();
    let press = |key, pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    let mut h = opened_by(Key::Space);
    h.input_mut().events.push(Event::PointerMoved(MIDDLE));
    h.step();
    // Right goes down, then Space opens the ring, the pointer flicks down and Space goes up, all
    // in one pass.
    let below = MIDDLE + vec2(0.0, RADIUS * 3.0);
    for e in [press(Key::ArrowRight, true), press(Key::Space, true), Event::PointerMoved(below), press(Key::Space, false)] {
        h.input_mut().events.push(e);
    }
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Picked("lock".into())]);
    assert!(!m.is_open(&h.ctx));
}

#[test]
fn only_a_key_the_ring_acts_on_before_the_release_stops_the_flick() {
    let space = |pressed| Event::Key { key: Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    let press = |key, modifiers| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    let flick_with = |before: Event, after: Event| {
        let mut h = opened_by(Key::Space);
        h.input_mut().events.push(Event::PointerMoved(MIDDLE));
        h.input_mut().events.push(space(true));
        h.step();
        h.step();
        move_to(&mut h, MIDDLE + vec2(0.0, RADIUS * 3.0));
        for e in [before, space(false), after] {
            h.input_mut().events.push(e);
        }
        h.step();
        h.state().events.clone()
    };
    // The ring does not take Ctrl+Right, so the release picks before the Esc after it.
    let events = flick_with(press(Key::ArrowRight, Modifiers::CTRL), press(Key::Escape, Modifiers::NONE));
    assert_eq!(events, vec![RadialEvent::Picked("lock".into())]);
    // Esc with Alt closes the ring before the release.
    let events = flick_with(press(Key::Escape, Modifiers::ALT), press(Key::Enter, Modifiers::NONE));
    assert_eq!(events, vec![RadialEvent::Closed]);
}

#[test]
fn each_tab_in_one_pass_moves_the_highlight() {
    let m = menu();
    for (modifiers, to) in [(Modifiers::NONE, 2), (Modifiers::SHIFT, 7)] {
        let mut h = harness(items(8));
        open(&mut h, MIDDLE, false);
        for _ in 0..2 {
            h.input_mut().events.push(Event::Key { key: Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers });
        }
        h.step();
        h.step();
        assert_eq!(focused(&h), Some(m.item_id(to)), "two presses with {modifiers:?}");
    }
}

#[test]
fn tab_and_an_arrow_in_one_pass_move_in_order() {
    let m = menu();
    for (keys, to) in [([Key::ArrowRight, Key::Tab], 3), ([Key::Tab, Key::ArrowRight], 2)] {
        let mut h = harness(items(8));
        open(&mut h, MIDDLE, false);
        for key in keys {
            h.input_mut().events.push(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        }
        h.step();
        h.step();
        assert_eq!(focused(&h), Some(m.item_id(to)), "{keys:?}");
    }
}

#[test]
fn an_arrow_held_with_alt_is_not_part_of_a_diagonal() {
    let m = menu();
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    h.input_mut().events.push(Event::Key { key: Key::ArrowRight, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::ALT });
    h.step();
    key(&mut h, Key::ArrowUp, Modifiers::NONE);
    assert_eq!(focused(&h), Some(m.item_id(0)), "Up alone");
}

#[test]
fn an_opening_arrow_pressed_again_after_its_release_is_held() {
    let m = menu();
    let arrow = |key, pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    let mut h = opened_by(Key::ArrowRight);
    h.input_mut().events.push(arrow(Key::ArrowRight, true));
    h.step();
    h.step();
    assert!(m.is_open(&h.ctx));
    // The opening press ends and a new one starts in one pass.
    h.input_mut().events.push(arrow(Key::ArrowRight, false));
    h.input_mut().events.push(arrow(Key::ArrowRight, true));
    h.step();
    assert_eq!(focused(&h), Some(m.item_id(2)));
    key(&mut h, Key::ArrowUp, Modifiers::NONE);
    assert_eq!(focused(&h), Some(m.item_id(1)), "Up with the new Right held is up and to the right");
}

#[test]
fn a_click_then_a_drag_in_one_pass_picks_where_the_click_ended() {
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    move_to(&mut h, slot(2, 8));
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    h.input_mut().events.push(button(slot(2, 8), true));
    h.input_mut().events.push(button(slot(2, 8), false));
    h.input_mut().events.push(button(slot(2, 8), true));
    h.input_mut().events.push(Event::PointerMoved(slot(4, 8)));
    h.input_mut().events.push(button(slot(4, 8), false));
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Picked("group".into())], "the drag that ended over Lock is not a click");
}

#[test]
fn a_flick_that_leaves_the_window_aims_from_where_the_pass_began() {
    let space = |pressed| Event::Key { key: Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    // Far enough right of the ring's center that a flick measured from the center would aim at
    // the item before the one below.
    let start = MIDDLE + vec2(RADIUS * 1.5, 0.0);
    let mut h = opened_by(Key::Space);
    h.input_mut().events.push(Event::PointerMoved(start));
    h.step();
    // The press, a flick straight down, the release and the pointer leaving, all in one pass.
    h.input_mut().events.push(space(true));
    h.input_mut().events.push(Event::PointerMoved(start + vec2(0.0, RADIUS * 3.0)));
    h.input_mut().events.push(space(false));
    h.input_mut().events.push(Event::PointerGone);
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Picked("lock".into())]);
}

#[test]
fn a_button_held_open_aims_from_where_it_went_down() {
    let m = menu();
    let below = MIDDLE + vec2(0.0, RADIUS * 3.0);
    let press = |pos, button| Event::PointerButton { pos, button, pressed: true, modifiers: Modifiers::NONE };
    // The button that opens the ring goes down at the ring's center and the pointer moves on in
    // the same pass. Another button that goes down elsewhere in that pass, before or after it,
    // does not move where the gesture starts.
    let opening = press(MIDDLE, PointerButton::Secondary);
    let other = press(MIDDLE + vec2(RADIUS * 3.0, 0.0), PointerButton::Middle);
    for (first, second) in [(opening.clone(), other.clone()), (other, opening)] {
        let mut h = harness(items(8));
        move_to(&mut h, MIDDLE);
        for e in [first, Event::PointerMoved(below), second, Event::PointerMoved(below)] {
            h.input_mut().events.push(e);
        }
        h.step();
        open(&mut h, MIDDLE, true);
        m.release(&h.ctx);
        h.step();
        assert_eq!(h.state().events, vec![RadialEvent::Picked("lock".into())]);
    }
}

#[test]
fn a_button_held_open_picks_from_where_it_went_up() {
    let m = menu();
    let mut h = harness(items(8));
    move_to(&mut h, MIDDLE);
    h.input_mut().events.push(Event::PointerButton { pos: MIDDLE, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    open(&mut h, MIDDLE, true);
    // The button goes up to the right, and the pointer moves down after it in the same pass.
    let right = MIDDLE + vec2(RADIUS * 3.0, 0.0);
    for e in [
        Event::PointerMoved(right),
        Event::PointerButton { pos: right, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE },
        Event::PointerMoved(MIDDLE + vec2(0.0, RADIUS * 3.0)),
    ] {
        h.input_mut().events.push(e);
    }
    m.release(&h.ctx);
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Picked("group".into())]);
}

#[test]
fn a_press_on_a_disabled_item_that_is_enabled_before_the_release_picks_nothing() {
    let mut list = items(8);
    list[2] = RadialItem::new("group", "Group").disabled("Select two objects");
    let mut h = harness(list);
    open(&mut h, MIDDLE, false);
    move_to(&mut h, slot(2, 8));
    h.input_mut().events.push(Event::PointerButton { pos: slot(2, 8), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.state_mut().items[2].enabled = true;
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: slot(2, 8), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.step();
    assert!(h.state().events.is_empty(), "{:?}", h.state().events);

    // Another item enabled during a press does not stop the click it was made for.
    let mut list = items(8);
    list[2] = RadialItem::new("group", "Group").disabled("Select two objects");
    let mut h = harness(list);
    open(&mut h, MIDDLE, false);
    move_to(&mut h, slot(4, 8));
    h.input_mut().events.push(Event::PointerButton { pos: slot(4, 8), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.state_mut().items[2].enabled = true;
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: slot(4, 8), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Picked("lock".into())]);
}

#[test]
fn a_press_on_a_disabled_item_released_over_its_neighbor_picks_nothing() {
    // Just inside Group's sector, and one point lower, just inside Ungroup's: closer together
    // than egui's click distance.
    let in_group = MIDDLE + vec2(66.0, 27.0);
    let in_ungroup = MIDDLE + vec2(66.0, 28.0);
    assert_eq!(hit(MIDDLE, in_group, 8, CENTER / 2.0, menu().outer_radius()), RingHit::Slot(2));
    assert_eq!(hit(MIDDLE, in_ungroup, 8, CENTER / 2.0, menu().outer_radius()), RingHit::Slot(3));
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    let disabled_group = || {
        let mut list = items(8);
        list[2] = RadialItem::new("group", "Group").disabled("Select two objects");
        let mut h = harness(list);
        open(&mut h, MIDDLE, false);
        move_to(&mut h, in_group);
        h
    };

    // The press and the release in one pass.
    let mut h = disabled_group();
    h.input_mut().events.push(button(in_group, true));
    h.input_mut().events.push(Event::PointerMoved(in_ungroup));
    h.input_mut().events.push(button(in_ungroup, false));
    h.step();
    assert!(h.state().events.is_empty(), "{:?}", h.state().events);
    assert!(menu().is_open(&h.ctx));

    // The release a pass after the press.
    let mut h = disabled_group();
    h.input_mut().events.push(button(in_group, true));
    h.step();
    h.input_mut().events.push(Event::PointerMoved(in_ungroup));
    h.input_mut().events.push(button(in_ungroup, false));
    h.step();
    assert!(h.state().events.is_empty(), "{:?}", h.state().events);

    // A click that goes down and up in Ungroup still picks it.
    let mut h = disabled_group();
    move_to(&mut h, in_ungroup);
    h.input_mut().events.push(button(in_ungroup, true));
    h.step();
    h.input_mut().events.push(button(in_ungroup, false));
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Picked("ungroup".into())]);
}

#[test]
fn a_press_on_a_ring_that_then_mirrors_picks_nothing() {
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    move_to(&mut h, slot(2, 8));
    h.input_mut().events.push(Event::PointerButton { pos: slot(2, 8), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    crate::direction::apply(&h.ctx, crate::direction::Direction::Rtl);
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: slot(2, 8), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.step();
    assert!(h.state().events.is_empty(), "pressed over Group, released over Undo: {:?}", h.state().events);

    // The ring mirrors and the button goes down and up over the same place in one pass.
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    move_to(&mut h, slot(2, 8));
    crate::direction::apply(&h.ctx, crate::direction::Direction::Rtl);
    for pressed in [true, false] {
        h.input_mut().events.push(Event::PointerButton { pos: slot(2, 8), button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
    h.step();
    assert!(h.state().events.is_empty(), "pressed over Group as drawn, released over Undo: {:?}", h.state().events);
}

#[test]
fn a_held_press_on_a_ring_that_then_mirrors_leaves_the_keys_working() {
    let m = menu();
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    move_to(&mut h, slot(2, 8));
    h.input_mut().events.push(Event::PointerButton { pos: slot(2, 8), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    crate::direction::apply(&h.ctx, crate::direction::Direction::Rtl);
    h.step();
    // Right in a right-to-left ring goes to Undo, and Enter picks it while the button is down.
    key(&mut h, Key::ArrowRight, Modifiers::NONE);
    assert_eq!(focused(&h), Some(m.item_id(6)));
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert_eq!(h.state().events, vec![RadialEvent::Picked("undo".into())]);

    // An assistive click on an item is not a pointer click either.
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    move_to(&mut h, slot(2, 8));
    h.input_mut().events.push(Event::PointerButton { pos: slot(2, 8), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    crate::direction::apply(&h.ctx, crate::direction::Direction::Rtl);
    h.step();
    h.input_mut().events.push(Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
        action: egui::accesskit::Action::Click,
        target_tree: egui::accesskit::TreeId::ROOT,
        target_node: m.item_id(4).accesskit_id(),
        data: None,
    }));
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Picked("lock".into())]);
}

#[test]
fn a_press_held_while_only_the_asked_radius_changes_still_clicks() {
    let ring = menu().radius(1.0);
    // Items 40 pt across raise a radius of 1 or 2 pt to the same one, so the ring does not move.
    assert_eq!(ring.ring_radius(), menu().radius(2.0).ring_radius());
    let at = MIDDLE + slot_direction(2, 8) * ring.ring_radius();
    let mut h = harness(items(8));
    h.state_mut().radius = 1.0;
    open(&mut h, MIDDLE, false);
    move_to(&mut h, at);
    h.input_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.state_mut().radius = 2.0;
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Picked("group".into())]);
}

#[test]
fn a_click_in_one_pass_on_an_item_enabled_in_that_pass_picks_nothing() {
    let mut list = items(8);
    list[2] = RadialItem::new("group", "Group").disabled("Select two objects");
    let mut h = harness(list);
    open(&mut h, MIDDLE, false);
    move_to(&mut h, slot(2, 8));
    h.state_mut().items[2].enabled = true;
    for pressed in [true, false] {
        h.input_mut().events.push(Event::PointerButton { pos: slot(2, 8), button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
    h.step();
    assert!(h.state().events.is_empty(), "Group was disabled when it was pressed: {:?}", h.state().events);

    // An item that was already enabled takes a click made in one pass.
    for pressed in [true, false] {
        h.input_mut().events.push(Event::PointerButton { pos: slot(4, 8), button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Picked("lock".into())]);

    // A press on Group while it was disabled ends, and a new click on it, now enabled, follows in
    // one pass: the new press decides.
    let mut list = items(8);
    list[2] = RadialItem::new("group", "Group").disabled("Select two objects");
    let mut h = harness(list);
    open(&mut h, MIDDLE, false);
    move_to(&mut h, slot(2, 8));
    h.input_mut().events.push(Event::PointerButton { pos: slot(2, 8), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.state_mut().items[2].enabled = true;
    h.step();
    for pressed in [false, true, false] {
        h.input_mut().events.push(Event::PointerButton { pos: slot(2, 8), button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Picked("group".into())]);
}

#[test]
fn a_click_and_a_drag_while_another_button_is_held_pick_where_the_click_ended() {
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    move_to(&mut h, slot(2, 8));
    let button = |pos, button, pressed| Event::PointerButton { pos, button, pressed, modifiers: Modifiers::NONE };
    h.input_mut().events.push(button(slot(2, 8), PointerButton::Secondary, true));
    h.step();
    // A click on Group, then a drag to Lock; the other button goes up before the drag ends.
    for e in [
        button(slot(2, 8), PointerButton::Primary, true),
        button(slot(2, 8), PointerButton::Primary, false),
        button(slot(2, 8), PointerButton::Primary, true),
        Event::PointerMoved(slot(4, 8)),
        button(slot(4, 8), PointerButton::Secondary, false),
        button(slot(4, 8), PointerButton::Primary, false),
    ] {
        h.input_mut().events.push(e);
    }
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Picked("group".into())], "the drag that ended over Lock is not a click");
}

#[test]
fn a_ring_too_large_for_the_window_shrinks_to_fit() {
    let big = || menu().button_size(200.0);
    let wide = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    assert_eq!(menu().fitted(wide).button, BUTTON, "a ring that fits keeps its size");
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(320.0, 240.0));
    let ring = big().fitted(screen);
    assert!(ring.outer_radius() <= 120.0 + 1e-3, "{ring:?} reaches {}", ring.outer_radius());
    // A long radius with the smallest items still fits a window that has room for it.
    let square = Rect::from_min_size(Pos2::ZERO, vec2(200.0, 200.0));
    let long = menu().radius(1024.0).button_size(16.0).fitted(square);
    assert!(long.outer_radius() <= 100.0 + 1e-3, "{long:?} reaches {}", long.outer_radius());
    let center = fit_center(screen.center(), ring.reach(), screen);
    for i in 0..MAX_ITEMS {
        assert!(screen.contains_rect(ring.item_rect(center, i, MAX_ITEMS, false)), "{ring:?}: item {i} off screen");
        assert_eq!(ring.ring_hit(center, ring.item_center(center, i, MAX_ITEMS, false), MAX_ITEMS, false), RingHit::Slot(i));
    }

    // A click on the item drawn at the right picks it.
    let mut h = Harness::builder().with_size(vec2(320.0, 240.0)).build_ui_state(
        move |ui, app: &mut App| {
            if let Some(event) = big().show(ui.ctx(), &app.items, &RadialLabels::default()) {
                app.events.push(event);
            }
        },
        app(items(8)),
    );
    h.step();
    big().open(&h.ctx, screen.center(), false);
    h.step();
    h.step();
    assert_eq!(big().hit_test(&h.ctx, ring.item_center(center, 2, MAX_ITEMS, false), MAX_ITEMS), RingHit::Slot(2));
    // The ring draws the item where the click lands.
    let drawn = h.ctx.read_response(big().item_id(2)).map(|r| r.rect);
    assert!(drawn.is_some_and(|r| screen.contains_rect(r)), "Group drawn at {drawn:?}");
    assert!(drawn.is_some_and(|r| r.center().distance(ring.item_center(center, 2, MAX_ITEMS, false)) < 1e-3), "Group drawn at {drawn:?}");
    click(&mut h, ring.item_center(center, 2, MAX_ITEMS, false));
    assert_eq!(h.state().events, vec![RadialEvent::Picked("group".into())]);
}

#[test]
fn a_click_after_the_app_changes_the_items_under_it_picks_nothing() {
    let mut h = harness(items(4));
    open(&mut h, MIDDLE, false);
    move_to(&mut h, slot(1, 4));
    h.input_mut().events.push(Event::PointerButton { pos: slot(1, 4), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.state_mut().items = items(8);
    h.input_mut().events.push(Event::PointerButton { pos: slot(1, 4), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.step();
    assert!(h.state().events.is_empty(), "the sectors moved: {:?}", h.state().events);

    // The command changes while the button is down, and the release comes a pass later.
    let mut h = harness(items(4));
    open(&mut h, MIDDLE, false);
    move_to(&mut h, slot(1, 4));
    h.input_mut().events.push(Event::PointerButton { pos: slot(1, 4), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.state_mut().items[1] = RadialItem::new("delete", "Delete");
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: slot(1, 4), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.step();
    assert!(h.state().events.is_empty(), "pressed over another command: {:?}", h.state().events);

    // The command changes in the pass the button goes down.
    let mut h = harness(items(4));
    open(&mut h, MIDDLE, false);
    move_to(&mut h, slot(1, 4));
    h.state_mut().items[1] = RadialItem::new("delete", "Delete");
    h.input_mut().events.push(Event::PointerButton { pos: slot(1, 4), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: slot(1, 4), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.step();
    assert!(h.state().events.is_empty(), "pressed over the command drawn before: {:?}", h.state().events);
}

#[test]
fn a_click_picks_where_the_button_went_up() {
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    move_to(&mut h, slot(2, 8));
    h.input_mut().events.push(Event::PointerButton { pos: slot(2, 8), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.input_mut().events.push(Event::PointerButton { pos: slot(2, 8), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.input_mut().events.push(Event::PointerMoved(slot(4, 8)));
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Picked("group".into())]);
}

#[test]
fn enter_does_not_activate_a_command_that_moved_into_the_focused_slot() {
    let m = menu();
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    assert_eq!(focused(&h), Some(m.item_id(0)));
    h.state_mut().items.remove(0);
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert!(h.state().events.is_empty(), "{:?}", h.state().events);
    key(&mut h, Key::Enter, Modifiers::NONE);
    assert_eq!(h.state().events, vec![RadialEvent::Picked("paste".into())], "a second Enter picks what is there now");
}

#[test]
fn alt_with_an_arrow_does_not_move_the_highlight() {
    let m = menu();
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    key(&mut h, Key::ArrowRight, Modifiers::ALT);
    assert_eq!(focused(&h), Some(m.item_id(0)));
    assert_eq!(h.state().later_keys, 0);
}

#[test]
fn keys_in_the_pass_the_window_loses_focus_do_not_reach_the_panels() {
    let m = menu();
    let mut h = harness(items(8));
    open(&mut h, MIDDLE, false);
    h.state_mut().later_keys = 0;
    h.input_mut().events.push(Event::Key { key: Key::Delete, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.input_mut().focused = false;
    h.step();
    assert_eq!(h.state().events, vec![RadialEvent::Closed]);
    assert!(!m.is_open(&h.ctx));
    assert_eq!(h.state().later_keys, 0);
}

#[test]
fn an_open_ring_keeps_other_keys_from_the_panels_under_it() {
    let mut h = harness(items(8));
    key(&mut h, Key::Z, Modifiers::COMMAND);
    assert!(h.state().later_keys > 0, "with the ring closed, panels see key presses");
    open(&mut h, MIDDLE, false);
    h.state_mut().later_keys = 0;
    key(&mut h, Key::Z, Modifiers::COMMAND);
    key(&mut h, Key::Delete, Modifiers::NONE);
    key(&mut h, Key::ArrowRight, Modifiers::SHIFT);
    h.input_mut().events.push(Event::Text("v".into()));
    h.input_mut().events.push(Event::Ime(egui::ImeEvent::Commit("v".into())));
    h.input_mut().events.push(Event::Paste("v".into()));
    h.input_mut().events.push(Event::MiddleClickPaste { text: "v".into(), pos: MIDDLE });
    h.step();
    assert_eq!(h.state().later_keys, 0);
    assert!(menu().is_open(&h.ctx));
}

#[test]
fn keys_before_the_opening_press_in_its_pass_still_reach_the_panels() {
    let mut h = Harness::builder().with_size(vec2(800.0, 600.0)).build_ui_state(
        |ui, app: &mut App| {
            let fresh = |i: &egui::InputState| i.events.iter().any(|e| matches!(e, Event::Key { key: Key::Space, pressed: true, repeat: false, .. }));
            if !menu().is_open(ui.ctx()) && ui.input(fresh) {
                ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Space));
                menu().open_held_key(ui.ctx(), MIDDLE, Key::Space);
            }
            if let Some(event) = menu().show(ui.ctx(), &app.items, &RadialLabels::default()) {
                app.events.push(event);
            }
            let other = |e: &&Event| matches!(e, Event::Key { key: Key::Z | Key::Delete, pressed: true, .. });
            app.later_keys += ui.input(|i| i.events.iter().filter(other).count() + i.raw.events.iter().filter(other).count());
        },
        app(items(8)),
    );
    let key = |key, pressed, modifiers| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers };
    for e in [key(Key::Z, true, Modifiers::COMMAND), key(Key::Space, true, Modifiers::NONE), key(Key::Delete, true, Modifiers::NONE)] {
        h.input_mut().events.push(e);
    }
    h.step();
    assert!(menu().is_open(&h.ctx));
    assert_eq!(h.state().later_keys, 2, "the Ctrl+Z before Space, in both lists, and not the Delete after it");

    // Space pressed twice in the pass that opens the ring: a key between the two presses came
    // after the ring opened.
    menu().close(&h.ctx);
    h.input_mut().events.push(key(Key::Z, false, Modifiers::COMMAND));
    h.input_mut().events.push(key(Key::Delete, false, Modifiers::NONE));
    h.input_mut().events.push(key(Key::Space, false, Modifiers::NONE));
    h.step();
    h.state_mut().later_keys = 0;
    for e in [
        key(Key::Space, true, Modifiers::NONE),
        key(Key::Space, false, Modifiers::NONE),
        key(Key::Delete, true, Modifiers::NONE),
        key(Key::Space, true, Modifiers::NONE),
    ] {
        h.input_mut().events.push(e);
    }
    h.step();
    assert!(menu().is_open(&h.ctx));
    assert_eq!(h.state().later_keys, 0, "the Delete between the two presses of Space");
}

#[test]
fn a_key_equal_to_one_before_the_opening_press_and_one_after_counts_as_after() {
    let press = |key| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND };
    let raw = [press(Key::Z), press(Key::Space), press(Key::Z)];
    // The app consumed one Z and the Space: the Z left over is read as the one after the Space.
    assert_eq!(processed_before(&raw, &[press(Key::Z)], 1), 0);
    assert_eq!(processed_before(&raw, &[press(Key::Z), press(Key::Z)], 1), 1);
    assert_eq!(processed_before(&raw, &raw, 2), 2);
}

#[test]
fn small_sizes_keep_items_apart_and_inside_their_sectors() {
    for m in [menu().radius(0.0).button_size(0.0), menu().radius(0.0).button_size(60.0), menu().radius(30.0).button_size(200.0)] {
        let (inner, outer) = (m.inner_radius(), m.outer_radius());
        assert!(outer > inner, "{m:?}");
        // Each round button, sampled around its edge.
        let radius = m.button / 2.0;
        for count in 1..=MAX_ITEMS {
            let centers: Vec<Pos2> = (0..count).map(|i| m.item_center(MIDDLE, i, count, false)).collect();
            for (i, at) in centers.iter().enumerate() {
                assert!(at.distance(MIDDLE) - radius > inner, "{m:?}: item {i} of {count} over the close button");
                for step in 0..32 {
                    let edge = *at + slot_direction(step, 32) * radius * 0.999;
                    assert_eq!(hit(MIDDLE, edge, count, inner, outer), RingHit::Slot(i), "{m:?}: item {i} of {count} at {edge:?}");
                }
                for (j, other) in centers.iter().enumerate().skip(i + 1) {
                    assert!(at.distance(*other) >= 2.0 * radius - 1e-3, "{m:?}: items {i} and {j} of {count} overlap");
                }
            }
        }
    }
}

#[test]
fn a_right_to_left_ring_runs_counterclockwise() {
    let m = menu();
    let mut h = harness(items(8));
    crate::direction::apply(&h.ctx, crate::direction::Direction::Rtl);
    h.step();
    open(&mut h, MIDDLE, false);
    key(&mut h, Key::ArrowLeft, Modifiers::NONE);
    assert_eq!(focused(&h), Some(m.item_id(2)), "item 2 is on the left");
    assert_eq!(m.hit_test(&h.ctx, MIDDLE + vec2(RADIUS, 0.0), 8), RingHit::Slot(6));
    click(&mut h, MIDDLE + vec2(RADIUS, 0.0));
    assert_eq!(h.state().events, vec![RadialEvent::Picked("undo".into())], "item 6 is on the right");
}
