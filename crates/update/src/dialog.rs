//! Startup update prompt dialog for egui frontends.

use egui::{Align, Align2, Color32, Layout, RichText, Vec2};

use crate::app::App;
use crate::release::Release;
use crate::updater::Status;

/// What the user chose in the startup update prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupPromptResponse {
    /// Dialog is still open and waiting for user input.
    Pending,
    /// User chose to update and restart now.
    UpdateAndRestart,
    /// User chose to download manually via browser.
    Download,
    /// User dismissed the dialog for now.
    Later,
}

/// State for the startup update prompt dialog.
#[derive(Clone, Debug)]
pub struct StartupPrompt {
    pub app: &'static App,
    pub release: Release,
    pub running_version: String,
    pub is_self_updating: bool,
    pub open: bool,
    pub status_message: Option<String>,
    pub is_installing: bool,
}

impl StartupPrompt {
    /// Creates a new prompt dialog state for the given app and newly discovered release.
    pub fn new(app: &'static App, release: Release, running_version: impl Into<String>) -> Self {
        let is_self_updating = crate::self_update::is_self_updating(app);
        Self { app, release, running_version: running_version.into(), is_self_updating, open: true, status_message: None, is_installing: false }
    }

    /// Renders the modal window on the given egui context.
    pub fn show(&mut self, ctx: &egui::Context) -> StartupPromptResponse {
        if !self.open {
            return StartupPromptResponse::Later;
        }

        let mut response = StartupPromptResponse::Pending;

        backdrop(ctx);

        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            response = StartupPromptResponse::Later;
        }

        let pal = Palette::new(ctx, self.app);
        let Palette { is_dark, card_bg, border_color, text_primary, text_muted, fill, on_fill } = pal;

        egui::Area::new(egui::Id::unique("craft_update.startup_prompt"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(card_bg)
                    .stroke(egui::Stroke::new(1.0, border_color))
                    .corner_radius(egui::CornerRadius::ZERO)
                    .inner_margin(egui::Margin::symmetric(20, 18))
                    .show(ui, |ui| {
                        ui.set_width(400.0);
                        style_card(ui, &pal);
                        let version_line = format!("v{} to {}", self.running_version, self.release.version_label());
                        if card_header(ui, self.app, &pal, true, &version_line) {
                            response = StartupPromptResponse::Later;
                        }

                        ui.add_space(8.0);

                        if let Some(msg) = &self.status_message {
                            let alert_bg = if is_dark { Color32::from_rgb(50, 30, 20) } else { Color32::from_rgb(255, 242, 235) };
                            let alert_color = Color32::from_rgb(225, 95, 30);
                            egui::Frame::new()
                                .fill(alert_bg)
                                .stroke(egui::Stroke::new(1.0, alert_color))
                                .corner_radius(egui::CornerRadius::ZERO)
                                .inner_margin(egui::Margin::symmetric(10, 6))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(RichText::new(msg).size(12.0).color(alert_color));
                                });
                            ui.add_space(4.0);
                        }

                        if self.is_installing {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Downloading and installing update…").size(12.0).color(text_primary));
                            });
                        } else {
                            let desc =
                                format!("{} {} is available with the latest features and bug fixes.", self.app.name, self.release.version_label());
                            ui.label(RichText::new(desc).size(12.5).color(text_muted));
                        }

                        ui.add_space(10.0);
                        let (hairline_rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
                        ui.painter().hline(hairline_rect.x_range(), hairline_rect.top(), egui::Stroke::new(1.0, border_color));
                        ui.add_space(8.0);

                        // Footer actions: a row as tall as the buttons, everything centred in it, so the link
                        // lines up with them (a plain horizontal row puts it at the top).
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.set_min_height(26.0);
                            // Body text, so it reads on any card; the underline carries the accent.
                            let link =
                                ui.add(egui::Label::new(RichText::new("What's new").size(11.5).color(text_primary)).sense(egui::Sense::click()));
                            link.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, "What's new"));
                            let y = link.rect.bottom() - 0.5;
                            ui.painter()
                                .line_segment([egui::pos2(link.rect.left(), y), egui::pos2(link.rect.right(), y)], egui::Stroke::new(1.0, fill));
                            // Keyboard focus (Tab, then Enter or Space opens it): the buttons' 2 px ring.
                            if link.has_focus() {
                                let ring = egui::Stroke::new(2.0, text_primary);
                                ui.painter().rect_stroke(link.rect.expand(2.0), egui::CornerRadius::ZERO, ring, egui::StrokeKind::Outside);
                            }
                            if link.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                ui.ctx().open_url(egui::OpenUrl::new_tab(&self.release.html_url));
                            }

                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if self.is_installing {
                                    let dismiss_btn = egui::Button::new(RichText::new("DISMISS").size(10.5).monospace().color(text_muted))
                                        .corner_radius(egui::CornerRadius::ZERO)
                                        .min_size(Vec2::new(72.0, 26.0));
                                    if ui.add(dismiss_btn).clicked() {
                                        response = StartupPromptResponse::Later;
                                    }
                                } else {
                                    if self.is_self_updating {
                                        let update_btn =
                                            egui::Button::new(RichText::new("UPDATE AND RESTART").size(10.5).strong().monospace().color(on_fill))
                                                .fill(fill)
                                                .corner_radius(egui::CornerRadius::ZERO)
                                                .min_size(Vec2::new(140.0, 26.0));
                                        if ui.add(update_btn).clicked() {
                                            response = StartupPromptResponse::UpdateAndRestart;
                                        }
                                    } else {
                                        let dl_btn = egui::Button::new(RichText::new("DOWNLOAD").size(10.5).strong().monospace().color(on_fill))
                                            .fill(fill)
                                            .corner_radius(egui::CornerRadius::ZERO)
                                            .min_size(Vec2::new(90.0, 26.0));
                                        if ui.add(dl_btn).clicked() {
                                            response = StartupPromptResponse::Download;
                                        }
                                    }

                                    let later_btn = egui::Button::new(RichText::new("LATER").size(10.5).monospace().color(text_primary))
                                        .corner_radius(egui::CornerRadius::ZERO)
                                        .min_size(Vec2::new(65.0, 26.0));
                                    if ui.add(later_btn).clicked() {
                                        response = StartupPromptResponse::Later;
                                    }
                                }
                            });
                        });
                    });
            });

        if response == StartupPromptResponse::Later {
            self.open = false;
        }

        response
    }
}

