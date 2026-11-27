//! The colour panels through the fixture: the Color panel, the Swatches panel, the Color Guide and the swatch
//! palette and table.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_behavior::{EditEvent, Mixed, Selection};
use craft_color_ui::palette::{SwatchColor, SwatchGroup};
use craft_color_ui::ui::{
    ColorPanel, ColorPanelLayout, ColorPanelMode, ColorPanelShown, ColorPanelState, HarmonyPanel, HarmonyPanelState, HarmonyPicked, HarmonyWheel,
    PanelTarget, PickedColor, SwatchEdit, SwatchKey, SwatchLibrary, SwatchPalette, SwatchesCommand, SwatchesMenu, SwatchesPanel, SwatchesState,
    SwatchesWords, swatch_rows,
};
use craft_host::target::Os;
use craft_theme::{ColorModel as _, HsbModel, Srgba};
use craft_ui::buttons::Button;
use craft_ui::choice::SwatchTable;
use craft_ui::contract::Role;
use craft_ui::contract::tooltip::DELAY;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::runtime::env::{Label, Scale};
use craft_ui::runtime::requests::ColorRequest;
use craft_ui::testing::focus::AccessActionKind;
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
        let start = ColorPanelState { mode, ..Default::default() };
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
/// takes the room they leave. In a group shorter than the rows and the band the body scrolls.
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
            assert!((slider.rect.right() - sliders[0].rect.right()).abs() < 0.5, "{case}: the sliders end at one x ({slider:?}, {:?})", sliders[0]);
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
    // Four rows and the band do not fit the short group: the body scrolls, and the K row comes into view.
    let mut rig = PanelRig::in_room(ColorPanelMode::CmykSliders, short);
    let k = |rig: &PanelRig| rig.fixture.by_role(Role::SPIN_BUTTON).last().expect("the K field").rect;
    assert!(k(&rig).bottom() > short.y + 0.5, "CMYK in {short:?}: the K row starts below the view ({:?})", k(&rig));
    rig.fixture.frame(vec![moved(egui::pos2(short.x / 2.0, short.y / 2.0))]);
    rig.fixture.frame(vec![craft_ui::testing::pointer::wheel(-200.0)]).settle();
    assert!(k(&rig).bottom() <= short.y + 0.5 && k(&rig).top() >= 0.0, "CMYK in {short:?}: the K row scrolls into view ({:?})", k(&rig));
}

/// A step of one channel leaves the others as they were: the fields round what they show, the edit does not.
#[trial]
fn a_channel_step_keeps_the_other_channels_unrounded() {
    let start = Srgba::rgb(0.2, 0.5, 0.7);
    let mut rig = PanelRig::new(ColorPanelMode::HsbSliders);
    rig.set_colors([start, BLUE]);
    let hue = rig.fixture.by_role(Role::SLIDER).into_iter().next().expect("the hue slider");
    rig.fixture.access(AccessActionKind::Focus, &hue).settle();
    rig.fixture.key(Key::ArrowRight, Modifiers::NONE).settle();
    let edits = rig.edits.borrow();
    let edited = edits.iter().rev().find_map(|(_, edit)| edit.value()).expect("the step's edit").color;
    let [before, after] = [start, edited].map(|color| HsbModel.from_srgb(color).values);
    assert_ne!(before[0], after[0], "the hue moved");
    for channel in [1, 2] {
        assert!((before[channel] - after[channel]).abs() < 0.01, "channel {channel} kept ({before:?}, {after:?})");
    }
}

/// The rect of the FillStroke spectrum in Web Color for a foreground of `color`.
fn spectrum_rect(color: Srgba) -> egui::Rect {
    let mut fixture = Fixture::new(move |ui| {
        let mut state = ColorPanelState { mode: ColorPanelMode::WebColorSliders, ..Default::default() };
        let _ = ColorPanel::new(Bind::bound(&mut state), [color, BLUE]).layout(ColorPanelLayout::FillStroke).show(ui);
    });
    fixture.room(COLOR_ROOM).settle();
    for tip in ["Out of gamut for printing: click for the nearest printable color", "Not a web color: click for the nearest web color"] {
        assert_eq!(fixture.by_label(tip).is_some(), color != Srgba::rgb(0.0, 0.0, 0.0), "{tip} for {color:?}");
    }
    fixture.by_label("Color Ramp").expect("the spectrum").rect
}

/// The warnings of the FillStroke layout keep their room: the spectrum is where it was whether the colour has no
/// warning (black) or both (a printable-out, non-web teal), so a drag across white and black does not move it.
#[trial]
fn a_fill_stroke_panels_warnings_keep_their_room_so_the_spectrum_holds_still() {
    let (clean, warned) = (spectrum_rect(Srgba::rgb(0.0, 0.0, 0.0)), spectrum_rect(Srgba::rgb(0.07, 0.85, 0.58)));
    assert_eq!(clean, warned, "the spectrum with no warning and with both");
}

/// A host's names for the FillStroke layout's swap and default buttons replace the panel's words, so a host can name
/// its keys there; without them the buttons keep the words.
#[trial]
fn a_fill_stroke_panel_names_its_buttons_as_the_host_asks() {
    let names = |given: bool| {
        let mut fixture = Fixture::new(move |ui| {
            let mut state = ColorPanelState::default();
            let panel = ColorPanel::new(Bind::bound(&mut state), [RED, BLUE]).layout(ColorPanelLayout::FillStroke);
            let panel = if given { panel.buttons("Swap (Shift+X)", "Default (D)") } else { panel };
            let _ = panel.show(ui);
        });
        fixture.room(COLOR_ROOM).settle();
        ["Swap (Shift+X)", "Default (D)", "Swap fill and stroke", "Default fill and stroke"].map(|name| fixture.by_label(name).is_some())
    };
    assert_eq!(names(true), [true, true, false, false], "the host's names");
    assert_eq!(names(false), [false, false, true, true], "the panel's words");
}

