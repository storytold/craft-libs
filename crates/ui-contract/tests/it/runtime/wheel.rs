//! The wheel service: the begin hook splits the pass's wheel input among last pass's takers.

use craft_behavior::overlay::{Align, Side};
use craft_behavior::{GESTURE_GAP, Sense, Wheel, WheelNotch, WheelPhase, WheelTaker, WheelUnit};
use craft_host::target::Os;
use craft_ui::contract::overlay::{Anchor, Place};
use craft_ui::contract::{
    Common, Component, Container, ContainerProps, Cx, Mount, Overlay, Pointer as _, Props as _, Role, Show as _, ShowContainer as _, Sizing,
};
use craft_ui::runtime::env::{RootEnv, WheelSpeed};
use craft_ui::style::StyleCx;
use craft_ui::testing::pointer;
use craft_ui::testing::sensing::{ProbeRoot, show};
use craft_ui::testing::wheel::{register, take, take_share};
use craft_ui::testing::{Fixture, Frames, trial};
use craft_ui_test_kit::contract::{PlainStyle, Probe, Seen};
use egui::{Event, Id, Modifiers, MouseWheelSource, MouseWheelUnit, Pos2, Rect, Response, TouchPhase, Ui, Vec2, ViewportId};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Skeleton: `frames_state::slider_takes_the_wheel_after_a_press_until_the_pointer_leaves`, second half: a press
/// arms a value control for the wheel until the pointer leaves it.
#[trial]
fn a_press_arms_a_gated_taker_until_the_pointer_leaves() {
    let panel = ProbeRoot::id("armed_panel");
    let slider = ProbeRoot::id("armed_slider");
    let panel_rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0));
    let slider_rect = Rect::from_min_size(Pos2::new(50.0, 50.0), Vec2::new(100.0, 20.0));
    let probe = ProbeRoot::new("armed_slider", slider_rect, Sense::CLICK | Sense::DRAG);
    let read = std::cell::Cell::new((None, None));
    let mut show_frame = |ui: &mut egui::Ui| {
        show(ui, std::slice::from_ref(&probe));
        register(ui.ctx(), WheelTaker::new(panel, panel_rect).vertical(true));
        register(ui.ctx(), WheelTaker::new(slider, slider_rect).vertical(true).gated(true));
        read.set((take(ui.ctx(), panel), take(ui.ctx(), slider)));
    };
    let scroll = Some(Vec2::new(0.0, -30.0));
    let on_slider = Pos2::new(70.0, 60.0);
    let mut frames = Frames::new();
    frames.idle(2, &mut show_frame);
    frames.frame(vec![pointer::moved(on_slider)], &mut show_frame);
    frames.frame(vec![pointer::wheel(-30.0)], &mut show_frame);
    assert_eq!(read.get(), (scroll, None), "before a press the panel takes the wheel");
    frames.frame(vec![pointer::press(on_slider)], &mut show_frame);
    frames.frame(vec![pointer::release(on_slider)], &mut show_frame);
    frames.frame(vec![pointer::wheel(-30.0)], &mut show_frame);
    assert_eq!(read.get(), (None, scroll), "after a press the slider takes it");
    frames.frame(vec![pointer::moved(Pos2::new(300.0, 300.0))], &mut show_frame);
    frames.frame(vec![pointer::wheel(-30.0)], &mut show_frame);
    assert_eq!(read.get(), (scroll, None), "once the pointer leaves, the panel takes it again");
}

/// A wheel event of `delta` in `unit` with `modifiers`, in device phase `phase`.
fn wheel_event(unit: MouseWheelUnit, delta: Vec2, phase: TouchPhase, modifiers: Modifiers) -> Event {
    Event::MouseWheel { unit, delta, phase, modifiers, source: MouseWheelSource::Wheel }
}

