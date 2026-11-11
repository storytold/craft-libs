//! The details sheet: a family's every style in its own face, its facts and languages, and
//! installing or removing it whole or a style at a time.

mod rows;
#[cfg(feature = "screen-testing")]
mod stories;

use std::sync::Arc;

use craft_behavior::ColumnWidth;
use craft_host::Host;
use craft_theme::variant::{Emphasis, Tone};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, CollectionEvent, NoValue, Outcome, Props as _, Show as _, ShowCollection as _, ShowContainer as _, clicked};
use craft_ui::dialog::Dialog;
use craft_ui::display::{Badge, Progress, Shade, Text};
use craft_ui::layout_components::{Grid, Row, Stack};
use craft_ui::notices::{Notice, NoticeAction};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::Variant;
use egui::{Response, Ui};

use self::rows::{Action, Lines, StyleLine, StyleList, StyleRow};
use crate::index::{Family, FontFile, Index, script_name, source_label};
use crate::panel::app::{Detail, FontsPanel, FontsPanelWords, Pending, category_label, family_key, regular, source_name, style_count, style_name};
use crate::panel::faces::Owner;
use crate::panel::jobs::Kind;
use crate::panel::specimen::{FontSample, SampleFace};
use crate::{Catalog, FaceInfo, Source};

/// The sample is at least this many points: the sheet shows styles, not thumbnails.
const MIN_SAMPLE_POINTS: f32 = 24.0;
/// The large "Aa" in the sheet's header.
const HERO_POINTS: f32 = 96.0;
/// The most languages the sheet lists; the rest are counted.
const LANGUAGES_SHOWN: usize = 60;

/// The facts' labels over their values.
static FACT_COLUMNS: [ColumnWidth; 2] = [ColumnWidth::Content, ColumnWidth::fill(1.0)];

/// The installed catalog and the preview catalog, which the faces of the two owners load from.
type Catalogs = (Arc<Catalog>, Arc<Catalog>);

craft_ui::words! {
    /// The details sheet's words, English by default.
    #[non_exhaustive]
    pub struct DetailsWords {
        /// `{designer}`.
        designed_by: template "fontDetails.designedBy" = "Designed by {designer}";
        installing: "fontDetails.installing" = "Installing";
        removing: "fontDetails.removing" = "Removing";
        downloading: "fontDetails.downloading" = "Downloading";
        installed: "fontDetails.installed" = "Installed";
        installed_on_system: "fontDetails.installedOnSystem" = "Installed on this system";
        install_all: "fontDetails.installAll" = "Install all";
        remove_all: "fontDetails.removeAll" = "Remove all";
        preview: "fontDetails.preview" = "Preview";
        install: "fontDetails.install" = "Install";
        remove: "fontDetails.remove" = "Remove";
        /// `{style}`.
        install_style: template "fontDetails.installStyle" = "Install {style}";
        /// `{style}`.
        remove_style: template "fontDetails.removeStyle" = "Remove {style}";
        /// `{size}`.
        download_size_line: template "fontDetails.downloadSizeLine" = "{size} download";
        /// `{size}`.
        preview_note: template "fontDetails.previewNote" = "Preview downloads this font ({size}) for this session without installing it.";
        not_curated: "fontDetails.notCurated" = "Not curated";
        /// `{url}`.
        not_curated_note: template "fontDetails.notCuratedNote" = "This family comes from a source you added ({url}). Craft doesn't review its fonts or their licences.";
        couldnt_install: "fontDetails.couldntInstall" = "Couldn't install";
        couldnt_remove: "fontDetails.couldntRemove" = "Couldn't remove";
        couldnt_download: "fontDetails.couldntDownload" = "Couldn't download";
        retry: "fontDetails.retry" = "Retry";
        system_fonts_note: "fontDetails.systemFontsNote" = "System fonts are removed with your system's own font tools.";
        system_fonts_removal: "fontDetails.systemFontsRemoval" = "Removed with your system's font tools";
        source: "fontDetails.source" = "Source";
        designer: "fontDetails.designer" = "Designer";
        licence: "fontDetails.licence" = "Licence";
        styles_fact: "fontDetails.stylesFact" = "Styles";
        files_fact: "fontDetails.filesFact" = "Files";
        download_size: "fontDetails.downloadSize" = "Download size";
        scripts: "fontDetails.scripts" = "Scripts";
        /// `{n}` styles, all installed.
        styles_installed: count "fontDetails.stylesInstalled" = "{n} style installed" | "{n} styles installed";
        /// The empty list of styles.
        no_styles: "fontDetails.noStyles" = "No styles";
        /// `{n}` styles.
        styles: count "fontDetails.styles" = "{n} style" | "{n} styles";
        /// `{n}` languages.
        languages: template "fontDetails.languages" = "Languages · {n}";
        /// `{list}` of the first languages and `{n}` more.
        languages_more: template "fontDetails.languagesMore" = "{list} and {n} more";
        /// `{what}` the job, `{percent}` how far along.
        progress: template "fontDetails.progress" = "{what} {percent}";
        /// `{what}` the job.
        progress_unknown: template "fontDetails.progressUnknown" = "{what}…";
    }
}

