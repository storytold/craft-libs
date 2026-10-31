//! The details sheet: a family's every style in its own face, its facts and languages, and
//! installing or removing it whole or a style at a time.

use std::sync::Arc;

use craft_ui::direction;
use egui::{
    Align, Color32, Context, CornerRadius, FontFamily, FontId, Id, Layout, Margin, Order, Rect, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder,
    vec2,
};

use crate::index::{Family, FontFile, Index, script_name, source_label};
use crate::panel::app::{Detail, FontsPanel, Pending, category_label, family_key, regular, source_name, style_count, style_name};
use crate::panel::faces::Owner;
use crate::panel::i18n::{self, fmt, tl, tn};
use crate::panel::jobs::Kind;
use crate::panel::samples;
use crate::panel::widgets::{self, Glyph, Skin};
use crate::{FaceInfo, Source};

/// From this width the sheet reads in two columns.
const COLUMNS_FROM: f32 = 760.0;

/// What a style row offers at its end.
enum RowAction {
    None,
    Install(String),
    Remove(Box<FaceInfo>),
    Running(Option<f32>),
}

struct StyleRow {
    label: String,
    face: Option<(Owner, FaceInfo)>,
    action: RowAction,
}

fn weight_name(weight: u16) -> &'static str {
    match weight {
        0..=149 => tl!("Thin"),
        150..=249 => tl!("ExtraLight"),
        250..=349 => tl!("Light"),
        350..=449 => tl!("Regular"),
        450..=549 => tl!("Medium"),
        550..=649 => tl!("SemiBold"),
        650..=749 => tl!("Bold"),
        750..=849 => tl!("ExtraBold"),
        _ => tl!("Black"),
    }
}

/// "Bold Italic", or "Variable 100–900" for a variable file.
fn file_label(file: &FontFile) -> String {
    let base = match file.axes.iter().find(|a| a.tag == "wght") {
        Some(a) => fmt("Variable {from}–{to}", &[("from", &(a.min.round() as u32).to_string()), ("to", &(a.max.round() as u32).to_string())]),
        None => weight_name(file.weight).to_owned(),
    };
    match (file.is_italic(), file.weight) {
        (true, 350..=449) if file.axes.is_empty() => tl!("Italic").to_owned(),
        (true, _) => format!("{base} {}", tl!("Italic")),
        (false, _) => base,
    }
}

fn licence_name(id: &str) -> &str {
    match id {
        "OFL" => tl!("SIL Open Font License"),
        "APACHE2" => tl!("Apache License 2.0"),
        "UFL" => tl!("Ubuntu Font Licence"),
        other => other,
    }
}

fn file_name(face: &FaceInfo) -> Option<String> {
    face.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned())
}

/// The index file a face comes from: by PostScript name, else by weight and slant.
fn file_of<'a>(family: &'a Family, face: &FaceInfo) -> Option<&'a FontFile> {
    let by_name = file_name(face).and_then(|n| family.files.iter().find(|f| f.file_name() == n));
    let by_postscript = || face.postscript_name.as_ref().and_then(|ps| family.files.iter().find(|f| f.postscript.contains(ps)));
    by_name.or_else(by_postscript).or_else(|| family.files.iter().find(|f| f.has_weight(face.weight.round() as u16, face.italic)))
}