#[trial]
fn a_fractional_ctrl_wheel_is_a_pinch_and_a_whole_notch_stays_a_scroll() {
    // egui reports the Ctrl key as `ctrl` and `command` off macOS.
    let ctrl = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    let ctrl_lines = |dy: f32| wheel_event(MouseWheelUnit::Line, Vec2::new(0.0, dy), TouchPhase::Move, ctrl);
    let pinched = |shares: &[Wheel]| shares.len() == 1 && shares[0].zoom > 1.0 && shares[0].scroll == Vec2::ZERO;
    let scrolled = |shares: &[Wheel]| shares.len() == 1 && shares[0].zoom == 1.0 && shares[0].scroll.y > 0.0;
    for os in [Os::Windows, Os::Linux, Os::Mac] {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let shares = Rc::clone(&seen);
        let view = Id::unique("pinch_view");
        let mut fixture = Fixture::new(move |ui| {
            register(ui.ctx(), WheelTaker::new(view, Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0))).vertical(true).zoom(true));
            shares.borrow_mut().extend(take_share(ui.ctx(), view));
        });
        fixture.os(os);
        // As PhotoCraft sets egui: no wheel modifier is egui's own zoom, so a Ctrl+wheel reaches the view as a scroll.
        fixture.ctx().options_mut(|options| options.input_options.zoom_modifier = Modifiers::NONE);
        fixture.settle();
        fixture.frame(vec![pointer::moved(Pos2::new(200.0, 150.0))]);
        seen.borrow_mut().clear();
        let step = |fixture: &mut Fixture, events: Vec<Event>| -> Vec<Wheel> {
            fixture.frame(events);
            std::mem::take(&mut *seen.borrow_mut())
        };

        // A touchpad driver's pinch step: a fractional Ctrl+wheel line.
        let fractional = step(&mut fixture, vec![ctrl_lines(0.4)]);
        if os == Os::Mac {
            assert!(scrolled(&fractional), "{os:?}: macOS reports its pinch itself, a Ctrl+wheel stays a scroll: {fractional:?}");
        } else {
            assert!(pinched(&fractional), "{os:?}: a fractional Ctrl+wheel is a pinch step: {fractional:?}");
        }
        let rounded = step(&mut fixture, vec![ctrl_lines(1.0)]);
        if os == Os::Mac {
            assert!(scrolled(&rounded), "{os:?}: {rounded:?}");
        } else {
            assert!(pinched(&rounded), "{os:?}: a whole notch right after a pinch step is the pinch's: {rounded:?}");
        }
        fixture.advance(0.5);
        let notch = step(&mut fixture, vec![ctrl_lines(1.0)]);
        assert!(scrolled(&notch), "{os:?}: a whole notch after the pinch paused is a wheel notch: {notch:?}");

        // A native pinch ends the run of folded steps.
        step(&mut fixture, vec![ctrl_lines(0.4)]);
        let native = step(&mut fixture, vec![Event::Zoom(1.1)]);
        assert!(native.len() == 1 && (native[0].zoom - 1.1).abs() < 1e-4, "{os:?}: a native pinch reaches the view as sent: {native:?}");
        let after_native = step(&mut fixture, vec![ctrl_lines(1.0)]);
        assert!(scrolled(&after_native), "{os:?}: a whole notch after a native pinch is a wheel notch: {after_native:?}");

        // A host whose Ctrl+wheel does something else (Zoom with Trackpad Pinch off, or its own Ctrl-scroll zoom rate)
        // turns the fold off: the fractional step stays a wheel event.
        fixture.root(RootEnv::default().ctrl_wheel_pinch(false));
        fixture.advance(0.5);
        let unfolded = step(&mut fixture, vec![ctrl_lines(0.4)]);
        assert!(scrolled(&unfolded), "{os:?}: with `ctrl_wheel_pinch` off a fractional Ctrl+wheel is a wheel event: {unfolded:?}");
    }
}

