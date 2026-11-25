//! Renders the shared chrome components (ribbon, font box, status bar with zoom) in each Craft
//! app's accent, for design review: `cargo run -p craft-ui --example chrome_preview -- OUT_DIR`.
//!
//! Optional: `CRAFT_PREVIEW_FONTS=dir` with `Inter-Regular.ttf` (e.g. WordCraft's `assets/fonts`)
//! renders in Inter, as the apps do; otherwise egui's default font is used.
//!
//! The title bar, tabs, icons and document area here are a mock drawn in this file only; the
//! ribbon frame and groups, the font box, the toggles and the status bar are the real components.

use std::path::{Path, PathBuf};

use craft_ui::font_box::{FontBox, FontBoxLabels, FontBoxStyle};
use craft_ui::ribbon::{self, Ribbon, RibbonFrame, RibbonGroup, RibbonStyle};
use craft_ui::slider::SliderStyle;
use craft_ui::slider::{ZoomBar, ZoomBarLabels, ZoomBarStyle};
use craft_ui::status_bar::{self, StatusBar, StatusBarStyle};
use egui::{Align2, Color32, CornerRadius, FontId, Painter, Rect, Stroke, StrokeKind, pos2, vec2};

struct App {
    name: &'static str,
    accent: Color32,
    title: &'static str,
    tabs: &'static [&'static str],
    status: &'static [&'static str],
}

const APPS: &[App] = &[
    App {
        name: "wordcraft",
        accent: Color32::from_rgb(0x3B, 0x5B, 0xDB),
        title: "The Open Studio Handbook",
        tabs: &["File", "Home", "Insert", "Draw", "Design", "Layout", "References", "Review", "View"],
        status: &["Page 1 of 2", "361 words", "English (United States)"],
    },
    App {
        name: "gridcraft",
        accent: Color32::from_rgb(0x10, 0x7C, 0x41),
        title: "Quarterly Sales Dashboard",
        tabs: &["File", "Home", "Insert", "Draw", "Page Layout", "Formulas", "Data", "Review", "View"],
        status: &["Ready", "Average: 25 620", "Sum: 307 440"],
    },
    App {
        name: "deckcraft",
        accent: Color32::from_rgb(0xD9, 0x5A, 0x1A),
        title: "Northern Lights",
        tabs: &["File", "Home", "Insert", "Draw", "Design", "Transitions", "Animations", "Slide Show", "View"],
        status: &["Slide 1 of 8", "English (United States)"],
    },
    App {
        name: "pdfcraft",
        accent: Color32::from_rgb(0x1B, 0x63, 0xE0),
        title: "PrintCraft Showcase",
        tabs: &["All tools", "Read", "Edit", "Convert", "E-Sign"],
        status: &["Page 2 of 13", "A4 · 210 × 297 mm"],
    },
];

struct State {
    app: usize,
    frame: RibbonFrame,
    family: String,
    size: Option<f32>,
    zoom: f64,
    bold: bool,
    /// PdfCraft: the status bar under the document only, panels down to the window edge (the
    /// Adobe layout) instead of across the whole window (the Office layout).
    doc_bar: bool,
}