/// The card's colours for the current theme and app.
#[derive(Clone, Copy)]
struct Palette {
    is_dark: bool,
    card_bg: Color32,
    border_color: Color32,
    text_primary: Color32,
    text_muted: Color32,
    /// The app's accent.
    fill: Color32,
    /// Text on `fill`.
    on_fill: Color32,
}

impl Palette {
    fn new(ctx: &egui::Context, app: &App) -> Self {
        let is_dark = ctx.theme() == egui::Theme::Dark;
        let fill = app.accent.map_or(DEFAULT_ACCENT, |[r, g, b]| Color32::from_rgb(r, g, b));
        Self {
            is_dark,
            card_bg: if is_dark { Color32::from_rgb(26, 26, 28) } else { Color32::from_rgb(255, 255, 255) },
            border_color: if is_dark { Color32::from_rgb(52, 52, 56) } else { Color32::from_rgb(215, 215, 220) },
            text_primary: if is_dark { Color32::from_rgb(245, 245, 248) } else { Color32::from_rgb(20, 20, 25) },
            text_muted: if is_dark { Color32::from_rgb(150, 150, 160) } else { Color32::from_rgb(105, 105, 115) },
            fill,
            on_fill: ink_on(fill),
        }
    }
}

/// The dimmed modal backdrop; it senses clicks so they don't reach widgets underneath. True when
/// it was clicked (a click outside the card).
fn backdrop(ctx: &egui::Context) -> bool {
    let screen_rect = ctx.content_rect();
    egui::Area::new(egui::Id::unique("craft_update_backdrop"))
        .order(egui::Order::Middle)
        .fixed_pos(screen_rect.min)
        .show(ctx, |ui| {
            let (rect, response) = ui.allocate_exact_size(screen_rect.size(), egui::Sense::click_and_drag());
            ui.painter().rect_filled(rect, 0.0, Color32::from_black_alpha(150));
            response.clicked()
        })
        .inner
}

/// The host app's widget style isn't ours to rely on (a light theme's hover can be dark under
/// this card's dark text), so the card's buttons get their own states.
fn style_card(ui: &mut egui::Ui, pal: &Palette) {
    let widgets = &mut ui.visuals_mut().widgets;
    let (hover_bg, press_bg) = button_states(pal.is_dark);
    for (state, bg) in [(&mut widgets.inactive, pal.card_bg), (&mut widgets.hovered, hover_bg), (&mut widgets.active, press_bg)] {
        state.weak_bg_fill = bg;
        state.bg_fill = bg;
        state.bg_stroke = egui::Stroke::new(1.0, pal.border_color);
    }
    // egui draws a focused button (and a pressed one) in the active state: its border is
    // the focus ring, in the body text colour so it shows on the card and around a fill.
    widgets.active.bg_stroke = egui::Stroke::new(2.0, pal.text_primary);
    ui.spacing_mut().item_spacing = Vec2::new(8.0, 6.0);
}

