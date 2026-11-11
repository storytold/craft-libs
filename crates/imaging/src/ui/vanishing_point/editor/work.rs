//! What the tool and the control do to the editor's state: the shared logic of one pass.

use std::sync::Arc;

use craft_behavior::geometry::quad::Homography;
use craft_behavior::kurbo::{Point, Rect};
use craft_behavior::{EditEvent, GestureId, Mixed};
use craft_keymap::Mods;

use super::{DragStart, PlaneEdit, StampStroke, VanishingDrag, VanishingPointEditor, VanishingPointSession, VanishingPointSettings};
use super::{VanishingPointEvent, VanishingTool};
use crate::ui::vanishing_point::VanishingPointSource;
use crate::ui::vanishing_point::planes::{MAX_PLANES, PlaneCheck, PlaneEdge, TornPlane, VanishingPlane, VanishingScene, fit_torn, slide_edge};
use crate::ui::vanishing_point::words::VanishingPointWords;

/// A stroke's dabs lie a tenth of the diameter apart, at least one document pixel (`stamp_to`).
const DAB_SPACING: f64 = 0.1;
const MIN_DAB_STEP: f64 = 1.0;

/// The paste's mesh is this many cells a side.
pub(in crate::ui::vanishing_point) const PASTE_CELLS: u32 = 24;

/// The editor, the session and the app's side of one pass.
pub(in crate::ui::vanishing_point) struct Work<'w> {
    pub ctx: &'w egui::Context,
    /// The id of the gesture the host runs for the press in hand; the first history entry the press opens takes it.
    pub gesture: Option<GestureId>,
    pub editor: &'w mut VanishingPointEditor,
    pub session: &'w mut VanishingPointSession,
    pub scene: &'w dyn VanishingScene,
    pub settings: &'w VanishingPointSettings,
    pub source: &'w dyn VanishingPointSource,
}

/// What a background press did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui::vanishing_point) enum Pressed {
    /// A drag began: the tool owns the gesture.
    Claimed,
    /// A click handled whole.
    Done,
}

fn at(p: [f64; 2]) -> Point {
    Point::new(p[0], p[1])
}

/// A torn plane the scene could not rebuild has all-zero corners.
pub(in crate::ui::vanishing_point) fn is_unset(plane: &VanishingPlane) -> bool {
    plane.corners.iter().flatten().all(|v| *v == 0.0)
}

fn label(text: &str) -> Arc<str> {
    Arc::from(text)
}

/// The plane the paste lands on while Paste is on and the app has an image: the selected plane, else the first. One
/// rule for the tool and the drawing.
pub(in crate::ui::vanishing_point) fn paste_plane(
    settings: &VanishingPointSettings,
    source: &dyn VanishingPointSource,
    planes: usize,
    selected: Option<usize>,
) -> Option<usize> {
    if !settings.paste || source.paste().is_none() {
        return None;
    }
    selected.filter(|i| *i < planes).or((planes > 0).then_some(0))
}

/// The pasted image's size in plane coordinates: `width` of the plane's width, the height from the image's aspect in the
/// plane's metric (`lu`, `lv` its sides' lengths). One rule for the tool and the drawing.
pub(in crate::ui::vanishing_point) fn paste_extent(image: (f64, f64), width: f64, lu: f64, lv: f64) -> [f64; 2] {
    let (w, h) = image;
    [width, width * lu / lv * h / w.max(1.0)]
}

/// Plane `index` closed by `corners`: the scene normalises it, it is selected, Edit Plane takes over and a `Set` is
/// reported. `false` past [`MAX_PLANES`].
pub(in crate::ui::vanishing_point) fn close_plane(
    editor: &mut VanishingPointEditor,
    session: &mut VanishingPointSession,
    scene: &dyn VanishingScene,
    corners: [[f64; 2]; 4],
    step: &str,
) -> bool {
    if session.planes.len() >= MAX_PLANES {
        return false;
    }
    let before = PlaneEdit::new(session.planes.clone(), session.torn.clone());
    editor.pending.clear();
    let plane = scene.normalized(VanishingPlane { corners });
    session.torn.resize(session.planes.len(), None);
    session.planes.push(plane);
    session.torn.push(None);
    editor.selected = Some(session.planes.len() - 1);
    session.torn.resize(session.planes.len(), None);
    session.planes = scene.resolve(&session.planes, &session.torn);
    // Photoshop hands over to Edit Plane once a plane is closed.
    editor.tool = VanishingTool::EditPlane;
    let after = PlaneEdit::new(session.planes.clone(), session.torn.clone());
    editor.events.push(VanishingPointEvent::Planes(EditEvent::set(Mixed::Same(before), after, label(step), None)));
    true
}