fn visuals(accent: Color32) -> egui::Visuals {
    let mut v = egui::Visuals::light();
    v.selection.bg_fill = accent;
    v.selection.stroke = Stroke::new(1.5, accent);
    v.panel_fill = Color32::from_rgb(0xF3, 0xF3, 0xF3);
    v.extreme_bg_color = Color32::WHITE;
    v.widgets.noninteractive.bg_stroke.color = Color32::from_rgb(0xDD, 0xDD, 0xDD);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0xEA, 0xEA, 0xEA);
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(0xE4, 0xE4, 0xE4);
    v.override_text_color = Some(Color32::from_rgb(0x24, 0x24, 0x24));
    v
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "chrome-preview".into()));
    std::fs::create_dir_all(&out)?;
    let fonts = std::env::var_os("CRAFT_PREVIEW_FONTS").map(PathBuf::from);
    for (i, app) in APPS.iter().enumerate() {
        for (tag, frame) in [("card", RibbonFrame::Card { margin: 6.0 }), ("flush", RibbonFrame::Flush)] {
            if app.name == "pdfcraft" && tag == "flush" {
                continue;
            }
            let state = State { app: i, frame, family: "Inter".into(), size: Some(11.0), zoom: 100.0, bold: true, doc_bar: false };
            render(&out.join(format!("{}-{tag}.png", app.name)), fonts.as_deref(), state)?;
            if app.name == "pdfcraft" {
                let state = State { app: i, frame, family: "Inter".into(), size: Some(11.0), zoom: 100.0, bold: true, doc_bar: true };
                render(&out.join("pdfcraft-docbar.png"), fonts.as_deref(), state)?;
            }
        }
    }
    states(&out.join("font-box-states.png"), fonts.as_deref())?;
    Ok(())
}

/// The three states side by side: a value, a mixed selection (blank) and no text to format
/// (dimmed, showing the theme font as a hint).
fn states(path: &Path, fonts: Option<&Path>) -> Result<(), Box<dyn std::error::Error>> {
    let draw = |ui: &mut egui::Ui, _: &mut ()| {
        let v = visuals(APPS[2].accent);
        ui.ctx().set_visuals(v.clone());
        ui.painter().rect_filled(ui.max_rect(), CornerRadius::ZERO, v.extreme_bg_color);
        let fs = FontBoxStyle::from_visuals(&v);
        let labels =
            FontBoxLabels { font: "Font", size: "Font Size", grow: "Increase Font Size", shrink: "Decrease Font Size", clear: "Clear Formatting" };
        let families: Vec<String> = vec!["Inter".into(), "Georgia".into()];
        ui.add_space(12.0);
        for (caption, family, size, enabled) in [
            ("Text selected", "Georgia", Some(18.0), true),
            ("Mixed fonts in the selection", "", None, true),
            ("No text to format (a picture, or nothing selected)", "", None, false),
        ] {
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                let (mut f, mut z) = (family.to_string(), size);
                FontBox::new(&mut f, &mut z, &families, &fs, labels).clear(true).enabled(enabled).hint("Inter", 11.0).show(ui);
                ui.add_space(16.0);
                ui.label(egui::RichText::new(caption).size(12.0).color(v.weak_text_color()));
            });
            ui.add_space(10.0);
        }
    };
    let mut harness = egui_kittest::Harness::builder().with_size(vec2(640.0, 130.0)).with_pixels_per_point(2.0).wgpu().build_ui_state(draw, ());
    if let Some(dir) = fonts
        && let Ok(bytes) = std::fs::read(dir.join("Inter-Regular.ttf"))
    {
        let mut defs = egui::FontDefinitions::default();
        defs.font_data.insert("inter".into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
        if let Some(f) = defs.families.get_mut(&egui::FontFamily::Proportional) {
            f.insert(0, "inter".into());
        }
        harness.ctx.set_fonts(defs);
    }
    harness.run_steps(4);
    harness.render().map_err(|e| e.to_string())?.save(path)?;
    println!("{}", path.display());
    Ok(())
}

