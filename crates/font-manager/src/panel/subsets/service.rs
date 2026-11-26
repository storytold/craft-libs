//! The subsets of the preview service.
//!
//! The rows on screen ask for their face's sample subset first, then the rows one screen above and two below; rows
//! that leave that window are cancelled. Once the screen's samples are in, its Latin rows also get the ASCII subset, so
//! typed ASCII and the presets draw at once. Other typed text asks for a subset of its own characters, for the rows on
//! screen only, once the typing pauses. Without a service nothing is asked and the list keeps its whole-family
//! previews; with the previews setting off, only subsets in the disk cache are shown.
//!
//! The details sheet asks for each file it shows, before the list's rows: a static file's subset as the list would, a
//! variable file's at the `var` instance, which keeps every axis.
//!
//! Drawing only looks subsets up ([`Subsets::font`], [`Subsets::detail_font`]). What the pass showed is taken in once,
//! after the real run of the list ([`Subsets::request`]) and of the sheet ([`Subsets::request_detail`]).
//!
//! The service holds the built-in sources' files only, and asking it for another file would tell it the file's hash: a
//! family of a source the user added is never asked for (`key_of`).

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use craft_font_store::preview::{Codepoints, Instance, PreviewError, PreviewKey, Previews, SampleHash, Subset};
use craft_host::Wake;

use super::{Row, SubsetFont};
use crate::index::{Family, FontFile, Index};
use crate::panel::app::list_file;
use crate::store::{Installer, PreviewStore};

/// Rows on screen.
const VISIBLE: u32 = 3;
/// Rows one screen above and two below.
const WINDOW: u32 = 2;
/// ASCII subsets for the rows on screen, once their samples are in.
const BACKGROUND: u32 = 1;
/// The files of the family the details sheet shows.
const DETAIL: u32 = 4;

/// How long typing pauses before its characters are asked for.
const PAUSE_SECS: f64 = 0.25;
/// A failed subset is asked for again after this long, not every frame.
const RETRY_SECS: f64 = 30.0;
/// Decoded subsets kept in memory, more while more are in use; the disk cache holds more.
const MAX_HELD: usize = 256;
/// A subset wanted within this many passes is in use.
const IN_USE: u64 = 2;

/// A decoded subset and the pass it was last wanted in.
struct Held {
    bytes: Arc<[u8]>,
    used: u64,
}

pub(crate) struct Subsets {
    installer: Arc<dyn Installer>,
    /// `None` only while it is being given its wake.
    pool: Option<Previews>,
    woken: bool,
    /// A service is set, so rows draw from subsets; and previews are on, so subsets not in the disk cache are fetched.
    available: bool,
    fetching: bool,
    /// The effective service the last settings read saw; another one starts afresh.
    service: Option<String>,
    /// The settings are read on the next pass.
    settings_stale: bool,
    fonts: HashMap<PreviewKey, Held>,
    /// Subsets that didn't load, and when; kept until one loads, so a family whose subset is asked for again still
    /// offers a session preview meanwhile.
    failed: HashMap<PreviewKey, f64>,
    /// Sample subsets the service doesn't have for the text this index gives (HTTP 404), so those faces draw from a
    /// subset of the sample text's characters. Keyed by the sample's hash too, so another sample text asks for the
    /// sample again.
    sample_missing: HashSet<PreviewKey>,
    /// Asked for and not back yet, at this priority.
    asked: HashMap<PreviewKey, u32>,
    /// The subset each face (by sha256 and instance) was last drawn with, drawn while the next one loads.
    shown: HashMap<(String, Instance), PreviewKey>,
    /// Asked for by the details sheet; the list leaves them be.
    detail: HashSet<PreviewKey>,
    /// The text setting as last seen, and since when.
    typed: (String, f64),
    /// The details sheet or the list shows typed characters that are asked for once typing pauses.
    waits: bool,
    /// The service and the error of its last failed request, until a preview loads from it: the page tells the user the
    /// server can't be reached instead of showing no previews silently.
    unreachable: Option<(String, String)>,
    pass: u64,
    now: f64,
}

impl Subsets {
    /// Subsets fetched with `installer`'s own client and cache.
    pub(crate) fn new(installer: Arc<dyn Installer>) -> Subsets {
        let store: Arc<dyn PreviewStore> = Arc::clone(&installer) as Arc<dyn PreviewStore>;
        Subsets::with_pool(installer, Previews::new(store))
    }

