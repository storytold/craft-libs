//! Renders the places panel in its three hosts, without and with remote storage, in light and
//! dark, for design review: `cargo run -p craft-ui --example places_preview -- OUT_DIR`.
//!
//! Optional: `CRAFT_PREVIEW_FONTS=dir` with `Inter-Regular.ttf` (e.g. WordCraft's `assets/fonts`)
//! renders in Inter, as the apps do; otherwise egui's default font is used.
//!
//! The menu bar, the New and Open buttons, the other menu items, the icons of the File column and
//! the thumbnail pictures are a mock drawn in this file only; the panel, the places in the menu and
//! the File page's navigation column are the real components.

use std::path::{Path, PathBuf};

use craft_ui::file_nav::{FileNav, FileNavItem, FileNavStyle};
use craft_ui::places::{self, Place, PlaceEntry, PlaceKind, PlacesContent, PlacesLabels, PlacesNotice, PlacesPanel, PlacesStyle, PlacesView};
use egui::{Align2, Color32, CornerRadius, FontId, Painter, Rect, Stroke, UiBuilder, pos2, vec2};

const ACCENT: Color32 = Color32::from_rgb(0x3B, 0x5B, 0xDB);
const LABELS: PlacesLabels<'static> = PlacesLabels { places: "Places", add_place: "Add a place", browse: "Browse…", loading: "Loading…" };

const LOCAL: &[Place<'static>] = &[
    Place { kind: PlaceKind::Recent, label: "Recent", detail: None },
    Place { kind: PlaceKind::Synced, label: "OneDrive", detail: Some("Contoso") },
    Place { kind: PlaceKind::Local, label: "This computer", detail: None },
];
const WITH_ACCOUNTS: &[Place<'static>] = &[
    Place { kind: PlaceKind::Recent, label: "Recent", detail: None },
    Place { kind: PlaceKind::Account, label: "Work", detail: Some("you@company.example") },
    Place { kind: PlaceKind::Account, label: "Personal", detail: Some("you@home.example") },
    Place { kind: PlaceKind::Local, label: "This computer", detail: None },
];

const fn file(name: &'static str, detail: &'static str, remote: bool) -> PlaceEntry<'static> {
    PlaceEntry { name, folder: false, detail, attention: false, remote }
}

const RECENT_DOCUMENTS: &[PlaceEntry<'static>] = &[
    file("Q4 proposal.docx", "Documents · yesterday", false),
    file("Framework agreement.docx", "Desktop · Oct 3", false),
    file("Meeting notes.docx", "Documents · Sep 28", false),
    file("Price list.docx", "Downloads · Sep 21", false),
];
const WORK_DOCUMENTS: &[PlaceEntry<'static>] = &[
    PlaceEntry { name: "2026", folder: true, detail: "folder", attention: false, remote: true },
    file("Q4 proposal.docx", "yesterday", true),
    PlaceEntry { name: "Meeting notes.docx", folder: false, detail: "Anna is editing", attention: true, remote: true },
    file("Price list for the autumn and winter season, final version 3.docx", "shared by Tom · view only", true),
];
const LOCAL_PICTURES: &[PlaceEntry<'static>] = &[
    file("Poster A2.pcd", "Pictures · today", false),
    file("Banner.pcd", "Desktop · yesterday", false),
    file("Portrait retouch.pcd", "Pictures · Oct 3", false),
    file("Logo draft.pcd", "Documents · Sep 28", false),
];
const MIXED_PICTURES: &[PlaceEntry<'static>] = &[
    file("Poster A2.pcd", "Work · today", true),
    file("Banner.pcd", "Desktop · yesterday", false),
    file("Portrait retouch.pcd", "Personal · Oct 3", true),
    file("Logo draft.pcd", "Documents · Sep 28", false),
];

