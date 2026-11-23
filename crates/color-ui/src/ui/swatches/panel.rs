//! `SwatchesPanel`: the panel body, its state and what it reports.

use craft_cms::convert::CmykConverter;
use craft_i18n::Format;
use craft_theme::Srgba;
use craft_theme::variant::Tone;
use craft_ui::bars::StatusMessage;
use craft_ui::contract::{Bind, Held, NoValue, Outcome, Pattern, ShowContainer as _, request_rename};
use craft_ui::layout_components::Stack;
use craft_ui::patterns::presets::{PresetBrowser, PresetEvent, PresetFileEvent, PresetFooterOptions, PresetReorder, PresetWords};
use craft_ui::runtime::env::{Count, Env};
use egui::{Response, Ui};

use super::library::{find_swatch, has};
use super::menu::event_edits;
use super::{SWATCH_ZOOM, SwatchAt, SwatchEdit, SwatchKey, SwatchesFileOutput, SwatchesState, SwatchesWords, swatch_source};
use crate::palette::{SwatchGroup, SwatchNote, SwatchReport};

/// What [`SwatchesPanel::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the swatch and runs the edits"]
pub struct SwatchesPanelOutput {
    /// The panel's response.
    pub response: Response,
    /// A swatch was clicked: its colour (through the document's converter) for the foreground, or for the background
    /// when Alt (Ctrl or Cmd too, as PhotoCraft) was held.
    pub used: Option<(SwatchAt, Srgba, bool)>,
    /// The edits the user made this pass, in pass order, for the host to run; the panel never writes the library.
    pub edits: Vec<SwatchEdit>,
}

impl Outcome for SwatchesPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The Swatches panel on the library's groups, built on `PresetBrowser`: groups as folders, swatches as flat touching
/// squares with no gap or corner radius (Photoshop), the size the browser's zoom chooses.
///
/// - Click uses the swatch for the foreground; Alt-click (or Ctrl or Cmd) for the background. The swatch is selected.
///   Double-click or F2 renames in place; Delete removes; dragging onto a group moves it. Swatches do not drop onto
///   the canvas.
/// - The footer's + is New Swatch: the foreground colour into the selected swatch's group, named "Color Swatch N", then
///   renamed in place.
/// - Every change of the library comes back as a [`SwatchEdit`] with resolved names and positions; the host runs it.
/// - Tooltips name the swatch and its value; AccessKit as the preset browser's grid, each swatch named.
/// - The files of Import, Replace and Export go out as `PresetFileRequest`s from the panel menu; the host answers
///   them with [`SwatchesPanel::apply_file`].
/// - It fills the room it is given, and its browser scrolls the swatches: show it outside a body that scrolls.
#[must_use = "show the panel with `show`"]
#[derive(Pattern)]
pub struct SwatchesPanel<'a> {
    groups: &'a [SwatchGroup],
    state: Bind<'a, SwatchesState>,
    cmyk: Option<&'a CmykConverter>,
    foreground: Srgba,
}

/// What the body decided; `show` stores the state and reports the rest after the real run.
struct Settled {
    before_state: SwatchesState,
    st: SwatchesState,
    used: Option<(SwatchAt, Srgba, bool)>,
    edits: Vec<SwatchEdit>,
}

impl<'a> SwatchesPanel<'a> {
    /// The panel over the library's `groups` and the host-owned `state`; `foreground` is what New Swatch adds.
    pub fn new(groups: &'a [SwatchGroup], state: Bind<'a, SwatchesState>, foreground: Srgba) -> Self {
        SwatchesPanel { groups, state, cmyk: None, foreground }
    }

    /// The document's CMYK converter, which draws CMYK swatches and gives their colours when used; without it they
    /// use the built-in coated CMYK.
    pub fn cmyk(mut self, converter: &'a CmykConverter) -> Self {
        self.cmyk = Some(converter);
        self
    }

