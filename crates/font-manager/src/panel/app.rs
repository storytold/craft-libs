//! The panel's model, what the host calls, and the view: the tabs and search across the top, the filters at the
//! reading start, the family cards, the details sheet and the download consent.

#[cfg(feature = "screen-testing")]
mod stories;

use std::collections::HashMap;
use std::sync::Arc;

use crate::craft_font_catalog::{FontEnv, Limits, Os};
use crate::filter::{Query, Search};
use crate::index::{Category, Family, Index, categories, source_label};
use crate::installed::{InstalledFamily, group};
use crate::store::{Installer, RemoveRequest, StoreError};
use crate::{Catalog, FaceInfo, Source};
use craft_behavior::ColumnWidth;
use craft_host::{Host, Section, Wake};
use craft_i18n::Folder;
use craft_i18n::matcher::Matcher;
use craft_ui::contract::{NoValue, Outcome, Props as _, ShowContainer as _};
use craft_ui::dialog::Prompt;
use craft_ui::fonts::{EmbeddedFace, I18nScripts, UiFonts};
use craft_ui::layout_components::{Grid, Stack};
use craft_ui::runtime::commands::{CommandSet as _, CommandState, Commands as _};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::focus::{self, FocusTarget};
use craft_ui::runtime::words::CommonWords;
use craft_ui::sections::{DialogRole, FooterButton};
use egui::{Context, Response, Ui};

use crate::panel::browse::{FontsBrowseWords, Texts};
use crate::panel::details::DetailsSheet;
use crate::panel::i18n::LANGUAGES;
use crate::panel::jobs::{self, Finished, Jobs, Kind, Request};
use crate::panel::samples;

/// Installs the standalone app's interface fonts in `ctx`: craft-ui's font stack for `language`, with `craft_faces`
/// (craft-fonts' faces the app embeds with craft-fonts-build, or none) and the system's fonts for every other
/// character. A host with its own fonts doesn't call it. The interface direction is the app's `RootEnv::direction`.
pub fn install_interface_fonts(ctx: &Context, language: &str, craft_faces: &[EmbeddedFace]) {
    UiFonts::new(&I18nScripts).languages(LANGUAGES).ui_language(language).craft_faces(craft_faces).install(ctx);
}

/// The filters column takes the width of its widest section; the list and toolbar take the rest.
static PANES: [ColumnWidth; 2] = [ColumnWidth::Content, ColumnWidth::fill(1.0)];

/// Preview downloads: at most this much per file, and this many faces in a session.
const PREVIEW_LIMITS: Limits = Limits { max_file_bytes: 64 << 20, max_faces_per_file: 512, max_added_faces: 20_000 };

craft_ui::words! {
    /// The words of the Fonts panel: the names it shows for categories, sources and styles, the download consent and
    /// the notes that belong to no card.
    pub struct FontsPanelWords {
        panel: "fontsPanel.panel" = "Fonts";
        sans_serif: "fontsPanel.sansSerif" = "Sans serif";
        serif: "fontsPanel.serif" = "Serif";
        display: "fontsPanel.display" = "Display";
        handwriting: "fontsPanel.handwriting" = "Handwriting";
        monospace: "fontsPanel.monospace" = "Monospace";
        font_store: "fontsPanel.fontStore" = "Font store";
        system: "fontsPanel.system" = "System";
        regular: "fontsPanel.regular" = "Regular";
        styles: count "fontsPanel.styles" = "{n} style" | "{n} styles";
        families: count "fontsPanel.families" = "{n} family" | "{n} families";
        setting_failed: template "fontsPanel.settingFailed" = "The download setting couldn't be saved: {error}";
        consent_title: template "fontsPanel.consentTitle" = "Download from {source}?";
        consent_text: template "fontsPanel.consentText" = "Craft Font Manager downloads {family} ({size}) from {source}, and checks every file before using it.";
        consent_text_host: template "fontsPanel.consentTextHost" = "Craft Font Manager downloads {family} ({size}) from {host}, the server of {source}, a source you added, and checks every file before using it.";
        consent_note: "fontsPanel.consentNote" = "Downloads stay allowed until you turn them off.";
        consent_allow: "fontsPanel.consentAllow" = "Download";
    }
}