fn weight_name(weight: u16) -> &'static str {
    match weight {
        0..=149 => "Thin",
        150..=249 => "ExtraLight",
        250..=349 => "Light",
        350..=449 => "Regular",
        450..=549 => "Medium",
        550..=649 => "SemiBold",
        650..=749 => "Bold",
        750..=849 => "ExtraBold",
        _ => "Black",
    }
}

/// "Bold Italic", or "Variable 100–900" for a variable file. Style and weight names are the font's own English
/// names in every language.
fn file_label(file: &FontFile) -> String {
    let base = match file.axes.iter().find(|a| a.tag == "wght") {
        Some(a) => format!("Variable {}–{}", a.min.round() as u32, a.max.round() as u32),
        None => weight_name(file.weight).to_owned(),
    };
    match (file.is_italic(), file.weight) {
        (true, 350..=449) if file.axes.is_empty() => "Italic".to_owned(),
        (true, _) => format!("{base} Italic"),
        (false, _) => base,
    }
}

fn licence_name(id: &str) -> &str {
    match id {
        "OFL" => "SIL Open Font License",
        "APACHE2" => "Apache License 2.0",
        "UFL" => "Ubuntu Font Licence",
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

/// What the sheet asks of the panel once the pass has drawn it.
enum Do {
    Ask(Pending),
    Remove { faces: Vec<FaceInfo>, whole: bool },
}

/// The job running for the whole family, as the header shows it.
struct Run {
    kind: Kind,
    fraction: Option<f32>,
}

/// What the header's Remove all does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Removal {
    /// The family has nothing the store can remove.
    None,
    Allowed,
    /// The fonts belong to the system.
    System,
}

/// What a failed job left to show.
struct Failed {
    what: Kind,
    error: String,
    retry: Option<Pending>,
}

/// Everything the sheet shows, read from the panel before the pass draws it.
struct Sheet {
    eyebrow: String,
    title: String,
    byline: Option<String>,
    hero: Option<(Owner, FaceInfo)>,
    run: Option<Run>,
    badge: Option<Label>,
    install_all: Option<Pending>,
    removal: Removal,
    store: Vec<FaceInfo>,
    preview: Option<Pending>,
    size_line: Option<String>,
    preview_note: Option<String>,
    uncurated: Option<String>,
    failure: Option<Failed>,
    lines: Vec<StyleLine>,
    facts: Vec<(Label, String)>,
    languages: Vec<String>,
    scripts: Vec<String>,
    /// The family's position in the index; none for an installed family, whose styles cannot be installed.
    index: Option<usize>,
    sample: String,
}

/// What the details sheet reports.
#[derive(Debug)]
#[non_exhaustive]
pub(crate) struct DetailsOutput {
    /// The dialog's response.
    pub response: Response,
    /// The sheet closed in this pass: Escape, its close action or a family that is gone.
    pub closed: bool,
}

impl Outcome for DetailsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The sheet of the family `panel.detail` names: a dialog on the modal preset, closed by Escape or its close action.
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub(crate) struct DetailsSheet<'a> {
    panel: &'a mut FontsPanel,
}

impl<'a> DetailsSheet<'a> {
    /// The sheet of `panel`'s open detail.
    pub(crate) fn new(panel: &'a mut FontsPanel) -> Self {
        DetailsSheet { panel }
    }