fn render(path: &Path, fonts: Option<&Path>, state: State) -> Result<(), Box<dyn std::error::Error>> {
    let mut harness = egui_kittest::Harness::builder().with_size(vec2(1180.0, 420.0)).with_pixels_per_point(2.0).wgpu().build_ui_state(draw, state);
    if let Some(dir) = fonts
        && let Ok(bytes) = std::fs::read(dir.join("Inter-Regular.ttf"))
    {
        let mut defs = egui::FontDefinitions::default();
        defs.font_data.insert("inter".into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
        if let Some(f) = defs.families.get_mut(&egui::FontFamily::Proportional) {
            f.insert(0, "inter".into());
        }
        harness.ctx.set_fonts(defs);
    }
    harness.run_steps(4);
    harness.render().map_err(|e| e.to_string())?.save(path)?;
    println!("{}", path.display());
    Ok(())
}

fn draw(ui: &mut egui::Ui, s: &mut State) {
    let app = &APPS[s.app];
    let v = visuals(app.accent);
    ui.ctx().set_visuals(v.clone());
    let full = ui.max_rect();
    ui.painter().rect_filled(full, CornerRadius::ZERO, v.panel_fill);
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    title_bar(ui, app, &v);
    tabs(ui, app, &v);
    let rs = RibbonStyle::from_visuals(&v);
    let fs = FontBoxStyle::from_visuals(&v);
    let labels =
        FontBoxLabels { font: "Font", size: "Font Size", grow: "Increase Font Size", shrink: "Decrease Font Size", clear: "Clear Formatting" };
    let families: Vec<String> = ["Inter", "Georgia", "Helvetica", "Menlo"].iter().map(|f| f.to_string()).collect();
    let pdf = app.name == "pdfcraft";
    if pdf {
        // PdfCraft has no ribbon: the same font box lives in its Edit text panel, a full-height
        // side panel like Acrobat's, with the panel rail on the right.
        let top = ui.cursor().min.y;
        let bottom = if s.doc_bar { full.bottom() } else { full.bottom() - 26.0 };
        let panel = Rect::from_min_max(pos2(full.left(), top), pos2(full.left() + 300.0, bottom));
        let rail = Rect::from_min_max(pos2(full.right() - 44.0, top), pos2(full.right(), bottom));
        for (r, edge) in [(panel, panel.right() - 0.5), (rail, rail.left() + 0.5)] {
            ui.painter().rect_filled(r, CornerRadius::ZERO, v.extreme_bg_color);
            ui.painter().vline(edge, r.y_range(), Stroke::new(1.0, rs.border));
        }
        ui.painter().hline(full.x_range(), top + 0.5, Stroke::new(1.0, rs.border));
        for i in 0..5 {
            let c = pos2(rail.center().x, rail.top() + 24.0 + i as f32 * 34.0);
            ui.painter().rect_stroke(
                Rect::from_center_size(c, vec2(16.0, 16.0)),
                CornerRadius::same(3),
                Stroke::new(1.2, Color32::from_gray(0x60)),
                StrokeKind::Inside,
            );
        }
        ui.scope_builder(egui::UiBuilder::new().max_rect(panel.shrink(14.0)), |ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 8.0);
            ui.label(egui::RichText::new("FORMAT TEXT").size(11.0).color(rs.caption));
            FontBox::new(&mut s.family, &mut s.size, &families, &fs, labels).widths(150.0, 56.0).show(ui);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                format_toggles(ui, &rs, s);
            });
        });
        let doc = Rect::from_min_max(pos2(panel.right(), top), pos2(rail.left(), bottom));
        let page_area = Rect::from_min_max(doc.min + vec2(16.0, 8.0), pos2(doc.right() - 16.0, doc.bottom() - if s.doc_bar { 34.0 } else { 8.0 }));
        page(ui.painter(), page_area);
        if s.doc_bar {
            let ss = StatusBarStyle::from_visuals(&v);
            let zs = ZoomBarStyle { slider: SliderStyle::from_visuals(&v), icon: ss.icon, hover: ss.hover, text: ss.text, font: ss.font };
            let zl = ZoomBarLabels { zoom: "Zoom", zoom_in: "Zoom In", zoom_out: "Zoom Out" };
            let bar = Rect::from_min_max(pos2(doc.left(), doc.bottom() - ss.height), doc.max);
            ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
                StatusBar::new(&ss).show(
                    ui,
                    |ui| {
                        for t in app.status {
                            status_bar::item(ui, &ss, t);
                        }
                    },
                    |ui| {
                        ZoomBar::new(&mut s.zoom, 10.0..=500.0, zl, &zs).slider_width(90.0).show(ui);
                    },
                );
            });
            return;
        }
        ui.allocate_space(vec2(ui.available_width(), (full.bottom() - 26.0 - ui.cursor().min.y).max(0.0)));
    } else {
        // Both systems' defaults side by side, whatever system renders the preview.
        Ribbon::new(&rs).frame(s.frame).captions(matches!(s.frame, RibbonFrame::Card { .. })).show(ui, |ui| {
            RibbonGroup::new("Clipboard", &rs).launcher("Clipboard").show(ui, |ui| {
                ribbon::large_button(ui, &rs, "Paste", paste_icon);
                ui.vertical(|ui| {
                    ribbon::small_button(ui, &rs, "Cut", cut_icon);
                    ribbon::small_button(ui, &rs, "Copy", copy_icon);
                });
            });
            RibbonGroup::new("Font", &rs).launcher("Font Settings").show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    FontBox::new(&mut s.family, &mut s.size, &families, &fs, labels).clear(true).show(ui);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        format_toggles(ui, &rs, s);
                    });
                });
            });
            RibbonGroup::new("Paragraph", &rs).launcher("Paragraph Settings").last().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (i, label) in ["Align Left", "Center", "Align Right", "Justify"].iter().enumerate() {
                        ribbon::toggle(ui, &rs, i == 0, label, |p, r, c| align_icon(p, r, c, i));
                    }
                });
            });
        });
        let doc = Rect::from_min_max(pos2(full.left() + 8.0, ui.cursor().min.y + 8.0), pos2(full.right() - 8.0, full.bottom() - 34.0));
        page(ui.painter(), doc);
        ui.allocate_space(vec2(ui.available_width(), (full.bottom() - 26.0 - ui.cursor().min.y).max(0.0)));
    }
    let ss = StatusBarStyle::from_visuals(&v);
    let zs = ZoomBarStyle { slider: SliderStyle::from_visuals(&v), icon: ss.icon, hover: ss.hover, text: ss.text, font: ss.font };
    let zl = ZoomBarLabels { zoom: "Zoom", zoom_in: "Zoom In", zoom_out: "Zoom Out" };
    StatusBar::new(&ss).show(
        ui,
        |ui| {
            for t in app.status {
                status_bar::item(ui, &ss, t);
            }
        },
        |ui| {
            ZoomBar::new(&mut s.zoom, 10.0..=500.0, zl, &zs).show(ui);
            ui.add_space(6.0);
            for (i, l) in ["Web Layout", "Print Layout", "Read Mode"].iter().enumerate().rev() {
                status_bar::view_button(ui, &ss, i == 1, l, |p, r, c| view_icon(p, r, c, i));
            }
        },
    );
}

