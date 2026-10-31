//! The panel's state, what the host calls, and the frame: the tabs and search across the top,
//! the filters at the reading start, the family cards, and the details sheet over them.

use std::collections::HashMap;
use std::sync::Arc;

use crate::craft_font_catalog::{FontEnv, Limits, Os};
use crate::filter::{Query, Search};
use crate::index::{Category, Family, FontFile, Index, categories, source_label};
use crate::installed::{InstalledFamily, group};
use crate::remote::Previews;
use crate::store::{Installer, LegalText, RemoveRequest, StoreError};
use crate::{Catalog, FaceInfo, Source};
use craft_i18n::Folder;
use craft_ui::direction;
use craft_ui::fonts::{CraftFace, I18nScripts, UiFonts};
use egui::{CentralPanel, Context, Frame, Margin, Panel, Ui};

use crate::panel::details::Legal;
use crate::panel::faces::FaceFonts;
use crate::panel::i18n::{self, I18N, LANGUAGES, fmt, tl};
use crate::panel::jobs::{Finished, Jobs, Kind, Request, Runner};
use crate::panel::look::{Look, Palette};
use crate::panel::samples;
use crate::panel::subsets::Subsets;
use crate::panel::widgets::{self, Skin};

/// Installs the standalone app's interface fonts in `ctx` and sets the interface direction: craft-ui's
/// font stack for the panel's interface language, with `craft_faces` (craft-fonts' faces the app
/// embeds with craft-fonts-build, or none) and the system's fonts for every other character. A host
/// with its own fonts doesn't call it.
pub fn install_interface_fonts(ctx: &Context, craft_faces: &[CraftFace]) {
    let language = I18N.code(i18n::current()).to_owned();
    UiFonts::new(&I18nScripts).languages(LANGUAGES).ui_language(&language).craft_faces(craft_faces).install(ctx);
    direction::apply(ctx, direction::resolve(&language, direction::Setting::Language));
}

/// Preview downloads: at most this much per file, and this many faces in a session.
const PREVIEW_LIMITS: Limits = Limits { max_file_bytes: 64 << 20, max_faces_per_file: 512, max_added_faces: 20_000 };

/// How often the download consent is read again from the store's settings file, which another
/// Craft app can change.
const CONSENT_RECHECK_SECS: f64 = 3.0;

/// What a sample shows when the user hasn't typed one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Preset {
    /// A sentence in the language filtered by, or in the family's own script.
    #[default]
    Sentence,
    Alphabet,
    Numerals,
}

/// What the app keeps between runs.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The sample text; empty for the preset.
    pub sample: String,
    pub preset: Preset,
    pub size: f32,
    /// The user closed the note that downloads are off.
    pub hide_download_note: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { sample: String::new(), preset: Preset::Sentence, size: 36.0, hide_download_note: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Installed,
    Catalogue,
}

/// Which catalogue families show by install state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum Show {
    #[default]
    All,
    Installed,
    NotInstalled,
}

/// Which installed families show by where they come from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Origin {
    #[default]
    All,
    System,
    Store,
}

/// What the Installed tab's list was worked out for: the search, the source and the catalog's
/// generation.
type InstalledFilter = (String, Origin, Option<u64>);

/// What the app is built from; tests pass their own catalog, store and index.
pub struct Options {
    pub catalog: Arc<Catalog>,
    pub installer: Arc<dyn Installer>,
    /// The Google Fonts index, or why it couldn't be read.
    pub index: Result<Index, String>,
    pub runner: Runner,
}

/// A download, waiting for the user's consent or to be tried again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Pending {
    Install(usize),
    /// One file of a family, by its path in the index.
    InstallFile(usize, String),
    Preview(usize),
}

impl Pending {
    pub(crate) fn family(&self) -> usize {
        match self {
            Pending::Install(i) | Pending::InstallFile(i, _) | Pending::Preview(i) => *i,
        }
    }
}

/// The family the details sheet shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Detail {
    /// A family of the index, by position.
    Family(usize),
    /// An installed family, by name and source.
    Installed(String, Source),
}

/// Why the last job for a family failed, and what Retry runs.
#[derive(Clone, Debug)]
pub(crate) struct Failure {
    pub kind: Kind,
    pub error: String,
    pub retry: Option<Pending>,
}