/// The spectrum's tooltip is the shown mode's name, and a Shift click on it moves the state to the next mode without
/// editing a paint.
#[trial]
fn the_fill_stroke_spectrums_tooltip_names_the_shown_mode_and_shift_click_cycles_the_mode() {
    let state = Rc::new(RefCell::new(ColorPanelState::default()));
    let edits: Edits = Default::default();
    let mode = Rc::new(Cell::new(ColorPanelMode::RgbSliders));
    let (held, seen, shown) = (Rc::clone(&state), Rc::clone(&edits), Rc::clone(&mode));
    let mut fixture = Fixture::new(move |ui| {
        let mut current = *held.borrow();
        let out = ColorPanel::new(Bind::bound(&mut current), [RED, BLUE])
            .layout(ColorPanelLayout::FillStroke)
            .shown(ColorPanelShown::Mode(shown.get()))
            .show(ui);
        *held.borrow_mut() = current;
        seen.borrow_mut().extend(out.edits);
    });
    fixture.room(COLOR_ROOM).settle();
    let tip_of = |fixture: &mut Fixture| {
        let ramp = fixture.by_label("Color Ramp").expect("the spectrum");
        fixture.frame(vec![moved(egui::pos2(1.0, 1.0))]);
        fixture.frame(vec![moved(ramp.center())]);
        fixture.advance(DELAY);
        fixture.settle();
        ["RGB Sliders", "HSB Sliders"].map(|name| fixture.texts().iter().any(|run| run.text == name))
    };
    assert_eq!(tip_of(&mut fixture), [true, false], "the RGB mode's name");
    mode.set(ColorPanelMode::HsbSliders);
    fixture.settle();
    assert_eq!(tip_of(&mut fixture), [false, true], "the HSB mode's name");

    let ramp = fixture.by_label("Color Ramp").expect("the spectrum");
    fixture.hold(Modifiers::SHIFT);
    fixture.click(&ramp).settle();
    fixture.hold(Modifiers::NONE).settle();
    assert_eq!(state.borrow().mode, ColorPanelMode::HsbSliders.fill_stroke_next(), "the next mode of the shown one");
    assert!(edits.borrow().is_empty(), "a Shift click picks nothing: {:?}", edits.borrow());
}

