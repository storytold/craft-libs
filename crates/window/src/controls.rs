//! Window buttons and resize edges for a window whose frame the app draws ([`Frame::App`]).
//!
//! GNOME on Wayland draws no window frames (its compositor offers no `zxdg_decoration_manager_v1`),
//! so an app that turns winit's imitation frame off has to give the window back what the frame did:
//!
//! - **Window buttons** ([`buttons`]): minimize, maximize/restore and close in Adwaita's style:
//!   round, with symbolic glyphs drawn in code (no icon files). The app places them at the ends of
//!   its own header bar in the desktop's order ([`Layout`], GNOME's `button-layout`), reserving
//!   [`width`] for each end.
//! - **Resize edges** ([`resize_edges`]): zones along the window's edges, with L-shaped corners,
//!   above the rest of the UI, and a hairline outline while the window is neither maximized nor
//!   full screen.
//! - **The button after a move or resize** ([`DesktopGrab`]): `StartDrag` and `BeginResize` hand
//!   the pointer to the desktop, which keeps the button's release (on Wayland the compositor grabs
//!   the pointer; emilk/egui#7959). egui then still thinks the button is held, so the next press on
//!   an edge or the title bar does nothing until a click inside the window, and the press has taken
//!   the keyboard from the page. [`desktop_grab`] notes the hand-over; the [`DesktopGrab`] plugin
//!   gives the release back on the window's next input and the keyboard back once the desktop lets
//!   go, in every window: eframe runs `App::raw_input_hook` for the root window only, but egui runs
//!   plugins for every viewport.
//!
//! The look comes from the app's [`egui::Visuals`] ([`Look::from_visuals`]) unless the app passes
//! its own colours, and the tooltips and screen-reader labels from [`Labels`], which the app
//! translates. Desktops that draw the frame ([`Frame::Desktop`]) never need any of this.
//!
//! ```ignore
//! // Once, at startup:
//! craft_window::controls::install(&cc.egui_ctx);
//!
//! // In each window's header bar, where the app draws the frame:
//! if let Some(layout) = frame.layout() {
//!     let (look, labels) = (Look::from_visuals(ui.visuals()), app.window_button_labels());
//!     ui.horizontal(|ui| controls::buttons(ui, &layout.left, &look, &labels));
//!     // … the header bar's own content, then `layout.right` at the other end …
//!     controls::resize_edges(ui.ctx(), &look);
//! }
//! ```
//!
//! [`Frame::App`]: crate::Frame::App
//! [`Frame::Desktop`]: crate::Frame::Desktop

use egui::viewport::ResizeDirection;
use egui::{Align2, Color32, CursorIcon, Pos2, Rect, Sense, Stroke, Vec2};

pub use crate::{Button, Layout};

/// The round button's diameter, as in Adwaita.
pub const BUTTON: f32 = 24.0;
/// Between two window buttons.
pub const GAP: f32 = 6.0;
/// Between the window buttons and the rest of the header bar.
pub const SIDE_GAP: f32 = 10.0;

/// How far into the window an edge resizes it.
const EDGE: f32 = 5.0;
/// How far along an edge a corner reaches, so corners are easy to hit.
const CORNER: f32 = 16.0;

/// The colours the window buttons and the window outline are drawn in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// The glyphs, and (washed out) the round button behind them.
    pub glyph: Color32,
    /// The ring around a button that has the keyboard.
    pub focus: Color32,
    /// The hairline around a window that isn't maximized. Without the desktop's frame or shadow it
    /// keeps the window apart from what's behind it.
    pub outline: Color32,
}

impl Look {
    /// The colours `visuals` gives text and the keyboard focus, and a hairline darker on a dark
    /// theme, where a faint one would vanish.
    pub fn from_visuals(visuals: &egui::Visuals) -> Self {
        Self {
            glyph: visuals.text_color(),
            focus: visuals.selection.stroke.color,
            outline: Color32::from_black_alpha(if visuals.dark_mode { 160 } else { 60 }),
        }
    }
}