    fn with_pool(installer: Arc<dyn Installer>, pool: Previews) -> Subsets {
        Subsets {
            installer,
            pool: Some(pool),
            woken: false,
            available: false,
            fetching: false,
            service: None,
            settings_stale: true,
            fonts: HashMap::new(),
            failed: HashMap::new(),
            sample_missing: HashSet::new(),
            asked: HashMap::new(),
            shown: HashMap::new(),
            detail: HashSet::new(),
            typed: (String::new(), f64::NEG_INFINITY),
            waits: false,
            unreachable: None,
            pass: 0,
            now: 0.0,
        }
    }

    /// Fetches through `pool` instead.
    pub(crate) fn replace(&mut self, pool: Previews) {
        *self = Subsets::with_pool(Arc::clone(&self.installer), pool);
    }

    /// Reads the service and previews settings again on the next pass.
    pub(crate) fn read_settings_soon(&mut self) {
        self.settings_stale = true;
    }

    /// Whether rows draw from subsets: a service is set.
    pub(crate) fn available(&self) -> bool {
        self.available
    }

    /// Whether the details sheet draws the styles of `family` from subsets: a service is set, which holds its files.
    pub(crate) fn serves(&self, family: &Family) -> bool {
        self.available && family.is_curated()
    }

    /// Whether rows and the details sheet get their previews from the service: a service is set and previews are on.
    /// Otherwise a session download is the only way to see a font.
    fn previewing(&self) -> bool {
        self.available && self.fetching
    }

    /// Whether the service draws `row`, or is still loading it: previews are on and its subset hasn't failed. A row it
    /// doesn't draw offers a session preview.
    pub(crate) fn draws(&self, index: &Index, row: &Row<'_>) -> bool {
        self.previewing() && self.key(index, row).is_some_and(|k| !self.failed.contains_key(&k))
    }

    /// Whether the service draws every file of `family` in the details sheet with `text`, or is still loading them: for
    /// each, the subset `text` needs or the sample subset hasn't failed.
    pub(crate) fn draws_detail(&self, index: &Index, family: &Family, text: &str) -> bool {
        let fine = |key: Option<PreviewKey>| key.is_some_and(|k| !self.failed.contains_key(&k));
        self.previewing()
            && family.files.iter().all(|file| {
                let instance = detail_instance(file);
                fine(text_key(index, family, file, instance, text, &self.sample_missing)) || fine(self.sample_key(index, family, file, instance))
            })
    }

    /// Reads which service is set and whether previews are on; a change asks for everything again.
    fn read_settings(&mut self) {
        let service = self.installer.preview_service();
        let available = service.is_some();
        let fetching = self.pool.as_ref().is_some_and(Previews::available);
        if (available, fetching, &service) != (self.available, self.fetching, &self.service) {
            (self.available, self.fetching, self.service) = (available, fetching, service);
            self.failed.clear();
            self.detail.clear();
            self.sample_missing.clear();
            self.unreachable = None;
            self.cancel_all();
        }
    }

    /// The preview server and the error of its last failed request, while none has loaded since.
    pub(crate) fn unreachable(&self) -> Option<(&str, &str)> {
        self.unreachable.as_ref().map(|(service, error)| (service.as_str(), error.as_str()))
    }

    /// Whether subsets are being fetched.
    pub(crate) fn loading(&self) -> bool {
        !self.asked.is_empty()
    }

    /// Takes in what the pool fetched, at the pass time `now`. `wake` asks the UI for a pass when a fetch ends.
    pub(crate) fn begin(&mut self, now: f64, wake: &Wake) {
        self.now = now;
        self.waits = false;
        if !std::mem::replace(&mut self.woken, true) {
            let wake = wake.clone();
            self.pool = self.pool.take().map(|pool| pool.on_ready(move || wake.wake()));
        }
        if std::mem::take(&mut self.settings_stale) {
            self.read_settings();
        }
        let Some(pool) = &self.pool else { return };
        for preview in pool.poll() {
            // A result for a key no longer asked for still fills the cache in memory.
            self.asked.remove(&preview.key);
            match preview.font {
                Ok(font) => {
                    self.unreachable = None;
                    self.failed.remove(&preview.key);
                    self.fonts.insert(preview.key, Held { bytes: font.into(), used: self.pass });
                }
                Err(e) => {
                    log::debug!("preview {}: {e}", preview.key.path());
                    if let (PreviewError::Failed(message), Some(service)) = (&e, &self.service) {
                        if self.unreachable.as_ref().is_none_or(|(s, _)| s != service) {
                            log::warn!("font preview server {service} didn't answer: {message}");
                        }
                        self.unreachable = Some((service.clone(), message.clone()));
                    }
                    if e == PreviewError::NotFound && matches!(preview.key.subset(), Subset::Sample(_)) {
                        self.sample_missing.insert(preview.key);
                    } else {
                        self.failed.insert(preview.key, self.now);
                    }
                }
            }
        }
    }

