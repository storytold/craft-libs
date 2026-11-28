//! [`WarpGrid`]: the mesh of a warp (PhotoCraft's Edit > Transform > Warp) as a canvas control: patch
//! boundaries, anchors and their handles. Every anchor and handle end is a part in the canvas's
//! arena, so a press on one stays with the grid and a press elsewhere reaches the tool.

use craft_behavior::kurbo::Point;
use craft_behavior::kurbo::Rect as DocRect;
use craft_behavior::{
    CancelReason, CursorIcon, CursorPriority, HitLevel, HitPriority, HitShape, Intent, Metrics, NavShape, PartKey, PointerButton, PressPhase,
    PressState, Roving, Sense, resize_cursor,
};
use craft_ui::canvas::{CanvasSpace, NUDGE, NUDGE_FAR, canvas_space};
use craft_ui::canvas_controls::path_runs;
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, MeasureCx, Name, Outcome, PaintUi, PartsCx, Pointer as _,
    Props as _, Remember as _, Role, Styled as _, WordSetCx as _,
};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::KeyUse;
use craft_ui::style::{ComponentStyle, DataColor, HandleLook, StyleColor, StyleCx};
use egui::{CornerRadius, Pos2, Rect, Response, Vec2};

use super::model::{SplitAt, WarpEdit, WarpMesh, WarpSide, WarpSplit};

/// How far from the pointer the mesh's handles show, in hit targets (`size.hit.target`; 56 at an 8 target).
const REVEAL_IN_TARGETS: f32 = 7.0;

/// An anchor or a handle end: `POINT.at(index)` with the index into [`WarpMesh::points`].
pub const POINT: PartKey = PartKey::new("point");

/// The surface of the mesh, inside its outer boundary.
pub const SURFACE: PartKey = PartKey::new("surface");

craft_ui::words! {
    /// The accessible names of the warp grid: the group and, for a point, its row and column from 1.
    pub struct WarpWords {
        group: "warp.group" = "Warp";
        anchor: template "warp.anchor" = "Row {row}, column {column}";
        handle_up: template "warp.handleUp" = "Row {row}, column {column}, up handle";
        handle_left: template "warp.handleLeft" = "Row {row}, column {column}, left handle";
        handle_right: template "warp.handleRight" = "Row {row}, column {column}, right handle";
        handle_down: template "warp.handleDown" = "Row {row}, column {column}, down handle";
    }
}

/// The host's surface parameters under a document point: [`WarpGrid::surface`].
pub struct WarpSurface<'a>(Box<dyn Fn(Point) -> Option<(f64, f64)> + 'a>);

impl<'a> WarpSurface<'a> {
    /// The rule `surface`.
    pub fn new(surface: impl Fn(Point) -> Option<(f64, f64)> + 'a) -> Self {
        WarpSurface(Box::new(surface))
    }
}

type WarpRuleFn<'a> = Box<dyn Fn(&WarpMesh, &WarpEdit) -> WarpMesh + 'a>;

/// The host's way to move its mesh: [`WarpGrid::rule`].
pub struct WarpRule<'a>(WarpRuleFn<'a>);

impl<'a> WarpRule<'a> {
    /// The rule `rule`: the mesh an edit makes of the mesh at the press.
    pub fn new(rule: impl Fn(&WarpMesh, &WarpEdit) -> WarpMesh + 'a) -> Self {
        WarpRule(Box::new(rule))
    }
}

/// The look of the warp grid: lines in `overlay.line`, arms in `overlay.handle_stroke`, the split
/// preview in `overlay.stroke` over `overlay.halo`, anchors as handle squares and handle ends as
/// small handle circles.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct WarpGridStyle {
    #[style(metrics)]
    metrics: Metrics,
    line: StyleColor,
    arm: StyleColor,
    split: StyleColor,
    halo: StyleColor,
    anchor: HandleLook,
    end: HandleLook,
    /// How far from an anchor or a handle end a press takes it.
    reach: f32,
    /// How far from the pointer a handle and its arm show; measured to the handle or to its anchor.
    reveal: f32,
}

