//! [`LutPicker`] (F04.04): the 3D LUT choice of a Color Lookup adjustment, as PhotoCraft's: Load
//! 3D LUT…, None, the built-in looks ([`craft_cms::lutfile::BUILTIN`]), the files of the app's
//! LUT folder, and the loaded LUT. Load 3D LUT… picks the file through the host's
//! [`Files`](craft_host::Files) and reads and parses it as a [`Jobs`](craft_host::Jobs) job.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use craft_behavior::{EditEvent, EditEvents, Metrics, Mixed, Proposal, Sense};
use craft_cms::lutfile::{LutError, LutFile};
use craft_host::{PickId, PickRequest, Picked};
use craft_ui::contract::{
    Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Entry, Job, MeasureCx, Outcome, PaintUi, Pending, Remember as _, Role,
    WordSetCx as _,
};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, StyleCx};
use egui::{Response, Vec2};

#[cfg(feature = "screen-testing")]
mod stories;

craft_ui::words! {
    /// The LUT picker's rows, English by default, and the built-in looks' names in
    /// [`BUILTIN`](craft_cms::lutfile::BUILTIN)'s order.
    #[non_exhaustive]
    pub struct LutWords {
        label: "lut.label" = "3D LUT File";
        load: "lut.load" = "Load 3D LUT…";
        none: "lut.none" = "None";
        /// The loaded LUT's row; `{name}` its title, `{size}` its grid size ("Film (33³)").
        custom: template "lut.custom" = "{name} ({size}³)";
        warm: "lut.builtinWarm" = "Warm Filter";
        cool: "lut.builtinCool" = "Cool Filter";
        teal_orange: "lut.builtinTealOrange" = "Teal & Orange";
        bleach_bypass: "lut.builtinBleachBypass" = "Bleach Bypass";
        faded_film: "lut.builtinFadedFilm" = "Faded Film";
        day_for_night: "lut.builtinDayForNight" = "Day for Night";
        mono_contrast: "lut.builtinMonoContrast" = "Mono High Contrast";
        cross_process: "lut.builtinCrossProcess" = "Cross Process";
    }
}

/// The chosen LUT.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LutChoice {
    /// No LUT.
    None,
    /// A built-in look by id ([`craft_cms::lutfile::builtin`]).
    Builtin(&'static str),
    /// A file of the LUT folder; the app reads and embeds it.
    Folder(PathBuf),
    /// The LUT the layer holds, loaded before: its title and grid size.
    #[non_exhaustive]
    Loaded {
        /// The LUT's title.
        name: String,
        /// The grid size: points per side.
        size: usize,
    },
}

impl LutChoice {
    /// The LUT the layer holds, called `name`, with `size` points per side.
    pub fn loaded(name: impl Into<String>, size: usize) -> Self {
        LutChoice::Loaded { name: name.into(), size }
    }
}

type FolderFiles = Vec<(String, PathBuf)>;

/// What [`LutPicker`] returns.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the choice"]
pub struct LutPickerOutput {
    /// A row was chosen. A step of the wheel or the arrow keys over the closed field reports only None and the
    /// built-ins; a folder file's LUT is read only on the committed pick.
    pub edit: EditResponse<LutChoice>,
    /// The LUT file Load 3D LUT… picked, once it is read and parsed, or why it could not be: the
    /// app embeds the LUT and sets the choice to [`LutChoice::Loaded`]. The choice itself does not
    /// change until the app does. A cancelled picker reports nothing.
    pub loaded: Option<Result<LutFile, LutError>>,
}

impl Outcome for LutPickerOutput {
    type Value = LutChoice;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}

/// The style of the picker: no look of its own, the select is a child.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct LutPickerStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// A row of the select: Load 3D LUT… or a choice.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Row {
    Load,
    Choice(LutChoice),
}

/// How often the LUT folder is scanned again, in seconds.
const RESCAN: u64 = 2;

/// The purpose of the LUT file picker: its folder is remembered under it.
const PICK_PURPOSE: &str = "lut";

/// Where Load 3D LUT… is: the picker open, or the chosen file being read.
#[derive(Clone, Default)]
enum Load {
    #[default]
    Idle,
    Picking(PickId),
    Reading(PickId, Source),
}

/// What the chosen file is read from.
#[derive(Clone)]
enum Source {
    Path(PathBuf),
    Contents(String, Arc<Vec<u8>>),
}

/// The scan and the load kept between passes.
#[derive(Clone, Default)]
struct LutState {
    folder: Option<PathBuf>,
    files: Arc<FolderFiles>,
    load: Load,
}

/// A parsed file as a job result.
#[derive(Clone)]
struct Parsed(Arc<Result<LutFile, LutError>>);

impl Default for Parsed {
    fn default() -> Self {
        Parsed(Arc::new(Err(LutError(String::new()))))
    }
}

