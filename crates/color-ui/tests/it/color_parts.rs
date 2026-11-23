//! The colour area, strip, ramp, disc and wheel: the colour maths and the edits they report.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{CancelReason, CanvasView, EditEvent, PointerButton};
use craft_color_ui::AreaMode;
use craft_color_ui::ui::{
    ColorArea, ColorDisc, ColorHud, ColorRamp, DiscMove, RampKind, RampPick, WheelGeometry, hue_at, triangle_corners, triangle_point, triangle_sv,
};
use craft_keymap::Mods;
use craft_theme::{ColorModel, HsbModel, ModelValues, Srgba};
use craft_ui::canvas::{Canvas, CanvasTool, ToolAction, ToolContext, ToolHost, ToolInput, ToolPath, ToolPointer, tool_pass};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::primitives::Draw;
use craft_ui::runtime::overlay::CloseReason;
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Fixture, Story, StoryData, StoryOutput, trial};
use egui::{Event, Id, Key, Modifiers, Pos2, Rect, Ui, pos2, vec2};

fn within(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

/// The wheel's pure geometry: a triangle point reads back as its saturation and brightness, and the ring hue is whole
/// degrees counter-clockwise from the right.
#[trial]
fn the_wheel_geometry_round_trips() {
    let center = pos2(100.0, 100.0);
    let corners = triangle_corners(center, 60.0, 210.0);
    for (s, b) in [(0.0, 1.0), (1.0, 1.0), (0.4, 0.7), (0.9, 0.2)] {
        let (read_s, read_b) = triangle_sv(corners, triangle_point(corners, s, b)).expect("a triangle point reads back");
        assert!(within(read_s, s) && within(read_b, b), "({s}, {b}) read as ({read_s}, {read_b})");
    }
    assert_eq!(hue_at(center, center + vec2(10.0, 0.0)), Some(0.0));
    assert_eq!(hue_at(center, center + vec2(0.0, -10.0)), Some(90.0), "up is 90 degrees");
    assert_eq!(hue_at(center, Pos2::new(f32::NAN, 0.0)), None);
    let geometry = WheelGeometry::new(Rect::from_min_size(Pos2::ZERO, vec2(200.0, 200.0)), 4.0, (8.0, 24.0), 0.09, 1.0);
    assert!(geometry.inner < geometry.outer && geometry.triangle < geometry.inner);
}

fn web_safe_area(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (values, values_at) = data.value_at("values", HsbModel.from_srgb(Srgba::rgb(0.4, 0.6, 0.8)));
    ColorArea::new("Saturation and brightness", AreaMode::Hue, values_at.bind(values)).quantize(6).show(ui).into()
}

/// A key on a quantising area steps by the channel's step and goes on until the colour is a different one, so each
/// press reports a new web-safe colour.
#[trial]
fn each_key_on_a_web_safe_area_reaches_another_colour() {
    let mut fixture = Fixture::story(&Story::new("web safe", web_safe_area));
    fixture.settle();
    let color = |fixture: &Fixture| HsbModel.to_srgb(&fixture.data().value("values", ModelValues::new("hsb", &[])).values);
    fixture.key(Key::Tab, Modifiers::NONE);
    fixture.idle(2);
    for key in [Key::ArrowRight, Key::ArrowRight, Key::ArrowDown, Key::ArrowDown] {
        let before = color(&fixture);
        fixture.key(key, Modifiers::NONE);
        fixture.idle(2);
        assert_ne!(color(&fixture), before, "{key:?} reaches another colour");
    }
}

/// A disc of three markers whose host selects the marker the disc reports as pressed.
struct DiscRig {
    fixture: Fixture,
    pressed: Rc<RefCell<Vec<usize>>>,
    moved: Rc<Cell<Option<DiscMove>>>,
}

impl DiscRig {
    fn new(chosen: usize) -> Self {
        let markers = [Srgba::rgb(0.9, 0.2, 0.2), Srgba::rgb(0.2, 0.9, 0.2), Srgba::rgb(0.2, 0.2, 0.9)];
        let (chosen, pressed, moved) = (Rc::new(Cell::new(chosen)), Rc::new(RefCell::new(Vec::new())), Rc::new(Cell::new(None)));
        let (p, m) = (pressed.clone(), moved.clone());
        let mut fixture = Fixture::new(move |ui| {
            let out = ColorDisc::new("Harmony", &markers, Bind::given(DiscMove::new(0, 0.0, 0.0))).chosen(chosen.get()).show(ui);
            if let Some(moved) = out.edit.value() {
                m.set(Some(*moved));
            }
            if let Some(index) = out.pressed {
                p.borrow_mut().push(index);
                chosen.set(index);
            }
        });
        fixture.room(vec2(320.0, 240.0));
        fixture.settle();
        DiscRig { fixture, pressed, moved }
    }

    fn focused(&self) -> Option<String> {
        let tree = self.fixture.tree();
        tree.name_of(craft_ui::testing::access::focused_key(tree)).map(str::to_owned)
    }

    fn marker(&self, n: usize) -> craft_ui::testing::Target {
        self.fixture.by_label(&format!("Color {n}")).unwrap_or_else(|| panic!("marker {n}"))
    }
}

/// The markers are one roving group: Tab lands on the chosen marker, a press on another focuses it in the press pass
/// and reports it, assistive focus on a marker reports it too, and the arrows move the focused marker.
#[trial]
fn the_disc_markers_take_focus_from_tab_a_press_and_assistive_technology() {
    let mut rig = DiscRig::new(1);
    rig.fixture.key(Key::Tab, Modifiers::NONE);
    rig.fixture.idle(2);
    assert_eq!(rig.focused().as_deref(), Some("Color 2"), "Tab lands on the chosen marker");
    assert!(rig.pressed.borrow().is_empty(), "focus on the chosen marker reports nothing");

    let third = rig.marker(3).center();
    rig.fixture.drag_hold_at(third, egui::Vec2::ZERO);
    assert_eq!(rig.pressed.borrow().last(), Some(&2), "the press reports the marker it grabbed");
    assert_eq!(rig.focused().as_deref(), Some("Color 3"), "and focuses it before the release");
    rig.fixture.release_at(third);
    rig.fixture.idle(2);

    let first = rig.marker(1);
    rig.fixture.access(AccessActionKind::Focus, &first);
    rig.fixture.idle(2);
    assert_eq!(rig.focused().as_deref(), Some("Color 1"));
    assert_eq!(rig.pressed.borrow().last(), Some(&0), "assistive focus on a marker reports it for the host to select");

    rig.fixture.key(Key::ArrowRight, Modifiers::NONE);
    rig.fixture.idle(2);
    assert_eq!(rig.moved.get().map(|moved| moved.marker), Some(0), "the arrows move the focused marker");
    assert_eq!(rig.focused().as_deref(), Some("Color 1"), "and leave focus on it");
}

/// Alt with Enter on the focused bar or a box picks for the colour the host is not editing, as Alt with a click does.
#[trial]
fn alt_with_enter_on_the_ramp_picks_the_other_colour() {
    let picks = Rc::new(RefCell::new(Vec::new()));
    let p = picks.clone();
    let mut fixture = Fixture::new(move |ui| {
        let out = ColorRamp::new("Color ramp", RampKind::Grayscale, Bind::given(RampPick::new(Srgba::rgb(0.5, 0.5, 0.5), false))).show(ui);
        if let Some(pick) = out.edit.value() {
            p.borrow_mut().push(pick.other);
        }
    });
    fixture.room(vec2(320.0, 80.0));
    fixture.settle();
    let press = |fixture: &mut Fixture, key: Key, modifiers: Modifiers| {
        fixture.key(key, modifiers);
        fixture.idle(2);
    };
    press(&mut fixture, Key::Tab, Modifiers::NONE);
    press(&mut fixture, Key::Enter, Modifiers::NONE);
    press(&mut fixture, Key::Enter, Modifiers::ALT);
    assert_eq!(*picks.borrow(), [false, true], "the bar: Enter, then Alt+Enter");
    press(&mut fixture, Key::Tab, Modifiers::NONE);
    press(&mut fixture, Key::Enter, Modifiers::ALT);
    assert_eq!(*picks.borrow(), [false, true, true], "the white box: Alt+Enter");
}

/// Where the host's press began.
const HUD_AT: Pos2 = Pos2::new(300.0, 200.0);

/// A HUD whose host owns the press: it sets the held state and the test moves the pointer, and applies the HUD's edits to its colour.
struct HudRig {
    fixture: Fixture,
    held: Rc<Cell<bool>>,
    color: Rc<Cell<Srgba>>,
    events: Rc<RefCell<Vec<EditEvent<Srgba>>>>,
    closed: Rc<RefCell<Vec<CloseReason>>>,
}

impl HudRig {
    fn new(start: Srgba) -> Self {
        let (held, color) = (Rc::new(Cell::new(true)), Rc::new(Cell::new(start)));
        let (events, closed) = (Rc::new(RefCell::new(Vec::new())), Rc::new(RefCell::new(Vec::new())));
        let (h, c, e, x) = (held.clone(), color.clone(), events.clone(), closed.clone());
        let mut fixture = Fixture::new(move |ui| {
            let out = ColorHud::new("Color HUD", HUD_AT, Bind::given(c.get())).start(start).held(h.get()).show(ui);
            for event in out.events {
                if let EditEvent::Update { value, .. } = &event {
                    c.set(*value);
                }
                e.borrow_mut().push(event);
            }
            if let Some(reason) = out.closed {
                x.borrow_mut().push(reason);
            }
        });
        fixture.room(vec2(600.0, 400.0));
        fixture.settle();
        HudRig { fixture, held, color, events, closed }
    }

    fn count(&self, which: impl Fn(&EditEvent<Srgba>) -> bool) -> usize {
        self.events.borrow().iter().filter(|event| which(event)).count()
    }

    /// Moves the pointer to `to` in one frame, then runs two more frames without a move: the pick is made once.
    fn move_to(&mut self, to: Pos2) {
        self.fixture.frame(vec![Event::PointerMoved(to)]);
        self.fixture.idle(2);
    }

    fn hsb(&self) -> [f32; 3] {
        let values = HsbModel.from_srgb(self.color.get()).values;
        [values[0], values[1], values[2]]
    }
}

/// The HUD maps the host's pointer through its own parts: the start colour's point sits under `at`, the pointer resting
/// at `at` picks nothing, moving over the area and then the strip picks there, one gesture covers the whole open, and
/// the host's release commits and closes with the colour kept.
#[trial]
fn color_hud_follows_the_hosts_press() {
    // Hue 210, saturation 50, brightness 80.
    let start = Srgba::rgb(0.4, 0.6, 0.8);
    let mut rig = HudRig::new(start);
    let area = rig.fixture.by_label(AreaMode::Hue.name()).expect("the area").rect;
    let strip = rig.fixture.by_label("Hue").expect("the strip").rect;
    assert!((area.left() + 0.5 * area.width() - HUD_AT.x).abs() < 2.0, "the start colour's saturation is under `at` across");
    assert!((area.top() + 0.2 * area.height() - HUD_AT.y).abs() < 2.0, "and its brightness down");

    rig.fixture.idle(2);
    assert!(rig.events.borrow().is_empty(), "no edit while the pointer rests at `at`");

    rig.move_to(area.center());
    let [hue, saturation, brightness] = rig.hsb();
    assert!((hue - 210.0).abs() < 3.0, "the area leaves the hue: {hue}");
    assert!((saturation - 50.0).abs() < 3.0, "the area's middle across is half saturation: {saturation}");
    assert!((brightness - 50.0).abs() < 3.0, "the area's middle down is half brightness: {brightness}");

    rig.move_to(strip.center());
    let [hue, saturation, brightness] = rig.hsb();
    assert!((hue - 180.0).abs() < 3.0, "the strip's middle is half the hue range: {hue}");
    assert!((saturation - 50.0).abs() < 3.0 && (brightness - 50.0).abs() < 3.0, "the strip leaves the rest: {saturation}, {brightness}");
    assert_eq!(rig.count(|event| matches!(event, EditEvent::Begin { .. })), 1, "one gesture per open");
    assert_eq!(rig.count(|event| matches!(event, EditEvent::Commit { .. })), 0, "it runs while the press is held");

    let picked = rig.color.get();
    rig.held.set(false);
    rig.fixture.idle(2);
    assert_eq!(rig.count(|event| matches!(event, EditEvent::Commit { .. })), 1, "the release commits");
    assert_eq!(rig.count(|event| matches!(event, EditEvent::Cancel { .. })), 0);
    assert_eq!(rig.color.get(), picked, "the picked colour stays");
    assert!(!rig.closed.borrow().is_empty(), "and the HUD reports that it closed");
}

/// Escape during the gesture keeps what was picked and closes the HUD.
#[trial]
fn color_hud_escape_keeps_the_picked_colour() {
    let start = Srgba::rgb(0.4, 0.6, 0.8);
    let mut rig = HudRig::new(start);
    let area = rig.fixture.by_label(AreaMode::Hue.name()).expect("the area").rect;
    rig.move_to(area.center());
    let picked = rig.color.get();
    assert_ne!(picked, start, "the pointer picked a colour");

    rig.fixture.key(Key::Escape, Modifiers::NONE);
    rig.fixture.idle(2);
    assert_eq!(rig.count(|event| matches!(event, EditEvent::Begin { .. })), 1);
    assert_eq!(rig.count(|event| matches!(event, EditEvent::Commit { .. })), 1, "Escape commits");
    assert_eq!(rig.count(|event| matches!(event, EditEvent::Cancel { .. })), 0, "and does not revert");
    assert_eq!(rig.color.get(), picked, "the picked colour stays");
    assert!(!rig.closed.borrow().is_empty(), "and the HUD reports that it closed");
}

/// A canvas tool whose secondary press opens the HUD at the press, as an app's HUD Color Picker chord does.
#[derive(Default)]
struct HudOpener {
    open: Option<Pos2>,
}

impl CanvasTool for HudOpener {
    fn press(&mut self, pointer: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        self.open = Some(pointer.pos);
        ToolAction::Claimed
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn takes(&self, button: PointerButton, _: Mods) -> bool {
        button == PointerButton::Secondary
    }

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the HUD opener of a trial")
    }
}

/// Automation drives the HUD through the tool host as a hand does: the injected secondary press opens it, the injected
/// move picks where it went (the HUD reads the pass pointer), and the injected release commits.
#[trial]
fn color_hud_picks_at_an_injected_move() {
    let start = Srgba::rgb(0.4, 0.6, 0.8);
    let (color, events, canvas) = (Rc::new(Cell::new(start)), Rc::new(RefCell::new(Vec::new())), Rc::new(Cell::new(Id::NULL)));
    let (c, e, shown_canvas) = (color.clone(), events.clone(), canvas.clone());
    let mut tool = HudOpener::default();
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let view = CanvasView::new(room, Point::new(300.0, 200.0), 1.0);
        let shown = Canvas::new("Document").document(Size::new(600.0, 400.0)).view(view).show(ui, |ui, body| {
            if let Some(pass) = tool_pass(&body.env) {
                let _ = ToolHost::new().run(ui, &pass, &mut tool);
            }
        });
        let id = shown.output.response.id;
        shown_canvas.set(id);
        let Some(at) = tool.open else { return };
        let out = ColorHud::new("Color HUD", at, Bind::given(c.get())).start(start).held(ToolHost::in_gesture(ui.ctx(), id)).show(ui);
        for event in out.events {
            if let EditEvent::Update { value, .. } = &event {
                c.set(*value);
            }
            e.borrow_mut().push(event);
        }
        if out.closed.is_some() {
            tool.open = None;
        }
    });
    fixture.room(vec2(600.0, 400.0));
    fixture.settle();
    let ctx = fixture.ctx().clone();
    let count = |which: fn(&EditEvent<Srgba>) -> bool| events.borrow().iter().filter(|event| which(event)).count();

    ToolHost::inject(&ctx, canvas.get(), ToolInput::down(PointerButton::Secondary, HUD_AT));
    fixture.run_injected().settle();
    let area = fixture.by_label(AreaMode::Hue.name()).expect("the injected press opened the HUD").rect;

    ToolHost::inject(&ctx, canvas.get(), ToolInput::moved(area.center()));
    fixture.run_injected();
    let values = HsbModel.from_srgb(color.get()).values;
    assert!((values[1] - 50.0).abs() < 3.0, "the area's middle across is half saturation: {}", values[1]);
    assert!((values[2] - 50.0).abs() < 3.0, "the area's middle down is half brightness: {}", values[2]);
    assert_eq!(count(|event| matches!(event, EditEvent::Commit { .. })), 0, "it runs while the press is held");

    ToolHost::inject(&ctx, canvas.get(), ToolInput::up(area.center()));
    fixture.run_injected().idle(2);
    assert_eq!(count(|event| matches!(event, EditEvent::Commit { .. })), 1, "the injected release commits");
}
