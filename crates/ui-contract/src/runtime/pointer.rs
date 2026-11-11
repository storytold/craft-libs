//! The pointer snapshot, press targets, the shape record and press cancels (8.1, 8.3).
//!
//! The begin hook converts the pass's pointer input once. The sensing core in `primitives` runs the press
//! lifecycle over it and writes the shapes it registers into the record the focus service reads.

use crate::runtime::drag;
use crate::runtime::egui::areas::{self, Layer};
use crate::runtime::egui::{input, pass};
use crate::runtime::frame::{PerViewport, Stamped};
use craft_behavior::{
    Buttons, MultiTouch, PartRef, PointerButton, PointerEvent, PointerKind, PointerSample, PressCancel, PressStart, ResolveFor, Shape, Winner,
    count_clicks, resolve,
};
use craft_keymap::Mods;
use craft_theme::HitSizes;
use egui::{Context, Id, Pos2, Rect, Vec2, ViewportId};
use std::collections::HashMap;
use std::sync::Arc;

/// Points one wheel line adds to `PointerFrame.scroll`; scroll owners apply their own notch (the wheel service).
pub(crate) const WHEEL_LINE_POINTS: f32 = 40.0;

/// Points one wheel page adds to `PointerFrame.scroll`.
pub(crate) const WHEEL_PAGE_POINTS: f32 = 400.0;

/// The pointer input of one pass of one viewport, egui-converted.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PointerFrame {
    /// The latest pointer position, `None` while the pointer is not in the window.
    pub pos: Option<Pos2>,
    /// The pointer movement of the pass.
    pub delta: Vec2,
    /// The buttons held.
    pub buttons: Buttons,
    /// The primary press of the pass.
    pub press: Option<PressStart>,
    /// The button released in the pass.
    pub release: Option<PointerButton>,
    /// The modifiers held.
    pub mods: Mods,
    /// The pass's time in seconds.
    pub time: f64,
    /// The device.
    pub kind: PointerKind,
    /// Pressure and tilt of the current contact.
    pub sample: Option<PointerSample>,
    /// The two-finger gesture.
    pub multi_touch: Option<MultiTouch>,
    /// Wheel and trackpad scroll, in points.
    pub scroll: Vec2,
    /// Pinch and Ctrl or Cmd with the wheel, folded into one factor.
    pub zoom: f32,
    /// The movement that turns a press into a drag.
    pub slop: f32,
    /// The least hit size across for this pointer kind.
    pub target: f32,
}

/// egui's click and drag widgets for the latest press.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[doc(hidden)]
pub struct PressTargets {
    pub click: Option<Id>,
    pub drag: Option<Id>,
}

/// What the begin hook read of the pass, for the sensing core and the services.
#[derive(Clone, Debug, Default)]
#[doc(hidden)]
pub struct PassPointer {
    pub frame: Option<PointerFrame>,
    /// The primary events, presses carrying their click counts.
    pub events: Vec<PointerEvent>,
    pub hover: Option<Pos2>,
    pub primary_down: bool,
    pub press_targets: PressTargets,
    pub focus_lost: bool,
    /// Whether the real pointer moved or entered, or its modifiers changed, in the pass.
    pub stirred: bool,
}

#[derive(Default)]
struct Memory {
    last_press: Option<(Pos2, f64, u32)>,
    press_targets: PressTargets,
    /// A cancel a service requested for the running pass.
    requested: Option<(u64, PressCancel)>,
    /// Whether any arena of the viewport holds an open press; sensing registers it, as the layer above.
    press_probe: Option<fn(&Context) -> bool>,
}

/// What one root registered on one layer: its shapes and its egui widgets.
#[derive(Clone, Default)]
struct RootRecord {
    shapes: Vec<Shape<Id>>,
    /// The root's egui widgets: the click and drag targets that stand for it.
    widgets: Vec<Id>,
    /// The root's widgets sense a click.
    click: bool,
}

type LayerRecord = HashMap<Layer, HashMap<Id, RootRecord>>;

#[derive(Default)]
struct Shapes {
    this: LayerRecord,
    last: LayerRecord,
}