#[trial]
fn a_wheel_gesture_ends_for_a_value_control_in_any_wheel_scope_and_not_for_a_scroll_region_or_the_canvas() {
    let panel = Id::unique("gesture_panel");
    let slider = Id::unique("gesture_slider");
    let canvas = Id::unique("gesture_canvas");
    let seen: RefCell<Vec<(Id, Wheel)>> = RefCell::new(Vec::new());
    let mut show_frame = |ui: &mut egui::Ui| {
        let ctx = ui.ctx();
        register(ctx, WheelTaker::new(panel, Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0))).vertical(true).scrolls(true));
        // A slider in a free wheel scope: neither gated nor a field, a value control all the same.
        register(ctx, WheelTaker::new(slider, Rect::from_min_size(Pos2::new(50.0, 50.0), Vec2::new(100.0, 20.0))).vertical(true).value(true));
        register(ctx, WheelTaker::new(canvas, Rect::from_min_size(Pos2::new(200.0, 200.0), Vec2::splat(180.0))).vertical(true).zoom(true));
        for id in [panel, slider, canvas] {
            if let Some(share) = take_share(ctx, id) {
                seen.borrow_mut().push((id, share));
            }
        }
    };
    let ends = |shares: &[(Id, Wheel)]| -> Vec<Id> {
        let ends: Vec<&(Id, Wheel)> = shares.iter().filter(|(_, share)| share.phase == WheelPhase::End).collect();
        assert!(ends.iter().all(|(_, share)| share.scroll == Vec2::ZERO), "an end share moves nothing: {ends:?}");
        ends.into_iter().map(|(id, _)| *id).collect()
    };
    // Lets the gap pass with idle frames and returns every share seen since the last call.
    let close = |frames: &mut Frames, show_frame: &mut dyn FnMut(&mut egui::Ui)| -> Vec<(Id, Wheel)> {
        frames.advance(GESTURE_GAP);
        frames.idle(2, &mut *show_frame);
        std::mem::take(&mut *seen.borrow_mut())
    };
    let (on_slider, on_panel, on_canvas) = (Pos2::new(70.0, 60.0), Pos2::new(300.0, 100.0), Pos2::new(290.0, 290.0));
    let mut frames = Frames::new();
    frames.idle(2, &mut show_frame);
    seen.borrow_mut().clear();

    // The gap ends a gesture of a value control: one end share, and a pass is asked for when the gap ends.
    frames.frame(vec![pointer::moved(on_slider)], &mut show_frame);
    frames.frame(vec![pointer::wheel(-30.0)], &mut show_frame);
    let turned = frames.ctx.input(|input| input.time);
    // egui smooths a large wheel step over a few frames and asks for a pass at once until it is spent; the gap's wake
    // is the delay once egui has settled.
    let mut delay = Some(0.0);
    for _ in 0..60 {
        frames.idle(1, &mut show_frame);
        delay = frames.repaint_delay().map(|delay| delay.as_secs_f64());
        if delay != Some(0.0) {
            break;
        }
    }
    let left = GESTURE_GAP - (frames.ctx.input(|input| input.time) - turned);
    assert!(
        left > 0.0 && delay.is_some_and(|delay| (delay - left).abs() < 0.02),
        "a running gesture of a value control asks for a pass when its gap ends: {delay:?} against {left}"
    );
    let shares = close(&mut frames, &mut show_frame);
    assert!(shares.first().is_some_and(|(id, share)| *id == slider && share.phase == WheelPhase::Move), "the slider took the wheel: {shares:?}");
    assert_eq!(ends(&shares), [slider], "the gap ends the slider's gesture once");

    // A press ends it at once.
    frames.frame(vec![pointer::moved(on_slider)], &mut show_frame);
    frames.frame(vec![pointer::wheel(-30.0)], &mut show_frame);
    seen.borrow_mut().clear();
    frames.frame(vec![pointer::press(on_panel)], &mut show_frame);
    let pressed = std::mem::take(&mut *seen.borrow_mut());
    assert_eq!(ends(&pressed), [slider], "a press ends the slider's gesture in its pass");
    frames.frame(vec![pointer::release(on_panel)], &mut show_frame);
    assert!(ends(&close(&mut frames, &mut show_frame)).is_empty(), "an ended gesture ends once");

    // A scroll region and the canvas hear no end of their own.
    for at in [on_panel, on_canvas] {
        frames.frame(vec![pointer::moved(at)], &mut show_frame);
        frames.frame(vec![pointer::wheel(-30.0)], &mut show_frame);
        let shares = close(&mut frames, &mut show_frame);
        assert!(!shares.is_empty(), "the wheel at {at:?} reached its taker");
        assert!(ends(&shares).is_empty(), "no end share for the taker at {at:?}: {shares:?}");
    }

    // A device that reports phases ends the gesture itself; its end reaches the owner as sent, and only once.
    for (at, owner) in [(on_slider, slider), (on_panel, panel)] {
        frames.frame(vec![pointer::moved(at)], &mut show_frame);
        for (dy, phase) in [(-10.0, TouchPhase::Start), (-10.0, TouchPhase::Move), (0.0, TouchPhase::End)] {
            frames.frame(vec![wheel_event(MouseWheelUnit::Point, Vec2::new(0.0, dy), phase, Modifiers::NONE)], &mut show_frame);
        }
        assert_eq!(ends(&close(&mut frames, &mut show_frame)), [owner], "the device's end reaches the gesture's owner once");
    }
}

