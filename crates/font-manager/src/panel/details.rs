//! The details sheet: a family's every style in its own face, its facts, languages and licence,
//! and installing or removing it whole or a style at a time.
//!
//! With a preview service, the styles not installed draw with its subsets: a static file's own,
//! and one subset of each variable file with every axis kept, drawn at each named instance and at
//! the values of the file's axis sliders. The axis values go with the text drawn, not with the
//! font registered, so the sliders never register the face again.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use craft_ui::direction;
use egui::{
    Align, Color32, Context, CornerRadius, FontFamily, FontId, Id, Layout, Margin, Order, Rect, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder,
    vec2,
};

use crate::index::{Family, FontFile, Index, script_name, source_label};
use crate::panel::app::{Detail, FontsPanel, Pending, category_label, family_key, regular, source_name, style_count, style_name};
use crate::panel::available::in_os_fonts;
use crate::panel::faces::{Owner, Shown};
use crate::panel::i18n::{self, fmt, tl, tn};
use crate::panel::jobs::Kind;
use crate::panel::samples;
use crate::panel::widgets::{self, Glyph, Skin};
use crate::store::{LegalText, StoreError};
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

/// What a style row draws its sample with.
enum RowFace {
    /// An installed face, or one of a downloaded preview file.
    Face(Owner, Box<FaceInfo>),
    /// The preview service's subset of the family's file at this position, at these axis values
    /// (none for a static file, or a variable file's default).
    Subset(usize, Vec<([u8; 4], f32)>),
    /// A variable file's subset at the values of its axis sliders.
    Sliders(usize),
}

struct StyleRow {
    label: String,
    face: Option<RowFace>,
    action: RowAction,
}

/// A licence or NOTICE text being read, or read.
pub(crate) enum Legal {
    Loading(Receiver<Result<Option<String>, StoreError>>),
    /// `None` when the family has no such file.
    Ready(Option<String>),
    Failed(String),
}

/// A variable axis as its slider shows it.
struct AxisRange {
    tag: [u8; 4],
    name: String,
    min: f32,
    default: f32,
    max: f32,
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

/// `tag` as an OpenType tag: four printable ASCII characters, spaces only at the end.
fn axis_tag(tag: &[u8]) -> Option<[u8; 4]> {
    let tag: [u8; 4] = tag.try_into().ok()?;
    let printable = tag.iter().all(|b| (0x20..=0x7e).contains(b));
    let spaces_last = tag.iter().skip_while(|b| **b != b' ').all(|b| *b == b' ');
    (printable && spaces_last && tag[0] != b' ').then_some(tag)
}

fn axis_name(tag: [u8; 4]) -> Option<&'static str> {
    Some(match &tag {
        b"wght" => tl!("Weight"),
        b"wdth" => tl!("Width"),
        b"ital" => tl!("Italic"),
        b"slnt" => tl!("Slant"),
        b"opsz" => tl!("Optical size"),
        _ => return None,
    })
}