/// The begin hook (H5): converts the pass's pointer input.
pub(crate) fn begin(ctx: &Context, hit: HitSizes) {
    let viewport = pass::current_viewport(ctx);
    let pass_nr = pass::pass_nr(ctx, viewport);
    let mut read = input::pointer_read(ctx);
    // A drag that started in another window reaches this one through the desktop pointer (8.4).
    if read.pos.is_none() {
        read.pos = drag::cross_window_pointer(ctx, viewport);
    }
    let wheel = input::wheel_read(ctx, WHEEL_LINE_POINTS, WHEEL_PAGE_POINTS);
    let hits = input::hit_targets(ctx);
    let other_pressed =
        read.events.iter().any(|event| matches!(event, PointerEvent::Pressed { button: PointerButton::Middle | PointerButton::Secondary, .. }));
    let (events, targets) = PerViewport::<Memory>::with(ctx, viewport, |memory| {
        let mut events = read.events.clone();
        for event in &mut events {
            if let PointerEvent::Pressed { button: PointerButton::Primary, pos, time, clicks, .. } = event {
                *clicks = count_clicks(memory.last_press, *pos, *time, read.slop);
                memory.last_press = Some((*pos, *time, *clicks));
            }
        }
        if read.primary_pressed || other_pressed {
            memory.press_targets = PressTargets { click: hits.0, drag: hits.1 };
        }
        (events, memory.press_targets)
    });
    let press = events.iter().rev().find_map(|event| match event {
        PointerEvent::Pressed { button, pos, time, clicks, .. } => Some(PressStart::new(*pos, *button, *time, *clicks)),
        _ => None,
    });
    let press = press.or_else(|| read.other_press.map(|(button, at)| PressStart::new(at, button, read.time, 1)));
    let (scroll, wheel_zoom) = (wheel.map_or(Vec2::ZERO, |(scroll, ..)| scroll), read.zoom);
    let frame = PointerFrame {
        pos: read.pos,
        delta: read.delta,
        buttons: read.buttons,
        press,
        release: read.release,
        mods: read.mods,
        time: read.time,
        kind: read.kind,
        sample: read.sample,
        multi_touch: read.multi_touch.map(|(translation, zoom, rotation, center)| MultiTouch::new(translation, zoom, rotation, center)),
        scroll,
        zoom: wheel_zoom,
        slop: read.slop,
        target: hit.target,
    };
    let pass_pointer = PassPointer {
        frame: Some(frame),
        events,
        hover: read.hover,
        primary_down: read.primary_down,
        press_targets: targets,
        focus_lost: read.focus_lost,
        stirred: read.stirred,
    };
    PerViewport::<Stamped<Arc<PassPointer>>>::with_pass(ctx, viewport, pass_nr, |_, _| {}, |record| *record = Arc::new(pass_pointer));
    PerViewport::<Shapes>::with(ctx, viewport, |shapes| shapes.last = std::mem::take(&mut shapes.this));
}

/// The running viewport's pass number.
pub(crate) fn pass_of(ctx: &Context) -> u64 {
    pass::pass_nr(ctx, pass::current_viewport(ctx))
}

/// What the begin hook read of the running pass.
pub(crate) fn pass_pointer(ctx: &Context) -> Arc<PassPointer> {
    let viewport = pass::current_viewport(ctx);
    PerViewport::<Stamped<Arc<PassPointer>>>::with_pass(ctx, viewport, pass::pass_nr(ctx, viewport), |_, _| {}, |record| Arc::clone(record))
}

/// Reads what the begin hook read of the running pass in place, without cloning its events.
#[doc(hidden)]
pub fn with_pass_pointer<R>(ctx: &Context, f: impl FnOnce(&PassPointer) -> R) -> R {
    let viewport = pass::current_viewport(ctx);
    let pass_nr = pass::pass_nr(ctx, viewport);
    PerViewport::<Stamped<Arc<PassPointer>>>::with_pass(ctx, viewport, pass_nr, |_, _| {}, |record| f(record))
}

/// Reads the running pass's pointer frame, `None` before the begin hook, and its hover position in place.
pub(crate) fn with_frame<R>(ctx: &Context, f: impl FnOnce(Option<&PointerFrame>, Option<Pos2>) -> R) -> R {
    with_pass_pointer(ctx, |pointer| f(pointer.frame.as_ref(), pointer.hover))
}