/// In a host sized by its content (a floating Color panel) the FillStroke spectrum is as tall as its end boxes stacked,
/// each at least as tall as it is wide, not a thin band; in a short group it keeps that height and the body scrolls.
#[trial]
fn a_fill_stroke_spectrum_keeps_its_end_boxes_square_in_a_content_sized_host() {
    for (room, content_sized) in [(COLOR_ROOM, true), (Vec2::new(280.0, 96.0), false)] {
        let mut fixture = Fixture::new(move |ui| {
            let mut state = ColorPanelState::default();
            let show = |ui: &mut egui::Ui, state: &mut ColorPanelState| {
                let _ = ColorPanel::new(Bind::bound(state), [RED, BLUE]).layout(ColorPanelLayout::FillStroke).show(ui);
            };
            if content_sized {
                let _ = Stack::new().show(ui, |ui, _| show(ui, &mut state));
            } else {
                show(ui, &mut state);
            }
        });
        fixture.room(room).settle();
        let bar = fixture.by_label("Color Ramp").expect("the spectrum").rect;
        let ends: Vec<_> = fixture
            .by_role(Role::BUTTON)
            .into_iter()
            .filter(|button| button.rect.left() >= bar.right() - 0.5 && button.rect.y_range().intersects(bar.y_range()))
            .collect();
        assert_eq!(ends.len(), 2, "{room:?}: the white and black boxes beside the bar ({bar:?})");
        for end in ends {
            assert!(end.rect.height() >= end.rect.width() - 0.5, "{room:?}: an end box at least square ({:?} beside {bar:?})", end.rect);
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

/// What a palette reported over the passes since the last `take`.
#[derive(Default)]
struct PaletteReports {
    edits: Vec<EditEvent<Srgba>>,
    requests: Vec<ColorRequest>,
    picked: Vec<Srgba>,
    swatches: Vec<usize>,
}

type Reports = Rc<RefCell<PaletteReports>>;

/// Collects what one pass of a palette reported.
fn collect(reports: &Reports, shown: craft_color_ui::ui::SwatchPaletteOutput) {
    let mut reports = reports.borrow_mut();
    reports.edits.extend(shown.edit);
    reports.requests.extend(shown.pick_color);
    reports.picked.extend(shown.picked);
    reports.swatches.extend(shown.picked_swatch);
}

fn palette_swatches() -> Vec<(Label, Srgba)> {
    SWATCHES.iter().map(|(name, [r, g, b])| (Label::from(*name), Srgba::rgb(*r, *g, *b))).collect()
}

/// The colour of `SWATCHES` named `name`.
fn swatch(name: &str) -> Srgba {
    SWATCHES.iter().find(|(n, _)| *n == name).map(|(_, [r, g, b])| Srgba::rgb(*r, *g, *b)).unwrap_or_else(|| panic!("no swatch {name}"))
}

const RECENT: [Srgba; 1] = [Srgba::rgb(0.40, 0.10, 0.50)];

/// Activating the Other… cell files a request keyed by the host's key at the bound colour and sets nothing; a pick in
/// the recent row is one `Set` of its colour; a click on the swatch of the bound colour reports `picked` and no edit.
#[trial]
fn the_other_cell_files_a_request_at_the_colour_and_a_recent_pick_sets_it() {
    let swatches = palette_swatches();
    let green = swatch("Green");
    let bound = Rc::new(Cell::new(green));
    let reports = Reports::default();
    let (c, r) = (bound.clone(), reports.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut current = c.get();
        let shown = SwatchPalette::new("Swatches", Bind::bound(&mut current), &swatches)
            .other("custom", "Other colour…")
            .recent("Recent colours", &RECENT)
            .show(ui);
        c.set(current);
        collect(&r, shown);
    });
    fixture.room(PALETTE_WIDE).settle();
    let other = fixture.by_label("Other colour…").expect("the Other… cell is named by its label");
    let palette = fixture.by_label("Swatches").expect("the palette's list box");
    assert!(palette.rect.contains_rect(other.rect), "the Other… cell is a cell of the palette's table: {other:?} in {palette:?}");

    fixture.click(&other).settle();
    let taken = std::mem::take(&mut *reports.borrow_mut());
    assert_eq!(taken.requests.len(), 1, "one request for one activation");
    assert_eq!(taken.requests[0].key, "custom", "the request carries the host's key");
    assert_eq!(taken.requests[0].color.map(Srgba::to_rgba), Some(green.to_rgba()), "the picker opens at the bound colour");
    assert!(taken.edits.is_empty() && taken.picked.is_empty(), "the Other… cell sets nothing");
    assert_eq!(bound.get().to_rgba(), green.to_rgba(), "the bound colour stays");

    let recent = fixture.by_label(&RECENT[0].to_hex()).expect("a recent colour is named by its hex");
    fixture.click(&recent).settle();
    let taken = std::mem::take(&mut *reports.borrow_mut());
    assert!(taken.requests.is_empty() && taken.swatches.is_empty(), "a recent pick is not a palette entry");
    assert_eq!(taken.edits.len(), 1, "one edit for one recent pick");
    assert!(
        matches!(&taken.edits[0], EditEvent::Set { before: Mixed::Same(before), after, .. }
            if before.to_rgba() == green.to_rgba() && after.to_rgba() == RECENT[0].to_rgba()),
        "a Set from the bound colour to the recent one: {:?}",
        taken.edits[0]
    );
    assert_eq!(bound.get().to_rgba(), RECENT[0].to_rgba(), "the recent pick is written");

    let violet = fixture.by_label("Violet").expect("violet");
    fixture.click(&violet).settle();
    let _ = std::mem::take(&mut *reports.borrow_mut());
    assert_eq!(bound.get().to_rgba(), swatch("Violet").to_rgba());
    // Past the double-click time, so the repeat is a single click of its own.
    fixture.advance(1.0);
    let violet = fixture.by_label("Violet").expect("violet");
    fixture.click(&violet).settle();
    let taken = std::mem::take(&mut *reports.borrow_mut());
    assert!(taken.edits.is_empty(), "a click on the bound colour changes nothing: {:?}", taken.edits);
    assert_eq!(taken.picked.iter().map(|c| c.to_rgba()).collect::<Vec<_>>(), [swatch("Violet").to_rgba()], "and still reports the pick");
}

#[trial]
fn a_palette_reports_the_actual_swatch_when_its_color_matches_the_bound_color() {
    let color = swatch("Green");
    let swatches = [(Label::from("First green"), color), (Label::from("Second green"), color), (Label::from("Third green"), color)];
    let selected = Rc::new(Cell::new(2));
    let preferred = Rc::clone(&selected);
    let reports = Reports::default();
    let out = reports.clone();
    let mut fixture = Fixture::new(move |ui| {
        collect(&out, SwatchPalette::new("Swatches", Bind::given(color), &swatches).selected_swatch(Some(preferred.get())).show(ui));
        let _ = Button::new("Next control").show(ui);
    });
    fixture.room(PALETTE_WIDE).settle();
    let third = fixture.by_label("Third green").expect("the linked entry");
    assert_eq!(fixture.tree().selected_of(third.node), Some(true), "the linked entry stays selected");
    let second = fixture.by_label("Second green").expect("the second entry");
    fixture.hold(Modifiers::SHIFT).click(&second).hold(Modifiers::NONE).settle();
    let click = std::mem::take(&mut *reports.borrow_mut());
    assert_eq!(click.swatches, [1]);
    assert_eq!(click.picked.iter().map(|picked| picked.to_rgba()).collect::<Vec<_>>(), [color.to_rgba()]);
    assert!(click.edits.is_empty(), "the color binding has not changed");

    fixture.key(Key::ArrowRight, Modifiers::NONE).key(Key::Space, Modifiers::NONE).settle();
    let keys = std::mem::take(&mut *reports.borrow_mut());
    assert_eq!(keys.swatches, [2], "Space picks the already selected swatch");
    assert!(keys.edits.is_empty());

    let focused = |fixture: &Fixture| {
        let tree = fixture.tree();
        tree.name_of(craft_ui::testing::access::focused_key(tree)).map(str::to_owned)
    };
    fixture.key(Key::Tab, Modifiers::NONE).settle();
    assert_eq!(focused(&fixture).as_deref(), Some("Next control"));
    selected.set(0);
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::SHIFT).settle();
    assert_eq!(focused(&fixture).as_deref(), Some("First green"));
    fixture.key(Key::Space, Modifiers::NONE).settle();
    assert_eq!(reports.borrow().swatches, [0], "Tab returns to the host's current swatch");
}

/// The FillStroke layout shows its Recent Colors caption while the host's list is empty, as it does with colors in
/// it; a host that gives no recent colors gets no row.
#[trial]
fn a_fill_stroke_panel_shows_its_recent_colors_row_while_the_list_is_empty() {
    let caption = |recent: Option<&'static [Srgba]>| {
        let mut fixture = Fixture::new(move |ui| {
            let mut state = ColorPanelState::default();
            let panel = ColorPanel::new(Bind::bound(&mut state), [RED, BLUE]).layout(ColorPanelLayout::FillStroke);
            let _ = match recent {
                Some(colors) => panel.recent(colors),
                None => panel,
            }
            .show(ui);
        });
        fixture.room(COLOR_ROOM).settle();
        fixture.texts().iter().any(|run| run.text == "Recent Colors")
    };
    assert!(caption(Some(&[])), "an empty list shows the row's caption");
    assert!(caption(Some(&RECENT)), "a list with a color shows it");
    assert!(!caption(None), "a host with no recent colors shows no row");
}