    /// Draws the panel and returns the user's actions this pass.
    pub fn show(self, ui: &mut Ui) -> SwatchesPanelOutput {
        let SwatchesPanel { groups, state, cmyk, foreground } = self;
        let cmyk = cmyk.unwrap_or_else(|| CmykConverter::coated());
        let words = Env::of(ui).words::<SwatchesWords>();
        let mut held_state = Held::new(state);
        let ctx = ui.ctx().clone();
        let rename_scope = ui.scope_id().with("browser-rows");
        let mut shown = Stack::new().fill().show(ui, |ui, cx| {
            let before_state = held_state.read(cx)?;
            let mut st = before_state.clone();
            if st.selected.as_ref().is_some_and(|k| !has(groups, k)) {
                st.selected = None;
            }
            let source = swatch_source(groups, cmyk);
            let out = {
                let mut browser = PresetBrowser::new(words.title.clone(), Bind::bound(&mut st.browser), &source)
                    .thumb_range(SWATCH_ZOOM)
                    .toolbar(true)
                    .reorder(PresetReorder::items_and_groups())
                    .footer(PresetFooterOptions::new().can_delete(st.selected.is_some()));
                if let Some(key) = &st.selected {
                    browser = browser.selected(key.clone());
                }
                browser.show(ui)
            };
            let mut used = None;
            let mut edits = Vec::new();
            for event in out.events {
                if let PresetEvent::Select { key: SwatchKey::Swatch(name), mods, .. } = &event
                    && let Some(at) = find_swatch(groups, name)
                    && let Some(entry) = groups.get(at.group).and_then(|g| g.entries.get(at.index))
                {
                    used = Some((at, entry.color.to_srgb(cmyk), mods.alt || mods.cmd || mods.ctrl));
                }
                edits.extend(event_edits(groups, &mut st, event, foreground, &words));
            }
            Some(Settled { before_state, st, used, edits })
        });
        // Once per pass, after the real run: the rename request and the edited state are stored, the edits reported.
        let (used, edits) = shown
            .settle(|cx, settled| {
                let Settled { before_state, mut st, used, edits } = settled.take()?;
                if let Some(key) = st.rename.take() {
                    request_rename(&ctx, rename_scope, &key);
                }
                if st != before_state {
                    held_state.write(cx, &before_state, st, words.title.as_str());
                }
                Some((used, edits))
            })
            .flatten()
            .unwrap_or_default();
        SwatchesPanelOutput { response: shown.output, used, edits }
    }

    /// Carries out the host's answer to a file request of the panel menu: Import and Replace read the file into an
    /// `Import` edit for the host (which makes names unique), a finished export reports the notes it kept, and a
    /// failure says why. Returns the edit, the report and the status message the host shows, if any.
    pub fn apply_file(
        state: &mut SwatchesState,
        event: &PresetFileEvent,
        words: &SwatchesWords,
        presets: &PresetWords,
        format: &Format,
    ) -> SwatchesFileOutput {
        let mut output = SwatchesFileOutput { edit: None, report: None, status: None };
        match event {
            PresetFileEvent::Loaded { file_name, bytes, replace, .. } => match crate::palette::read_swatches(bytes, file_name) {
                Ok((groups, report)) => {
                    output.edit = Some(SwatchEdit::Import { groups, replace: *replace });
                    output.status = Self::notes_status(&report, false, words, format);
                    output.report = Some(report);
                }
                Err(error) => {
                    output.status = Some(StatusMessage::new(words.import_failed.fill(&[("reason", &error.to_string())])).tone(Tone::Bad));
                }
            },
            PresetFileEvent::Saved { .. } => {
                if let Some(report) = state.export_notes.take() {
                    output.status = Self::notes_status(&report, true, words, format);
                    output.report = Some(report);
                }
            }
            PresetFileEvent::Failed { .. } => {
                state.export_notes = None;
                output.status = Some(StatusMessage::new(event.status(presets)).tone(Tone::Bad));
            }
            _ => {}
        }
        output
    }

    /// The notes of a swatch file read or written as sentences; counts follow the locale's plural rules. `writing`
    /// tells an export's notes from an import's, which word a colour-book colour differently.
    pub fn note_lines(report: &SwatchReport, writing: bool, words: &SwatchesWords, format: &Format) -> Vec<String> {
        let count = |row: &Count, n: usize| row.text(n as u64, &format.integer(n as u64));
        report
            .notes
            .iter()
            .map(|note| match *note {
                SwatchNote::BookColors(n) if writing => count(&words.book_left_out, n),
                SwatchNote::BookColors(n) => count(&words.book_read, n),
                SwatchNote::Skipped(n) => count(&words.skipped, n),
                SwatchNote::GroupsFlattened => words.groups_flattened.as_str().to_owned(),
                SwatchNote::ConvertedToRgb(n) => count(&words.converted, n),
                SwatchNote::KindsDropped(n) => count(&words.kinds_dropped, n),
                SwatchNote::Truncated(n) => count(&words.truncated, n),
                SwatchNote::NamesCut(n) => count(&words.names_cut, n),
                SwatchNote::TransparentKept(n) => count(&words.transparent_kept, n),
            })
            .collect()
    }

    /// The status message for the notes of an import (or, with `writing`, an export): the title and the notes in one line, as
    /// a warning; `None` when there is nothing to say.
    fn notes_status(report: &SwatchReport, writing: bool, words: &SwatchesWords, format: &Format) -> Option<StatusMessage> {
        let lines = Self::note_lines(report, writing, words, format);
        if lines.is_empty() {
            return None;
        }
        let title = if writing { &words.notice_export } else { &words.notice_import };
        Some(StatusMessage::new(format!("{}: {}", title.as_str(), lines.join(" "))).tone(Tone::Warn))
    }
}