/// A popup layer that holds one probe.
#[derive(craft_ui::contract::Component)]
#[component(container)]
struct Popup {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
}

impl Component for Popup {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: PopupStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Popup {}

#[trial]
fn a_wheel_over_an_open_popup_goes_to_the_popup_and_not_to_the_region_under_it() {
    let anchor = Rect::from_min_size(Pos2::new(200.0, 100.0), Vec2::new(30.0, 20.0));
    let (field_seen, popup_seen) = (RefCell::new(Seen::default()), RefCell::new(Seen::default()));
    let open = std::cell::Cell::new(false);
    let popup_rect = std::cell::Cell::new(Rect::NOTHING);
    let field = std::cell::Cell::new(Id::NULL);
    let popup = Id::unique("wheel_popup");
    let panel = Id::unique("wheel_panel");
    let shares: RefCell<Vec<(Id, Wheel)>> = RefCell::new(Vec::new());
    let mut show_frame = |ui: &mut Ui| {
        let ctx = ui.ctx().clone();
        let field_rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(100.0, 20.0));
        field.set(Probe::new(&field_seen).size(field_rect.size()).sense(Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS).show(ui).id);
        if open.get() {
            let overlay = Overlay::popover().place(Place::anchored(Anchor::Rect(anchor), Side::Below, Align::Start));
            let props = ContainerProps::new().mount(Mount::Layer(overlay)).sizing(Sizing::Content);
            let out = Popup { common: Common::new(), props }.id_salt("wheel_popup_layer").show(ui, |ui, _| {
                Probe::new(&popup_seen).size(Vec2::new(120.0, 60.0)).show(ui);
            });
            popup_rect.set(out.output.rect);
        }
        register(&ctx, WheelTaker::new(panel, Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0))).vertical(true).scrolls(true));
        register(&ctx, WheelTaker::new(field.get(), field_rect).vertical(true).field(true));
        if open.get() {
            register(&ctx, WheelTaker::new(popup, popup_rect.get()).vertical(true).scrolls(true));
        }
        for id in [panel, field.get(), popup] {
            if let Some(share) = take_share(&ctx, id) {
                shares.borrow_mut().push((id, share));
            }
        }
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show_frame);
    pointer::click(&mut frames, Pos2::new(50.0, 10.0), &mut show_frame);
    // Each wheel starts its own gesture, which splits by position.
    let wheel_at = |frames: &mut Frames, at: Pos2, show_frame: &mut dyn FnMut(&mut Ui)| -> Vec<Id> {
        frames.advance(GESTURE_GAP);
        frames.frame(vec![pointer::moved(at)], &mut *show_frame);
        shares.borrow_mut().clear();
        frames.frame(vec![pointer::wheel(-30.0)], &mut *show_frame);
        shares.borrow().iter().map(|(id, _)| *id).collect()
    };

    let beside = Pos2::new(300.0, 300.0);
    assert_eq!(wheel_at(&mut frames, beside, &mut show_frame), [field.get()], "the focused field takes the wheel over the region");

    open.set(true);
    frames.idle(4, &mut show_frame);
    assert!(popup_rect.get().is_positive(), "the popup shows: {:?}", popup_rect.get());
    let on_popup = popup_rect.get().center();
    assert_eq!(
        wheel_at(&mut frames, on_popup, &mut show_frame),
        [popup],
        "the popup under the pointer takes the wheel, not the focused field or the region"
    );
}

