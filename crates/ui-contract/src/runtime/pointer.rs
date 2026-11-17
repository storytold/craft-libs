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
use egui::{Context, Id, IdMap, Pos2, Rect, Vec2, ViewportId};
use std::collections::HashMap;
use std::sync::Arc;

/// Points one wheel line adds to `PointerFrame.scroll`; scroll owners apply their own notch (the wheel service).
pub(crate) const WHEEL_LINE_POINTS: f32 = 40.0;

/// Points one wheel page adds to `PointerFrame.scroll`.
pub(crate) const WHEEL_PAGE_POINTS: f32 = 400.0;

/// The pointer input of one pass of one viewport, egui-converted. Input enters a frame's first pass only (egui hands
/// the frame's events to it, and injected steps and held-back presses follow the frame): a rerun pass of the
/// frame carries no events, so its `delta`, `press`, `release` and `scroll` are empty while the held facts (`pos`,
/// `buttons`) persist. A consumer applies what its pass hands it and never turns a held fact into an event.
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
    /// The modifiers held, with the ones the app latches for the user (a Modifier Keys panel) added: the app holds
    /// them down for every press and drag. Key chords read the keyboard's own modifiers instead.
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
    /// The pass's summed `Event::Rotate` radians.
    pub rotation: f32,
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

/// What the begin hook read of the pass, for the sensing core and the services. Built from the pass's own events,
/// so a rerun pass of a frame (see [`PointerFrame`]) reads none.
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
pub(crate) struct Memory {
    /// The latest primary press: where, when, its click count and the modifiers held with it.
    last_press: Option<(Pos2, f64, u32, Mods)>,
    press_targets: PressTargets,
    /// A cancel a service requested for the running pass.
    requested: Option<(u64, PressCancel)>,
    /// Whether any arena of the viewport holds an open press; sensing registers it, as the layer above.
    press_probe: Option<fn(&Context) -> bool>,
    /// The arena roots whose members the next pass resolves without (`withhold_members`), with the pass that asked.
    withheld: IdMap<u64>,
}

/// What one root registered on one layer: its shapes and its egui widgets.
#[derive(Clone, Default)]
struct RootRecord {
    shapes: Vec<Shape<Id>>,
    /// Where each shape of `shapes` stands, at the same index.
    placed: Vec<Placed>,
    /// The root's egui widgets: the click and drag targets that stand for it.
    widgets: Vec<Id>,
    /// The root's widgets sense a click.
    click: bool,
}

/// Where a recorded shape stands: its place in the pass's paint order (the registration that recorded it; later is on
/// top) and the clip it shows through, in its layer's space.
#[derive(Clone, Copy)]
struct Placed {
    order: u32,
    clip: Rect,
}

type LayerRecord = HashMap<Layer, IdMap<RootRecord>>;

/// The transformed body layers of a pass: each layer's root and the layer the root itself is shown on.
type BodyRecord = HashMap<Layer, (Id, Layer)>;

#[derive(Default)]
pub(crate) struct Shapes {
    this: LayerRecord,
    last: LayerRecord,
    this_bodies: BodyRecord,
    last_bodies: BodyRecord,
    /// The paint order the running pass's next registration takes.
    next: u32,
    /// The press winner found over `last`, for the point and targets it was asked for: every root asks for the
    /// same press.
    winner: Option<(Pos2, PressTargets, PressWinners)>,
}

/// The begin hook (H5): converts the pass's pointer input.
pub(crate) fn begin(ctx: &Context, hit: HitSizes, latched: craft_keymap::Mods) {
    let viewport = pass::current_viewport(ctx);
    let pass_nr = pass::pass_nr(ctx, viewport);
    let mut read = input::pointer_read(ctx);
    // Every pass reads its own input and the frame's latches, so a second pass of a frame builds the same
    // modifiers from its own (event-free) input and counts no press twice.
    let mods = with_latched(read.mods, latched);
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
                *clicks = count_clicks(memory.last_press, *pos, *time, mods, hit.slop);
                memory.last_press = Some((*pos, *time, *clicks, mods));
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
        mods,
        time: read.time,
        kind: read.kind,
        sample: read.sample,
        multi_touch: read.multi_touch.map(|(translation, zoom, rotation, center)| MultiTouch::new(translation, zoom, rotation, center)),
        scroll,
        zoom: wheel_zoom,
        rotation: read.rotation,
        slop: hit.slop,
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
    PerViewport::<Shapes>::with(ctx, viewport, |shapes| {
        shapes.last = std::mem::take(&mut shapes.this);
        shapes.last_bodies = std::mem::take(&mut shapes.this_bodies);
        shapes.next = 0;
        shapes.winner = None;
    });
}