/// The most thin guide lines a host can ask for inside one cell, each way. It only bounds the work
/// of one paint; a host's own maximum is its business.
pub const GUIDE_LINES_MAX: usize = 32;

/// When the thin guide lines inside the cells draw: as the grid always did, never, or only while a
/// press drags the mesh (the Auto row a host's popup gives).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WarpGuidesShow {
    /// The lines draw at rest, under a hovering pointer and during a drag.
    #[default]
    Always,
    /// The lines draw only while the grid has a press running that drags the mesh: none at rest or
    /// under a hover, and none again once the press releases.
    WhileDragging,
    /// The lines never draw. The patch boundaries and the anchors do, as ever.
    Never,
}

/// The thin guide lines inside the cells of a [`WarpGrid`], as the host wants them: their colour, how
/// much of their alpha shows, how many there are in a cell and when they show at all. Set through
/// [`WarpGrid::guides`]; no `guides` at all paints the grid's own rule, and so does every field left
/// at its default here.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct WarpGuides {
    /// The colour of the thin lines. `None` paints them in the style's `line`, as the grid always did.
    pub line: Option<DataColor>,
    /// The share of that colour's alpha, 0 to 1: 1 shows the colour whole, 0.5 half as strong. A share
    /// that is not a number shows the colour whole.
    pub share: f32,
    /// How many thin lines to draw inside every cell, each way. `None` keeps the grid's own rule:
    /// the two rule-of-thirds curves of a one-patch mesh, none once it is cut. A number is taken
    /// as 1 to [`GUIDE_LINES_MAX`].
    pub density: Option<usize>,
    /// When the thin lines draw. The patch boundaries and the anchors always do.
    pub show: WarpGuidesShow,
}

impl Default for WarpGuides {
    fn default() -> Self {
        WarpGuides { line: None, share: 1.0, density: None, show: WarpGuidesShow::Always }
    }
}

/// What a press drags.
#[derive(Clone, Copy, Debug)]
enum Target {
    Point(usize),
    Surface(f64, f64),
    /// A split: the insert happens at the release.
    Split,
}

/// A press the grid owns: the mesh at the press, which every move of the drag is applied to.
#[derive(Clone, Debug)]
struct Press {
    mesh: WarpMesh,
    target: Target,
    began: bool,
    /// A quick split: no tool was set at the press.
    quick: bool,
}

impl Default for Press {
    fn default() -> Self {
        Press { mesh: WarpMesh { us: Vec::new(), vs: Vec::new(), points: Vec::new() }, target: Target::Split, began: false, quick: false }
    }
}

/// What the grid remembers between passes.
#[derive(Clone, Default)]
struct Slot {
    picked: Option<usize>,
    /// The anchor or handle being dragged.
    dragging: Option<usize>,
    /// A press that drags the mesh - an anchor or the surface - is down: between its down and its
    /// release or cancel. What `WarpGuidesShow::WhileDragging` asks for.
    press_open: bool,
    hover: Option<Pos2>,
    preview: Option<SplitAt>,
}

#[derive(Default)]
struct Report {
    edit: Option<WarpEdit>,
    picked: Option<usize>,
}