impl FontsPanelWords {
    /// The name of a category.
    pub(crate) fn category(&self, category: Category) -> &Label {
        match category {
            Category::SansSerif => &self.sans_serif,
            Category::Serif => &self.serif,
            Category::Display => &self.display,
            Category::Handwriting => &self.handwriting,
            Category::Monospace => &self.monospace,
        }
    }
}

/// What a sample shows when the user hasn't typed one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum Preset {
    /// A sentence in the language filtered by, or in the family's own script.
    #[default]
    Sentence,
    #[allow(missing_docs)]
    Alphabet,
    #[allow(missing_docs)]
    Numerals,
}

/// What the panel keeps between runs: the `fonts_panel` section of the app's settings, through the
/// [`Host`](craft_host::Host).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct FontsPanelSettings {
    /// The sample text; empty for the preset.
    pub sample: String,
    /// Which sample text shows while `sample` is empty.
    pub preset: Preset,
    /// Sample size in points, 12 to 120.
    pub size: f32,
    /// The user closed the note that downloads are off.
    pub hide_download_note: bool,
}

impl Section for FontsPanelSettings {
    const KEY: &'static str = "fonts_panel";
}

impl Default for FontsPanelSettings {
    fn default() -> Self {
        FontsPanelSettings { sample: String::new(), preset: Preset::Sentence, size: 36.0, hide_download_note: false }
    }
}

/// A tab of the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Tab {
    /// Families installed on this machine.
    Installed,
    /// Families in the downloadable catalogue.
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum Origin {
    #[default]
    All,
    System,
    Store,
}

/// What the Installed tab's list was worked out for: the search, the source and the catalog's generation.
type InstalledFilter = (String, Origin, Option<u64>);

/// What the app is built from; tests pass their own catalog, store and index.
#[non_exhaustive]
pub struct Options {
    /// The installed fonts the panel lists.
    pub catalog: Arc<Catalog>,
    /// Installs and removes fonts and fetches downloads.
    pub installer: Arc<dyn Installer>,
    /// The Google Fonts index, or why it couldn't be read.
    pub index: Result<Index, String>,
}

impl Options {
    /// Options over `catalog`, `installer` and `index`.
    pub fn new(catalog: Arc<Catalog>, installer: Arc<dyn Installer>, index: Result<Index, String>) -> Self {
        Options { catalog, installer, index }
    }
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

/// The Fonts panel's state: the installed fonts and the catalogue with the search, the filters, the downloads in
/// progress and the settings. Build it with `Options`; the host shows it with [`FontsPanelView`] once per frame.
pub struct FontsPanel {
    pub(crate) settings: FontsPanelSettings,
    /// What the Host's section last held, so a change is written once.
    saved: FontsPanelSettings,
    /// The settings were read from the Host, on the first frame.
    settings_loaded: bool,
    pub(crate) tab: Tab,
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
    pub(crate) jobs: Jobs,
    pub(crate) asking: Option<Pending>,
    /// By lower-case family name.
    pub(crate) failed: HashMap<String, Failure>,
    /// A problem that belongs to no family: why the download setting couldn't be saved.
    pub(crate) notice: Option<String>,
    /// The window had OS focus on the last pass.
    window_active: bool,
    /// The download consent as last read.
    consent: Option<bool>,
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

pub(crate) fn source_name(words: &FontsPanelWords, source: Source) -> &Label {
    match source {
        Source::Store => &words.font_store,
        _ => &words.system,
    }
}

/// The face closest to an upright regular.
pub(crate) fn regular(faces: &[FaceInfo]) -> Option<&FaceInfo> {
    faces.iter().min_by_key(|f| (f.italic, ((f.weight - 400.0).abs() * 10.0) as i64, ((f.width - 100.0).abs() * 10.0) as i64))
}

pub(crate) fn style_name<'a>(words: &'a FontsPanelWords, face: &'a FaceInfo) -> &'a str {
    if face.style.is_empty() { words.regular.as_str() } else { &face.style }
}

/// How many styles a family of the index has: each static file one, a variable file one per hundred of weight its
/// `wght` axis covers.
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

pub(crate) fn category_label(words: &FontsPanelWords, family: &Family) -> String {
    categories(family).map(|c| words.category(c).as_str()).collect::<Vec<_>>().join(", ")
}

impl FontsPanel {
    /// A panel whose settings are the Host's `fonts_panel` section, read on its first frame and written when the
    /// panel changes them; defaults kept in memory without a Host.
    pub fn new(options: Options) -> FontsPanel {
        let catalogue = options.index.map(|index| Catalogue { search: Search::new(&index), index: Arc::new(index) });
        let previews = Arc::new(Catalog::new(FontEnv { os: Os::None, ..FontEnv::default() }).with_font_service(false));
        FontsPanel {
            settings: FontsPanelSettings::default(),
            saved: FontsPanelSettings::default(),
            settings_loaded: false,
            tab: Tab::Catalogue,
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
            jobs: Jobs::new(),
            asking: None,
            failed: HashMap::new(),
            notice: None,
            window_active: true,
            consent: None,
        }
    }

