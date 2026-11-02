//! The list's previews from the font preview service: a small subset of each row's face instead
//! of the whole family.
//!
//! The rows on screen ask for their face's sample subset first, then the rows one screen above
//! and two below; rows that leave that window are cancelled. Once the screen's samples are in,
//! its Latin rows also get the ASCII subset, so typed ASCII and the presets draw at once. Other
//! typed text asks for a subset of its own characters, for the rows on screen only, once the
//! typing pauses. Without a service nothing is asked and the list keeps its whole-family
//! previews; with the previews setting off, only subsets in the disk cache are shown.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

use egui::Context;

use crate::index::{Family, Index};
use crate::panel::app::list_file;
use crate::remote::{Codepoints, Instance, PreviewError, PreviewKey, Previews, SampleHash, Subset};
use crate::store::Installer;

/// Rows on screen.
const VISIBLE: u32 = 3;
/// Rows one screen above and two below.
const WINDOW: u32 = 2;
/// ASCII subsets for the rows on screen, once their samples are in.
const BACKGROUND: u32 = 1;

/// How long typing pauses before its characters are asked for.
const PAUSE_SECS: f64 = 0.25;
/// A failed subset is asked for again after this long, not every frame.
const RETRY_SECS: f64 = 30.0;
/// Decoded subsets kept in memory; the disk cache holds more.
const MAX_HELD: usize = 256;
/// How often the service and previews settings are read again, as another Craft app can change
/// them.
const SETTINGS_RECHECK_SECS: f64 = 3.0;

/// A row of the list: the family, whose list face draws it, and the text it shows.
pub(crate) struct Row<'a> {
    pub family: &'a Family,
    pub text: String,
}

pub(crate) struct Subsets {
    installer: Arc<dyn Installer>,
    /// `None` only while it is being given its repaint callback.
    pool: Option<Previews>,
    armed: bool,
    /// A service is set, so rows draw from subsets; and previews are on, so subsets not in the
    /// disk cache are fetched. Read every few seconds, not every frame.
    available: bool,
    fetching: bool,
    /// The effective service the last settings read saw; another one starts afresh.
    service: Option<String>,
    /// When those were read.
    read_at: Option<f64>,
    /// Decoded subsets and the frame each was last wanted in.
    fonts: HashMap<PreviewKey, (Arc<Vec<u8>>, u64)>,
    /// Subsets that didn't load, and when.
    failed: HashMap<PreviewKey, f64>,
    /// Sample subsets the service doesn't have for the text this index gives (HTTP 404), so
    /// those faces draw from a subset of the sample text's characters. Keyed by the sample's
    /// hash too, so another sample text asks for the sample again.
    sample_missing: HashSet<PreviewKey>,
    /// Asked for and not back yet, at this priority.
    asked: HashMap<PreviewKey, u32>,
    /// The subset each face (by sha256) was last drawn with, drawn while the next one loads.
    shown: HashMap<String, PreviewKey>,
    /// The text setting as last seen, and since when.
    typed: (String, f64),
    frame: u64,
    now: f64,
}

impl Subsets {
    pub(crate) fn new(installer: Arc<dyn Installer>, pool: Previews) -> Subsets {
        Subsets {
            installer,
            pool: Some(pool),
            armed: false,
            available: false,
            fetching: false,
            service: None,
            read_at: None,
            fonts: HashMap::new(),
            failed: HashMap::new(),
            sample_missing: HashSet::new(),
            asked: HashMap::new(),
            shown: HashMap::new(),
            typed: (String::new(), 0.0),
            frame: 0,
            now: 0.0,
        }
    }

    /// Fetches through `pool` instead.
    pub(crate) fn replace(&mut self, pool: Previews) {
        *self = Subsets::new(Arc::clone(&self.installer), pool);
    }

    /// Reads the service and previews settings again on the next frame.
    pub(crate) fn read_settings_soon(&mut self) {
        self.read_at = None;
    }

    /// Whether rows draw from subsets: a service is set.
    pub(crate) fn available(&self) -> bool {
        self.available
    }

    /// Whether rows and the detail view get their previews from the service: a service is set
    /// and previews are on. Otherwise a session download is the only way to see a font.
    pub(crate) fn previewing(&self) -> bool {
        self.available && self.fetching
    }

    /// Reads which service is set and whether previews are on; a change asks for everything again.
    fn read_settings(&mut self) {
        let service = self.installer.preview_service();
        let available = service.is_some();
        let fetching = self.pool.as_ref().is_some_and(Previews::available);
        if (available, fetching, &service) != (self.available, self.fetching, &self.service) {
            (self.available, self.fetching, self.service) = (available, fetching, service);
            self.failed.clear();
            self.sample_missing.clear();
            self.cancel_all();
        }
    }