/// A palette over a colour that may be none, bound to no colour: no swatch is selected and Tab enters on the first;
/// a click writes `Same(Some(colour))` and reports one `Set` from `Mixed`.
#[trial]
fn an_optional_palette_bound_to_no_colour_selects_nothing_and_a_pick_writes_a_colour() {
    let swatches = palette_swatches();
    let bound = Rc::new(Cell::new(Mixed::<Option<Srgba>>::Same(None)));
    let reports = Reports::default();
    let (c, r) = (bound.clone(), reports.clone());
    let mut fixture = Fixture::new(move |ui| {
        let mut current = c.get();
        let shown = SwatchPalette::optional("Swatches", Bind::bound_mixed(&mut current), &swatches).show(ui);
        let _ = craft_ui::buttons::Button::new("After").show(ui);
        c.set(current);
        collect(&r, shown);
    });
    fixture.room(PALETTE_WIDE).settle();
    for (name, _) in SWATCHES {
        let cell = fixture.by_label(name).unwrap_or_else(|| panic!("the {name} cell"));
        assert_ne!(fixture.tree().selected_of(cell.node), Some(true), "no colour selects no swatch, {name} is selected");
    }
    fixture.key(Key::Tab, Modifiers::NONE);
    let tree = fixture.tree();
    assert_eq!(tree.name_of(craft_ui::testing::access::focused_key(tree)), Some("Red"), "Tab enters on the first swatch");

    let blue = fixture.by_label("Blue").expect("blue");
    fixture.click(&blue).settle();
    let written = bound.get();
    assert!(matches!(written, Mixed::Same(Some(color)) if color.to_rgba() == swatch("Blue").to_rgba()), "the pick is written: {written:?}");
    let taken = std::mem::take(&mut *reports.borrow_mut());
    assert_eq!(taken.edits.len(), 1, "one edit for one pick");
    assert!(
        matches!(&taken.edits[0], EditEvent::Set { before: Mixed::Mixed, after, .. } if after.to_rgba() == swatch("Blue").to_rgba()),
        "a Set from no colour, reported as from Mixed: {:?}",
        taken.edits[0]
    );
    let blue = fixture.by_label("Blue").expect("blue");
    assert_eq!(fixture.tree().selected_of(blue.node), Some(true), "the written colour's swatch is selected");
}

/// The Swatches panel over two groups, collecting the edits it reports and never applying them.
fn swatches_rig(groups: Vec<SwatchGroup>, state: SwatchesState) -> (Fixture, Rc<RefCell<Vec<SwatchEdit>>>) {
    let edits = Rc::new(RefCell::new(Vec::new()));
    let state = Rc::new(RefCell::new(state));
    let (seen, held) = (Rc::clone(&edits), state);
    let mut fixture = Fixture::new(move |ui| {
        let mut current = held.borrow().clone();
        let out = SwatchesPanel::new(&groups, Bind::bound(&mut current), RED).show(ui);
        *held.borrow_mut() = current;
        seen.borrow_mut().extend(out.edits);
    });
    fixture.room(Vec2::new(320.0, 400.0)).settle();
    (fixture, edits)
}

#[trial]
fn default_swatches_fit_the_ten_grays_in_a_narrow_dock_and_still_zoom() {
    let grays = SwatchLibrary::default().groups.into_iter().find(|group| group.name == "Grays").expect("the factory Grays group");
    let groups = vec![grays];
    let names: Vec<_> = swatch_rows(&groups, &SwatchesWords::default()).into_iter().filter_map(|row| row.accessible_name).collect();
    assert_eq!(names.len(), 10);

    let (mut fixture, _) = swatches_rig(groups.clone(), SwatchesState::default());
    fixture.os(Os::Windows).scale(Scale::new(1.25)).room(Vec2::new(220.0, 150.0)).settle();
    let visible_grays =
        |fixture: &Fixture| names.iter().filter_map(|name| fixture.by_label(name).and_then(|tile| fixture.visible(&tile))).collect::<Vec<_>>();
    let defaults = visible_grays(&fixture);
    assert_eq!(defaults.len(), 10, "all ten Grays are visible in the default dock");
    let first = defaults[0].rect;
    assert!(defaults.iter().all(|tile| tile.rect.top() == first.top()), "all ten Grays fit on one row");

    let larger = fixture.by_label("Larger Thumbnails").expect("the larger thumbnail button");
    fixture.click(&larger).settle();
    let after_click = visible_grays(&fixture)[0].rect.width();
    assert!(after_click > first.width(), "the zoom button enlarges the tiles");

    let slider = fixture.by_role(Role::SLIDER).into_iter().next().expect("the thumbnail slider");
    fixture.access(AccessActionKind::Focus, &slider).settle();
    fixture.key(Key::ArrowRight, Modifiers::NONE).settle();
    assert!(visible_grays(&fixture)[0].rect.width() > after_click, "the slider enlarges the tiles with the keyboard");
    fixture.click(&slider).click(&slider).settle();
    assert_eq!(visible_grays(&fixture)[0].rect.width(), first.width(), "double-click resets to the compact default");

    let mut larger_state = SwatchesState::default();
    larger_state.browser.zoom = 1.75;
    let (mut fixture, _) = swatches_rig(groups, larger_state);
    fixture.room(Vec2::new(220.0, 310.0)).settle();
    let slider = fixture.by_role(Role::SLIDER).into_iter().next().expect("the reopened thumbnail slider");
    let shown = fixture.tree().numeric_of(slider.node).expect("the slider's accessible value and range");
    assert_eq!(shown.value, Some(1.75), "a previously chosen larger zoom is still displayed");
    assert_eq!(*shown.range.end(), 5.0, "the larger end of the zoom range stays available");
}