fn format_toggles(ui: &mut egui::Ui, rs: &RibbonStyle, s: &mut State) {
    if ribbon::toggle(ui, rs, s.bold, "Bold", |p, r, c| letter(p, r, c, "B", true, false)).clicked() {
        s.bold = !s.bold;
    }
    ribbon::toggle(ui, rs, false, "Italic", |p, r, c| letter(p, r, c, "I", false, false));
    ribbon::toggle(ui, rs, false, "Underline", |p, r, c| letter(p, r, c, "U", false, true));
    ribbon::toggle(ui, rs, false, "Strikethrough", |p, r, c| {
        letter(p, r, c, "S", false, false);
        p.hline(r.x_range(), r.center().y, Stroke::new(1.2, c));
    });
    ribbon::toggle(ui, rs, false, "Font Color", |p, r, c| {
        letter(p, Rect::from_min_max(r.min, pos2(r.right(), r.bottom() - 3.0)), c, "A", false, false);
        p.rect_filled(
            Rect::from_min_max(pos2(r.left() + 1.0, r.bottom() - 3.0), pos2(r.right() - 1.0, r.bottom())),
            CornerRadius::ZERO,
            Color32::from_rgb(0xD1, 0x34, 0x38),
        );
    });
}

/// Mock title bar: 40 tall with the macOS window controls, the quick access icons and the
/// title on one centre line (the height and alignment proposed for every app).
fn title_bar(ui: &mut egui::Ui, app: &App, v: &egui::Visuals) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::ZERO, v.panel_fill);
    let cy = rect.center().y;
    for (i, c) in
        [Color32::from_rgb(0xFF, 0x5F, 0x57), Color32::from_rgb(0xFE, 0xBC, 0x2E), Color32::from_rgb(0x28, 0xC8, 0x40)].into_iter().enumerate()
    {
        p.circle_filled(pos2(rect.left() + 18.0 + i as f32 * 20.0, cy), 6.0, c);
    }
    for i in 0..4 {
        let r = Rect::from_center_size(pos2(rect.left() + 92.0 + i as f32 * 26.0, cy), vec2(14.0, 14.0));
        p.rect_stroke(r, CornerRadius::same(3), Stroke::new(1.2, Color32::from_gray(0x60)), StrokeKind::Inside);
    }
    p.text(pos2(rect.center().x, cy), Align2::CENTER_CENTER, app.title, FontId::proportional(13.0), Color32::from_gray(0x24));
    let search = Rect::from_center_size(pos2(rect.right() - 150.0, cy), vec2(240.0, 26.0));
    p.rect(search, CornerRadius::same(13), Color32::WHITE, Stroke::new(1.0, Color32::from_gray(0xDD)), StrokeKind::Inside);
    p.text(pos2(search.left() + 14.0, cy), Align2::LEFT_CENTER, "Search", FontId::proportional(12.0), Color32::from_gray(0x80));
}

