//! The details sheet: a family's every style in its own face, its facts and languages, and
//! installing or removing it whole or a style at a time.
//!
//! With a preview service, the styles not installed draw with its subsets: a static file's own, and one subset of each
//! variable file with every axis kept, drawn at each named instance and at the values of the file's axis sliders. The
//! axis values go with the text drawn, not with the face registered, so the sliders never register the face again.

pub(crate) mod rows;
#[cfg(feature = "suite")]
pub(crate) mod stories;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use craft_behavior::ColumnWidth;
use craft_host::Host;
use craft_theme::variant::{Emphasis, Tone};
use craft_ui::buttons::{Button, Link, SplitButton};
use craft_ui::contract::overlay::Outside;
use craft_ui::contract::{
    Bind, CollectionEvent, Entry, NoValue, Outcome, Props as _, Show as _, ShowCollection as _, ShowContainer as _, Sizing, clicked,
};
use craft_ui::dialog::Dialog;
use craft_ui::display::{Badge, Progress, Shade, Text};
use craft_ui::layout_components::{Grid, Row, Stack};
use craft_ui::notices::{Notice, NoticeAction};
use craft_ui::primitives::Wrap;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::focus::Initial;
use craft_ui::style::Variant;
use craft_ui::text::TextField;
use egui::{Response, Ui};

use self::rows::{Action, Lines, StyleLine, StyleList, StyleRow};
use crate::axes::{AxisSliders, AxisValues, FontAxis, OpticalSize, read_axes, shown_axes};
use crate::index::{Family, FontFile, Index, script_name, source_label};
use crate::panel::app::{
    Detail, FontsPanel, FontsPanelWords, NotCopied, Pending, category_label, family_key, regular, source_name, style_count, style_name,
};
use crate::panel::faces::Owner;
use crate::panel::jobs::Kind;
use crate::panel::legal::{Legal, legal_key};
use crate::panel::specimen::{FontSample, HeldFace};
use crate::panel::subsets::SubsetFont;
use crate::store::LegalText;
use crate::{FaceInfo, Source};

/// The sample is at least this many points: the sheet shows styles, not thumbnails.
const MIN_SAMPLE_POINTS: f32 = 24.0;
/// The large "Aa" in the sheet's header.
const HERO_POINTS: f32 = 96.0;
/// The most languages the sheet lists; the rest are counted.
const LANGUAGES_SHOWN: usize = 60;

/// The most lines of a licence or NOTICE text in view; the rest scrolls.
const LEGAL_LINES: usize = 10;
/// The lines its box has for a shorter text: the least of a multi-line field.
const LEGAL_LEAST_LINES: usize = 2;

/// The sheet's sliders show the face's optical sizes, which its samples are drawn at.
const OPTICAL_SIZE: OpticalSize = OpticalSize::Slider;

/// The facts' labels over their values.
const FACT_COLUMNS: [ColumnWidth; 2] = [ColumnWidth::Content, ColumnWidth::fill(1.0)];

