//! The colour panels through the fixture: the Color panel, the Swatches panel, the Color Guide and the swatch
//! palette and table.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_behavior::{EditEvent, Selection};
use craft_color::ui::{ColorPanel, ColorPanelMode, ColorPanelState, PanelTarget, PickedColor, SwatchPalette, SwatchTable};
use craft_theme::Srgba;
use craft_ui::contract::Role;
use craft_ui::contract::tooltip::DELAY;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::requests::ColorRequest;
use craft_ui::testing::pointer::moved;
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
        Self::in_room(mode, COLOR_ROOM)
    }

    fn in_room(mode: ColorPanelMode, room: Vec2) -> Self {
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
        fixture.room(room);
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

/// The cubes fill the panel's room as Photoshop's field fills its dock group: the field takes the height the readout
/// leaves and the width the wells and the strip leave, so in a short, narrow group the strip and the readout stay in
/// view, and a taller group gives a taller field. The readout is one line in a 280 pt group and folds its R, G and B
/// fields under the hex field in a narrower one, never cut at the edge.
#[trial]
fn the_cubes_take_the_room_the_readout_and_the_strip_leave() {
    let mut heights = Vec::new();
    for room in [Vec2::new(280.0, 110.0), Vec2::new(280.0, 400.0)] {
        for (mode, name, strip) in
            [(ColorPanelMode::HueCube, "Saturation and brightness", "Hue"), (ColorPanelMode::BrightnessCube, "Hue and saturation", "Brightness")]
        {
            let rig = PanelRig::in_room(mode, room);
            let area = rig.fixture.by_label(name).expect("the field");
            let strip = rig.fixture.by_label(strip).expect("the strip");
            let readout = rig.fixture.by_label("Blue").expect("the readout's last field");
            let case = format!("{mode:?} in {room:?}");
            assert!(
                area.rect.right() <= strip.rect.left(),
                "{case}: the strip beside the field ({:?}, {:?}, readout {:?})",
                area.rect,
                strip.rect,
                readout.rect
            );
            assert!(strip.rect.right() <= room.x + 0.5, "{case}: the strip in view ({:?})", strip.rect);
            assert!(
                room.x - strip.rect.right() < 24.0,
                "{case}: the field takes the width the wells and the strip leave ({:?}, {:?})",
                area.rect,
                strip.rect
            );
            assert!(area.rect.bottom() <= readout.rect.top(), "{case}: the readout under the field");
            assert!(readout.rect.bottom() <= room.y + 0.5, "{case}: the readout in view ({:?})", readout.rect);
            assert!((area.rect.height() - strip.rect.height()).abs() < 1.0, "{case}: the strip as tall as the field");
            let hex = rig.fixture.by_label("Hex color").expect("the hex field");
            assert!(readout.rect.right() <= room.x + 0.5, "{case}: the readout fits the width ({:?})", readout.rect);
            assert!(hex.rect.y_range().contains(readout.rect.center().y), "{case}: the readout on one line ({:?}, {:?})", hex.rect, readout.rect);
            heights.push(area.rect.height());
        }
    }
    assert!(heights[2] > heights[0] + 200.0 && heights[3] > heights[1] + 200.0, "the field grows with the room: {heights:?}");
    let narrow = Vec2::new(170.0, 400.0);
    let rig = PanelRig::in_room(ColorPanelMode::HueCube, narrow);
    let hex = rig.fixture.by_label("Hex color").expect("the hex field");
    let blue = rig.fixture.by_label("Blue").expect("the readout's last field");
    assert!(blue.rect.top() >= hex.rect.bottom(), "the readout folds in {narrow:?} ({:?}, {:?})", hex.rect, blue.rect);
    assert!(blue.rect.right() <= narrow.x + 0.5, "the folded readout fits the width ({:?})", blue.rect);
}

/// The slider modes show each channel on one line: its slider and its number field side by side, every slider starting
/// at one x past the channels' letters. The rows come first and stay in view in a short group; the ramp under them
/// takes the room they leave.
#[trial]
fn the_slider_modes_show_a_channel_on_one_line() {
    let (rgb, cmyk, hsb) = ((ColorPanelMode::RgbSliders, 3), (ColorPanelMode::CmykSliders, 4), (ColorPanelMode::HsbSliders, 3));
    // A Pro Color group's default height holds three rows and a band of the ramp.
    let short = Vec2::new(280.0, 96.0);
    for (room, (mode, channels)) in [(COLOR_ROOM, rgb), (COLOR_ROOM, cmyk), (COLOR_ROOM, hsb), (short, rgb), (short, hsb)] {
        let case = format!("{mode:?} in {room:?}");
        let rig = PanelRig::in_room(mode, room);
        let sliders = rig.fixture.by_role(Role::SLIDER);
        let fields = rig.fixture.by_role(Role::SPIN_BUTTON);
        assert_eq!((sliders.len(), fields.len()), (channels, channels), "{case}");
        for (slider, field) in sliders.iter().zip(&fields) {
            let line = slider.rect.y_range();
            assert!(
                line.contains(field.rect.center().y) && slider.rect.right() <= field.rect.left(),
                "{case}: {slider:?} then {field:?} on one line"
            );
            assert!((slider.rect.left() - sliders[0].rect.left()).abs() < 0.5, "{case}: the sliders start at one x ({slider:?}, {:?})", sliders[0]);
            assert!(field.rect.bottom() <= room.y + 0.5, "{case}: every row in view ({field:?})");
        }
        let last = fields.iter().map(|field| field.rect.bottom()).fold(0.0, f32::max);
        let ramp = rig.fixture.by_label("Color Ramp").expect("the ramp").rect;
        assert!(ramp.top() >= last && ramp.height() > 0.0, "{case}: the ramp under the rows ({last}, {ramp:?})");
        assert!(ramp.bottom() <= room.y + 0.5, "{case}: the ramp in view ({ramp:?})");
        if room == COLOR_ROOM {
            assert!(ramp.bottom() > room.y - 40.0, "{case}: the ramp takes the room the rows leave ({ramp:?})");
        }
    }
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

/// `count` entries of a colour table, each named by its index.
fn table_entries(count: usize) -> Vec<(Label, Srgba)> {
    (0..count).map(|i| (Label::from(format!("Index {i}")), Srgba::rgb(i as f32 / count as f32, 0.5, 0.25))).collect()
}

/// The component runs of a settled pass of a table of `count` entries.
fn settled_table_runs(count: usize) -> u32 {
    let entries = table_entries(count);
    let mut fixture = Fixture::new(move |ui| {
        let _ = SwatchTable::new("Color Table", &entries).per_row(8).show_all(ui);
    });
    fixture.room(Vec2::new(400.0, 400.0)).settle().idle(1);
    craft_ui::testing::last_pass_runs(fixture.ctx()).iter().map(|(_, runs)| runs).sum()
}

/// A table's cells are members of its run: a settled pass runs as many components for 64 cells as for 4.
#[trial]
fn a_swatch_table_runs_as_many_components_for_many_cells_as_for_few() {
    let (few, many) = (settled_table_runs(4), settled_table_runs(64));
    assert!(few > 0, "the table runs");
    assert_eq!(few, many, "the runs grow with the cells: {few} for 4 cells, {many} for 64");
}

/// A spanning table of 4 cells to a row: the cells of a row cover the table's width edge to edge, each next to the one
/// before without overlap, and the second row starts under the first.
#[trial]
fn a_spanning_swatch_table_divides_its_width_among_a_rows_cells() {
    let entries = table_entries(8);
    let mut fixture = Fixture::new(move |ui| {
        let _ = SwatchTable::new("Color Table", &entries).per_row(4).span(true).show_all(ui);
    });
    fixture.room(Vec2::new(400.0, 200.0)).settle();
    let table = fixture.by_role(Role::LIST_BOX).first().expect("the table").rect;
    let cell = |i: usize| fixture.by_label(&format!("Index {i}")).unwrap_or_else(|| panic!("cell {i}")).rect;
    let row: Vec<_> = (0..4).map(cell).collect();
    assert!((row[0].left() - table.left()).abs() < 0.5, "the first cell starts at the table's edge: {row:?} in {table:?}");
    assert!((row[3].right() - table.right()).abs() < 0.5, "the last cell ends at the table's edge: {row:?} in {table:?}");
    for pair in row.windows(2) {
        assert!(pair[1].left() >= pair[0].right() - 0.5 && pair[1].left() <= pair[0].right() + 0.5, "cells touch without overlap: {pair:?}");
    }
    assert!(row[0].width() > row[0].height(), "a spanning cell is wider than the square cell: {:?}", row[0]);
    assert!(cell(4).top() >= row[0].bottom() - 0.5 && (cell(4).left() - row[0].left()).abs() < 0.5, "the fifth cell starts the second row");
}

/// A table's cell is a list box option named by its entry with the colour as its value; a click selects it, and the
/// pointer's rest shows its name.
#[trial]
fn a_swatch_table_cell_reports_its_colour_selects_on_click_and_shows_its_name() {
    let entries: Vec<(Label, Srgba)> = [("Red", RED), ("Blue", BLUE)].into_iter().map(|(name, color)| (Label::from(name), color)).collect();
    let chosen = Rc::new(RefCell::new(Selection::<usize>::default()));
    let held = Rc::clone(&chosen);
    let mut fixture = Fixture::new(move |ui| {
        let mut selection = held.borrow().clone();
        let _ = SwatchTable::new("Color Table", &entries).tips(true).selection(Bind::bound(&mut selection)).show_all(ui);
        *held.borrow_mut() = selection;
    });
    fixture.room(Vec2::new(400.0, 200.0)).settle();
    let options = fixture.by_role(Role::LIST_BOX_OPTION);
    assert_eq!(options.len(), 2, "one option per entry");
    let blue = fixture.by_label("Blue").expect("the Blue cell is named by its entry");
    assert_eq!(fixture.tree().value_of(blue.node), Some(BLUE.to_hex().as_str()), "the cell's value is its colour");
    fixture.click(&blue).settle();
    assert_eq!(chosen.borrow().items, [1], "a click selects the cell");
    let blue = fixture.by_label("Blue").expect("blue");
    assert_eq!(fixture.tree().selected_of(blue.node), Some(true), "the selected cell reports it");
    assert!(!fixture.texts().iter().any(|run| run.text == "Red"), "no tooltip before the rest");
    let red = fixture.by_label("Red").expect("red");
    fixture.frame(vec![moved(red.center())]);
    fixture.advance(DELAY);
    fixture.settle();
    assert!(fixture.texts().iter().any(|run| run.text == "Red"), "the pointer's rest on a cell shows its name");
}