/// The mesh of a warp over a canvas: patch boundaries, anchors and their handles, dragged by the
/// pointer and moved from the keyboard as PhotoCraft's Warp has them. The mesh is the binding; the
/// host's [`rule`](Self::rule) moves it (an anchor carrying its handles, a pull of the surface).
///
/// - Look: patch boundaries `overlay.line` at `stroke.thin`, and the thin guide lines inside each cell
///   under them: the one-patch mesh's rule-of-thirds lines, or the colour, share, density and
///   show-mode of [`guides`](Self::guides). Anchors always show; a handle and its arm only near the
///   pointer, while its anchor or one of its handles is dragged, or while one of its anchor's parts is
///   picked and the grid has the keyboard.
/// - Pointer: the nearest anchor or handle within the hit reach drags that point; elsewhere on the
///   surface the press pulls it. One drag is one gesture; Esc cancels it.
/// - Split: with a [`split`](Self::split) tool set, a hover over the surface previews the new lines and a
///   click inserts them. With no tool, Alt or Ctrl at the press is a quick split chosen by the pointer.
/// - Keyboard: one Tab stop. Page Down and Page Up pick the next and previous point (anchors row by row,
///   each followed by its handles), Home and End the first and last; the arrows move the picked point.
///   With a split tool set, Enter splits the picked anchor's patch in the middle.
/// - Nothing mirrors in a right-to-left interface: the mesh is the document's.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control)]
pub struct WarpGrid<'a> {
    #[common]
    common: Common,
    mesh: Bind<'a, WarpMesh>,
    bounds: DocRect,
    /// Only the patch boundaries draw: no points, no surface (PhotoCraft's Warp on type, which takes only
    /// the warp styles).
    #[prop(flag)]
    lines_only: bool,
    /// The mesh is a preset fitted as one patch: a split cuts it as it is instead of making its
    /// rule-of-thirds lines real first.
    #[prop(flag)]
    preset: bool,
    /// The split tool, when one is armed.
    #[prop]
    split: Option<WarpSplit>,
    /// The thin guide lines inside the cells, as the host wants them: `None` draws the grid's own rule
    /// in the style's line, which is how the grid painted before a host had a say.
    #[prop]
    guides: Option<WarpGuides>,
    /// The mesh parameters `(s, t)` under a document point, `None` off the surface; by default
    /// [`WarpMesh::surface_at`].
    #[prop(func = "Fn(Point) -> Option<(f64, f64)>")]
    surface: Option<WarpSurface<'a>>,
    /// The mesh an edit makes of the mesh at the press; by default [`WarpMesh::edited`].
    #[prop(func = "Fn(&WarpMesh, &WarpEdit) -> WarpMesh")]
    rule: Option<WarpRule<'a>>,
    report: Report,
}

impl<'a> WarpGrid<'a> {
    /// A grid named `label` over `mesh`, a warp made on the document rectangle `bounds` (which the quick
    /// split's reach is measured against).
    pub fn new(label: impl Into<Label>, mesh: Bind<'a, WarpMesh>, bounds: DocRect) -> Self {
        WarpGrid {
            common: Common::new(),
            mesh,
            bounds,
            lines_only: false,
            preset: false,
            split: None,
            guides: None,
            surface: None,
            rule: None,
            report: Report::default(),
        }
        .label(label)
    }

    fn apply(&self, mesh: &WarpMesh, edit: &WarpEdit) -> WarpMesh {
        match &self.rule {
            Some(rule) => (rule.0)(mesh, edit),
            None => mesh.edited(edit),
        }
    }

    fn surface_params(&self, mesh: &WarpMesh, p: Point) -> Option<(f64, f64)> {
        match &self.surface {
            Some(surface) => (surface.0)(p),
            None => mesh.surface_at(p),
        }
    }

    /// The anchors row by row, each followed by its handles.
    fn points(mesh: &WarpMesh) -> Vec<usize> {
        mesh.anchors().flat_map(|anchor| std::iter::once(anchor).chain(mesh.handles(anchor).into_iter().map(|(index, _)| index))).collect()
    }

