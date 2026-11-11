//! Window hit testing: resize edges, caption regions, the hit sink the OS asks, and exclusive parts.

use super::Record;
#[cfg(doc)]
use super::WindowHost;
use crate::runtime::egui::input::RawInput;
use crate::runtime::egui::{pass, scale, widgets};
use crate::runtime::frame::{PassStep, PerViewport, Stamped};
use crate::runtime::pointer;
use craft_behavior::PartRef;
use craft_window::Button;
use craft_window::native::{CaptionEvent, Edge, Hits, Zone};
use egui::{Context, Id, Pos2, Rect, ViewportId};
use std::sync::{Arc, Mutex};

/// An edge or corner a window resize starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResizeEdge {
    /// The top edge.
    Top,
    /// The bottom edge.
    Bottom,
    /// The left edge.
    Left,
    /// The right edge.
    Right,
    /// The top left corner.
    TopLeft,
    /// The top right corner.
    TopRight,
    /// The bottom left corner.
    BottomLeft,
    /// The bottom right corner.
    BottomRight,
}

/// What a part of the window does for the operating system: a press on it moves the window, resizes it from an edge,
/// or is a caption button. A component tags a part with it through `PartBuilder::exclusive`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WindowHit {
    /// Drags the window.
    Move,
    /// Resizes the window from the edge or corner.
    Resize(ResizeEdge),
    /// A caption button.
    Caption(Button),
}

/// Where the exclusive parts of one window lie, in points of its viewport: the window procedure answers a Windows
/// hit test from them. Read it with [`WindowHost::caption_regions`].
#[derive(Clone, Debug, PartialEq, Default)]
#[non_exhaustive]
pub struct CaptionRegions {
    /// The caption buttons.
    pub buttons: Vec<(Button, Rect)>,
    /// The edge and corner zones.
    pub edges: Vec<(ResizeEdge, Rect)>,
    /// The areas that drag the window.
    pub moves: Vec<Rect>,
    /// For each drag area, at the same index in `moves`, the parts where an in-app hit lies over its band: an
    /// interactive widget shown above the band's component, such as a bar's button or menu. A press there goes to
    /// that widget under the press-target rule (8.3), so the window does not move from that area there. A cover
    /// belongs to its own area only: a drag area shown above another (a modal's strip over the title bar) covers
    /// the lower one and still answers itself.
    pub covers: Vec<Vec<Rect>>,
}

impl CaptionRegions {
    /// What lies at `point`: a button wins over an edge, and an edge over a drag area. A drag area answers only where
    /// none of its covers lies, so its band is the top in-app hit there.
    pub fn hit(&self, point: Pos2) -> Option<WindowHit> {
        let moves =
            || self.moves.iter().zip(&self.covers).any(|(rect, covers)| rect.contains(point) && !covers.iter().any(|cover| cover.contains(point)));
        self.buttons
            .iter()
            .find(|(_, rect)| rect.contains(point))
            .map(|(button, _)| WindowHit::Caption(*button))
            .or_else(|| self.edges.iter().find(|(_, rect)| rect.contains(point)).map(|(edge, _)| WindowHit::Resize(*edge)))
            .or_else(|| moves().then_some(WindowHit::Move))
    }
}

/// What a window's last complete pass published for its hit test: the regions and the scale they are in.
#[derive(Clone, Debug)]
struct Published {
    regions: CaptionRegions,
    /// Physical pixels per point of the viewport, interface zoom included.
    pixels_per_point: f32,
}

/// The Windows hit test of one window: the window procedure asks [`HitSink::answer`] where the OS asks for a hit
/// test, and the answer comes from the exclusive parts the window's last pass declared, so the caption buttons,
/// edges and drag areas the app draws are the ones the OS acts on (snap layouts, native move and resize). The sink
/// holds the scale with the regions, so the window procedure needs nothing from egui.
///
/// Over the maximize button the pointer lies in the non-client area and the app gets no client pointer, so the
/// procedure reports the button's hover, press and release through [`Hits::caption`]; the next pass turns them into
/// the pointer events the button would have had. [`WindowHost::install_hit_sink`] makes the sink; the platform layer
/// that owns the window procedure holds it.
#[derive(Clone, Debug, Default)]
pub struct HitSink {
    published: Arc<Mutex<Option<Published>>>,
    /// The caption events the window procedure reported since the last pass.
    events: Arc<Mutex<Vec<CaptionEvent>>>,
    /// The window to repaint when an event arrives from the window procedure.
    wake: Option<(Context, ViewportId)>,
}

impl HitSink {
    pub(super) fn linked(ctx: &Context, viewport: ViewportId) -> Self {
        HitSink { wake: Some((ctx.clone(), viewport)), ..HitSink::default() }
    }