/// The modifiers `held` with the app's `latched` ones added, field by field: `Mods` has no operator for it.
fn with_latched(held: Mods, latched: Mods) -> Mods {
    Mods { cmd: held.cmd || latched.cmd, ctrl: held.ctrl || latched.ctrl, alt: held.alt || latched.alt, shift: held.shift || latched.shift }
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

/// The buttons held in the latest pass of `viewport`, which may be a child window whose pass ran inside this one.
pub(crate) fn buttons_of(ctx: &Context, viewport: ViewportId) -> Buttons {
    PerViewport::<Stamped<Arc<PassPointer>>>::with_pass(
        ctx,
        viewport,
        pass::pass_nr(ctx, viewport),
        |_, _| {},
        |record| record.frame.as_ref().map_or(Buttons::empty(), |frame| frame.buttons),
    )
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
        rotation: 0.0,
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

/// Whether the arena of `root` resolves the presses and hover of the next pass without its members' shapes. A canvas
/// host sets it each pass from the tool it runs: while a momentary tool (the Hand of a held Space, the Zoom of a held
/// Command) is in effect, a press anywhere on the canvas lands on the canvas itself, over the session members on its
/// body (a transform box, a crop frame), and the host gives it to that tool. A press already open keeps its target.
pub fn withhold_members(ctx: &Context, root: Id, withheld: bool) {
    if !crate::runtime::measure::effect(ctx, "pointer::withhold_members") {
        return;
    }
    let viewport = pass::current_viewport(ctx);
    let pass_nr = pass::pass_nr(ctx, viewport);
    PerViewport::<Memory>::with(ctx, viewport, |memory| {
        if withheld {
            memory.withheld.insert(root, pass_nr);
        } else {
            memory.withheld.remove(&root);
        }
    });
}

/// Whether the arena of `root` resolves the running pass of `viewport` (`pass_nr`) without its members: its host
/// asked so in this pass or the one before, the pass whose shapes this one resolves over.
pub(crate) fn members_withheld(ctx: &Context, viewport: ViewportId, pass_nr: u64, root: Id) -> bool {
    PerViewport::<Memory>::with(ctx, viewport, |memory| memory.withheld.get(&root).is_some_and(|asked| asked + 1 >= pass_nr))
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

/// Writes `shapes` of the arena rooted at `root`, shown through `clip` (layer space), into the running pass's shape
/// record under `layer`, above every shape recorded before them in the pass. Stack positions are per arena, so each
/// root's shapes stay apart.
pub(crate) fn record_shapes(ctx: &Context, layer: Layer, root: Id, clip: Rect, shapes: impl IntoIterator<Item = Shape<Id>>) {
    record_shapes_in(ctx, pass::current_viewport(ctx), layer, root, clip, shapes);
}

/// `record_shapes` for a caller that already read the running viewport.
pub(crate) fn record_shapes_in(ctx: &Context, viewport: ViewportId, layer: Layer, root: Id, clip: Rect, shapes: impl IntoIterator<Item = Shape<Id>>) {
    let shapes: Vec<Shape<Id>> = shapes.into_iter().collect();
    crate::runtime::replay::journal(ctx, || {
        let shapes = shapes.clone();
        move |ctx: &Context| record_shapes(ctx, layer, root, clip, shapes.iter().cloned())
    });
    PerViewport::<Shapes>::with(ctx, viewport, |record| {
        let order = record.next;
        record.next += 1;
        let entry = record.this.entry(layer).or_default().entry(root).or_default();
        entry.shapes.extend(shapes);
        entry.placed.resize(entry.shapes.len(), Placed { order, clip });
    });
}

/// Records that `layer` shows the transformed body of the root `root`, which is itself shown on `parent`: a press on
/// the body layer reaches the root's own shapes beneath the body's.
pub(crate) fn record_body_layer(ctx: &Context, layer: Layer, parent: Layer, root: Id) {
    crate::runtime::replay::journal(ctx, || move |ctx: &Context| record_body_layer(ctx, layer, parent, root));
    PerViewport::<Shapes>::with(ctx, pass::current_viewport(ctx), |record| {
        record.this_bodies.insert(layer, (root, parent));
    });
}

/// Records the egui widgets that stand for the root at `root` (`click` when they sense a click): the root's own at
/// step 8, and the `over` widgets of its members after their bodies.
pub(crate) fn record_widgets(ctx: &Context, layer: Layer, root: Id, widgets: &[Id], click: bool) {
    record_widgets_in(ctx, pass::current_viewport(ctx), layer, root, widgets, click);
}

/// `record_widgets` for a caller that already read the running viewport.
pub(crate) fn record_widgets_in(ctx: &Context, viewport: ViewportId, layer: Layer, root: Id, widgets: &[Id], click: bool) {
    crate::runtime::replay::journal(ctx, || {
        let widgets = widgets.to_vec();
        move |ctx: &Context| record_widgets(ctx, layer, root, &widgets, click)
    });
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

/// The hit areas the running pass has registered so far for the components `of` takes, in screen points: each shape's
/// bounds grown to its least size across, with the component and part it belongs to.
pub(crate) fn hit_areas(ctx: &Context, of: impl Fn(Id) -> bool) -> Vec<(Id, Option<PartRef>, egui::Rect)> {
    let viewport = pass::current_viewport(ctx);
    let found: Vec<(Layer, Id, Option<PartRef>, egui::Rect)> = PerViewport::<Shapes>::with(ctx, viewport, |record| {
        let mut found = Vec::new();
        for (layer, roots) in &record.this {
            for shape in roots.values().flat_map(|root| root.shapes.iter()).filter(|shape| of(shape.owner)) {
                let bounds = shape.shape.bounds();
                let grow = (egui::Vec2::splat(shape.min_across) - bounds.size()).max(egui::Vec2::ZERO) / 2.0;
                found.push((*layer, shape.owner, shape.part, bounds.expand2(grow)));
            }
        }
        found
    });
    found
        .into_iter()
        .map(|(layer, owner, part, rect)| {
            let to_screen = areas::layer_to_screen(ctx, layer);
            (owner, part, to_screen * rect)
        })
        .collect()
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
    owned_part_bounds(ctx, |shape_owner| shape_owner == owner, part)
}

/// Whether the part `part` of `owner` (the arena root or a member) senses all of `sense` in last pass's shape record.
#[cfg(feature = "testing")]
pub(crate) fn part_senses(ctx: &Context, owner: Id, part: craft_behavior::PartRef, sense: craft_behavior::Sense) -> bool {
    let viewport = pass::current_viewport(ctx);
    PerViewport::<Shapes>::with(ctx, viewport, |record| {
        record
            .last
            .values()
            .flat_map(|roots| roots.values())
            .flat_map(|root| root.shapes.iter())
            .any(|shape| shape.owner == owner && shape.part == Some(part) && shape.sense.contains(sense))
    })
}

/// [`part_bounds`] of the owner whose AccessKit node is `node`, for a test that holds the node it found, not the id.
#[cfg(feature = "testing")]
pub(crate) fn node_part_bounds(ctx: &Context, node: crate::runtime::egui::access::NodeId, part: craft_behavior::PartRef) -> Option<egui::Rect> {
    owned_part_bounds(ctx, |shape_owner| crate::runtime::egui::access::node_id(shape_owner) == node, part)
}

fn owned_part_bounds(ctx: &Context, owner: impl Fn(Id) -> bool, part: craft_behavior::PartRef) -> Option<egui::Rect> {
    let viewport = pass::current_viewport(ctx);
    PerViewport::<Shapes>::with(ctx, viewport, |record| {
        record
            .last
            .values()
            .flat_map(|roots| roots.values())
            .flat_map(|root| root.shapes.iter())
            .find(|shape| owner(shape.owner) && shape.part == Some(part))
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

/// Whether `id` is a root or an egui widget that a component registered in the running pass of `viewport`.
pub(crate) fn made_by_component(ctx: &Context, viewport: ViewportId, id: Id) -> bool {
    PerViewport::<Shapes>::with(ctx, viewport, |record| {
        record.this.values().any(|roots| roots.contains_key(&id) || roots.values().any(|root| root.widgets.contains(&id)))
    })
}

/// The roots a press at `at` (screen points) goes to, each with the shape of it that wins it: its click's and its
/// drag's (8.3, H7). On the topmost layer under the press, each root recorded there in the last pass resolves its own
/// shapes by the arena rule, and the roots rank by their winning shapes' place in the pass's paint order, top first. The
/// top one takes every kind of press its shape senses. Under a top shape that only drags (a window's resize grip), the
/// click goes to the first root below whose shape clicks; under one that only clicks (a button), the drag goes to the
/// first root below whose shape drags, when that root takes no click anywhere. A transformed body's layer shows its root's own shapes
/// beneath everything on it. A shape counts only where its clip shows it; at a point no clip on the layer holds (one
/// automation names past the window), a shape counts when some of it shows inside its clip. A press egui's hit test
/// gives to a widget no component registered (a raw egui widget on top) goes to no root.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct PressWinners {
    pub click: Option<(Winner<Id>, Id)>,
    pub drag: Option<(Winner<Id>, Id)>,
}

impl PressWinners {
    /// The root that starts the press for its kinds of press, with the kinds it takes.
    pub(crate) fn senses_of(&self, root: Id) -> craft_behavior::Sense {
        let mut sense = craft_behavior::Sense::empty();
        if self.click.is_some_and(|(_, owner)| owner == root) {
            sense |= craft_behavior::Sense::CLICK;
        }
        if self.drag.is_some_and(|(_, owner)| owner == root) {
            sense |= craft_behavior::Sense::DRAG;
        }
        sense
    }
}

pub(crate) fn press_winners(ctx: &Context, at: Pos2, targets: PressTargets) -> PressWinners {
    let viewport = pass::current_viewport(ctx);
    let found = PerViewport::<Shapes>::with(ctx, viewport, |record| {
        record.winner.filter(|(point, asked, _)| *point == at && *asked == targets).map(|(_, _, found)| found)
    });
    if let Some(found) = found {
        return found;
    }
    let found = find_press_winners(ctx, viewport, at, targets);
    PerViewport::<Shapes>::with(ctx, viewport, |record| record.winner = Some((at, targets, found)));
    found
}

/// The root a press at `at` belongs to, with its winning shape: the click's root (`press_winners`), else the drag's.
/// What the focus service classifies a press by and a host asks a press about.
pub(crate) fn press_winner(ctx: &Context, at: Pos2, targets: PressTargets) -> Option<(Winner<Id>, Id)> {
    let winners = press_winners(ctx, at, targets);
    winners.click.or(winners.drag)
}

fn find_press_winners(ctx: &Context, viewport: ViewportId, at: Pos2, targets: PressTargets) -> PressWinners {
    let Some(layer) = areas::layer_at(ctx, at).or_else(|| layer_past_window(ctx, viewport, at)) else {
        if let Some(test) = diag_test() {
            eprintln!("DIAG {test}: press_winner at {at:?}: no layer");
        }
        return PressWinners::default();
    };
    if let Some(test) = diag_test().filter(|test| ["liquify", "half_plane", "crop_drag"].iter().any(|name| test.contains(name))) {
        diag_layers(ctx, viewport, at, layer, test);
    }
    let to_layer = |layer: Layer| areas::layer_map(ctx, layer).map_or(at, |map| map.to_layer(at));
    let body = PerViewport::<Shapes>::with(ctx, viewport, |record| record.last_bodies.get(&layer).copied());
    let beneath = body.map(|(root, parent)| (root, parent, to_layer(parent)));
    let on_layer = to_layer(layer);
    let found = PerViewport::<Shapes>::with(ctx, viewport, |record| {
        // Bottom to top: the body layer's root on its own layer, then the roots on the layer in paint order.
        let mut stack: Vec<(u8, Id, &RootRecord, Pos2)> = Vec::new();
        if let Some((root, parent, point)) = beneath
            && let Some(entry) = record.last.get(&parent).and_then(|roots| roots.get(&root))
        {
            stack.push((0, root, entry, point));
        }
        if let Some(roots) = record.last.get(&layer) {
            stack.extend(roots.iter().map(|(root, entry)| (1, *root, entry, on_layer)));
        }
        let shown = stack.iter().any(|(_, _, entry, point)| entry.placed.iter().any(|placed| placed.clip.contains(*point)));
        if let Some(test) = diag_test() {
            let hits: Vec<String> = stack
                .iter()
                .flat_map(|(tier, root, entry, point)| {
                    entry.shapes.iter().zip(&entry.placed).filter(|(shape, _)| shape.hits(*point)).map(move |(shape, placed)| {
                        format!(
                            "[tier {tier} root {} owner {} part {:?} sense {:?} level {:?} pos {} order {} clip_in {}]",
                            diag_id(*root),
                            diag_id(shape.owner),
                            shape.part,
                            shape.sense,
                            shape.priority.level,
                            shape.position,
                            placed.order,
                            placed.clip.contains(*point)
                        )
                    })
                })
                .collect();
            let foreign = |widget: Option<Id>| widget.map(|widget| (diag_id(widget), !registered_in(&record.last, widget)));
            eprintln!(
                "DIAG {test}: press_winner at {at:?} layer {layer:?} body {:?} click {:?} drag {:?} shown {shown} hits {hits:?}",
                beneath.map(|(root, parent, _)| (diag_id(root), parent)),
                foreign(targets.click),
                foreign(targets.drag)
            );
        }
        if targets.click.is_some_and(|widget| !registered_in(&record.last, widget)) {
            return PressWinners::default();
        }
        let mut ranked: Vec<_> = stack
            .into_iter()
            .filter_map(|(tier, root, entry, point)| {
                let shows = |index: usize| {
                    let (shape, placed) = (&entry.shapes[index], entry.placed[index]);
                    if shown { placed.clip.contains(point) } else { shape.shape.bounds().intersects(placed.clip) }
                };
                let kept: Vec<usize> = (0..entry.shapes.len())
                    .filter(|&index| entry.shapes[index].sense.takes_press() && entry.shapes[index].hits(point) && shows(index))
                    .collect();
                let shapes: Vec<Shape<Id>> = kept.iter().map(|&index| entry.shapes[index].clone()).collect();
                let mut winner = resolve(&shapes, point, ResolveFor::Press)?;
                winner.index = kept[winner.index];
                Some(((tier, entry.placed[winner.index].order), winner, root, entry.click))
            })
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0));
        let mut winners = PressWinners::default();
        for (_, winner, root, root_clicks) in ranked {
            let (clicks, drags) = (winner.sense.contains(craft_behavior::Sense::CLICK), winner.sense.contains(craft_behavior::Sense::DRAG));
            match (winners.click.is_some(), winners.drag.is_some()) {
                // The top root takes what its shape senses; one that takes both kinds takes the whole press.
                (false, false) => {
                    winners.click = clicks.then_some((winner, root));
                    winners.drag = drags.then_some((winner, root));
                    if clicks && drags {
                        break;
                    }
                }
                // Under a top shape that only drags, the first one below that clicks takes the click.
                (false, true) => {
                    if clicks {
                        winners.click = Some((winner, root));
                        break;
                    }
                }
                // Under a top shape that only clicks, the first root below whose shape drags takes the drag, unless the
                // root takes clicks anywhere: a drag-only root (a scroll view) is dragged from a button in it.
                (true, false) => {
                    if clicks || root_clicks {
                        break;
                    }
                    if drags {
                        winners.drag = Some((winner, root));
                        break;
                    }
                }
                (true, true) => break,
            }
        }
        winners
    });
    if let Some(test) = diag_test() {
        let show = |found: Option<(Winner<Id>, Id)>| found.map(|(winner, root)| (diag_id(root), diag_id(winner.owner), winner.part, winner.sense));
        eprintln!("DIAG {test}: press_winner at {at:?} -> click {:?} drag {:?}", show(found.click), show(found.drag));
    }
    found
}

/// The layer a press past the window goes to, where egui has none (only automation names such a point: a document
/// point off the window): the topmost layer of last pass's shape record with a shape there that takes a press and
/// shows some of itself inside its clip.
fn layer_past_window(ctx: &Context, viewport: ViewportId, at: Pos2) -> Option<Layer> {
    let mut layers: Vec<Layer> = PerViewport::<Shapes>::with(ctx, viewport, |record| record.last.keys().copied().collect());
    layers.sort_by_key(|layer| std::cmp::Reverse(areas::layer_position(ctx, *layer)));
    layers.into_iter().find(|layer| {
        let point = areas::layer_map(ctx, *layer).map_or(at, |map| map.to_layer(at));
        PerViewport::<Shapes>::with(ctx, viewport, |record| {
            record.last.get(layer).is_some_and(|roots| {
                roots.values().any(|entry| {
                    entry
                        .shapes
                        .iter()
                        .zip(&entry.placed)
                        .any(|(shape, placed)| shape.sense.takes_press() && shape.hits(point) && shape.shape.bounds().intersects(placed.clip))
                })
            })
        })
    })
}

/// DIAG: every layer of last pass's shape record at `at`, top first: its place in egui's order, whether it is the
/// press's layer, the body it shows, and each root's shapes under the point (sensing or not).
fn diag_layers(ctx: &Context, viewport: ViewportId, at: Pos2, chosen: Layer, test: &str) {
    let mut layers: Vec<Layer> = PerViewport::<Shapes>::with(ctx, viewport, |record| record.last.keys().copied().collect());
    layers.sort_by_key(|layer| std::cmp::Reverse(areas::layer_position(ctx, *layer)));
    for layer in layers {
        let point = areas::layer_map(ctx, layer).map_or(at, |map| map.to_layer(at));
        let position = areas::layer_position(ctx, layer);
        let interactable = egui::AreaState::load(ctx, layer.id).map(|state| (state.interactable, state.rect()));
        let line = PerViewport::<Shapes>::with(ctx, viewport, |record| {
            let body = record.last_bodies.get(&layer).map(|(root, parent)| (diag_id(*root), *parent));
            let roots: Vec<String> = record
                .last
                .get(&layer)
                .into_iter()
                .flat_map(|roots| roots.iter())
                .map(|(root, entry)| {
                    let under: Vec<String> = entry
                        .shapes
                        .iter()
                        .zip(&entry.placed)
                        .filter(|(shape, _)| shape.hits(point))
                        .map(|(shape, placed)| {
                            format!("{} {:?} {:?} clip_in {}", diag_id(shape.owner), shape.part, shape.sense, placed.clip.contains(point))
                        })
                        .collect();
                    format!("root {} shapes {} under {under:?}", diag_id(*root), entry.shapes.len())
                })
                .collect();
            format!("layer {layer:?} at {position} area {interactable:?} chosen {} body {body:?} point {point:?} roots {roots:?}", layer == chosen)
        });
        eprintln!("DIAG {test}: layers at {at:?}: {line}");
    }
}

/// DIAG: the test under study this thread runs, if any.
pub(crate) fn diag_test() -> Option<&'static str> {
    const TESTS: [&str; 8] = [
        "delete_removes_the_plane_and_escape_drops_a_half_plane_first",
        "a_window_mounts_edges_resize_the_window_and_the_childs_press_keeps_its_click",
        "gradient_map_stop_picker_closed_without_a_colour_changes_nothing",
        "photo_filter_colour_picker_previews_live_cancel_restores_and_ok_is_one_step",
        "every_paragraph_box_handle_resizes_the_box_with_snapping_on",
        "click_inside_the_paragraph_box_off_the_glyphs_moves_the_caret",
        "a_crop_drag_begun_before_the_dialog_still_ends_under_it",
        "move_pointer_still_reaches_the_liquify_workspace",
    ];
    // Trials run on unnamed pool threads: there the run names its trial in `CRAFT_DIAG_TEST`.
    let thread = std::thread::current();
    let name = std::env::var("CRAFT_DIAG_TEST").ok().or_else(|| thread.name().map(str::to_owned))?;
    TESTS.into_iter().find(|test| name.ends_with(test))
}

/// DIAG: the tail of an id's path.
pub(crate) fn diag_id(id: Id) -> String {
    let text = format!("{id:?}");
    text.char_indices().rev().nth(90).map_or(text.clone(), |(at, _)| text[at..].to_owned())
}

/// Whether some root registered the egui widget `widget` in `layers`.
fn registered_in(layers: &LayerRecord, widget: Id) -> bool {
    layers.values().any(|roots| roots.values().any(|root| root.widgets.contains(&widget)))
}

/// Whether the egui widget `widget`, one of the latest press's targets, is one no component registered in the pass
/// the press was hit-tested against: a raw egui widget.
pub(crate) fn foreign_target(ctx: &Context, widget: Id) -> bool {
    PerViewport::<Shapes>::with(ctx, pass::current_viewport(ctx), |record| !registered_in(&record.last, widget))
}

/// Every shape of the last pass on the topmost layer at `at` (screen points) whose area contains it, as
/// `(owner, part)`, sensing or not, in no particular order; empty where no layer is. The hit record a press is
/// classified on, for rules that ask what lies under a point rather than who wins it.
pub fn shapes_at(ctx: &Context, at: Pos2) -> Vec<(Id, Option<PartRef>)> {
    let Some(layer) = areas::layer_at(ctx, at) else { return Vec::new() };
    // Every shape recorded on a layer lies in that layer's space.
    let at = areas::layer_map(ctx, layer).map_or(at, |map| map.to_layer(at));
    PerViewport::<Shapes>::with(ctx, pass::current_viewport(ctx), |record| {
        record
            .last
            .get(&layer)
            .into_iter()
            .flat_map(|roots| roots.values())
            .flat_map(|root| root.shapes.iter())
            .filter(|shape| shape.hits(at))
            .map(|shape| (shape.owner, shape.part))
            .collect()
    })
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
