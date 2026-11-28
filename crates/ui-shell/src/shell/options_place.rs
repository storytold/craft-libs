//! Where the options bar sits (docked at the top or the bottom, or floating) and the grip that moves it.

use super::app_shell::ShellRegionOutput;
use super::canvas_shell::slots as keys;
use super::words::ShellWords;
use crate::contract::{
    Access, Bind, Common, Component, Container, ContainerProps, Cx, DragPreview, DropMark, DropOperation, DropTarget, MeasureCx, Mount, Name,
    Overlay, Pad, PaintUi, Pointer as _, Props as _, Role, Sizing, Styled as _, WordSetCx as _,
};
use crate::primitives::IconSpec;
use crate::runtime::env::{Label, ShellEntry};
use crate::style::{ComponentStyle, IconLook, Ink, StyleCx, SurfaceKind};
use craft_behavior::{CursorIcon, CursorPriority, IconId, Metrics, PressPhase, Sense};
use craft_theme::Stateful;
use egui::{Pos2, Rect, Response, Vec2, pos2, vec2};

/// Where the options bar sits: docked at the top or the bottom, or floating with its top-left corner `at`, relative to
/// the body. A non-finite `at` counts as unset. Floating keeps the edge; docking keeps `at`.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct OptionsBarPlace {
    /// Docked at the bottom instead of the top.
    #[serde(default)]
    pub bottom: bool,
    /// Floating over the body instead of docked.
    #[serde(default)]
    pub floating: bool,
    /// The floating bar's top-left corner relative to the body, once moved.
    #[serde(default)]
    pub at: Option<[f32; 2]>,
}

impl OptionsBarPlace {
    /// A place docked at the `bottom` or the top, or `floating` with the top-left corner `at`.
    pub const fn new(bottom: bool, floating: bool, at: Option<[f32; 2]>) -> Self {
        Self { bottom, floating, at }
    }

    /// The floating bar's top-left corner on screen: `at` inside `body`, or `fallback` in from the body's corner while
    /// the bar has not been moved.
    pub(super) fn corner(&self, body: Rect, fallback: f32) -> Pos2 {
        match self.at.filter(|at| at.iter().all(|coordinate| coordinate.is_finite())) {
            Some([x, y]) => body.min + vec2(x, y),
            None => body.min + Vec2::splat(fallback),
        }
    }
}

/// What a drag of a [`BarGrip`] carries: where the pointer holds the bar and how big the bar is.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct OptionsBarDrag {
    /// The pointer's offset inside the bar where the drag began.
    pub grab: Vec2,
    /// The bar's size.
    pub size: Vec2,
}

/// Where a drop of an options bar drag lands the bar: the place it writes and the rect the bar takes there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Landing {
    pub(super) place: OptionsBarPlace,
    rect: Rect,
}

/// The drop target over the body while an options bar is dragged: an edge band docks the bar there, a short room docks
/// it at the nearer half, and anywhere else it floats under the pointer.
pub(super) struct PlaceTarget {
    pub(super) body: Rect,
    /// The docked bar's height, which is also the height of an edge band.
    pub(super) bar: f32,
    pub(super) held: OptionsBarPlace,
}

impl DropTarget<OptionsBarDrag> for PlaceTarget {
    type Drop = Landing;

    fn accept(&self, drag: &OptionsBarDrag, at: Pos2, _: DropOperation) -> Option<Landing> {
        if !at.x.is_finite() || !at.y.is_finite() {
            return None;
        }
        let PlaceTarget { body, bar, held } = self;
        let docked = |bottom: bool| {
            let y = if bottom { body.bottom() - bar } else { body.top() };
            Landing { place: OptionsBarPlace::new(bottom, false, held.at), rect: Rect::from_min_size(pos2(body.left(), y), vec2(body.width(), *bar)) }
        };
        // Two bands and a bar between them do not fit a shorter room: the bar docks at the nearer half.
        if body.height() < 3.0 * bar {
            return Some(docked(at.y > body.center().y));
        }
        if at.y < body.top() + bar {
            return Some(docked(false));
        }
        if at.y > body.bottom() - bar {
            return Some(docked(true));
        }
        let corner = at - drag.grab;
        let min = pos2(
            corner.x.clamp(body.left(), (body.right() - drag.size.x).max(body.left())),
            corner.y.clamp(body.top(), (body.bottom() - drag.size.y).max(body.top())),
        );
        Some(Landing {
            place: OptionsBarPlace::new(held.bottom, true, Some([min.x - body.left(), min.y - body.top()])),
            rect: Rect::from_min_size(min, drag.size),
        })
    }