/// Header row: app icon (or its initial in a badge), name, developer, `second_line` and a close
/// button. `update_pill` adds the "UPDATE" chip. True when the close button was clicked.
fn card_header(ui: &mut egui::Ui, app: &App, pal: &Palette, update_pill: bool, second_line: &str) -> bool {
    let Palette { text_primary, text_muted, fill, on_fill, .. } = *pal;
    let mut closed = false;
    ui.horizontal(|ui| {
        let (badge_rect, _) = ui.allocate_exact_size(Vec2::new(34.0, 34.0), egui::Sense::hover());
        if let Some(icon) = icon_texture(ui.ctx(), app) {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(icon.id(), badge_rect, uv, Color32::WHITE);
        } else {
            ui.painter().rect_filled(badge_rect, egui::CornerRadius::ZERO, fill);
            let initial = app.name.chars().next().map(|c| c.to_string()).unwrap_or_else(|| "A".to_string());
            ui.painter().text(badge_rect.center(), Align2::CENTER_CENTER, initial, egui::FontId::monospace(18.0), on_fill);
        }

        ui.add_space(4.0);

        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.horizontal(|ui| {
                ui.label(RichText::new(app.name).size(15.0).strong().color(text_primary));
                ui.label(RichText::new(format!("by {}", app.developer)).size(11.5).color(text_muted));
                if update_pill {
                    // A chip, not accent-coloured text: small accent text misses WCAG's 4.5:1 for
                    // several apps (PdfCraft's red on the dark card, amber on the light one).
                    egui::Frame::new().fill(fill).inner_margin(egui::Margin::symmetric(4, 1)).show(ui, |ui| {
                        ui.label(RichText::new("UPDATE").size(10.0).monospace().color(on_fill));
                    });
                }
            });
            // ASCII only, like the rest of the card: an app whose fonts lack a glyph may panic
            // instead of drawing a box, and the startup prompt shows up unasked.
            ui.label(RichText::new(second_line).size(11.0).monospace().color(text_muted));
        });

        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            // Two strokes, not a "✕" glyph: none of the bundled faces has U+2715.
            let (rect, close) = ui.allocate_exact_size(Vec2::splat(18.0), egui::Sense::click());
            close.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Close"));
            let color = if close.hovered() { text_primary } else { text_muted };
            let (center, reach) = (rect.center(), 4.5);
            let stroke = egui::Stroke::new(1.5, color);
            ui.painter().line_segment([center + Vec2::new(-reach, -reach), center + Vec2::new(reach, reach)], stroke);
            ui.painter().line_segment([center + Vec2::new(-reach, reach), center + Vec2::new(reach, -reach)], stroke);
            if close.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                closed = true;
            }
        });
    });
    closed
}

/// What the user chose in the result dialog of a manual update check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckResultResponse {
    /// Dialog is still open and waiting for user input.
    None,
    /// OK, the close button, Esc or a click outside: the app stops showing the dialog.
    Close,
    /// Failed state only: the app starts another requested check.
    CheckAgain,
}

/// The result of a manual "Check for Updates", in the same card as [`StartupPrompt`]: a spinner
/// while checking, "newest version" when up to date, the error and a retry when it failed.
///
/// An app shows it while a requested check is in view: open it when the user starts a check
/// (`Updater::check_now`), call [`show`](Self::show) every frame with `updater.status()` until it
/// returns [`CheckResultResponse::Close`], then stop calling it. On
/// [`CheckResultResponse::CheckAgain`] start another requested check and keep showing it. An
/// available update goes to [`StartupPrompt`] instead; the app closes this dialog when the prompt opens.
#[derive(Clone, Copy, Debug)]
pub struct CheckResultDialog {
    app: &'static App,
    running_version: &'static str,
}

impl CheckResultDialog {
    /// A dialog for `app`, which is running `running_version`.
    pub fn new(app: &'static App, running_version: &'static str) -> Self {
        Self { app, running_version }
    }

    /// Draws the modal card for `status` and returns what the user chose. [`Status::Idle`] draws
    /// nothing and returns [`CheckResultResponse::None`].
    pub fn show(&self, ctx: &egui::Context, status: &Status) -> CheckResultResponse {
        if matches!(status, Status::Idle) {
            return CheckResultResponse::None;
        }
        let mut response = CheckResultResponse::None;
        if backdrop(ctx) {
            response = CheckResultResponse::Close;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            response = CheckResultResponse::Close;
        }

        let pal = Palette::new(ctx, self.app);
        let Palette { card_bg, border_color, text_primary, text_muted, fill, on_fill, .. } = pal;
        egui::Area::new(egui::Id::unique("craft_update.check_result")).anchor(Align2::CENTER_CENTER, Vec2::ZERO).order(egui::Order::Foreground).show(
            ctx,
            |ui| {
                egui::Frame::new()
                    .fill(card_bg)
                    .stroke(egui::Stroke::new(1.0, border_color))
                    .corner_radius(egui::CornerRadius::ZERO)
                    .inner_margin(egui::Margin::symmetric(20, 18))
                    .show(ui, |ui| {
                        ui.set_width(400.0);
                        style_card(ui, &pal);
                        if card_header(ui, self.app, &pal, false, &format!("v{}", self.running_version)) {
                            response = CheckResultResponse::Close;
                        }
                        ui.add_space(8.0);

                        match status {
                            Status::Checking => {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(RichText::new("Checking for updates…").size(12.5).color(text_primary));
                                });
                            }
                            Status::UpToDate => {
                                let text = format!("{} {} is the newest version available.", self.app.name, self.running_version);
                                ui.label(RichText::new(text).size(12.5).color(text_muted));
                            }
                            Status::Failed(error) => {
                                let alert_color = Color32::from_rgb(225, 95, 30);
                                ui.label(RichText::new("Could not check for updates.").size(12.5).strong().color(alert_color));
                                ui.add(egui::Label::new(RichText::new(error).size(12.0).color(text_muted)).selectable(true));
                                ui.label(
                                    RichText::new(format!("All releases are listed at {}", self.app.releases_url())).size(11.5).color(text_muted),
                                );
                            }
                            Status::Idle => {}
                        }

                        if !matches!(status, Status::Checking) {
                            ui.add_space(10.0);
                            let (hairline_rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
                            ui.painter().hline(hairline_rect.x_range(), hairline_rect.top(), egui::Stroke::new(1.0, border_color));
                            ui.add_space(8.0);
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.set_min_height(26.0);
                                let ok = egui::Button::new(RichText::new("OK").size(10.5).strong().monospace().color(on_fill))
                                    .fill(fill)
                                    .corner_radius(egui::CornerRadius::ZERO)
                                    .min_size(Vec2::new(65.0, 26.0));
                                if ui.add(ok).clicked() {
                                    response = CheckResultResponse::Close;
                                }
                                if matches!(status, Status::Failed(_)) {
                                    let again = egui::Button::new(RichText::new("CHECK AGAIN").size(10.5).monospace().color(text_primary))
                                        .corner_radius(egui::CornerRadius::ZERO)
                                        .min_size(Vec2::new(100.0, 26.0));
                                    if ui.add(again).clicked() {
                                        response = CheckResultResponse::CheckAgain;
                                    }
                                }
                            });
                        }
                    });
            },
        );
        response
    }
}