/// Through the panel, a selected swatch deleted reports one `Delete`, and a swatch dragged onto another group's header
/// reports one `MoveTo` the host can run; New Swatch from the menu reports an `Add` in the selected swatch's group under
/// the next free numbered name. The slots of multi-swatch moves are checked in library.rs.
#[trial]
fn swatches_panel_reports_each_edit_and_leaves_the_library() {
    let red = SwatchColor::Rgb([1.0, 0.0, 0.0]);
    let blue = SwatchColor::Rgb([0.0, 0.0, 1.0]);
    let entry = |name: &str, color: SwatchColor| craft_color_ui::palette::SwatchEntry::new(name.to_owned(), color);
    let groups = vec![
        SwatchGroup { name: "Basics".to_owned(), entries: vec![entry("Red", red), entry("Color Swatch 1", blue)] },
        SwatchGroup { name: "Other".to_owned(), entries: vec![entry("Blue", blue)] },
    ];
    let (mut fixture, edits) = swatches_rig(groups.clone(), SwatchesState::default());
    let row = fixture.by_label("Red, #ff0000").expect("the Red swatch is named with its value");
    fixture.click(&row).settle();
    assert!(edits.borrow().is_empty(), "a click selects and uses, it edits nothing: {:?}", edits.borrow());
    fixture.key(Key::Delete, Modifiers::NONE).settle();
    assert_eq!(*edits.borrow(), [SwatchEdit::delete("Red".to_owned())], "one resolved edit, once");

    let (mut fixture, edits) = swatches_rig(groups.clone(), SwatchesState::default());
    let row = fixture.by_label("Red, #ff0000").expect("the Red swatch");
    let header = fixture.by_label("Other").expect("the Other group's header").center();
    fixture.drag(&row, header - row.center()).settle();
    let reported = edits.borrow().clone();
    assert_eq!(reported.len(), 1, "one edit for one dragged swatch: {reported:?}");
    assert!(matches!(&reported[0], SwatchEdit::MoveTo { swatch, group, .. } if swatch == "Red" && group == "Other"), "{reported:?}");
    let mut library = SwatchLibrary::new(groups.clone());
    assert!(library.apply(&reported[0]), "the host can run it");
    assert!(library.find("Red").is_some_and(|at| library.groups[at.group].name == "Other"), "Red is in Other");

    let mut state = SwatchesState::default();
    state.selection = Selection::single(SwatchKey::Swatch("Red".to_owned()));
    let made = SwatchesMenu::run_command(&groups, &mut state, SwatchesCommand::NewSwatch, RED, &Default::default());
    assert_eq!(
        made.edit,
        Some(SwatchEdit::add(Some("Basics".to_owned()), "Color Swatch 2".to_owned(), red)),
        "the next free number, in the selected swatch's group, with the foreground"
    );
    assert_eq!(state.rename, Some(SwatchKey::Swatch("Color Swatch 2".to_owned())), "the new swatch opens for renaming");
}

#[trial]
fn a_mixed_swatches_selection_rejects_reorder_and_deletes_each_entry_once() {
    use craft_ui::contract::SelectMode;

    let entry = |name: &str, color| craft_color_ui::palette::SwatchEntry::new(name.to_owned(), SwatchColor::Rgb(color));
    let groups = vec![
        SwatchGroup { name: "Basics".to_owned(), entries: vec![entry("Red", [1.0, 0.0, 0.0])] },
        SwatchGroup { name: "Other".to_owned(), entries: vec![entry("Blue", [0.0, 0.0, 1.0])] },
    ];
    let state = Rc::new(RefCell::new(SwatchesState::default()));
    let edits = Rc::new(RefCell::new(Vec::new()));
    let reorders = Rc::new(Cell::new(0));
    let (seen, moves, held) = (edits.clone(), reorders.clone(), state.clone());
    let mut fixture = Fixture::new(move |ui| {
        let out = SwatchesPanel::new(&groups, Bind::bound(&mut *held.borrow_mut()), RED).select(SelectMode::Multi).select_groups(true).show(ui);
        seen.borrow_mut().extend(out.edits);
        for event in out.events {
            if matches!(event, craft_ui::patterns::presets::PresetEvent::Reorder(_)) {
                moves.set(moves.get() + 1);
            }
        }
    });
    fixture.room(Vec2::new(320.0, 400.0)).settle();
    let group = |name: &str| {
        let tree = fixture.tree();
        tree.with_role(Role::LIST_BOX_OPTION)
            .into_iter()
            .find(|&key| tree.name_of(key).is_some_and(|label| label == name || label.starts_with(&format!("{name},"))))
            .and_then(|key| fixture.target(key))
            .unwrap_or_else(|| panic!("the {name} group"))
    };
    let basics = group("Basics");
    let red = fixture.by_label("Red, #ff0000").expect("the Red swatch");
    let blue = fixture.by_label("Blue, #0000ff").expect("the Blue swatch");
    let other = group("Other");
    fixture.click(&basics).hold(Modifiers::COMMAND).click(&red).click(&blue).hold(Modifiers::NONE).settle();
    let keys = [SwatchKey::Group("Basics".to_owned()), SwatchKey::Swatch("Red".to_owned()), SwatchKey::Swatch("Blue".to_owned())];
    assert_eq!(state.borrow().selection.items, keys, "the group and swatch keys are selected together");
    fixture.drag(&red, other.center() - red.center()).settle();
    assert!(edits.borrow().is_empty());
    assert_eq!(reorders.get(), 0, "the swatch source refuses a mixed reorder");
    fixture.key(Key::Delete, Modifiers::NONE).settle();
    let reported = edits.borrow();
    assert_eq!(reported.len(), 2, "the group delete already includes Red: {reported:?}");
    assert!(reported.contains(&SwatchEdit::DeleteGroup { group: "Basics".to_owned() }));
    assert!(reported.contains(&SwatchEdit::delete("Blue".to_owned())));
}

