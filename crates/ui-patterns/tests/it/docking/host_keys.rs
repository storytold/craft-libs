//! An app that hosts its own egui UI in the dock: its canvas is a group the host draws (`group_view`) and the window's
//! key target, and its other panels are its own egui code (`PanelSpec::host_body`). Keys its shortcut table reads reach
//! egui's input.

use craft_behavior::Axis;
use craft_behavior::dock::{Layout, Node, SplitSize, Workspace};
use craft_keymap::{Chord, Key as KeyChord, Mods, Overrides};
use craft_ui::contract::{Bind, BodyCx, PlainData, Tracked};
use craft_ui::docking::{DockConfig, DockPanels, GroupAt, PanelSpec, PanelSpecs};
use craft_ui::patterns::dock::DockArea;
use craft_ui::runtime::commands::{CommandSet as _, NavCommand};
use craft_ui::runtime::focus::{EditSet, KeepKeys, KeyUse, host_canvas};
use craft_ui::testing::input::{key_press, key_reached};
use craft_ui::testing::{Fixture, trial};
use egui::{Id, Key, Modifiers, Sense, Ui, Vec2};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

/// Panel 0 is the app's canvas; panels 1 and 2 are its own egui panels.
const CANVAS: usize = 0;

/// The Alt+digit chords the canvas claims.
const CLAIMS: [Chord; 2] = [Chord::new(Mods::ALT, KeyChord::Num1), Chord::new(Mods::ALT, KeyChord::Num2)];

/// The canvas's keys: Tab, the edit chords and its Alt+digit claims stay in egui's input for the app.
const KEYS: KeyUse = KeyUse::CANVAS.with_keep(KeepKeys::Tab).with_edits(EditSet::ALL).with_claims(&CLAIMS);

#[derive(PlainData)]
struct Specs;

impl PanelSpecs<usize> for Specs {
    fn spec(&self, panel: &usize) -> PanelSpec {
        let spec = PanelSpec::new(format!("Panel {panel}")).host_body();
        if *panel == CANVAS { spec.canvas() } else { spec }
    }
}

/// The app's panels; `f6` records whether an F6 press reached egui's input in a pass the canvas ran in.
struct Panels {
    f6: Rc<Cell<bool>>,
}

impl DockPanels<usize> for Panels {
    fn body(&mut self, _: &usize, ui: &mut Ui, _: &mut BodyCx<'_>) {
        ui.label("rows");
    }

    fn group_view(&mut self, panels: &[usize], _: usize, ui: &mut Ui, at: GroupAt, _: &mut BodyCx<'_>) -> Option<f32> {
        if !panels.contains(&CANVAS) {
            return None;
        }
        let GroupAt::Show(rect) = at else { return Some(0.0) };
        let id = Id::unique("app canvas");
        ui.interact(rect, id, Sense::click_and_drag());
        host_canvas(ui.ctx(), id, KEYS);
        if key_reached(ui.ctx(), Key::F6) {
            self.f6.set(true);
        }
        Some(rect.height())
    }
}

/// With both pane moves unbound, F6 reaches the app's canvas in a dock of `host_body` panels.
#[trial]
fn an_unbound_f6_reaches_a_hosted_canvas_in_the_dock() {
    let f6 = Rc::new(Cell::new(false));
    let root = Node::split(
        Axis::X,
        SplitSize::Ratio(0.25),
        Node::tabs(vec![1], 0),
        Node::split(Axis::X, SplitSize::Ratio(0.7), Node::tabs(vec![CANVAS], 0), Node::tabs(vec![2], 0)),
    );
    let mut workspace = Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())));
    let (mut panels, specs) = (Panels { f6: f6.clone() }, Tracked::new(Specs));
    let mut fixture = Fixture::new(move |ui| {
        DockArea::new("Dock", Bind::bound(&mut workspace), &specs, &mut panels, DockConfig::default()).show(ui);
    });
    let mut unbound = Overrides::default();
    unbound.insert(NavCommand::NextPane.id(), Vec::new());
    unbound.insert(NavCommand::PreviousPane.id(), Vec::new());
    fixture.overrides(Arc::new(unbound)).room(Vec2::new(1200.0, 600.0)).settle();
    f6.set(false);
    fixture.frame(vec![key_press(Key::F6, Modifiers::NONE)]);
    assert!(f6.get(), "the app reads F6 from egui");
}