fn tabs(ui: &mut egui::Ui, app: &App, v: &egui::Visuals) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), egui::Sense::hover());
    let p = ui.painter();
    let mut x = rect.left() + 12.0;
    for (i, t) in app.tabs.iter().enumerate() {
        let selected = i == if app.name == "pdfcraft" { 0 } else { 1 };
        let g = p.layout_no_wrap(t.to_string(), FontId::proportional(13.0), if selected { app.accent } else { Color32::from_gray(0x30) });
        let w = g.size().x;
        p.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, Color32::BLACK);
        if selected {
            p.rect_filled(Rect::from_min_size(pos2(x, rect.bottom() - 4.0), vec2(w, 3.0)), CornerRadius::same(1), app.accent);
        }
        x += w + 22.0;
    }
    let _ = v;
}

fn page(p: &Painter, area: Rect) {
    let w = (area.height() * 0.77).min(area.width() - 40.0);
    let r = Rect::from_min_size(pos2(area.center().x - w / 2.0, area.top() + 8.0), vec2(w, area.height() + 40.0));
    p.with_clip_rect(area).rect(r, CornerRadius::ZERO, Color32::WHITE, Stroke::new(1.0, Color32::from_gray(0xDD)), StrokeKind::Inside);
    for i in 0..5 {
        let y = r.top() + 30.0 + i as f32 * 16.0;
        if y < area.bottom() - 6.0 {
            p.hline((r.left() + 30.0)..=(r.right() - 30.0 - (i % 2) as f32 * 40.0), y, Stroke::new(5.0, Color32::from_gray(0xE6)));
        }
    }
}

fn letter(p: &Painter, r: Rect, c: Color32, s: &str, bold: bool, underline: bool) {
    let font = FontId::proportional(14.0);
    p.text(r.center(), Align2::CENTER_CENTER, s, font.clone(), c);
    if bold {
        p.text(r.center() + vec2(0.6, 0.0), Align2::CENTER_CENTER, s, font, c);
    }
    if underline {
        p.hline((r.center().x - 4.0)..=(r.center().x + 4.0), r.bottom() - 1.0, Stroke::new(1.2, c));
    }
}

