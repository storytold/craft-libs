//! The colour area, strip, ramp, disc and wheel: the colour maths and the edits they report.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{CancelReason, CanvasView, EditEvent, PointerButton};
use craft_color_ui::AreaMode;
use craft_color_ui::ui::{
    ColorArea, ColorDisc, ColorHud, ColorRamp, ColorWheel, DiscMove, DiscRadius, RampKind, RampPick, WheelGeometry, WheelInner, hue_at,
    hue_direction, patch_at, triangle_corners, triangle_point, triangle_sv,
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

/// The segmented disc's patch centres: 24 sectors by 5 rings, each hue and radius in one patch, the hue wrapping; and the
/// brightness radius puts a marker at its brightness.
#[trial]
fn segmented_disc_patches_and_brightness_radius() {
    let mut centres: Vec<(u32, u32)> = Vec::new();
    for sector in 0..24 {
        for ring in 0..5 {
            let (hue, radial) = patch_at(sector as f32 * 15.0 + 3.0, (ring as f32 + 0.3) / 5.0);
            centres.push((hue.to_bits(), radial.to_bits()));
        }
    }
    centres.sort_unstable();
    centres.dedup();
    assert_eq!(centres.len(), 120, "every sector and ring has its own patch");
    assert_eq!(patch_at(7.0, 1.0), (0.0, 0.9), "7 degrees is still the red sector; the rim is the last ring");
    assert_eq!(patch_at(8.0, 0.0).0, 15.0, "8 degrees is the next sector");
    assert_eq!(patch_at(355.0, 0.5).0, 0.0, "the hue wraps to the red sector");
    assert_eq!(patch_at(-10.0, 0.5).0, 345.0, "a negative hue wraps");

    // A saturated red at 80 % brightness: on the rim of a saturation disc, 0.8 of the way on a brightness disc.
    let markers = [Srgba::rgb(0.8, 0.0, 0.0), Srgba::BLACK];
    let distance = |radius: DiscRadius| {
        let mut fixture = Fixture::new(move |ui| {
            let _ = ColorDisc::new("Harmony", &markers, Bind::kept()).radius(radius).show(ui);
        });
        fixture.room(vec2(320.0, 240.0));
        fixture.settle();
        let red = fixture.by_label("Color 1").expect("marker 1").center();
        let centre = fixture.by_label("Color 2").expect("marker 2").center();
        red.distance(centre)
    };
    let ratio = distance(DiscRadius::Brightness) / distance(DiscRadius::Saturation);
    assert!((ratio - 0.8).abs() < 0.02, "the brightness radius places the marker at its brightness: {ratio}");
}

/// A square wheel's rig: the host keeps the values the wheel reports and the marker it picked and holds.
struct WheelRig {
    fixture: Fixture,
    values: Rc<Cell<ModelValues>>,
    picked: Rc<Cell<Option<usize>>>,
    held: Rc<Cell<Option<usize>>>,
}

impl WheelRig {
    fn new(inner: WheelInner, markers: usize) -> Self {
        let colors = [Srgba::rgb(0.9, 0.2, 0.2), Srgba::rgb(0.2, 0.9, 0.2), Srgba::rgb(0.2, 0.2, 0.9)];
        let names = ["Warm", "Green", "Cool"].map(|name| craft_ui::runtime::env::Label::from(name.to_owned())).to_vec();
        let values = Rc::new(Cell::new(ModelValues::new("hsb", &[200.0, 50.0, 50.0])));
        let (picked, held) = (Rc::new(Cell::new(None)), Rc::new(Cell::new(None)));
        let (v, p, h) = (values.clone(), picked.clone(), held.clone());
        let mut fixture = Fixture::new(move |ui| {
            let out =
                ColorWheel::new("Color wheel", Bind::given(v.get())).inner(inner).markers(&colors[..markers]).marker_names(names.clone()).show(ui);
            if let Some(values) = out.edit.value() {
                v.set(*values);
            }
            if out.picked.is_some() {
                p.set(out.picked);
            }
            if out.held.is_some() {
                h.set(out.held);
            }
        });
        fixture.room(vec2(320.0, 320.0));
        fixture.settle();
        WheelRig { fixture, values, picked, held }
    }

    fn hue(&self) -> f32 {
        self.values.get().values[0]
    }
}

/// A square wheel hands a press between the square and the ring to the square; its named markers speak their colour,
/// pick on a click, and a press held on one still drags the ring; Shift snaps a ring drag to 45 degrees, in both
/// variants.
#[trial]
fn a_square_wheel_hands_the_inner_press_to_the_square_and_its_markers_pick() {
    let mut rig = WheelRig::new(WheelInner::Square, 3);
    let square = rig.fixture.by_label("Saturation and brightness").expect("the square").rect;
    rig.fixture.click_at(pos2(square.right() + 2.0, square.center().y));
    rig.fixture.idle(2);
    let [hue, saturation, brightness, _] = rig.values.get().values;
    assert_eq!(hue, 200.0, "the square leaves the hue");
    assert_eq!(saturation, 100.0, "beside the square's right edge is full saturation");
    assert!((brightness - 50.0).abs() < 4.0, "level with the square's middle is half brightness: {brightness}");

    let warm = rig.fixture.by_label("Warm").expect("marker");
    let hex = Srgba::rgb(0.9, 0.2, 0.2).to_hex();
    assert_eq!(rig.fixture.tree().value_of(warm.node), Some(hex.as_str()), "a named marker speaks its colour as its value");
    let warm = warm.center();
    rig.fixture.click_at(warm);
    rig.fixture.idle(2);
    assert_eq!(rig.picked.get(), Some(0), "a click on a marker picks it");

    let green = rig.fixture.by_label("Green").expect("marker").center();
    rig.fixture.drag_hold_at(green, egui::Vec2::ZERO);
    assert_eq!(rig.held.get(), Some(1), "a press on a marker holds it");
    assert!((rig.hue() - 120.0).abs() <= 1.0, "and the ring takes the press: {}", rig.hue());
    rig.fixture.release_at(green);
    rig.fixture.idle(2);

    for inner in [WheelInner::Triangle, WheelInner::Square] {
        let mut rig = WheelRig::new(inner, 0);
        let wheel = rig.fixture.by_label("Color wheel").expect("the wheel").rect;
        let (centre, reach) = (wheel.center(), wheel.width().min(wheel.height()) / 2.0 - 8.0);
        let from = centre + hue_direction(0.0) * reach;
        let to = centre + hue_direction(25.0) * reach;
        rig.fixture.hold(Modifiers::SHIFT);
        rig.fixture.drag_at(from, to - from);
        rig.fixture.idle(2);
        rig.fixture.hold(Modifiers::NONE);
        assert_eq!(rig.hue(), 45.0, "{inner:?}: Shift snaps 25 degrees to 45");
        let mut free = WheelRig::new(inner, 0);
        free.fixture.drag_at(from, to - from);
        free.fixture.idle(2);
        assert!((free.hue() - 25.0).abs() <= 1.0, "{inner:?}: without Shift the hue follows the pointer: {}", free.hue());
    }
}

/// A primary click reports the grabbed marker and the point under the release; a secondary click reports the marker in
/// reach, else the marker whose spoke it is on, else neither, and nothing outside the circle.
#[trial]
fn a_disc_click_reports_its_point_and_a_secondary_click_what_it_hit() {
    let markers = [Srgba::rgb(0.8, 0.0, 0.0), Srgba::rgb(0.0, 0.8, 0.8), Srgba::rgb(0.0, 0.8, 0.0)];
    let (clicked, context) = (Rc::new(Cell::new(None)), Rc::new(Cell::new(None)));
    let (k, c) = (clicked.clone(), context.clone());
    let mut fixture = Fixture::new(move |ui| {
        let out = ColorDisc::new("Harmony", &markers, Bind::kept()).show(ui);
        if out.clicked.is_some() {
            k.set(out.clicked);
        }
        if out.context.is_some() {
            c.set(out.context);
        }
    });
    fixture.room(vec2(320.0, 240.0));
    fixture.settle();
    // Red at 0 degrees and cyan at 180 are on the rim, opposite: their midpoint is the centre.
    let (red, cyan) = (fixture.by_label("Color 1").expect("marker").center(), fixture.by_label("Color 2").expect("marker").center());
    let (centre, radius) = (red.lerp(cyan, 0.5), red.distance(cyan) / 2.0);
    let at = |hue: f32, share: f32| centre + hue_direction(hue) * (radius * share);

    let up = at(90.0, 0.5);
    fixture.click_at(up);
    fixture.idle(2);
    let hit = clicked.get().expect("a click reports");
    assert_eq!(hit.marker, None, "a press away from every marker grabs none");
    assert!((hit.hue - 90.0).abs() <= 1.0 && (hit.radial - 0.5).abs() < 0.05, "the point under the release: {hit:?}");
    clicked.set(None);
    fixture.click_at(red);
    fixture.idle(2);
    assert_eq!(clicked.get().map(|hit| hit.marker), Some(Some(0)), "a click on a marker reports it");

    let secondary = |fixture: &mut Fixture, at: Pos2| {
        context.set(None);
        for pressed in [true, false] {
            fixture.frame(vec![
                Event::PointerMoved(at),
                Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Modifiers::NONE },
            ]);
        }
        fixture.idle(2);
        context.get()
    };
    let on_marker = secondary(&mut fixture, red).expect("a secondary click in the circle reports");
    assert_eq!((on_marker.marker, on_marker.spoke), (Some(0), None), "the marker in reach");
    let on_spoke = secondary(&mut fixture, at(120.0, 0.5)).expect("reports");
    assert_eq!((on_spoke.marker, on_spoke.spoke), (None, Some(2)), "else the marker whose spoke it is on");
    let between = secondary(&mut fixture, at(300.0, 0.5)).expect("reports");
    assert_eq!((between.marker, between.spoke), (None, None), "else neither");
    assert!((between.hue - 300.0).abs() <= 1.0, "with the point: {between:?}");
    assert!(secondary(&mut fixture, centre + vec2(radius * 1.05, radius * 0.5)).is_none(), "nothing outside the circle");
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

/// A colour as a host that keeps 8-bit colours stores it.
fn eight_bit(color: Srgba) -> Srgba {
    let [r, g, b, _] = color.to_rgba().0;
    Srgba::rgb(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0)
}

/// Drags the first marker of a brightness disc 60 points left in 20 steps, the host applying each update and the commit
/// as `apply` makes them of the marker's colour at the press and its colour now, and giving the disc its round trip
/// `stored`. Returns how many updates arrived and how the drag ended (`None`: committed).
fn brightness_disc_drag(
    stored: Option<fn(usize, Srgba) -> Srgba>,
    apply: impl Fn(Srgba, Srgba, &DiscMove) -> Srgba + 'static,
) -> (usize, Vec<Option<CancelReason>>) {
    let colours = Rc::new(RefCell::new(vec![eight_bit(Srgba::rgb(0.9, 0.3, 0.2)), Srgba::rgb(0.2, 0.5, 0.8)]));
    let start = colours.borrow()[0];
    let (updates, ends) = (Rc::new(Cell::new(0)), Rc::new(RefCell::new(Vec::new())));
    let (shown, u, e) = (colours.clone(), updates.clone(), ends.clone());
    let mut fixture = Fixture::new(move |ui| {
        let markers = shown.borrow().clone();
        let mut disc = ColorDisc::new("Harmony", &markers, Bind::kept()).radius(DiscRadius::Brightness);
        if let Some(stored) = &stored {
            disc = disc.stored_as(stored);
        }
        let out = disc.show(ui);
        for event in out.edit.events.iter() {
            let moved = match event {
                EditEvent::Update { value, .. } => {
                    u.set(u.get() + 1);
                    value
                }
                EditEvent::Commit { after, .. } => {
                    e.borrow_mut().push(None);
                    after
                }
                EditEvent::Cancel { reason, .. } => {
                    e.borrow_mut().push(Some(*reason));
                    continue;
                }
                _ => continue,
            };
            let mut all = shown.borrow_mut();
            all[moved.marker] = apply(start, all[moved.marker], moved);
        }
    });
    fixture.room(vec2(320.0, 240.0));
    fixture.settle();
    let from = fixture.by_label("Color 1").expect("the first marker").center();
    fixture.frame(vec![craft_ui::testing::pointer::moved(from)]);
    fixture.frame(vec![craft_ui::testing::pointer::press(from)]);
    for step in 1..=DRAG_STEPS {
        fixture.frame(vec![craft_ui::testing::pointer::moved(from - vec2(3.0 * step as f32, 0.0))]);
    }
    fixture.frame(vec![craft_ui::testing::pointer::release(from - vec2(3.0 * DRAG_STEPS as f32, 0.0))]);
    fixture.settle();
    let ends = ends.borrow().clone();
    (updates.get(), ends)
}

/// The moves of `brightness_disc_drag`.
const DRAG_STEPS: usize = 20;

/// A drag on a brightness disc whose host keeps 8-bit colours runs to its release: the host's colour, read back, is
/// what the drag made of the colour before, not an outside change that cancels it.
#[trial]
fn a_brightness_disc_drag_the_host_applies_in_8_bit_colours_runs_to_its_release() {
    let (updates, ends) = brightness_disc_drag(None, |_, now, moved| {
        let v = HsbModel.from_srgb(now).values;
        eight_bit(HsbModel.to_srgb(&ModelValues::new("hsb", &[moved.hue, v[1], moved.radial * 100.0]).values))
    });
    assert!(updates >= DRAG_STEPS / 2, "the drag updated as it moved: {updates}");
    assert_eq!(ends, [None], "the drag commits, never cancelled");
}

/// A host that keeps the channel the radius does not show from the colour at the press, against the drift of 8-bit
/// colours, puts the marker where the drag reported it: the drag runs to its release.
#[trial]
fn a_brightness_disc_drag_whose_host_keeps_the_saturation_from_the_press_runs_to_its_release() {
    let (updates, ends) = brightness_disc_drag(None, |start, _, moved| {
        let v = HsbModel.from_srgb(start).values;
        eight_bit(HsbModel.to_srgb(&ModelValues::new("hsb", &[moved.hue, v[1], moved.radial * 100.0]).values))
    });
    assert!(updates >= DRAG_STEPS / 2, "the drag updated as it moved: {updates}");
    assert_eq!(ends, [None], "the drag commits, never cancelled");
}

/// A host whose colour goes somewhere other than where the drag put the marker, partway through the drag, changed it
/// from outside: the drag is cancelled.
#[trial]
fn a_brightness_disc_drag_whose_host_takes_another_colour_is_cancelled() {
    let applied = Cell::new(0);
    let (_, ends) = brightness_disc_drag(None, move |_, now, moved| {
        applied.set(applied.get() + 1);
        if applied.get() < DRAG_STEPS / 2 {
            let v = HsbModel.from_srgb(now).values;
            eight_bit(HsbModel.to_srgb(&ModelValues::new("hsb", &[moved.hue, v[1], moved.radial * 100.0]).values))
        } else {
            Srgba::rgb(0.1, 0.8, 0.3)
        }
    });
    assert_eq!(ends, [Some(CancelReason::HostChanged)], "an outside change cancels the drag");
}

/// A colour as a host stores it whose model's round trip darkens every colour by a tenth (as a CMYK profile's can), to
/// 8 bits.
fn darkened(_: usize, color: Srgba) -> Srgba {
    eight_bit(Srgba::rgb(color.r * 0.9, color.g * 0.9, color.b * 0.9))
}

/// Tb-10 (3): a host that stores each colour in another model, whose round trip moves it further than an 8-bit one,
/// gives the disc that round trip: its drag runs to its release, an outside change still cancels it, and without the
/// round trip the moved colour reads as an outside change.
#[trial]
fn a_disc_drag_through_the_hosts_lossy_model_runs_to_its_release() {
    let through_model = |_: Srgba, now: Srgba, moved: &DiscMove| {
        let v = HsbModel.from_srgb(now).values;
        darkened(0, HsbModel.to_srgb(&ModelValues::new("hsb", &[moved.hue, v[1], moved.radial * 100.0]).values))
    };
    let (updates, ends) = brightness_disc_drag(Some(darkened), through_model);
    assert!(updates >= DRAG_STEPS / 2, "the drag updated as it moved: {updates}");
    assert_eq!(ends, [None], "the drag commits, never cancelled");

    let (_, ends) = brightness_disc_drag(None, through_model);
    assert_eq!(ends, [Some(CancelReason::HostChanged)], "without the host's round trip the stored colour is another");

    let applied = Cell::new(0);
    let (_, ends) = brightness_disc_drag(Some(darkened), move |start, now, moved| {
        applied.set(applied.get() + 1);
        if applied.get() < DRAG_STEPS / 2 { through_model(start, now, moved) } else { Srgba::rgb(0.1, 0.8, 0.3) }
    });
    assert_eq!(ends, [Some(CancelReason::HostChanged)], "an outside change still cancels the drag");
}

/// Tb-8 (2): a marker of a linked harmony wheel dragged a quarter turn, a few points at a time, by a host that moves
/// every colour from the colours at the drag's start (`move_on_wheel`, linked) and keeps 8-bit colours, as the recolor
/// dialog does: the drag runs to its release and the other colour turns with it.
#[trial]
fn a_linked_harmony_drag_turns_the_other_colour_to_the_release() {
    use craft_color_ui::harmony::move_on_wheel;
    use craft_color_ui::ui::HarmonyWheel;
    let colours = Rc::new(RefCell::new(vec![Srgba::rgb(1.0, 0.0, 0.0), Srgba::rgb(0.0, 1.0, 1.0)]));
    let (shown, start, ends) = (colours.clone(), Rc::new(RefCell::new(None::<Vec<Srgba>>)), Rc::new(RefCell::new(Vec::new())));
    let (began, ended) = (start.clone(), ends.clone());
    let mut fixture = Fixture::new(move |ui| {
        let now = shown.borrow().clone();
        let out = HarmonyWheel::new("Color wheel", &now).selected_marker(Some(0)).base(Some(0)).show(ui);
        for (marker, event) in out.edits {
            match event {
                EditEvent::Begin { .. } => *began.borrow_mut() = Some(shown.borrow().clone()),
                EditEvent::Cancel { reason, .. } => ended.borrow_mut().push(Some(reason)),
                event => {
                    let Some(set) = event.value().copied() else { continue };
                    let mut moved = began.borrow().clone().unwrap_or_else(|| shown.borrow().clone());
                    let old = moved[marker].to_hsb();
                    let target = [set[0].unwrap_or(old.h), set[1].unwrap_or(old.s), set[2].unwrap_or(old.b)];
                    move_on_wheel(&mut moved, marker, target, true);
                    *shown.borrow_mut() = moved.into_iter().map(eight_bit).collect();
                    if matches!(event, EditEvent::Commit { .. }) {
                        ended.borrow_mut().push(None);
                    }
                }
            }
        }
    });
    fixture.room(vec2(320.0, 400.0));
    fixture.settle();
    let red = fixture.by_label(&Srgba::rgb(1.0, 0.0, 0.0).to_hex()).expect("the red marker").center();
    let cyan = fixture.by_label(&Srgba::rgb(0.0, 1.0, 1.0).to_hex()).expect("the cyan marker").center();
    let (centre, radius) = (red + (cyan - red) / 2.0, red.distance(cyan) / 2.0);
    let path: Vec<Pos2> = (1..=12).map(|k| centre + hue_direction(6.0 * k as f32) * radius * (1.0 - 0.25 * k as f32 / 12.0)).collect();
    fixture.frame(vec![craft_ui::testing::pointer::moved(red)]);
    fixture.frame(vec![craft_ui::testing::pointer::press(red)]);
    for at in &path {
        fixture.frame(vec![craft_ui::testing::pointer::moved(*at)]);
    }
    let turned = colours.borrow()[1].to_hsb().h;
    fixture.frame(vec![craft_ui::testing::pointer::release(path[11])]);
    fixture.settle();
    let gap = (turned - 180.0).rem_euclid(360.0).min((180.0 - turned).rem_euclid(360.0));
    assert!(gap > 10.0, "the other colour turned with the dragged one: {:?}", colours.borrow());
    assert_eq!(*ends.borrow(), [None], "the drag commits, never cancelled");
}
