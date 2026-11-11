//! `DocumentWindow`: a document's window, a container in its own OS viewport.

#[cfg(feature = "testing")]
mod stories;

use crate::contract::overlay::OverlayFit;
use crate::contract::{
    Bind, BindMut, BodyCx, Common, Component, Container, ContainerProps, Cx, DragBand, HeaderRegion, Mount, NoValue, Outcome, Overlay, Pad,
    Props as _, Role, SaveKey, Show as _, Sizing, WordSetCx as _,
};
use crate::runtime::env::Label;
use crate::runtime::memory::Remember as _;
use crate::runtime::overlay::CloseReason;
use crate::runtime::windows::{WindowAction, WindowHost, WindowSpec};
use crate::sections::{HeaderAction, HeaderStrength, TitleRow};
use crate::style::{ComponentStyle, StyleCx, SurfaceKind};
use craft_behavior::placement::{cascade, monitor_of};
use craft_behavior::{Metrics, Monitor, Sense};
use craft_window::{WindowId, WindowList};
use egui::{Rect, Response, Ui, Vec2};

/// The style of a document window: the surface comes from the container.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct DocumentWindowStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The offset between cascaded windows (`size.window.cascade`).
    cascade: f32,
}

/// What a document window reports: its response, how it was asked to close in this pass, and whether it has OS
/// focus.
#[derive(Debug)]
#[non_exhaustive]
pub struct DocumentWindowOutput {
    /// The window's response.
    pub response: Response,
    /// How the window was asked to close in this pass: its OS close button or the fallback layer's close action.
    /// Reported once for each request. The window stays until the app stops showing it.
    pub close_requested: Option<CloseReason>,
    /// The window has OS focus. False in the fallback layer, which has no window of its own.
    pub focused: bool,
    /// The step of a cascade of windows (`size.window.cascade`): the `step` of [`DocumentWindow::cascaded`].
    pub cascade: f32,
    /// The window gained OS focus in this pass.
    pub focus_gained: bool,
}

impl Outcome for DocumentWindowOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A document's window: the container of one entry of the app's window list, in its own OS window.
///
/// The app shows one for each open document and stops showing it to close it. The body holds the document's whole
/// window, including its title bar and shell; the window adds none of its own where the OS window exists.
///
/// - **Placement.** The window opens where [`first_placed`](DocumentWindow::first_placed) puts it (see
///   [`cascaded`](DocumentWindow::cascaded)) until a placement saved under [`remember`](DocumentWindow::remember)
///   takes over. The minimum and maximum size (through [`overlay`](DocumentWindow::overlay)) and the resizability are the
///   fallback layer's, which the OS window reads.
/// - **Close.** The OS close button, and the close action of the fallback layer, report `close_requested`. The
///   window stays while the app shows it, for example while it asks about unsaved changes, and closes when the app
///   stops showing it.
/// - **Title and focus.** The title is the label given to `new`; it follows the document while the app changes it.
///   [`raise`](DocumentWindow::raise) brings the window forward in the pass it is set. With
///   [`in_list`](DocumentWindow::in_list) the window makes itself the list's active window when it gains OS focus.
/// - **Fallback.** Where OS windows are unavailable the document shows on a movable, resizable layer with a title row.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container)]
pub struct DocumentWindow<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    /// The open state the pipeline reads: always open, so a close request never ends the window.
    open: Bind<'a, bool>,
    /// Makes the window the active one of the app's window list.
    activate: Option<Box<dyn FnMut() + 'a>>,
    /// Raises the window and gives it OS focus in this pass.
    #[prop(flag)]
    raise: bool,
    closed: Option<CloseReason>,
}

impl<'a> DocumentWindow<'a> {
    /// A window titled `title`, resizable, with the installed decision's frame.
    pub fn new(title: impl Into<Label>) -> Self {
        let fallback = Overlay::floating().resizable(true).fit(OverlayFit::Resizable);
        let props = ContainerProps::new()
            .surface(SurfaceKind::Background)
            .pad(Pad::None)
            .sizing(Sizing::Fill)
            .header(HeaderRegion::new().band(DragBand::Move))
            .mount(Mount::viewport_with(fallback, WindowSpec::document()));
        DocumentWindow { common: Common::new(), props, open: Bind::given(true), activate: None, raise: false, closed: None }.label(title)
    }

    /// Changes the layer the window falls back to, for the settings without a setter of their own: the smallest and
    /// largest size (`Overlay::size_limits`), which the OS window takes as its limits.
    #[must_use]
    pub fn overlay(mut self, change: impl FnOnce(Overlay) -> Overlay) -> Self {
        self.props = match std::mem::take(&mut self.props.mount) {
            Mount::Viewport { fallback, spec } => self.props.mount(Mount::Viewport { fallback: change(fallback), spec }),
            other => self.props.mount(other),
        };
        self
    }