    /// In how many seconds the next pass asks for something this one waited with: typed characters, or a subset that
    /// failed.
    pub(crate) fn next_wake(&self) -> Option<f64> {
        let typed = self.waits.then_some(PAUSE_SECS - (self.now - self.typed.1));
        let retries = self.failed.values().map(|at| RETRY_SECS - (self.now - at));
        typed.into_iter().chain(retries).filter(|seconds| *seconds > 0.0).reduce(f64::min)
    }

    /// Takes in the text setting (the sample typed, the preset and the language filtered by) once the controls have
    /// changed it this pass, before anything is asked for: typed characters are asked for once it pauses.
    pub(crate) fn typed(&mut self, typed: String) {
        if typed != self.typed.0 {
            self.typed = (typed, self.now);
        }
    }

    fn paused(&self) -> bool {
        self.now - self.typed.1 >= PAUSE_SECS
    }

    /// The subset `row` draws with: the one its text needs, else while that loads the one its face was last drawn with.
    /// `None` without a service, and for a row whose subset failed: the row then shows what it shows without the
    /// service.
    pub(crate) fn font(&self, index: &Index, row: &Row<'_>) -> Option<SubsetFont> {
        if !self.available {
            return None;
        }
        self.drawn(&self.key(index, row)?).and_then(|key| self.subset_font(key))
    }

    /// The subset that draws `text` in `file` of `family` in the details sheet, which asks for it with
    /// [`Subsets::request_detail`]: the one `text` needs, else the file's sample subset, which draws while that loads,
    /// when it failed, and for text of more characters than a request may name. `None` without a service, while nothing
    /// of the file has loaded, and when both failed.
    pub(crate) fn detail_font(&self, index: &Index, family: &Family, file: &FontFile, text: &str) -> Option<SubsetFont> {
        if !self.available {
            return None;
        }
        self.detail_drawn(index, family, file, text).and_then(|key| self.subset_font(&key))
    }

    fn detail_drawn(&self, index: &Index, family: &Family, file: &FontFile, text: &str) -> Option<PreviewKey> {
        let instance = detail_instance(file);
        let drawn = |key: Option<PreviewKey>| key.and_then(|key| self.drawn(&key).cloned());
        drawn(text_key(index, family, file, instance, text, &self.sample_missing)).or_else(|| drawn(self.sample_key(index, family, file, instance)))
    }

    /// The key of the subset drawn where `want` is wanted: `want` if it is here, else the one its face was last drawn
    /// with at its instance. `None` when `want` failed.
    fn drawn<'a>(&'a self, want: &'a PreviewKey) -> Option<&'a PreviewKey> {
        if self.has_failed(want) {
            return None;
        }
        if self.fonts.contains_key(want) {
            return Some(want);
        }
        self.shown.get(&face_of(want)).filter(|last| self.fonts.contains_key(*last))
    }

    fn subset_font(&self, key: &PreviewKey) -> Option<SubsetFont> {
        self.fonts.get(key).map(|held| SubsetFont { key: key.path(), bytes: Arc::clone(&held.bytes) })
    }

    /// Takes in that this pass drew the subset `key`.
    fn note_drawn(&mut self, key: PreviewKey) {
        if let Some(held) = self.fonts.get_mut(&key) {
            held.used = self.pass;
            self.shown.insert(face_of(&key), key);
        }
    }

    /// Asks for the subsets of the files (by position) of `shown`'s family that the sheet draws with its text, before
    /// the list's rows; called at the end of each pass, also when the sheet is closed. What it no longer wants goes back
    /// to the list, which cancels what it doesn't want either.
    pub(crate) fn request_detail(&mut self, shown: Option<(&Index, &Family, &str)>, files: &[usize]) {
        self.pass += 1;
        let Some((index, family, text)) = shown.filter(|_| self.available) else {
            self.detail.clear();
            return;
        };
        let mut asking = HashMap::new();
        for file in files.iter().filter_map(|n| family.files.get(*n)) {
            let instance = detail_instance(file);
            if let Some(sample) = self.sample_key(index, family, file, instance) {
                self.want(&mut asking, sample, DETAIL);
            }
            if let Some(want) = text_key(index, family, file, instance, text, &self.sample_missing) {
                if matches!(want.subset(), Subset::Codepoints(_)) && !self.paused() {
                    self.waits = true;
                } else {
                    self.want(&mut asking, want, DETAIL);
                }
            }
            if let Some(key) = self.detail_drawn(index, family, file, text) {
                self.note_drawn(key);
            }
        }
        self.detail = asking.keys().cloned().collect();
        self.ask(asking);
    }