/// The running pass's pointer input.
pub fn frame(ctx: &Context) -> PointerFrame {
    with_pass_pointer(ctx, |pointer| pointer.frame.clone()).unwrap_or_else(|| PointerFrame {
        pos: None,
        delta: Vec2::ZERO,
        buttons: Buttons::empty(),
        press: None,
        release: None,
        mods: Mods::NONE,
        time: 0.0,
        kind: PointerKind::Mouse,
        sample: None,
        multi_touch: None,
        scroll: Vec2::ZERO,
        zoom: 1.0,
        slop: 0.0,
        target: 0.0,
    })
}

/// Ends every captured press of the running viewport at its next lifecycle run in this pass (Escape, a part
/// turning disabled). The next begin hook ends egui's own drag so hover reaches other widgets again.
pub(crate) fn cancel_press(ctx: &Context, reason: PressCancel) {
    let viewport = pass::current_viewport(ctx);
    let pass_nr = pass::pass_nr(ctx, viewport);
    PerViewport::<Memory>::with(ctx, viewport, |memory| memory.requested = Some((pass_nr, reason)));
}

/// Registers how the viewport tells that a press is captured. Sensing owns the arenas' machines.
pub(crate) fn register_press_probe(ctx: &Context, probe: fn(&Context) -> bool) {
    PerViewport::<Memory>::with(ctx, pass::current_viewport(ctx), |memory| memory.press_probe = Some(probe));
}

/// Whether a press of the running viewport is open: Escape cancels it before anything else.
pub(crate) fn press_captured(ctx: &Context) -> bool {
    let probe = PerViewport::<Memory>::with(ctx, pass::current_viewport(ctx), |memory| memory.press_probe);
    probe.is_some_and(|probe| probe(ctx))
}

/// Why captured presses end in the running pass: a cancel a service requested, else the window losing focus.
pub(crate) fn cancel_of_pass(ctx: &Context) -> Option<PressCancel> {
    let viewport = pass::current_viewport(ctx);
    let pass_nr = pass::pass_nr(ctx, viewport);
    let requested = PerViewport::<Memory>::with(ctx, viewport, |memory| memory.requested.filter(|(nr, _)| *nr == pass_nr).map(|(_, reason)| reason));
    requested.or_else(|| with_pass_pointer(ctx, |pointer| pointer.focus_lost).then_some(PressCancel::FocusLost))
}

/// Writes `shapes` of the arena rooted at `root` into the running pass's shape record under `layer`. Stack
/// positions are per arena, so each root's shapes stay apart.
pub(crate) fn record_shapes(ctx: &Context, layer: Layer, root: Id, shapes: impl IntoIterator<Item = Shape<Id>>) {
    record_shapes_in(ctx, pass::current_viewport(ctx), layer, root, shapes);
}

/// `record_shapes` for a caller that already read the running viewport.
pub(crate) fn record_shapes_in(ctx: &Context, viewport: ViewportId, layer: Layer, root: Id, shapes: impl IntoIterator<Item = Shape<Id>>) {
    PerViewport::<Shapes>::with(ctx, viewport, |record| record.this.entry(layer).or_default().entry(root).or_default().shapes.extend(shapes));
}

/// Records the egui widgets that stand for the root at `root` (`click` when they sense a click): the root's own at
/// step 8, and the `over` widgets of its members after their bodies.
pub(crate) fn record_widgets(ctx: &Context, layer: Layer, root: Id, widgets: &[Id], click: bool) {
    record_widgets_in(ctx, pass::current_viewport(ctx), layer, root, widgets, click);
}

/// `record_widgets` for a caller that already read the running viewport.
pub(crate) fn record_widgets_in(ctx: &Context, viewport: ViewportId, layer: Layer, root: Id, widgets: &[Id], click: bool) {
    PerViewport::<Shapes>::with(ctx, viewport, |record| {
        let entry = record.this.entry(layer).or_default().entry(root).or_default();
        entry.widgets.extend_from_slice(widgets);
        entry.click |= click;
    });
}