/// A group with an empty name holds the swatches outside every group: they show at the top with no header, so no
/// heading or disclosure button of the panel is left without a name, and a named group keeps its header.
#[trial]
fn swatches_outside_every_group_show_without_a_header() {
    let entry = |name: &str, rgb: [f32; 3]| craft_color_ui::palette::SwatchEntry::new(name.to_owned(), SwatchColor::Rgb(rgb));
    let groups = vec![
        SwatchGroup { name: String::new(), entries: vec![entry("Red", [1.0, 0.0, 0.0]), entry("Blue", [0.0, 0.0, 1.0])] },
        SwatchGroup { name: "Other".to_owned(), entries: vec![entry("Green", [0.0, 1.0, 0.0])] },
    ];
    let (fixture, _) = swatches_rig(groups, SwatchesState::default());
    let tree = fixture.tree();
    let unnamed: Vec<_> = tree
        .keys()
        .into_iter()
        .filter(|&key| matches!(tree.role_of(key), Some(Role::HEADING | Role::BUTTON)))
        .filter(|&key| tree.name_of(key).is_none_or(|name| name.is_empty() || name.starts_with(',')))
        .map(|key| (tree.role_of(key), tree.name_of(key), tree.bounds_of(key)))
        .collect();
    assert!(unnamed.is_empty(), "every heading and button is named, starting with a label: {unnamed:?}");
    assert!(fixture.by_label("Red, #ff0000").is_some(), "the ungrouped swatches show");
    let headings: Vec<_> = tree.with_role(Role::HEADING).into_iter().filter_map(|key| tree.name_of(key)).collect();
    assert!(headings.iter().any(|name| name.starts_with("Other")), "a named group keeps its header: {headings:?}");
}

/// Tb-8 (5): a click uses any swatch, as production applied every one: a colour swatch reports its colour, and a
/// gradient, pattern or None swatch reports where it is with no colour, for the host to apply its paint. The panel
/// reported only colour swatches.
#[trial]
fn a_click_uses_every_kind_of_swatch() {
    use craft_color_ui::palette::{SwatchEntry, SwatchPaint};
    let groups = vec![SwatchGroup {
        name: "Basics".to_owned(),
        entries: vec![
            SwatchEntry::new("Red".to_owned(), SwatchColor::Rgb([1.0, 0.0, 0.0])),
            SwatchEntry::with_paint("Fade", SwatchPaint::Gradient),
            SwatchEntry::with_paint("Dots", SwatchPaint::Pattern),
            SwatchEntry::with_paint("None", SwatchPaint::None),
        ],
    }];
    let used = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&used);
    let mut state = SwatchesState::default();
    let shown = groups.clone();
    let mut fixture = Fixture::new(move |ui| {
        let out = SwatchesPanel::new(&shown, Bind::bound(&mut state), RED).show(ui);
        seen.borrow_mut().extend(out.used.map(|(at, color, _)| (at.index, color.is_some())));
    });
    fixture.room(Vec2::new(320.0, 400.0)).settle();
    for name in ["Red, #ff0000", "Fade", "Dots", "None"] {
        let swatch = fixture.by_label(name).expect("the swatch");
        fixture.click(&swatch).settle();
    }
    assert_eq!(*used.borrow(), [(0, true), (1, false), (2, false), (3, false)], "each click uses its swatch, a colour only where it holds one");
}

/// `count` entries of a colour table, each named by its index.
/// A swatch dragged out of the panel carries the host's payload to the host's drop target (the app's paint, which a
/// gradient's stops take), and the panel's own drag still moves a swatch onto another group.
#[trial]
fn a_dragged_swatch_carries_the_hosts_payload_beside_the_panels_own_move() {
    use craft_ui::contract::{ListDrag, ShowContainer as _};
    use craft_ui::display::Text;
    use craft_ui::sections::DropAction;
    use egui::{Pos2, Rect, UiBuilder};
    #[derive(Clone, Debug, PartialEq)]
    struct Paint(String);
    const PANEL: Rect = Rect { min: Pos2::new(0.0, 0.0), max: Pos2::new(320.0, 300.0) };
    const TARGET: Rect = Rect { min: Pos2::new(0.0, 320.0), max: Pos2::new(320.0, 400.0) };
    let entry = |name: &str| craft_color_ui::palette::SwatchEntry::new(name.to_owned(), SwatchColor::Rgb([1.0, 0.0, 0.0]));
    let groups = vec![
        SwatchGroup { name: "Basics".to_owned(), entries: vec![entry("Red"), entry("Rose")] },
        SwatchGroup { name: "Other".to_owned(), entries: vec![entry("Blue")] },
    ];
    let edits = Rc::new(RefCell::new(Vec::new()));
    let dropped = Rc::new(RefCell::new(Vec::new()));
    let (seen, taken) = (Rc::clone(&edits), Rc::clone(&dropped));
    let mut state = SwatchesState::default();
    let mut fixture = Fixture::new(move |ui| {
        ui.scope_builder(UiBuilder::new().max_rect(PANEL), |ui| {
            let out = SwatchesPanel::new(&groups, Bind::bound(&mut state), RED)
                .drag_payload(|drag: &ListDrag<SwatchKey>| match &drag.grabbed {
                    SwatchKey::Swatch(name) => Some(Paint(name.clone())),
                    _ => None,
                })
                .show(ui);
            seen.borrow_mut().extend(out.edits);
        });
        ui.scope_builder(UiBuilder::new().max_rect(TARGET), |ui| {
            let out = DropAction::<Paint>::new(|_| true).show(ui, |ui, _| {
                Text::new("Gradient").show(ui);
            });
            taken.borrow_mut().extend(out.output.dropped);
        });
    });
    fixture.room(Vec2::new(320.0, 400.0)).settle();
    let red = fixture.by_label("Red, #ff0000").expect("the Red swatch").center();
    let other = fixture.by_label("Other").expect("the Other group's header").center();
    fixture.drag_at(red, other - red);
    fixture.settle();
    let reported = edits.borrow().clone();
    assert!(matches!(&reported[..], [SwatchEdit::MoveTo { swatch, group, .. }] if swatch == "Red" && group == "Other"), "{reported:?}");
    assert!(dropped.borrow().is_empty(), "the move within the panel reaches no host target");

    let rose = fixture.by_label("Rose, #ff0000").expect("the Rose swatch").center();
    let target = fixture.by_label("Gradient").expect("the host's drop target").center();
    fixture.drag_at(rose, target - rose);
    fixture.settle();
    assert_eq!(*dropped.borrow(), [Paint("Rose".to_owned())], "the host's target takes the swatch's paint");
    assert_eq!(edits.borrow().len(), 1, "a drop outside the panel edits nothing: {:?}", edits.borrow());
}

