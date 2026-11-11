//! `show_viewport_immediate`, viewport-builder conversion, viewport facts and window commands.

use craft_host::target::WindowChrome;
use egui::{Context, Pos2, Rect, Ui, Vec2, ViewportBuilder, ViewportCommand, ViewportId, ViewportInfo, WindowLevel};

/// Whether the integration opens OS viewports: false where egui embeds them in the root (web, tests).
pub(crate) fn available(ctx: &Context) -> bool {
    !ctx.embed_viewports()
}

/// How a viewport opens.
pub(crate) struct Spec<'a> {
    /// The window title.
    pub title: &'a str,
    /// The inner size in points.
    pub size: Option<Vec2>,
    /// The outer position in desktop points.
    pub pos: Option<Pos2>,
    /// Who draws the frame.
    pub chrome: WindowChrome,
    /// Whether the OS draws caption buttons (the traffic lights on macOS) where it draws a title bar.
    pub os_buttons: bool,
    /// The smallest inner size.
    pub min_size: Option<Vec2>,
    /// The largest inner size.
    pub max_size: Option<Vec2>,
    /// The window level.
    pub level: WindowLevel,
    /// Whether the user can resize the window.
    pub resizable: bool,
    /// Whether the window has a taskbar or Dock entry.
    pub taskbar: bool,
    /// Whether the window opens hidden; [`show`] reveals it.
    pub hidden: bool,
}

/// Opens (or keeps) the OS viewport `id` and runs `body` once for each pass the viewport gets in this call.
pub(crate) fn show_immediate<T>(ctx: &Context, id: ViewportId, spec: &Spec<'_>, mut body: impl FnMut(&mut Ui) -> T) -> T {
    let mut builder =
        ViewportBuilder::default().with_title(spec.title).with_resizable(spec.resizable).with_taskbar(spec.taskbar).with_window_level(spec.level);
    if spec.hidden {
        builder = builder.with_visible(false);
    }
    builder = match spec.chrome {
        WindowChrome::Integrated => builder.with_decorations(false),
        WindowChrome::Inset => builder.with_decorations(true).with_fullsize_content_view(true).with_title_shown(false).with_titlebar_shown(false),
        _ => builder.with_decorations(true),
    };
    if !spec.os_buttons {
        builder = builder.with_titlebar_buttons_shown(false);
    }
    if let Some(size) = spec.size {
        builder = builder.with_inner_size(size);
    }
    if let Some(size) = spec.min_size {
        builder = builder.with_min_inner_size(size);
    }
    if let Some(size) = spec.max_size {
        builder = builder.with_max_inner_size(size);
    }
    if let Some(pos) = spec.pos {
        builder = builder.with_position(pos);
    }
    ctx.show_viewport_immediate(id, builder, |ui, _| body(ui))
}

/// Makes the hidden viewport `id` visible.
pub(crate) fn show(ctx: &Context, id: ViewportId) {
    ctx.send_viewport_cmd_to(id, egui::ViewportCommand::Visible(true));
}

/// The viewport id of the container `id`.
pub(crate) fn viewport_id(id: egui::Id) -> ViewportId {
    ViewportId::from_hash_of(id)
}

/// What a viewport reports about its window.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[doc(hidden)]
pub struct Facts {
    /// The window has OS focus; unknown counts as focused.
    pub focused: bool,
    /// The window reported that it has focus (unknown does not count).
    pub focus_reported: bool,
    pub minimized: bool,
    pub maximized: bool,
    pub fullscreen: bool,
    /// The inner rect in desktop points.
    pub inner: Option<Rect>,
    /// The outer rect in desktop points.
    pub outer: Option<Rect>,
    /// Physical pixels per point of the monitor, without interface zoom.
    pub native_scale: Option<f32>,
    /// The window is drawn: false while minimized or fully covered, when only the app's logic runs.
    pub visible: bool,
    /// The OS asked the window to close.
    pub close_requested: bool,
}

/// The facts one `ViewportInfo` holds.
pub(crate) fn facts_of(info: &ViewportInfo) -> Facts {
    Facts {
        focused: info.focused.unwrap_or(true),
        focus_reported: info.focused == Some(true),
        minimized: info.minimized.unwrap_or(false),
        maximized: info.maximized.unwrap_or(false),
        fullscreen: info.fullscreen.unwrap_or(false),
        inner: info.inner_rect,
        outer: info.outer_rect,
        native_scale: info.native_pixels_per_point,
        visible: info.visible() != Some(false),
        close_requested: info.close_requested(),
    }
}

