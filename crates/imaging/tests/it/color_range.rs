//! The Color Range body: its preview samples with the tool, Shift and Alt, through the real pointer path.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_behavior::PixelSize;
use craft_imaging::ui::color_range::{ColorRangeDialog, ColorRangeSelect, ColorRangeValue, SampleSign, SampleTool};
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::contract::Bind;
use craft_ui::primitives::Revision;
use craft_ui::testing::{Fixture, trial};
use egui::{Color32, ColorImage, Modifiers, Pos2, Rect, vec2};

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