    /// Shows the sheet; the panel's detail is cleared when it closes.
    pub(crate) fn show(self, ui: &mut Ui) -> DetailsOutput {
        let DetailsSheet { panel } = self;
        let env = Env::of(ui);
        let words = env.words::<DetailsWords>();
        let host: Option<Arc<Host>> = env.view().host().cloned();
        let Some(sheet) = Sheet::read(panel, &env, &words) else {
            panel.detail = None;
            return DetailsOutput { response: ui.response(), closed: true };
        };
        let catalogs: Catalogs = (Arc::clone(&panel.catalog), Arc::clone(&panel.previews));
        let points = panel.settings.size.max(MIN_SAMPLE_POINTS);
        let mut todo: Vec<Do> = Vec::new();
        let mut open = true;
        let shown = Dialog::prompt(sheet.title.clone()).opened(Bind::bound(&mut open)).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                sheet.header(ui, &env, &words, &catalogs, &mut todo);
                sheet.body(ui, &env, &words, &catalogs, points, &mut todo);
            });
        });
        let closed = shown.output.closed.is_some();
        if closed {
            panel.detail = None;
        }
        for action in todo {
            match action {
                Do::Ask(pending) => panel.ask(host.as_ref(), pending),
                Do::Remove { faces, whole } => panel.remove(host.as_ref(), &sheet.title, &faces, whole),
            }
        }
        DetailsOutput { response: shown.output.response, closed }
    }
}

impl Sheet {
    /// The sheet of the panel's open detail; `None` when there is none or its family is gone.
    fn read(panel: &FontsPanel, env: &Env, words: &DetailsWords) -> Option<Sheet> {
        let mut sheet = match panel.detail.clone()? {
            Detail::Family(i) => Sheet::family(panel, env, words, i)?,
            Detail::Installed(name, source) => Sheet::installed(panel, env, words, &name, source)?,
        };
        // The sample is what the user typed, else the preset for the family's scripts.
        sheet.sample = panel.sample_for(sheet.scripts.iter().map(String::as_str));
        Some(sheet)
    }

    fn family(panel: &FontsPanel, env: &Env, words: &DetailsWords, i: usize) -> Option<Sheet> {
        let index = panel.index()?;
        let family = index.families().get(i).cloned()?;
        let key = family_key(&family.source, &family.family);
        let installed: Vec<FaceInfo> = panel.installed_by_name.get(&family.family.to_lowercase()).cloned().unwrap_or_default();
        let store: Vec<FaceInfo> = installed.iter().filter(|f| f.source == Source::Store).cloned().collect();
        let on_system = !installed.is_empty() && store.is_empty();
        let previewed = panel.previewed.get(&key).cloned();
        let job = panel.jobs.of(&family.source, &family.family).map(|j| (j.kind, j.file.clone(), j.fraction()));
        let (source, curated) = source_label(&index, &family);
        let names = env.words::<FontsPanelWords>();
        let eyebrow = [source.to_owned(), category_label(&names, &family)].join(" · ");
        let byline = (!family.designer.is_empty()).then(|| words.designed_by.fill(&[("designer", &family.designer)]));
        let hero = regular(&installed).cloned().map(|f| (Owner::Installed, f));
        let hero = hero.or_else(|| previewed.as_deref().and_then(regular).cloned().map(|f| (Owner::Preview, f)));
        let size = env.units().file_size(family.size());
        let all_installed =
            !installed.is_empty() && (on_system || family.files.iter().all(|f| store.iter().any(|x| file_name(x).as_deref() == Some(f.file_name()))));
        let run = match &job {
            Some((kind, None, fraction)) => Some(Run { kind: *kind, fraction: *fraction }),
            _ => None,
        };
        let installed_word = if on_system { &words.installed_on_system } else { &words.installed };
        let unseen = installed.is_empty() && previewed.is_none();
        let uncurated = (!curated).then(|| index.source_of(&family).and_then(|f| f.user_url.clone()).unwrap_or_default());
        let failure = panel.failed.get(&key).map(|f| Failed { what: f.kind, error: f.error.clone(), retry: f.retry.clone() });
        let lines = family_lines(&names, &family, &installed, &store, on_system, previewed.as_deref(), &job);
        let mut facts = family_facts(env, words, &index, &family, &size);
        let not_curated = env.view().resolve(&words.not_curated);
        let source_fact = if curated { source.to_owned() } else { format!("{source} · {}", not_curated.as_str()) };
        facts.insert(0, (words.source.clone(), source_fact));
        // Most speakers first: the list is cut short.
        let mut languages: Vec<_> = family.languages(&index).filter(|l| !l.historical).collect();
        languages.sort_by(|a, b| b.population.cmp(&a.population).then_with(|| a.name.cmp(&b.name)));
        Some(Sheet {
            eyebrow,
            title: family.family.clone(),
            byline,
            hero,
            run,
            badge: all_installed.then(|| installed_word.clone()),
            install_all: (!all_installed).then_some(Pending::Install(i)),
            removal: if store.is_empty() { Removal::None } else { Removal::Allowed },
            store,
            preview: unseen.then_some(Pending::Preview(i)),
            size_line: Some(words.download_size_line.fill(&[("size", &size)])),
            preview_note: unseen.then(|| words.preview_note.fill(&[("size", &size)])),
            uncurated,
            failure,
            lines,
            facts,
            languages: languages.into_iter().map(|l| l.name.clone()).collect(),
            scripts: family.scripts.clone(),
            index: Some(i),
            sample: String::new(),
        })
    }

