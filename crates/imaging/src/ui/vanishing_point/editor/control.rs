//! [`VanishingPlanes`]: the planes of the editor as a canvas control. Every corner of a drawn plane is a part in the
//! canvas's arena (the keyboard path: Page keys pick, arrows move it), and so is every edge node, which takes the
//! drags that slide an edge, tear a plane off or set a torn plane's depth.

use craft_behavior::{
    CancelReason, CursorIcon, HitLevel, HitPriority, HitShape, NavShape, PartKey, PointerButton, PressPhase, PressState, Roving, Sense,
};
use craft_ui::canvas::{NUDGE, NUDGE_FAR, canvas_space};
use craft_ui::contract::{
    Access, Common, Component, Cx, Focus as _, MeasureCx, Name, NoValue, Outcome, PaintUi, PartsCx, Pointer as _, Props as _, Role, Styled as _,
    WordSetCx as _,
};
use craft_ui::runtime::edit::next_gesture_id;
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::KeyUse;
use craft_ui::style::StyleCx;
use egui::{Rect, Response, Vec2};

use super::is_unset;
use super::{Shared, VanishingTool};
use crate::ui::vanishing_point::draw::{self, Snapshot};
use crate::ui::vanishing_point::style::{self, VanishingPointStyle};
use crate::ui::vanishing_point::words::VanishingPointWords;

/// A corner of a plane: `CORNER.at(plane * 4 + corner)`, corners counted top-left, top-right, bottom-right,
/// bottom-left.
const CORNER: PartKey = PartKey::new("corner");

/// An edge node of a plane: `EDGE.at(plane * 4 + edge)`, edges counted top, right, bottom, left.
const EDGE: PartKey = PartKey::new("edge");

/// What the planes control reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct VanishingPlanesOutput {
    /// The control's response.
    pub response: Response,
    /// The selected plane after the pass.
    pub selected: Option<usize>,
}

impl Outcome for VanishingPlanesOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The planes of the editor on a canvas: outlines, grids, nodes, Create Plane's pending corners, the stamp ring and
/// the source cross, all through the theme's overlay colours.
///
/// - **Parts.** Each corner of a drawn plane is a focusable part named "Plane n, top-left corner" and valued "x, y" in
///   document pixels, one roving group (Page keys pick and select the plane, Home and End go to the ends). While a
///   corner has the keyboard the arrow keys move it one document pixel (Shift ten). Each edge node is a part that
///   takes a drag.
/// - **Pointer.** With Edit Plane a drag on a corner reshapes the plane and a drag on an edge node slides the edge along
///   the plane's perspective; Command and a drag on an edge node tear off a perpendicular plane whose far edge node
///   sets its depth (Alt: its angle). With Create Plane a press on an edge node is a corner, as everywhere else.
/// - **Look.** A plane is drawn in the colour of the scene's check: the accent (valid), the warning colour
///   (questionable), the error colour (invalid, outline only). The selected plane's outline is thick and shows its
///   nodes.
/// - The planes never mirror in a right-to-left interface: they are the document's.
#[derive(craft_ui::contract::Component)]
#[component(canvas_control)]
pub struct VanishingPlanes<'a> {
    #[common]
    common: Common,
    pass: &'a dyn Shared,
}

impl<'a> VanishingPlanes<'a> {
    pub(super) fn new(pass: &'a dyn Shared) -> Self {
        VanishingPlanes { common: Common::new(), pass }.label(Label::Empty)
    }

    fn snapshot(&self) -> Snapshot {
        let mut found = None;
        self.pass.with(&mut |work| found = Some(Snapshot::of(work)));
        found.expect("the pass runs the closure once")
    }
}

/// What a part press landed on.
#[derive(Clone, Copy)]
enum Node {
    Corner(usize, usize),
    Edge(usize, usize),
}

fn key_of(plane: usize, index: usize) -> u64 {
    (plane * 4 + index) as u64
}