    fn mark(&self, drop: &Landing) -> DropMark {
        DropMark::region(drop.rect)
    }
}

/// The style of a bar grip.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct BarGripStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The side and the ink colour of the grip icon.
    #[style(icon)]
    icon: IconLook,
}

/// The grip of an options bar: a drag on it starts a drag session that moves the bar and docks or floats it on the
/// release.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct BarGrip {
    #[common]
    common: Common,
}

impl BarGrip {
    /// A grip.
    pub fn new() -> Self {
        BarGrip { common: Common::new() }
    }
}

impl Default for BarGrip {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for BarGrip {
    const ROLE: Role = Role::BUTTON;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const SHOWS_LABEL: bool = false;
    const INPUT: craft_behavior::Input = craft_behavior::Input::Pointer;
    const OVERLAYS: bool = false;
    type Style = BarGripStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: BarGripStyleKey) -> BarGripStyle {
        BarGripStyle { metrics: Metrics::new(), icon: IconLook::new(cx.control().icon, Stateful::new(cx.ink(Ink::default()))) }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.icon.side)
    }

    fn sense(&self) -> Sense {
        Sense::DRAG
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        if cx.resolved.label.is_empty() {
            access.name(Name::Text(cx.words::<ShellWords>().move_bar.as_str().to_owned()));
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if !matches!(cx.press().phase, PressPhase::Idle) {
            cx.cursor(CursorIcon::Grab, CursorPriority::Part);
        }
        if !matches!(cx.press().phase, PressPhase::DragStart { .. }) {
            return;
        }
        let Some(start) = cx.press().phase.start() else { return };
        // PLAN: a floating bar's rect is in no shell region and the grip cannot read its layer's rect, so its grab and size are the grip's own.
        let bar = cx
            .env
            .get::<ShellEntry>()
            .0
            .and_then(|shell| shell.regions().slots.iter().find(|(key, _)| *key == keys::OPTIONS).map(|(_, rect)| *rect))
            .unwrap_or(cx.rect);
        cx.start_drag(OptionsBarDrag { grab: start.origin - bar.min, size: bar.size() }, DragPreview::None);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let mut spec = IconSpec::svg(IconId::new("grip"));
        spec.color = Some(cx.resolve(&cx.style.icon.color));
        ui.draw().icon(cx.content_rect, &spec);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// The style of the floating options bar's layer: the surface comes from the container.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct FloatingBarStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The layer a floating options bar shows in: a modeless layer over the body, as high and wide as the bar's content.
#[derive(crate::contract::Component)]
#[component(container, unregistered)]
pub(super) struct FloatingBar {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    open: Bind<'static, bool>,
}

impl FloatingBar {
    /// The layer `overlay` places, named `label` when the slot has a name.
    pub(super) fn new(overlay: Overlay, label: Option<Label>) -> Self {
        let props = ContainerProps::new().mount(Mount::Layer(overlay)).surface(SurfaceKind::Floating).pad(Pad::None).sizing(Sizing::Content);
        let bar = FloatingBar { common: Common::new(), props, open: Bind::given(true) };
        match label {
            Some(label) => bar.label(label),
            None => bar,
        }
    }
}

impl Component for FloatingBar {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = FloatingBarStyle;
    type Output = ShellRegionOutput;

    fn style(_: &StyleCx<'_>, _: FloatingBarStyleKey) -> FloatingBarStyle {
        FloatingBarStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> ShellRegionOutput {
        ShellRegionOutput { response: cx.response.clone() }
    }
}

impl Container for FloatingBar {
    fn open(&mut self) -> Option<crate::contract::BindMut<'_, bool>> {
        Some(self.open.reborrow())
    }
}
