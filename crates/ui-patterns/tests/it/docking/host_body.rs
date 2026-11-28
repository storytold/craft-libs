//! A panel whose body is the app's own egui code (`PanelSpec::host_body`) lays out with the style of the `Ui` the dock
//! was shown in, on that style's panel fill and without the group's body padding; any other body keeps the group's
//! spacing, padding and surface.

use craft_behavior::Axis;
use craft_behavior::dock::{Layout, Node, SplitSize, Workspace};
use craft_ui::contract::{Bind, BodyCx, PlainData, Tracked};
use craft_ui::docking::{DockConfig, DockPanels, PanelSpec, PanelSpecs};
use craft_ui::patterns::dock::DockArea;
use craft_ui::testing::{Fixture, trial};
use egui::{Color32, Rect, Shape, Ui, Vec2};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// The app's own egui panel; the other is a plain panel.
const HOSTED: usize = 0;

#[derive(PlainData)]
struct Specs;

impl PanelSpecs<usize> for Specs {
    fn spec(&self, panel: &usize) -> PanelSpec {
        let spec = PanelSpec::new(format!("Panel {panel}"));
        if *panel == HOSTED { spec.host_body() } else { spec }
    }
}

/// The app's panel fill, which no theme surface has.
const APP_FILL: Color32 = Color32::from_rgb(3, 140, 7);

/// What a body's `Ui` had: its item spacing and its room.
type Seen = Option<(Vec2, Rect)>;

/// Records what each body's `Ui` had, by panel.
struct Panels {
    seen: Rc<RefCell<[Seen; 2]>>,
}

impl DockPanels<usize> for Panels {
    fn body(&mut self, panel: &usize, ui: &mut Ui, _: &mut BodyCx<'_>) {
        self.seen.borrow_mut()[*panel] = Some((ui.spacing().item_spacing, ui.max_rect()));
    }
}

#[trial]
fn a_host_body_lays_out_with_the_style_outside_the_dock_and_other_bodies_keep_the_groups() {
    let seen = Rc::new(RefCell::new([None; 2]));
    let outside = Rc::new(RefCell::new(None));
    let root = Node::split(Axis::X, SplitSize::Ratio(0.5), Node::tabs(vec![0], 0), Node::tabs(vec![1], 0));
    let mut workspace = Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())));
    let (mut panels, specs) = (Panels { seen: seen.clone() }, Tracked::new(Specs));
    let outer = outside.clone();
    let mut fixture = Fixture::new(move |ui| {
        ui.visuals_mut().panel_fill = APP_FILL;
        *outer.borrow_mut() = Some(ui.spacing().item_spacing);
        DockArea::new("Dock", Bind::bound(&mut workspace), &specs, &mut panels, DockConfig::default()).show(ui);
    });
    fixture.room(Vec2::new(1200.0, 600.0)).settle();
    fixture.record_paint().frame(Vec::new());
    let outside = outside.borrow().expect("the dock was shown");
    let [hosted, plain] = *seen.borrow();
    let (hosted, hosted_room) = hosted.expect("the hosted body was shown");
    let (plain, plain_room) = plain.expect("the plain body was shown");
    assert_eq!(hosted, outside, "the app's own egui body lays out as it does outside the dock");
    assert_ne!(plain, outside, "a framework body keeps the group's spacing");
    assert!(hosted_room.width() > plain_room.width(), "the app's body has the group's whole width, a framework body its padding less");

    let mut paint = fixture.take_paint().expect("the frame recorded its paint");
    paint.textures_delta.clear();
    let filled = |room: Rect| {
        paint.shapes.iter().any(|clipped| matches!(&clipped.shape, Shape::Rect(rect) if rect.fill == APP_FILL && rect.rect.contains(room.center())))
    };
    assert!(filled(hosted_room), "the app's body shows on the app's panel fill");
    assert!(!filled(plain_room), "a framework body keeps the theme's surface");
}