    /// The rows to pass to [`Subsets::request`] when `visible` of `count` rows are on screen: those and one screen above
    /// and two below.
    pub(crate) fn window(visible: &Range<usize>, count: usize) -> Range<usize> {
        let screen = visible.len();
        visible.start.saturating_sub(screen)..visible.end.saturating_add(2 * screen).min(count)
    }

    /// Asks for the subsets of the list's rows from `start` on (`None` for a row drawn without the service, such as an
    /// installed family), `visible` of them on screen, and cancels those no longer wanted.
    pub(crate) fn request(&mut self, index: &Index, start: usize, rows: &[Option<Row<'_>>], visible: Range<usize>) {
        if !self.available {
            return;
        }
        let row = |n: usize| rows.get(n.checked_sub(start)?)?.as_ref();
        let mut wanted: HashMap<PreviewKey, u32> = HashMap::new();
        let screen = visible.len();
        let paused = self.paused();
        let mut settled = true;
        let mut latin = Vec::new();
        for row in visible.clone().filter_map(row) {
            let family = row.0;
            // A family without a sample text still gets the subset its row's text needs.
            if let Some(sample) = list_file(family).and_then(|f| self.sample_key(index, family, f, Instance::List)) {
                settled &= self.fonts.contains_key(&sample) || self.has_failed(&sample);
                self.want(&mut wanted, sample, VISIBLE);
            }
            if let Some(ascii) = list_file(family).filter(|_| covers_latin(family)).and_then(|f| key_of(family, f, Instance::List, Subset::Ascii)) {
                latin.push(ascii);
            }
            let Some(key) = self.key(index, row) else { continue };
            if let Some(drawn) = self.drawn(&key).cloned() {
                self.note_drawn(drawn);
            }
            if matches!(key.subset(), Subset::Codepoints(_)) && !paused {
                self.waits = true;
            } else {
                self.want(&mut wanted, key, VISIBLE);
            }
        }
        if settled {
            for ascii in latin {
                self.want(&mut wanted, ascii, BACKGROUND);
            }
        }
        let above = visible.start.saturating_sub(screen)..visible.start;
        let below = visible.end..visible.end.saturating_add(2 * screen);
        for (family, _) in above.chain(below).filter_map(row) {
            if let Some(sample) =
                list_file(family).and_then(|f| sample_subset(index, family, f, Instance::List)).filter(|k| !self.sample_missing.contains(k))
            {
                self.want(&mut wanted, sample, WINDOW);
            }
        }

        let detail = &self.detail;
        let keep = |k: &PreviewKey| wanted.contains_key(k) || detail.contains(k);
        if let Some(pool) = &self.pool
            && self.asked.keys().any(|k| !keep(k))
        {
            pool.retain(&keep);
            self.asked.retain(|k, _| keep(k));
        }
        wanted.retain(|key, _| !detail.contains(key));
        self.ask(wanted);
        self.trim();
    }

    /// Sends `wanted` to the pool, where it does not know them at this priority.
    fn ask(&mut self, wanted: HashMap<PreviewKey, u32>) {
        let Some(pool) = &self.pool else { return };
        for (key, priority) in wanted {
            if self.asked.get(&key) != Some(&priority) {
                pool.request(key.clone(), priority);
                self.asked.insert(key, priority);
            }
        }
    }

    /// The sample subset of `file` at `instance`, or when the service doesn't have it for this text its
    /// [`sample_fallback`]. `None` when there is neither.
    fn sample_key(&self, index: &Index, family: &Family, file: &FontFile, instance: Instance) -> Option<PreviewKey> {
        let sample = sample_subset(index, family, file, instance)?;
        if self.sample_missing.contains(&sample) { sample_fallback(index, family, file, instance) } else { Some(sample) }
    }

    fn key(&self, index: &Index, (family, text): &Row<'_>) -> Option<PreviewKey> {
        text_key(index, family, list_file(family)?, Instance::List, text, &self.sample_missing)
    }

    /// Adds `key` to `wanted` at `priority` unless it is here, failed lately, or can't load: with previews off, only
    /// what the disk cache has.
    fn want(&mut self, wanted: &mut HashMap<PreviewKey, u32>, key: PreviewKey, priority: u32) {
        if let Some(held) = self.fonts.get_mut(&key) {
            held.used = self.pass;
            return;
        }
        if self.has_failed(&key) || wanted.get(&key).is_some_and(|p| *p >= priority) {
            return;
        }
        let cached = || self.pool.as_ref().and_then(Previews::cache).is_some_and(|c| c.contains(&key.path()));
        if self.fetching || self.asked.contains_key(&key) || cached() {
            wanted.insert(key, priority);
        }
    }

    fn has_failed(&self, key: &PreviewKey) -> bool {
        self.failed.get(key).is_some_and(|at| self.now - at < RETRY_SECS)
    }

    /// Cancels everything the list asked for, as when it isn't shown.
    pub(crate) fn cancel_all(&mut self) {
        if self.asked.keys().all(|k| self.detail.contains(k)) {
            return;
        }
        let detail = &self.detail;
        if let Some(pool) = &self.pool {
            pool.retain(|k| detail.contains(k));
        }
        self.asked.retain(|k, _| detail.contains(k));
    }

    /// Drops the subsets wanted least recently past [`MAX_HELD`], but none in use.
    fn trim(&mut self) {
        if self.fonts.len() <= MAX_HELD {
            return;
        }
        let mut by_use: Vec<(u64, PreviewKey)> =
            self.fonts.iter().filter(|(_, held)| held.used + IN_USE < self.pass).map(|(k, held)| (held.used, k.clone())).collect();
        by_use.sort_by_key(|(used, _)| *used);
        for (_, key) in by_use.into_iter().take(self.fonts.len() - MAX_HELD) {
            self.fonts.remove(&key);
        }
        self.shown.retain(|_, k| self.fonts.contains_key(k));
    }
}

/// The face a subset is of, and the instance.
fn face_of(key: &PreviewKey) -> (String, Instance) {
    (key.sha256().to_owned(), key.instance())
}

/// The instance the details sheet draws `file` at: every axis for a variable file.
fn detail_instance(file: &FontFile) -> Instance {
    if file.is_variable() { Instance::Var } else { Instance::List }
}

/// The key of `subset` of `file` at `instance`; `None` for a family of a source the user added.
fn key_of(family: &Family, file: &FontFile, instance: Instance, subset: Subset) -> Option<PreviewKey> {
    family.is_curated().then(|| PreviewKey::new(&file.sha256, instance, subset)).flatten()
}

fn covers_latin(family: &Family) -> bool {
    family.scripts.iter().any(|s| s == "Latn")
}

/// The sample subset of `file` at `instance` for the text `index` gives it.
fn sample_subset(index: &Index, family: &Family, file: &FontFile, instance: Instance) -> Option<PreviewKey> {
    let text = index.sample_text(family, file)?;
    key_of(family, file, instance, Subset::Sample(SampleHash::of(text)))
}

/// The subset that draws `text` in `file`: the sample when the sample subset has every character (the sentence and the
/// family name), else ASCII for printable ASCII in a Latin face, else the text's own characters. A sample the service
/// doesn't have gives [`sample_fallback`] instead. `None` for text of more characters than a request may name.
fn text_key(index: &Index, family: &Family, file: &FontFile, instance: Instance, text: &str, missing: &HashSet<PreviewKey>) -> Option<PreviewKey> {
    let sample = index.sample_text(family, file);
    let in_sample = |c: char| c == ' ' || sample.is_some_and(|s| s.contains(c)) || family.family.contains(c);
    if sample == Some(text) || (sample.is_some() && text.chars().all(in_sample)) {
        let key = sample_subset(index, family, file, instance)?;
        if !missing.contains(&key) {
            return Some(key);
        }
        if let Some(fallback) = sample_fallback(index, family, file, instance) {
            return Some(fallback);
        }
    }
    let subset = if covers_latin(family) && text.chars().all(|c| matches!(c, ' '..='~')) {
        Subset::Ascii
    } else {
        Subset::Codepoints(Codepoints::from_text(text))
    };
    key_of(family, file, instance, subset)
}

/// The subset of the sample text's characters and the family name, which the sample subset holds too. `None` when that
/// is more characters than a request may name.
fn sample_fallback(index: &Index, family: &Family, file: &FontFile, instance: Instance) -> Option<PreviewKey> {
    let text = index.sample_text(family, file)?;
    key_of(family, file, instance, Subset::Codepoints(Codepoints::from_text(&format!("{text}{}", family.family))))
}