    /// What lies at `pixel`, in physical pixels from the top left of the window's client area: the hit test's screen
    /// point after `ScreenToClient`. `None` where no exclusive part lies there: the window procedure leaves the hit
    /// test to the window's content.
    pub fn answer(&self, pixel: Pos2) -> Option<WindowHit> {
        let published = self.published.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let Published { regions, pixels_per_point } = published.as_ref()?;
        regions.hit((pixel.to_vec2() / *pixels_per_point).to_pos2())
    }

    fn publish(&self, published: Option<Published>) {
        *self.published.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = published;
    }

    fn take_events(&self) -> Vec<CaptionEvent> {
        std::mem::take(&mut *self.events.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
    }
}

impl Hits for HitSink {
    fn zone_at(&self, pixel: [f32; 2]) -> Option<Zone> {
        Some(match self.answer(Pos2::new(pixel[0], pixel[1]))? {
            WindowHit::Move => Zone::Move,
            WindowHit::Resize(edge) => Zone::Edge(edge_of(edge)),
            WindowHit::Caption(button) => Zone::Button(button),
        })
    }

    fn caption(&self, event: CaptionEvent) {
        self.events.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(event);
        if let Some((ctx, viewport)) = &self.wake {
            ctx.request_repaint_of(*viewport);
        }
    }
}

/// The platform layer's name for `edge`.
fn edge_of(edge: ResizeEdge) -> Edge {
    match edge {
        ResizeEdge::Top => Edge::Top,
        ResizeEdge::Bottom => Edge::Bottom,
        ResizeEdge::Left => Edge::Left,
        ResizeEdge::Right => Edge::Right,
        ResizeEdge::TopLeft => Edge::TopLeft,
        ResizeEdge::TopRight => Edge::TopRight,
        ResizeEdge::BottomLeft => Edge::BottomLeft,
        ResizeEdge::BottomRight => Edge::BottomRight,
    }
}

/// Turns the caption events the window procedure reported for `viewport` into the pointer events the caption button
/// would have had, so the button looks and acts as on the other platforms: a hover moves the pointer onto it, a press
/// presses the primary button, a release releases it there (a click), and a leave or a cancelled press moves the
/// pointer away first so the release is no click.
pub(super) fn replay_caption_events(ctx: &Context, viewport: ViewportId, input: &mut RawInput) {
    let Some(sink) = PerViewport::<Record>::with(ctx, viewport, |record| record.sink.clone()) else { return };
    let events = sink.take_events();
    if events.is_empty() {
        return;
    }
    let (buttons, moves) =
        with_exclusive(ctx, viewport, |exclusive| exclusive.done.as_ref().map(|regions| (regions.buttons.clone(), regions.moves.clone())))
            .unwrap_or_default();
    let center = |button: Button| buttons.iter().find(|(held, _)| *held == button).map(|(_, rect)| rect.center());
    let press = |pos, button, pressed| egui::Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers::NONE };
    let primary = |pos, pressed| press(pos, egui::PointerButton::Primary, pressed);
    let scale = scale::pixels_per_point(ctx).max(f32::EPSILON);
    // Only a position on a drag area is replayed there.
    let on_band = |pixel: [f32; 2]| {
        let at = Pos2::new(pixel[0] / scale, pixel[1] / scale);
        moves.iter().any(|rect| rect.contains(at)).then_some(at)
    };
    let mut replay = Vec::new();
    for event in events {
        match event {
            CaptionEvent::Hover(caption) => replay.extend(center(caption).map(egui::Event::PointerMoved)),
            CaptionEvent::Press(caption) => {
                if let Some(at) = center(caption) {
                    replay.extend([egui::Event::PointerMoved(at), primary(at, true)]);
                }
            }
            CaptionEvent::Release(caption) => replay.extend(center(caption).map(|at| primary(at, false))),
            CaptionEvent::Leave => replay.push(egui::Event::PointerGone),
            CaptionEvent::Cancel => replay.extend([egui::Event::PointerMoved(OUTSIDE), primary(OUTSIDE, false), egui::Event::PointerGone]),
            CaptionEvent::BandDouble(pixel) => {
                if let Some(at) = on_band(pixel) {
                    replay.extend([egui::Event::PointerMoved(at), primary(at, true), primary(at, false), primary(at, true), primary(at, false)]);
                }
            }
            CaptionEvent::BandMenu(pixel) => {
                if let Some(at) = on_band(pixel) {
                    replay.extend([
                        egui::Event::PointerMoved(at),
                        press(at, egui::PointerButton::Secondary, true),
                        press(at, egui::PointerButton::Secondary, false),
                    ]);
                }
            }
            _ => {}
        }
    }
    // Before the events winit delivered in the same frame: a pointer that moved on from the button to the client
    // area is the later fact.
    input.events.splice(0..0, replay);
    ctx.request_repaint_of(viewport);
}