/// A panel shown for one document after another keeps a rename left open with its document: the other document's
/// swatch of the same name opens no field, and Enter there renames nothing.
#[trial]
fn a_rename_left_open_stays_with_its_document() {
    let entry = |name: &str| craft_color_ui::palette::SwatchEntry::new(name.to_owned(), SwatchColor::Rgb([1.0, 0.0, 0.0]));
    let groups = vec![SwatchGroup { name: "Basics".to_owned(), entries: vec![entry("Red"), entry("Rose")] }];
    let document = Rc::new(Cell::new(1_u32));
    let edits = Rc::new(RefCell::new(Vec::new()));
    let (shown, seen) = (Rc::clone(&document), Rc::clone(&edits));
    let mut state = SwatchesState::default();
    let mut fixture = Fixture::new(move |ui| {
        let out = SwatchesPanel::new(&groups, Bind::bound(&mut state), RED).document(shown.get()).show(ui);
        seen.borrow_mut().extend(out.edits);
    });
    fixture.room(Vec2::new(320.0, 400.0)).settle();
    let red = fixture.by_label("Red, #ff0000").expect("the Red swatch");
    fixture.click(&red).key(Key::F2, Modifiers::NONE).type_text("X").settle();
    let typed = |fixture: &Fixture, text: &str| {
        let tree = fixture.tree();
        tree.keys().into_iter().any(|key| tree.value_of(key) == Some(text))
    };
    assert!(typed(&fixture, "X"), "the rename field is open in the first document");
    document.set(2);
    fixture.settle();
    fixture.type_text("Y").key(Key::Enter, Modifiers::NONE).settle();
    assert!(!edits.borrow().iter().any(|edit| matches!(edit, SwatchEdit::Rename { .. })), "nothing is renamed: {:?}", edits.borrow());
}

#[trial]
fn opt_in_reports_only_rejected_swatch_and_group_renames() {
    use craft_ui::patterns::presets::{PresetEvent, PresetView};

    let entry = |name: &str| craft_color_ui::palette::SwatchEntry::new(name.to_owned(), SwatchColor::Rgb([1.0, 0.0, 0.0]));
    let groups = vec![
        SwatchGroup { name: "Basics".to_owned(), entries: vec![entry("Red"), entry("Rose")] },
        SwatchGroup { name: "Other".to_owned(), entries: Vec::new() },
    ];
    let red = SwatchKey::Swatch("Red".to_owned());
    let basics = SwatchKey::Group("Basics".to_owned());
    for (opt_in, key, name, accepted) in [
        (false, &red, "Rose", false),
        (false, &basics, "Other", false),
        (true, &red, "Rose", false),
        (true, &basics, "Other", false),
        (true, &red, " Red ", false),
        (true, &red, "Coral", true),
        (true, &basics, "Favorites", true),
    ] {
        let mut start = SwatchesState::default();
        start.browser.view = PresetView::List;
        let state = Rc::new(RefCell::new(start));
        let edits = Rc::new(RefCell::new(Vec::new()));
        let renames = Rc::new(RefCell::new(Vec::new()));
        let (held, seen_edits, seen_renames) = (Rc::clone(&state), Rc::clone(&edits), Rc::clone(&renames));
        let groups = groups.clone();
        let mut fixture = Fixture::new(move |ui| {
            let mut current = held.borrow_mut();
            let mut panel = SwatchesPanel::new(&groups, Bind::bound(&mut *current), RED);
            if opt_in {
                panel = panel.report_invalid_renames(true);
            }
            let out = panel.show(ui);
            seen_edits.borrow_mut().extend(out.edits);
            seen_renames.borrow_mut().extend(out.events.into_iter().filter_map(|event| match event {
                PresetEvent::Rename { key, text, .. } => Some((key, text)),
                _ => None,
            }));
        });
        fixture.room(Vec2::new(320.0, 400.0)).settle();
        let label = if key == &red { "Red" } else { "Basics" };
        let tree = fixture.tree();
        let row = tree
            .with_role(Role::TREE_ITEM)
            .into_iter()
            .find(|&node| tree.name_of(node).is_some_and(|actual| actual == label || actual.starts_with(&format!("{label},"))))
            .and_then(|node| fixture.target(node))
            .unwrap_or_else(|| {
                panic!("the {label} row is shown, names: {:?}", tree.keys().into_iter().filter_map(|key| tree.name_of(key)).collect::<Vec<_>>())
            });
        fixture.click(&row).key(Key::F2, Modifiers::NONE).type_text(name).settle();
        let tree = fixture.tree();
        assert!(tree.keys().into_iter().any(|key| tree.value_of(key) == Some(name)), "the rename field shows {name}");
        let selection = state.borrow().selection.clone();
        fixture.key(Key::Enter, Modifiers::NONE).settle();
        let rejected = opt_in && matches!(name, "Rose" | "Other");
        if rejected {
            assert_eq!(*renames.borrow(), [(key.clone(), name.to_owned())]);
        } else {
            assert!(renames.borrow().is_empty(), "{name}");
        }
        if accepted {
            let reported = edits.borrow();
            assert!(
                match &reported[..] {
                    [SwatchEdit::Rename { swatch, name: new }] => key == &SwatchKey::Swatch(swatch.clone()) && new == name,
                    [SwatchEdit::RenameGroup { group, name: new }] => key == &SwatchKey::Group(group.clone()) && new == name,
                    _ => false,
                },
                "{name}: {reported:?}",
            );
        } else {
            assert!(edits.borrow().is_empty(), "{name}");
            assert_eq!(state.borrow().selection, selection, "{name}");
        }
    }
}

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

