//! The Color Range body: its preview samples with the tool, Shift and Alt, through the real pointer path.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use craft_behavior::PixelSize;
use craft_host::target::Os;
use craft_imaging::selection::PreviewMode;
use craft_imaging::ui::color_range::{ColorRangeDialog, ColorRangeSelect, ColorRangeValue, HostedColorRange, SampleSign, SampleTool};
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::contract::{Bind, Role};
use craft_ui::dialog::DialogHost;
use craft_ui::fonts::{I18nScripts, UiFonts};
use craft_ui::primitives::Revision;
use craft_ui::runtime::env::Scale;
use craft_ui::testing::{Fixture, trial};
use egui::{Color32, ColorImage, Modifiers, Pos2, Rect, Vec2, vec2};

/// The proxy: 40 × 20 pixels shown in a square well, so the picture is letterboxed and the well has room off it. It is
/// large enough that the fitted zoom stays inside the pane's zoom range, so the picture spans the well's width.
const PROXY: [u32; 2] = [40, 20];
const SCALE: u32 = 5;
const DOCUMENT: [u32; 2] = [200, 100];

struct Flat;

impl RasterSource for Flat {
    fn size(&self) -> PixelSize {
        PixelSize::new(PROXY[0], PROXY[1])
    }

    fn revision(&self) -> Revision {
        Revision::of(&PROXY)
    }

    fn damage(&self, _: Revision) -> Option<PixelRect> {
        None
    }

    fn read(&self, _: PixelRect, _: u32, out: &mut ColorImage) {
        out.pixels.fill(Color32::WHITE);
    }
}

struct State {
    value: ColorRangeValue,
    tool: SampleTool,
}

/// The dialog body in a fixture over a shared state; the tool the body reports is adopted as the host does. While
/// `twice` is on, egui runs each frame twice and gives the frame's input to its first pass only.
fn fixture(state: &Rc<RefCell<State>>, twice: &Rc<Cell<bool>>) -> Fixture {
    let (shown, twice) = (state.clone(), Rc::clone(twice));
    let mut fixture = Fixture::new(move |ui| {
        let mut st = shown.borrow_mut();
        let tool = st.tool;
        let out = ColorRangeDialog::new("Color Range", Bind::bound(&mut st.value))
            .mask(&Flat)
            .image(&Flat)
            .picture_scale(SCALE)
            .document(DOCUMENT)
            .tool(tool)
            .show(ui);
        if let Some(next) = out.tool {
            st.tool = next;
        }
        if twice.get() && ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("the frame runs twice");
        }
    });
    fixture.settle();
    fixture
}

/// The screen point at the centre of proxy cell (`x`, `y`) of the letterboxed picture in `well`.
fn cell(well: Rect, x: f32, y: f32) -> Pos2 {
    let fx = (x + 0.5) / PROXY[0] as f32;
    let fy = 0.25 + 0.5 * (y + 0.5) / PROXY[1] as f32;
    well.min + vec2(well.width() * fx, well.height() * fy)
}

#[trial]
fn preview_presses_and_drags_sample_add_and_subtract_in_order() {
    let state = Rc::new(RefCell::new(State { value: ColorRangeValue::default(), tool: SampleTool::Sample }));
    let twice = Rc::new(Cell::new(false));
    let mut fixture = fixture(&state, &twice);
    let well = fixture.by_label("Color Range preview").expect("the preview pane").rect;
    let samples = || state.borrow().value.samples.clone();
    let cell_at = |x: f64, y: f64| [x * f64::from(SCALE) + f64::from(SCALE / 2), y * f64::from(SCALE) + f64::from(SCALE / 2)];

    // A plain press replaces the samples with the cell's centre in document pixels.
    fixture.click_at(cell(well, 2.0, 3.0));
    fixture.settle();
    assert_eq!(samples().add, vec![cell_at(2.0, 3.0)]);
    assert_eq!(samples().order(), Some(&[SampleSign::Add][..]));
    fixture.click_at(cell(well, 4.0, 4.0));
    fixture.settle();
    assert_eq!(samples().add, vec![cell_at(4.0, 4.0)], "the eyedropper replaces the samples");

    // Add to Sample appends, and a drag adds one sample for the press and one per frame the pointer moves; a held
    // frame and a rerun pass of the same frame add none.
    state.borrow_mut().tool = SampleTool::Add;
    fixture.click_at(cell(well, 1.0, 1.0));
    fixture.settle();
    assert_eq!(samples().add.len(), 2);
    let before = samples().add.len();
    let from = cell(well, 6.0, 2.0);
    let by = cell(well, 14.0, 2.0) - from;
    fixture.drag_hold_at(from, by);
    assert_eq!(samples().add.len(), before + 1 + 4, "the press and each of the four moving frames add one");
    assert_eq!(samples().add.last(), Some(&cell_at(14.0, 2.0)));
    fixture.frame(Vec::new());
    assert_eq!(samples().add.len(), before + 1 + 4, "a held frame with no move adds none");
    twice.set(true);
    fixture.frame(Vec::new());
    twice.set(false);
    assert_eq!(samples().add.len(), before + 1 + 4, "a rerun pass adds none");
    fixture.release_at(from + by);
    fixture.settle();
    assert_eq!(samples().add.len(), before + 1 + 4, "the release adds none");

    // Shift adds and Alt subtracts whatever the tool; repeated Alt presses each append.
    state.borrow_mut().tool = SampleTool::Sample;
    let added = samples().add.len();
    fixture.hold(Modifiers { shift: true, ..Modifiers::NONE });
    fixture.click_at(cell(well, 0.0, 0.0));
    fixture.settle();
    assert_eq!(samples().add.len(), added + 1);
    fixture.hold(Modifiers { alt: true, ..Modifiers::NONE });
    fixture.click_at(cell(well, 9.0, 0.0));
    fixture.settle();
    fixture.click_at(cell(well, 9.0, 4.0));
    fixture.settle();
    fixture.hold(Modifiers::NONE);
    assert_eq!(samples().subtract, vec![cell_at(9.0, 0.0), cell_at(9.0, 4.0)]);
    assert_eq!(samples().order.last(), Some(&SampleSign::Subtract));
    assert!(samples().order().is_some());

    // The order answers only while it matches the lists.
    state.borrow_mut().value.samples.add.pop();
    assert_eq!(samples().order(), None);

    // A press off the picture samples nothing.
    let count = samples().order.len();
    fixture.click_at(well.min + vec2(well.width() * 0.5, well.height() * 0.05));
    fixture.settle();
    assert_eq!(samples().order.len(), count);

    // A first Add with no samples gives one added sample.
    {
        let mut st = state.borrow_mut();
        st.value.samples.clear();
        st.tool = SampleTool::Add;
    }
    fixture.click_at(cell(well, 3.0, 2.0));
    fixture.settle();
    assert_eq!(samples().add, vec![cell_at(3.0, 2.0)]);
    assert_eq!(samples().order(), Some(&[SampleSign::Add][..]));

    // With Sampled Colors off the preview takes no sample.
    state.borrow_mut().value.select = ColorRangeSelect::Reds;
    let count = samples().order.len();
    fixture.click_at(cell(well, 5.0, 2.0));
    fixture.settle();
    assert_eq!(samples().order.len(), count);
}