/// Backgrounds of an unfilled button (LATER, DISMISS) when hovered and when pressed: a step from
/// the card, so its text keeps the contrast it has at rest.
fn button_states(is_dark: bool) -> (Color32, Color32) {
    if is_dark {
        (Color32::from_rgb(44, 44, 48), Color32::from_rgb(58, 58, 64))
    } else {
        (Color32::from_rgb(236, 236, 241), Color32::from_rgb(222, 222, 230))
    }
}

/// The prompt's blue for an app that names no accent ([`App::accent`]).
const DEFAULT_ACCENT: Color32 = Color32::from_rgb(52, 120, 246);

/// WCAG relative luminance of `c`: 0 for black to 1 for white.
fn luminance(c: Color32) -> f32 {
    let channel = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
}

/// WCAG contrast ratio of two colours: 1 (none) to 21 (black on white).
fn contrast(a: Color32, b: Color32) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// Text for a label on `fill`: white or black, whichever contrasts more. Every Craft app accent
/// then reaches WCAG AA's 4.5:1 for the prompt's small labels (5:1 at the least).
fn ink_on(fill: Color32) -> Color32 {
    if contrast(fill, Color32::WHITE) >= contrast(fill, Color32::BLACK) { Color32::WHITE } else { Color32::BLACK }
}

/// The largest icon the prompt decodes, a side; app icons are 128 px.
const ICON_MAX_SIDE: u32 = 1024;

/// `app`'s icon as a texture, decoded once per context and app; `None` when the app has none or
/// it doesn't decode, and the prompt shows the app's initial instead.
fn icon_texture(ctx: &egui::Context, app: &App) -> Option<egui::TextureHandle> {
    let png = app.icon_png?;
    let id = egui::Id::unique(("craft_update.icon", app.id));
    if let Some(cached) = ctx.data(|d| d.get_temp::<Option<egui::TextureHandle>>(id)) {
        return cached;
    }
    // The renderer's own limit too: load_texture panics (debug) on a texture larger than it takes.
    let max_side = u32::try_from(ctx.input(|i| i.max_texture_side)).unwrap_or(u32::MAX).min(ICON_MAX_SIDE);
    let texture =
        decode_icon(png, max_side).map(|image| ctx.load_texture(format!("craft_update.icon.{}", app.id), image, egui::TextureOptions::LINEAR));
    ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
    texture
}