    fn installed(panel: &FontsPanel, env: &Env, words: &DetailsWords, name: &str, source: Source) -> Option<Sheet> {
        let family = panel.installed.iter().find(|f| f.name == name && f.source == source).cloned()?;
        let names = env.words::<FontsPanelWords>();
        let hero = regular(&family.faces).cloned().map(|f| (Owner::Installed, f));
        let job = panel.jobs.of("", &family.name).map(|j| (j.kind, j.file.clone(), j.fraction()));
        let run = match &job {
            Some((Kind::Remove, None, _)) => Some(Run { kind: Kind::Remove, fraction: None }),
            _ => None,
        };
        let lines = family
            .faces
            .iter()
            .map(|face| StyleLine {
                label: style_name(&names, face).to_owned(),
                face: Some((Owner::Installed, face.clone())),
                action: if face.source == Source::Store { Action::Remove(Box::new(face.clone())) } else { Action::None },
            })
            .collect();
        let mut files: Vec<_> = family.faces.iter().filter_map(|f| f.path.clone()).collect();
        files.sort();
        files.dedup();
        let format = env.format();
        let facts = vec![
            (words.source.clone(), source_name(&names, source).as_str().to_owned()),
            (words.styles_fact.clone(), format.integer(family.faces.len() as u64)),
            (words.files_fact.clone(), format.integer(files.len() as u64)),
        ];
        let scripts = panel.index().and_then(|index| index.family(&family.name).map(|f| f.scripts.clone())).unwrap_or_default();
        Some(Sheet {
            eyebrow: source_name(&names, source).as_str().to_owned(),
            title: family.name.clone(),
            byline: None,
            hero,
            run,
            badge: None,
            install_all: None,
            removal: if family.removable() { Removal::Allowed } else { Removal::System },
            store: family.faces.clone(),
            preview: None,
            size_line: None,
            preview_note: None,
            uncurated: None,
            failure: None,
            lines,
            facts,
            languages: Vec::new(),
            scripts,
            index: None,
            sample: String::new(),
        })
    }

