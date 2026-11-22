//! Touch fingers: the multi-finger tap, the modal gate, the second finger that ends a press and the two-finger pan.

use craft_behavior::press::MULTI_TAP_SLOP;
use craft_behavior::{PressCancel, PressPhase, Sense, WheelTaker};
use craft_ui::contract::{Common, Component, Container, ContainerProps, Cx, Mount, Overlay, Props as _, Role, ShowContainer as _};
use craft_ui::style::StyleCx;
use craft_ui::testing::pointer;
use craft_ui::testing::sensing::{ProbeRoot, show};
use craft_ui::testing::wheel::{register, take_share};
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Event, Id, Pos2, Rect, Response, TouchPhase, Ui, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const CENTER: Pos2 = Pos2::new(20.0, 20.0);

/// A modal layer with an empty body.
#[derive(craft_ui::contract::Component)]
#[component(container, unregistered)]
struct Modal {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
}

impl Component for Modal {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: ModalStyleKey) -> PlainStyle {
        PlainStyle::min(Vec2::new(120.0, 60.0))
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Modal {}

/// Finger `id` in `phase`, 40 points right of the one before it, moved `shift` points right.
fn touch(phase: TouchPhase, id: usize, shift: f32) -> Event {
    let pos = CENTER + Vec2::new(40.0 * id as f32 + shift, 0.0);
    Event::Touch { device_id: egui::TouchDeviceId(0), id: egui::TouchId(id as u64), phase, pos, force: None }
}

/// `fingers` fingers in `phase`.
fn all(phase: TouchPhase, fingers: usize) -> Vec<Event> {
    (0..fingers).map(|id| touch(phase, id, 0.0)).collect()
}

/// The centre of the landing points of `fingers` fingers.
fn centre(fingers: usize) -> Pos2 {
    CENTER + Vec2::new(20.0 * (fingers - 1) as f32, 0.0)
}

type Taps = Rc<RefCell<Vec<(u8, Pos2)>>>;

/// A frame that logs the tap of its pass and shows a modal layer while the returned flag holds.
fn scene() -> (impl FnMut(&mut Ui), Taps, Rc<Cell<bool>>) {
    let taps = Taps::default();
    let sink = taps.clone();
    let open = Rc::new(Cell::new(false));
    let shown = open.clone();
    let frame = move |ui: &mut Ui| {
        if let Some(tap) = craft_ui::runtime::pointer::frame(ui.ctx()).multi_tap {
            sink.borrow_mut().push((tap.fingers, tap.at));
        }
        if shown.get() {
            let props = ContainerProps::new().mount(Mount::Layer(Overlay::modal()));
            Modal { common: Common::new(), props }.id_salt("modal").show(ui, |_, _| {});
        }
    };
    (frame, taps, open)
}

/// The taps reported for `fingers` fingers that land, are moved `drift` points when it is not zero, stay `hold` seconds
/// and end in `end`.
fn tapped(fingers: usize, hold: f64, end: TouchPhase, drift: f32) -> Vec<(u8, Pos2)> {
    let (mut show, taps, _) = scene();
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    frames.frame(all(TouchPhase::Start, fingers), &mut show);
    if drift != 0.0 {
        frames.frame(vec![touch(TouchPhase::Move, 0, drift)], &mut show);
    }
    if hold > 0.0 {
        frames.advance(hold);
    }
    frames.frame(all(end, fingers), &mut show);
    frames.idle(2, &mut show);
    taps.borrow().clone()
}

#[trial]
fn two_and_three_fingers_tap_and_a_pinch_a_hold_a_cancel_or_one_finger_do_not() {
    for fingers in [2, 3] {
        let taps = tapped(fingers, 0.0, TouchPhase::End, 0.0);
        assert_eq!(taps.len(), 1, "{fingers} fingers tap once: {taps:?}");
        assert_eq!(taps[0].0 as usize, fingers);
        assert!(taps[0].1.distance(centre(fingers)) < 0.5, "the tap is at the fingers' centre: {:?}", taps[0].1);
    }
    assert_eq!(tapped(2, 0.0, TouchPhase::End, MULTI_TAP_SLOP / 2.0).len(), 1, "a finger may drift past a press's slop while the others land");
    assert!(tapped(2, 0.0, TouchPhase::End, MULTI_TAP_SLOP * 2.0).is_empty(), "a finger that moved past the tap's slop pinches or pans");
    assert!(tapped(2, 0.6, TouchPhase::End, 0.0).is_empty(), "fingers held past the tap time");
    assert!(tapped(2, 0.0, TouchPhase::Cancel, 0.0).is_empty(), "a cancelled touch");
    assert!(tapped(1, 0.0, TouchPhase::End, 0.0).is_empty(), "one finger is a press");
}

/// The taps of a touch that lands, may have a modal layer open for a moment (`opens`), and ends.
fn tapped_beside_a_modal(opens: bool) -> Vec<(u8, Pos2)> {
    let (mut show, taps, open) = scene();
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    frames.frame(all(TouchPhase::Start, 2), &mut show);
    open.set(opens);
    frames.idle(3, &mut show);
    open.set(false);
    frames.idle(1, &mut show);
    frames.frame(all(TouchPhase::End, 2), &mut show);
    frames.idle(2, &mut show);
    taps.borrow().clone()
}

#[trial]
fn a_modal_blocks_the_tap_and_a_touch_held_when_a_modal_opens_taps_nothing_after_it_closes() {
    let (mut show, taps, open) = scene();
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    open.set(true);
    frames.idle(30, &mut show);
    frames.frame(all(TouchPhase::Start, 2), &mut show);
    frames.frame(all(TouchPhase::End, 2), &mut show);
    frames.idle(2, &mut show);
    assert!(taps.borrow().is_empty(), "a modal holds: no tap outside the canvas's hole");
    open.set(false);
    frames.idle(30, &mut show);
    frames.frame(all(TouchPhase::Start, 2), &mut show);
    frames.frame(all(TouchPhase::End, 2), &mut show);
    assert_eq!(taps.borrow().len(), 1, "the modal is gone: the tap reports");

    assert_eq!(tapped_beside_a_modal(false).len(), 1, "the same touch with no modal taps");
    assert!(tapped_beside_a_modal(true).is_empty(), "a modal that opened dropped the touch under way");
}

#[trial]
fn a_second_finger_cancels_an_open_press_and_two_fingers_moving_together_give_the_wheel_a_pan() {
    let pad = vec![ProbeRoot::new("pad", Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::new(200.0, 100.0)), Sense::CLICK | Sense::DRAG)];
    let phases: Rc<RefCell<Vec<PressPhase>>> = Rc::default();
    let sink = phases.clone();
    let mut show_pad = move |ui: &mut Ui| {
        let reports = show(ui, &pad);
        sink.borrow_mut().push(reports[0].root.press.phase);
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show_pad);
    let on = Pos2::new(150.0, 120.0);
    frames.frame(vec![pointer::moved(on), pointer::press(on)], &mut show_pad);
    phases.borrow_mut().clear();
    frames.frame(all(TouchPhase::Start, 2), &mut show_pad);
    frames.idle(1, &mut show_pad);
    assert!(
        phases.borrow().iter().any(|phase| matches!(phase, PressPhase::Cancel { reason: PressCancel::Gesture, .. })),
        "the second finger ended the press: {:?}",
        phases.borrow()
    );

    // PLAN: the sheet pans a `Canvas` with `NavigationOptions::touch_gestures`; ui-contract's trials cannot reach
    // ui-canvas, so the wheel share a taker reads is what this trial checks.
    let id = Id::unique("pan_taker");
    let pans: Rc<RefCell<Vec<Vec2>>> = Rc::default();
    let sink = pans.clone();
    let mut show_taker = move |ui: &mut Ui| {
        let ctx = ui.ctx().clone();
        register(&ctx, WheelTaker::new(id, Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0))).horizontal(true).vertical(true).zoom(true));
        if let Some(wheel) = take_share(&ctx, id) {
            sink.borrow_mut().push(wheel.touch_pan);
        }
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show_taker);
    frames.frame(vec![pointer::moved(Pos2::new(40.0, 20.0))], &mut show_taker);
    frames.frame(all(TouchPhase::Start, 2), &mut show_taker);
    frames.idle(1, &mut show_taker);
    frames.frame((0..2).map(|finger| touch(TouchPhase::Move, finger, 30.0)).collect(), &mut show_taker);
    frames.idle(1, &mut show_taker);
    assert!(pans.borrow().iter().any(|pan| pan.x > 0.0), "two fingers moving right give the taker a rightward pan: {:?}", pans.borrow());
}
