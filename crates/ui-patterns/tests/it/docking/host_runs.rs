//! The app's own egui code in a dock (`PanelSpec::host_body`: a panel's body, or the view it takes its group with) runs
//! once per pass, in the real run: a measuring run takes the size its last real run took. egui then sees each of the
//! app's widgets once per pass, so it reports no id clash.

use craft_behavior::Axis;
use craft_behavior::dock::{Layout, Node, SplitSize, Workspace};
use craft_ui::contract::{Bind, BodyCx, PlainData, Tracked};
use craft_ui::docking::{DockConfig, DockPanels, GroupAt, PanelSpec, PanelSpecs};
use craft_ui::patterns::dock::DockArea;
use craft_ui::runtime::measure;
use craft_ui::testing::{Fixture, trial};
use egui::{Id, Sense, Shape, Ui, Vec2};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// The app's egui panel, its canvas (a view that takes its group) and a framework panel.
const HOSTED: usize = 0;
const CANVAS: usize = 1;

#[derive(PlainData)]
struct Specs;

impl PanelSpecs<usize> for Specs {
    fn spec(&self, panel: &usize) -> PanelSpec {
        let spec = PanelSpec::new(format!("Panel {panel}"));
        match *panel {
            HOSTED => spec.host_body(),
            CANVAS => spec.canvas().host_body(),
            _ => spec,
        }
    }
}

/// One run of the app's code: what ran, in which pass, and whether a measuring run was open.
type Ran = (&'static str, u64, bool);

struct Panels {
    runs: Rc<RefCell<Vec<Ran>>>,
}

impl DockPanels<usize> for Panels {
    fn body(&mut self, panel: &usize, ui: &mut Ui, _: &mut BodyCx<'_>) {
        if *panel != HOSTED {
            return;
        }
        self.runs.borrow_mut().push(("body", ui.ctx().cumulative_pass_nr(), measure::active(ui.ctx())));
        let _ = ui.button("Apply");
        egui::CollapsingHeader::new("Section").default_open(true).show(ui, |ui| {
            let _ = ui.button("Inside");
        });
    }

    fn group_view(&mut self, panels: &[usize], _: usize, ui: &mut Ui, at: GroupAt, _: &mut BodyCx<'_>) -> Option<f32> {
        let GroupAt::Show(rect) = at else { return None };
        if !panels.contains(&CANVAS) {
            return None;
        }
        self.runs.borrow_mut().push(("view", ui.ctx().cumulative_pass_nr(), measure::active(ui.ctx())));
        let _ = ui.interact(rect, Id::unique("host canvas"), Sense::click_and_drag());
        Some(rect.height())
    }
}

#[trial]
fn the_apps_egui_code_in_a_dock_runs_once_per_pass_and_raises_no_id_clash() {
    let runs = Rc::new(RefCell::new(Vec::new()));
    let left = Node::split(Axis::Y, SplitSize::Ratio(0.5), Node::tabs(vec![HOSTED], 0), Node::tabs(vec![2], 0));
    let root = Node::split(Axis::X, SplitSize::Ratio(0.3), left, Node::tabs(vec![CANVAS], 0));
    let mut workspace = Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())));
    let (mut panels, specs) = (Panels { runs: runs.clone() }, Tracked::new(Specs));
    let mut fixture = Fixture::new(move |ui| {
        ui.ctx().options_mut(|options| options.warn_on_id_clash = true);
        DockArea::new("Dock", Bind::bound(&mut workspace), &specs, &mut panels, DockConfig::default()).show(ui);
    });
    // From the first frame on, through a new room, so the dock measures its groups again.
    fixture.room(Vec2::new(1200.0, 600.0)).settle();
    fixture.room(Vec2::new(1000.0, 700.0)).settle();
    fixture.record_paint().frame(Vec::new());

    let runs = runs.borrow().clone();
    for part in ["body", "view"] {
        let ran: Vec<_> = runs.iter().filter(|(name, ..)| *name == part).collect();
        assert!(!ran.is_empty(), "the app's {part} ran");
        assert!(ran.iter().all(|(.., measuring)| !measuring), "the app's {part} ran in a measuring run: {ran:?}");
        let mut passes: Vec<_> = ran.iter().map(|(_, pass, _)| *pass).collect();
        passes.dedup();
        assert_eq!(passes.len(), ran.len(), "the app's {part} ran more than once in a pass: {ran:?}");
    }
    let paint = fixture.take_paint().expect("the frame recorded its paint");
    let clashes: Vec<String> = paint
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            Shape::Text(text) if text.galley.job.text.contains("use of widget ID") => Some(text.galley.job.text.clone()),
            _ => None,
        })
        .collect();
    assert!(clashes.is_empty(), "egui reported id clashes: {clashes:?}");
}