craft_ui::words! {
    /// The details sheet's words, English by default.
    #[non_exhaustive]
    pub struct DetailsWords {
        /// `{designer}`.
        designed_by: template "fontDetails.designedBy" = "Designed by {designer}";
        installing: "fontDetails.installing" = "Installing";
        removing: "fontDetails.removing" = "Removing";
        downloading: "fontDetails.downloading" = "Downloading";
        updating_system_fonts: "fontDetails.updatingSystemFonts" = "Updating system fonts";
        installed: "fontDetails.installed" = "Installed";
        installed_on_system: "fontDetails.installedOnSystem" = "Installed on this system";
        install_all: "fontDetails.installAll" = "Install all";
        install_all_options: "fontDetails.installAllOptions" = "More install all options";
        install_all_apps: "fontDetails.installAllApps" = "Install for all apps";
        install_craft_apps: "fontDetails.installCraftApps" = "Install for Craft apps only";
        /// `{reason}`.
        only_craft_apps: template "fontDetails.onlyCraftApps" = "Only in Craft apps: {reason}";
        available_to_all: "fontDetails.availableToAll" = "Available to all apps";
        kept_in_craft_apps: "fontDetails.keptInCraftApps" = "Kept in Craft apps only";
        make_available: "fontDetails.makeAvailable" = "Make available to all apps";
        keep_in_craft_apps: "fontDetails.keepInCraftApps" = "Keep in Craft apps only";
        /// `{error}`.
        copy_failed: template "fontDetails.copyFailed" = "Couldn't copy to your system's fonts folder: {error}";
        /// `{file}` and `{name}`.
        copy_name_taken: template "fontDetails.copyNameTaken" = "Not copied to your system's fonts folder: {file} already has the font name {name}";
        /// `{file}`.
        copy_in_the_way: template "fontDetails.copyInTheWay" = "Not copied to your system's fonts folder: {file} is in the way";
        remove_all: "fontDetails.removeAll" = "Remove all";
        preview: "fontDetails.preview" = "Preview";
        install: "fontDetails.install" = "Install";
        remove: "fontDetails.remove" = "Remove";
        copyright: "fontDetails.copyright" = "Copyright";
        /// `{url}`.
        upstream: template "fontDetails.upstream" = "Upstream: {url}";
        show_licence: "fontDetails.showLicence" = "Show licence text";
        show_notice: "fontDetails.showNotice" = "Show notice";
        licence_text: "fontDetails.licenceText" = "Licence text";
        notice_text: "fontDetails.noticeText" = "Notice";
        loading_text: "fontDetails.loadingText" = "Loading…";
        /// The style of a variable file that names no instances.
        default_style: "fontDetails.defaultStyle" = "Default";
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
        couldnt_update_system_fonts: "fontDetails.couldntUpdateSystemFonts" = "Couldn't update system fonts";
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
    AvailableToAll { files: Vec<PathBuf>, on: bool },
    ReadLegal(LegalText),
}

/// A licence or NOTICE text as the sheet shows it.
enum LegalShown {
    /// Not read yet.
    Offer,
    Loading,
    Text(String),
    Failed(String),
}

/// The licence and NOTICE texts `family` has, as far as they are read.
fn legal_texts(panel: &FontsPanel, family: &Family) -> Vec<(LegalText, LegalShown)> {
    let files = [(LegalText::Notice, family.notice_file.is_some()), (LegalText::License, family.license_file_name().is_some())];
    files
        .into_iter()
        .filter(|(_, has)| *has)
        .filter_map(|(which, _)| {
            let shown = match panel.legal.get(&legal_key(family, which)) {
                None => LegalShown::Offer,
                Some(Legal::Loading(_)) => LegalShown::Loading,
                Some(Legal::Ready(text)) => LegalShown::Text(text.clone()?),
                Some(Legal::Failed(error)) => LegalShown::Failed(error.clone()),
            };
            Some((which, shown))
        })
        .collect()
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

/// Whether the family's store fonts are in the OS user font folder for all apps, or why not.
struct SystemFonts {
    text: Label,
    tone: Option<Tone>,
    /// The store files.
    files: Vec<PathBuf>,
    /// What the line's action sets; none while it can't change.
    switch: Option<bool>,
}

impl SystemFonts {
    /// The line of the family named `family`; none when `faces` has no store font.
    fn read(panel: &FontsPanel, words: &DetailsWords, family: &str, faces: &[FaceInfo]) -> Option<SystemFonts> {
        let files: Vec<PathBuf> = faces.iter().filter(|f| f.source == Source::Store).filter_map(|f| f.path.clone()).collect();
        if files.is_empty() {
            return None;
        }
        let not_here = panel.installer.available_to_all_unsupported();
        let available = not_here.is_none() && panel.installer.is_available_to_all(family, &files);
        let (text, tone) = if let Some(reason) = not_here {
            (Label::from(words.only_craft_apps.fill(&[("reason", &reason)])), None)
        } else if available {
            (words.available_to_all.clone(), Some(Tone::Good))
        } else if let Some(not_copied) = panel.not_copied.get(&family.to_lowercase()) {
            (Label::from(not_copied_text(words, not_copied)), Some(Tone::Warn))
        } else {
            (words.kept_in_craft_apps.clone(), None)
        };
        let switch = (panel.installer.available_to_all_supported() && panel.jobs.of("", family).is_none()).then_some(!available);
        Some(SystemFonts { text, tone, files, switch })
    }

    fn show(&self, ui: &mut Ui, words: &DetailsWords, todo: &mut Vec<Do>) {
        Row::new().wrap(true).show(ui, |ui, _| {
            let text = Text::new(self.text.clone());
            match self.tone {
                Some(tone) => text.tone(tone),
                None => text.shade(Shade::Muted),
            }
            .show(ui);
            if let Some(on) = self.switch {
                let action = if on { &words.make_available } else { &words.keep_in_craft_apps };
                if clicked(&Link::new(action.clone()).show(ui)) {
                    todo.push(Do::AvailableToAll { files: self.files.clone(), on });
                }
            }
        });
    }
}

/// What the sheet says about a family an install couldn't copy into the OS user font folder.
fn not_copied_text(words: &DetailsWords, not_copied: &NotCopied) -> String {
    let conflict = match not_copied {
        NotCopied::Failed(error) => return words.copy_failed.fill(&[("error", error)]),
        NotCopied::Refused(conflicts) => conflicts.first(),
    };
    let file = conflict.map(|c| c.path.display().to_string()).unwrap_or_default();
    match conflict.and_then(|c| c.postscript.as_deref()) {
        Some(name) => words.copy_name_taken.fill(&[("file", &file), ("name", name)]),
        None => words.copy_in_the_way.fill(&[("file", &file)]),
    }
}

/// A variable file drawn by the preview service, and its axes.
struct FileAxes {
    sha256: String,
    label: String,
    axes: Vec<FontAxis>,
}

/// Everything the sheet shows, read from the panel before the pass draws it.
struct Sheet {
    eyebrow: String,
    title: String,
    byline: Option<String>,
    hero: Option<HeldFace>,
    run: Option<Run>,
    badge: Option<Label>,
    /// The family's position in the index when it can be installed whole.
    install_all: Option<usize>,
    /// Whether Install all's menu offers installing for all apps or for Craft apps only.
    destinations: bool,
    removal: Removal,
    store: Vec<FaceInfo>,
    preview: Option<Pending>,
    size_line: Option<String>,
    system_fonts: Option<SystemFonts>,
    preview_note: Option<String>,
    uncurated: Option<String>,
    failure: Option<Failed>,
    lines: Vec<StyleLine>,
    /// The family's files, by position, whose styles the preview service draws.
    subset_files: Vec<usize>,
    /// The variable files among them, with the sliders of their axes.
    axes: Vec<FileAxes>,
    facts: Vec<(Label, String)>,
    languages: Vec<String>,
    /// The web page of the family's own project.
    upstream: Option<String>,
    legal: Vec<(LegalText, LegalShown)>,
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
    /// The sheet closed in this pass: Escape, its close action, a press outside it or a family that is gone.
    pub closed: bool,
}

impl Outcome for DetailsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The sheet of the family `panel.detail` names: a dialog on the modal preset, closed by Escape, its close action or a
/// press outside it, which reaches nothing below.
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
        panel.read_notice(host.as_ref());
        let Some(sheet) = Sheet::read(panel, &env, &words) else {
            panel.detail = None;
            return DetailsOutput { response: ui.response(), closed: true };
        };
        let points = panel.settings.size.max(MIN_SAMPLE_POINTS);
        let mut todo: Vec<Do> = Vec::new();
        let mut open = true;
        // The sheet is for reading: no field of it takes the keyboard when it opens.
        let mut shown = Dialog::prompt(sheet.title.clone())
            .initial_focus(Initial::Keep)
            .overlay(|overlay| {
                let dismiss = overlay.dismiss.outside(Outside::Absorb);
                overlay.dismiss(dismiss)
            })
            .opened(Bind::bound(&mut open))
            .show(ui, |ui, _| {
                Stack::new().fill_width().show(ui, |ui, _| {
                    sheet.header(ui, &env, &words, &mut todo);
                    sheet.body(ui, &env, &words, points, &mut panel.detail_axes, &mut todo);
                });
            });
        let index = panel.index();
        let family = sheet.index.zip(index.as_deref()).and_then(|(i, index)| Some((index, index.families().get(i)?, sheet.sample.as_str())));
        if !craft_ui::runtime::frame::will_rerun(ui.ctx()) {
            shown.settle(|_, ()| panel.subsets.request_detail(family, &sheet.subset_files));
        }
        let closed = shown.output.closed.is_some();
        if closed {
            panel.detail = None;
        }
        for action in todo {
            match action {
                Do::Ask(pending) => panel.ask(host.as_ref(), pending),
                Do::Remove { faces, whole } => panel.remove(host.as_ref(), &sheet.title, &faces, whole),
                Do::ReadLegal(which) => {
                    if let Some(i) = sheet.index {
                        panel.load_legal(host.as_ref(), i, which);
                    }
                }
                Do::AvailableToAll { files, on } => panel.set_available_to_all(host.as_ref(), &sheet.title, files, on),
            }
        }
        DetailsOutput { response: shown.output.response, closed }
    }
}