pub(crate) struct Catalogue {
    pub index: Arc<Index>,
    pub search: Search,
}

pub struct FontsPanel {
    pub(crate) settings: Settings,
    pub(crate) tab: Tab,
    pub(crate) look: Look,
    pub(crate) skin: Skin,
    /// The panel shows its own download switch; off when the host has one in its settings.
    pub(crate) downloads_switch: bool,
    pub(crate) catalog: Arc<Catalog>,
    pub(crate) installer: Arc<dyn Installer>,
    pub(crate) catalogue: Result<Catalogue, String>,
    pub(crate) query: Query,
    pub(crate) show: Show,
    pub(crate) results: Vec<usize>,
    results_for: Option<(Query, Show, Option<String>, Option<u64>)>,
    /// Only this source's families (a source ID).
    pub(crate) source: Option<String>,
    /// Families per source, in [`Index::sources`] order, for the other filters as set.
    pub(crate) source_counts: Vec<usize>,
    /// The indexes being downloaded again (the sources' Update).
    sources_job: Option<std::sync::mpsc::Receiver<Result<Index, StoreError>>>,
    /// What the Sources settings show: the URL being typed, and the last problem.
    pub(crate) source_url: String,
    pub(crate) source_message: Option<(bool, String)>,
    /// Families per category (all first) and per install state, for the other filters as set.
    pub(crate) category_counts: [usize; 6],
    pub(crate) show_counts: [usize; 3],
    pub(crate) language_text: String,
    pub(crate) installed: Vec<InstalledFamily>,
    /// Installed faces by lower-case family name, every source together.
    pub(crate) installed_by_name: HashMap<String, Vec<FaceInfo>>,
    installed_generation: Option<u64>,
    pub(crate) installed_text: String,
    pub(crate) origin: Origin,
    /// [`FontsPanel::installed_shown`] and what it was worked out for.
    installed_shown: (Vec<usize>, Option<InstalledFilter>),
    pub(crate) detail: Option<Detail>,
    /// Downloaded preview files, never installed.
    pub(crate) previews: Arc<Catalog>,
    pub(crate) previewed: HashMap<String, Vec<FaceInfo>>,
    /// The list's previews from the font preview service.
    pub(crate) subsets: Subsets,
    pub(crate) faces: FaceFonts,
    /// The detail view's axis sliders, by file sha256 and axis tag.
    pub(crate) detail_axes: HashMap<(String, [u8; 4]), f32>,
    /// Licence and NOTICE texts, by family key.
    pub(crate) legal: HashMap<(String, LegalText), Legal>,
    pub(crate) jobs: Jobs,
    pub(crate) asking: Option<Pending>,
    /// By lower-case family name.
    pub(crate) failed: HashMap<String, Failure>,
    /// A problem that belongs to no family (the download setting couldn't be saved).
    pub(crate) notice: Option<String>,
    focused: bool,
    pub(crate) folder: Folder,
    /// The download consent as last read, and when.
    consent: Option<(bool, f64)>,
    /// The card that has (or is to get) the keyboard focus, by row.
    pub(crate) focus_row: Option<usize>,
    /// Bring the focused row into view on the next frame.
    pub(crate) scroll_to_focus: bool,
    /// Give the focus row the keyboard focus on the next frame.
    pub(crate) move_focus: bool,
    /// Focus the search field on the next frame.
    pub(crate) focus_search: bool,
}

/// A family's key in the panel's state: families of the same name can come from two sources.
/// A removal goes by name alone (`source` empty).
pub(crate) fn family_key(source: &str, family: &str) -> String {
    format!("{source}\n{}", family.to_lowercase())
}

/// The host of an https URL.
fn url_host(url: &str) -> Option<&str> {
    url.strip_prefix("https://")?.split(['/', '?', '#']).next().filter(|h| !h.is_empty())
}

pub(crate) fn category_name(c: Category) -> &'static str {
    match c {
        Category::SansSerif => tl!("Sans serif"),
        Category::Serif => tl!("Serif"),
        Category::Display => tl!("Display"),
        Category::Handwriting => tl!("Handwriting"),
        Category::Monospace => tl!("Monospace"),
    }
}