/// Whether the Info panel over `measure` shows each of the names "A", "L", "L1", "L2".
fn measure_names(measure: Option<craft_color_ui::ui::InfoMeasure>) -> [bool; 4] {
    let mut fixture = Fixture::new(move |ui| {
        let readings = craft_color_ui::ui::InfoReadings::new().measure(measure.clone()).document(Some([640, 480]));
        let _ = craft_color_ui::ui::InfoPanel::new("Info", &readings).show(ui);
    });
    fixture.settle();
    ["A", "L", "L1", "L2"].map(|name| fixture.by_label(name).is_some())
}

/// The painted run whose text is `text`, without a colon.
fn run(fixture: &Fixture, text: &str) -> egui::Rect {
    fixture.texts().iter().find(|run| run.text.trim_end_matches(':') == text).unwrap_or_else(|| panic!("no run \"{text}\"")).rect
}

#[trial]
fn the_info_panel_shows_rgb_and_cmyk_as_two_aligned_columns() {
    let mut fixture = Fixture::new(|ui| {
        let color = craft_color_ui::ui::InfoColor::from_parts([10, 200, 30], [1, 22, 33, 44]);
        let readings = craft_color_ui::ui::InfoReadings::new().color(Some(color)).document(Some([640, 480]));
        let _ = craft_color_ui::ui::InfoPanel::new("Info", &readings).show(ui);
    });
    fixture.settle();
    let [r, g, b] = ["10", "200", "30"].map(|value| run(&fixture, value));
    let [c, m, y, k] = ["1%", "22%", "33%", "44%"].map(|value| run(&fixture, value));
    assert!(c.left() > r.right().max(g.right()).max(b.right()), "CMYK starts right of RGB: {r:?} {c:?}");
    assert!(r.left() == g.left() && g.left() == b.left(), "the RGB values share a column");
    assert!(c.left() == m.left() && m.left() == y.left() && y.left() == k.left(), "the CMYK values share a column");
    let [rn, gn, bn] = ["R", "G", "B"].map(|name| run(&fixture, name));
    assert!(rn.right() == gn.right() && gn.right() == bn.right(), "the RGB labels end at one x");
    assert!(rn.right() < r.left(), "the labels sit left of their values");
    assert!(fixture.texts().iter().any(|run| run.text == ":"), "a label carries its colon");
}

#[trial]
fn the_info_panel_shows_the_rulers_measure_while_it_has_one() {
    use craft_color_ui::ui::InfoMeasure;
    assert_eq!(measure_names(Some(InfoMeasure::line("-36.9°", "250.00"))), [true, true, false, false], "a ruler: A and L");
    assert_eq!(measure_names(Some(InfoMeasure::protractor("45.0°", "200.00", "141.42"))), [true, false, true, true], "a protractor: A, L1 and L2");
    assert_eq!(measure_names(None), [false; 4], "no ruler, no measure names");
    let mut fixture = Fixture::new(|ui| {
        let readings =
            craft_color_ui::ui::InfoReadings::new().measure(Some(InfoMeasure::protractor("45.0°", "200.00", "141.42"))).document(Some([640, 480]));
        let _ = craft_color_ui::ui::InfoPanel::new("Info", &readings).show(ui);
    });
    fixture.settle();
    for value in ["45.0°", "200.00", "141.42"] {
        assert!(fixture.by_label(value).is_some(), "the protractor's {value} shows");
    }
}

/// A click on a harmony wheel's marker reports that marker, as the disc under it does.
#[trial]
fn a_click_on_a_harmony_wheel_marker_reports_it() {
    let colors = [Srgba::rgb(0.86, 0.24, 0.16), Srgba::rgb(0.16, 0.86, 0.24)];
    let clicked = Rc::new(Cell::new(None));
    let seen = Rc::clone(&clicked);
    let mut fixture = Fixture::new(move |ui| {
        let out = HarmonyWheel::new("Harmony", &colors).selected_marker(Some(0)).base(Some(0)).show(ui);
        if let Some(hit) = out.clicked {
            seen.set(Some(hit.marker));
        }
    });
    fixture.settle();
    let marker = fixture.by_label(&colors[1].to_hex()).expect("the second marker is in the tree");
    fixture.click(&marker).idle(1);
    assert_eq!(clicked.get(), Some(Some(1)), "the click names the second marker");
}

/// The Color Guide's variation end labels sit at the two sides of the grid: Shades at its left edge and Tints at its
/// right edge.
#[trial]
fn the_color_guide_end_labels_sit_at_the_two_sides_of_the_grid() {
    let mut fixture = Fixture::new(|ui| {
        let (mut state, mut picked) = (HarmonyPanelState::default(), HarmonyPicked::default());
        let _ = HarmonyPanel::new(Bind::bound(&mut state), Bind::bound(&mut picked), RED).show(ui);
    });
    fixture.room(COLOR_ROOM).settle();
    let grid = fixture.by_label("Variations").expect("the variations grid").rect;
    let run = |text: &str| fixture.texts().iter().find(|run| run.text == text).map(|run| run.rect).unwrap_or_else(|| panic!("{text} is painted"));
    let (shades, tints) = (run("Shades"), run("Tints"));
    assert!(shades.left() <= grid.left() + 1.0 && shades.right() < grid.center().x, "Shades {shades:?} starts at the left edge of {grid:?}");
    assert!(tints.right() >= grid.right() - 1.0 && tints.left() > grid.center().x, "Tints {tints:?} ends at the right edge of {grid:?}");
}