#[derive(Clone, Copy, PartialEq)]
enum Host {
    /// File › Open in an Office-style app.
    Office,
    /// The home screen of a graphics app.
    Home,
    /// A classic File menu.
    Menu,
    /// The panel alone, at a folder the system will not let the app read.
    Denied,
    /// The panel alone, with nothing to list.
    Empty,
}

#[derive(Clone, Copy)]
struct Scene {
    host: Host,
    dark: bool,
    accounts: bool,
}

fn visuals(dark: bool) -> egui::Visuals {
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = Stroke::new(1.5, ACCENT);
    if !dark {
        v.panel_fill = Color32::from_rgb(0xF3, 0xF3, 0xF3);
        v.extreme_bg_color = Color32::WHITE;
        v.widgets.noninteractive.bg_stroke.color = Color32::from_rgb(0xDD, 0xDD, 0xDD);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0xEA, 0xEA, 0xEA);
        v.override_text_color = Some(Color32::from_rgb(0x24, 0x24, 0x24));
    }
    v
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "places-preview".into()));
    std::fs::create_dir_all(&out)?;
    let fonts = std::env::var_os("CRAFT_PREVIEW_FONTS").map(PathBuf::from);
    for (host, name) in [(Host::Office, "office"), (Host::Home, "home"), (Host::Menu, "menu")] {
        for (dark, theme) in [(false, "light"), (true, "dark")] {
            for (accounts, storage) in [(false, "local"), (true, "accounts")] {
                render(&out.join(format!("{name}-{storage}-{theme}.png")), fonts.as_deref(), Scene { host, dark, accounts })?;
            }
        }
    }
    for (host, name) in [(Host::Denied, "denied"), (Host::Empty, "empty")] {
        for (dark, theme) in [(false, "light"), (true, "dark")] {
            render(&out.join(format!("{name}-{theme}.png")), fonts.as_deref(), Scene { host, dark, accounts: true })?;
        }
    }
    Ok(())
}

