//! `FloatingGroup`: the window a floating dock group shows in.
//!
//! The group is a movable, resizable container on a layer (or in an OS window) whose rect the caller owns: the
//! caller places it with the rect the dock model keeps and takes the rect the output reports after each step of
//! a gesture. The title row carries the active panel's title and the Menu, Float and Close actions; the Menu action
//! opens the `menu` rows. The body step supplies the move band, the edge grips and the keyboard Move and Size modes.

use super::drop::{DragUnit, start_group_drag};
use super::{DockDropTarget, DockDropped, DockWords, GroupDrag, PanelDrag, PanelKey, native};
use crate::contract::BodyCx;
use crate::contract::SizeLimits;
use crate::contract::overlay::{PlacementChange, PlacementKind, Popup};
use crate::contract::{
    Common, Component, Container, ContainerProps, Cx, DragBand, HeaderRegion, Inner, Mount, NoValue, Outcome, Overlay, Pad, Pointer as _, Props as _,
    Role, Show as _, ShowContainer as _, Sizing, WordSetCx as _,
};
use crate::runtime::env::{Env, Label};
use crate::runtime::windows::WindowSpec;
use crate::sections::{HeaderAction, HeaderStrength, MenuRows, TitleRow};
use crate::style::{ComponentStyle, StyleCx, SurfaceKind};
use craft_behavior::{GesturePhase, Metrics, Sense};
use egui::{Id, Rect, Response, Ui, Vec2};
use std::cell::RefCell;
use std::hash::Hash;

#[cfg(feature = "testing")]
mod stories;

/// The actions a floating group's title row shows unless `actions` says otherwise.
const ACTIONS: [HeaderAction; 3] = [HeaderAction::Menu, HeaderAction::Float, HeaderAction::Close];

/// The style of a floating group: the surface and padding come from the container.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct FloatingGroupStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What a floating group reports in a pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct FloatingGroupOutput {
    /// The group's response.
    pub response: Response,
    /// The last step of a move or resize this pass, with the rect the caller keeps: while the group is minimized
    /// the rect has the height the group has when restored.
    pub placed: Option<PlacementChange>,
    /// The title row's action part taken this pass. Float and Close are the host's: the dock model docks or closes
    /// the group. Menu opens the `menu` rows.
    pub action: Option<HeaderAction>,
    /// Whether a double click on the title band asks to minimize the group, or restore it when it is minimized.
    pub minimized_toggled: bool,
}

impl Outcome for FloatingGroupOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A floating dock group: a titled window the user moves by its title, resizes by its edges and corners, and
/// minimizes by a double click on the title.
///
/// - Placement: the group shows at the rect given to `new`; the user's moves and resizes come back in
///   `FloatingGroupOutput::placed` and the caller shows the next pass at that rect. A move or resize stays inside
///   `bounds` and the panel `limits` (the limits are the body's; the title row adds its height).
/// - Sizing: with `Sizing::Content` (the default) the rect fixes the position and the width, and the height follows
///   the body; with another sizing the rect is the size. The user's resizes are reported either way.
/// - Minimized: only the title row shows, at the width of the rect, and the edges do not resize. The rect given to
///   `new` keeps the height the group has when restored.
/// - Docking: with [`window_of`](FloatingGroup::window_of) set, a move by the title starts a window-unit drag
///   (`GroupDrag`) for the dock's `DockDropTarget`, which shows the drop mark and docks the window on release.
/// - Window: with `native` set the group opens in an OS window of that kind, with the layer as the fallback where
///   the platform has no windows. The window comes to the front when the user begins to move or resize it.
/// - Menu: the Menu action and a context request on the title row open the `menu` rows, under the action's part.
/// - Show it with [`show_group`](FloatingGroup::show_group): the closure fills the body, and is not called while
///   the group is minimized.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container)]
pub struct FloatingGroup<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    rect: Rect,
    /// The rect a move or resize keeps the group inside.
    #[prop]
    bounds: Option<Rect>,
    /// The body's size limits.
    #[prop]
    limits: SizeLimits,
    /// The group's opacity, from 0 (invisible) to 1.
    #[prop(quantity = "opacity")]
    opacity: f32,
    /// Whether only the title row shows.
    #[prop]
    minimized: bool,
    /// Whether the height follows the body (`Sizing::Content`) or the rect gives it.
    #[prop]
    sized: Sizing,
    /// Opens the group in an OS window of this kind.
    #[prop]
    native: Option<WindowSpec>,
    /// The rows of the menu the Menu action opens.
    #[prop(func = "FnMut(&mut Ui, &mut BodyCx<'_>)")]
    menu: Option<MenuRows<'a>>,
    actions: Vec<HeaderAction>,
    taken: Option<HeaderAction>,
    menu_opener: Option<Id>,
    toggled: bool,
    placed: Option<PlacementChange>,
    /// Starts the window-unit drag; set by `window_of`.
    start_drag: Option<StartDrag<'a>>,
    /// Takes the drops released on the group, on the group's own layer.
    take_drop: Option<TakeDrop<'a>>,
    /// A move by the title began this pass.
    moving: bool,
}