/// The window buttons' tooltips and screen-reader labels, in the app's language. The default is
/// English; the app passes its own translations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Labels {
    pub minimize: String,
    pub maximize: String,
    /// Maximize's label while the window is maximized.
    pub restore: String,
    pub close: String,
}

impl Default for Labels {
    fn default() -> Self {
        Self { minimize: "Minimize".into(), maximize: "Maximize".into(), restore: "Restore".into(), close: "Close".into() }
    }
}

/// The width `buttons` take in the header bar, including [`SIDE_GAP`] to the rest of it; nothing
/// for an end with no buttons.
pub fn width(buttons: &[Button]) -> f32 {
    match buttons.len() {
        0 => 0.0,
        n => n as f32 * BUTTON + (n - 1) as f32 * GAP + SIDE_GAP,
    }
}

/// Draw the window buttons for one end of the header bar, in reading order, inside a left-to-right
/// layout; the caller adds [`SIDE_GAP`] where the header bar continues. Each button sends its
/// command to the window it is drawn in. Close sends [`egui::ViewportCommand::Close`], the same
/// request the desktop's close button makes, so the app's unsaved-changes check still runs.
/// Nothing is drawn in full screen.
pub fn buttons(ui: &mut egui::Ui, buttons: &[Button], look: &Look, labels: &Labels) {
    let (maximized, fullscreen) = ui.input(|i| (i.viewport().maximized.unwrap_or(false), i.viewport().fullscreen.unwrap_or(false)));
    if fullscreen {
        return;
    }
    // The buttons are [`GAP`] apart, whatever the caller's item spacing; restored afterwards.
    let spacing = ui.spacing().item_spacing.x;
    ui.spacing_mut().item_spacing.x = 0.0;
    for (n, &button) in buttons.iter().enumerate() {
        if n > 0 {
            ui.add_space(GAP);
        }
        let label = match button {
            Button::Minimize => &labels.minimize,
            Button::Maximize if maximized => &labels.restore,
            Button::Maximize => &labels.maximize,
            Button::Close => &labels.close,
        };
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(BUTTON), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
        let response = response.on_hover_text(label.as_str());
        if ui.is_rect_visible(rect) {
            paint(ui.painter(), rect, button, maximized, look, &response);
        }
        if response.clicked() {
            ui.ctx().send_viewport_cmd(command(button, maximized));
        }
    }
    ui.spacing_mut().item_spacing.x = spacing;
}

/// What a click on `button` asks of the window.
fn command(button: Button, maximized: bool) -> egui::ViewportCommand {
    match button {
        Button::Minimize => egui::ViewportCommand::Minimized(true),
        Button::Maximize => egui::ViewportCommand::Maximized(!maximized),
        Button::Close => egui::ViewportCommand::Close,
    }
}

/// Adwaita's look: a round wash of the glyph colour, stronger on hover and press, and a 16-point
/// symbolic glyph drawn with lines.
fn paint(painter: &egui::Painter, rect: Rect, button: Button, maximized: bool, look: &Look, response: &egui::Response) {
    let alpha = if response.is_pointer_button_down_on() {
        0.28
    } else if response.hovered() {
        0.16
    } else {
        0.10
    };
    painter.circle_filled(rect.center(), BUTTON / 2.0, look.glyph.gamma_multiply(alpha));
    if response.has_focus() {
        painter.circle_stroke(rect.center(), BUTTON / 2.0 - 0.5, Stroke::new(1.0, look.focus));
    }
    let stroke = Stroke::new(1.25, look.glyph);
    // The glyph's 16-point square, its corners on pixel centres so lines stay crisp.
    let c = rect.center();
    let at = |x: f32, y: f32| Pos2::new((c.x - 8.0 + x).round() + 0.5, (c.y - 8.0 + y).round() + 0.5);
    match button {
        Button::Minimize => {
            painter.line_segment([at(4.0, 11.0), at(11.0, 11.0)], stroke);
        }
        Button::Maximize if maximized => {
            // Restore: a smaller window in front of another.
            painter.rect_stroke(Rect::from_two_pos(at(4.0, 6.0), at(10.0, 12.0)), 1.0, stroke, egui::StrokeKind::Middle);
            painter.line_segment([at(6.0, 6.0), at(6.0, 4.0)], stroke);
            painter.line_segment([at(6.0, 4.0), at(12.0, 4.0)], stroke);
            painter.line_segment([at(12.0, 4.0), at(12.0, 10.0)], stroke);
            painter.line_segment([at(12.0, 10.0), at(10.0, 10.0)], stroke);
        }
        Button::Maximize => {
            painter.rect_stroke(Rect::from_two_pos(at(4.0, 4.0), at(11.0, 11.0)), 1.0, stroke, egui::StrokeKind::Middle);
        }
        Button::Close => {
            painter.line_segment([at(4.0, 4.0), at(11.0, 11.0)], stroke);
            painter.line_segment([at(11.0, 4.0), at(4.0, 11.0)], stroke);
        }
    }
}