impl Sheet {
    /// The sheet of the panel's open detail; `None` when there is none or its family is gone.
    fn read(panel: &FontsPanel, env: &Env, words: &DetailsWords) -> Option<Sheet> {
        match panel.detail.clone()? {
            Detail::Family(i) => Sheet::family(panel, env, words, i),
            Detail::Installed(name, source) => Sheet::installed(panel, env, words, &name, source),
        }
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
        let hero = regular(&installed).cloned().map(|f| HeldFace::local(Owner::Installed, f, &panel.catalog));
        let hero = hero.or_else(|| previewed.as_deref().and_then(regular).cloned().map(|f| HeldFace::local(Owner::Preview, f, &panel.previews)));
        let size = env.units().file_size(family.size());
        let all_installed =
            !installed.is_empty() && (on_system || family.files.iter().all(|f| store.iter().any(|x| file_name(x).as_deref() == Some(f.file_name()))));
        let run = match &job {
            Some((kind, None, fraction)) => Some(Run { kind: *kind, fraction: *fraction }),
            _ => None,
        };
        let installed_word = if on_system { &words.installed_on_system } else { &words.installed };
        let sample = panel.family_sample(&index, &family);
        let unseen = installed.is_empty() && previewed.is_none() && !panel.subsets.draws_detail(&index, &family, &sample);
        let uncurated = (!curated).then(|| index.source_of(&family).and_then(|f| f.user_url.clone()).unwrap_or_default());
        let failure = panel.failed.get(&key).map(|f| Failed { what: f.kind, error: f.error.clone(), retry: f.retry.clone() });
        let styles = FamilyStyles {
            panel,
            names: &names,
            words,
            index: &index,
            family: &family,
            sample: &sample,
            installed: &installed,
            store: &store,
            on_system,
            previewed: previewed.as_deref(),
            job: &job,
        };
        let (lines, subset_files) = styles.lines();
        let axes = styles.axes(&subset_files);
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
            install_all: (!all_installed).then_some(i),
            destinations: panel.installer.available_to_all_supported(),
            removal: if store.is_empty() { Removal::None } else { Removal::Allowed },
            store,
            preview: unseen.then_some(Pending::Preview(i)),
            size_line: Some(words.download_size_line.fill(&[("size", &size)])),
            system_fonts: SystemFonts::read(panel, words, &family.family, &installed),
            preview_note: unseen.then(|| words.preview_note.fill(&[("size", &size)])),
            uncurated,
            failure,
            lines,
            subset_files,
            axes,
            facts,
            languages: languages.into_iter().map(|l| l.name.clone()).collect(),
            upstream: family.upstream.clone().filter(|url| url.starts_with("https://") || url.starts_with("http://")),
            legal: legal_texts(panel, &family),
            index: Some(i),
            sample,
        })
    }

    fn installed(panel: &FontsPanel, env: &Env, words: &DetailsWords, name: &str, source: Source) -> Option<Sheet> {
        let family = panel.installed.iter().find(|f| f.name == name && f.source == source).cloned()?;
        let names = env.words::<FontsPanelWords>();
        let hero = regular(&family.faces).cloned().map(|f| HeldFace::local(Owner::Installed, f, &panel.catalog));
        let job = panel.jobs.of("", &family.name).map(|j| (j.kind, j.file.clone(), j.fraction()));
        let run = match &job {
            Some((Kind::Remove, None, _)) => Some(Run { kind: Kind::Remove, fraction: None }),
            _ => None,
        };
        let lines = family
            .faces
            .iter()
            .map(|face| {
                let action = if face.source == Source::Store { Action::Remove(Box::new(face.clone())) } else { Action::None };
                StyleLine::new(style_name(&names, face), Some(HeldFace::local(Owner::Installed, face.clone(), &panel.catalog)), action)
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
        Some(Sheet {
            eyebrow: source_name(&names, source).as_str().to_owned(),
            title: family.name.clone(),
            byline: None,
            hero,
            run,
            badge: None,
            install_all: None,
            destinations: false,
            removal: if family.removable() { Removal::Allowed } else { Removal::System },
            store: family.faces.clone(),
            preview: None,
            size_line: None,
            system_fonts: SystemFonts::read(panel, words, &family.name, &family.faces),
            preview_note: None,
            uncurated: None,
            failure: None,
            lines,
            subset_files: Vec::new(),
            axes: Vec::new(),
            facts,
            languages: Vec::new(),
            upstream: None,
            legal: Vec::new(),
            index: None,
            sample: panel.installed_sample(&family.name),
        })
    }

    /// The eyebrow, the "Aa" in the family's face, the byline, and the family's install and remove controls.
    fn header(&self, ui: &mut Ui, env: &Env, words: &DetailsWords, todo: &mut Vec<Do>) {
        Text::new(self.eyebrow.clone()).shade(Shade::Faint).show(ui);
        if let Some(face) = &self.hero {
            FontSample::new(format!("{}: Aa", self.title), "Aa", HeldFace::sample(Some(face)), HERO_POINTS).show(ui);
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
                    Kind::SystemFonts => &words.updating_system_fonts,
                };
                progress(ui, env, words, what, run.fraction);
                return;
            }
            if let Some(badge) = &self.badge {
                Badge::new(badge.clone()).variant(Variant::look(Emphasis::Soft, Tone::Good)).show(ui);
            }
            if let Some(i) = self.install_all
                && let Some(share) = self.install_button(ui, words)
            {
                todo.push(Do::Ask(Pending::Install(i, share)));
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
        if let Some(system_fonts) = &self.system_fonts {
            system_fonts.show(ui, words, todo);
        }
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
                Kind::SystemFonts => &words.couldnt_update_system_fonts,
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

    /// Install all, with a menu that chooses where the fonts go when they can be copied into the OS user font folder.
    /// `None` when not pressed; `Some(None)` for the button itself (the family's choice, else the setting), else the
    /// choice.
    fn install_button(&self, ui: &mut Ui, words: &DetailsWords) -> Option<Option<bool>> {
        if !self.destinations {
            return clicked(&Button::new(words.install_all.clone()).variant(Variant::PRIMARY).show(ui)).then_some(None);
        }
        let entries = [Entry::new(true, words.install_all_apps.clone()), Entry::new(false, words.install_craft_apps.clone())];
        let out =
            SplitButton::new(words.install_all.clone(), &entries).arrow_label(words.install_all_options.clone()).variant(Variant::PRIMARY).show(ui);
        out.chosen.map(Some).or_else(|| out.activated.then_some(None))
    }

    /// The styles, the axis sliders of each variable file under its name, the facts, the languages and the licence.
    fn body(&self, ui: &mut Ui, env: &Env, words: &DetailsWords, points: f32, axes: &mut HashMap<String, AxisValues>, todo: &mut Vec<Do>) {
        self.styles(ui, env, words, points, todo);
        for file in &self.axes {
            Text::new(file.label.clone()).shade(Shade::Faint).show(ui);
            let values = axes.entry(file.sha256.clone()).or_default();
            AxisSliders::new(file.label.clone(), &file.axes, Bind::bound(values)).optical_size(OPTICAL_SIZE).show(ui);
        }
        self.side(ui, env, words);
        self.licence(ui, words, todo);
    }

    fn styles(&self, ui: &mut Ui, env: &Env, words: &DetailsWords, points: f32, todo: &mut Vec<Do>) {
        let total = self.lines.iter().filter(|line| !line.sliders).count();
        let installed = self.lines.iter().filter(|line| line.face.as_ref().is_some_and(HeldFace::installed)).count();
        let count = env.format().integer(total as u64);
        let counted = if installed == total { &words.styles_installed } else { &words.styles };
        Text::new(counted.text(total as u64, &count)).shade(Shade::Faint).show(ui);
        let source = Lines(&self.lines);
        let out = StyleList::new(self.title.clone(), words.no_styles.clone())
            .show_items(ui, &source, &mut |key, _| StyleRow::new(&self.lines[*key], &self.title, &self.sample, points, words));
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

impl Sheet {
    /// The project's page, and the licence and NOTICE texts: each a button that reads it, then the text, or why it
    /// couldn't be read.
    fn licence(&self, ui: &mut Ui, words: &DetailsWords, todo: &mut Vec<Do>) {
        if let Some(url) = &self.upstream {
            let shown = url.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/');
            Link::new(shown.to_owned()).url(url.clone()).wrap(Wrap::Elide).description(words.upstream.fill(&[("url", url)])).show(ui);
        }
        for (at, (which, shown)) in self.legal.iter().enumerate() {
            let (show, title) = match which {
                LegalText::License => (&words.show_licence, &words.licence_text),
                LegalText::Notice => (&words.show_notice, &words.notice_text),
            };
            let read = match shown {
                LegalShown::Offer => clicked(&Button::new(show.clone()).variant(Variant::SECONDARY).show(ui)),
                LegalShown::Loading => {
                    Text::new(words.loading_text.clone()).shade(Shade::Muted).show(ui);
                    false
                }
                LegalShown::Text(text) => {
                    Text::new(title.clone()).shade(Shade::Faint).show(ui);
                    TextField::new(title.clone(), Bind::given(text.clone()))
                        .lines(LEGAL_LEAST_LINES)
                        .max_lines(LEGAL_LINES)
                        .sizing(Sizing::FillWidth)
                        .mono()
                        .read_only(true)
                        .show(ui);
                    false
                }
                LegalShown::Failed(error) => {
                    let retry = NoticeAction::new(words.retry.clone());
                    Notice::new(Tone::Bad, error.clone()).id_salt(("legal", at)).title(title.clone()).action(retry).show(ui).action.is_some()
                }
            };
            if read {
                todo.push(Do::ReadLegal(*which));
            }
        }
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

/// The styles of a family of the index, as its sheet lists them.
struct FamilyStyles<'a> {
    panel: &'a FontsPanel,
    names: &'a FontsPanelWords,
    words: &'a DetailsWords,
    index: &'a Index,
    family: &'a Family,
    sample: &'a str,
    installed: &'a [FaceInfo],
    store: &'a [FaceInfo],
    on_system: bool,
    previewed: Option<&'a [FaceInfo]>,
    job: &'a Option<(Kind, Option<String>, Option<f32>)>,
}

impl FamilyStyles<'_> {
    /// The rows: the installed faces, then the index files not installed, as the previewed faces when the preview has
    /// them, else as the preview service draws them; and the files, by position, of the last kind.
    fn lines(&self) -> (Vec<StyleLine>, Vec<usize>) {
        let mut lines = Vec::new();
        let mut subset_files = Vec::new();
        for face in self.installed {
            let action = match self.job {
                Some((Kind::Remove, Some(p), _)) if face.path.as_ref().is_some_and(|x| x.display().to_string() == *p) => Action::Running(None),
                _ if face.source == Source::Store => Action::Remove(Box::new(face.clone())),
                _ => Action::None,
            };
            lines.push(self.line(Owner::Installed, face, action));
        }
        if self.on_system {
            return (lines, subset_files);
        }
        for (n, file) in self.family.files.iter().enumerate() {
            if self.store.iter().any(|x| file_name(x).as_deref() == Some(file.file_name())) {
                continue;
            }
            let preview_faces: Vec<&FaceInfo> =
                self.previewed.unwrap_or_default().iter().filter(|f| file_of(self.family, f).is_some_and(|x| x.path == file.path)).collect();
            let action = match self.job {
                Some((Kind::Install, Some(p), fraction)) if *p == file.path => Action::Running(*fraction),
                _ => Action::Install(file.path.clone()),
            };
            if !preview_faces.is_empty() {
                // A variable file's instances install together: the file's action on its first.
                let actions = std::iter::once(action).chain(std::iter::repeat(Action::None));
                lines.extend(preview_faces.into_iter().zip(actions).map(|(face, action)| self.line(Owner::Preview, face, action)));
            } else if self.panel.subsets.serves(self.family) {
                subset_files.push(n);
                self.subset_lines(file, action, &mut lines);
            } else {
                lines.push(StyleLine::new(file_label(file), None, action));
            }
        }
        (lines, subset_files)
    }

    fn line(&self, owner: Owner, face: &FaceInfo, action: Action) -> StyleLine {
        let catalog = match owner {
            Owner::Installed => &self.panel.catalog,
            Owner::Preview => &self.panel.previews,
        };
        StyleLine::new(style_name(self.names, face), Some(HeldFace::local(owner, face.clone(), catalog)), action)
    }

    /// The rows of a file drawn by the preview service: a static file's one, or a variable file's named instances and
    /// the row that follows its axis sliders, all at coordinates of one subset that keeps every axis.
    fn subset_lines(&self, file: &FontFile, action: Action, lines: &mut Vec<StyleLine>) {
        let face = self.subset(file).map(HeldFace::Subset);
        if !file.is_variable() {
            lines.push(StyleLine::new(file_label(file), face, action));
            return;
        }
        // A variable file installs whole: the file's action on its first row.
        if file.instances.is_empty() {
            lines.push(StyleLine::new(self.words.default_style.as_str(), face.clone(), action));
        } else {
            let actions = std::iter::once(action).chain(std::iter::repeat(Action::None));
            lines.extend(file.instances.iter().zip(actions).map(|(instance, action)| {
                let coords = instance.coordinates.iter().filter_map(|(tag, v)| Some((axis_tag(tag)?, *v))).collect();
                StyleLine::new(instance.name.clone(), face.clone(), action).at(coords)
            }));
        }
        let values = self.panel.detail_axes.get(&file.sha256).cloned().unwrap_or_default();
        lines.push(StyleLine { sliders: true, ..StyleLine::new(file_label(file), face, Action::None).at(values) });
    }

    fn subset(&self, file: &FontFile) -> Option<SubsetFont> {
        self.panel.subsets.detail_font(self.index, self.family, file, self.sample)
    }

    /// The axes of the variable files among `files` that have a slider: the subset's own once it is here, else the index's
    /// ranges.
    fn axes(&self, files: &[usize]) -> Vec<FileAxes> {
        files
            .iter()
            .filter_map(|n| self.family.files.get(*n))
            .filter(|file| file.is_variable())
            .map(|file| {
                let own = self.subset(file).map(|font| read_axes(&font.bytes, 0)).filter(|axes| !axes.is_empty());
                FileAxes { sha256: file.sha256.clone(), label: file_label(file), axes: own.unwrap_or_else(|| index_axes(file)) }
            })
            .filter(|file| shown_axes(&file.axes, OPTICAL_SIZE).next().is_some())
            .collect()
    }
}

/// `tag` as an OpenType tag: four printable ASCII characters, spaces only at the end.
fn axis_tag(tag: &str) -> Option<[u8; 4]> {
    let tag: [u8; 4] = tag.as_bytes().try_into().ok()?;
    let printable = tag.iter().all(|b| (0x20..=0x7e).contains(b));
    let spaces_last = tag.iter().skip_while(|b| **b != b' ').all(|b| *b == b' ');
    (printable && spaces_last && tag[0] != b' ').then_some(tag)
}

/// A variable file's axes as the index gives them: its ranges about the file's default style.
fn index_axes(file: &FontFile) -> Vec<FontAxis> {
    file.axes
        .iter()
        .filter(|axis| axis.min < axis.max)
        .filter_map(|axis| {
            let tag = axis_tag(&axis.tag)?;
            let default = match &tag {
                b"wght" => f32::from(file.weight),
                b"wdth" => file.width,
                b"ital" => f32::from(u8::from(file.is_italic())),
                _ => 0.0,
            };
            Some(FontAxis::new(tag, "", axis.min, default.clamp(axis.min, axis.max), axis.max))
        })
        .collect()
}

fn family_facts(env: &Env, words: &DetailsWords, index: &Index, family: &Family, size: &str) -> Vec<(Label, String)> {
    let scripts: Vec<&str> = family.scripts.iter().map(|id| script_name(index, id).unwrap_or(id)).collect();
    let format = env.format();
    let mut facts: Vec<(Label, String)> = Vec::new();
    if !family.designer.is_empty() {
        facts.push((words.designer.clone(), family.designer.clone()));
    }
    facts.push((words.licence.clone(), licence_name(&family.license).to_owned()));
    if let Some(copyright) = family.copyright.as_deref().map(str::trim).filter(|text| !text.is_empty()) {
        facts.push((words.copyright.clone(), copyright.to_owned()));
    }
    facts.push((words.styles_fact.clone(), format.integer(style_count(family) as u64)));
    facts.push((words.files_fact.clone(), format.integer(family.files.len() as u64)));
    facts.push((words.download_size.clone(), size.to_owned()));
    if !scripts.is_empty() {
        facts.push((words.scripts.clone(), scripts.join(", ")));
    }
    facts
}