    /// Shows `index` (after the host fetched the sources, say). The details sheet closes, as family positions
    /// change.
    pub fn set_index(&mut self, index: Result<Index, String>) {
        self.catalogue = index.map(|index| Catalogue { search: Search::new(&index), index: Arc::new(index) });
        self.results_for = None;
        self.detail = None;
        self.asking = None;
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
    pub(crate) fn fetch_sources(&mut self, host: Option<&Arc<Host>>) {
        if self.sources_job.is_some() || !self.downloads_allowed() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let installer = Arc::clone(&self.installer);
        self.sources_job = Some(rx);
        let wake = self.jobs.waker();
        jobs::spawn(
            host,
            "font-sources",
            Box::new(move || {
                let _ = tx.send(installer.fetch_indexes());
                wake.wake();
            }),
        );
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

    /// Whether the panel shows its own "Allow downloads" switch. A host with the switch in its own settings turns it
    /// off; the panel still offers to allow downloads where one is needed.
    pub fn show_downloads_switch(&mut self, show: bool) {
        self.downloads_switch = show;
    }

    /// The panel's own settings: the sample text, preset and size, and whether the downloads note was closed.
    pub fn settings(&self) -> &FontsPanelSettings {
        &self.settings
    }

    /// The current tab.
    pub fn tab(&self) -> Tab {
        self.tab
    }

    /// Switch to `tab`.
    pub fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
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

    /// Has the user allowed downloads from Google Fonts? A suite-wide setting of the font store, off until they
    /// turn it on or confirm an install or preview.
    pub fn downloads_allowed(&self) -> bool {
        self.installer.downloads_allowed()
    }

    /// Turns downloads on or off, as the user's own action (a host's settings page, say). A failure also shows in
    /// the panel.
    pub fn set_downloads_allowed(&mut self, allowed: bool) -> Result<(), StoreError> {
        let result = self.installer.allow_downloads(allowed);
        self.consent = None;
        self.notice = result.as_ref().err().map(|e| e.0.clone());
        result
    }

    /// The consent, read from the store's settings file (which another Craft app can change) when the window regains
    /// focus and after the panel changes it itself.
    pub(crate) fn allowed(&mut self) -> bool {
        *self.consent.get_or_insert_with(|| self.installer.downloads_allowed())
    }

    /// Everything a pass reads before it draws: the settings from the Host (once), finished jobs, the installed fonts
    /// (read again when the window regains focus) and the results of the search.
    fn prepare(&mut self, host: Option<&Arc<Host>>, window_active: bool) {
        self.load_settings(host);
        self.poll_jobs();
        if window_active && !self.window_active {
            if self.catalog.is_ready() {
                self.catalog.rescan_if_changed();
            }
            self.consent = None;
        }
        self.window_active = window_active;
        self.refresh_installed();
        self.refresh_results();
    }

    /// Reads the settings from the Host's section, once.
    fn load_settings(&mut self, host: Option<&Arc<Host>>) {
        if std::mem::replace(&mut self.settings_loaded, true) {
            return;
        }
        if let Some(host) = host {
            self.settings = host.setting();
            self.saved = self.settings.clone();
        }
    }

    /// Writes the settings to the Host's section when the pass changed them.
    fn save_settings(&mut self, host: Option<&Arc<Host>>) {
        if let Some(host) = host
            && self.settings != self.saved
        {
            host.set_setting(&self.settings);
            self.saved = self.settings.clone();
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
    pub(crate) fn installed_shown(&mut self, folder: &Folder) -> Vec<usize> {
        let key = (self.installed_text.clone(), self.origin, self.installed_generation);
        if self.installed_shown.1.as_ref() != Some(&key) {
            let matcher = Matcher::new(folder, &self.installed_text);
            let shown = (0..self.installed.len())
                .filter(|&i| {
                    let Some(f) = self.installed.get(i) else { return false };
                    let origin = match self.origin {
                        Origin::All => true,
                        Origin::System => f.source != Source::Store,
                        Origin::Store => f.source == Source::Store,
                    };
                    origin && matcher.score(&f.name).is_some()
                })
                .collect();
            self.installed_shown = (shown, Some(key));
        }
        self.installed_shown.0.clone()
    }

    /// The sample for a family with `scripts`: what the user typed, else the preset.
    pub(crate) fn sample_for<'a>(&self, scripts: impl IntoIterator<Item = &'a str>) -> String {
        if !self.settings.sample.trim().is_empty() {
            return self.settings.sample.clone();
        }
        match self.settings.preset {
            Preset::Alphabet => samples::ALPHABET.to_owned(),
            Preset::Numerals => samples::NUMERALS.to_owned(),
            Preset::Sentence => {
                let language = self.catalogue.as_ref().ok().zip(self.query.language).and_then(|(c, l)| c.index.languages().get(l));
                language.and_then(|l| samples::for_language(&l.id)).unwrap_or_else(|| samples::for_scripts(scripts)).to_owned()
            }
        }
    }

    /// Runs `pending` if downloads are allowed, or asks first.
    pub(crate) fn ask(&mut self, host: Option<&Arc<Host>>, pending: Pending) {
        if self.allowed() {
            self.download(host, pending);
        } else {
            self.asking = Some(pending);
        }
    }

    fn download(&mut self, host: Option<&Arc<Host>>, pending: Pending) {
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
        self.jobs.start(host, Arc::clone(&self.installer), Arc::clone(&self.catalog), request);
    }

    /// Removes store files: a whole family, or the files of some of its styles.
    pub(crate) fn remove(&mut self, host: Option<&Arc<Host>>, family: &str, faces: &[FaceInfo], whole: bool) {
        let mut files: Vec<_> = faces.iter().filter(|f| f.source == Source::Store).filter_map(|f| f.path.clone()).collect();
        files.sort();
        files.dedup();
        if files.is_empty() {
            return;
        }
        self.failed.remove(&family_key("", family));
        let request = RemoveRequest { family: family.to_owned(), files, whole };
        self.jobs.start(host, Arc::clone(&self.installer), Arc::clone(&self.catalog), Request::Remove(request));
    }

    pub(crate) fn index(&self) -> Option<Arc<Index>> {
        self.catalogue.as_ref().ok().map(|c| Arc::clone(&c.index))
    }

    /// The consent prompt for `pending`: its title and message, the file's size in the user's units and, for a source
    /// the user added, the server the file comes from.
    fn consent_text(&self, pending: &Pending, words: &FontsPanelWords, units: &craft_ui::runtime::env::Units) -> Option<(String, String)> {
        let Ok(c) = &self.catalogue else { return None };
        let family = c.index.families().get(pending.family())?;
        let file = match pending {
            Pending::InstallFile(_, path) => family.files.iter().find(|f| &f.path == path),
            _ => family.files.first(),
        };
        let size = match pending {
            Pending::InstallFile(..) => file.map_or(0, |f| f.size),
            _ => family.size(),
        };
        let (source, curated) = source_label(&c.index, family);
        // A curated source's files come through its repository's CDN; a source the user added can point anywhere, so
        // the prompt names the server.
        let host = file.filter(|_| !curated).and_then(|f| c.index.file_urls(family, f).first().and_then(|u| url_host(u)).map(str::to_owned));
        let size = units.file_size(size);
        let args = [("family", family.family.as_str()), ("size", size.as_str()), ("source", source), ("host", host.as_deref().unwrap_or(""))];
        let text = match &host {
            None => words.consent_text.fill(&args),
            Some(_) => words.consent_text_host.fill(&args),
        };
        Some((words.consent_title.fill(&[("source", source)]), text))
    }
}

/// The panel's own commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, craft_ui::contract::CommandSet)]
#[command(prefix = "fonts", scope = "fonts")]
#[non_exhaustive]
pub enum FontsCommand {
    /// Move the keyboard focus to the search field.
    #[command(label = "Find", icon = "search", default = "Cmd+F")]
    Find,
}

/// What [`FontsPanelView::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct FontsPanelOutput {
    /// The panel's response.
    pub response: Response,
}

impl Outcome for FontsPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The Fonts panel over a [`FontsPanel`]: the Installed and Browse tabs with a search field across the top, the
/// filters at the reading start, the toolbar for the sample and the list of family cards. A press or Enter on a card
/// opens its details sheet; the download consent is a prompt.
///
/// - **Keys.** The Find command (Cmd or Ctrl+F) focuses the search field. The cards are one Tab stop: Up and Down move
///   between them, Enter opens the focused one. Escape closes the details sheet, and answers the consent prompt with
///   Cancel; Enter answers it with Download.
/// - **State.** The model is the host's [`FontsPanel`]; the view keeps nothing of its own between passes.
#[must_use = "show the panel with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct FontsPanelView<'a> {
    panel: &'a mut FontsPanel,
}

impl<'a> FontsPanelView<'a> {
    /// The view of `panel`.
    pub fn new(panel: &'a mut FontsPanel) -> Self {
        FontsPanelView { panel }
    }