type StartDrag<'a> = Box<dyn for<'b, 'c> FnOnce(&'b mut Cx<'c, FloatingGroup<'a>>) + 'a>;

type TakeDrop<'a> = Box<dyn for<'b, 'c> FnMut(&'b mut Cx<'c, FloatingGroup<'a>>) + 'a>;

impl<'a> FloatingGroup<'a> {
    /// A floating group titled `title` at `rect`, with the Menu, Float and Close actions.
    pub fn new(title: impl Into<Label>, rect: Rect) -> Self {
        let props = ContainerProps::new()
            .surface(SurfaceKind::Floating)
            .pad(Pad::None)
            .sizing(Sizing::Content)
            .header(HeaderRegion::new().band(DragBand::Move).rule(true));
        FloatingGroup {
            common: Common::new(),
            props,
            rect,
            bounds: None,
            limits: SizeLimits::default(),
            opacity: 1.0,
            minimized: false,
            sized: Sizing::Content,
            native: None,
            menu: None,
            actions: ACTIONS.to_vec(),
            taken: None,
            menu_opener: None,
            toggled: false,
            placed: None,
            start_drag: None,
            take_drop: None,
            moving: false,
        }
        .label(title)
    }

    /// The group is the floating window of the dock model that holds `anchor`: a move by its title starts a
    /// window-unit drag, so the dock marks where the window would dock while it moves and docks it on release.
    #[must_use]
    pub fn window_of<P: Clone + Hash + Send + Sync + 'static>(mut self, anchor: P) -> Self {
        let active = PanelKey::of(&anchor);
        self.start_drag = Some(Box::new(move |cx| start_group_drag(cx, DragUnit::Window(anchor), active, None)));
        self
    }

    /// The group takes the drops released over it through `target`, the dock's target of the spots its groups
    /// register, on the group's own layer: a drop over a floating window lands on it, not on what lies under it.
    /// The drop taken in a pass lands in `dropped`.
    #[must_use]
    #[doc(hidden)]
    pub fn takes_drops<P>(mut self, target: DockDropTarget<'a, P>, dropped: &'a RefCell<Option<DockDropped<P>>>) -> Self
    where
        P: Clone + Eq + Hash + Send + Sync + 'static,
    {
        self.take_drop = Some(Box::new(move |cx| {
            let taken = cx.drop_target::<GroupDrag<P>, _>(&target).or_else(|| cx.drop_target::<PanelDrag, _>(&target));
            if taken.is_some() {
                *dropped.borrow_mut() = taken;
            }
        }));
        self
    }

    /// The action parts at the end of the title row, in reading order.
    #[must_use]
    pub fn actions(mut self, actions: impl IntoIterator<Item = HeaderAction>) -> Self {
        self.actions = actions.into_iter().collect();
        self
    }

    /// Shows the group; `body` fills the area under the title row unless the group is minimized.
    pub fn show_group<R>(mut self, ui: &mut Ui, mut body: impl FnMut(&mut Ui, &mut BodyCx<'_>) -> R) -> Inner<FloatingGroupOutput, R> {
        let header = Env::of(ui).tokens.size.panel_header;
        let mount = self.mount(header);
        self.props = self.props.mount(mount);
        let minimized = self.minimized;
        let shown = self.show(ui, |ui, cx| (!minimized).then(|| body(ui, cx)));
        Inner { output: shown.output, inner: shown.inner.flatten(), settle: shown.settle }
    }

    /// The layer or window the group shows in. `header` is the height of the title row.
    ///
    /// A minimized group, and one with `Sizing::Content`, is placed by its position and width; its height is its
    /// content's, which is the title row alone while minimized.
    fn mount(&self, header: f32) -> Mount {
        // The frame's limits are the body's with the title row on top.
        let frame = self.limits.around(Vec2::new(0.0, header));
        let mut overlay = Overlay::floating().movable(true).resizable(!self.minimized).opacity(self.opacity).body_scroll(false);
        overlay = if self.minimized {
            let width = self.rect.width();
            overlay.placed_at(self.rect.min).size_limits(SizeLimits::new(Vec2::new(width, 0.0), Vec2::new(width, f32::INFINITY)))
        } else if self.sized == Sizing::Content {
            let width = self.rect.width().max(frame.min.x).min(frame.max.map_or(f32::INFINITY, |max| max.x));
            overlay.placed_at(self.rect.min).size_limits(SizeLimits { min: Vec2::new(width, frame.min.y), max: frame.max })
        } else {
            overlay.placed(self.rect).size_limits(frame)
        };
        if let Some(bounds) = self.bounds {
            overlay = overlay.bounds(bounds);
        }
        match &self.native {
            Some(spec) => Mount::viewport_with(overlay, spec.clone()),
            None => Mount::Layer(overlay),
        }
    }
}