    /// One press phase of a point part (`point`) or of the surface (`None`).
    fn run_press(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, mesh: &WarpMesh, point: Option<usize>, press: PressState) {
        let view = &space.view;
        match press.phase {
            PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                let splitting = self.split.is_some() || press.mods.alt || press.mods.cmd;
                let target = if splitting {
                    Some(Target::Split)
                } else if let Some(index) = point {
                    Some(Target::Point(index))
                } else {
                    self.surface_params(mesh, view.to_doc(start.origin)).map(|(s, t)| Target::Surface(s, t))
                };
                if let Some(Target::Point(index)) = target {
                    let slot = cx.memory::<Slot>();
                    slot.picked = Some(index);
                    slot.dragging = Some(index);
                }
                cx.memory::<Slot>().press_open = matches!(target, Some(Target::Point(_)) | Some(Target::Surface(..)));
                *cx.press_slot::<Press>() = target.map(|target| Press { mesh: mesh.clone(), target, began: false, quick: self.split.is_none() });
            }
            PressPhase::DragStart { start, pos, .. } | PressPhase::Drag { start, pos, .. } if start.button == PointerButton::Primary => {
                let Some(mut held) = cx.press_slot::<Press>().clone() else { return };
                // The host replaced the mesh under the press (a split, a style pick). A begun press was cancelled by the
                // framework (`HostChanged`); this only drops the press.
                if mesh.us != held.mesh.us || mesh.vs != held.mesh.vs {
                    let slot = cx.memory::<Slot>();
                    slot.dragging = None;
                    slot.press_open = false;
                    *cx.press_slot::<Press>() = None;
                    return;
                }
                let by = view.to_doc(pos) - view.to_doc(start.origin);
                let edit = match held.target {
                    Target::Point(index) => WarpEdit::Point { index, by },
                    Target::Surface(s, t) => WarpEdit::Surface { s, t, by },
                    Target::Split => return,
                };
                if !held.began {
                    cx.begin();
                    held.began = true;
                    *cx.press_slot::<Press>() = Some(held.clone());
                }
                cx.propose(self.apply(&held.mesh, &edit));
                self.report.edit = Some(edit);
            }
            PressPhase::Release { start, pos, .. } if start.button == PointerButton::Primary => {
                let slot = cx.memory::<Slot>();
                slot.dragging = None;
                slot.press_open = false;
                let Some(held) = cx.press_slot::<Press>().take() else { return };
                match held.target {
                    Target::Split => {
                        if let Some(at) = mesh.split_at(self.preset, self.bounds, self.split, held.quick, view.to_doc(pos)) {
                            let edit = at.edit();
                            cx.set(self.apply(mesh, &edit));
                            self.report.edit = Some(edit);
                        }
                    }
                    _ if held.began => cx.commit(),
                    _ => {}
                }
            }
            PressPhase::Cancel { .. } => Self::end_press(cx),
            _ => {}
        }
    }

    /// Ends the running press without a result: reports the cancel when the drag began and clears the press slot.
    fn end_press(cx: &mut Cx<'_, Self>) {
        let slot = cx.memory::<Slot>();
        slot.dragging = None;
        slot.press_open = false;
        if cx.press_slot::<Press>().take().is_some_and(|held| held.began) {
            cx.cancel(CancelReason::Escape);
        }
    }

    /// The presses on the points and the surface, and the pointer over them.
    fn run_pointer(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, mesh: &WarpMesh) {
        let mut hover: Option<(Pos2, bool)> = None;
        let mut results: Vec<(Option<usize>, PressState)> = Vec::new();
        let declared = Self::points(mesh).into_iter().map(Some).chain(std::iter::once(None));
        for point in declared {
            let found = match point {
                Some(index) => cx.part_if_declared(POINT.at(index as u64)),
                None => cx.part_if_declared(SURFACE),
            };
            let Some(result) = found else { continue };
            let hover_at = match point {
                Some(index) => cx.hover_pos(POINT.at(index as u64)),
                None => cx.hover_pos(SURFACE),
            };
            let press = result.press;
            if hover.is_none()
                && let Some(at) = hover_at
            {
                hover = Some((at, press.mods.alt || press.mods.cmd));
            }
            if point.is_none()
                && self.split.is_none()
                && let Some(at) = hover_at
                && self.surface_params(mesh, space.view.to_doc(at)).is_some()
            {
                cx.cursor(CursorIcon::Grab, CursorPriority::Part);
            }
            results.push((point, press));
        }
        for (point, press) in results {
            self.run_press(cx, space, mesh, point, press);
        }
        let quick = hover.is_some_and(|(_, held)| held);
        let preview = hover.map(|(at, _)| at).filter(|_| self.split.is_some() || quick).and_then(|at| {
            let found = mesh.split_at(self.preset, self.bounds, self.split, self.split.is_none(), space.view.to_doc(at))?;
            let cursor = match found.motion {
                None => CursorIcon::Crosshair,
                Some(across) => resize_cursor(&space.view, across),
            };
            cx.cursor(cursor, CursorPriority::Part);
            Some(found)
        });
        let slot = cx.memory::<Slot>();
        slot.hover = hover.map(|(at, _)| at);
        slot.preview = preview;
    }

    /// The keys of the picked point: the arrows move it, Enter splits its patch under a split tool.
    fn run_keys(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, mesh: &WarpMesh) {
        let focused = cx.active_part().filter(|part| part.key == POINT).and_then(|part| part.index).map(|index| index as usize);
        if let Some(index) = focused {
            cx.memory::<Slot>().picked = Some(index);
        }
        let Some(index) = focused else { return };
        if let Some((direction, mods)) = cx.intents().take_nudge() {
            let step = if mods.shift { NUDGE_FAR } else { NUDGE };
            let by = space.view.unmap_vec(direction) * step;
            let edit = WarpEdit::Point { index, by };
            cx.set(self.apply(mesh, &edit));
            self.report.edit = Some(edit);
        }
        if let Some(tool) = self.split
            && let Some(anchor) = mesh.anchor_of(index)
            && cx.intents().take(Intent::Submit)
        {
            // The anchor's corner of the mesh the cut is made on: a one-patch mesh's thirds put it at the
            // same corner of the 3 x 3.
            let (base, thirds) = mesh.split_base(self.preset);
            let (w, scale) = (mesh.width(), if thirds { 3 } else { 1 });
            let (row, column) = ((anchor / w / 3 * scale).min(base.rows() - 1), (anchor % w / 3 * scale).min(base.columns() - 1));
            let edit = WarpEdit::Split { split: tool, column, row, u: 0.5, v: 0.5, thirds };
            cx.set(self.apply(mesh, &edit));
            self.report.edit = Some(edit);
        }
    }
}