/// The resize zones of a window filling `rect`: edges `edge` deep, and each corner an L of two
/// strips reaching `corner` along both edges (a square would reach into the content).
fn zones(rect: Rect, edge: f32, corner: f32) -> Vec<(ResizeDirection, Rect)> {
    let (l, r, t, b) = (rect.left(), rect.right(), rect.top(), rect.bottom());
    // Never inverted, however small the window.
    let zone = |x0: f32, x1: f32, y0: f32, y1: f32| Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1.max(x0), y1.max(y0)));
    vec![
        (ResizeDirection::NorthWest, zone(l, l + corner, t, t + edge)),
        (ResizeDirection::NorthWest, zone(l, l + edge, t + edge, t + corner)),
        (ResizeDirection::NorthEast, zone(r - corner, r, t, t + edge)),
        (ResizeDirection::NorthEast, zone(r - edge, r, t + edge, t + corner)),
        (ResizeDirection::SouthWest, zone(l, l + corner, b - edge, b)),
        (ResizeDirection::SouthWest, zone(l, l + edge, b - corner, b - edge)),
        (ResizeDirection::SouthEast, zone(r - corner, r, b - edge, b)),
        (ResizeDirection::SouthEast, zone(r - edge, r, b - corner, b - edge)),
        (ResizeDirection::North, zone(l + corner, r - corner, t, t + edge)),
        (ResizeDirection::South, zone(l + corner, r - corner, b - edge, b)),
        (ResizeDirection::West, zone(l, l + edge, t + corner, b - corner)),
        (ResizeDirection::East, zone(r - edge, r, t + corner, b - corner)),
    ]
}

fn cursor(direction: ResizeDirection) -> CursorIcon {
    match direction {
        ResizeDirection::North => CursorIcon::ResizeNorth,
        ResizeDirection::South => CursorIcon::ResizeSouth,
        ResizeDirection::East => CursorIcon::ResizeEast,
        ResizeDirection::West => CursorIcon::ResizeWest,
        ResizeDirection::NorthEast => CursorIcon::ResizeNorthEast,
        ResizeDirection::SouthEast => CursorIcon::ResizeSouthEast,
        ResizeDirection::NorthWest => CursorIcon::ResizeNorthWest,
        ResizeDirection::SouthWest => CursorIcon::ResizeSouthWest,
    }
}