    /// Where the window opens, as its outer rect in desktop points. A saved placement comes first.
    #[must_use]
    pub fn first_placed(self, rect: Rect) -> Self {
        self.overlay(|overlay| overlay.placed(rect))
    }

    /// Whether the user can resize the window.
    #[must_use]
    pub fn resizable(self, on: bool) -> Self {
        self.overlay(|overlay| overlay.resizable(on))
    }

    /// Makes the window the active one of `windows` when it gains OS focus: the entry `id` the app opened for this
    /// document. The Window menu's check mark follows.
    #[must_use]
    pub fn in_list<T>(mut self, windows: &'a mut WindowList<T>, id: WindowId) -> Self {
        self.activate = Some(Box::new(move || {
            windows.focus(id);
        }));
        self
    }

    /// Saves the window's placement and size under `key`, so a reopened document sits where it was left. Build the key
    /// of each document with [`SaveKey::named`].
    #[must_use]
    pub fn remember(mut self, key: SaveKey) -> Self {
        self.props = self.props.save(key);
        self
    }

    /// Where a window of `size` opens when it follows the window at `previous`: one cascade `step` down and across
    /// on the monitor showing most of `previous`; an axis on which that would leave the monitor starts again at the
    /// monitor's edge. `previous`, `size` and `step` are points of a window whose scale is `scale`. Without
    /// `monitors` the step is not bounded.
    pub fn cascade_in(previous: Rect, size: Vec2, monitors: &[Monitor], scale: f32, step: f32) -> Rect {
        let previous_px = Rect::from_min_max(previous.min * scale, previous.max * scale);
        match monitor_of(monitors, previous_px) {
            Some(monitor) => cascade(previous, size, monitor.points_at(scale), step),
            None => Rect::from_min_size(previous.min + Vec2::splat(step), size),
        }
    }

    /// [`cascade_in`](DocumentWindow::cascade_in) over the monitors the window host lists. `step` is the
    /// [`cascade`](DocumentWindowOutput::cascade) a shown window reports.
    pub fn cascaded(ui: &Ui, previous: Rect, size: Vec2, step: f32) -> Rect {
        let host = WindowHost::of(ui.ctx());
        let scale = host.viewport_at(previous.center()).and_then(|viewport| host.facts(viewport)).and_then(|facts| facts.scale);
        let monitors = if scale.is_some() { host.monitors() } else { Vec::new() };
        Self::cascade_in(previous, size, &monitors, scale.unwrap_or(1.0), step)
    }
}

/// Whether the window had OS focus in the pass before.
#[derive(Clone, Copy, Default)]
struct HadFocus(bool);

impl Component for DocumentWindow<'_> {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = DocumentWindowStyle;
    type Output = DocumentWindowOutput;

    fn style(cx: &StyleCx<'_>, _: DocumentWindowStyleKey) -> DocumentWindowStyle {
        DocumentWindowStyle { cascade: cx.tokens.size.window.cascade, ..DocumentWindowStyle::default() }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(mut self, mut cx: Cx<'_, Self>) -> DocumentWindowOutput {
        let host = WindowHost::of(&cx.ctx);
        let viewport = host.viewport_of(cx.id);
        if self.raise
            && let Some(viewport) = viewport
        {
            host.act(viewport, WindowAction::Focus);
        }
        let focused = viewport.and_then(|viewport| host.facts(viewport)).is_some_and(|facts| facts.focused);
        let had_focus = std::mem::replace(&mut cx.keep::<HadFocus>().0, focused);
        let focus_gained = focused && !had_focus;
        if focus_gained && let Some(activate) = self.activate.as_mut() {
            activate();
        }
        DocumentWindowOutput { response: cx.response.clone(), close_requested: self.closed, focused, cascade: cx.style.cascade, focus_gained }
    }
}

impl Container for DocumentWindow<'_> {
    fn open(&mut self) -> Option<BindMut<'_, bool>> {
        Some(self.open.reborrow())
    }

    /// The title row of the fallback layer; an OS window has the title bar the app composes in its body.
    fn header(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        if WindowHost::of(cx.memory.ctx()).viewports_available() {
            return;
        }
        let shown = TitleRow::new(cx.label()).strength(HeaderStrength::Strong).actions([HeaderAction::Close]).show(ui);
        if shown.action == Some(HeaderAction::Close) {
            cx.request_close();
        }
    }

    fn closed(&mut self, why: CloseReason) {
        self.closed.get_or_insert(why);
    }
}
