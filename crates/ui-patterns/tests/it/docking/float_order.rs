//! A dock's floating windows stack with the app's own egui windows in egui's area order: the window shown or pressed
//! last is on top, so a dialog the app opens over a floating panel shows above it.

use super::Numbered;
use craft_behavior::dock::{Floating, Layout, Node, Workspace};
use craft_ui::contract::{Bind, BodyCx, Tracked};
use craft_ui::docking::{DockConfig, DockPanels};
use craft_ui::patterns::dock::DockArea;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Fixture, trial};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

const WINDOW_AT: Rect = Rect { min: Pos2::new(130.0, 90.0), max: Pos2::new(330.0, 210.0) };
/// The app's dialog: it covers the floating window's right part.
const DIALOG_AT: Pos2 = Pos2::new(220.0, 120.0);
/// Inside both the floating window and the dialog.
const BOTH: Pos2 = Pos2::new(260.0, 170.0);
/// Inside the floating window only.
const WINDOW_ONLY: Pos2 = Pos2::new(160.0, 190.0);

struct Panels;

impl DockPanels<usize> for Panels {
    fn body(&mut self, _: &usize, _: &mut Ui, _: &mut BodyCx<'_>) {}
}

#[trial]
fn an_app_dialog_opened_over_a_floating_window_shows_above_it_until_the_window_is_pressed() {
    let layout = Layout::new(Some(Node::tabs(vec![1], 0)), vec![Floating::single(100, vec![2], 0, WINDOW_AT)]);
    let mut workspace = Arc::new(Workspace::new(layout));
    let dialog = Rc::new(Cell::new(false));
    let shows = Rc::clone(&dialog);
    let specs = Tracked::new(Numbered);
    let mut fixture = Fixture::new(move |ui| {
        DockArea::new("Dock", Bind::bound(&mut workspace), &specs, &mut Panels, DockConfig::default()).show(ui);
        if shows.get() {
            egui::Window::new("App dialog").fixed_pos(DIALOG_AT).fixed_size(Vec2::new(300.0, 200.0)).collapsible(false).show(ui.ctx(), |ui| {
                ui.label("Dialog");
            });
        }
    });
    fixture.room(Vec2::new(800.0, 600.0)).settle();
    let float = fixture.ctx().layer_id_at(BOTH).expect("the floating window shows");
    dialog.set(true);
    fixture.settle();
    let top = fixture.ctx().layer_id_at(BOTH).expect("a layer shows there");
    assert_ne!(top, float, "the dialog the app opened last is above the floating window");
    assert_eq!(fixture.ctx().layer_id_at(WINDOW_ONLY), Some(float), "the floating window still shows beside the dialog");
    fixture.frame(vec![moved(WINDOW_ONLY)]).frame(vec![press(WINDOW_ONLY)]).frame(vec![release(WINDOW_ONLY)]).settle();
    assert_eq!(fixture.ctx().layer_id_at(BOTH), Some(float), "a press raises the floating window above the dialog");
}

const LOWER_AT: Rect = Rect { min: Pos2::new(100.0, 100.0), max: Pos2::new(300.0, 260.0) };
const UPPER_AT: Rect = Rect { min: Pos2::new(200.0, 150.0), max: Pos2::new(400.0, 310.0) };
/// Inside both windows.
const OVERLAP: Pos2 = Pos2::new(250.0, 220.0);
/// Inside the lower window only.
const LOWER_ONLY: Pos2 = Pos2::new(130.0, 240.0);

/// `DockArea::raise` brings the floating window holding the panel to the front for the pass it is given in, and the
/// window stays there after it.
#[trial]
fn a_raised_floating_panel_comes_to_the_front_of_the_window_it_overlaps() {
    let layout =
        Layout::new(Some(Node::tabs(vec![1], 0)), vec![Floating::single(100, vec![2], 0, LOWER_AT), Floating::single(101, vec![3], 0, UPPER_AT)]);
    let mut workspace = Arc::new(Workspace::new(layout));
    let raise = Rc::new(Cell::new(None));
    let asked = Rc::clone(&raise);
    let specs = Tracked::new(Numbered);
    let mut fixture = Fixture::new(move |ui| {
        let mut panels = Panels;
        let mut area = DockArea::new("Dock", Bind::bound(&mut workspace), &specs, &mut panels, DockConfig::default());
        if let Some(panel) = asked.take() {
            area = area.raise(panel);
        }
        area.show(ui);
    });
    fixture.room(Vec2::new(800.0, 600.0)).settle();
    let lower = fixture.ctx().layer_id_at(LOWER_ONLY).expect("the lower window shows");
    assert_ne!(fixture.ctx().layer_id_at(OVERLAP), Some(lower), "the window shown last is on top");
    raise.set(Some(2));
    fixture.frame(Vec::new()).settle();
    assert_eq!(fixture.ctx().layer_id_at(OVERLAP), Some(lower), "the raised window is on top");
    raise.set(Some(1));
    fixture.frame(Vec::new()).settle();
    assert_eq!(fixture.ctx().layer_id_at(OVERLAP), Some(lower), "a docked panel's raise moves no window");
}
