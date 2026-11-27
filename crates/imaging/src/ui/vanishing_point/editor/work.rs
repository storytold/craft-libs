//! What the tool and the control do to the editor's state: the shared logic of one pass.

use std::sync::Arc;

use craft_behavior::geometry::quad::Homography;
use craft_behavior::kurbo::Point;
use craft_behavior::{EditEvent, GestureId, Mixed};
use craft_keymap::Mods;
use craft_theme::variant::Tone;
use craft_ui::bars::StatusMessage;
use craft_ui::runtime::env::Label;

use super::{DragStart, PasteImage, PlaneEdit, StampStroke, VanishingDrag, VanishingPointEditor, VanishingPointSession, VanishingPointSettings};
use super::{VanishingPointEvent, VanishingTool};
use crate::ui::vanishing_point::VanishingPointSource;
use crate::ui::vanishing_point::planes::{MAX_PLANES, PlaneCheck, PlaneEdge, TornPlane, VanishingPlane, VanishingScene, fit_torn, slide_edge};
use crate::ui::vanishing_point::words::VanishingPointWords;

/// Transform turns the selected paste from a press this many corner reaches from a corner, outside the paste.
const ROTATE_REACH: f64 = 4.0;

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
    /// How near a Transform corner a press must land, in document pixels (the tool sets it from the view).
    pub reach: f64,
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

/// The pasted image's size in plane coordinates: `width` of the plane's width, the height from the image's aspect in the
/// plane's metric (`lu`, `lv` its sides' lengths). One rule for the tool and the drawing.
pub(in crate::ui::vanishing_point) fn paste_extent(image: (f64, f64), width: f64, lu: f64, lv: f64) -> [f64; 2] {
    let (w, h) = image;
    [width, width * lu / lv * h / w.max(1.0)]
}

/// The paste's width range, as a fraction of its plane's width (the bar's 1..=1000 percent).
const PASTE_WIDTHS: std::ops::RangeInclusive<f64> = 0.01..=10.0;

/// A new paste's width as a fraction of its plane's width, and the most of the plane's height it may take.
const NEW_PASTE_WIDTH: f64 = 0.5;
const NEW_PASTE_HEIGHT: f64 = 0.8;