/// Open the same form used by Select > Color Range with a ready image and mask.
fn hosted_fixture(room: Vec2) -> (Rc<RefCell<DialogHost>>, Fixture) {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    UiFonts::new(&I18nScripts).install(fixture.ctx());
    fixture.os(Os::Windows).scale(Scale::new(1.25)).room(room);
    let mut dialog = HostedColorRange::new("Color Range", ColorRangeValue::default(), DOCUMENT);
    dialog.set_preview(Some(Arc::new(Flat)), Some(Arc::new(Flat)), SCALE, None);
    host.borrow_mut().open(dialog);
    fixture.settle().idle(10);
    (host, fixture)
}

#[trial]
fn color_range_15558_hosted_controls_fit_at_125_percent() {
    for room in [vec2(1600.0, 1000.0), vec2(1280.0, 800.0)] {
        let (_, fixture) = hosted_fixture(room);
        let dialog = fixture.by_role(Role::DIALOG)[0].rect;
        for (label, role) in [
            ("Eyedropper", Role::RADIO_BUTTON),
            ("Add to Sample", Role::RADIO_BUTTON),
            ("Subtract from Sample", Role::RADIO_BUTTON),
            ("Invert", Role::CHECK_BOX),
            ("Selection", Role::RADIO_BUTTON),
            ("Image", Role::RADIO_BUTTON),
            ("Selection Preview", Role::COMBO_BOX),
        ] {
            let target = fixture
                .by_role(role)
                .into_iter()
                .find(|target| fixture.tree().name_of(target.node) == Some(label))
                .unwrap_or_else(|| panic!("missing {label} at {room:?}"));
            assert!(target.rect.width() > 0.0 && dialog.contains_rect(target.rect), "{label}: {:?} outside {dialog:?} at {room:?}", target.rect);
        }
        let hint = fixture.texts().iter().find(|text| text.text == "Sample: foreground color").expect("sample source hint");
        assert!(hint.clip.contains_rect(hint.rect), "sample source hint is clipped");
    }
}

#[trial]
fn color_range_15558_hosted_controls_respond_to_pointer() {
    let (host, mut fixture) = hosted_fixture(vec2(1280.0, 800.0));
    let well = fixture.by_label("Color Range preview").expect("preview pane").rect;
    let click = |fixture: &mut Fixture, label: &str| {
        let target = fixture.by_label(label).expect(label);
        fixture.click_at(target.center()).settle();
    };
    let tool = || host.borrow_mut().get_mut::<HostedColorRange>().expect("open").tool();
    let samples = || host.borrow_mut().get_mut::<HostedColorRange>().expect("open").value().samples.clone();

    fixture.click_at(cell(well, 2.0, 3.0)).settle();
    click(&mut fixture, "Add to Sample");
    assert_eq!(tool(), SampleTool::Add);
    fixture.click_at(cell(well, 5.0, 3.0)).settle();
    assert_eq!(samples().add, [[12.0, 17.0], [27.0, 17.0]]);
    click(&mut fixture, "Subtract from Sample");
    assert_eq!(tool(), SampleTool::Subtract);
    fixture.click_at(cell(well, 7.0, 3.0)).settle();
    assert_eq!(samples().subtract, [[37.0, 17.0]]);
    click(&mut fixture, "Eyedropper");
    assert_eq!(tool(), SampleTool::Sample);

    click(&mut fixture, "Invert");
    click(&mut fixture, "Image");
    click(&mut fixture, "Selection Preview");
    click(&mut fixture, "Black Matte");
    let mut host = host.borrow_mut();
    let dialog = host.get_mut::<HostedColorRange>().expect("open after pointer input");
    assert!(dialog.value().invert);
    assert!(dialog.shows_image());
    assert_eq!(dialog.preview_mode(), PreviewMode::BlackMatte);
}