fn render(path: &Path, fonts: Option<&Path>, scene: Scene) -> Result<(), Box<dyn std::error::Error>> {
    let mut harness = egui_kittest::Harness::builder().with_size(vec2(860.0, 400.0)).with_pixels_per_point(2.0).wgpu().build_ui_state(draw, scene);
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

fn draw(ui: &mut egui::Ui, scene: &mut Scene) {
    let v = visuals(scene.dark);
    ui.ctx().set_visuals(v.clone());
    let style = PlacesStyle::from_visuals(&v);
    let full = ui.max_rect();
    ui.painter().rect_filled(full, CornerRadius::ZERO, v.panel_fill);
    let places = if scene.accounts { WITH_ACCOUNTS } else { LOCAL };
    match scene.host {
        Host::Office => {
            let nav_style = FileNavStyle {
                width: 160.0,
                row_height: 26.0,
                foot_row_height: 22.0,
                font: FontId::proportional(13.0),
                selected_font: FontId::proportional(13.0),
                foot_font: FontId::proportional(12.0),
                ..FileNavStyle::on_accent(ACCENT)
            };
            let nav = Rect::from_min_size(full.min, vec2(nav_style.width, full.height()));
            let item = |key, label| FileNavItem { key, label };
            let top = [item("home", "Home"), item("new", "New"), item("open", "Open")];
            let mut rest = vec![item("info", "Info"), item("save", "Save"), item("saveAs", "Save as")];
            let mut foot = vec![item("options", "Options")];
            if scene.accounts {
                rest.push(item("share", "Share"));
                foot.push(item("accounts", "Accounts"));
            }
            rest.extend([item("print", "Print"), item("export", "Export")]);
            foot.push(item("about", "About"));
            ui.scope_builder(UiBuilder::new().max_rect(nav), |ui| {
                FileNav::new("Back", &[&top, &rest], Some("open"), &nav_style, &glyph).foot(&foot).show(ui);
            });
            let (selected, crumbs, entries): (usize, &[&str], _) =
                if scene.accounts { (1, &["Work", "Documents", "Proposals"], WORK_DOCUMENTS) } else { (0, &["Recent"], RECENT_DOCUMENTS) };
            ui.scope_builder(UiBuilder::new().max_rect(Rect::from_min_max(nav.right_top(), full.max)), |ui| {
                PlacesPanel::new(places, Some(selected), PlacesContent::Entries(entries), &style, LABELS).crumbs(crumbs).add_place(true).show(ui);
            });
        }
        Host::Home => {
            let bar = menu_bar(ui, full, &v, &["File", "Edit", "Image", "Layer", "Select", "Filter", "View", "Window", "Help"], None);
            let entries = if scene.accounts { MIXED_PICTURES } else { LOCAL_PICTURES };
            ui.scope_builder(UiBuilder::new().max_rect(Rect::from_min_max(bar.left_bottom(), full.max)), |ui| {
                PlacesPanel::new(places, Some(0), PlacesContent::Entries(entries), &style, LABELS)
                    .crumbs(&["Recent"])
                    .view(PlacesView::Thumbnails)
                    .thumbnails(&picture)
                    .add_place(true)
                    .browse(false)
                    .header(|ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            let width = (ui.available_width() - 6.0) / 2.0;
                            let new = egui::Button::new(egui::RichText::new("New file").color(Color32::WHITE)).fill(ACCENT);
                            ui.add_sized([width, 28.0], new);
                            ui.add_sized([width, 28.0], egui::Button::new("Open…"));
                        });
                    })
                    .show(ui);
            });
        }
        Host::Menu => {
            let bar = menu_bar(ui, full, &v, &["File", "Edit", "Clip", "Sequence", "View", "Window", "Help"], Some(0));
            let anchor = ui.interact(
                Rect::from_min_size(bar.left_top() + vec2(6.0, 0.0), vec2(40.0, bar.height())),
                ui.id().with("file"),
                egui::Sense::hover(),
            );
            egui::Popup::menu(&anchor).open(true).show(|ui| {
                ui.set_min_width(250.0);
                let _ = ui.add(egui::Button::new("New…").shortcut_text("⌘N"));
                let _ = ui.add(egui::Button::new("Open…").shortcut_text("⌘O"));
                ui.separator();
                ui.label(egui::RichText::new("Open from").small().weak());
                places::menu(ui, places, LABELS, true);
                ui.separator();
                let _ = ui.add(egui::Button::new("Save").shortcut_text("⌘S"));
                let _ = ui.add(egui::Button::new("Save as…").shortcut_text("⇧⌘S"));
            });
        }
        Host::Denied | Host::Empty => {
            let what = if scene.host == Host::Denied {
                PlacesNotice {
                    icon: places::PlacesIcon::Locked,
                    title: "No access to this folder",
                    body: "The system has not allowed the app to read it. Browse opens a document from it; to list it here, allow the app in System Settings.",
                    action: Some("Browse…"),
                }
            } else {
                PlacesNotice { icon: places::PlacesIcon::Folder, title: "This folder is empty", body: "", action: None }
            };
            let crumbs: &[&str] = if scene.host == Host::Denied { &["This computer", "Users", "ada", "Documents"] } else { &["Personal", "Drafts"] };
            let selected = if scene.host == Host::Denied { 3 } else { 2 };
            PlacesPanel::new(places, Some(selected), PlacesContent::Notice(what), &style, LABELS).crumbs(crumbs).add_place(true).show(ui);
        }
    }
}