/// A position outside every window: a release there ends a press without a click.
const OUTSIDE: Pos2 = Pos2::new(-1.0, -1.0);

/// The regions the parts `parts` of the running pass declare; `None` for none. A drag area's covers are read from
/// the pass's widgets, so this runs at the end of the pass.
fn regions_of(ctx: &Context, viewport: ViewportId, parts: &[ExclusivePart]) -> Option<CaptionRegions> {
    if parts.is_empty() {
        return None;
    }
    let mut regions = CaptionRegions::default();
    for ExclusivePart { owner, part, hit, rect } in parts {
        match hit {
            WindowHit::Move => {
                regions.moves.push(*rect);
                regions.covers.push(covers_of(ctx, viewport, *owner, *part, *rect));
            }
            WindowHit::Resize(edge) => regions.edges.push((*edge, *rect)),
            WindowHit::Caption(button) => regions.buttons.push((*button, *rect)),
        }
    }
    Some(regions)
}

/// The parts of the drag area `band`, the part `part` of the component `owner`, where an in-app hit lies over it,
/// clipped to the band: each interactive egui widget of the running pass that egui's hit test puts above the
/// component's widgets, and each of the component's own shapes that a press there goes to instead of the band (a
/// member over it, a higher part). The component's widgets resolve as the component (8.3), so they are not hits
/// over the band themselves.
fn covers_of(ctx: &Context, viewport: ViewportId, owner: Id, part: PartRef, band: Rect) -> Vec<Rect> {
    let over = pointer::over_part(ctx, viewport, owner, part);
    widgets::above(ctx, owner, &over.widgets)
        .into_iter()
        .chain(over.above)
        .map(|cover| cover.intersect(band))
        .filter(|cover| cover.is_positive())
        .collect()
}

/// Keeps the regions of the running pass's exclusive parts for [`WindowHost::caption_regions`] and gives them to
/// the viewport's hit-test sink.
pub(super) fn publish_regions(ctx: &Context, viewport: ViewportId) {
    let parts = with_exclusive(ctx, viewport, |exclusive| exclusive.current.clone());
    let regions = regions_of(ctx, viewport, &parts);
    with_exclusive(ctx, viewport, |exclusive| exclusive.done = regions.clone());
    if let Some(sink) = PerViewport::<Record>::with(ctx, viewport, |record| record.sink.clone()) {
        let pixels_per_point = scale::pixels_per_point(ctx);
        sink.publish(regions.map(|regions| Published { regions, pixels_per_point }));
    }
}

/// Whether the platform answers the hit test of `viewport`'s window from the app's exclusive parts.
pub(crate) fn hit_sink_installed(ctx: &Context, viewport: ViewportId) -> bool {
    PerViewport::<Record>::with(ctx, viewport, |record| record.sink.is_some())
}

/// An exclusive part as its component declared it.
#[derive(Clone, Debug)]
struct ExclusivePart {
    owner: Id,
    part: PartRef,
    hit: WindowHit,
    rect: Rect,
}

/// A viewport's exclusive parts: those the running pass declared so far, and the regions of its last complete pass.
#[derive(Default)]
pub(super) struct Exclusive {
    current: Vec<ExclusivePart>,
    pub(super) done: Option<CaptionRegions>,
}

pub(super) fn with_exclusive<R>(ctx: &Context, viewport: ViewportId, f: impl FnOnce(&mut Exclusive) -> R) -> R {
    PerViewport::<Stamped<Exclusive>>::with_pass(
        ctx,
        viewport,
        pass::pass_nr(ctx, viewport),
        |exclusive, step| {
            exclusive.current.clear();
            if matches!(step, PassStep::Skipped) {
                exclusive.done = None;
            }
        },
        f,
    )
}

/// Records that `part` of the component `owner` is, over `rect`, what `hit` says for the running viewport's window.
/// A part declared again in the same pass replaces its entry.
pub(crate) fn note_exclusive(ctx: &Context, owner: Id, part: PartRef, hit: WindowHit, rect: Rect) {
    with_exclusive(ctx, pass::current_viewport(ctx), |exclusive| {
        let entry = ExclusivePart { owner, part, hit, rect };
        match exclusive.current.iter_mut().find(|held| held.owner == owner && held.part == part) {
            Some(held) => *held = entry,
            None => exclusive.current.push(entry),
        }
    });
}