pub(crate) fn source_name(s: Source) -> &'static str {
    match s {
        Source::Store => tl!("Font store"),
        _ => tl!("System"),
    }
}

/// The file of the face the list shows for a family of the index: the one closest to an upright
/// regular.
pub(crate) fn list_file(family: &Family) -> Option<&FontFile> {
    family.files.iter().min_by_key(|f| (f.is_italic(), !f.has_weight(400, f.is_italic()), f.weight.abs_diff(400), !f.has_normal_width()))
}

/// The face closest to an upright regular.
pub(crate) fn regular(faces: &[FaceInfo]) -> Option<&FaceInfo> {
    faces.iter().min_by_key(|f| (f.italic, ((f.weight - 400.0).abs() * 10.0) as i64, ((f.width - 100.0).abs() * 10.0) as i64))
}

pub(crate) fn style_name(face: &FaceInfo) -> &str {
    if face.style.is_empty() { tl!("Regular") } else { &face.style }
}

/// How many styles a family of the index has: each static file one, a variable file one per
/// hundred of weight its `wght` axis covers.
pub(crate) fn style_count(family: &Family) -> usize {
    family
        .files
        .iter()
        .map(|f| match f.axes.iter().find(|a| a.tag == "wght") {
            Some(a) => ((a.min / 100.0).ceil() as usize..=(a.max / 100.0).floor() as usize).count().max(1),
            None => 1,
        })
        .sum()
}

pub(crate) fn category_label(family: &Family) -> String {
    categories(family).map(category_name).collect::<Vec<_>>().join(", ")
}

impl FontsPanel {
    pub fn new(options: Options, settings: Settings) -> FontsPanel {
        let catalogue = options.index.map(|index| Catalogue { search: Search::new(&index), index: Arc::new(index) });
        let previews = Arc::new(Catalog::new(FontEnv { os: Os::None, ..FontEnv::default() }).with_font_service(false));
        let look = Look::default();
        let subsets = Subsets::new(Arc::clone(&options.installer), Previews::new(Arc::clone(&options.installer)));
        FontsPanel {
            settings,
            tab: Tab::Catalogue,
            skin: Skin { p: Palette::from_visuals(&egui::Visuals::dark()), look: look.clone() },
            look,
            downloads_switch: true,
            catalog: options.catalog,
            installer: options.installer,
            catalogue,
            query: Query::default(),
            show: Show::All,
            results: Vec::new(),
            results_for: None,
            source: None,
            source_counts: Vec::new(),
            sources_job: None,
            source_url: String::new(),
            source_message: None,
            category_counts: [0; 6],
            show_counts: [0; 3],
            language_text: String::new(),
            installed: Vec::new(),
            installed_by_name: HashMap::new(),
            installed_generation: None,
            installed_text: String::new(),
            origin: Origin::All,
            installed_shown: (Vec::new(), None),
            detail: None,
            previews,
            previewed: HashMap::new(),
            subsets,
            faces: FaceFonts::default(),
            detail_axes: HashMap::new(),
            legal: HashMap::new(),
            jobs: Jobs::new(options.runner),
            asking: None,
            failed: HashMap::new(),
            notice: None,
            focused: true,
            folder: Folder::new("en"),
            consent: None,
            focus_row: None,
            scroll_to_focus: false,
            move_focus: false,
            focus_search: false,
        }
    }

    /// Fetches the list's previews through `previews` instead of craft-font-store's client.
    pub fn with_previews(mut self, previews: Previews) -> FontsPanel {
        self.subsets.replace(previews);
        self
    }

    /// Reads the preview service and previews settings again on the next frame, after a host's
    /// settings page changed them. The panel also reads them every few seconds and when its
    /// window regains focus.
    pub fn reload_preview_settings(&mut self) {
        self.subsets.read_settings_soon();
    }

    /// Whether previews from the font preview service are being fetched.
    pub fn loading_previews(&self) -> bool {
        self.subsets.loading()
    }

    /// Shows `index` (after the host fetched the sources, say). The details sheet closes, as
    /// family positions change.
    pub fn set_index(&mut self, index: Result<Index, String>) {
        self.catalogue = index.map(|index| Catalogue { search: Search::new(&index), index: Arc::new(index) });
        self.results_for = None;
        self.detail = None;
        self.asking = None;
        self.focus_row = None;
        if let Ok(c) = &self.catalogue
            && self.source.as_ref().is_some_and(|s| c.index.source(s).is_none())
        {
            self.source = None;
        }
    }