/// The seconds since the first scan of the process, for the rescan interval.
fn seconds() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs()
}

/// A select of LUTs.
///
/// - Rows: Load 3D LUT…, None, the eight built-in looks, the LUT folder's files by name (in the language's
///   order), and the loaded LUT ("name (33³)") when the layer holds one.
/// - Load 3D LUT… asks the host's file service for a `.cube`, `.3dl` or `.look` file (purpose
///   `"lut"`) and answers in [`LutPickerOutput::loaded`] on the pass the file is parsed. Without
///   a host it does nothing.
/// - The folder is scanned as a host job at most every 2 seconds (a network share can be
///   slow); until a scan ends the list shows the last one. Another folder drops the old list at
///   once. Web has no folder.
/// - The wheel and the arrow keys over the closed field step through None and the built-ins
///   only (a folder file would be read from disk at each step).
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component]
pub struct LutPicker<'a> {
    #[common]
    common: Common,
    value: Bind<'a, LutChoice>,
    /// The app's LUT folder (Preferences › File Handling › LUT Folder).
    #[prop(into)]
    folder: Option<PathBuf>,
    loaded: Option<Result<LutFile, LutError>>,
    load_asked: bool,
}

impl<'a> LutPicker<'a> {
    /// A picker over `value`.
    pub fn new(value: Bind<'a, LutChoice>) -> Self {
        LutPicker { common: Common::new(), value, folder: None, loaded: None, load_asked: false }
    }

    fn entries(words: &LutWords, current: Option<&LutChoice>, files: &FolderFiles) -> Vec<Entry<Row>> {
        let mut entries = vec![Entry::new(Row::Load, words.load.clone()), Entry::new(Row::Choice(LutChoice::None), words.none.clone())];
        for (id, _) in craft_cms::lutfile::BUILTIN {
            entries.push(Entry::new(Row::Choice(LutChoice::Builtin(id)), builtin_name(words, id)));
        }
        let mut listed = false;
        for (name, path) in files {
            listed |= matches!(current, Some(LutChoice::Folder(held)) if held == path);
            entries.push(Entry::new(Row::Choice(LutChoice::Folder(path.clone())), Label::from(name.clone())));
        }
        // The file in use while a scan has not found it yet (or the folder changed under it).
        if let Some(LutChoice::Folder(path)) = current
            && !listed
        {
            let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
            entries.push(Entry::new(Row::Choice(LutChoice::Folder(path.clone())), Label::from(name)));
        }
        if let Some(LutChoice::Loaded { name, size }) = current {
            let text = words.custom.fill(&[("name", name), ("size", &size.to_string())]);
            entries.push(Entry::new(Row::Choice(current.cloned().unwrap_or(LutChoice::None)), Label::from(text)));
        }
        entries
    }

    fn select<'e>(words: &LutWords, shown: Mixed<Row>, entries: &'e [Entry<Row>]) -> Select<'e, Row> {
        Select::new(words.label.clone(), Bind::given(shown), entries)
    }
}