/// What the root `root` registered in the running pass of `viewport`, read for its part `part`: the egui widgets
/// that stand for the root, and the screen bounds of the root's shapes that take a press and outrank the part.
pub(crate) struct OverPart {
    /// The root's egui widgets: a press on them resolves among the root's shapes, not as a widget over the part.
    pub widgets: Vec<Id>,
    /// The shapes a press on the part goes to instead: a higher stack position (a member, an `over` part), level or
    /// sub-level (8.3). A shape of the same rank wins only by its distance to the point, so it is not one of them.
    pub above: Vec<Rect>,
}

/// What lies over the part `part` of the root `root` in the running pass of `viewport`, among the root's widgets and
/// shapes. Read at the end of the pass, when the record is complete.
pub(crate) fn over_part(ctx: &Context, viewport: ViewportId, root: Id, part: PartRef) -> OverPart {
    let (widgets, layers) = PerViewport::<Shapes>::with(ctx, viewport, |record| {
        let entries = || record.this.iter().filter_map(|(layer, roots)| roots.get(&root).map(|entry| (*layer, entry)));
        let rank = |shape: &Shape<Id>| (shape.position, shape.priority.level, shape.priority.sub);
        let own =
            entries().flat_map(|(_, entry)| entry.shapes.iter()).filter(|shape| shape.owner == root && shape.part == Some(part)).map(rank).max();
        let widgets: Vec<Id> = entries().flat_map(|(_, entry)| entry.widgets.iter().copied()).collect();
        let layers: Vec<(Layer, Vec<Rect>)> = entries()
            .map(|(layer, entry)| {
                let above = entry.shapes.iter().filter(|shape| shape.sense.takes_press() && own.is_some_and(|own| rank(shape) > own));
                (layer, above.map(|shape| shape.shape.bounds()).collect())
            })
            .collect();
        (widgets, layers)
    });
    let above = layers
        .into_iter()
        .flat_map(|(layer, bounds)| {
            let to_screen = areas::layer_to_screen(ctx, layer);
            bounds.into_iter().map(move |rect| to_screen * rect)
        })
        .collect();
    OverPart { widgets, above }
}

/// The parts of the root `root` that sense `sense` in last pass's shape record, in registration order.
#[cfg(feature = "testing")]
pub(crate) fn sensing_parts(ctx: &Context, root: Id, sense: craft_behavior::Sense) -> Vec<craft_behavior::PartRef> {
    let viewport = pass::current_viewport(ctx);
    PerViewport::<Shapes>::with(ctx, viewport, |record| {
        let mut parts = Vec::new();
        for layer in record.last.values() {
            if let Some(entry) = layer.get(&root) {
                parts.extend(entry.shapes.iter().filter(|shape| shape.sense.contains(sense)).filter_map(|shape| shape.part));
            }
        }
        parts
    })
}

/// The bounds of the part `part` of `owner` (the arena root or a member) in last pass's shape record: pointer-only
/// parts have no access node, so this record is where their place is read.
pub(crate) fn part_bounds(ctx: &Context, owner: Id, part: craft_behavior::PartRef) -> Option<egui::Rect> {
    let viewport = pass::current_viewport(ctx);
    PerViewport::<Shapes>::with(ctx, viewport, |record| {
        record
            .last
            .values()
            .flat_map(|roots| roots.values())
            .flat_map(|root| root.shapes.iter())
            .find(|shape| shape.owner == owner && shape.part == Some(part))
            .map(|shape| shape.shape.bounds())
    })
}