impl Work<'_> {
    pub fn plane_edit(&self) -> PlaneEdit {
        PlaneEdit::new(self.session.planes.clone(), self.session.torn.clone())
    }

    /// Rebuilds the torn planes from their parents after an edit.
    fn resolve(&mut self) {
        self.session.torn.resize(self.session.planes.len(), None);
        if !self.session.planes.is_empty() {
            self.session.planes = self.scene.resolve(&self.session.planes, &self.session.torn);
        }
    }

    pub fn on_plane(&self, p: [f64; 2]) -> bool {
        self.scene.plane_at(&self.session.planes, p).is_some()
    }

    fn frame(&self, index: usize) -> Option<(Homography, f64, f64)> {
        let (h, lu, lv) = self.scene.frame(&self.session.planes, index)?;
        (lu.is_finite() && lv.is_finite() && lu > 1e-12 && lv > 1e-12).then_some((h, lu, lv))
    }

    fn paste_plane(&self) -> Option<usize> {
        paste_plane(self.settings, self.source, self.session.planes.len(), self.editor.selected)
    }

    fn image_size(&self) -> Option<(f64, f64)> {
        let size = self.source.paste()?.size();
        Some((f64::from(size.width), f64::from(size.height)))
    }

    /// The pasted image's size in plane coordinates (it keeps its metric aspect).
    fn paste_extent(&self, lu: f64, lv: f64) -> Option<[f64; 2]> {
        Some(paste_extent(self.image_size()?, self.session.paste.width, lu, lv))
    }

    /// The plane the paste lies on: its frame (the unit square onto the plane) and the paste's rectangle in plane
    /// coordinates; `None` without a usable plane.
    pub fn paste_placement(&self) -> Option<(Homography, Rect)> {
        let index = self.paste_plane()?;
        let plane = self.session.planes.get(index)?;
        if is_unset(plane) || self.scene.check(&self.session.planes, index) == PlaneCheck::Invalid {
            return None;
        }
        let (h, lu, lv) = self.frame(index)?;
        let extent = self.paste_extent(lu, lv)?;
        let at = self.session.paste.at;
        Some((h, Rect::new(at[0], at[1], at[0] + extent[0], at[1] + extent[1])))
    }

    /// The pointer's offset from the paste's top-left in plane coordinates, when `p` is on the pasted image.
    fn on_paste(&self, p: [f64; 2]) -> Option<[f64; 2]> {
        let i = self.paste_plane()?;
        let (h, lu, lv) = self.frame(i)?;
        let ext = self.paste_extent(lu, lv)?;
        let q = h.inverse()?.apply(at(p));
        let (a, b) = (self.session.paste.at, [self.session.paste.at[0] + ext[0], self.session.paste.at[1] + ext[1]]);
        ((a[0]..=b[0]).contains(&q.x) && (a[1]..=b[1]).contains(&q.y)).then_some([a[0] - q.x, a[1] - q.y])
    }

    fn begin_drag(&mut self, drag: VanishingDrag, before: DragStart, event: impl FnOnce(craft_behavior::GestureId) -> VanishingPointEvent) {
        let Some(id) = self.gesture.take() else { return };
        self.editor.drag = Some((id, drag));
        self.editor.before = Some(before);
        self.editor.events.push(event(id));
    }

    fn begin_plane_drag(&mut self, drag: VanishingDrag, words: &VanishingPointWords) {
        let before = self.plane_edit();
        let begun = Mixed::Same(before.clone());
        let name = label(words.edit_planes_step.as_str());
        self.begin_drag(drag, DragStart::Planes(before), |id| VanishingPointEvent::Planes(EditEvent::begin(id, begun, name, None)));
    }

    /// Create Plane's click: the fourth corner makes the plane.
    pub fn add_corner(&mut self, p: [f64; 2], words: &VanishingPointWords) {
        if self.session.planes.len() >= MAX_PLANES || self.gesture.is_none() {
            return;
        }
        self.editor.pending.push(p);
        if let Ok(corners) = <[[f64; 2]; 4]>::try_from(self.editor.pending.as_slice()) {
            close_plane(self.editor, self.session, self.scene, corners, words.edit_planes_step.as_str());
        }
    }

    /// Command-press on an edge node: a perpendicular plane, half as deep, whose far edge the drag moves.
    fn tear_off(&mut self, from: usize, edge: usize, p: [f64; 2], words: &VanishingPointWords) {
        if self.session.planes.len() >= MAX_PLANES || self.gesture.is_none() {
            return;
        }
        let Some(&edge) = PlaneEdge::ALL.get(edge) else { return };
        let before = self.plane_edit();
        self.session.torn.resize(self.session.planes.len(), None);
        self.session.planes.push(VanishingPlane { corners: [[0.0; 2]; 4] });
        self.session.torn.push(Some(TornPlane { from, edge, angle: 90.0, depth: 0.5 }));
        let plane = self.session.planes.len() - 1;
        self.editor.selected = Some(plane);
        self.resolve();
        // As after closing a plane, Edit Plane takes over.
        self.editor.tool = VanishingTool::EditPlane;
        self.editor.pending.clear();
        let name = label(words.edit_planes_step.as_str());
        let begun = Mixed::Same(before.clone());
        self.begin_drag(VanishingDrag::Torn { plane, rotate: false, start: p, moved: false }, DragStart::Planes(before), |id| {
            VanishingPointEvent::Planes(EditEvent::begin(id, begun, name, None))
        });
    }

    /// A press on corner `corner` of plane `plane`, with Edit Plane picked.
    pub fn corner_down(&mut self, plane: usize, corner: usize, p: [f64; 2], words: &VanishingPointWords) {
        let Some(c) = self.session.planes.get(plane).and_then(|pl| pl.corners.get(corner)).copied().filter(|_| self.gesture.is_some()) else {
            return;
        };
        self.editor.selected = Some(plane);
        self.begin_plane_drag(VanishingDrag::Corner { plane, corner, offset: [c[0] - p[0], c[1] - p[1]] }, words);
    }

    /// A press on edge node `edge` of plane `plane`: Command tears off a plane; then Edit Plane slides the edge (or
    /// drags a torn plane's far edge) and Create Plane takes the press as a corner.
    pub fn edge_down(&mut self, plane: usize, edge: usize, p: [f64; 2], mods: Mods, words: &VanishingPointWords) {
        if self.gesture.is_none() {
            return;
        }
        if mods.cmd {
            self.tear_off(plane, edge, p, words);
            return;
        }
        if self.editor.tool != VanishingTool::EditPlane {
            self.add_corner(p, words);
            return;
        }
        if let Some(t) = self.session.torn.get(plane).copied().flatten() {
            // A torn plane follows its parent: only its far edge node drags.
            if PlaneEdge::ALL.iter().position(|e| *e == t.edge) == Some(edge) {
                self.editor.selected = Some(plane);
                self.begin_plane_drag(VanishingDrag::Torn { plane, rotate: mods.alt, start: p, moved: false }, words);
            } else {
                self.editor.selected = self.scene.plane_at(&self.session.planes, p);
            }
            return;
        }
        let Some(at_press) = self.session.planes.get(plane).and_then(VanishingPlane::homography) else { return };
        self.editor.selected = Some(plane);
        self.begin_plane_drag(VanishingDrag::Edge { plane, edge, at_press }, words);
    }

    /// A press on the canvas away from every node. `None`: nothing handled it.
    pub fn background_down(&mut self, p: [f64; 2], mods: Mods, words: &VanishingPointWords) -> Option<Pressed> {
        // pc: crates/ui-egui/src/dialogs/vp_ui.rs:658 `press`, :353 `add_corner`, :469 `stamp_press`.
        if !p.iter().all(|v| v.is_finite()) || self.gesture.is_none() {
            return None;
        }
        self.editor.pointer = Some(p);
        self.editor.status = None;
        match self.editor.tool {
            VanishingTool::Stamp => match self.stamp_press(p, mods, words) {
                Ok(pressed) => Some(pressed),
                Err(message) => {
                    self.editor.status = Some(message);
                    Some(Pressed::Done)
                }
            },
            VanishingTool::CreatePlane => {
                self.add_corner(p, words);
                Some(Pressed::Done)
            }
            VanishingTool::EditPlane => {
                if let Some(offset) = self.on_paste(p) {
                    let before = self.session.paste;
                    let name = label(words.paste_step.as_str());
                    self.begin_drag(VanishingDrag::Paste { offset }, DragStart::Paste(before), |id| {
                        VanishingPointEvent::Paste(EditEvent::begin(id, Mixed::Same(before), name, None))
                    });
                    return Some(Pressed::Claimed);
                }
                self.editor.selected = self.scene.plane_at(&self.session.planes, p);
                Some(Pressed::Done)
            }
        }
    }

    /// Stamp's press: Alt sets the source, otherwise a stroke starts. `Err` is the message of a refused press.
    fn stamp_press(&mut self, p: [f64; 2], mods: Mods, words: &VanishingPointWords) -> Result<Pressed, String> {
        if mods.alt {
            if !self.on_plane(p) {
                return Err(words.stamp_no_source.as_str().to_owned());
            }
            self.session.source = Some(p);
            self.session.aligned_from = None;
            return Ok(Pressed::Done);
        }
        let Some(source) = self.session.source else { return Err(words.stamp_no_source.as_str().to_owned()) };
        if !self.on_plane(p) {
            return Ok(Pressed::Done);
        }
        let dabs: usize = self.session.strokes.iter().map(|s| s.points.len()).sum();
        if self.session.strokes.len() >= self.editor.limits.strokes || dabs >= self.editor.limits.dabs {
            return Err(words.stamp_too_many.as_str().to_owned());
        }
        let aligned = self.session.aligned_from.filter(|_| self.settings.stamp.aligned);
        let src = match aligned {
            Some((s0, a0)) => self.scene.clone_source(&self.session.planes, s0, a0, p).filter(|s| self.on_plane(*s)).unwrap_or(source),
            None => source,
        };
        let before = self.session.aligned_from;
        if before.is_none() {
            self.session.aligned_from = Some((src, p));
        }
        let o = self.settings.stamp;
        let stroke = StampStroke::new(src, vec![p], o.diameter, o.hardness_share(), o.opacity_share());
        self.session.strokes.push(stroke.clone());
        let name = label(words.stamp_step.as_str());
        let first = stroke.clone();
        self.begin_drag(VanishingDrag::Stamp, DragStart::Stamp(before, stroke), |id| {
            VanishingPointEvent::Stamp(EditEvent::begin(id, Mixed::Same(first), name, None))
        });
        Ok(Pressed::Claimed)
    }

    /// Extends the running stroke to `p` with dabs a tenth of the brush apart.
    fn stamp_to(&mut self, p: [f64; 2]) -> bool {
        let used: usize = self.session.strokes.iter().map(|s| s.points.len()).sum();
        let room = self.editor.limits.dabs.saturating_sub(used);
        let Some(stroke) = self.session.strokes.last_mut() else { return false };
        let Some(last) = stroke.points.last().copied() else { return false };
        let step = (stroke.size * DAB_SPACING).max(MIN_DAB_STEP);
        let d = (p[0] - last[0]).hypot(p[1] - last[1]);
        if !d.is_finite() || d < step {
            return false;
        }
        // The count is a whole number of dabs: truncation is the intent.
        let n = ((d / step) as usize).min(room);
        stroke.points.extend((1..=n).map(|k| {
            let t = k as f64 * step / d;
            [last[0] + (p[0] - last[0]) * t, last[1] + (p[1] - last[1]) * t]
        }));
        n > 0
    }

    /// Moves what the drag holds to `p` and reports an Update when that changed something. `dead` is the
    /// click-versus-drag distance of a torn plane's far edge node, in document pixels.
    pub fn drag_to(&mut self, p: [f64; 2], dead: f64) {
        if !p.iter().all(|v| v.is_finite()) {
            return;
        }
        self.editor.pointer = Some(p);
        let Some((id, drag)) = self.editor.drag.clone() else { return };
        let event = match drag {
            VanishingDrag::Corner { plane, corner, offset } => {
                let to = [p[0] + offset[0], p[1] + offset[1]];
                match self.session.planes.get_mut(plane).and_then(|pl| pl.corners.get_mut(corner)) {
                    Some(c) if *c != to => {
                        *c = to;
                        self.resolve();
                        Some(VanishingPointEvent::Planes(EditEvent::update(id, self.plane_edit())))
                    }
                    _ => None,
                }
            }
            VanishingDrag::Torn { plane, rotate, start, moved } => {
                // A click without a drag leaves the plane as it is.
                let moved = moved || (p[0] - start[0]).hypot(p[1] - start[1]) > dead;
                if let Some((_, VanishingDrag::Torn { moved: m, .. })) = &mut self.editor.drag {
                    *m = moved;
                }
                let fit = moved.then(|| fit_torn(self.scene, &self.session.planes, &self.session.torn, plane, rotate, p)).flatten();
                match fit {
                    Some(t) if self.session.torn.get(plane).copied().flatten() != Some(t) => {
                        self.session.torn[plane] = Some(t);
                        self.resolve();
                        Some(VanishingPointEvent::Planes(EditEvent::update(id, self.plane_edit())))
                    }
                    _ => None,
                }
            }
            VanishingDrag::Edge { plane, edge, at_press } => match slide_edge(self.scene, &at_press, edge, p) {
                Some(slid) if self.session.planes.get(plane).is_some_and(|pl| pl.corners != slid.corners) => {
                    self.session.planes[plane].corners = slid.corners;
                    self.resolve();
                    Some(VanishingPointEvent::Planes(EditEvent::update(id, self.plane_edit())))
                }
                _ => None,
            },
            VanishingDrag::Paste { offset } => {
                let inv = self.paste_plane().and_then(|i| self.session.planes.get(i)).and_then(VanishingPlane::homography).and_then(|h| h.inverse());
                inv.and_then(|inv| {
                    let q = inv.apply(at(p));
                    let to = [(q.x + offset[0]).clamp(-1.0, 2.0), (q.y + offset[1]).clamp(-1.0, 2.0)];
                    (to.iter().all(|v| v.is_finite()) && to != self.session.paste.at).then(|| {
                        self.session.paste.at = to;
                        VanishingPointEvent::Paste(EditEvent::update(id, self.session.paste))
                    })
                })
            }
            VanishingDrag::Stamp => {
                if self.stamp_to(p) {
                    self.session.strokes.last().map(|s| VanishingPointEvent::Stamp(EditEvent::update(id, s.clone())))
                } else {
                    None
                }
            }
        };
        self.editor.events.extend(event);
    }

    /// The button came up at `p`: the drag commits.
    pub fn release_at(&mut self, p: [f64; 2], dead: f64) {
        // pc: crates/ui-egui/src/dialogs/vp_ui.rs:1025 `pointer` (Up ends the drag; the history settles).
        self.drag_to(p, dead);
        let Some((id, drag)) = self.editor.drag.take() else { return };
        let event = match (drag, self.editor.before.take()) {
            (VanishingDrag::Stamp, Some(DragStart::Stamp(_, first))) => {
                self.session.strokes.last().map(|s| VanishingPointEvent::Stamp(EditEvent::commit(id, Mixed::Same(first), s.clone())))
            }
            (VanishingDrag::Paste { .. }, Some(DragStart::Paste(before))) => {
                Some(VanishingPointEvent::Paste(EditEvent::commit(id, Mixed::Same(before), self.session.paste)))
            }
            (_, Some(DragStart::Planes(before))) => Some(VanishingPointEvent::Planes(EditEvent::commit(id, Mixed::Same(before), self.plane_edit()))),
            _ => None,
        };
        self.editor.events.extend(event);
    }

    /// An arrow key on a focused corner: moves it by `by` document pixels and reports a `Set`. A torn plane's corners
    /// follow its parent and do not move.
    pub fn nudge_corner(&mut self, plane: usize, corner: usize, by: [f64; 2], words: &VanishingPointWords) {
        if self.gesture.is_none() || self.editor.drag.is_some() || self.session.torn.get(plane).copied().flatten().is_some() {
            return;
        }
        let before = self.plane_edit();
        let Some(c) = self.session.planes.get_mut(plane).filter(|p| !is_unset(p)).and_then(|p| p.corners.get_mut(corner)) else { return };
        *c = [c[0] + by[0], c[1] + by[1]];
        self.editor.selected = Some(plane);
        self.resolve();
        let after = self.plane_edit();
        self.editor.events.push(VanishingPointEvent::Planes(EditEvent::set(
            Mixed::Same(before),
            after,
            label(words.edit_planes_step.as_str()),
            None,
        )));
    }
}