impl Component for VanishingPlanes<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::CANVAS;
    type Style = VanishingPointStyle;
    type Output = VanishingPlanesOutput;

    fn style(cx: &StyleCx<'_>, _: VanishingPlanesStyleKey) -> VanishingPointStyle {
        style::style(cx)
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    /// The corners in one roving group in keyboard order, and the edge nodes: drag parts at handle priority, the
    /// selected plane's first.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let snap = self.snapshot();
        let reach = Vec2::splat(cx.style.reach);
        let edit = snap.tool == VanishingTool::EditPlane;
        let selected = snap.selected_plane();
        for (i, plane) in snap.planes.iter().enumerate() {
            if is_unset(plane) {
                continue;
            }
            let sub = u8::from(selected == Some(i));
            let priority = HitPriority::new(HitLevel::Handle, sub);
            let torn = snap.torn.get(i).copied().flatten().is_some();
            if !torn {
                let sense = if edit { Sense::DRAG | Sense::TAKE_FOCUS } else { Sense::TAKE_FOCUS };
                for (c, corner) in plane.corners.iter().enumerate() {
                    let rect = Rect::from_center_size(space.view.to_screen(craft_behavior::kurbo::Point::new(corner[0], corner[1])), reach);
                    cx.part(CORNER.at(key_of(i, c)), HitShape::Rect(rect))
                        .sense(sense)
                        .priority(priority)
                        .cursor(CursorIcon::Move)
                        .focusable()
                        .nudges();
                }
            }
            if snap.tool != VanishingTool::Stamp && plane.homography().is_some() {
                for (e, node) in plane.edge_nodes().iter().enumerate() {
                    let rect = Rect::from_center_size(space.view.to_screen(craft_behavior::kurbo::Point::new(node[0], node[1])), reach);
                    cx.part(EDGE.at(key_of(i, e)), HitShape::Rect(rect))
                        .sense(Sense::DRAG)
                        .priority(priority)
                        .cursor(CursorIcon::Move)
                        .keyboard_via(CORNER);
                }
            }
        }
        cx.roving(Roving::new(NavShape::Flat).page(Some(1)));
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let words = cx.words::<VanishingPointWords>();
        let snap = self.snapshot();
        let dead = space.view.doc_len(cx.style.slop);
        let doc = |at: egui::Pos2| {
            let p = space.view.to_doc(at);
            [p.x, p.y]
        };
        let mut presses: Vec<(Node, PressState)> = Vec::new();
        for i in 0..snap.planes.len() {
            for index in 0..4 {
                for (node, key) in [(Node::Corner(i, index), CORNER), (Node::Edge(i, index), EDGE)] {
                    if let Some(result) = cx.part_if_declared(key.at(key_of(i, index)))
                        && result.press.phase.is_press()
                    {
                        presses.push((node, result.press));
                    }
                }
            }
        }
        let focused = cx.active_part().filter(|part| part.key == CORNER).and_then(|part| part.index).map(|index| index as usize);
        let nudge = focused.and_then(|_| cx.intents().take_nudge());
        self.pass.with(&mut |work| {
            for (node, press) in &presses {
                match press.phase {
                    PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                        let p = doc(start.origin);
                        // The control is no tool: the host reports no gesture for its presses.
                        work.gesture = Some(next_gesture_id(work.ctx));
                        match *node {
                            Node::Corner(plane, corner) => work.corner_down(plane, corner, p, &words),
                            Node::Edge(plane, edge) => work.edge_down(plane, edge, p, press.mods, &words),
                        }
                        work.gesture = None;
                    }
                    PressPhase::DragStart { start, pos, .. } | PressPhase::Drag { start, pos, .. } if start.button == PointerButton::Primary => {
                        work.drag_to(doc(pos), dead);
                    }
                    PressPhase::Release { start, pos, .. } if start.button == PointerButton::Primary => work.release_at(doc(pos), dead),
                    PressPhase::Cancel { .. } => work.editor.cancel(work.session, CancelReason::Escape),
                    _ => {}
                }
            }
            // A corner that has the keyboard selects its plane and takes the arrows.
            if let Some(key) = focused {
                let (plane, corner) = (key / 4, key % 4);
                if work.session.planes.get(plane).is_some() && work.editor.drag.is_none() {
                    work.editor.selected = Some(plane);
                }
                if let Some((direction, mods)) = nudge {
                    let step = if mods.shift { NUDGE_FAR } else { NUDGE };
                    let by = space.view.unmap_vec(direction) * step;
                    work.gesture = Some(next_gesture_id(work.ctx));
                    work.nudge_corner(plane, corner, [by.x, by.y], &words);
                    work.gesture = None;
                }
            }
        });
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let snap = self.snapshot();
        let draw = ui.draw();
        let fill = &cx.style.handle.fill;
        let node = |plane: usize, index: usize, corner: bool| {
            let part = if corner { CORNER } else { EDGE }.at(key_of(plane, index));
            // A node with no part (a torn plane's corner, any edge node under the Stamp) takes the control's own state.
            if cx.part_if_declared(part).is_some() { cx.resolve_part(part, fill) } else { cx.resolve(fill) }
        };
        draw::paint(&draw, &space.view, &snap, cx.style, &node);
    }

    /// A group named by the words, and one node per corner named by its plane and place, valued by its position.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<VanishingPointWords>();
        access.name(Name::Text(words.parts.as_str().to_owned()));
        let Some(space) = canvas_space(&cx.env) else { return };
        let snap = self.snapshot();
        let (rulers, units) = (space.rulers(), cx.env.units());
        let places = [&words.top_left, &words.top_right, &words.bottom_right, &words.bottom_left];
        for (i, plane) in snap.planes.iter().enumerate() {
            if is_unset(plane) || snap.torn.get(i).copied().flatten().is_some() {
                continue;
            }
            let n = (i + 1).to_string();
            let note = match snap.checks[i] {
                crate::ui::vanishing_point::PlaneCheck::Questionable => Some(words.questionable.as_str().to_owned()),
                crate::ui::vanishing_point::PlaneCheck::Invalid => Some(words.invalid.as_str().to_owned()),
                _ => None,
            };
            for (c, (corner, place)) in plane.corners.iter().zip(places).enumerate() {
                let value = format!(
                    "{}, {}",
                    rulers.x.format_value(&units, corner[0], rulers.zero.x),
                    rulers.y.format_value(&units, corner[1], rulers.zero.y)
                );
                let node = access.node(CORNER.at(key_of(i, c)), Role::GRAPHICS_OBJECT);
                node.name = Name::Text(words.corner_part.fill(&[("n", n.as_str()), ("corner", place.as_str())]));
                node.description = Some(match &note {
                    Some(note) => format!("{value}. {note}"),
                    None => value,
                });
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> VanishingPlanesOutput {
        let selected = self.snapshot().selected_plane();
        VanishingPlanesOutput { response: cx.response.clone(), selected }
    }
}