impl Component for FloatingGroup<'_> {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = FloatingGroupStyle;
    type Output = FloatingGroupOutput;

    fn style(_: &StyleCx<'_>, _: FloatingGroupStyleKey) -> FloatingGroupStyle {
        FloatingGroupStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if let Some(mut take) = self.take_drop.take() {
            take(cx);
            self.take_drop = Some(take);
        }
    }

    fn finish(mut self, mut cx: Cx<'_, Self>) -> FloatingGroupOutput {
        if self.start_drag.is_some() {
            native::keeps_app_drag(&cx.core.ctx, cx.core.id);
        }
        if self.moving
            && let Some(start) = self.start_drag.take()
        {
            start(&mut cx);
        }
        if let Some(opener) = self.menu_opener {
            Popup::toggle(opener, &mut cx);
        }
        let output = FloatingGroupOutput { response: cx.response.clone(), placed: self.placed, action: self.taken, minimized_toggled: self.toggled };
        native::raise(&cx.core.ctx, &output);
        native::fade(&cx.core.ctx, &output);
        output
    }
}

impl Container for FloatingGroup<'_> {
    fn header(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        let shown = TitleRow::new(cx.label()).strength(HeaderStrength::Strong).actions(self.actions.iter().copied()).show(ui);
        self.taken = shown.action.or(self.taken);
        if let Some(rows) = self.menu.as_mut() {
            self.menu_opener = rows.hang(ui, &shown, cx.words::<DockWords>().panel_menu.clone());
        }
        self.toggled |= cx.band_double_pressed();
    }

    fn placement_changed(&mut self, change: &PlacementChange) {
        let mut change = *change;
        if self.minimized {
            change.rect = Rect::from_min_size(change.rect.min, Vec2::new(change.rect.width(), self.rect.height()));
        }
        self.moving |= change.kind == PlacementKind::Move && change.phase == GesturePhase::Begin;
        self.placed = Some(change);
    }
}