/// A PNG as an RGBA image, or `None` when it doesn't decode or is over `max_side` a side.
fn decode_icon(png: &[u8], max_side: u32) -> Option<egui::ColorImage> {
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(max_side);
    limits.max_image_height = Some(max_side);
    reader.limits(limits);
    let rgba = reader.decode().ok()?.to_rgba8();
    let size = [usize::try_from(rgba.width()).ok()?, usize::try_from(rgba.height()).ok()?];
    Some(egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite::PHOTOCRAFT;
    use egui::{Context, Event, FullOutput, Key, Modifiers, RawInput, Rect};

    const DARK_CARD: Color32 = Color32::from_rgb(26, 26, 28);
    const DARK_STATUS: Color32 = Color32::from_rgb(50, 30, 20);

    fn screen() -> Rect {
        Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 480.0))
    }

    /// A dark, animation-free context so a single frame paints final (unfaded) colors.
    fn dark_context() -> Context {
        let ctx = Context::default();
        ctx.set_theme(egui::ThemePreference::Dark);
        ctx.global_style_mut(|style| style.animation_time = 0.0);
        ctx
    }

    /// A prompt in a known installation mode, without probing the host for a real install.
    fn prompt_for(is_self_updating: bool) -> StartupPrompt {
        let release = Release { version: crate::version::parse("0.2.0"), tag: "v0.2.0".to_string(), ..Release::default() };
        StartupPrompt {
            app: &PHOTOCRAFT,
            release,
            running_version: "0.1.0".to_string(),
            is_self_updating,
            open: true,
            status_message: None,
            is_installing: false,
        }
    }

    fn frame(ctx: &Context, prompt: &mut StartupPrompt, events: Vec<Event>) -> (StartupPromptResponse, FullOutput) {
        let mut response = StartupPromptResponse::Pending;
        let mut output = ctx.run_ui(RawInput { screen_rect: Some(screen()), events, ..RawInput::default() }, |ui| {
            response = prompt.show(ui.ctx());
        });
        output.textures_delta.clear(); // Headless test: no renderer consumes the font atlas.
        (response, output)
    }

    fn collect_text(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect_text(shape, out);
                }
            }
            egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
            _ => {}
        }
    }

    /// Every string the frame actually painted, one per line.
    fn painted_text(output: &FullOutput) -> String {
        let mut texts = Vec::new();
        for clipped in &output.shapes {
            collect_text(&clipped.shape, &mut texts);
        }
        texts.join("\n")
    }

    /// The rectangle fills painted this frame, so the card and alert surfaces can be told apart.
    fn rect_fills(output: &FullOutput) -> Vec<(Rect, Color32)> {
        fn collect(shape: &egui::Shape, out: &mut Vec<(Rect, Color32)>) {
            match shape {
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, out);
                    }
                }
                egui::Shape::Rect(rect) => out.push((rect.rect, rect.fill)),
                _ => {}
            }
        }
        let mut fills = Vec::new();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut fills);
        }
        fills
    }

    /// A `width` x `height` PNG of one opaque color.
    fn png(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(width, height, image::Rgba(color)).write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    /// The textured meshes painted this frame (the app icon; text is drawn as `Shape::Text`).
    fn textured_meshes(output: &FullOutput) -> Vec<egui::TextureId> {
        fn collect(shape: &egui::Shape, out: &mut Vec<egui::TextureId>) {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
                egui::Shape::Mesh(mesh) if mesh.texture_id != egui::TextureId::default() => out.push(mesh.texture_id),
                _ => {}
            }
        }
        let mut ids = Vec::new();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut ids);
        }
        ids
    }

    #[test]
    fn icons_decode_and_bad_ones_are_refused() {
        let image = decode_icon(&png(3, 2, [10, 20, 30, 255]), ICON_MAX_SIDE).unwrap();
        assert_eq!(image.size, [3, 2]);
        assert_eq!(image.pixels[0], Color32::from_rgb(10, 20, 30));
        assert!(decode_icon(b"\x89PNG\r\n\x1a\nnot really", ICON_MAX_SIDE).is_none(), "a corrupt PNG must not decode");
        assert!(decode_icon(&png(ICON_MAX_SIDE + 1, 1, [0, 0, 0, 255]), ICON_MAX_SIDE).is_none(), "an oversized icon must not decode");
        // A renderer that takes smaller textures than the icon: no texture, so the initial is shown.
        assert!(decode_icon(&png(3, 2, [0, 0, 0, 255]), 2).is_none(), "an icon over the renderer's limit must not decode");
    }

    #[test]
    fn prompt_shows_the_app_icon_or_else_its_initial() {
        static ICON_APP: std::sync::OnceLock<App> = std::sync::OnceLock::new();
        let icon_png: &'static [u8] = Box::leak(png(128, 128, [200, 40, 40, 255]).into_boxed_slice());
        let with_icon: &'static App = ICON_APP.get_or_init(|| App { icon_png: Some(icon_png), ..PHOTOCRAFT });
        let ctx = dark_context();
        let mut prompt = StartupPrompt { app: with_icon, ..prompt_for(false) };
        frame(&ctx, &mut prompt, Vec::new());
        let (_, output) = frame(&ctx, &mut prompt, Vec::new());
        assert_eq!(textured_meshes(&output).len(), 1, "the app icon is drawn");
        let badge = rect_fills(&output).iter().any(|(rect, fill)| *fill == DEFAULT_ACCENT && rect.width() == 34.0);
        assert!(!badge, "no letter badge beside the icon");
        assert!(!painted_text(&output).lines().any(|l| l == "P"), "no initial beside the icon");

        // No icon (or one that won't decode): the initial in the accent badge, as before.
        let ctx = dark_context();
        let mut prompt = prompt_for(false);
        frame(&ctx, &mut prompt, Vec::new());
        let (_, output) = frame(&ctx, &mut prompt, Vec::new());
        assert!(textured_meshes(&output).is_empty());
        assert!(painted_text(&output).lines().any(|l| l == "P"), "the initial stands in for a missing icon");
    }

    #[test]
    fn an_apps_accent_fills_the_buttons_with_readable_text() {
        // An amber accent: its buttons take it, with black text (white is under 3:1 on it).
        static AMBER: std::sync::OnceLock<App> = std::sync::OnceLock::new();
        let amber: &'static App = AMBER.get_or_init(|| App { accent: Some([0xf2, 0xa5, 0x16]), ..PHOTOCRAFT });
        let ctx = dark_context();
        let mut prompt = StartupPrompt { app: amber, ..prompt_for(false) };
        frame(&ctx, &mut prompt, Vec::new());
        let (_, output) = frame(&ctx, &mut prompt, Vec::new());
        let fill = Color32::from_rgb(0xf2, 0xa5, 0x16);
        assert!(rect_fills(&output).iter().any(|(_, f)| *f == fill), "the accent fills the button and badge");
        assert!(!rect_fills(&output).iter().any(|(_, f)| *f == DEFAULT_ACCENT), "no default blue beside an accent");
        assert_eq!(ink_on(fill), Color32::BLACK);
        // Whichever of white and black contrasts more: black on the default blue, white on
        // WordCraft's deeper blue and PdfCraft's red.
        assert_eq!(ink_on(DEFAULT_ACCENT), Color32::BLACK);
        assert_eq!(ink_on(Color32::from_rgb(0x3f, 0x5f, 0xe0)), Color32::WHITE);
        assert_eq!(ink_on(Color32::from_rgb(0xb7, 0x02, 0x28)), Color32::WHITE);
        // The accent underlines "What's new" (whose text is the body colour).
        let underlined = line_strokes(&output).contains(&fill);
        assert!(underlined, "the link's underline takes the accent");
        assert!((contrast(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.01);
    }

    #[test]
    fn hovering_later_uses_the_prompts_own_background_not_the_hosts() {
        // A host whose light theme hovers in black: LATER's dark text would vanish on it.
        let ctx = Context::default();
        ctx.set_theme(egui::ThemePreference::Light);
        ctx.global_style_mut(|style| {
            style.animation_time = 0.0;
            style.visuals.widgets.hovered.weak_bg_fill = Color32::BLACK;
            style.visuals.widgets.hovered.bg_fill = Color32::BLACK;
        });
        let mut prompt = prompt_for(false);
        frame(&ctx, &mut prompt, Vec::new());
        let (_, output) = frame(&ctx, &mut prompt, Vec::new());
        let later = text_pos(&output, "LATER").expect("LATER is painted");
        frame(&ctx, &mut prompt, vec![Event::PointerMoved(later)]);
        let (_, hovered) = frame(&ctx, &mut prompt, vec![Event::PointerMoved(later)]);
        let fills = rect_fills(&hovered);
        assert!(fills.iter().any(|(r, f)| *f == button_states(false).0 && r.contains(later)), "LATER hovers in the prompt's grey: {fills:?}");
        assert!(!fills.iter().any(|(r, f)| *f == Color32::BLACK && r.contains(later)), "not in the host's black");
    }

    /// The strokes of the rectangles painted this frame, with each rectangle.
    fn rect_strokes(output: &FullOutput) -> Vec<(Rect, egui::Stroke)> {
        fn collect(shape: &egui::Shape, out: &mut Vec<(Rect, egui::Stroke)>) {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
                egui::Shape::Rect(rect) => out.push((rect.rect, rect.stroke)),
                _ => {}
            }
        }
        let mut strokes = Vec::new();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut strokes);
        }
        strokes
    }

    #[test]
    fn a_pressed_or_focused_button_shows_a_ring() {
        // Focus and a press both draw the active state; its 2 px ring must show on LATER (whose
        // border used to be fixed) and on the filled DOWNLOAD.
        for label in ["LATER", "DOWNLOAD"] {
            let ctx = dark_context();
            let mut prompt = prompt_for(false);
            frame(&ctx, &mut prompt, Vec::new());
            let (_, output) = frame(&ctx, &mut prompt, Vec::new());
            let at = text_pos(&output, label).expect("the button is painted");
            let down = Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE };
            frame(&ctx, &mut prompt, vec![Event::PointerMoved(at)]);
            let (_, pressed) = frame(&ctx, &mut prompt, vec![down]);
            let ring = rect_strokes(&pressed).iter().any(|(r, s)| r.contains(at) && s.width == 2.0 && s.color == Color32::from_rgb(245, 245, 248));
            assert!(ring, "{label} shows the focus ring while pressed");
        }
    }

    #[test]
    fn a_focused_whats_new_link_shows_a_ring() {
        let ctx = dark_context();
        let mut prompt = prompt_for(false);
        frame(&ctx, &mut prompt, Vec::new());
        let (_, output) = frame(&ctx, &mut prompt, Vec::new());
        let link = text_pos(&output, "What's new").expect("the link is painted");
        let tab = Event::Key { key: Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        let ringed = (0..8).any(|_| {
            let (_, out) = frame(&ctx, &mut prompt, vec![tab.clone()]);
            rect_strokes(&out).iter().any(|(r, s)| r.contains(link) && s.width == 2.0 && s.color == Color32::from_rgb(245, 245, 248))
        });
        assert!(ringed, "tabbing to the link shows the focus ring");
    }

    #[test]
    fn whats_new_lines_up_with_the_buttons() {
        let ctx = dark_context();
        let mut prompt = prompt_for(false);
        frame(&ctx, &mut prompt, Vec::new());
        let (_, output) = frame(&ctx, &mut prompt, Vec::new());
        let link = text_pos(&output, "What's new").expect("the link is painted");
        let button = text_pos(&output, "DOWNLOAD").expect("the button is painted");
        assert!((link.y - button.y).abs() <= 1.0, "link centred at {} but the button at {}", link.y, button.y);
    }

    #[test]
    fn clicking_download_answers_download() {
        let ctx = dark_context();
        let mut prompt = prompt_for(false);
        frame(&ctx, &mut prompt, Vec::new());
        let (_, output) = frame(&ctx, &mut prompt, Vec::new());
        let at = text_pos(&output, "DOWNLOAD").expect("DOWNLOAD is painted");
        let button = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        frame(&ctx, &mut prompt, vec![Event::PointerMoved(at)]);
        frame(&ctx, &mut prompt, vec![button(true)]);
        let (response, _) = frame(&ctx, &mut prompt, vec![button(false)]);
        assert_eq!(response, StartupPromptResponse::Download);
    }

    /// The colours of the line segments painted this frame.
    fn line_strokes(output: &FullOutput) -> Vec<Color32> {
        fn collect(shape: &egui::Shape, out: &mut Vec<Color32>) {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
                egui::Shape::LineSegment { stroke, .. } => out.push(stroke.color),
                _ => {}
            }
        }
        let mut colors = Vec::new();
        for clipped in &output.shapes {
            collect(&clipped.shape, &mut colors);
        }
        colors
    }

    #[test]
    fn every_craft_accent_reaches_aa_for_its_labels() {
        // The 13 apps' accents (as ArtCraft Launcher shows them, PdfCraft's new red) and the default.
        let accents = [
            0x2f7bf5, 0xe8573f, 0x8b5cf6, 0xf2a516, 0xb70228, 0xe0368f, 0x7bb51c, 0x02da96, 0x3f5fe0, 0x1fa5c9, 0xf07a1e, 0x3ea6e8, 0x1f9d55,
            0x3478f6,
        ];
        for hex in accents {
            let fill = Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8);
            let ratio = contrast(fill, ink_on(fill));
            assert!(ratio >= 4.5, "#{hex:06x}: {ratio:.2}:1");
        }
    }

    /// Where the frame painted `text` (inside its galley, so a pointer there is over it).
    fn text_pos(output: &FullOutput, text: &str) -> Option<egui::Pos2> {
        fn find(shape: &egui::Shape, text: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, text)),
                egui::Shape::Text(t) if t.galley.text() == text => Some(t.pos + t.galley.rect.center().to_vec2()),
                _ => None,
            }
        }
        output.shapes.iter().find_map(|c| find(&c.shape, text))
    }

    #[test]
    fn the_prompt_names_the_developer() {
        let ctx = dark_context();
        let mut prompt = prompt_for(false);
        frame(&ctx, &mut prompt, Vec::new());
        let (_, output) = frame(&ctx, &mut prompt, Vec::new());
        assert!(painted_text(&output).lines().any(|l| l == "by ArtCraft"), "every suite app is ArtCraft's by default");

        // An app can name its own developer.
        static OTHER: std::sync::OnceLock<App> = std::sync::OnceLock::new();
        let other: &'static App = OTHER.get_or_init(|| App { developer: "Someone Else", ..PHOTOCRAFT });
        let ctx = dark_context();
        let mut prompt = StartupPrompt { app: other, ..prompt_for(false) };
        frame(&ctx, &mut prompt, Vec::new());
        let (_, output) = frame(&ctx, &mut prompt, Vec::new());
        let painted = painted_text(&output);
        assert!(painted.lines().any(|l| l == "by Someone Else") && !painted.contains("by ArtCraft"), "{painted}");
    }

    #[test]
    fn creates_startup_prompt_dialog() {
        let release = Release { version: crate::version::parse("0.2.0"), tag: "v0.2.0".to_string(), ..Release::default() };
        let prompt = StartupPrompt::new(&PHOTOCRAFT, release, "0.1.0");
        assert_eq!(prompt.app.name, "PhotoCraft");
        assert_eq!(prompt.running_version, "0.1.0");
        assert!(prompt.open);
    }

    #[test]
    fn download_prompt_paints_card_actions_and_stays_pending() {
        let ctx = dark_context();
        let mut prompt = prompt_for(false);
        // A new Area first measures itself invisibly; inspect the following visible frame.
        let (sizing, _) = frame(&ctx, &mut prompt, Vec::new());
        assert_eq!(sizing, StartupPromptResponse::Pending);
        let (response, output) = frame(&ctx, &mut prompt, Vec::new());
        assert_eq!(response, StartupPromptResponse::Pending);
        assert!(prompt.open);
        let painted = painted_text(&output);
        for expected in [
            "PhotoCraft",
            "UPDATE",
            "v0.1.0 to 0.2.0",
            "PhotoCraft 0.2.0 is available with the latest features and bug fixes.",
            "What's new",
            "DOWNLOAD",
            "LATER",
        ] {
            assert!(painted.contains(expected), "missing {expected:?} in painted text {painted:?}");
        }
        assert!(!painted.contains("UPDATE AND RESTART"), "download branch must not offer the self-update button: {painted:?}");
        // No glyph that a font might lack: apps that panic on a missing glyph would take the prompt down (#343).
        assert!(painted.is_ascii(), "the prompt paints a non-ASCII character: {painted:?}");
        let fills = rect_fills(&output);
        let backdrop = fills.iter().any(|(rect, fill)| *fill == Color32::from_black_alpha(150) && rect.size() == screen().size());
        assert!(backdrop, "modal backdrop missing: {fills:?}");
        let card = fills.iter().any(|(_, fill)| *fill == DARK_CARD);
        assert!(card, "dialog card missing: {fills:?}");
    }

    #[test]
    fn self_updating_prompt_offers_update_and_restart() {
        let ctx = dark_context();
        let mut prompt = prompt_for(true);
        // A new Area first measures itself invisibly; inspect the following visible frame.
        let (sizing, _) = frame(&ctx, &mut prompt, Vec::new());
        assert_eq!(sizing, StartupPromptResponse::Pending);
        let (response, output) = frame(&ctx, &mut prompt, Vec::new());
        assert_eq!(response, StartupPromptResponse::Pending);
        assert!(prompt.open);
        let painted = painted_text(&output);
        assert!(painted.contains("UPDATE AND RESTART"), "self-update button missing: {painted:?}");
        assert!(painted.contains("LATER"), "dismiss button missing: {painted:?}");
        assert!(!painted.contains("DOWNLOAD"), "self-updating copy must not offer the manual download button: {painted:?}");
        let card = rect_fills(&output).iter().any(|(_, fill)| *fill == DARK_CARD);
        assert!(card, "dialog card missing");
    }

    #[test]
    fn installing_prompt_paints_status_and_dismiss() {
        let ctx = dark_context();
        let mut prompt = prompt_for(true);
        prompt.is_installing = true;
        prompt.status_message = Some("Couldn't reach GitHub".to_string());
        // A new Area first measures itself invisibly; inspect the following visible frame.
        let (sizing, _) = frame(&ctx, &mut prompt, Vec::new());
        assert_eq!(sizing, StartupPromptResponse::Pending);
        let (response, output) = frame(&ctx, &mut prompt, Vec::new());
        assert_eq!(response, StartupPromptResponse::Pending);
        assert!(prompt.open);
        let painted = painted_text(&output);
        for expected in ["PhotoCraft", "Downloading and installing update…", "Couldn't reach GitHub", "DISMISS", "What's new"] {
            assert!(painted.contains(expected), "missing {expected:?} in painted text {painted:?}");
        }
        for absent in ["LATER", "DOWNLOAD", "UPDATE AND RESTART"] {
            assert!(!painted.contains(absent), "unexpected {absent:?} while installing: {painted:?}");
        }
        let fills = rect_fills(&output);
        let alert = fills.iter().any(|(_, fill)| *fill == DARK_STATUS);
        assert!(alert, "status alert frame missing: {fills:?}");
    }

    #[test]
    fn escape_dismisses_open_prompt() {
        let ctx = dark_context();
        let mut prompt = prompt_for(false);
        let (first, _) = frame(&ctx, &mut prompt, Vec::new());
        assert_eq!(first, StartupPromptResponse::Pending);
        assert!(prompt.open);
        let escape = Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        let (dismissed, dismissed_output) = frame(&ctx, &mut prompt, vec![escape]);
        assert_eq!(dismissed, StartupPromptResponse::Later);
        assert!(!prompt.open);
        // The dismissing frame still paints the dialog; the closed state applies from the next frame on.
        assert!(!painted_text(&dismissed_output).is_empty(), "dismissing frame should have painted the dialog");
        let (later, closed_output) = frame(&ctx, &mut prompt, Vec::new());
        assert_eq!(later, StartupPromptResponse::Later);
        assert!(closed_output.shapes.is_empty(), "closed prompt painted {} shapes", closed_output.shapes.len());
    }

    #[test]
    fn closed_prompt_returns_later_without_painting() {
        let ctx = dark_context();
        let mut prompt = prompt_for(false);
        prompt.open = false;
        let (response, output) = frame(&ctx, &mut prompt, Vec::new());
        assert_eq!(response, StartupPromptResponse::Later);
        assert!(!prompt.open);
        assert!(output.shapes.is_empty(), "closed prompt painted {} shapes", output.shapes.len());
        assert!(painted_text(&output).is_empty());
    }

    #[test]
    fn check_result_paints_each_state_and_escape_closes() {
        let dialog = CheckResultDialog::new(&PHOTOCRAFT, "0.1.0");
        let run = |ctx: &Context, status: &Status, events: Vec<Event>| {
            // The first frame of an area only sizes it; the second paints.
            let mut frames = [Vec::new(), events].into_iter().map(|events| {
                let mut response = CheckResultResponse::None;
                let mut output = ctx.run_ui(RawInput { screen_rect: Some(screen()), events, ..RawInput::default() }, |ui| {
                    response = dialog.show(ui.ctx(), status);
                });
                output.textures_delta.clear();
                (response, output)
            });
            frames.next();
            frames.next().unwrap()
        };
        let ctx = dark_context();
        let (_, checking) = run(&ctx, &Status::Checking, Vec::new());
        assert!(painted_text(&checking).contains("Checking for updates…"));
        assert!(!painted_text(&checking).contains("UPDATE"), "no update pill on a result");
        let (_, up_to_date) = run(&ctx, &Status::UpToDate, Vec::new());
        let text = painted_text(&up_to_date);
        assert!(text.contains("PhotoCraft 0.1.0 is the newest version available.") && text.contains("OK"), "{text}");
        let (response, failed) = run(&ctx, &Status::Failed("no network".into()), Vec::new());
        let text = painted_text(&failed);
        for want in ["Could not check for updates.", "no network", "All releases are listed at https://github.com/", "CHECK AGAIN", "OK"] {
            assert!(text.contains(want), "{want} missing in {text}");
        }
        assert_eq!(response, CheckResultResponse::None);
        let (_, idle) = run(&ctx, &Status::Idle, Vec::new());
        assert!(painted_text(&idle).is_empty());

        let escape = Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        assert_eq!(run(&dark_context(), &Status::UpToDate, vec![escape]).0, CheckResultResponse::Close);
    }
}
