//! The colour panels through the fixture: the Color panel, the Swatches panel, the Color Guide and the swatch
//! palette and table.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_behavior::EditEvent;
use craft_color::ui::{ColorPanel, ColorPanelMode, ColorPanelState, PanelTarget, PickedColor, SwatchPalette};
use craft_theme::Srgba;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::requests::ColorRequest;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Vec2};

const RED: Srgba = Srgba::rgb(1.0, 0.0, 0.0);
const BLUE: Srgba = Srgba::rgb(0.0, 0.0, 1.0);

/// The rooms the panels are shown in.
const COLOR_ROOM: Vec2 = Vec2::new(320.0, 600.0);
const PALETTE_WIDE: Vec2 = Vec2::new(600.0, 300.0);

type Edits = Rc<RefCell<Vec<(PanelTarget, EditEvent<PickedColor>)>>>;

/// The Color panel on a state and two colours the test can change, collecting what it reports.
struct PanelRig {
    fixture: Fixture,
    colors: Rc<Cell<[Srgba; 2]>>,
    edits: Edits,
}

impl PanelRig {
    fn new(mode: ColorPanelMode) -> Self {
        let mut start = ColorPanelState::default();
        start.mode = mode;
        let state = Rc::new(RefCell::new(start));
        let colors = Rc::new(Cell::new([RED, BLUE]));
        type Shared = (Edits, Rc<RefCell<Vec<ColorRequest>>>, Rc<RefCell<Vec<PanelTarget>>>);
        let (edits, opened, added): Shared = Default::default();
        let (s, c, e, o, a) = (state, colors.clone(), edits.clone(), opened, added);
        let mut fixture = Fixture::new(move |ui| {
            let mut current = *s.borrow();
            let shown = ColorPanel::new(Bind::bound(&mut current), c.get()).show(ui);
            *s.borrow_mut() = current;
            e.borrow_mut().extend(shown.edits);
            o.borrow_mut().extend(shown.pick_color);
            a.borrow_mut().extend(shown.add_to_swatches);
        });
        fixture.room(COLOR_ROOM);
        fixture.settle();
        PanelRig { fixture, colors, edits }
    }

    fn set_colors(&mut self, colors: [Srgba; 2]) {
        self.colors.set(colors);
        self.fixture.settle();
    }
}

/// A saturated colour dragged to the cube's bottom right (black) keeps the remembered hue and saturation: the knob stays
/// in the corner and the strip keeps its hue.
#[trial]
fn dragging_to_the_bottom_right_keeps_the_knob_there() {
    let mut rig = PanelRig::new(ColorPanelMode::HueCube);
    rig.set_colors([Srgba::rgb(0.2, 0.4, 0.6), BLUE]);
    let area = rig.fixture.by_label("Saturation and brightness").expect("the area");
    rig.fixture.drag(&area, area.rect.size());
    let picked = rig.edits.borrow().iter().rev().find_map(|(_, e)| e.value().map(|v| v.color)).expect("a picked colour");
    assert_eq!(picked.to_rgba(), Srgba::rgb(0.0, 0.0, 0.0).to_rgba(), "the corner is black");

    rig.set_colors([picked, BLUE]);
    let area = rig.fixture.focused().expect("the dragged area keeps the focus");
    let value = rig.fixture.tree().value(area).expect("the area reports its value");
    assert!(value.contains("Saturation 100") && value.contains("Brightness 0"), "the knob stays at the corner: {value}");
}

const SWATCHES: [(&str, [f32; 3]); 6] = [
    ("Red", [0.90, 0.28, 0.30]),
    ("Orange", [0.97, 0.42, 0.08]),
    ("Green", [0.19, 0.64, 0.42]),
    ("Teal", [0.07, 0.65, 0.58]),
    ("Blue", [0.24, 0.39, 0.87]),
    ("Violet", [0.56, 0.31, 0.78]),
];

/// A palette over `SWATCHES` choosing the colour the test sets.
struct PaletteRig {
    fixture: Fixture,
    events: Rc<Cell<usize>>,
}

impl PaletteRig {
    fn new(start: Srgba, room: Vec2) -> Self {
        let swatches: Vec<(Label, Srgba)> = SWATCHES.iter().map(|(name, [r, g, b])| (Label::from(*name), Srgba::rgb(*r, *g, *b))).collect();
        let events = Rc::new(Cell::new(0));
        let (c, e) = (Rc::new(Cell::new(start)), events.clone());
        let mut fixture = Fixture::new(move |ui| {
            let mut current = c.get();
            let shown = SwatchPalette::new("Swatches", Bind::bound(&mut current), &swatches).show(ui);
            // A second Tab stop for the Tab that leaves the palette to land on.
            let _ = craft_ui::buttons::Button::new("After").show(ui);
            c.set(current);
            e.set(e.get() + usize::from(shown.edit.is_some()));
        });
        fixture.room(room);
        fixture.settle();
        PaletteRig { fixture, events }
    }

    fn focused(&self) -> Option<String> {
        let tree = self.fixture.tree();
        tree.name_of(craft_ui::testing::access::focused_key(tree)).map(str::to_owned)
    }
}

/// Up and Down walk the wrapped rows of the palette to the swatch in the same column.
#[trial]
fn the_palette_arrows_walk_the_wrapped_grid() {
    // Three to a row: Red Orange Green / Teal Blue Violet. The room is three cells and the gaps the wide rig lays out.
    let wide = PaletteRig::new(Srgba::rgb(0.19, 0.64, 0.42), PALETTE_WIDE);
    let (first, third) = (wide.fixture.by_label("Red").expect("red").rect, wide.fixture.by_label("Green").expect("green").rect);
    let narrow = Vec2::new(third.max.x + first.min.x, PALETTE_WIDE.y);
    let mut rig = PaletteRig::new(Srgba::rgb(0.19, 0.64, 0.42), narrow);
    let (red, teal) = (rig.fixture.by_label("Red").expect("red").rect, rig.fixture.by_label("Teal").expect("teal").rect);
    assert!(teal.min.y > red.min.y, "the palette wrapped");
    rig.fixture.key(Key::Tab, Modifiers::NONE);
    let steps = [
        (Key::ArrowDown, "Violet"),
        (Key::ArrowUp, "Green"),
        (Key::ArrowUp, "Green"),
        (Key::ArrowRight, "Teal"),
        (Key::ArrowLeft, "Green"),
        (Key::ArrowLeft, "Orange"),
        (Key::ArrowDown, "Blue"),
        (Key::ArrowRight, "Violet"),
        (Key::ArrowRight, "Violet"),
    ];
    for (key, expected) in steps {
        rig.fixture.key(key, Modifiers::NONE);
        assert_eq!(rig.focused().as_deref(), Some(expected), "{key:?}");
    }
    assert_eq!(rig.events.get(), 0);
}