/// The bounds of the part `part` that senses `sense`, on a component other than `subject` and lying inside the
/// subject's own shapes in last pass's shape record: where a component's child keeps the part a drive acts on. When
/// several children match, the first by top edge, then start edge, then root id.
#[cfg(feature = "testing")]
pub(crate) fn child_part_bounds(ctx: &Context, subject: Id, part: craft_behavior::PartRef, sense: craft_behavior::Sense) -> Option<egui::Rect> {
    let viewport = pass::current_viewport(ctx);
    PerViewport::<Shapes>::with(ctx, viewport, |record| {
        let shapes = || record.last.values().flat_map(|roots| roots.values()).flat_map(|root| root.shapes.iter());
        let room = shapes().filter(|shape| shape.owner == subject).map(|shape| shape.shape.bounds()).reduce(|all, next| all.union(next))?;
        shapes()
            .filter(|shape| {
                shape.owner != subject && shape.part == Some(part) && shape.sense.contains(sense) && room.contains_rect(shape.shape.bounds())
            })
            .min_by(|a, b| {
                let (a_bounds, b_bounds) = (a.shape.bounds(), b.shape.bounds());
                a_bounds.min.y.total_cmp(&b_bounds.min.y).then(a_bounds.min.x.total_cmp(&b_bounds.min.x)).then(a.owner.value().cmp(&b.owner.value()))
            })
            .map(|shape| shape.shape.bounds())
    })
}

/// The roots and egui widgets the root viewport's latest pass registered: what bypass detection counts as made by
/// components.
#[cfg(feature = "testing")]
pub(crate) fn registered_widgets(ctx: &Context) -> std::collections::HashSet<Id> {
    PerViewport::<Shapes>::with(ctx, egui::ViewportId::ROOT, |record| {
        record
            .this
            .values()
            .flat_map(|roots| roots.iter())
            .flat_map(|(root, rec)| std::iter::once(*root).chain(rec.widgets.iter().copied()))
            .collect()
    })
}

/// The winner of a press at `at` among last pass's shapes of the root egui hands the press to, on the topmost
/// layer under it, with that root: what the focus service classifies the press by (8.3, H7). A press whose egui
/// target is no root's widget has no winner. A root's widget covers its whole rect, so when no shape of the click
/// root lies under the press (a title row between its parts), the press goes to the drag root below it. Shapes of
/// transformed bodies lie on their own layers.
pub(crate) fn press_winner(ctx: &Context, at: Pos2, targets: PressTargets) -> Option<(Winner<Id>, Id)> {
    let layer = areas::layer_at(ctx, at)?;
    let viewport = pass::current_viewport(ctx);
    let candidates = PerViewport::<Shapes>::with(ctx, viewport, |record| {
        let roots = || record.last.values().flat_map(|roots| roots.iter());
        let root_of = |widget: Option<Id>| {
            let widget = widget?;
            roots().find(|(_, root)| root.widgets.contains(&widget)).map(|(id, _)| *id)
        };
        let mut found: Vec<(Id, Vec<Shape<Id>>)> = Vec::new();
        for root in [root_of(targets.click), root_of(targets.drag)].into_iter().flatten() {
            if found.iter().all(|(seen, _)| *seen != root)
                && let Some(shapes) = record.last.get(&layer).and_then(|roots| roots.get(&root))
            {
                found.push((root, shapes.shapes.clone()));
            }
        }
        found
    });
    let at = areas::layer_map(ctx, layer).map_or(at, |map| map.to_layer(at));
    candidates.into_iter().find_map(|(root, shapes)| resolve(&shapes, at, ResolveFor::Press).map(|winner| (winner, root)))
}

/// The screen bounds of the root `root` from its shape record, with the viewport and layer they were recorded in: the
/// running pass's shapes when the root has registered in it, else the previous pass's. Searched over the live
/// viewports, so the input hook reads it before the pass begins.
pub(crate) fn root_bounds(ctx: &Context, root: Id) -> Option<(ViewportId, Layer, egui::Rect)> {
    pass::live_viewports(ctx).into_iter().find_map(|viewport| {
        let (layer, bounds) = PerViewport::<Shapes>::with(ctx, viewport, |record| {
            [&record.this, &record.last].into_iter().find_map(|layers| {
                layers.iter().find_map(|(layer, roots)| {
                    let shapes = &roots.get(&root)?.shapes;
                    let bounds = shapes.iter().fold(egui::Rect::NOTHING, |bounds, shape| bounds.union(shape.shape.bounds()));
                    bounds.is_positive().then_some((*layer, bounds))
                })
            })
        })?;
        let screen =
            areas::layer_map(ctx, layer).map_or(bounds, |map| egui::Rect::from_two_pos(map.to_screen(bounds.min), map.to_screen(bounds.max)));
        Some((viewport, layer, screen))
    })
}