    /// Reads the indexes again without a request: after a source was added or removed.
    pub(crate) fn reload_index(&mut self) {
        let index = self.installer.indexes().map_err(|e| e.0);
        self.set_index(index);
    }

    /// Downloads every source's index again on a worker thread, when downloads are allowed.
    pub(crate) fn fetch_sources(&mut self, ctx: &Context) {
        if self.sources_job.is_some() || !self.downloads_allowed() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let installer = Arc::clone(&self.installer);
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("font-sources".into()).spawn(move || {
            let _ = tx.send(installer.fetch_indexes());
            ctx.request_repaint();
        });
        match spawned {
            Ok(_) => self.sources_job = Some(rx),
            Err(e) => self.source_message = Some((true, e.to_string())),
        }
    }

    /// Whether the sources' indexes are being downloaded again.
    pub fn fetching_sources(&self) -> bool {
        self.sources_job.is_some()
    }

    pub(crate) fn poll_sources(&mut self) {
        let Some(rx) = &self.sources_job else { return };
        match rx.try_recv() {
            Ok(Ok(index)) => {
                self.sources_job = None;
                self.source_message = None;
                self.set_index(Ok(index));
            }
            Ok(Err(e)) => {
                self.sources_job = None;
                self.source_message = Some((true, e.0));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.sources_job = None,
        }
    }

    /// Draws with the host's colours and font families.
    pub fn set_look(&mut self, look: Look) {
        self.look = look;
    }

    /// Whether the panel shows its own "Allow downloads" switch. A host with the switch in its
    /// own settings turns it off; the panel still offers to allow downloads where one is needed.
    pub fn show_downloads_switch(&mut self, show: bool) {
        self.downloads_switch = show;
    }

    /// The panel's own settings, for the host to save.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn settings_mut(&mut self) -> &mut Settings {
        &mut self.settings
    }

    pub fn tab(&self) -> Tab {
        self.tab
    }