/// The map from a paste's image (its unit square) into its plane's unit square: the unturned box from `at`, `width` of
/// the plane wide and as tall as the image's aspect makes it in the plane's metric (`lu`, `lv` the plane's sides),
/// mirrored and turned about its centre in that metric. Affine. The engine places the pixels by the same rule.
pub(in crate::ui::vanishing_point) fn paste_affine(paste: &PasteImage, image: (f64, f64), lu: f64, lv: f64) -> Homography {
    let ext = paste_extent(image, paste.width, lu, lv);
    let (w, h) = (ext[0] * lu, ext[1] * lv);
    let (fx, fy) = (if paste.flip_h { -1.0 } else { 1.0 }, if paste.flip_v { -1.0 } else { 1.0 });
    let (sin, cos) = paste.angle.to_radians().sin_cos();
    let centre = [paste.at[0] + ext[0] / 2.0, paste.at[1] + ext[1] / 2.0];
    // (s, t) in the image → metric offset from the centre ((s - ½)·w·fx, (t - ½)·h·fy), turned, back to plane units.
    let (a, b) = (w * fx * cos / lu, -h * fy * sin / lu);
    let (d, e) = (w * fx * sin / lv, h * fy * cos / lv);
    Homography::from_rows([[a, b, centre[0] - (a + b) / 2.0], [d, e, centre[1] - (d + e) / 2.0], [0.0, 0.0, 1.0]])
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

    fn image_size(&self) -> Option<(f64, f64)> {
        let size = self.source.paste()?.size();
        Some((f64::from(size.width), f64::from(size.height)))
    }

    /// A usable plane for pastes: set, not invalid, with usable metric sides.
    fn paste_frame(&self, plane: usize) -> Option<(Homography, f64, f64)> {
        let pl = self.session.planes.get(plane)?;
        if is_unset(pl) || self.scene.check(&self.session.planes, plane) == PlaneCheck::Invalid {
            return None;
        }
        self.frame(plane)
    }

    /// Paste `index`'s plane frame (the unit square onto the document), its map from the image into the plane's unit
    /// square, and the plane's metric sides; `None` without an image or a usable plane.
    fn paste_maps(&self, index: usize) -> Option<(Homography, Homography, f64, f64)> {
        let paste = self.session.pastes.get(index)?;
        let (h, lu, lv) = self.paste_frame(paste.plane)?;
        Some((h, paste_affine(paste, self.image_size()?, lu, lv), lu, lv))
    }

    /// Where paste `index` lies: its plane's frame and the map from the image into the plane's unit square.
    pub fn paste_placement(&self, index: usize) -> Option<(Homography, Homography)> {
        self.paste_maps(index).map(|(h, a, _, _)| (h, a))
    }

    /// Paste `index`'s corners in document pixels, clockwise from the image's top-left.
    pub fn paste_quad(&self, index: usize) -> Option<[[f64; 2]; 4]> {
        let (h, a, _, _) = self.paste_maps(index)?;
        let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].map(|(s, t)| {
            let p = h.apply(a.apply(Point::new(s, t)));
            [p.x, p.y]
        });
        corners.iter().flatten().all(|v| v.is_finite()).then_some(corners)
    }

    /// The topmost paste under document point `p` and the image point (its unit square) there.
    fn paste_at(&self, p: [f64; 2]) -> Option<(usize, [f64; 2])> {
        (0..self.session.pastes.len()).rev().find_map(|i| {
            let (h, a, _, _) = self.paste_maps(i)?;
            let plane = h.inverse()?.apply(at(p));
            if !(0.0..=1.0).contains(&plane.x) || !(0.0..=1.0).contains(&plane.y) {
                return None;
            }
            let q = a.inverse()?.apply(plane);
            ((0.0..=1.0).contains(&q.x) && (0.0..=1.0).contains(&q.y)).then_some((i, [q.x, q.y]))
        })
    }

    /// Paste `index`'s centre and the pointer `p`, both in its plane's metric.
    fn paste_metric(&self, index: usize, p: [f64; 2]) -> Option<([f64; 2], [f64; 2])> {
        let (h, a, lu, lv) = self.paste_maps(index)?;
        let c = a.apply(Point::new(0.5, 0.5));
        let q = h.inverse()?.apply(at(p));
        let out = ([c.x * lu, c.y * lv], [q.x * lu, q.y * lv]);
        out.0.iter().chain(&out.1).all(|v| v.is_finite()).then_some(out)
    }

    /// ⌘V: the clipboard image once more, centred on the selected plane (else the first usable one), selected, with
    /// Transform picked; a `Paste` `Set`.
    pub fn paste_new(&mut self, words: &VanishingPointWords) -> Result<(), Box<StatusMessage>> {
        let refused = |text: &Label| Box::new(StatusMessage::new(text.clone()).tone(Tone::Warn));
        if self.editor.busy() {
            return Err(refused(&words.paste_busy));
        }
        let image = self.image_size().ok_or_else(|| refused(&words.paste_no_image))?;
        let plane = self
            .editor
            .selected
            .filter(|i| self.paste_frame(*i).is_some())
            .or_else(|| (0..self.session.planes.len()).find(|i| self.paste_frame(*i).is_some()))
            .ok_or_else(|| refused(&words.paste_no_plane))?;
        let (_, lu, lv) = self.paste_frame(plane).ok_or_else(|| refused(&words.paste_no_plane))?;
        let mut width = NEW_PASTE_WIDTH;
        let tall = paste_extent(image, width, lu, lv)[1];
        if tall > NEW_PASTE_HEIGHT {
            width *= NEW_PASTE_HEIGHT / tall;
        }
        let ext = paste_extent(image, width, lu, lv);
        // It lands over the strokes made so far; later strokes paint over it.
        let paste = PasteImage::new([0.5 - ext[0] / 2.0, 0.5 - ext[1] / 2.0], width).on(plane).after(self.session.strokes.len());
        let before = self.session.pastes.clone();
        self.session.pastes.push(paste);
        self.editor.selected_paste = Some(self.session.pastes.len() - 1);
        self.editor.selected = Some(plane);
        self.editor.set_tool(VanishingTool::Transform);
        let after = self.session.pastes.clone();
        self.editor.events.push(VanishingPointEvent::Paste(EditEvent::set(Mixed::Same(before), after, label(words.paste_new_step.as_str()), None)));
        Ok(())
    }

    /// Starts a drag of the pastes (move, scale or turn).
    fn begin_paste_drag(&mut self, drag: VanishingDrag, step: &str) -> Pressed {
        let before = self.session.pastes.clone();
        let begun = Mixed::Same(before.clone());
        let name = label(step);
        self.begin_drag(drag, DragStart::Pastes(before), |id| VanishingPointEvent::Paste(EditEvent::begin(id, begun, name, None)));
        Pressed::Claimed
    }

    /// A press on paste `index` at image point `grab`: selects it and its plane, and moves it.
    fn press_paste(&mut self, index: usize, grab: [f64; 2], words: &VanishingPointWords) -> Pressed {
        self.editor.selected_paste = Some(index);
        self.editor.selected = self.session.pastes.get(index).map(|p| p.plane);
        self.begin_paste_drag(VanishingDrag::Paste { index, grab }, words.paste_step.as_str())
    }

    /// Transform's press: a corner of the selected paste scales it, a press on a paste moves it, one near the selected
    /// paste's corners turns it; elsewhere the paste is let go and the plane under the press selected.
    fn transform_press(&mut self, p: [f64; 2], words: &VanishingPointWords) -> Pressed {
        if let Some(i) = self.editor.selected_paste
            && let Some(quad) = self.paste_quad(i)
            && let Some((c, m)) = self.paste_metric(i, p)
        {
            let near = |r: f64| quad.iter().any(|q| (q[0] - p[0]).hypot(q[1] - p[1]) <= r);
            let paste = self.session.pastes[i];
            if near(self.reach) {
                let from = (m[0] - c[0]).hypot(m[1] - c[1]);
                if from > 1e-12 {
                    return self.begin_paste_drag(VanishingDrag::Scale { index: i, from, width: paste.width }, words.transform_step.as_str());
                }
            } else if self.paste_at(p).is_none() && near(self.reach * ROTATE_REACH) {
                let from = (m[1] - c[1]).atan2(m[0] - c[0]);
                return self.begin_paste_drag(VanishingDrag::Rotate { index: i, from, angle: paste.angle }, words.transform_step.as_str());
            }
        }
        if let Some((i, grab)) = self.paste_at(p) {
            return self.press_paste(i, grab, words);
        }
        self.editor.selected_paste = None;
        self.editor.selected = self.scene.plane_at(&self.session.planes, p);
        Pressed::Done
    }

    /// Moves what a paste drag holds to `p`; whether the paste changed.
    fn paste_drag_to(&mut self, drag: &VanishingDrag, p: [f64; 2]) -> bool {
        let next = match *drag {
            VanishingDrag::Paste { index, grab } => self.paste_moved(index, grab, p),
            VanishingDrag::Scale { index, from, width } => self.paste_metric(index, p).and_then(|(c, m)| {
                let mut paste = *self.session.pastes.get(index)?;
                let (_, lu, lv) = self.paste_frame(paste.plane)?;
                let image = self.image_size()?;
                let minimum = width.min(*PASTE_WIDTHS.start());
                let width = (width * (m[0] - c[0]).hypot(m[1] - c[1]) / from).clamp(minimum, *PASTE_WIDTHS.end());
                let ext = paste_extent(image, width, lu, lv);
                paste.width = width;
                paste.at = [c[0] / lu - ext[0] / 2.0, c[1] / lv - ext[1] / 2.0];
                Some((index, paste))
            }),
            VanishingDrag::Rotate { index, from, angle } => self.paste_metric(index, p).and_then(|(c, m)| {
                let mut paste = *self.session.pastes.get(index)?;
                let turn = (m[1] - c[1]).atan2(m[0] - c[0]) - from;
                paste.angle = (angle + turn.to_degrees() + 180.0).rem_euclid(360.0) - 180.0;
                Some((index, paste))
            }),
            _ => None,
        };
        match next {
            Some((index, paste)) if paste.at.iter().all(|v| v.is_finite()) && self.session.pastes.get(index) != Some(&paste) => {
                self.session.pastes[index] = paste;
                true
            }
            _ => false,
        }
    }

    /// Paste `index` moved so its image point `grab` lies under `p`: on the usable plane under `p` (Photoshop's paste
    /// follows the pointer onto another plane, keeping its size in the planes' shared metric), else on its own.
    fn paste_moved(&self, index: usize, grab: [f64; 2], p: [f64; 2]) -> Option<(usize, PasteImage)> {
        let mut paste = *self.session.pastes.get(index)?;
        let (_, lu, _) = self.paste_frame(paste.plane)?;
        let image = self.image_size()?;
        let under = self.scene.plane_at(&self.session.planes, p).filter(|i| self.paste_frame(*i).is_some());
        if let Some(plane) = under.filter(|i| *i != paste.plane)
            && let Some((_, lu2, _)) = self.paste_frame(plane)
        {
            paste.width = (paste.width * lu / lu2).clamp(*PASTE_WIDTHS.start(), *PASTE_WIDTHS.end());
            paste.plane = plane;
        }
        let (h, lu, lv) = self.paste_frame(paste.plane)?;
        let q = h.inverse()?.apply(at(p));
        // The map is a translation of the one at `at` = 0: the grabbed point lands under the pointer.
        let origin = paste_affine(&PasteImage { at: [0.0, 0.0], ..paste }, image, lu, lv).apply(Point::new(grab[0], grab[1]));
        paste.at = [(q.x - origin.x).clamp(-2.0, 3.0), (q.y - origin.y).clamp(-2.0, 3.0)];
        Some((index, paste))
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
                    self.editor.status = Some(*message);
                    Some(Pressed::Done)
                }
            },
            VanishingTool::CreatePlane => {
                self.add_corner(p, words);
                Some(Pressed::Done)
            }
            VanishingTool::EditPlane => {
                if let Some((index, grab)) = self.paste_at(p) {
                    return Some(self.press_paste(index, grab, words));
                }
                self.editor.selected_paste = None;
                self.editor.selected = self.scene.plane_at(&self.session.planes, p);
                // A press inside a plane moves it whole.
                if let Some(plane) = self.editor.selected.map(|i| self.root_of(i))
                    && let Some(from) = self.session.planes.get(plane).filter(|pl| !is_unset(pl)).map(|pl| pl.corners)
                {
                    self.begin_plane_drag(VanishingDrag::Plane { plane, start: p, from }, words);
                    return Some(Pressed::Claimed);
                }
                Some(Pressed::Done)
            }
            VanishingTool::Transform => Some(self.transform_press(p, words)),
        }
    }

    /// Stamp's press: Alt sets the source, otherwise a stroke starts. `Err` is the message of a refused press.
    fn stamp_press(&mut self, p: [f64; 2], mods: Mods, words: &VanishingPointWords) -> Result<Pressed, Box<StatusMessage>> {
        let refused = |text: &Label| Box::new(StatusMessage::new(text.clone()).tone(Tone::Warn));
        if mods.alt {
            if !self.on_plane(p) {
                return Err(refused(&words.stamp_no_source));
            }
            self.session.source = Some(p);
            self.session.aligned_from = None;
            return Ok(Pressed::Done);
        }
        let Some(source) = self.session.source else { return Err(refused(&words.stamp_no_source)) };
        if !self.on_plane(p) {
            return Ok(Pressed::Done);
        }
        let dabs: usize = self.session.strokes.iter().map(|s| s.points.len()).sum();
        if self.session.strokes.len() >= self.editor.limits.strokes || dabs >= self.editor.limits.dabs {
            return Err(refused(&words.stamp_too_many));
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
            VanishingDrag::Plane { plane, start, from } => self
                .shift_plane(plane, from, [p[0] - start[0], p[1] - start[1]])
                .then(|| VanishingPointEvent::Planes(EditEvent::update(id, self.plane_edit()))),
            VanishingDrag::Edge { plane, edge, at_press } => match slide_edge(self.scene, &at_press, edge, p) {
                Some(slid) if self.session.planes.get(plane).is_some_and(|pl| pl.corners != slid.corners) => {
                    self.session.planes[plane].corners = slid.corners;
                    self.resolve();
                    Some(VanishingPointEvent::Planes(EditEvent::update(id, self.plane_edit())))
                }
                _ => None,
            },
            VanishingDrag::Paste { .. } | VanishingDrag::Scale { .. } | VanishingDrag::Rotate { .. } => {
                self.paste_drag_to(&drag, p).then(|| VanishingPointEvent::Paste(EditEvent::update(id, self.session.pastes.clone())))
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
            (VanishingDrag::Paste { .. } | VanishingDrag::Scale { .. } | VanishingDrag::Rotate { .. }, Some(DragStart::Pastes(before))) => {
                Some(VanishingPointEvent::Paste(EditEvent::commit(id, Mixed::Same(before), self.session.pastes.clone())))
            }
            (_, Some(DragStart::Planes(before))) => Some(VanishingPointEvent::Planes(EditEvent::commit(id, Mixed::Same(before), self.plane_edit()))),
            _ => None,
        };
        self.editor.events.extend(event);
    }

    /// An arrow key with a paste selected: moves it by `by` document pixels (onto the plane under its new centre) and
    /// reports a `Set`; whether a paste was selected. Nothing while a drag runs.
    pub fn nudge_paste(&mut self, by: [f64; 2], step: &str) -> bool {
        let Some(i) = self.editor.selected_paste.filter(|_| self.editor.drag.is_none()) else { return false };
        let Some((h, a, _, _)) = self.paste_maps(i) else { return false };
        let c = h.apply(a.apply(Point::new(0.5, 0.5)));
        let Some((_, moved)) = self.paste_moved(i, [0.5, 0.5], [c.x + by[0], c.y + by[1]]) else { return true };
        if moved != self.session.pastes[i] {
            let before = self.session.pastes.clone();
            self.session.pastes[i] = moved;
            self.editor.selected = Some(moved.plane);
            self.editor.events.push(VanishingPointEvent::Paste(EditEvent::set(Mixed::Same(before), self.session.pastes.clone(), label(step), None)));
        }
        true
    }

    /// The plane a move of plane `index` moves: a torn plane follows its parent, so the plane its chain starts from.
    fn root_of(&self, mut index: usize) -> usize {
        // A torn plane's parent always has a lower index, so the chain ends.
        while let Some(Some(t)) = self.session.torn.get(index).copied() {
            index = t.from;
        }
        index
    }

    /// Puts plane `index` at its corners `from` moved by `by` document pixels; the planes torn off it and the pastes on
    /// them follow. Whether it moved.
    fn shift_plane(&mut self, index: usize, from: [[f64; 2]; 4], by: [f64; 2]) -> bool {
        let to = from.map(|c| [c[0] + by[0], c[1] + by[1]]);
        match self.session.planes.get_mut(index) {
            Some(plane) if plane.corners != to => {
                plane.corners = to;
                self.resolve();
                true
            }
            _ => false,
        }
    }

    /// An arrow key with a plane selected and no paste: moves the plane (the one a torn plane hangs off) by `by`
    /// document pixels and reports a `Set`; whether a plane was selected. Nothing while a drag runs.
    pub fn nudge_plane(&mut self, by: [f64; 2], step: &str) -> bool {
        let Some(plane) = self.editor.selected.filter(|_| self.editor.drag.is_none()).map(|i| self.root_of(i)) else { return false };
        let Some(from) = self.session.planes.get(plane).filter(|pl| !is_unset(pl)).map(|pl| pl.corners) else { return false };
        let before = self.plane_edit();
        if self.shift_plane(plane, from, by) {
            let after = self.plane_edit();
            self.editor.events.push(VanishingPointEvent::Planes(EditEvent::set(Mixed::Same(before), after, label(step), None)));
        }
        true
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use craft_ui::testing::trial;

    /// A turn or a mirror keeps the paste's centre; a quarter turn swaps its sides in the plane's metric.
    #[trial]
    fn paste_affine_turns_and_mirrors_about_the_centre() {
        let (lu, lv) = (2.0, 1.0);
        let plain = PasteImage::new([0.1, 0.2], 0.25);
        let centre = |p: &PasteImage| paste_affine(p, (100.0, 50.0), lu, lv).apply(Point::new(0.5, 0.5));
        let c = centre(&plain);
        for turned in [plain.turned(90.0, false, false), plain.turned(-37.0, true, false), plain.turned(0.0, true, true)] {
            let d = centre(&turned);
            assert!((c.x - d.x).abs() < 1e-12 && (c.y - d.y).abs() < 1e-12, "{turned:?}");
        }
        let side = |p: &PasteImage, s: f64, t: f64| {
            let (a, b) =
                (paste_affine(p, (100.0, 50.0), lu, lv).apply(Point::new(0.0, 0.0)), paste_affine(p, (100.0, 50.0), lu, lv).apply(Point::new(s, t)));
            ((b.x - a.x) * lu).hypot((b.y - a.y) * lv)
        };
        let quarter = plain.turned(90.0, false, false);
        assert!((side(&plain, 1.0, 0.0) - side(&quarter, 1.0, 0.0)).abs() < 1e-12, "a turn keeps the width in the metric");
        let top = paste_affine(&quarter, (100.0, 50.0), lu, lv);
        assert!((top.apply(Point::new(1.0, 0.0)).x - top.apply(Point::new(0.0, 0.0)).x).abs() < 1e-12, "a quarter turn stands the top edge upright");
        let flipped = paste_affine(&plain.turned(0.0, true, false), (100.0, 50.0), lu, lv);
        let unflipped = paste_affine(&plain, (100.0, 50.0), lu, lv);
        assert!(
            (flipped.apply(Point::new(0.0, 0.0)).x - unflipped.apply(Point::new(1.0, 0.0)).x).abs() < 1e-12,
            "Flip Horizontal swaps left and right"
        );
    }
}
