//! A floating tool rail window: the window and every surface its content paints keep the window's rounded corners.

use craft_behavior::dock::{Column, Floating, Group, Layout, Workspace};
use craft_ui::contract::{Bind, BodyCx, Tracked};
use craft_ui::docking::{DockConfig, DockGrid, DockPanels, GroupAt, PanelSpec, PanelSpecs};
use craft_ui::patterns::dock::DockArea;
use craft_ui::rails::{LastUsed, RailTool, ToolRail, ToolSlot};
use craft_ui::testing::{Fixture, trial};
use egui::{Pos2, Rect, Ui, Vec2};
use std::sync::Arc;

const RAIL: usize = 1;
const WINDOW_AT: Rect = Rect { min: Pos2::new(100.0, 40.0), max: Pos2::new(148.0, 440.0) };

#[derive(craft_ui::contract::PlainData)]
struct Specs;

impl PanelSpecs<usize> for Specs {
    fn spec(&self, _: &usize) -> PanelSpec {
        PanelSpec::new("Toolbar").grid(DockGrid::rail(4))
    }
}

struct Panels {
    slots: Tracked<Vec<ToolSlot<u8>>>,
    last: Tracked<LastUsed<u8>>,
}

impl DockPanels<usize> for Panels {
    fn body(&mut self, _: &usize, _: &mut Ui, _: &mut BodyCx<'_>) {}

    fn group_view(&mut self, _: &[usize], _: usize, ui: &mut Ui, at: GroupAt, _: &mut BodyCx<'_>) -> Option<f32> {
        let rail = ToolRail::new("Toolbar", &self.slots, &0u8, &self.last);
        match at {
            GroupAt::Measure(width) => Some(rail.measure_rail(ui, width, |_, _| {}).y),
            GroupAt::Show(rect) => {
                rail.show_rail(ui, |_, _| {});
                Some(rect.height())
            }
        }
    }
}

fn panels() -> Panels {
    let slots = (0..4u8).map(|id| ToolSlot::new(u32::from(id), vec![RailTool::new(id, craft_ui::runtime::commands::EditCommands::Undo)])).collect();
    Panels { slots: Tracked::new(slots), last: Tracked::new(LastUsed::default()) }
}

/// The window's own surface: the rect painted at its place with an opaque fill and round corners.
fn window_of(shapes: &[(Rect, egui::Shape)]) -> egui::epaint::RectShape {
    let mut rects = shapes.iter().filter_map(|(_, shape)| if let egui::Shape::Rect(rect) = shape { Some(rect) } else { None });
    let window = rects.find(|rect| rect.fill.a() == 255 && (rect.rect.min - WINDOW_AT.min).length() < 1.0 && rect.corner_radius.sw > 0);
    window.cloned().unwrap_or_else(|| panic!("the window's surface is painted at {WINDOW_AT:?}"))
}

/// The opaque rect surfaces that reach a lower corner of `window` with a smaller radius there than the window's.
fn square_at_the_foot(shapes: &[(Rect, egui::Shape)], window: &egui::epaint::RectShape) -> Vec<String> {
    let near = |a: egui::Pos2, b: egui::Pos2| (a - b).length() <= 0.75;
    let (rect, radius) = (window.rect, window.corner_radius);
    let mut out = Vec::new();
    for (_, shape) in shapes {
        let egui::Shape::Rect(inner) = shape else { continue };
        if inner.fill.a() < 255 || inner.rect == rect {
            continue;
        }
        let left = near(inner.rect.left_bottom(), rect.left_bottom()) && inner.corner_radius.sw < radius.sw;
        let right = near(inner.rect.right_bottom(), rect.right_bottom()) && inner.corner_radius.se < radius.se;
        if left || right {
            out.push(format!("{:?} {:?}", inner.rect, inner.corner_radius));
        }
    }
    out
}

fn shown(rounded: bool) -> Vec<(Rect, egui::Shape)> {
    let layout = Layout::new(None, vec![Floating::new(1, Column::new(vec![Group::new(vec![RAIL], 0)]).with_steps(1), WINDOW_AT)]);
    let workspace = std::cell::RefCell::new(Arc::new(Workspace::new(layout)));
    let specs = Tracked::new(Specs);
    let mut panels = panels();
    let mut fixture = Fixture::new(move |ui| {
        let mut shown = workspace.borrow().clone();
        let config = DockConfig::default().native_windows(false).floating_grip(true).floating_rounded_body(rounded);
        let _ = DockArea::new("Dock", Bind::bound(&mut shown), &specs, &mut panels, config).show(ui);
        *workspace.borrow_mut() = shown;
    });
    fixture.room(Vec2::new(600.0, 600.0));
    fixture.settle();
    fixture.frame_clipped_shapes()
}

/// A floating tool rail (production's quick toolbar) is one rounded piece: the dock column and the rail's chrome inside
/// the window keep to its rounded lower corners, and the window's edge is drawn again over them. Without the opt-in the
/// inner surfaces run square to the window's foot, over its corners.
#[trial]
fn a_rounded_window_body_keeps_its_surfaces_inside_the_rounded_corners() {
    let plain = shown(false);
    let window = window_of(&plain);
    assert!(!square_at_the_foot(&plain, &window).is_empty(), "by default the inner surfaces run square to the window's foot");

    let rounded = shown(true);
    let window = window_of(&rounded);
    let square = square_at_the_foot(&rounded, &window);
    assert!(square.is_empty(), "no inner surface squares a corner of {:?}: {square:?}", window.rect);
    let rect_at = |at: usize| if let Some((_, egui::Shape::Rect(rect))) = rounded.get(at) { Some(rect) } else { None };
    let edge = (0..rounded.len())
        .rev()
        .find(|at| rect_at(*at).is_some_and(|rect| rect.rect == window.rect && rect.fill.a() == 0 && rect.stroke.width > 0.0));
    let inner = (0..rounded.len()).rev().find(|at| {
        rect_at(*at).is_some_and(|rect| {
            rect.fill.a() == 255 && rect.rect != window.rect && window.rect.contains_rect(rect.rect) && rect.rect.width() >= window.rect.width() - 1.0
        })
    });
    assert!(edge.zip(inner).is_some_and(|(edge, inner)| edge > inner), "the window's edge ({edge:?}) is drawn over the inner surfaces ({inner:?})");
}