/// What the view under the pointer took over one step: the summed scroll, the product of the zooms and the number of
/// shares.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Took {
    scroll: Vec2,
    zoom: f32,
    shares: usize,
}

/// One step of the wheel-speed trial in `viewport`: a frame with `events` that runs two passes, then a frame with no
/// input, as an app's tick between OS frames runs one. Returns what the view took over both frames.
fn speed_step(
    frames: &mut Frames,
    viewport: ViewportId,
    events: Vec<Event>,
    show: &mut dyn FnMut(&mut egui::Ui),
    rerun: &Cell<bool>,
    seen: &RefCell<Vec<Wheel>>,
) -> Took {
    seen.borrow_mut().clear();
    rerun.set(true);
    let passes = frames.frame_in_viewport(viewport, events, &mut *show);
    rerun.set(false);
    assert_eq!(passes, 2, "the wheel frame runs a second pass, which gets none of the frame's events");
    frames.frame_in_viewport(viewport, Vec::new(), &mut *show);
    let shares = std::mem::take(&mut *seen.borrow_mut());
    Took {
        scroll: shares.iter().fold(Vec2::ZERO, |sum, share| sum + share.scroll),
        zoom: shares.iter().map(|share| share.zoom).product(),
        shares: shares.len(),
    }
}

/// The wheel-speed trial's steps on `os` with the user's wheel speed `speed`: a line notch, a point delta, a zoom
/// notch (Ctrl+wheel, Cmd+wheel on macOS) in the main window, then a point delta in a second window.
fn speed_steps(os: Os, speed: WheelSpeed) -> [Took; 4] {
    let view = Id::unique("wheel_speed_view");
    let seen = RefCell::new(Vec::new());
    let rerun = Cell::new(false);
    let mut show = |ui: &mut egui::Ui| {
        register(ui.ctx(), WheelTaker::new(view, Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0))).vertical(true).zoom(true));
        let share = take_share(ui.ctx(), view);
        seen.borrow_mut().extend(share);
        if rerun.get() && ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("the wheel frame runs twice");
        }
    };
    let at = Pos2::new(200.0, 150.0);
    let mut frames = Frames::new();
    craft_ui::testing::windows::set_os(&frames.ctx, os);
    frames.root(RootEnv::default().wheel_speed(speed));
    frames.idle(2, &mut show);
    frames.frame(vec![pointer::moved(at)], &mut show);
    // egui reports Ctrl as `ctrl` and `command` off macOS, and Cmd as `mac_cmd` and `command` on it: egui's zoom
    // modifier either way.
    let zoom_mods = if os == Os::Mac {
        Modifiers { mac_cmd: true, command: true, ..Modifiers::NONE }
    } else {
        Modifiers { ctrl: true, command: true, ..Modifiers::NONE }
    };
    let zoom_notch = wheel_event(MouseWheelUnit::Line, Vec2::new(0.0, 1.0), TouchPhase::Move, zoom_mods);
    let root = ViewportId::ROOT;
    let line = speed_step(&mut frames, root, vec![pointer::wheel_lines(-1.0)], &mut show, &rerun, &seen);
    let point = speed_step(&mut frames, root, vec![pointer::wheel(-30.0)], &mut show, &rerun, &seen);
    let zoom = speed_step(&mut frames, root, vec![zoom_notch], &mut show, &rerun, &seen);
    let second = ViewportId::from_hash_of("wheel speed second window");
    frames.frame_in_viewport(second, vec![pointer::moved(at)], &mut show);
    frames.frame_in_viewport(second, Vec::new(), &mut show);
    let secondary = speed_step(&mut frames, second, vec![pointer::wheel(-30.0)], &mut show, &rerun, &seen);
    [line, point, zoom, secondary]
}