    /// The eyebrow, the "Aa" in the family's face, the byline, and the family's install and remove controls.
    fn header(&self, ui: &mut Ui, env: &Env, words: &DetailsWords, catalogs: &Catalogs, todo: &mut Vec<Do>) {
        Text::new(self.eyebrow.clone()).shade(Shade::Faint).show(ui);
        if let Some((owner, face)) = &self.hero {
            let catalog = match owner {
                Owner::Installed => &catalogs.0,
                Owner::Preview => &catalogs.1,
            };
            let face = SampleFace::Face { catalog, owner: *owner, face };
            FontSample::new(format!("{}: Aa", self.title), "Aa", face, HERO_POINTS).show(ui);
        }
        if let Some(byline) = &self.byline {
            Text::new(byline.clone()).shade(Shade::Muted).show(ui);
        }
        Row::new().wrap(true).show(ui, |ui, _| {
            if let Some(run) = &self.run {
                let what = match run.kind {
                    Kind::Install => &words.installing,
                    Kind::Remove => &words.removing,
                    Kind::Preview => &words.downloading,
                };
                progress(ui, env, words, what, run.fraction);
                return;
            }
            if let Some(badge) = &self.badge {
                Badge::new(badge.clone()).variant(Variant::look(Emphasis::Soft, Tone::Good)).show(ui);
            }
            if let Some(pending) = &self.install_all
                && clicked(&Button::new(words.install_all.clone()).variant(Variant::PRIMARY).show(ui))
            {
                todo.push(Do::Ask(pending.clone()));
            }
            match self.removal {
                Removal::Allowed => {
                    if clicked(&Button::new(words.remove_all.clone()).variant(Variant::DANGER).show(ui)) {
                        todo.push(Do::Remove { faces: self.store.clone(), whole: true });
                    }
                }
                Removal::System => {
                    Button::new(words.remove_all.clone()).enabled(false).tooltip(words.system_fonts_note.clone()).show(ui);
                    Text::new(words.system_fonts_removal.clone()).shade(Shade::Faint).show(ui);
                }
                Removal::None => {}
            }
            if let Some(pending) = &self.preview
                && clicked(&Button::new(words.preview.clone()).variant(Variant::SECONDARY).show(ui))
            {
                todo.push(Do::Ask(pending.clone()));
            }
            if let Some(line) = &self.size_line {
                Text::new(line.clone()).shade(Shade::Faint).show(ui);
            }
        });
        if let Some(note) = &self.preview_note {
            Text::new(note.clone()).shade(Shade::Muted).show(ui);
        }
        if let Some(url) = &self.uncurated {
            Notice::new(Tone::Warn, words.not_curated_note.fill(&[("url", url)])).title(words.not_curated.clone()).show(ui);
        }
        if let Some(failure) = &self.failure {
            let what = match failure.what {
                Kind::Install => &words.couldnt_install,
                Kind::Remove => &words.couldnt_remove,
                Kind::Preview => &words.couldnt_download,
            };
            let mut notice = Notice::new(Tone::Bad, failure.error.clone()).title(what.clone());
            if failure.retry.is_some() {
                notice = notice.action(NoticeAction::new(words.retry.clone()));
            }
            let out = notice.show(ui);
            if out.action.is_some()
                && let Some(pending) = &failure.retry
            {
                todo.push(Do::Ask(pending.clone()));
            }
        }
    }

    /// The styles, and beside them (or below, when narrow) the facts and the languages.
    fn body(&self, ui: &mut Ui, env: &Env, words: &DetailsWords, catalogs: &Catalogs, points: f32, todo: &mut Vec<Do>) {
        Row::new().wrap(true).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| self.styles(ui, env, words, catalogs, points, todo));
            Stack::new().show(ui, |ui, _| self.side(ui, env, words));
        });
    }

    fn styles(&self, ui: &mut Ui, env: &Env, words: &DetailsWords, catalogs: &Catalogs, points: f32, todo: &mut Vec<Do>) {
        let total = self.lines.len() as u64;
        let installed = self.lines.iter().filter(|l| matches!(l.face, Some((Owner::Installed, _)))).count();
        let count = env.format().integer(total);
        let heading = if installed == self.lines.len() { words.styles_installed.text(total, &count) } else { words.styles.text(total, &count) };
        Text::new(heading).shade(Shade::Faint).show(ui);
        let source = Lines(&self.lines);
        let out = StyleList::new(self.title.clone(), words.no_styles.clone()).show_items(ui, &source, &mut |key, _| {
            let line = &self.lines[*key];
            let face = match &line.face {
                Some((owner, face)) => {
                    let catalog = match owner {
                        Owner::Installed => &catalogs.0,
                        Owner::Preview => &catalogs.1,
                    };
                    SampleFace::Face { catalog, owner: *owner, face }
                }
                None => SampleFace::Interface,
            };
            StyleRow::new(line, &self.title, &self.sample, points, face, words)
        });
        for event in out.events {
            let CollectionEvent::Item(key, _) = event else { continue };
            match &self.lines[key].action {
                Action::Install(path) => {
                    if let Some(i) = self.index {
                        todo.push(Do::Ask(Pending::InstallFile(i, path.clone())));
                    }
                }
                Action::Remove(face) => todo.push(Do::Remove { faces: vec![(**face).clone()], whole: false }),
                _ => {}
            }
        }
    }

    /// The facts as label and value pairs, then the languages.
    fn side(&self, ui: &mut Ui, env: &Env, words: &DetailsWords) {
        Grid::new(&FACT_COLUMNS).show(ui, |ui, _| {
            for (label, value) in &self.facts {
                Text::new(label.clone()).shade(Shade::Faint).show(ui);
                Text::new(value.clone()).show(ui);
            }
        });
        if self.languages.is_empty() {
            return;
        }
        let count = env.format().integer(self.languages.len() as u64);
        Text::new(words.languages.fill(&[("n", &count)])).shade(Shade::Faint).show(ui);
        let mut text = self.languages.iter().take(LANGUAGES_SHOWN).cloned().collect::<Vec<_>>().join(", ");
        if self.languages.len() > LANGUAGES_SHOWN {
            let more = env.format().integer((self.languages.len() - LANGUAGES_SHOWN) as u64);
            text = words.languages_more.fill(&[("list", &text), ("n", &more)]);
        }
        Text::new(text).shade(Shade::Muted).show(ui);
    }
}

