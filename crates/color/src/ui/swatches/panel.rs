//! `SwatchesPanel`: the panel body, its state and what it reports.

use std::collections::HashMap;

use craft_cms::convert::CmykConverter;
use craft_i18n::Format;
use craft_theme::Srgba;
use craft_theme::variant::Tone;
use craft_ui::choice::SwatchFace;
use craft_ui::contract::{Bind, Held, NoValue, Outcome, Pattern, ShowContainer as _, request_rename};
use craft_ui::layout_components::Stack;
use craft_ui::patterns::presets::{PresetBrowser, PresetFileEvent, PresetFooterOptions, PresetReorder, PresetRow, PresetWords, ThumbInfo};
use craft_ui::primitives::ContentPainterCx;
use craft_ui::runtime::env::{Count, Env};
use craft_ui::runtime::toast::Toast;
use egui::{Rect, Response, Ui};

use super::library::value_text;
use super::menu::Actions;
use super::{SwatchAt, SwatchKey, SwatchLibrary, SwatchesFileOutput, SwatchesState, SwatchesWords};
use crate::palette::{SwatchColor, SwatchNote, SwatchReport};

/// What [`SwatchesPanel::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the swatch"]
pub struct SwatchesPanelOutput {
    /// The panel's response.
    pub response: Response,
    /// A swatch was clicked: its colour (through the document's converter) for the foreground, or for the background
    /// when Alt (Ctrl or Cmd too, as PhotoCraft) was held.
    pub used: Option<(SwatchAt, Srgba, bool)>,
    /// The library changed (new, rename, delete, move, group edits): the app saves it.
    pub changed: bool,
}

impl Outcome for SwatchesPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The Swatches panel on a [`SwatchLibrary`], built on `PresetBrowser`: groups as folders, swatches as flat touching
/// squares with no gap or corner radius (Photoshop), the size the browser's zoom chooses.
///
/// - Click uses the swatch for the foreground; Alt-click (or Ctrl or Cmd) for the background. The swatch is selected.
///   Double-click or F2 renames in place; Delete removes; dragging onto a group moves it. Swatches do not drop onto
///   the canvas.
/// - The footer's + is New Swatch: the foreground colour into the selected swatch's group, named "Color Swatch N", then
///   renamed in place.
/// - Tooltips name the swatch and its value; AccessKit as the preset browser's grid, each swatch named.
/// - The files of Import, Replace and Export go out as [`PresetFileRequest`]s from the panel menu; the host answers
///   them with [`SwatchesPanel::apply_file`].
#[must_use = "show the panel with `show`"]
#[derive(Pattern)]
pub struct SwatchesPanel<'a> {
    library: Bind<'a, SwatchLibrary>,
    state: Bind<'a, SwatchesState>,
    cmyk: Option<&'a CmykConverter>,
    foreground: Srgba,
}

/// What the body decided; `show` stores it after the real run.
struct Settled {
    before: SwatchLibrary,
    lib: SwatchLibrary,
    before_state: SwatchesState,
    st: SwatchesState,
    used: Option<(SwatchAt, Srgba, bool)>,
    changed: bool,
}

impl<'a> SwatchesPanel<'a> {
    /// The panel over `library` and the host-owned `state`; `foreground` is what New Swatch adds.
    pub fn new(library: Bind<'a, SwatchLibrary>, state: Bind<'a, SwatchesState>, foreground: Srgba) -> Self {
        SwatchesPanel { library, state, cmyk: None, foreground }
    }

    /// The document's CMYK converter, which draws CMYK swatches and gives their colours when used; without it they
    /// use the built-in coated CMYK.
    pub fn cmyk(mut self, converter: &'a CmykConverter) -> Self {
        self.cmyk = Some(converter);
        self
    }