/// SG-31: the runtime multiplies each wheel delta by the user's `WheelSpeed` in the raw-input step, `line` for notches
/// and `point` for touchpad deltas, in every window. A frame's events reach only its first pass, so a delta is scaled
/// once however many passes the frame runs and whatever frames follow it. A Ctrl+wheel (Cmd+wheel) zooms by the
/// device's notch: its distance does not follow the speed.
#[trial]
fn the_users_wheel_speed_scales_each_delta_once_and_leaves_zoom_alone() {
    let speed = WheelSpeed::new(3.0, 2.0);
    let close = |a: Vec2, b: Vec2| (a - b).length() < 1e-3;
    for os in [Os::Windows, Os::Linux, Os::Mac] {
        let [line, point, zoom, secondary] = speed_steps(os, WheelSpeed::default());
        let [fast_line, fast_point, fast_zoom, fast_secondary] = speed_steps(os, speed);
        for (name, took) in [("line", fast_line), ("point", fast_point), ("zoom", fast_zoom), ("second window", fast_secondary)] {
            assert_eq!(took.shares, 1, "{os:?} {name}: one share over the frame's two passes and the frame after: {took:?}");
        }
        assert!(
            line.scroll.y != 0.0 && close(fast_line.scroll, line.scroll * speed.line),
            "{os:?}: a notch scrolls `line` times as far: {fast_line:?} against {line:?}"
        );
        assert!(
            point.scroll.y != 0.0 && close(fast_point.scroll, point.scroll * speed.point),
            "{os:?}: a point delta scrolls `point` times as far: {fast_point:?} against {point:?}"
        );
        assert!(
            zoom.zoom != 1.0 && (fast_zoom.zoom - zoom.zoom).abs() < 1e-4,
            "{os:?}: a zoom notch zooms as far at any speed: {fast_zoom:?} against {zoom:?}"
        );
        assert!(close(fast_zoom.scroll, Vec2::ZERO), "{os:?}: a zoom notch scrolls nothing: {fast_zoom:?}");
        assert!(
            secondary.scroll.y != 0.0 && close(fast_secondary.scroll, secondary.scroll * speed.point),
            "{os:?}: a second window's delta is scaled too: {fast_secondary:?} against {secondary:?}"
        );
    }
}

/// The points of one wheel line for the takers below.
const NOTCH: f32 = 25.0;

/// A wheel taker that reads its share each pass, eases the share's line scroll as a canvas eases its line pan, and
/// marks its gesture moved when asked: the `Pointer` methods, called as a component calls them. Each pass logs the
/// share it took and the step its ease paid.
#[derive(craft_ui::contract::Component)]
struct Easer<'a> {
    #[common]
    common: Common,
    mark: bool,
    passes: &'a RefCell<Vec<(Option<Wheel>, Vec2)>>,
}