impl Component for LutPicker<'_> {
    const SINGLE_VALUE: bool = true;
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = LutPickerStyle;
    type Output = LutPickerOutput;

    fn style(_: &StyleCx<'_>, _: LutPickerStyleKey) -> LutPickerStyle {
        LutPickerStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let words = cx.words::<LutWords>();
        let entries = Self::entries(&words, cx.value(), &Vec::new());
        let mut select = Self::select(&words, Mixed::Mixed, &entries);
        cx.child("select", &mut select, Proposal::Ideal)
    }

    fn bind(&mut self) -> Option<BindMut<'_, LutChoice>> {
        Some(self.value.reborrow())
    }

    /// Moves the scan and Load 3D LUT… along: a finished scan replaces the list, a new scan starts at most every
    /// [`RESCAN`] seconds, the picker's answer starts the read and a finished read is reported.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let host = cx.env.host().cloned();
        let mut state = cx.memory::<LutState>().clone();
        if state.folder != self.folder {
            state = LutState { folder: self.folder.clone(), load: state.load, ..LutState::default() };
        }
        if let Some(folder) = self.folder.clone() {
            let key = ("lut-scan", folder.clone(), seconds() / RESCAN);
            let started = folder.clone();
            if let Pending::Ready(found) = cx.pending(key, move || Job::new(move |_, _| Ok::<_, ()>(Arc::new(list_folder(&started))))) {
                let mut files = (**found).clone();
                cx.env.collator().sort_by_key(&mut files, |(name, _)| name.as_str());
                state.files = Arc::new(files);
            }
        }
        if let Some(host) = host {
            match state.load.clone() {
                Load::Idle => {}
                Load::Picking(id) => {
                    if let Some(picked) = host.files().take_answer(id) {
                        state.load = match picked {
                            Picked::Paths(paths) => paths.into_iter().next().map_or(Load::Idle, |path| Load::Reading(id, Source::Path(path))),
                            Picked::Contents(contents) => contents
                                .into_iter()
                                .next()
                                .map_or(Load::Idle, |(name, bytes)| Load::Reading(id, Source::Contents(name, Arc::new(bytes)))),
                            _ => Load::Idle,
                        };
                    }
                }
                Load::Reading(id, source) => {
                    let reader = Arc::clone(&host);
                    let done = match cx.pending(("lut-load", id.0), move || {
                        Job::new(move |_, _| {
                            let parsed = match source {
                                Source::Path(path) => reader
                                    .files()
                                    .read(&path)
                                    .map_err(|e| LutError(e.to_string()))
                                    .and_then(|bytes| craft_cms::lutfile::parse(&file_name(&path), &bytes)),
                                Source::Contents(name, bytes) => craft_cms::lutfile::parse(&name, &bytes),
                            };
                            Ok::<_, ()>(Parsed(Arc::new(parsed)))
                        })
                    }) {
                        Pending::Ready(parsed) => Some((*parsed.0).clone()),
                        Pending::Failed(()) | Pending::Lost => Some(Err(LutError(String::new()))),
                        _ => None,
                    };
                    if let Some(result) = done {
                        state.load = Load::Idle;
                        self.loaded = Some(result);
                    }
                }
            }
        }
        *cx.memory::<LutState>() = state;
    }

    /// The select, with the rows of the last scan. A pick of a row is the component's edit; a pick of Load 3D LUT…
    /// keeps the value and asks for the file.
    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let words = cx.words::<LutWords>();
        let state = cx.recall::<LutState>();
        let entries = Self::entries(&words, cx.value(), &state.files);
        let shown = cx.value().map_or(Mixed::Mixed, |value| Mixed::Same(Row::Choice(value.clone())));
        let out = ui.child("select", cx.rect, Self::select(&words, shown, &entries));
        let mut adopted = EditEvents::new();
        for event in &out.events {
            match event.value() {
                Some(Row::Load) => self.load_asked |= matches!(event, EditEvent::Commit { .. }),
                Some(Row::Choice(choice)) => {
                    let step = matches!(event, EditEvent::Set { .. });
                    if !step || matches!(choice, LutChoice::None | LutChoice::Builtin(_)) {
                        adopted.push(event.clone());
                    }
                }
                None => {}
            }
        }
        let adopted = EditResponse::lifted(out.response().clone(), adopted, None);
        cx.adopt(&adopted, &LutChoice::None, |value, mine| {
            if let Row::Choice(choice) = value {
                mine.clone_from(choice);
            }
        });
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> LutPickerOutput {
        if self.load_asked
            && let Some(host) = cx.env.host().cloned()
        {
            if let Load::Picking(old) = cx.memory::<LutState>().load {
                host.files().forget(old);
            }
            let title = cx.words::<LutWords>().load.as_str().trim_end_matches('…').to_owned();
            let request = PickRequest::open(PICK_PURPOSE).title(title).filter("3D LUT", &["cube", "3dl", "look"]);
            cx.memory::<LutState>().load = Load::Picking(host.files().pick(request));
        }
        LutPickerOutput { edit: cx.edit_response(), loaded: self.loaded }
    }
}

fn builtin_name(words: &LutWords, id: &str) -> Label {
    match id {
        "warm" => words.warm.clone(),
        "cool" => words.cool.clone(),
        "tealOrange" => words.teal_orange.clone(),
        "bleachBypass" => words.bleach_bypass.clone(),
        "fadedFilm" => words.faded_film.clone(),
        "dayForNight" => words.day_for_night.clone(),
        "monoContrast" => words.mono_contrast.clone(),
        "crossProcess" => words.cross_process.clone(),
        other => Label::from(other.to_owned()),
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}
/// Most files [`list_folder`] lists.
const MAX_FILES: usize = 500;
/// Largest file [`list_folder`] lists: a 129³ `.cube` is about 60 MB.
const MAX_BYTES: u64 = 64 << 20;

/// The `.cube`, `.3dl` and `.look` files directly in `dir` as (file name, path), sorted by name
/// without regard to case. An unreadable folder lists nothing; hidden files, subfolders and files
/// too large to be a LUT are skipped, and a symlink to a LUT file is listed.
fn list_folder(dir: &Path) -> FolderFiles {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: FolderFiles = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let extension = name.rsplit_once('.')?.1.to_ascii_lowercase();
            if name.starts_with('.') || !matches!(extension.as_str(), "cube" | "3dl" | "look") {
                return None;
            }
            let path = entry.path();
            let meta = std::fs::metadata(&path).ok()?;
            (meta.is_file() && meta.len() <= MAX_BYTES).then_some((name, path))
        })
        .collect();
    out.sort_by_key(|(name, _)| name.to_lowercase());
    out.truncate(MAX_FILES);
    out
}