    /// Draws the panel and returns the user's actions this pass.
    pub fn show(self, ui: &mut Ui) -> SwatchesPanelOutput {
        let SwatchesPanel { library, state, cmyk, foreground } = self;
        let cmyk = cmyk.unwrap_or_else(|| CmykConverter::coated());
        let words = Env::of(ui).words::<SwatchesWords>();
        let mut held_library = Held::new(library);
        let mut held_state = Held::new(state);
        let ctx = ui.ctx().clone();
        let rename_scope = ui.scope_id().with("browser-rows");
        let mut shown = Stack::new().show(ui, |ui, cx| {
            let (Some(before), Some(before_state)) = (held_library.read(cx), held_state.read(cx)) else { return None };
            let (mut used, mut changed) = (None, false);
            let (mut lib, mut st) = (before.clone(), before_state.clone());
            if st.selected.as_ref().is_some_and(|k| !lib.has(k)) {
                st.selected = None;
            }
            let mut rows = Vec::with_capacity(lib.total() + lib.groups.len());
            for group in &lib.groups {
                rows.push(PresetRow::group(SwatchKey::Group(group.name.clone()), group.name.as_str(), 0).count(group.entries.len()));
                for entry in &group.entries {
                    let named = format!("{}, {}", entry.name, value_text(&entry.color, words.book_color.as_str()));
                    rows.push(
                        PresetRow::item(SwatchKey::Swatch(entry.name.clone()), entry.name.as_str(), 1).tooltip(named.clone()).accessible_name(named),
                    );
                }
            }
            let out = {
                let colors: HashMap<&str, &SwatchColor> =
                    lib.groups.iter().flat_map(|g| g.entries.iter().map(|e| (e.name.as_str(), &e.color))).collect();
                let mut browser = PresetBrowser::new(words.title.clone(), Bind::bound(&mut st.browser), &rows)
                    .toolbar(true)
                    .reorder(PresetReorder::items_and_groups())
                    .footer(PresetFooterOptions::new().can_delete(st.selected.is_some()))
                    .thumb(|cx: &ContentPainterCx<'_>, rect: Rect, info: ThumbInfo<'_, SwatchKey>| {
                        if let SwatchKey::Swatch(name) = info.key
                            && let Some(color) = colors.get(name.as_str())
                        {
                            SwatchFace::new(color.to_srgb(cmyk)).selected(info.selected).paint(cx, rect);
                        }
                    });
                if let Some(key) = &st.selected {
                    browser = browser.selected(key.clone());
                }
                browser.show(ui)
            };
            let mut actions = Actions {
                library: &mut lib,
                st: &mut st,
                used: &mut used,
                changed: &mut changed,
                foreground,
                template: words.swatch_n.as_str(),
                group_name: words.group.as_str(),
                cmyk,
            };
            for event in out.events {
                actions.event(event);
            }
            Some(Settled { before, lib, before_state, st, used, changed })
        });
        // Once per pass, after the real run: the rename request and the edited library and state are stored.
        let (used, changed) = shown
            .settle(|cx, settled| {
                let Settled { before, lib, before_state, mut st, used, changed } = settled.take()?;
                if let Some(key) = st.rename.take() {
                    request_rename(&ctx, rename_scope, &key);
                }
                if changed {
                    held_library.write(cx, &before, lib, words.title.as_str());
                }
                if st != before_state {
                    held_state.write(cx, &before_state, st, words.title.as_str());
                }
                Some((used, changed))
            })
            .flatten()
            .unwrap_or_default();
        SwatchesPanelOutput { response: shown.output, used, changed }
    }

    /// Carries out the host's answer to a file request of the panel menu: Import and Replace read the file into
    /// `library`, a finished export reports the notes it kept, and a failure says why. Returns what changed and the
    /// toast the host posts, if any.
    pub fn apply_file(
        library: &mut SwatchLibrary,
        state: &mut SwatchesState,
        event: &PresetFileEvent,
        words: &SwatchesWords,
        presets: &PresetWords,
        format: &Format,
    ) -> SwatchesFileOutput {
        let mut output = SwatchesFileOutput { changed: false, report: None, toast: None };
        match event {
            PresetFileEvent::Loaded { file_name, bytes, replace, .. } => match crate::palette::read_swatches(bytes, file_name) {
                Ok((groups, report)) => {
                    library.import(groups, *replace);
                    output.changed = true;
                    output.toast = Self::notes_toast(&report, false, words, format);
                    output.report = Some(report);
                }
                Err(error) => {
                    output.toast = Some(Toast::new(words.import_failed.fill(&[("reason", &error.to_string())])).tone(Tone::Bad));
                }
            },
            PresetFileEvent::Saved { .. } => {
                if let Some(report) = state.export_notes.take() {
                    output.toast = Self::notes_toast(&report, true, words, format);
                    output.report = Some(report);
                }
            }
            PresetFileEvent::Failed { .. } => {
                state.export_notes = None;
                output.toast = Some(Toast::new(event.status(presets)).tone(Tone::Bad));
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
            })
            .collect()
    }

    /// The toast for the notes of an import (or, with `writing`, an export): the title and the notes in one line, as
    /// a warning; `None` when there is nothing to say.
    fn notes_toast(report: &SwatchReport, writing: bool, words: &SwatchesWords, format: &Format) -> Option<Toast> {
        let lines = Self::note_lines(report, writing, words, format);
        if lines.is_empty() {
            return None;
        }
        let title = if writing { &words.notice_export } else { &words.notice_import };
        Some(Toast::new(format!("{}: {}", title.as_str(), lines.join(" "))).tone(Tone::Warn))
    }
}