/// A variable file's axes from its subset's `fvar` (the registered axes by their own names,
/// others by the font's), else the index's ranges about the file's default style.
fn axes_of(font: Option<&[u8]>, file: &FontFile) -> Vec<AxisRange> {
    use skrifa::MetadataProvider;
    if let Some(font) = font.and_then(|f| skrifa::FontRef::new(f).ok())
        && !font.axes().is_empty()
    {
        return font
            .axes()
            .iter()
            .filter(|a| !a.is_hidden() && a.min_value() < a.max_value())
            .filter_map(|a| {
                let tag = axis_tag(&a.tag().to_be_bytes())?;
                let own = || font.localized_strings(a.name_id()).english_or_first().map(|n| n.to_string());
                let name = axis_name(tag).map(str::to_owned).or_else(own).unwrap_or_else(|| String::from_utf8_lossy(&tag).trim().to_owned());
                Some(AxisRange { tag, name, min: a.min_value(), default: a.default_value(), max: a.max_value() })
            })
            .collect();
    }
    file.axes
        .iter()
        .filter(|a| a.min < a.max)
        .filter_map(|a| {
            let tag = axis_tag(a.tag.as_bytes())?;
            let default = match &tag {
                b"wght" => f32::from(file.weight),
                b"wdth" => file.width,
                b"ital" => f32::from(u8::from(file.is_italic())),
                _ => 0.0,
            };
            let name = axis_name(tag).map_or_else(|| a.tag.clone(), str::to_owned);
            Some(AxisRange { tag, name, min: a.min, default: default.clamp(a.min, a.max), max: a.max })
        })
        .collect()
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
        self.sheet(ctx, area);
        self.subsets.request_detail(ctx);
        if self.detail.is_none() {
            self.detail_axes.clear();
            // Read again when a sheet opens; outcomes are for the sheet they were shown in.
            self.availability.clear();
            self.sharing.clear();
        }
    }

    fn sheet(&mut self, ctx: &Context, area: Rect) {
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
        // Store files of a same-named family from another source are not this family's.
        let mut installed: Vec<FaceInfo> = self.installed_by_name.get(&family.family.to_lowercase()).cloned().unwrap_or_default();
        if let Some(folder) = self.installer.family_folder(&family) {
            keep_store_folder(&mut installed, &folder);
        }
        let store: Vec<FaceInfo> = installed.iter().filter(|f| f.source == Source::Store).cloned().collect();
        let on_system = !installed.is_empty() && store.is_empty();
        let previewed = self.previewed.get(&key).cloned();
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
                        Kind::MakeAvailable => &in_os_fonts("Installing in {place}"),
                        Kind::Withdraw => &in_os_fonts("Removing from {place}"),
                    };
                    self.progress(ui, s, what, *fraction, 220.0);
                }
                _ => {
                    if all_installed {
                        widgets::badge(ui, s, if on_system { tl!("Installed on this system") } else { tl!("Installed") }, s.p.good, true);
                    } else if widgets::button(ui, s, tl!("Install all"), widgets::Kind::Primary, s.h(), 0.0).clicked() {
                        self.ask(&ctx, Pending::Install(i));
                    }
                    // A style's install runs: the family's removal waits for it.
                    if !store.is_empty()
                        && ui
                            .add_enabled_ui(job.is_none(), |ui| widgets::button(ui, s, tl!("Remove all"), widgets::Kind::Danger, s.h(), 0.0))
                            .inner
                            .clicked()
                    {
                        self.remove(&ctx, &family.family, &store, true);
                    }
                    if installed.is_empty()
                        && previewed.is_none()
                        && widgets::button(ui, s, tl!("Preview"), widgets::Kind::Secondary, s.h(), 0.0).clicked()
                    {
                        self.ask(&ctx, Pending::Preview(i));
                    }
                    widgets::label(ui, s, &fmt("{size} download", &[("size", &size)]), s.p.faint);
                }
            }
        });
        self.available_to_all(ui, s, &family.family, &store);
        if installed.is_empty() && previewed.is_none() {
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
        // A removal goes by name alone.
        let failure = self.failed.get(&key).or_else(|| self.failed.get(&family_key("", &family.family))).cloned();
        if let Some(failure) = failure {
            ui.add_space(4.0);
            // Changes to availability report under their switch, never here.
            let what = match failure.kind {
                Kind::Remove => tl!("Couldn't remove"),
                Kind::Preview => tl!("Couldn't download"),
                _ => tl!("Couldn't install"),
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
            rows.push(StyleRow { label: style_name(face).to_owned(), face: Some(RowFace::Face(Owner::Installed, Box::new(face.clone()))), action });
        }
        if on_system {
            return rows;
        }
        for (n, file) in family.files.iter().enumerate() {
            if store.iter().any(|x| file_name(x).as_deref() == Some(file.file_name())) {
                continue;
            }
            let preview_faces: Vec<&FaceInfo> =
                previewed.unwrap_or_default().iter().filter(|f| file_of(family, f).is_some_and(|x| x.path == file.path)).collect();
            let action = match running(&file.path) {
                Some(fraction) => RowAction::Running(fraction),
                None => RowAction::Install(file.path.clone()),
            };
            if preview_faces.is_empty() && !self.subsets.available() {
                rows.push(StyleRow { label: file_label(file), face: None, action });
            } else if preview_faces.is_empty() && !file.is_variable() {
                rows.push(StyleRow { label: file_label(file), face: Some(RowFace::Subset(n, Vec::new())), action });
            } else if preview_faces.is_empty() {
                // A variable file installs whole: the file's action on its first row.
                let mut action = Some(action);
                let mut take = || action.take().unwrap_or(RowAction::None);
                for instance in &file.instances {
                    let coords = instance.coordinates.iter().filter_map(|(tag, v)| Some((axis_tag(tag.as_bytes())?, *v))).collect();
                    rows.push(StyleRow { label: instance.name.clone(), face: Some(RowFace::Subset(n, coords)), action: take() });
                }
                if file.instances.is_empty() {
                    rows.push(StyleRow { label: tl!("Default").to_owned(), face: Some(RowFace::Subset(n, Vec::new())), action: take() });
                }
                rows.push(StyleRow { label: file_label(file), face: Some(RowFace::Sliders(n)), action: take() });
            } else {
                for (n, face) in preview_faces.iter().enumerate() {
                    // A variable file's instances install together: the file's action on its first.
                    let action = if n == 0 { action_clone(&action) } else { RowAction::None };
                    rows.push(StyleRow {
                        label: style_name(face).to_owned(),
                        face: Some(RowFace::Face(Owner::Preview, Box::new((*face).clone()))),
                        action,
                    });
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
            let remove = ui
                .add_enabled_ui(family.removable() && job.is_none(), |ui| {
                    widgets::button(ui, s, tl!("Remove all"), widgets::Kind::Danger, s.h(), 0.0)
                })
                .inner;
            if !family.removable() {
                let _ = remove.on_disabled_hover_text(tl!("System fonts are removed with your system's own font tools."));
                widgets::label(ui, s, tl!("Removed with your system's font tools"), s.p.faint);
            } else if remove.clicked() {
                self.remove(&ctx, &family.name, &family.faces, true);
            }
        });
        if source == Source::Store {
            self.available_to_all(ui, s, &family.name, &family.faces);
        }
        ui.add_space(14.0);
        let rows = family
            .faces
            .iter()
            .map(|face| StyleRow {
                label: style_name(face).to_owned(),
                face: Some(RowFace::Face(Owner::Installed, Box::new(face.clone()))),
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
                if let Some(index) = self.index()
                    && let Some(family) = index.families().get(family_index)
                {
                    ui.add_space(8.0);
                    self.licence_block(ui, s, &index, family_index, family);
                }
            });
        });
    }

    /// The licence, the copyright and designer, the upstream link, the NOTICE text, and the
    /// licence text on demand; the fields the index lacks are left out.
    fn licence_block(&mut self, ui: &mut Ui, s: &Skin, index: &Arc<Index>, i: usize, family: &Family) {
        let ctx = ui.ctx().clone();
        let key = family_key(&family.source, &family.family);
        let allowed = self.allowed(ctx.input(|i| i.time));
        // An Apache family's NOTICE is shown with its fonts: read it once downloads are allowed.
        if family.notice_file.is_some() && allowed && !self.legal.contains_key(&(key.clone(), LegalText::Notice)) {
            self.load_legal(&ctx, index, i, LegalText::Notice);
        }
        for state in self.legal.values_mut() {
            if let Legal::Loading(rx) = state {
                match rx.try_recv() {
                    Ok(Ok(text)) => *state = Legal::Ready(text),
                    Ok(Err(e)) => *state = Legal::Failed(e.0),
                    Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Disconnected) => *state = Legal::Ready(None),
                }
            }
        }
        let mut facts = Vec::new();
        if !family.license.is_empty() {
            facts.push((tl!("Licence").to_owned(), licence_name(&family.license).to_owned()));
        }
        if let Some(copyright) = family.copyright.as_deref().filter(|c| !c.trim().is_empty()) {
            facts.push((tl!("Copyright").to_owned(), copyright.trim().to_owned()));
        }
        if !family.designer.is_empty() {
            facts.push((tl!("Designer").to_owned(), family.designer.clone()));
        }
        if !facts.is_empty() {
            facts_box(ui, s, &facts);
        }
        if let Some(url) = family.upstream.as_deref().filter(|u| u.starts_with("https://") || u.starts_with("http://")) {
            let shown = url.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/');
            let link = widgets::link(ui, s, shown);
            widgets::name(ui, &link, &fmt("Upstream: {url}", &[("url", url)]));
            if link.clicked() {
                ctx.open_url(egui::OpenUrl::new_tab(url));
            }
        }
        if family.notice_file.is_some() {
            self.legal_text(ui, s, index, i, &key, LegalText::Notice, tl!("Show notice"));
        }
        if family.license_file_name().is_some() {
            self.legal_text(ui, s, index, i, &key, LegalText::License, tl!("Show licence text"));
        }
    }

    /// A licence or NOTICE text: a button that reads it, then the text, or why it couldn't be read.
    #[allow(clippy::too_many_arguments)]
    fn legal_text(&mut self, ui: &mut Ui, s: &Skin, index: &Arc<Index>, i: usize, key: &str, which: LegalText, show: &str) {
        let ctx = ui.ctx().clone();
        let mut load = false;
        match self.legal.get(&(key.to_owned(), which)) {
            None => load = widgets::button(ui, s, show, widgets::Kind::Secondary, 28.0, 0.0).clicked(),
            Some(Legal::Loading(_)) => {
                ui.label(RichText::new(format!("{}…", tl!("Loading"))).size(12.5).color(s.p.muted));
            }
            Some(Legal::Ready(None)) => {}
            Some(Legal::Ready(Some(text))) => {
                let title = match which {
                    LegalText::License => tl!("Licence text"),
                    LegalText::Notice => tl!("Notice"),
                };
                widgets::label(ui, s, title, s.p.faint);
                let text = text.clone();
                egui::Frame::new().stroke(s.hairline()).corner_radius(s.look.corner).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::ScrollArea::vertical().id_salt(("craft-fonts.legal", key, which)).max_height(220.0).show(ui, |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(text).size(11.5).family(FontFamily::Monospace).color(s.p.muted)).wrap().selectable(true),
                        );
                    });
                });
            }
            Some(Legal::Failed(error)) => {
                let error = error.clone();
                ui.add(egui::Label::new(RichText::new(error).size(12.5).color(s.p.bad)).wrap());
                load = widgets::button(ui, s, tl!("Retry"), widgets::Kind::Secondary, 28.0, 0.0).clicked();
            }
        }
        if load {
            self.load_legal(&ctx, index, i, which);
        }
    }

    /// Reads the licence or NOTICE text of the `i`th family through the store, on a thread.
    fn load_legal(&mut self, ctx: &Context, index: &Arc<Index>, i: usize, which: LegalText) {
        let Some(family) = index.families().get(i) else { return };
        let key = (family_key(&family.source, &family.family), which);
        let (tx, rx) = std::sync::mpsc::channel();
        let installer = Arc::clone(&self.installer);
        let index = Arc::clone(index);
        let wake = ctx.clone();
        let spawned = std::thread::Builder::new().name("font-licence".into()).spawn(move || {
            if let Some(family) = index.families().get(i) {
                let _ = tx.send(installer.legal_text(&index, family, which));
            }
            wake.request_repaint();
        });
        let state = match spawned {
            Ok(_) => Legal::Loading(rx),
            Err(e) => Legal::Failed(e.to_string()),
        };
        self.legal.insert(key, state);
    }

    fn style_rows(&mut self, ui: &mut Ui, s: &Skin, family_index: usize, family: &str, scripts: &[String], rows: Vec<StyleRow>) {
        let ctx = ui.ctx().clone();
        let installed = rows.iter().filter(|r| matches!(r.face, Some(RowFace::Face(Owner::Installed, _)))).count();
        let styles = rows.iter().filter(|r| !matches!(r.face, Some(RowFace::Sliders(_)))).count();
        let title = if installed == styles {
            tn(styles as u64, "{n} style installed", "{n} styles installed")
        } else {
            tn(styles as u64, "{n} style", "{n} styles")
        };
        widgets::label(ui, s, &title, s.p.faint);
        let index = self.index();
        let own = index.as_deref().and_then(|index| {
            let family = index.families().get(family_index).or_else(|| index.family(family))?;
            samples::of_family(index, family)
        });
        // The family of the index whose files subset rows draw.
        let of_index = index.as_ref().and_then(|index| Some((Arc::clone(index), index.families().get(family_index)?.clone())));
        let sample = self.sample_for(own, scripts.iter().map(String::as_str));
        let size = self.settings.size.max(24.0);
        let width = ui.available_width();
        for row in rows {
            let top = ui.cursor().top();
            ui.painter().hline(ui.max_rect().x_range(), top, s.hairline());
            ui.add_space(10.0);
            ui.allocate_ui_with_layout(vec2(width, 28.0), direction::row(ui), |ui| {
                widgets::label(ui, s, &row.label, s.p.muted);
                if let Some(RowFace::Face(Owner::Installed, face)) = &row.face
                    && self.pending_style(family, face)
                {
                    widgets::label(ui, s, &in_os_fonts("Not yet in {place}"), s.p.warn);
                }
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
                        let idle = self.jobs.of("", family).is_none();
                        let remove = ui.add_enabled_ui(idle, |ui| widgets::button(ui, s, tl!("Remove"), widgets::Kind::Danger, 28.0, 0.0)).inner;
                        widgets::name(ui, &remove, &fmt("Remove {style}", &[("style", &row.label)]));
                        if remove.clicked() {
                            self.remove(&ctx, family, std::slice::from_ref(face), false);
                        }
                    }
                });
            });
            ui.add_space(2.0);
            let (shown, coords) = match row.face {
                Some(RowFace::Face(owner, face)) => {
                    let catalog = match owner {
                        Owner::Installed => Arc::clone(&self.catalog),
                        Owner::Preview => Arc::clone(&self.previews),
                    };
                    (Some(self.faces.family(&ctx, &catalog, owner, &face)), Vec::new())
                }
                Some(RowFace::Subset(n, coords)) => (self.subset_face(&ctx, of_index.as_ref(), n, &sample).0, coords),
                Some(RowFace::Sliders(n)) => {
                    let (shown, font) = self.subset_face(&ctx, of_index.as_ref(), n, &sample);
                    let file = of_index.as_ref().and_then(|(_, family)| family.files.get(n));
                    let coords = match file {
                        Some(file) => self.axis_sliders(ui, s, file, &axes_of(font.as_deref().map(Vec::as_slice), file), width),
                        None => Vec::new(),
                    };
                    (shown, coords)
                }
                None => (None, Vec::new()),
            };
            self.wrapped_sample(ui, s, shown, &coords, &sample, size, width);
            ui.add_space(12.0);
        }
    }

    /// The preview service's subset of the `n`th file of `family` for `text`, registered, and
    /// its bytes; nothing while none of that file has loaded, or without a service.
    fn subset_face(&mut self, ctx: &Context, family: Option<&(Arc<Index>, Family)>, n: usize, text: &str) -> (Option<Shown>, Option<Arc<Vec<u8>>>) {
        let Some((index, family)) = family else { return (None, None) };
        let Some(file) = family.files.get(n) else { return (None, None) };
        match self.subsets.detail_font(index, family, file, text) {
            Some((key, font)) => (Some(self.faces.subset(ctx, &key, &font)), Some(font)),
            None => (None, None),
        }
    }

    /// A slider for each of `axes`, with its name and value; returns the values set.
    fn axis_sliders(&mut self, ui: &mut Ui, s: &Skin, file: &FontFile, axes: &[AxisRange], width: f32) -> Vec<([u8; 4], f32)> {
        let mut coords = Vec::new();
        for axis in axes {
            let value = self.detail_axes.entry((file.sha256.clone(), axis.tag)).or_insert(axis.default);
            ui.allocate_ui_with_layout(vec2(width, s.h()), direction::row(ui), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.allocate_ui_with_layout(vec2(110.0, s.h()), direction::row(ui), |ui| {
                    ui.set_width(110.0);
                    widgets::label(ui, s, &axis.name, s.p.muted);
                });
                widgets::slider(ui, s, value, axis.min..=axis.max, (width - 180.0).clamp(80.0, 320.0), &axis.name);
                widgets::label(ui, s, &format!("{value:.0}"), s.p.faint);
            });
            coords.push((axis.tag, *value));
        }
        coords
    }

    /// `text` in the face shown, at `coords`, wrapped to `width`; faint in the interface font
    /// when there is no face.
    #[allow(clippy::too_many_arguments)]
    fn wrapped_sample(&mut self, ui: &mut Ui, s: &Skin, shown: Option<Shown>, coords: &[([u8; 4], f32)], text: &str, size: f32, width: f32) {
        let has_face = shown.is_some();
        let (family, color) = match shown {
            Some(Shown::Ready(family)) => (family, s.p.ink),
            Some(Shown::Loading) => {
                ui.allocate_space(vec2(width, size * 1.3));
                return;
            }
            Some(Shown::Unavailable) | None => (FontFamily::Proportional, s.p.faint),
        };
        let size = if has_face { size } else { size.min(20.0) };
        // Sample text keeps its own direction whatever the interface's.
        let align = if craft_i18n::base_direction(text) == Some(craft_i18n::Direction::Rtl) { Align::Max } else { Align::Min };
        ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(align), |ui| {
            let text = RichText::new(text).font(FontId::new(size, family)).color(color).variations(coords.iter().copied());
            ui.add(egui::Label::new(text).wrap());
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
    let mut facts = vec![
        (tl!("Styles").to_owned(), i18n::number(style_count(family) as u64)),
        (tl!("Files").to_owned(), i18n::number(family.files.len() as u64)),
        (tl!("Download size").to_owned(), size.to_owned()),
    ];
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

/// Drops the store faces outside `folder`: the files of a same-named family from another source.
fn keep_store_folder(faces: &mut Vec<FaceInfo>, folder: &std::path::Path) {
    faces.retain(|f| belongs(f.source, f.path.as_deref(), folder));
}

/// Whether a face is not a store face, or is one directly in `folder`.
fn belongs(source: Source, path: Option<&std::path::Path>, folder: &std::path::Path) -> bool {
    source != Source::Store || path.and_then(|p| p.parent()) == Some(folder)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn same_named_families_from_two_sources_are_told_apart() {
        let mine = Path::new("/s/mine/roboto");
        assert!(belongs(Source::Store, Some(Path::new("/s/mine/roboto/Regular.ttf")), mine));
        assert!(!belongs(Source::Store, Some(Path::new("/s/roboto/Regular.ttf")), mine));
        assert!(belongs(Source::System, Some(Path::new("/sys/Roboto.ttf")), mine));
    }
}