fn paste_icon(p: &Painter, r: Rect, c: Color32) {
    let board = Rect::from_min_max(pos2(r.left() + 3.0, r.top() + 3.0), pos2(r.right() - 6.0, r.bottom() - 1.0));
    p.rect_stroke(board, CornerRadius::same(3), Stroke::new(1.5, c), StrokeKind::Inside);
    p.rect_filled(Rect::from_center_size(pos2(board.center().x, board.top()), vec2(10.0, 5.0)), CornerRadius::same(2), c);
    let sheet = Rect::from_min_max(pos2(r.center().x, r.center().y - 2.0), pos2(r.right(), r.bottom()));
    p.rect(sheet, CornerRadius::same(2), Color32::WHITE, Stroke::new(1.5, Color32::from_rgb(0x2E, 0x6F, 0xD8)), StrokeKind::Inside);
}

fn cut_icon(p: &Painter, r: Rect, c: Color32) {
    let k = Stroke::new(1.3, c);
    p.circle_stroke(pos2(r.left() + 4.0, r.bottom() - 3.5), 2.8, k);
    p.circle_stroke(pos2(r.right() - 4.0, r.bottom() - 3.5), 2.8, k);
    p.line_segment([pos2(r.left() + 5.5, r.bottom() - 6.0), pos2(r.right() - 3.0, r.top())], k);
    p.line_segment([pos2(r.right() - 5.5, r.bottom() - 6.0), pos2(r.left() + 3.0, r.top())], k);
}

fn copy_icon(p: &Painter, r: Rect, c: Color32) {
    let k = Stroke::new(1.3, c);
    p.rect_stroke(Rect::from_min_max(r.min, r.max - vec2(5.0, 5.0)), CornerRadius::same(2), k, StrokeKind::Inside);
    p.rect(Rect::from_min_max(r.min + vec2(5.0, 5.0), r.max), CornerRadius::same(2), Color32::WHITE, k, StrokeKind::Inside);
}

fn align_icon(p: &Painter, r: Rect, c: Color32, kind: usize) {
    let k = Stroke::new(1.3, c);
    for (i, w) in [1.0, 0.65, 1.0, 0.65].iter().enumerate() {
        let y = r.top() + 2.0 + i as f32 * 4.0;
        let full = r.width();
        let len = if kind == 3 { full } else { full * w };
        let x0 = match kind {
            1 => r.center().x - len / 2.0,
            2 => r.right() - len,
            _ => r.left(),
        };
        p.hline(x0..=(x0 + len), y, k);
    }
}

fn view_icon(p: &Painter, r: Rect, c: Color32, kind: usize) {
    let k = Stroke::new(1.2, c);
    match kind {
        0 => {
            p.circle_stroke(r.center(), 6.0, k);
            p.hline((r.left() + 1.0)..=(r.right() - 1.0), r.center().y, k);
        }
        1 => {
            p.rect_stroke(r.shrink2(vec2(2.5, 0.5)), CornerRadius::same(1), k, StrokeKind::Inside);
            for i in 0..3 {
                p.hline((r.left() + 5.0)..=(r.right() - 5.0), r.top() + 4.0 + i as f32 * 3.0, k);
            }
        }
        _ => {
            p.rect_stroke(
                Rect::from_min_max(r.min + vec2(0.0, 2.0), pos2(r.center().x - 0.5, r.bottom() - 2.0)),
                CornerRadius::same(1),
                k,
                StrokeKind::Inside,
            );
            p.rect_stroke(
                Rect::from_min_max(pos2(r.center().x + 0.5, r.top() + 2.0), r.max - vec2(0.0, 2.0)),
                CornerRadius::same(1),
                k,
                StrokeKind::Inside,
            );
        }
    }
}