/// A job's progress: the job and the percent done when known, with a bar.
fn progress(ui: &mut Ui, env: &Env, words: &DetailsWords, what: &Label, fraction: Option<f32>) {
    let what = env.view().resolve(what);
    let text = match fraction {
        Some(f) => words.progress.fill(&[("what", what.as_str()), ("percent", &env.format().percent(f64::from(f), 0))]),
        None => words.progress_unknown.fill(&[("what", what.as_str())]),
    };
    Text::new(text.clone()).shade(Shade::Muted).show(ui);
    match fraction {
        Some(f) => Progress::new(f).label(text).show(ui),
        None => Progress::indeterminate().label(text).show(ui),
    };
}

/// The rows of the family sheet: the installed faces, then the index files not installed, as the previewed faces when
/// the preview has them.
fn family_lines(
    names: &FontsPanelWords,
    family: &Family,
    installed: &[FaceInfo],
    store: &[FaceInfo],
    on_system: bool,
    previewed: Option<&[FaceInfo]>,
    job: &Option<(Kind, Option<String>, Option<f32>)>,
) -> Vec<StyleLine> {
    let running = |path: &str| match job {
        Some((Kind::Install, Some(p), fraction)) if p == path => Some(*fraction),
        _ => None,
    };
    let mut lines = Vec::new();
    for face in installed {
        let action = match job {
            Some((Kind::Remove, Some(p), _)) if face.path.as_ref().is_some_and(|x| x.display().to_string() == *p) => Action::Running(None),
            _ if face.source == Source::Store => Action::Remove(Box::new(face.clone())),
            _ => Action::None,
        };
        lines.push(StyleLine { label: style_name(names, face).to_owned(), face: Some((Owner::Installed, face.clone())), action });
    }
    if on_system {
        return lines;
    }
    for file in &family.files {
        if store.iter().any(|x| file_name(x).as_deref() == Some(file.file_name())) {
            continue;
        }
        let preview_faces: Vec<&FaceInfo> =
            previewed.unwrap_or_default().iter().filter(|f| file_of(family, f).is_some_and(|x| x.path == file.path)).collect();
        let action = match running(&file.path) {
            Some(fraction) => Action::Running(fraction),
            None => Action::Install(file.path.clone()),
        };
        if preview_faces.is_empty() {
            lines.push(StyleLine { label: file_label(file), face: None, action });
        } else {
            for (n, face) in preview_faces.iter().enumerate() {
                // A variable file's instances install together: the file's action on its first.
                let action = if n == 0 { action.clone() } else { Action::None };
                lines.push(StyleLine { label: style_name(names, face).to_owned(), face: Some((Owner::Preview, (*face).clone())), action });
            }
        }
    }
    lines
}

fn family_facts(env: &Env, words: &DetailsWords, index: &Index, family: &Family, size: &str) -> Vec<(Label, String)> {
    let scripts: Vec<&str> = family.scripts.iter().map(|id| script_name(index, id).unwrap_or(id)).collect();
    let format = env.format();
    let mut facts: Vec<(Label, String)> = Vec::new();
    if !family.designer.is_empty() {
        facts.push((words.designer.clone(), family.designer.clone()));
    }
    facts.push((words.licence.clone(), licence_name(&family.license).to_owned()));
    facts.push((words.styles_fact.clone(), format.integer(style_count(family) as u64)));
    facts.push((words.files_fact.clone(), format.integer(family.files.len() as u64)));
    facts.push((words.download_size.clone(), size.to_owned()));
    if !scripts.is_empty() {
        facts.push((words.scripts.clone(), scripts.join(", ")));
    }
    facts
}