impl Component for Easer<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: EaserStyleKey) -> PlainStyle {
        PlainStyle::min(Vec2::new(200.0, 120.0))
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let share = cx.take_wheel();
        if self.mark {
            cx.mark_wheel_moved();
        }
        let pan = share.filter(|share| share.unit == WheelUnit::Line).map_or(Vec2::ZERO, |share| share.scroll);
        let step = cx.ease_wheel(pan);
        self.passes.borrow_mut().push((share, step));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// A scroll of `dy` points from `source` in device phase `phase`.
fn swipe(dy: f32, phase: TouchPhase, source: MouseWheelSource) -> Event {
    Event::MouseWheel { unit: MouseWheelUnit::Point, delta: Vec2::new(0.0, dy), phase, modifiers: Modifiers::NONE, source }
}

/// The shares logged since the last call.
fn shares(log: &RefCell<Vec<(Option<Wheel>, Vec2)>>) -> Vec<Wheel> {
    log.borrow_mut().drain(..).filter_map(|(share, _)| share).collect()
}

/// Two frames with no events, through the trial's own frame runner.
fn idle(frames: &mut Frames, wheel: &mut impl FnMut(&mut Frames, Vec<Event>) -> Vec<Wheel>) {
    for _ in 0..2 {
        wheel(frames, Vec::new());
    }
}

/// The steps logged since the last call, one per pass.
fn steps(log: &RefCell<Vec<(Option<Wheel>, Vec2)>>) -> Vec<Vec2> {
    log.borrow_mut().drain(..).map(|(_, step)| step).collect()
}

#[trial]
fn a_wheel_gesture_keeps_its_gathered_scroll_across_a_momentum_run_and_starts_again_on_a_new_gesture() {
    let (seen, other) = (RefCell::new(Vec::new()), RefCell::new(Vec::new()));
    let (mark, mark_other, rerun) = (Cell::new(false), Cell::new(false), Cell::new(false));
    let at = Cell::new(Pos2::ZERO);
    let mut show_frame = |ui: &mut egui::Ui| {
        if rerun.take() {
            ui.ctx().request_discard("the trial reruns the pass");
        }
        ui.horizontal(|ui| {
            let owner = Easer { common: Common::new(), mark: mark.get(), passes: &seen }.id_salt("owner").show(ui);
            let beside = Easer { common: Common::new(), mark: mark_other.get(), passes: &other }.id_salt("beside").show(ui);
            for view in [&owner, &beside] {
                register(ui.ctx(), WheelTaker::new(view.id, view.rect).vertical(true).notch(WheelNotch::Points(NOTCH)));
            }
            at.set(owner.rect.center());
        });
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show_frame);
    frames.frame(vec![pointer::moved(at.get())], &mut show_frame);
    let mut wheel = |frames: &mut Frames, events: Vec<Event>| -> Vec<Wheel> {
        frames.frame(events, &mut show_frame);
        shares(&other);
        shares(&seen)
    };
    wheel(&mut frames, Vec::new());

    // A line is the owner's notch in points, and the first share of a gesture gathers itself. The frame runs a second
    // pass, which has no input and gathers nothing again.
    rerun.set(true);
    let first = wheel(&mut frames, vec![pointer::wheel_lines(-1.0)]);
    assert_eq!(first.len(), 1, "one share, in the first pass: {first:?}");
    let one = first[0];
    assert_eq!(one.scroll.y.abs(), NOTCH, "a line is the owner's notch: {one:?}");
    assert_eq!((one.gathered, one.moved), (one.scroll, false), "a gesture's first share gathers itself: {one:?}");

    // Later shares gather on; another taker's mark leaves the gesture alone, the owner's marks it.
    mark_other.set(true);
    let two = wheel(&mut frames, vec![pointer::wheel_lines(-1.0)])[0];
    mark_other.set(false);
    mark.set(true);
    let three = wheel(&mut frames, vec![pointer::wheel_lines(-1.0)])[0];
    mark.set(false);
    let four = wheel(&mut frames, vec![pointer::wheel_lines(-1.0)])[0];
    assert_eq!(two.gathered, one.scroll + two.scroll, "the gesture gathers in its owner's points: {two:?}");
    assert_eq!(three.gathered, two.gathered + three.scroll);
    assert!(!two.moved && !three.moved, "another taker's mark is not the owner's: {three:?}");
    assert!(four.moved && four.gathered == three.gathered + four.scroll, "the owner's mark holds for the gesture: {four:?}");

    // The gap starts a new gesture.
    frames.advance(GESTURE_GAP);
    idle(&mut frames, &mut wheel);
    let five = wheel(&mut frames, vec![pointer::wheel_lines(-1.0)])[0];
    assert_eq!((five.gathered, five.moved), (five.scroll, false), "a new gesture after the gap starts again: {five:?}");

    // So does a press, well within the gap.
    mark.set(true);
    wheel(&mut frames, vec![pointer::wheel_lines(-1.0)]);
    mark.set(false);
    wheel(&mut frames, vec![pointer::press(at.get())]);
    wheel(&mut frames, vec![pointer::release(at.get())]);
    let six = wheel(&mut frames, vec![pointer::wheel_lines(-1.0)])[0];
    assert_eq!((six.gathered, six.moved), (six.scroll, false), "a press starts a new gesture: {six:?}");

    // A momentum run right after its swipe's device end goes on with the swipe's gathered scroll and mark.
    frames.advance(GESTURE_GAP);
    idle(&mut frames, &mut wheel);
    wheel(&mut frames, vec![swipe(-10.0, TouchPhase::Start, MouseWheelSource::Trackpad)]);
    mark.set(true);
    wheel(&mut frames, vec![swipe(-10.0, TouchPhase::Move, MouseWheelSource::Trackpad)]);
    mark.set(false);
    wheel(&mut frames, vec![swipe(0.0, TouchPhase::End, MouseWheelSource::Trackpad)]);
    let run = wheel(&mut frames, vec![swipe(-10.0, TouchPhase::Move, MouseWheelSource::Momentum)]);
    assert!(run.first().is_some_and(|share| share.gathered.y == -30.0 && share.moved), "the momentum run continues its swipe: {run:?}");

    // A momentum run that comes after the gap is a gesture of its own.
    frames.advance(GESTURE_GAP);
    idle(&mut frames, &mut wheel);
    wheel(&mut frames, vec![swipe(-10.0, TouchPhase::Start, MouseWheelSource::Trackpad)]);
    wheel(&mut frames, vec![swipe(0.0, TouchPhase::End, MouseWheelSource::Trackpad)]);
    frames.advance(GESTURE_GAP);
    let late = wheel(&mut frames, vec![swipe(-10.0, TouchPhase::Move, MouseWheelSource::Momentum)]);
    assert!(late.first().is_some_and(|share| share.gathered == share.scroll && !share.moved), "a late momentum run starts again: {late:?}");
}

#[trial]
fn direct_precise_input_cancels_only_older_motion_on_its_axis() {
    let seen = RefCell::new(Vec::new());
    let rerun = Cell::new(false);
    let at = Cell::new(Pos2::ZERO);
    let mut show_frame = |ui: &mut egui::Ui| {
        if rerun.take() {
            ui.ctx().request_discard("the trial reruns the pass");
        }
        let view = Easer { common: Common::new(), mark: false, passes: &seen }.id_salt("eased").show(ui);
        register(ui.ctx(), WheelTaker::new(view.id, view.rect).horizontal(true).vertical(true).notch(WheelNotch::Points(NOTCH)));
        at.set(view.rect.center());
    };
    let line = |dx: f32, dy: f32| Event::MouseWheel {
        unit: MouseWheelUnit::Line,
        delta: Vec2::new(dx, dy),
        phase: TouchPhase::Move,
        modifiers: Modifiers::NONE,
        source: MouseWheelSource::Wheel,
    };
    let mut frames = Frames::new();
    frames.idle(2, &mut show_frame);
    frames.frame(vec![pointer::moved(at.get())], &mut show_frame);
    steps(&seen);

    // A diagonal notch owes motion on both axes; the pass that fed it pays a first part, a second pass of the same
    // frame pays nothing, and the next frame pays more.
    rerun.set(true);
    let passes = frames.frame(vec![line(-1.0, -1.0)], &mut show_frame);
    let fed = steps(&seen);
    assert_eq!((passes, fed.len()), (2, 2), "the frame ran two passes: {fed:?}");
    assert!(fed[0].x < 0.0 && fed[0].y < 0.0 && fed[0].y > -NOTCH, "the notch pays part at once on both axes: {fed:?}");
    assert_eq!(fed[1], Vec2::ZERO, "a second pass of the frame pays nothing");
    let mut step = |frames: &mut Frames, events: Vec<Event>| -> Vec2 {
        frames.frame(events, &mut show_frame);
        steps(&seen).first().copied().unwrap_or(Vec2::ZERO)
    };
    let tail = step(&mut frames, Vec::new());
    assert!(tail.x < 0.0 && tail.y < 0.0, "the owed motion keeps paying: {tail:?}");

    // A trackpad scroll on y drops y's owed tail and only y's: x's keeps paying.
    let cut = step(&mut frames, vec![swipe(-3.0, TouchPhase::Move, MouseWheelSource::Trackpad)]);
    assert!(cut.x < 0.0 && cut.y == 0.0, "the point scroll stops the y tail and leaves the x tail: {cut:?}");
    let after = step(&mut frames, Vec::new());
    assert!(after.x < 0.0 && after.y == 0.0, "the y tail stays gone: {after:?}");

    // Motion fed after the point scroll is newer than it, and pays.
    let again = step(&mut frames, vec![line(0.0, -1.0)]);
    assert!(again.y < 0.0, "a later notch eases again: {again:?}");

    // A press drops every ease.
    assert_eq!(step(&mut frames, vec![pointer::press(at.get())]), Vec2::ZERO, "a press stops the owed motion");
    assert_eq!(step(&mut frames, vec![pointer::release(at.get())]), Vec2::ZERO);
}