/// Resize zones along the edges of the window being drawn, and its hairline outline, while the
/// window is neither maximized nor full screen. The zones sit above everything else (in the
/// foreground order), so a press there resizes instead of reaching a scrollbar or panel under it.
/// Call once per pass in each window whose frame the app draws.
pub fn resize_edges(ctx: &egui::Context, look: &Look) {
    let (maximized, fullscreen) = ctx.input(|i| (i.viewport().maximized.unwrap_or(false), i.viewport().fullscreen.unwrap_or(false)));
    if maximized || fullscreen {
        return;
    }
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::unique("craft_window_resize_edges"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .pivot(Align2::LEFT_TOP)
        .interactable(true)
        .show(ctx, |ui| {
            ui.set_min_size(Vec2::ZERO);
            for (n, (direction, rect)) in zones(screen, EDGE, CORNER).into_iter().enumerate() {
                if rect.width() <= 0.0 || rect.height() <= 0.0 {
                    continue;
                }
                let response = ui.interact(rect, ui.scope_id().with(("edge", n)), Sense::drag()).on_hover_cursor(cursor(direction));
                // Sensing only drags, a drag starts on the press itself.
                if response.drag_started() {
                    desktop_grab(ui.ctx(), egui::ViewportCommand::BeginResize(direction));
                }
            }
            ui.painter().rect_stroke(screen, 0.0, Stroke::new(1.0, look.outline), egui::StrokeKind::Inside);
        });
}

/// Where [`desktop_grab`] notes that the desktop holds the pointer of `viewport`'s window. Per
/// window, so the release goes back to the window that was moved or resized.
fn grab_id(viewport: egui::ViewportId) -> egui::Id {
    egui::Id::unique("craft_window_desktop_grab").with(viewport)
}

/// Where [`desktop_grab`] notes the widget that had the keyboard in `viewport`'s window.
fn grab_focus_id(viewport: egui::ViewportId) -> egui::Id {
    egui::Id::unique("craft_window_desktop_grab_focus").with(viewport)
}

/// Hand the pointer to the desktop to move (`StartDrag`) or resize (`BeginResize`) the window
/// being drawn. Use this instead of sending the command directly, so that with [`DesktopGrab`]
/// installed the button's release and the keyboard come back afterwards.
pub fn desktop_grab(ctx: &egui::Context, command: egui::ViewportCommand) {
    ctx.send_viewport_cmd(command);
    let viewport = ctx.viewport_id();
    // Read before `data_mut`: egui's context lock isn't reentrant.
    let focused = ctx.memory(|m| m.focused());
    let (id, focus_id) = (grab_id(viewport), grab_focus_id(viewport));
    ctx.data_mut(|d| {
        d.insert_temp(id, ());
        if let Some(widget) = focused {
            d.insert_temp(focus_id, widget);
        } else {
            d.remove::<egui::Id>(focus_id);
        }
    });
    ctx.request_repaint();
}

/// After [`desktop_grab`], add the primary button's release to the window's next input, unless it
/// brings one. [`DesktopGrab`] calls this for every window; an app that doesn't install it calls
/// this from its raw input hook (which eframe runs for the root window only).
///
/// The release goes before the input's first press (the next resize or move, or a click), which
/// egui would otherwise end straight away; a release after that press is the press's own. It also
/// goes before the pointer leaving, as a button event would bring the gone pointer back; a release
/// the input brings after the pointer left is the grab's own, and moves there instead.
pub fn release_after_desktop_grab(ctx: &egui::Context, raw: &mut egui::RawInput) {
    let viewport = raw.viewport_id;
    if ctx.data_mut(|d| d.remove_temp::<()>(grab_id(viewport))).is_none() {
        return;
    }
    let mut at = raw.events.len();
    let mut gone = None;
    let mut brought = None;
    for (n, e) in raw.events.iter().enumerate() {
        match e {
            egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed: false, .. } => {
                brought = Some(n);
                break;
            }
            egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed: true, .. } => {
                at = n;
                break;
            }
            egui::Event::PointerGone => {
                gone.get_or_insert(n);
            }
            _ => {}
        }
    }
    if let Some(n) = brought {
        // `gone` was found before `n`, so both are in range.
        if let Some(g) = gone {
            let release = raw.events.remove(n);
            raw.events.insert(g, release);
        }
        return;
    }
    let at = gone.unwrap_or(at);
    let (pos, modifiers) = ctx.input_for(viewport, |i| (i.pointer.interact_pos().unwrap_or_default(), i.modifiers));
    raw.events.insert(at, egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers });
}

/// Once the desktop has let go of the pointer after [`desktop_grab`], give the keyboard back to the
/// widget that had it in the window being drawn, so typing goes on after a move or resize: the
/// press on the edge or title bar took it, as any press elsewhere does in egui. [`DesktopGrab`]
/// calls this at the start of every pass.
pub fn focus_after_desktop_grab(ctx: &egui::Context) {
    // Not while the button is down, nor in the pass that takes the given-back release: egui would
    // take the keyboard away again for that click outside the widget.
    if ctx.input(|i| i.pointer.any_down() || i.pointer.any_released()) {
        return;
    }
    let focus_id = grab_focus_id(ctx.viewport_id());
    let widget = ctx.data_mut(|d| {
        let widget = d.get_temp::<egui::Id>(focus_id);
        d.remove::<egui::Id>(focus_id);
        widget
    });
    if let Some(widget) = widget {
        ctx.memory_mut(|m| m.request_focus(widget));
    }
}