/// Stand-in icons for the File column; a real app paints its own set here.
fn glyph(key: Option<&str>, p: &Painter, r: Rect, c: Color32) {
    let stroke = Stroke::new(1.4, c);
    let at = |x: f32, y: f32| pos2(r.left() + x * r.width(), r.top() + y * r.height());
    let line = |points: &[(f32, f32)]| egui::Shape::line(points.iter().map(|&(x, y)| at(x, y)).collect(), stroke);
    match key {
        None => drop(p.add(line(&[(0.62, 0.2), (0.34, 0.5), (0.62, 0.8)]))),
        Some("home") => drop(p.add(line(&[(0.15, 0.5), (0.5, 0.16), (0.85, 0.5), (0.85, 0.86), (0.15, 0.86), (0.15, 0.5)]))),
        Some("new") => places::paint_icon(places::PlacesIcon::File, p, r, c),
        Some("open") => places::paint_icon(places::PlacesIcon::Folder, p, r, c),
        Some("save" | "saveAs") => {
            p.add(line(&[(0.16, 0.16), (0.7, 0.16), (0.86, 0.32), (0.86, 0.86), (0.16, 0.86), (0.16, 0.16)]));
            p.add(line(&[(0.32, 0.86), (0.32, 0.58), (0.7, 0.58), (0.7, 0.86)]));
        }
        Some("print") => {
            p.add(line(&[(0.3, 0.38), (0.3, 0.14), (0.7, 0.14), (0.7, 0.38)]));
            p.add(line(&[(0.3, 0.7), (0.14, 0.7), (0.14, 0.38), (0.86, 0.38), (0.86, 0.7), (0.7, 0.7)]));
            p.add(line(&[(0.3, 0.58), (0.7, 0.58), (0.7, 0.88), (0.3, 0.88), (0.3, 0.58)]));
        }
        Some("export" | "share") => {
            p.add(line(&[(0.5, 0.64), (0.5, 0.14)]));
            p.add(line(&[(0.32, 0.32), (0.5, 0.14), (0.68, 0.32)]));
            p.add(line(&[(0.18, 0.56), (0.18, 0.86), (0.82, 0.86), (0.82, 0.56)]));
        }
        Some("options") => {
            p.circle_stroke(r.center(), r.width() * 0.36, stroke);
            p.circle_stroke(r.center(), r.width() * 0.12, stroke);
        }
        Some(_) => {
            p.circle_stroke(r.center(), r.width() * 0.36, stroke);
            p.add(line(&[(0.5, 0.46), (0.5, 0.7)]));
            p.circle_filled(at(0.5, 0.32), 1.0, c);
        }
    }
}

/// A menu bar across the top of `full`; returns its rect.
fn menu_bar(ui: &egui::Ui, full: Rect, v: &egui::Visuals, items: &[&str], open: Option<usize>) -> Rect {
    let bar = Rect::from_min_size(full.min, vec2(full.width(), 26.0));
    let p = ui.painter();
    p.rect_filled(bar, CornerRadius::ZERO, v.panel_fill);
    p.hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, v.widgets.noninteractive.bg_stroke.color));
    let mut x = bar.left() + 12.0;
    for (i, item) in items.iter().enumerate() {
        let galley = p.layout_no_wrap((*item).to_owned(), FontId::proportional(12.5), v.text_color());
        let width = galley.size().x;
        let colour = if open == Some(i) {
            p.rect_filled(Rect::from_min_size(pos2(x - 6.0, bar.top() + 3.0), vec2(width + 12.0, 20.0)), CornerRadius::same(4), ACCENT);
            Color32::WHITE
        } else {
            v.text_color()
        };
        p.text(pos2(x, bar.center().y), Align2::LEFT_CENTER, *item, FontId::proportional(12.5), colour);
        x += width + 16.0;
    }
    bar
}

/// A stand-in picture per entry; a real app paints the document's thumbnail here.
fn picture(index: usize, p: &Painter, rect: Rect) {
    const TINTS: [Color32; 4] = [
        Color32::from_rgb(0xDE, 0xDF, 0xD9),
        Color32::from_rgb(0xCD, 0xD6, 0xE8),
        Color32::from_rgb(0xE3, 0xD5, 0xC5),
        Color32::from_rgb(0xCF, 0xE0, 0xD4),
    ];
    p.rect_filled(rect, CornerRadius::same(6), TINTS[index % TINTS.len()]);
}