impl FontsPanel {
    pub(crate) fn details(&mut self, ctx: &Context, area: Rect) {
        let open = self.detail.is_some();
        let k = ctx.animate_bool_with_time(Id::unique("craft-fonts.sheet"), open, ctx.global_style().animation_time * 1.5);
        let Some(detail) = self.detail.clone() else { return };
        let s = self.skin.clone();
        let dark = ctx.theme() == egui::Theme::Dark;
        let width = (area.width() - 48.0).clamp(300.0, 1040.0).min(area.width());
        let height = (area.height() - 32.0).max(200.0).min(area.height());
        let sheet = Rect::from_center_size(area.center() + vec2(0.0, (1.0 - k) * 16.0), vec2(width, height));
        let mut close = false;
        egui::Area::new(Id::unique("craft-fonts.details")).order(Order::Foreground).fixed_pos(area.min).constrain(false).show(ctx, |ui| {
            let (rect, scrim) = ui.allocate_exact_size(area.size(), Sense::click());
            let alpha = if dark { 165.0 } else { 100.0 } * k;
            ui.painter().rect_filled(rect, CornerRadius::ZERO, Color32::from_black_alpha(alpha.round() as u8));
            close |= scrim.clicked();
            // Clicks on the sheet's own empty space stay on the sheet.
            let _ = ui.interact(sheet, Id::unique("craft-fonts.sheet-bg"), Sense::click());
            let painter = ui.painter();
            painter.add(
                egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(if dark { 140 } else { 36 }) }
                    .as_shape(sheet, s.look.corner),
            );
            painter.rect(sheet, s.look.corner, s.p.bg, Stroke::new(1.0, s.p.line_strong), StrokeKind::Inside);
            let mut child = ui.new_child(UiBuilder::new().max_rect(sheet).layout(direction::column(ui)));
            child.set_opacity(k.clamp(0.0, 1.0));
            child.set_clip_rect(sheet);
            egui::ScrollArea::vertical().id_salt(("craft-fonts.sheet-scroll", format!("{detail:?}"))).auto_shrink([false, false]).show(
                &mut child,
                |ui| {
                    egui::Frame::new().inner_margin(Margin::same(24)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.with_layout(direction::column(ui), |ui| {
                            ui.spacing_mut().item_spacing.y = 8.0;
                            close |= match &detail {
                                Detail::Family(i) => self.family_sheet(ui, &s, *i),
                                Detail::Installed(name, source) => self.installed_sheet(ui, &s, name, *source),
                            };
                        });
                    });
                },
            );
        });
        if close {
            self.detail = None;
        }
    }

    /// The eyebrow, the close button and the title; returns whether Close was clicked.
    fn sheet_header(&mut self, ui: &mut Ui, s: &Skin, eyebrow: &str, title: &str, byline: Option<String>, face: Option<(Owner, FaceInfo)>) -> bool {
        let mut close = false;
        // "Aa" in the face, large, at the header's reading end under the close button.
        if let Some((owner, face)) = face {
            let catalog = match owner {
                Owner::Installed => Arc::clone(&self.catalog),
                Owner::Preview => Arc::clone(&self.previews),
            };
            if let crate::panel::faces::Shown::Ready(family) = self.faces.family(ui.ctx(), &catalog, owner, &face) {
                let top = ui.cursor().top();
                let end = direction::end_x(ui, ui.max_rect()) - direction::forward(ui) * (s.h() + 24.0);
                let anchor = if direction::is_rtl(ui) { egui::Align2::LEFT_TOP } else { egui::Align2::RIGHT_TOP };
                ui.painter().text(egui::pos2(end, top - 14.0), anchor, "Aa", FontId::new(96.0, family), s.p.ink_strong);
            }
        }
        ui.allocate_ui_with_layout(vec2(ui.available_width(), s.h()), direction::row(ui), |ui| {
            widgets::label(ui, s, eyebrow, s.p.faint);
            ui.with_layout(direction::trailing(ui), |ui| {
                close = widgets::glyph_button(ui, s, Glyph::Close, tl!("Close"), s.h()).clicked();
            });
        });
        let title = RichText::new(title).size(30.0).family(s.look.display.clone()).color(s.p.ink_strong).extra_letter_spacing(-0.4);
        ui.add(egui::Label::new(title).wrap());
        if let Some(by) = byline {
            ui.label(RichText::new(by).color(s.p.muted));
        }
        ui.add_space(6.0);
        close
    }

    fn family_sheet(&mut self, ui: &mut Ui, s: &Skin, i: usize) -> bool {
        let ctx = ui.ctx().clone();
        let Some(index) = self.index() else { return true };
        let Some(family) = index.families().get(i).cloned() else { return true };
        let key = family_key(&family.source, &family.family);
        let installed: Vec<FaceInfo> = self.installed_by_name.get(&family.family.to_lowercase()).cloned().unwrap_or_default();
        let store: Vec<FaceInfo> = installed.iter().filter(|f| f.source == Source::Store).cloned().collect();
        let on_system = !installed.is_empty() && store.is_empty();
        let previewed = self.previewed.get(&key).cloned();
        let session_preview = installed.is_empty() && previewed.is_none() && !self.subsets.previewing();
        let job = self.jobs.of(&family.source, &family.family).map(|j| (j.kind, j.file.clone(), j.fraction()));
        let (source, curated) = source_label(&index, &family);
        let eyebrow = [source.to_owned(), category_label(&family)].join(" · ");
        let by = (!family.designer.is_empty()).then(|| fmt("Designed by {designer}", &[("designer", &family.designer)]));
        let hero = regular(&installed).cloned().map(|f| (Owner::Installed, f));
        let hero = hero.or_else(|| previewed.as_deref().and_then(regular).cloned().map(|f| (Owner::Preview, f)));
        let close = self.sheet_header(ui, s, &eyebrow, &family.family, by, hero);

        // Install or remove the whole family.
        let size = i18n::size(family.size());
        let all_installed =
            !installed.is_empty() && (on_system || family.files.iter().all(|f| store.iter().any(|x| file_name(x).as_deref() == Some(f.file_name()))));
        ui.allocate_ui_with_layout(vec2(ui.available_width(), s.h()), direction::row(ui), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            match &job {
                Some((kind, None, fraction)) => {
                    let what = match kind {
                        Kind::Install => tl!("Installing"),
                        Kind::Remove => tl!("Removing"),
                        Kind::Preview => tl!("Downloading"),
                    };
                    self.progress(ui, s, what, *fraction, 220.0);
                }
                _ => {
                    if all_installed {
                        widgets::badge(ui, s, if on_system { tl!("Installed on this system") } else { tl!("Installed") }, s.p.good, true);
                    } else if widgets::button(ui, s, tl!("Install all"), widgets::Kind::Primary, s.h(), 0.0).clicked() {
                        self.ask(&ctx, Pending::Install(i));
                    }
                    if !store.is_empty() && widgets::button(ui, s, tl!("Remove all"), widgets::Kind::Danger, s.h(), 0.0).clicked() {
                        self.remove(&ctx, &family.family, &store, true);
                    }
                    if session_preview && widgets::button(ui, s, tl!("Preview"), widgets::Kind::Secondary, s.h(), 0.0).clicked() {
                        self.ask(&ctx, Pending::Preview(i));
                    }
                    widgets::label(ui, s, &fmt("{size} download", &[("size", &size)]), s.p.faint);
                }
            }
        });
        if session_preview {
            let note = fmt("Preview downloads this font ({size}) for this session without installing it.", &[("size", &size)]);
            ui.label(RichText::new(note).size(12.5).color(s.p.muted));
        }
        if !curated {
            ui.add_space(4.0);
            let added = index.source_of(&family).and_then(|f| f.user_url.clone()).unwrap_or_default();
            widgets::note(ui, s, |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(RichText::new(tl!("Not curated")).color(s.p.warn));
                    let text = fmt(
                        "This family comes from a source you added ({url}). Craft doesn't review its fonts or their licences.",
                        &[("url", &added)],
                    );
                    ui.add(egui::Label::new(RichText::new(text).size(12.5).color(s.p.muted)).wrap());
                });
            });
        }
        if let Some(failure) = self.failed.get(&key).cloned() {
            ui.add_space(4.0);
            let what = match failure.kind {
                Kind::Install => tl!("Couldn't install"),
                Kind::Remove => tl!("Couldn't remove"),
                Kind::Preview => tl!("Couldn't download"),
            };
            let buttons: &[&str] = if failure.retry.is_some() { &[tl!("Retry")] } else { &[] };
            let mut retry = false;
            widgets::note_with_actions(
                ui,
                s,
                |ui| {
                    ui.label(RichText::new(what).color(s.p.bad));
                    ui.add(egui::Label::new(RichText::new(&failure.error).size(12.5).color(s.p.muted)).wrap());
                },
                buttons,
                |ui, _| retry = widgets::button(ui, s, tl!("Retry"), widgets::Kind::Secondary, 28.0, 0.0).clicked(),
                false,
            );
            if retry && let Some(pending) = failure.retry.clone() {
                self.ask(&ctx, pending);
            }
        }
        ui.add_space(14.0);

        let rows = self.family_rows(&family, &installed, &store, on_system, previewed.as_deref(), &job);
        let mut facts = family_facts(&index, &family, &size);
        let source_fact = if curated { source.to_owned() } else { format!("{source} · {}", tl!("Not curated")) };
        facts.insert(0, (tl!("Source").to_owned(), source_fact));
        // Most speakers first: the list is cut short.
        let mut languages: Vec<_> = family.languages(&index).filter(|l| !l.historical).collect();
        languages.sort_by(|a, b| b.population.cmp(&a.population).then_with(|| a.name.cmp(&b.name)));
        let languages: Vec<String> = languages.into_iter().map(|l| l.name.clone()).collect();
        self.sheet_body(ui, s, i, &family.family, &family.scripts, rows, facts, languages);
        close
    }

    fn family_rows(
        &self,
        family: &Family,
        installed: &[FaceInfo],
        store: &[FaceInfo],
        on_system: bool,
        previewed: Option<&[FaceInfo]>,
        job: &Option<(Kind, Option<String>, Option<f32>)>,
    ) -> Vec<StyleRow> {
        let running = |path: &str| match job {
            Some((Kind::Install, Some(p), fraction)) if p == path => Some(*fraction),
            _ => None,
        };
        let mut rows = Vec::new();
        for face in installed {
            let action = match job {
                Some((Kind::Remove, Some(p), _)) if face.path.as_ref().is_some_and(|x| x.display().to_string() == *p) => RowAction::Running(None),
                _ if face.source == Source::Store => RowAction::Remove(Box::new(face.clone())),
                _ => RowAction::None,
            };
            rows.push(StyleRow { label: style_name(face).to_owned(), face: Some((Owner::Installed, face.clone())), action });
        }
        if on_system {
            return rows;
        }
        for file in &family.files {
            if store.iter().any(|x| file_name(x).as_deref() == Some(file.file_name())) {
                continue;
            }
            let preview_faces: Vec<&FaceInfo> =
                previewed.unwrap_or_default().iter().filter(|f| file_of(family, f).is_some_and(|x| x.path == file.path)).collect();
            let action = match running(&file.path) {
                Some(fraction) => RowAction::Running(fraction),
                None => RowAction::Install(file.path.clone()),
            };
            if preview_faces.is_empty() {
                rows.push(StyleRow { label: file_label(file), face: None, action });
            } else {
                for (n, face) in preview_faces.iter().enumerate() {
                    // A variable file's instances install together: the file's action on its first.
                    let action = if n == 0 { action_clone(&action) } else { RowAction::None };
                    rows.push(StyleRow { label: style_name(face).to_owned(), face: Some((Owner::Preview, (*face).clone())), action });
                }
            }
        }
        rows
    }

    fn installed_sheet(&mut self, ui: &mut Ui, s: &Skin, name: &str, source: Source) -> bool {
        let ctx = ui.ctx().clone();
        let Some(family) = self.installed.iter().find(|f| f.name == name && f.source == source).cloned() else { return true };
        let hero = regular(&family.faces).cloned().map(|f| (Owner::Installed, f));
        let close = self.sheet_header(ui, s, source_name(source), &family.name, None, hero);
        let job = self.jobs.of("", &family.name).map(|j| (j.kind, j.file.clone(), j.fraction()));
        ui.allocate_ui_with_layout(vec2(ui.available_width(), s.h()), direction::row(ui), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if let Some((Kind::Remove, None, _)) = &job {
                self.progress(ui, s, tl!("Removing"), None, 220.0);
                return;
            }
            let remove =
                ui.add_enabled_ui(family.removable(), |ui| widgets::button(ui, s, tl!("Remove all"), widgets::Kind::Danger, s.h(), 0.0)).inner;
            if !family.removable() {
                let _ = remove.on_disabled_hover_text(tl!("System fonts are removed with your system's own font tools."));
                widgets::label(ui, s, tl!("Removed with your system's font tools"), s.p.faint);
            } else if remove.clicked() {
                self.remove(&ctx, &family.name, &family.faces, true);
            }
        });
        ui.add_space(14.0);
        let rows = family
            .faces
            .iter()
            .map(|face| StyleRow {
                label: style_name(face).to_owned(),
                face: Some((Owner::Installed, face.clone())),
                action: if face.source == Source::Store { RowAction::Remove(Box::new(face.clone())) } else { RowAction::None },
            })
            .collect();
        let mut files: Vec<_> = family.faces.iter().filter_map(|f| f.path.clone()).collect();
        files.sort();
        files.dedup();
        let facts = vec![
            (tl!("Source").to_owned(), source_name(source).to_owned()),
            (tl!("Styles").to_owned(), i18n::number(family.faces.len() as u64)),
            (tl!("Files").to_owned(), i18n::number(files.len() as u64)),
        ];
        let scripts = self.index().and_then(|index| index.family(&family.name).map(|f| f.scripts.clone())).unwrap_or_default();
        self.sheet_body(ui, s, usize::MAX, &family.name, &scripts, rows, facts, Vec::new());
        close
    }

    fn progress(&self, ui: &mut Ui, s: &Skin, what: &str, fraction: Option<f32>, width: f32) {
        let text = match fraction {
            Some(f) => format!("{what} {}", fmt("{percent} %", &[("percent", &i18n::number((f * 100.0).round() as u64))])),
            None => format!("{what}…"),
        };
        let (rect, resp) = ui.allocate_exact_size(vec2(width, s.h()), Sense::hover());
        let galley = s.label_galley(ui, &text, s.p.muted);
        widgets::paint_start(ui, rect.with_max_y(rect.bottom() - 10.0), galley, s.p.muted);
        widgets::progress_bar(ui, s, egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 4.0), rect.max), fraction);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ProgressIndicator, true, &text));
    }

    /// The styles, and beside them (or below, when narrow) the facts and languages.
    #[allow(clippy::too_many_arguments)]
    fn sheet_body(
        &mut self,
        ui: &mut Ui,
        s: &Skin,
        family_index: usize,
        family: &str,
        scripts: &[String],
        rows: Vec<StyleRow>,
        facts: Vec<(String, String)>,
        languages: Vec<String>,
    ) {
        let total = ui.available_width();
        let wide = total >= COLUMNS_FROM;
        let gap = 32.0;
        let side_w = if wide { (total * 0.32).clamp(240.0, 320.0) } else { total };
        let styles_w = if wide { total - side_w - gap } else { total };
        let rtl = direction::is_rtl(ui);
        let layout =
            if wide { if rtl { Layout::right_to_left(Align::Min) } else { Layout::left_to_right(Align::Min) } } else { direction::column(ui) };
        ui.allocate_ui_with_layout(vec2(total, 0.0), layout, |ui| {
            ui.spacing_mut().item_spacing = vec2(gap, 14.0);
            ui.allocate_ui_with_layout(vec2(styles_w, 0.0), direction::column(ui), |ui| {
                ui.set_width(styles_w);
                self.style_rows(ui, s, family_index, family, scripts, rows);
            });
            ui.allocate_ui_with_layout(vec2(side_w, 0.0), direction::column(ui), |ui| {
                ui.set_width(side_w);
                facts_box(ui, s, &facts);
                if !languages.is_empty() {
                    ui.add_space(8.0);
                    let title = fmt("Languages · {n}", &[("n", &i18n::number(languages.len() as u64))]);
                    widgets::label(ui, s, &title, s.p.faint);
                    const SHOWN: usize = 60;
                    let mut text = languages.iter().take(SHOWN).cloned().collect::<Vec<_>>().join(", ");
                    if languages.len() > SHOWN {
                        text = fmt("{list} and {n} more", &[("list", &text), ("n", &i18n::number((languages.len() - SHOWN) as u64))]);
                    }
                    ui.add(egui::Label::new(RichText::new(text).size(13.0).color(s.p.muted)).wrap());
                }
            });
        });
    }

    fn style_rows(&mut self, ui: &mut Ui, s: &Skin, family_index: usize, family: &str, scripts: &[String], rows: Vec<StyleRow>) {
        let ctx = ui.ctx().clone();
        let installed = rows.iter().filter(|r| matches!(r.face, Some((Owner::Installed, _)))).count();
        let title = if installed == rows.len() {
            tn(rows.len() as u64, "{n} style installed", "{n} styles installed")
        } else {
            tn(rows.len() as u64, "{n} style", "{n} styles")
        };
        widgets::label(ui, s, &title, s.p.faint);
        let index = self.index();
        let own = index.as_deref().and_then(|index| {
            let family = index.families().get(family_index).or_else(|| index.family(family))?;
            samples::of_family(index, family)
        });
        let sample = self.sample_for(own, scripts.iter().map(String::as_str));
        let size = self.settings.size.max(24.0);
        let width = ui.available_width();
        for row in rows {
            let top = ui.cursor().top();
            ui.painter().hline(ui.max_rect().x_range(), top, s.hairline());
            ui.add_space(10.0);
            ui.allocate_ui_with_layout(vec2(width, 28.0), direction::row(ui), |ui| {
                widgets::label(ui, s, &row.label, s.p.muted);
                ui.with_layout(direction::trailing(ui), |ui| match &row.action {
                    RowAction::None => {}
                    RowAction::Running(fraction) => self.progress(ui, s, tl!("Installing"), *fraction, 140.0),
                    RowAction::Install(path) => {
                        let install = widgets::button(ui, s, tl!("Install"), widgets::Kind::Secondary, 28.0, 0.0);
                        widgets::name(ui, &install, &fmt("Install {style}", &[("style", &row.label)]));
                        if install.clicked() && family_index != usize::MAX {
                            self.ask(&ctx, Pending::InstallFile(family_index, path.clone()));
                        }
                    }
                    RowAction::Remove(face) => {
                        let remove = widgets::button(ui, s, tl!("Remove"), widgets::Kind::Danger, 28.0, 0.0);
                        widgets::name(ui, &remove, &fmt("Remove {style}", &[("style", &row.label)]));
                        if remove.clicked() {
                            self.remove(&ctx, family, std::slice::from_ref(face), false);
                        }
                    }
                });
            });
            ui.add_space(2.0);
            self.wrapped_sample(ui, s, row.face.as_ref(), &sample, size, width);
            ui.add_space(12.0);
        }
    }

    /// `text` in the face, wrapped to `width`; faint in the interface font when the face isn't here.
    fn wrapped_sample(&mut self, ui: &mut Ui, s: &Skin, face: Option<&(Owner, FaceInfo)>, text: &str, size: f32, width: f32) {
        let (family, color) = match face {
            Some((owner, face)) => {
                let catalog = match owner {
                    Owner::Installed => Arc::clone(&self.catalog),
                    Owner::Preview => Arc::clone(&self.previews),
                };
                match self.faces.family(ui.ctx(), &catalog, *owner, face) {
                    crate::panel::faces::Shown::Ready(family) => (family, s.p.ink),
                    crate::panel::faces::Shown::Loading => {
                        ui.allocate_space(vec2(width, size * 1.3));
                        return;
                    }
                    crate::panel::faces::Shown::Unavailable => (FontFamily::Proportional, s.p.faint),
                }
            }
            None => (FontFamily::Proportional, s.p.faint),
        };
        let size = if face.is_some() { size } else { size.min(20.0) };
        // Sample text keeps its own direction whatever the interface's.
        let align = if craft_i18n::base_direction(text) == Some(craft_i18n::Direction::Rtl) { Align::Max } else { Align::Min };
        ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(align), |ui| {
            ui.add(egui::Label::new(RichText::new(text).font(FontId::new(size, family)).color(color)).wrap());
        });
    }
}