/// An egui plugin that runs [`release_after_desktop_grab`] on every window's input and
/// [`focus_after_desktop_grab`] at the start of every window's pass. Install it once with
/// [`install`].
#[derive(Clone, Copy, Debug, Default)]
pub struct DesktopGrab;

impl egui::Plugin for DesktopGrab {
    fn debug_name(&self) -> &'static str {
        "craft_window_desktop_grab"
    }

    fn input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        release_after_desktop_grab(ctx, input);
    }

    fn on_begin_pass(&mut self, ui: &mut egui::Ui) {
        focus_after_desktop_grab(ui.ctx());
    }
}

/// Register [`DesktopGrab`] with `ctx`. Once it is registered this only looks it up, so an app
/// may call it every frame.
pub fn install(ctx: &egui::Context) {
    if ctx.plugin_opt::<DesktopGrab>().is_none() {
        ctx.add_plugin(DesktopGrab);
    }
}

#[cfg(test)]
mod tests {
    use super::{BUTTON, Button, GAP, Layout, ResizeDirection, SIDE_GAP, zones};
    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, ViewportCommand, ViewportId};

    fn button(pressed: bool) -> Event {
        Event::PointerButton { pos: Pos2::new(5.0, 5.0), button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
    }

    fn release(e: &Event) -> bool {
        matches!(e, Event::PointerButton { button: PointerButton::Primary, pressed: false, .. })
    }

    fn primary(raw: &RawInput) -> Vec<bool> {
        raw.events.iter().filter_map(|e| if let Event::PointerButton { pressed, .. } = e { Some(*pressed) } else { None }).collect()
    }

    #[test]
    fn widths_cover_the_buttons_and_their_gaps() {
        assert_eq!(super::width(&[]), 0.0);
        assert_eq!(super::width(&[Button::Close]), BUTTON + SIDE_GAP);
        assert_eq!(super::width(&Layout::default().right), 3.0 * BUTTON + 2.0 * GAP + SIDE_GAP);
    }

    #[test]
    fn each_button_asks_the_window_for_its_own_command() {
        assert_eq!(super::command(Button::Minimize, false), ViewportCommand::Minimized(true));
        assert_eq!(super::command(Button::Maximize, false), ViewportCommand::Maximized(true));
        assert_eq!(super::command(Button::Maximize, true), ViewportCommand::Maximized(false));
        assert_eq!(super::command(Button::Close, false), ViewportCommand::Close);
    }

    #[test]
    fn the_buttons_are_a_gap_apart_whatever_the_item_spacing() {
        let ctx = egui::Context::default();
        let mut placed = Vec::new();
        ctx.run_ui(RawInput::default(), |ui| {
            ui.spacing_mut().item_spacing.x = 20.0;
            ui.horizontal(|ui| {
                let before = ui.cursor().min.x;
                super::buttons(ui, &Layout::default().right, &super::Look::from_visuals(ui.visuals()), &super::Labels::default());
                placed.push(ui.cursor().min.x - before);
                placed.push(ui.spacing().item_spacing.x);
            });
        })
        .textures_delta
        .clear();
        assert_eq!(placed, [3.0 * BUTTON + 2.0 * GAP, 20.0], "three buttons two gaps wide, and the caller's spacing back");
    }

    #[test]
    fn the_look_follows_the_theme() {
        let light = super::Look::from_visuals(&egui::Visuals::light());
        let dark = super::Look::from_visuals(&egui::Visuals::dark());
        assert_eq!(light.glyph, egui::Visuals::light().text_color());
        assert_eq!(dark.focus, egui::Visuals::dark().selection.stroke.color);
        assert!(dark.outline.a() > light.outline.a(), "a stronger hairline on a dark theme");
    }

    #[test]
    fn resize_zones_line_the_edges_without_overlapping() {
        let window = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let zones = zones(window, 5.0, 16.0);
        let at = |x: f32, y: f32| zones.iter().filter(|(_, r)| r.contains(Pos2::new(x, y))).map(|(d, _)| *d).collect::<Vec<_>>();
        assert_eq!(at(1.0, 1.0), [ResizeDirection::NorthWest]);
        assert_eq!(at(2.0, 12.0), [ResizeDirection::NorthWest]);
        assert_eq!(at(400.0, 2.0), [ResizeDirection::North]);
        assert_eq!(at(799.0, 300.0), [ResizeDirection::East]);
        assert_eq!(at(400.0, 599.0), [ResizeDirection::South]);
        assert_eq!(at(1.0, 300.0), [ResizeDirection::West]);
        assert_eq!(at(799.0, 599.0), [ResizeDirection::SouthEast]);
        assert!(at(400.0, 300.0).is_empty(), "the middle of the window is left alone");
        assert!(at(12.0, 12.0).is_empty(), "corners stay along the edges");
        // Strips meet without overlapping.
        for y in [0.5, 4.5, 5.5, 15.5, 16.5, 595.5] {
            for x in [0.5, 4.5, 5.5, 15.5, 16.5, 795.5, 799.5] {
                assert!(at(x, y).len() <= 1, "({x}, {y}): {:?}", at(x, y));
            }
        }
        // A window smaller than its corners gets empty (not inverted) edge zones.
        for (_, r) in zones_of(Rect::from_min_size(Pos2::ZERO, egui::vec2(20.0, 20.0))) {
            assert!(r.width() >= 0.0 && r.height() >= 0.0);
        }
    }

    fn zones_of(rect: Rect) -> Vec<(ResizeDirection, Rect)> {
        zones(rect, 5.0, 16.0)
    }

    #[test]
    fn a_desktop_grab_gives_the_button_release_back() {
        let ctx = egui::Context::default();
        let releases = |raw: &RawInput| raw.events.iter().filter(|e| release(e)).count();
        let mut raw = RawInput::default();
        super::release_after_desktop_grab(&ctx, &mut raw);
        assert_eq!(releases(&raw), 0, "no grab, no release");

        super::desktop_grab(&ctx, ViewportCommand::StartDrag);
        super::release_after_desktop_grab(&ctx, &mut raw);
        assert_eq!(releases(&raw), 1, "the release the desktop kept");

        let mut raw = RawInput::default();
        super::release_after_desktop_grab(&ctx, &mut raw);
        assert_eq!(releases(&raw), 0, "only once");

        super::desktop_grab(&ctx, ViewportCommand::BeginResize(ResizeDirection::East));
        let mut raw = RawInput { events: vec![button(false)], ..Default::default() };
        super::release_after_desktop_grab(&ctx, &mut raw);
        assert_eq!(releases(&raw), 1, "not doubled when the input brings it");
    }

    #[test]
    fn the_release_goes_back_to_the_window_that_was_moved() {
        let ctx = egui::Context::default();
        let second = ViewportId::from_hash_of("second window");
        // The input a backend gives the second window names it among the viewports.
        let input =
            || RawInput { viewport_id: second, viewports: std::iter::once((second, egui::ViewportInfo::default())).collect(), ..Default::default() };
        // The grab is made while the second window is drawn.
        ctx.run_ui(input(), |ui| super::desktop_grab(ui.ctx(), ViewportCommand::StartDrag)).textures_delta.clear();

        let mut root = RawInput::default();
        super::release_after_desktop_grab(&ctx, &mut root);
        assert!(primary(&root).is_empty(), "the first window's input is left alone");

        let mut input = input();
        super::release_after_desktop_grab(&ctx, &mut input);
        assert_eq!(primary(&input), [false], "the second window gets its release");
    }

    #[test]
    fn the_release_goes_before_the_press_of_the_next_grab() {
        let ctx = egui::Context::default();
        let frame = |events: Vec<Event>, grab: Option<ViewportCommand>| {
            let mut raw = RawInput { events, ..Default::default() };
            super::release_after_desktop_grab(&ctx, &mut raw);
            let order = primary(&raw);
            ctx.run_ui(raw, |ui| {
                if let Some(command) = grab.clone() {
                    super::desktop_grab(ui.ctx(), command);
                }
            })
            .textures_delta
            .clear();
            order
        };
        // The press of the first resize starts the grab; the desktop keeps its release.
        frame(vec![Event::PointerMoved(Pos2::new(5.0, 5.0)), button(true)], Some(ViewportCommand::BeginResize(ResizeDirection::East)));
        // The next input brings the press of the second resize.
        assert_eq!(frame(vec![button(true)], None), [false, true], "the kept release, then the new press");
        assert!(ctx.input(|i| i.pointer.primary_down()), "the second resize holds the button");

        // A quick click's release belongs to its own press, not to the grab before it.
        frame(vec![], Some(ViewportCommand::StartDrag));
        assert_eq!(frame(vec![button(true), button(false)], None), [false, true, false]);
    }

    #[test]
    fn the_release_goes_before_the_pointer_leaving() {
        let ctx = egui::Context::default();
        let frame = |events: Vec<Event>, grab: bool| {
            let mut raw = RawInput { events, ..Default::default() };
            super::release_after_desktop_grab(&ctx, &mut raw);
            let order = raw.events.clone();
            ctx.run_ui(raw, |ui| {
                if grab {
                    super::desktop_grab(ui.ctx(), ViewportCommand::StartDrag);
                }
            })
            .textures_delta
            .clear();
            order
        };
        // On Wayland the input after a move brings only that the pointer left.
        frame(vec![Event::PointerMoved(Pos2::new(5.0, 5.0)), button(true)], true);
        let order = frame(vec![Event::PointerGone], false);
        assert!(matches!(order.as_slice(), [r, Event::PointerGone] if release(r)), "{order:?}");
        assert_eq!(ctx.input(|i| i.pointer.latest_pos()), None, "the pointer stays gone");
        assert!(!ctx.input(|i| i.pointer.primary_down()));

        // A release after the pointer left is the one the desktop kept, not to be doubled.
        frame(vec![], true);
        let order = frame(vec![Event::PointerGone, button(false)], false);
        assert!(matches!(order.as_slice(), [r, Event::PointerGone] if release(r)), "moved, not doubled: {order:?}");
        assert_eq!(ctx.input(|i| i.pointer.latest_pos()), None, "the pointer stays gone");
    }

    #[test]
    fn the_plugin_gives_back_the_button_and_the_keyboard() {
        let ctx = egui::Context::default();
        super::install(&ctx);
        super::install(&ctx);
        let look = super::Look::from_visuals(&egui::Visuals::light());
        let field = egui::Id::unique("the page");
        let mut text = String::new();
        let window = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let mut pass = |events: Vec<Event>| {
            ctx.run_ui(RawInput { screen_rect: Some(window), events, ..Default::default() }, |ui| {
                ui.add(egui::TextEdit::singleline(&mut text).id(field));
                super::resize_edges(ui.ctx(), &look);
            })
            .textures_delta
            .clear();
            ctx.memory(|m| m.has_focus(field))
        };
        pass(vec![]);
        ctx.memory_mut(|m| m.request_focus(field));
        assert!(pass(vec![]), "the page has the keyboard");

        // A press on the right edge starts a resize; the desktop keeps the release.
        let edge = Pos2::new(798.0, 300.0);
        pass(vec![Event::PointerMoved(edge)]);
        pass(vec![Event::PointerButton { pos: edge, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
        assert!(ctx.input(|i| i.pointer.primary_down()));
        // The plugin gives the release back. It completes a click outside the page, which takes
        // the keyboard from it.
        pass(vec![]);
        assert!(!ctx.input(|i| i.pointer.primary_down()), "the button is up again");
        // Once the desktop has let go, the page gets the keyboard back.
        pass(vec![]);
        assert!(pass(vec![]), "typing goes on in the page");
    }
}
