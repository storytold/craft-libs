//! The colour area, strip, ramp, disc and wheel: the colour maths and the edits they report.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_color::AreaMode;
use craft_color::ui::{
    ColorArea, ColorDisc, ColorRamp, DiscMove, RampKind, RampPick, WheelGeometry, hue_at, triangle_corners, triangle_point, triangle_sv,
};
use craft_theme::{ColorModel, HsbModel, ModelValues, Srgba};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Fixture, Story, StoryData, StoryOutput, trial};
use egui::{Key, Modifiers, Pos2, Rect, Ui, pos2, vec2};

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
    let values = data.value("values", HsbModel.from_srgb(Srgba::rgb(0.4, 0.6, 0.8)));
    ColorArea::new("Saturation and brightness", AreaMode::Hue, Bind::bound(values)).quantize(6).show(ui).into()
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