fn action_clone(action: &RowAction) -> RowAction {
    match action {
        RowAction::None => RowAction::None,
        RowAction::Install(p) => RowAction::Install(p.clone()),
        RowAction::Remove(f) => RowAction::Remove(f.clone()),
        RowAction::Running(f) => RowAction::Running(*f),
    }
}

fn family_facts(index: &Index, family: &Family, size: &str) -> Vec<(String, String)> {
    let scripts: Vec<&str> = family.scripts.iter().map(|id| script_name(index, id).unwrap_or(id)).collect();
    let mut facts = Vec::new();
    if !family.designer.is_empty() {
        facts.push((tl!("Designer").to_owned(), family.designer.clone()));
    }
    facts.push((tl!("Licence").to_owned(), licence_name(&family.license).to_owned()));
    facts.push((tl!("Styles").to_owned(), i18n::number(style_count(family) as u64)));
    facts.push((tl!("Files").to_owned(), i18n::number(family.files.len() as u64)));
    facts.push((tl!("Download size").to_owned(), size.to_owned()));
    if !scripts.is_empty() {
        facts.push((tl!("Scripts").to_owned(), scripts.join(", ")));
    }
    facts
}

/// Facts in a hairline box, each a small label over its value.
fn facts_box(ui: &mut Ui, s: &Skin, facts: &[(String, String)]) {
    egui::Frame::new().stroke(s.hairline()).corner_radius(s.look.corner).inner_margin(Margin::symmetric(14, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 3.0;
        for (n, (label, value)) in facts.iter().enumerate() {
            if n > 0 {
                ui.add_space(6.0);
                let y = ui.cursor().top();
                ui.painter().hline(ui.max_rect().x_range(), y, s.hairline());
                ui.add_space(8.0);
            }
            widgets::label(ui, s, label, s.p.faint);
            ui.add(egui::Label::new(RichText::new(value).color(s.p.ink)).wrap());
        }
    });
}