    /// Whether subsets are being fetched.
    pub(crate) fn loading(&self) -> bool {
        !self.asked.is_empty()
    }

    /// Takes in what the pool fetched. `typed` is the text setting (the sample typed, the preset
    /// and the language filtered by), for the pause before typed characters are asked for.
    pub(crate) fn begin_frame(&mut self, ctx: &Context, typed: String) {
        self.frame += 1;
        self.now = ctx.input(|i| i.time);
        if !self.armed {
            self.armed = true;
            let ctx = ctx.clone();
            self.pool = self.pool.take().map(|pool| pool.on_ready(move || ctx.request_repaint()));
        }
        if typed != self.typed.0 {
            self.typed = (typed, self.now);
        }
        if self.read_at.is_none_or(|at| !(0.0..SETTINGS_RECHECK_SECS).contains(&(self.now - at))) {
            self.read_at = Some(self.now);
            self.read_settings();
        }
        let Some(pool) = &self.pool else { return };
        for preview in pool.poll() {
            // A result for a key no longer asked for still fills the cache in memory.
            self.asked.remove(&preview.key);
            match preview.font {
                Ok(font) => {
                    self.failed.remove(&preview.key);
                    self.fonts.insert(preview.key, (Arc::new(font), self.frame));
                }
                Err(e) => {
                    log::debug!("preview {}: {e}", preview.key.path());
                    if e == PreviewError::NotFound && matches!(preview.key.subset(), Subset::Sample(_)) {
                        self.sample_missing.insert(preview.key);
                    } else {
                        self.failed.insert(preview.key, self.now);
                    }
                }
            }
        }
    }

    /// The subset `row` draws with: the one its text needs, else while that loads the one its
    /// face was last drawn with. `None` without a service, and for a row whose subset failed:
    /// the row then shows what it shows without the service.
    pub(crate) fn font(&mut self, index: &Index, row: &Row<'_>) -> Option<(PreviewKey, Arc<Vec<u8>>)> {
        if !self.available {
            return None;
        }
        let want = self.key(index, row)?;
        if self.has_failed(&want) {
            return None;
        }
        let frame = self.frame;
        if let Some((font, used)) = self.fonts.get_mut(&want) {
            *used = frame;
            let font = Arc::clone(font);
            self.shown.insert(want.sha256().to_owned(), want.clone());
            return Some((want, font));
        }
        let last = self.shown.get(want.sha256())?;
        let (font, used) = self.fonts.get_mut(last)?;
        *used = frame;
        Some((last.clone(), Arc::clone(font)))
    }

    /// The rows to pass to [`Subsets::request`] when `visible` of `count` rows are on screen:
    /// those and one screen above and two below.
    pub(crate) fn window(visible: &Range<usize>, count: usize) -> Range<usize> {
        let screen = visible.len();
        visible.start.saturating_sub(screen)..visible.end.saturating_add(2 * screen).min(count)
    }

    /// Asks for the subsets of the list's rows from `start` on (`None` for a row drawn without
    /// the service, such as an installed family), `visible` of them on screen, and cancels those
    /// no longer wanted.
    pub(crate) fn request(&mut self, ctx: &Context, index: &Index, start: usize, rows: &[Option<Row<'_>>], visible: Range<usize>) {
        if !self.available {
            return;
        }
        let row = |n: usize| rows.get(n.checked_sub(start)?)?.as_ref();
        let mut wanted: HashMap<PreviewKey, u32> = HashMap::new();
        let screen = visible.len();
        let paused = self.now - self.typed.1 >= PAUSE_SECS;
        let mut settled = true;
        let mut latin = Vec::new();
        for row in visible.clone().filter_map(row) {
            // A family without a sample text still gets the subset its row's text needs.
            if let Some(sample) = self.sample_key(index, row.family) {
                settled &= self.fonts.contains_key(&sample) || self.has_failed(&sample);
                self.want(&mut wanted, sample, VISIBLE);
            }
            if let Some(ascii) = covers_latin(row.family).then(|| subset_key(row.family, Subset::Ascii)).flatten() {
                latin.push(ascii);
            }
            match self.key(index, row) {
                Some(k) if matches!(k.subset(), Subset::Codepoints(_)) && !paused => {}
                Some(k) => self.want(&mut wanted, k, VISIBLE),
                None => {}
            }
        }
        if settled {
            for ascii in latin {
                self.want(&mut wanted, ascii, BACKGROUND);
            }
        }
        let above = visible.start.saturating_sub(screen)..visible.start;
        let below = visible.end..visible.end.saturating_add(2 * screen);
        for row in above.chain(below).filter_map(row) {
            if let Some(sample) = sample_subset(index, row.family).filter(|k| !self.sample_missing.contains(k)) {
                self.want(&mut wanted, sample, WINDOW);
            }
        }
        if !paused {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(PAUSE_SECS - (self.now - self.typed.1)));
        }

        let Some(pool) = &self.pool else { return };
        if self.asked.keys().any(|k| !wanted.contains_key(k)) {
            pool.retain(|k| wanted.contains_key(k));
            self.asked.retain(|k, _| wanted.contains_key(k));
        }
        for (key, priority) in wanted {
            if self.asked.get(&key) != Some(&priority) {
                pool.request(key.clone(), priority);
                self.asked.insert(key, priority);
            }
        }
        self.trim();
    }