    pub fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.focus_row = None;
    }

    /// The installed families, as the Installed tab lists them.
    pub fn installed(&self) -> &[InstalledFamily] {
        &self.installed
    }

    /// The catalogue's families for the current search and filters.
    pub fn results(&self) -> Vec<String> {
        let Ok(c) = &self.catalogue else { return Vec::new() };
        self.results.iter().filter_map(|&i| c.index.families().get(i)).map(|f| f.family.clone()).collect()
    }

    /// The family the details sheet shows.
    pub fn details_family(&self) -> Option<String> {
        match &self.detail {
            Some(Detail::Family(i)) => self.catalogue.as_ref().ok()?.index.families().get(*i).map(|f| f.family.clone()),
            Some(Detail::Installed(name, _)) => Some(name.clone()),
            None => None,
        }
    }

    /// Shows the interface in `language` (a tag such as `de` or `pt-BR`), the host's interface
    /// language, when its catalog is embedded; English otherwise.
    pub fn set_language(&mut self, language: &str) {
        i18n::set_current(I18N.match_tag(language).unwrap_or(craft_i18n::Lang::EN));
    }

    /// Has the user allowed downloads from Google Fonts? A suite-wide setting of the font store,
    /// off until they turn it on or confirm an install or preview.
    pub fn downloads_allowed(&self) -> bool {
        self.installer.downloads_allowed()
    }

    /// Turns downloads on or off, as the user's own action (a host's settings page, say). A
    /// failure also shows in the panel.
    pub fn set_downloads_allowed(&mut self, allowed: bool) -> Result<(), StoreError> {
        let result = self.installer.allow_downloads(allowed);
        self.consent = None;
        self.notice = result.as_ref().err().map(|e| fmt("The download setting couldn't be saved: {error}", &[("error", &e.0)]));
        result
    }

    /// The consent, read from the store at most every few seconds.
    pub(crate) fn allowed(&mut self, now: f64) -> bool {
        match self.consent {
            Some((allowed, at)) if (0.0..CONSENT_RECHECK_SECS).contains(&(now - at)) => allowed,
            _ => {
                let allowed = self.installer.downloads_allowed();
                self.consent = Some((allowed, now));
                allowed
            }
        }
    }

    /// Draws the panel into `ui`, which it fills.
    pub fn ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.skin = Skin { p: self.look.palette(&ctx), look: self.look.clone() };
        self.faces.begin_frame(&ctx);
        let typed = format!("{:?}\n{:?}\n{}", self.settings.preset, self.query.language, self.settings.sample);
        self.subsets.begin_frame(&ctx, typed);
        self.poll_jobs();
        let focused = ctx.input(|i| i.focused);
        if focused && !self.focused {
            if self.catalog.is_ready() {
                self.catalog.rescan_if_changed();
            }
            self.consent = None;
            self.subsets.read_settings_soon();
        }
        self.focused = focused;
        self.refresh_installed();
        self.refresh_results();
        self.keys(&ctx);

        let area = ui.max_rect();
        let p = self.skin.p;
        let rtl = direction::is_rtl(ui);
        let sides = |start: i8, end: i8| if rtl { (end, start) } else { (start, end) };
        let top = Frame::new().fill(p.bg).inner_margin(Margin { left: 20, right: 20, top: 16, bottom: 14 });
        Panel::top("craft-fonts.top").frame(top).resizable(false).show(ui, |ui| self.top_bar(ui));
        let width = if area.width() < 860.0 { 212.0 } else { 248.0 };
        let (left, right) = sides(20, 18);
        let filters = Frame::new().fill(p.bg).inner_margin(Margin { left, right, top: 18, bottom: 12 });
        direction::start_panel(ui, "craft-fonts.filters").frame(filters).resizable(false).exact_size(width).show(ui, |ui| self.sidebar(ui));
        let main = Frame::new().fill(p.bg).inner_margin(Margin { left: 20, right: 20, top: 16, bottom: 0 });
        CentralPanel::default().frame(main).show(ui, |ui| self.main(ui));
        self.details(&ctx, area);
        self.consent(&ctx);
    }

    fn keys(&mut self, ctx: &Context) {
        let (find, escape) = ctx.input_mut(|i| (i.consume_key(egui::Modifiers::COMMAND, egui::Key::F), i.key_pressed(egui::Key::Escape)));
        if find {
            self.focus_search = true;
        }
        if escape && self.asking.is_none() && self.detail.is_some() {
            self.detail = None;
        }
    }

    fn poll_jobs(&mut self) {
        for (job, result) in self.jobs.poll() {
            let key = family_key(&job.source, &job.family);
            match result {
                Ok(Finished::Installed | Finished::Removed) => {
                    self.failed.remove(&key);
                }
                Ok(Finished::Previewed(files)) => {
                    self.failed.remove(&key);
                    let mut faces = Vec::new();
                    for file in files {
                        match self.previews.add_bytes(file.bytes, Source::User, &PREVIEW_LIMITS) {
                            Ok(added) => faces.extend(added),
                            Err(e) => log::warn!("preview of {}: {e}", file.path),
                        }
                    }
                    self.previewed.insert(key, faces);
                }
                Err(e) => {
                    let position =
                        self.catalogue.as_ref().ok().and_then(|c| {
                            c.index.families().iter().position(|f| f.source == job.source && f.family.eq_ignore_ascii_case(&job.family))
                        });
                    let retry = position.and_then(|i| match job.kind {
                        Kind::Install => Some(job.file.clone().map_or(Pending::Install(i), |path| Pending::InstallFile(i, path))),
                        Kind::Preview => Some(Pending::Preview(i)),
                        Kind::Remove => None,
                    });
                    log::warn!("{} ({:?}): {e}", job.family, job.kind);
                    self.failed.insert(key, Failure { kind: job.kind, error: e.0, retry });
                }
            }
        }
        self.poll_sources();
    }

    fn refresh_installed(&mut self) {
        if !self.catalog.is_ready() {
            return;
        }
        let generation = self.catalog.generation();
        if self.installed_generation == Some(generation) {
            return;
        }
        self.installed_generation = Some(generation);
        let faces = self.catalog.faces();
        self.installed = group(&faces);
        self.installed_by_name.clear();
        for family in &self.installed {
            self.installed_by_name.entry(family.name.to_lowercase()).or_default().extend(family.faces.iter().cloned());
        }
    }

    fn refresh_results(&mut self) {
        let Ok(c) = &self.catalogue else { return };
        let key = (self.query.clone(), self.show, self.source.clone(), self.installed_generation);
        if self.results_for.as_ref() == Some(&key) {
            return;
        }
        let from = |i: usize, source: &Option<String>| source.as_ref().is_none_or(|s| c.index.families().get(i).is_some_and(|f| &f.source == s));
        let installed = |i: usize| c.index.families().get(i).is_some_and(|f| self.installed_by_name.contains_key(&f.family.to_lowercase()));
        let shows = |show: Show, i: usize| match show {
            Show::All => true,
            Show::Installed => installed(i),
            Show::NotInstalled => !installed(i),
        };
        let any_category = Query { category: None, ..self.query.clone() };
        let mut by_category = [0usize; 6];
        let mut by_state = [0usize; 3];
        let mut by_source: Vec<usize> = vec![0; c.index.sources().len()];
        for i in c.search.families(&c.index, &any_category) {
            let Some(f) = c.index.families().get(i) else { continue };
            let in_category = self.query.category.is_none_or(|q| categories(f).any(|fc| fc == q));
            if in_category
                && shows(self.show, i)
                && let Some(n) = c.index.sources().iter().position(|s| s.id == f.source).and_then(|n| by_source.get_mut(n))
            {
                *n += 1;
            }
            if !from(i, &self.source) {
                continue;
            }
            if shows(self.show, i) {
                by_category[0] += 1;
                for fc in categories(f) {
                    if let Some(pos) = Category::ALL.iter().position(|x| *x == fc) {
                        by_category[pos + 1] += 1;
                    }
                }
            }
            if in_category {
                by_state[0] += 1;
                by_state[if installed(i) { 1 } else { 2 }] += 1;
            }
        }
        self.category_counts = by_category;
        self.show_counts = by_state;
        self.source_counts = by_source;
        self.results = c.search.families(&c.index, &self.query).into_iter().filter(|&i| shows(self.show, i) && from(i, &self.source)).collect();
        self.results_for = Some(key);
    }

    /// The installed families the Installed tab shows, by position in [`FontsPanel::installed`].
    pub(crate) fn installed_shown(&mut self) -> Vec<usize> {
        let key = (self.installed_text.clone(), self.origin, self.installed_generation);
        if self.installed_shown.1.as_ref() != Some(&key) {
            let text = self.folder.key(self.installed_text.trim());
            let shown = (0..self.installed.len())
                .filter(|&i| {
                    let Some(f) = self.installed.get(i) else { return false };
                    let origin = match self.origin {
                        Origin::All => true,
                        Origin::System => f.source != Source::Store,
                        Origin::Store => f.source == Source::Store,
                    };
                    origin && (text.is_empty() || self.folder.key(&f.name).contains(&text))
                })
                .collect();
            self.installed_shown = (shown, Some(key));
        }
        self.installed_shown.0.clone()
    }

    /// The sample for a family with `scripts` and the index sample `own` ([`samples::of_family`]):
    /// what the user typed, else the preset.
    pub(crate) fn sample_for<'a>(&self, own: Option<&str>, scripts: impl IntoIterator<Item = &'a str>) -> String {
        if !self.settings.sample.trim().is_empty() {
            return self.settings.sample.clone();
        }
        match self.settings.preset {
            Preset::Alphabet => samples::ALPHABET.to_owned(),
            Preset::Numerals => samples::NUMERALS.to_owned(),
            Preset::Sentence => {
                let language = self.catalogue.as_ref().ok().zip(self.query.language).and_then(|(c, l)| c.index.languages().get(l));
                language.and_then(|l| samples::for_language(&l.id)).or(own).unwrap_or_else(|| samples::for_scripts(scripts)).to_owned()
            }
        }
    }

    /// Runs `pending` if downloads are allowed, or asks first.
    pub(crate) fn ask(&mut self, ctx: &Context, pending: Pending) {
        let now = ctx.input(|i| i.time);
        if self.allowed(now) {
            self.download(ctx, pending);
        } else {
            self.asking = Some(pending);
        }
    }

    fn download(&mut self, ctx: &Context, pending: Pending) {
        if !self.downloads_allowed() {
            return;
        }
        let Ok(c) = &self.catalogue else { return };
        let index = Arc::clone(&c.index);
        if let Some(f) = index.families().get(pending.family()) {
            self.failed.remove(&family_key(&f.source, &f.family));
        }
        let request = match pending {
            Pending::Install(i) => Request::Install(index, i),
            Pending::InstallFile(i, path) => Request::InstallFile(index, i, path),
            Pending::Preview(i) => Request::Preview(index, i),
        };
        self.jobs.start(ctx, Arc::clone(&self.installer), Arc::clone(&self.catalog), request);
    }

    /// Removes store files: a whole family, or the files of some of its styles.
    pub(crate) fn remove(&mut self, ctx: &Context, family: &str, faces: &[FaceInfo], whole: bool) {
        let mut files: Vec<_> = faces.iter().filter(|f| f.source == Source::Store).filter_map(|f| f.path.clone()).collect();
        files.sort();
        files.dedup();
        if files.is_empty() {
            return;
        }
        self.failed.remove(&family_key("", family));
        let request = RemoveRequest { family: family.to_owned(), files, whole };
        self.jobs.start(ctx, Arc::clone(&self.installer), Arc::clone(&self.catalog), Request::Remove(request));
    }

    fn consent(&mut self, ctx: &Context) {
        let Some(pending) = self.asking.clone() else { return };
        let Ok(c) = &self.catalogue else { return };
        let Some(family) = c.index.families().get(pending.family()) else { return };
        let file = match &pending {
            Pending::InstallFile(_, path) => family.files.iter().find(|f| &f.path == path),
            _ => family.files.first(),
        };
        let size = match &pending {
            Pending::InstallFile(..) => file.map_or(0, |f| f.size),
            _ => family.size(),
        };
        let (source, curated) = source_label(&c.index, family);
        // A curated source's files come through its repository's CDN; a source the user added
        // can point anywhere, so the dialog names the server.
        let host = file.filter(|_| !curated).and_then(|f| c.index.file_urls(family, f).first().and_then(|u| url_host(u)).map(str::to_owned));
        let args = [("family", family.family.as_str()), ("size", &i18n::size(size)), ("source", source), ("host", host.as_deref().unwrap_or(""))];
        let text = match &host {
            None => fmt("Craft Font Manager downloads {family} ({size}) from {source}, and checks every file before using it.", &args),
            Some(_) => fmt(
                "Craft Font Manager downloads {family} ({size}) from {host}, the server of {source}, a source you added, and checks every file before using it.",
                &args,
            ),
        };
        let title = fmt("Download from {source}?", &[("source", source)]);
        let s = self.skin.clone();
        let mut allow = false;
        let mut cancel = false;
        let frame =
            Frame::new().fill(s.p.bg).stroke(egui::Stroke::new(1.0, s.p.line_strong)).corner_radius(s.look.corner).inner_margin(Margin::same(24));
        let response = egui::Modal::new(egui::Id::unique("craft-font-manager.consent")).frame(frame).show(ctx, |ui| {
            ui.set_max_width(440.0);
            ui.spacing_mut().item_spacing.y = 10.0;
            let title = egui::RichText::new(&title).size(22.0).family(s.look.display.clone()).color(s.p.ink_strong);
            ui.add(egui::Label::new(title).wrap());
            ui.add(egui::Label::new(egui::RichText::new(text).color(s.p.ink)).wrap());
            ui.add(egui::Label::new(egui::RichText::new(tl!("Downloads stay allowed until you turn them off.")).color(s.p.muted)).wrap());
            ui.add_space(8.0);
            direction::horizontal(ui, |ui| {
                allow = widgets::button(ui, &s, tl!("Download"), widgets::Kind::Primary, s.h(), 0.0).clicked();
                cancel = widgets::button(ui, &s, tl!("Cancel"), widgets::Kind::Secondary, s.h(), 0.0).clicked();
            });
        });
        if allow {
            self.asking = None;
            if self.set_downloads_allowed(true).is_ok() {
                self.download(ctx, pending);
            }
        } else if cancel || response.should_close() {
            self.asking = None;
        }
    }

    pub(crate) fn index(&self) -> Option<Arc<Index>> {
        self.catalogue.as_ref().ok().map(|c| Arc::clone(&c.index))
    }
}