/// The running viewport's facts.
#[doc(hidden)]
pub fn current(ctx: &Context) -> Facts {
    ctx.input(|input| facts_of(input.viewport()))
}

/// The facts of every viewport the last root input reported, from outside a pass.
pub(crate) fn reported(ctx: &Context) -> Vec<(ViewportId, Facts)> {
    ctx.input(|input| input.raw.viewports.iter().map(|(id, info)| (*id, facts_of(info))).collect())
}

/// The window's facts in the raw input of `viewport`, for the input hook: where `ctx.input` addresses the parent.
pub(crate) fn inner_size_in(input: &egui::RawInput, viewport: ViewportId) -> Option<Vec2> {
    input.viewports.get(&viewport).and_then(|info| info.inner_rect).map(|rect| rect.size())
}

/// Sends a window command to `viewport`.
pub(crate) fn command(ctx: &Context, viewport: ViewportId, command: ViewportCommand) {
    if !crate::runtime::measure::effect(ctx, "viewports::command") {
        return;
    }
    ctx.send_viewport_cmd_to(viewport, command);
}

/// Raises `viewport` and gives it OS focus.
pub(crate) fn raise(ctx: &Context, viewport: ViewportId) {
    command(ctx, viewport, ViewportCommand::Focus);
    ctx.request_repaint_of(viewport);
}

/// Cancels an OS close request of the running viewport so the owner can decide.
pub(crate) fn cancel_close(ctx: &Context) {
    ctx.send_viewport_cmd(ViewportCommand::CancelClose);
}

/// Whether a command to cancel the running viewport's close is already queued this pass.
pub(crate) fn close_cancelled(ctx: &Context) -> bool {
    ctx.viewport(|viewport| viewport.commands.contains(&ViewportCommand::CancelClose))
}

/// Asks the OS to close `viewport`.
pub(crate) fn close(ctx: &Context, viewport: ViewportId) {
    command(ctx, viewport, ViewportCommand::Close);
}

/// egui's resize direction for `edge`.
pub(crate) fn resize_direction(edge: crate::runtime::windows::ResizeEdge) -> egui::viewport::ResizeDirection {
    use crate::runtime::windows::ResizeEdge;
    use egui::viewport::ResizeDirection;
    match edge {
        ResizeEdge::Top => ResizeDirection::North,
        ResizeEdge::Bottom => ResizeDirection::South,
        ResizeEdge::Left => ResizeDirection::West,
        ResizeEdge::Right => ResizeDirection::East,
        ResizeEdge::TopLeft => ResizeDirection::NorthWest,
        ResizeEdge::TopRight => ResizeDirection::NorthEast,
        ResizeEdge::BottomLeft => ResizeDirection::SouthWest,
        ResizeEdge::BottomRight => ResizeDirection::SouthEast,
    }
}

/// Runs and paints the next pass of `viewport` even while its window is hidden, for a screenshot or an automation
/// client.
pub(crate) fn paint_while_hidden(ctx: &Context, viewport: ViewportId) {
    ctx.send_viewport_cmd_to(viewport, ViewportCommand::RequestPaintWhileHidden);
}

/// The pass number and movable flag the title bar of the running viewport last recorded.
#[doc(hidden)]
pub fn movable_record(ctx: &Context) -> Option<(u64, bool)> {
    super::memory::peek_clone::<(u64, bool)>(ctx, movable_id(ctx.viewport_id()))
}

/// The running viewport's pass number.
#[doc(hidden)]
pub fn pass_number(ctx: &Context) -> u64 {
    ctx.cumulative_pass_nr()
}

/// Records the movable flag of the running viewport's title bar for this pass.
#[doc(hidden)]
pub fn write_movable(ctx: &Context, movable: bool) {
    let (pass, id) = (ctx.cumulative_pass_nr(), movable_id(ctx.viewport_id()));
    super::memory::put(ctx, id, (pass, movable));
}

fn movable_id(viewport: ViewportId) -> egui::Id {
    super::memory::unique(("craft_ui.title_bar.movable", viewport))
}

/// Sets the `movable` flag of the key window when it has a full-size content view and visible window buttons;
/// `applied` holds the value last set. Does nothing off macOS.
#[doc(hidden)]
pub fn set_native_movable(movable: bool, applied: &mut Option<bool>) {
    craft_window::mac::set_movable(movable, applied);
}