    /// The sample subset of `family`'s face, or when the service doesn't have it for this text
    /// its [`sample_fallback`]. `None` when there is neither.
    fn sample_key(&self, index: &Index, family: &Family) -> Option<PreviewKey> {
        let sample = sample_subset(index, family)?;
        if self.sample_missing.contains(&sample) { sample_fallback(index, family) } else { Some(sample) }
    }

    fn key(&self, index: &Index, row: &Row<'_>) -> Option<PreviewKey> {
        key_of(index, row, &self.sample_missing)
    }

    /// Adds `key` to `wanted` at `priority` unless it is here, failed lately, or can't load:
    /// with previews off, only what the disk cache has.
    fn want(&mut self, wanted: &mut HashMap<PreviewKey, u32>, key: PreviewKey, priority: u32) {
        if let Some((_, used)) = self.fonts.get_mut(&key) {
            *used = self.frame;
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

    /// Cancels everything asked for, as when the list isn't shown.
    pub(crate) fn cancel_all(&mut self) {
        if self.asked.is_empty() {
            return;
        }
        if let Some(pool) = &self.pool {
            pool.retain(|_| false);
        }
        self.asked.clear();
    }

    /// Drops the subsets wanted least recently past [`MAX_HELD`].
    fn trim(&mut self) {
        if self.fonts.len() <= MAX_HELD {
            return;
        }
        let mut by_use: Vec<(u64, PreviewKey)> = self.fonts.iter().map(|(k, (_, used))| (*used, k.clone())).collect();
        by_use.sort_by_key(|(used, _)| *used);
        for (_, key) in by_use.into_iter().take(self.fonts.len() - MAX_HELD) {
            self.fonts.remove(&key);
        }
        self.shown.retain(|_, k| self.fonts.contains_key(k));
    }
}

fn subset_key(family: &Family, subset: Subset) -> Option<PreviewKey> {
    PreviewKey::new(&list_file(family)?.sha256, Instance::List, subset)
}

fn covers_latin(family: &Family) -> bool {
    family.scripts.iter().any(|s| s == "Latn")
}

/// The sample subset of `family`'s list face for the text `index` gives it.
fn sample_subset(index: &Index, family: &Family) -> Option<PreviewKey> {
    let text = index.sample_text(family, list_file(family)?)?;
    subset_key(family, Subset::Sample(SampleHash::of(text)))
}

/// The subset that draws `row`'s text: the sample when the sample subset has every character
/// (the sentence and the family name), else ASCII for printable ASCII in a Latin face, else the
/// text's own characters. A sample the service doesn't have gives [`sample_fallback`] instead.
/// `None` for a family without files, or text of more characters than a request may name.
fn key_of(index: &Index, row: &Row<'_>, missing: &HashSet<PreviewKey>) -> Option<PreviewKey> {
    let family = row.family;
    let file = list_file(family)?;
    let text = row.text.as_str();
    let sample = index.sample_text(family, file);
    let in_sample = |c: char| c == ' ' || sample.is_some_and(|s| s.contains(c)) || family.family.contains(c);
    if sample == Some(text) || (sample.is_some() && text.chars().all(in_sample)) {
        let key = sample_subset(index, family)?;
        if !missing.contains(&key) {
            return Some(key);
        }
        if let Some(fallback) = sample_fallback(index, family) {
            return Some(fallback);
        }
    }
    let subset = if covers_latin(family) && text.chars().all(|c| matches!(c, ' '..='~')) {
        Subset::Ascii
    } else {
        Subset::Codepoints(Codepoints::from_text(text))
    };
    PreviewKey::new(&file.sha256, Instance::List, subset)
}

/// The subset of the sample text's characters and the family name, which the sample subset
/// holds too. `None` when that is more characters than a request may name.
fn sample_fallback(index: &Index, family: &Family) -> Option<PreviewKey> {
    let text = index.sample_text(family, list_file(family)?)?;
    subset_key(family, Subset::Codepoints(Codepoints::from_text(&format!("{text}{}", family.family))))
}