    /// Shows the panel, filling the room it is given.
    pub fn show(self, ui: &mut Ui) -> FontsPanelOutput {
        let panel = self.panel;
        let ctx = ui.ctx().clone();
        panel.jobs.wake_with(|| Wake::new(move || ctx.request_repaint()));
        let env = Env::of(ui);
        let view = env.view();
        let words = env.words::<FontsPanelWords>();
        let browse = env.words::<FontsBrowseWords>();
        let texts = Texts { panel: &words, browse: &browse, format: view.format() };
        let host = view.host().cloned();
        panel.prepare(host.as_ref(), view.shell().window.active);
        let search = ui.scope_id().with("search");
        let folder = env.folder();
        let shown = Stack::new().fill().label(words.panel.clone()).command_scope(FontsCommand::SCOPE).show(ui, |ui, root| {
            while root.take_command::<FontsCommand>().is_some() {
                focus::request(ui.ctx(), FocusTarget::Id(search));
            }
            root.publish(FontsCommand::Find, CommandState::new());
            panel.top_bar(ui, &browse, texts.format, search);
            Grid::new(&PANES).fill().show(ui, |ui, _| {
                panel.filters(ui, &texts);
                panel.main(ui, &texts, &folder, host.as_ref());
            });
        });
        if panel.detail.is_some() && DetailsSheet::new(panel).show(ui).closed {
            panel.detail = None;
        }
        panel.consent_prompt(ui, &env, &words, host.as_ref());
        panel.save_settings(host.as_ref());
        FontsPanelOutput { response: shown.output }
    }
}

impl FontsPanel {
    /// The download consent, while a download waits for the user's answer.
    fn consent_prompt(&mut self, ui: &mut Ui, env: &Env, words: &FontsPanelWords, host: Option<&Arc<Host>>) {
        let Some(pending) = self.asking.clone() else { return };
        let Some((title, text)) = self.consent_text(&pending, words, &env.units()) else { return };
        let common = env.words::<CommonWords>();
        let answers =
            [FooterButton::new(words.consent_allow.clone(), DialogRole::Accept), FooterButton::new(common.cancel.clone(), DialogRole::Cancel)];
        let out = Prompt::new(Label::from(title), Label::from(text)).paragraph(words.consent_note.clone()).answers(answers).show(ui);
        match out.picked {
            Some(0) => {
                self.asking = None;
                if self.set_downloads_allowed(true).is_ok() {
                    self.download(host, pending);
                }
            }
            Some(_) => self.asking = None,
            None => {}
        }
    }
}