/// What the grid reports: the edit of the mesh, the last [`WarpEdit`] and the picked point.
#[derive(Debug)]
#[non_exhaustive]
pub struct WarpGridOutput {
    /// The edit of the mesh: a drag is one `Begin` to `Commit`; a nudge or a split is a `Set`.
    pub mesh: EditResponse<WarpMesh>,
    /// The edit the mesh last received this pass, as the pointer or the keys meant it.
    pub edit: Option<WarpEdit>,
    /// The control point the keys act on, picked by the pointer or the Page keys.
    pub picked: Option<usize>,
}

impl Outcome for WarpGridOutput {
    type Value = WarpMesh;

    fn response(&self) -> &Response {
        &self.mesh.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.mesh.story_facts()
    }
}

impl Component for WarpGrid<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::CANVAS.with_submits();
    type Style = WarpGridStyle;
    type Output = WarpGridOutput;

    fn style(cx: &StyleCx<'_>, _: WarpGridStyleKey) -> WarpGridStyle {
        let overlay = &cx.tokens.color.overlay;
        let mut end = cx.handle();
        end.side = cx.tokens.size.handle.sm;
        WarpGridStyle {
            metrics: Metrics::new(),
            line: cx.color(overlay.line),
            arm: cx.color(overlay.handle_stroke),
            split: cx.color(overlay.stroke),
            halo: cx.color(overlay.halo),
            anchor: cx.handle(),
            end,
            reach: cx.tokens.size.hit.target,
            reveal: cx.tokens.size.hit.target * REVEAL_IN_TARGETS,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn bind(&mut self) -> Option<BindMut<'_, WarpMesh>> {
        Some(self.mesh.reborrow())
    }

    /// The points in one roving group in keyboard order, and the surface under them.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let (Some(space), Some(mesh)) = (canvas_space(&cx.env), cx.value()) else { return };
        if self.lines_only || !mesh.is_valid() {
            return;
        }
        let armed = self.split.is_some();
        let sense = if armed { Sense::CLICK | Sense::TAKE_FOCUS } else { Sense::DRAG | Sense::TAKE_FOCUS };
        for index in Self::points(mesh) {
            let at = space.view.to_screen(mesh.points[index]);
            let part = cx
                .part(POINT.at(index as u64), HitShape::circle(at, cx.style.reach))
                .sense(sense)
                .priority(HitPriority::new(HitLevel::Handle, 0))
                .focusable();
            if !armed {
                part.cursor(CursorIcon::Crosshair);
            }
        }
        // The surface covers the content rect below the points: a press off the mesh does nothing unless a split tool
        // is armed or Alt or Cmd is held at the press, which cuts at the nearest point of the mesh.
        let part = cx
            .part(SURFACE, HitShape::Rect(cx.content_rect))
            .sense(Sense::DRAG | Sense::CLICK | Sense::TAKE_FOCUS)
            .priority(HitPriority::new(HitLevel::Body, 0))
            .keyboard_via(POINT);
        if !armed {
            part.drag_cursor(CursorIcon::Grabbing);
        }
        cx.roving(Roving::new(NavShape::Flat).page(Some(1)));
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let Some(mesh) = cx.value().filter(|mesh| mesh.is_valid()).cloned() else { return };
        if self.lines_only {
            return;
        }
        self.run_pointer(cx, &space, &mesh);
        self.run_keys(cx, &space, &mesh);
        let picked = cx.memory::<Slot>().picked.filter(|&index| index < mesh.points.len() && mesh.anchor_of(index).is_some());
        cx.memory::<Slot>().picked = picked;
        self.report.picked = picked;
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (Some(space), Some(mesh)) = (canvas_space(&cx.env), cx.value()) else { return };
        if !mesh.is_valid() {
            return;
        }
        let (draw, style, view) = (ui.draw(), &cx.style, &space.view);
        let stroke = |path: &craft_behavior::kurbo::BezPath, width: StrokeWidth, color: StyleColor| {
            for (run, _) in path_runs(view, path) {
                draw.stroke(&PaintShape::Path(run), width, color);
            }
        };
        let stroke_data = |path: &craft_behavior::kurbo::BezPath, width: StrokeWidth, color: DataColor| {
            for (run, _) in path_runs(view, path) {
                draw.stroke(&PaintShape::Path(run), width, color);
            }
        };
        // The thin guide lines inside the cells, under the patch boundaries: the grid's own rule when no
        // host rule is set, and the host's colour, share, density and show-mode when it sets them. A
        // `WhileDragging` host gets its lines only while a press of the grid's own runs.
        let slot = cx.recall::<Slot>();
        let drawn = match self.guides {
            None => true,
            Some(guides) => match guides.show {
                WarpGuidesShow::Always => true,
                WarpGuidesShow::WhileDragging => slot.press_open,
                WarpGuidesShow::Never => false,
            },
        };
        if drawn {
            let paths = match self.guides.and_then(|guides| guides.density) {
                None => mesh.own_guides(),
                Some(lines) => mesh.guide_paths(lines.clamp(1, GUIDE_LINES_MAX)),
            };
            // A host colour or share paints the lines as data; the grid's own look keeps the style colour.
            let data = self.guides.filter(|guides| guides.line.is_some() || guides.share != 1.0).map(|guides| {
                let share = if guides.share.is_finite() { guides.share.clamp(0.0, 1.0) } else { 1.0 };
                let DataColor(base) = guides.line.unwrap_or_else(|| DataColor::new(style.line.egui()));
                DataColor::new(if share == 1.0 { base } else { base.linear_multiply(share) })
            });
            for path in &paths {
                match data {
                    None => stroke(path, StrokeWidth::Hairline, style.line),
                    Some(line) => stroke_data(path, StrokeWidth::Hairline, line),
                }
            }
        }
        for path in mesh.boundaries() {
            stroke(&path, StrokeWidth::Thin, style.line);
        }
        if self.lines_only {
            return;
        }
        let screen = |index: usize| view.to_screen(mesh.points[index]);
        let near = |p: Pos2| slot.hover.is_some_and(|hover| hover.distance(p) <= cx.style.reveal);
        let kept = [slot.dragging, slot.picked.filter(|_| cx.focus_visible())].map(|index| index.and_then(|index| mesh.anchor_of(index)));
        let mut ends = Vec::new();
        for anchor in mesh.anchors() {
            for (index, _) in mesh.handles(anchor) {
                if near(screen(index)) || near(screen(anchor)) || kept.contains(&Some(anchor)) {
                    draw.stroke(&PaintShape::Path(vec![screen(anchor), screen(index)]), StrokeWidth::Hairline, style.arm);
                    ends.push(index);
                }
            }
        }
        // An edit of this pass can have changed the mesh since `parts` declared its points; those draw at rest.
        let look = |index: usize, fill: &craft_ui::style::Stateful<craft_ui::style::StyleColor>| {
            if cx.part_if_declared(POINT.at(index as u64)).is_some() { cx.resolve_part(POINT.at(index as u64), fill) } else { cx.resolve(fill) }
        };
        for anchor in mesh.anchors() {
            let square = PaintShape::Rect(Rect::from_center_size(screen(anchor), Vec2::splat(style.anchor.side)), CornerRadius::ZERO);
            draw.fill(&square, look(anchor, &style.anchor.fill));
            draw.stroke(&square, StrokeWidth::Thin, style.anchor.stroke);
        }
        for index in ends {
            let circle = PaintShape::Circle(screen(index), style.end.side / 2.0);
            draw.fill(&circle, look(index, &style.end.fill));
            draw.stroke(&circle, StrokeWidth::Thin, style.end.stroke);
        }
        if let Some(at) = slot.preview {
            for line in at.lines(mesh) {
                stroke(&line, StrokeWidth::Thick, style.halo);
                stroke(&line, StrokeWidth::Thin, style.split);
            }
        }
    }

    /// A group named by the words, and one node per point named by its row and column.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<WarpWords>();
        access.name(Name::Text(words.group.as_str().to_owned()));
        let (Some(space), Some(mesh)) = (canvas_space(&cx.env), cx.value()) else { return };
        if self.lines_only || !mesh.is_valid() {
            return;
        }
        let (rulers, units) = (space.rulers(), cx.env.units());
        let w = mesh.width();
        for anchor in mesh.anchors() {
            let (row, column) = ((anchor / w / 3 + 1).to_string(), (anchor % w / 3 + 1).to_string());
            let fields = [("row", row.as_str()), ("column", column.as_str())];
            let sides = mesh.handles(anchor).into_iter().map(|(index, side)| {
                let word = match side {
                    WarpSide::Up => &words.handle_up,
                    WarpSide::Left => &words.handle_left,
                    WarpSide::Right => &words.handle_right,
                    WarpSide::Down => &words.handle_down,
                };
                (index, word)
            });
            for (index, word) in std::iter::once((anchor, &words.anchor)).chain(sides) {
                let p = mesh.points[index];
                let value = format!("{}, {}", rulers.x.format_value(&units, p.x, rulers.zero.x), rulers.y.format_value(&units, p.y, rulers.zero.y));
                let node = access.node(POINT.at(index as u64), Role::GRAPHICS_OBJECT);
                node.name = Name::Text(word.fill(&fields));
                node.description = Some(value);
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> WarpGridOutput {
        WarpGridOutput { mesh: cx.edit_response(), edit: self.report.edit, picked: self.report.picked }
    }
}
