//! `SwatchesPanel`: the panel body, its state and what it reports.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use craft_cms::convert::CmykConverter;
use craft_i18n::Format;
use craft_theme::Srgba;
use craft_theme::variant::Tone;
use craft_ui::bars::StatusMessage;
use craft_ui::contract::{
    Bind, DropAt, DropPlace, DropZone, Entry, Held, ListDrag, MemoProps, NoValue, Outcome, Pattern, SelectMode, ShowContainer as _,
};
use craft_ui::layout_components::Stack;
use craft_ui::patterns::presets::{
    PresetBrowser, PresetDrop, PresetEvent, PresetFileEvent, PresetFooterOptions, PresetReorder, PresetRowMenu, PresetWords, ThumbFn,
};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Count, Env};
use egui::{AsIdSalt, IdSalt, Response, Ui};

use super::library::{HostThumb, find_swatch, has, panel_source};
use super::menu::event_edits;
use super::{SWATCH_ZOOM, SwatchAt, SwatchEdit, SwatchKey, SwatchesFileOutput, SwatchesState, SwatchesWords};
use crate::palette::{SwatchGroup, SwatchNote, SwatchReport};

/// What [`SwatchesPanel::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the swatch and runs the edits"]
pub struct SwatchesPanelOutput {
    /// The panel's response.
    pub response: Response,
    /// A swatch was clicked: where it is, its colour (through the document's converter) when it holds one, and whether
    /// it is for the background (Alt held; Ctrl or Cmd too, as PhotoCraft) rather than the foreground. A gradient,
    /// pattern or None swatch reports no colour: the host applies the paint it keeps for the entry.
    pub used: Option<(SwatchAt, Option<Srgba>, bool)>,
    /// The edits the user made this pass, in pass order, for the host to run; the panel never writes the library.
    pub edits: Vec<SwatchEdit>,
    /// The browser events the panel does not turn into `used` or `edits`, in the order they happened: `Activate`,
    /// `Menu`, `DropAt`, `Dropped` and `Reorder`. `Reorder` is also in `edits`; a host runs one of the two.
    pub events: Vec<PresetEvent<SwatchKey>>,
}

impl Outcome for SwatchesPanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

type ToolsFn<'a> = Box<dyn FnMut(&mut Ui) + 'a>;
type RowIconsFn<'a> = Box<dyn Fn(&SwatchKey) -> Vec<(IconSpec<'static>, String)> + 'a>;
type RowItemsFn<'a> = Box<dyn Fn(&SwatchKey) -> Vec<Entry<String>> + 'a>;
type DropMaker<'a> = Box<dyn Fn() -> PresetDrop<'a, SwatchKey> + 'a>;
type PayloadAdder<'a> = Box<dyn for<'b> Fn(PresetBrowser<'b, SwatchKey>) -> PresetBrowser<'b, SwatchKey> + 'a>;

/// The Swatches panel on the library's groups, built on `PresetBrowser`: groups as folders, swatches as flat touching
/// squares with no gap or corner radius (Photoshop), the size the browser's zoom chooses.
///
/// - Click uses the swatch for the foreground; Alt-click (or Ctrl or Cmd) for the background. The swatch is selected,
///   in `SwatchesState::selection`; [`select`](SwatchesPanel::select) chooses one swatch or several.
///   Double-click or F2 renames in place; Delete removes; dragging onto a group moves it. Swatches do not drop onto
///   the canvas.
/// - The footer's + is New Swatch: the foreground colour into the selected swatch's group, named "Color Swatch N", then
///   renamed in place. Its Delete removes every selected swatch and group.
/// - A dragged swatch carries the host's payloads ([`drag_payload`](SwatchesPanel::drag_payload)) to the host's drop
///   targets (a gradient's stops, the canvas), beside the move within the panel.
/// - Every change of the library comes back as a [`SwatchEdit`] with resolved names and positions; the host runs it.
/// - Tooltips name the swatch and its value; AccessKit as the preset browser's grid, each swatch named.
/// - The files of Import, Replace and Export go out as `PresetFileRequest`s from the panel menu; the host answers
///   them with [`SwatchesPanel::apply_file`].
/// - The browser's options [`thumb`](SwatchesPanel::thumb), [`tools`](SwatchesPanel::tools),
///   [`drop_zone`](SwatchesPanel::drop_zone), [`footer`](SwatchesPanel::footer), [`reveal`](SwatchesPanel::reveal),
///   [`search`](SwatchesPanel::search), [`row_icons`](SwatchesPanel::row_icons), [`takes_drop`](SwatchesPanel::takes_drop)
///   and [`row_items`](SwatchesPanel::row_items) pass through; the events they cause come out in
///   [`SwatchesPanelOutput::events`].
/// - [`document`](SwatchesPanel::document) keeps the browser's own state (an open rename field, the scroll position)
///   per document, so a host that shows one panel for several documents does not carry it from one to another.
/// - It fills the room it is given, and its browser scrolls the swatches: show it outside a body that scrolls.
#[must_use = "show the panel with `show`"]
#[derive(Pattern)]
pub struct SwatchesPanel<'a> {
    groups: &'a [SwatchGroup],
    state: Bind<'a, SwatchesState>,
    cmyk: Option<&'a CmykConverter>,
    foreground: Srgba,
    thumb: Option<HostThumb<'a>>,
    tools: Option<ToolsFn<'a>>,
    drop_zone: Option<DropZone>,
    footer: Option<Option<PresetFooterOptions<'a>>>,
    reveal: Option<SwatchKey>,
    search: bool,
    row_icons: Option<RowIconsFn<'a>>,
    select: SelectMode,
    takes_drop: Option<DropMaker<'a>>,
    payloads: Vec<PayloadAdder<'a>>,
    row_items: Option<RowItemsFn<'a>>,
    document: Option<IdSalt>,
}

/// What the body decided; `show` stores the state and reports the rest after the real run.
struct Settled {
    before_state: SwatchesState,
    st: SwatchesState,
    used: Option<(SwatchAt, Option<Srgba>, bool)>,
    edits: Vec<SwatchEdit>,
    events: Vec<PresetEvent<SwatchKey>>,
}

impl<'a> SwatchesPanel<'a> {
    /// The panel over the library's `groups` and the host-owned `state`; `foreground` is what New Swatch adds.
    pub fn new(groups: &'a [SwatchGroup], state: Bind<'a, SwatchesState>, foreground: Srgba) -> Self {
        SwatchesPanel {
            groups,
            state,
            cmyk: None,
            foreground,
            thumb: None,
            tools: None,
            drop_zone: None,
            footer: None,
            reveal: None,
            search: true,
            row_icons: None,
            select: SelectMode::Single,
            takes_drop: None,
            payloads: Vec::new(),
            row_items: None,
            document: None,
        }
    }

    /// The document's CMYK converter, which draws CMYK swatches and gives their colours when used; without it they
    /// use the built-in coated CMYK.
    pub fn cmyk(mut self, converter: &'a CmykConverter) -> Self {
        self.cmyk = Some(converter);
        self
    }

    /// Paints each swatch's thumbnail with `paint` over the host's `inputs` in place of the panel's own swatch face
    /// (the host's gradient and pattern previews), as `PresetFns::thumb` does: the inputs are all the painter reads, so
    /// the swatch list keys on them. The painter draws the selected state from `ThumbInfo::selected`.
    pub fn thumb<I: MemoProps + Send + 'a>(mut self, inputs: I, paint: ThumbFn<I, SwatchKey>) -> Self {
        self.thumb = Some(HostThumb::new(inputs, paint));
        self
    }

    /// The host's tools, shown beside the browser's toolbar.
    pub fn tools(mut self, tools: impl FnMut(&mut Ui) + 'a) -> Self {
        self.tools = Some(Box::new(tools));
        self
    }

    /// Where a drop onto the panel lands.
    pub fn drop_zone(mut self, zone: DropZone) -> Self {
        self.drop_zone = Some(zone);
        self
    }

    /// The footer's options; `None` leaves the footer out. Without it the footer keeps Delete enabled while a swatch
    /// is selected.
    pub fn footer(mut self, footer: Option<PresetFooterOptions<'a>>) -> Self {
        self.footer = Some(footer);
        self
    }

    /// The swatch to scroll into view.
    pub fn reveal(mut self, key: Option<SwatchKey>) -> Self {
        self.reveal = key;
        self
    }

    /// Whether the browser shows its search field.
    pub fn search(mut self, on: bool) -> Self {
        self.search = on;
        self
    }

    /// The icons trailing each swatch row, through `PresetRow::trailing`.
    pub fn row_icons(mut self, icons: impl Fn(&SwatchKey) -> Vec<(IconSpec<'static>, String)> + 'a) -> Self {
        self.row_icons = Some(Box::new(icons));
        self
    }

    /// How picks select: one swatch, or several with Shift and Cmd.
    pub fn select(mut self, mode: SelectMode) -> Self {
        self.select = mode;
        self
    }

    /// A host payload of type `P` can be dropped onto a swatch or between swatches at `place` where `accepts` agrees;
    /// the drop comes back as `PresetEvent::Dropped`.
    pub fn takes_drop<P: Send + Sync + 'static>(mut self, place: DropPlace, accepts: impl Fn(&P, &DropAt<SwatchKey>) -> bool + 'a) -> Self {
        // The body can run more than once a pass, and a `PresetDrop` is used up by the browser: it is built per run.
        let accepts = Rc::new(accepts);
        self.takes_drop = Some(Box::new(move || {
            let accepts = Rc::clone(&accepts);
            PresetDrop::new::<P>(place, move |payload: &P, at: &DropAt<SwatchKey>| accepts(payload, at))
        }));
        self
    }

    /// A swatch drag carries `map` of the dragged swatches too, for the host's drop targets of type `P` (the app's
    /// paint payload), as `PresetBrowser::drag_payload` does.
    pub fn drag_payload<P: Send + Sync + 'static>(mut self, map: impl Fn(&ListDrag<SwatchKey>) -> Option<P> + Send + Sync + 'static) -> Self {
        let map = Arc::new(map);
        self.payloads.push(Box::new(move |browser| {
            let map = Arc::clone(&map);
            browser.drag_payload(move |drag: &ListDrag<SwatchKey>| map(drag))
        }));
        self
    }

    /// The host's rows of the row menu for a swatch; the standard rows give way to them.
    pub fn row_items(mut self, items: impl Fn(&SwatchKey) -> Vec<Entry<String>> + 'a) -> Self {
        self.row_items = Some(Box::new(items));
        self
    }

    /// The document whose swatches the panel shows. The browser keeps its own state (an open rename field, the scroll
    /// position) for each document, so a rename left open in one document does not open on another document's
    /// swatch of the same name. Without it the browser keeps that state for the `Ui` it is shown in.
    pub fn document(mut self, document: impl AsIdSalt) -> Self {
        self.document = Some(IdSalt::new(document));
        self
    }

    /// Draws the panel and returns the user's actions this pass.
    pub fn show(self, ui: &mut Ui) -> SwatchesPanelOutput {
        let SwatchesPanel {
            groups,
            state,
            cmyk,
            foreground,
            thumb,
            mut tools,
            drop_zone,
            footer,
            reveal,
            search,
            row_icons,
            select,
            takes_drop,
            payloads,
            row_items,
            document,
        } = self;
        let cmyk = cmyk.unwrap_or_else(|| CmykConverter::coated());
        let words = Env::of(ui).words::<SwatchesWords>();
        let mut held_state = Held::new(state);
        let ctx = ui.ctx().clone();
        // A document's browser has an id of its own; otherwise the browser takes the scope id of the `Ui` it is shown in.
        let document_id = document.map(|document| ui.scope_id().with(document));
        let browser_id = document_id.unwrap_or_else(|| ui.scope_id());
        let icons: BTreeMap<&str, Vec<(IconSpec<'static>, String)>> = match &row_icons {
            Some(icons) => {
                groups.iter().flat_map(|g| g.entries.iter()).map(|e| (e.name.as_str(), icons(&SwatchKey::Swatch(e.name.clone())))).collect()
            }
            None => BTreeMap::new(),
        };
        // Built once: the body can run more than once a pass, and the host's painter is moved into the source.
        let source = panel_source(groups, cmyk, icons, thumb);
        let mut shown = Stack::new().fill().show(ui, |ui, cx| {
            let before_state = held_state.read(cx)?;
            let mut st = before_state.clone();
            let live = |key: &SwatchKey| has(groups, key);
            st.selection.items.retain(live);
            st.selection.lead = st.selection.lead.take().filter(live);
            st.selection.anchor = st.selection.anchor.take().filter(live);
            let footer_options = match &footer {
                Some(host) => host.clone(),
                None => Some(PresetFooterOptions::new().can_delete(!st.selection.items.is_empty())),
            };
            let out = {
                let mut browser = PresetBrowser::new(words.title.clone(), Bind::bound(&mut st.browser), &source)
                    .thumb_range(SWATCH_ZOOM)
                    .toolbar(true)
                    .search(search)
                    .select(select)
                    .reorder(PresetReorder::items_and_groups())
                    .selection(Bind::bound(&mut st.selection));
                if let Some(id) = document_id {
                    browser = browser.browser_id(id);
                }
                if let Some(options) = footer_options {
                    browser = browser.footer(options);
                }
                if let Some(zone) = drop_zone {
                    browser = browser.drop_zone(zone);
                }
                if let Some(key) = &reveal {
                    browser = browser.reveal(key.clone());
                }
                if let Some(tools) = tools.as_mut() {
                    browser = browser.tools(|ui: &mut Ui| tools(ui));
                }
                if let Some(items) = &row_items {
                    browser = browser.row_menu(PresetRowMenu::Host).row_items(|key: &SwatchKey| items(key));
                }
                if let Some(make) = &takes_drop {
                    browser = browser.takes_drop(make());
                }
                for add in &payloads {
                    browser = add(browser);
                }
                browser.show(ui)
            };
            let mut used = None;
            let mut edits = Vec::new();
            let mut events = Vec::new();
            for event in out.events {
                if let PresetEvent::Select { key: SwatchKey::Swatch(name), mods, .. } = &event
                    && let Some(at) = find_swatch(groups, name)
                    && let Some(entry) = groups.get(at.group).and_then(|g| g.entries.get(at.index))
                {
                    used = Some((at, entry.color().map(|color| color.to_srgb(cmyk)), mods.alt || mods.cmd || mods.ctrl));
                }
                let forwarded = matches!(
                    event,
                    PresetEvent::Activate { .. }
                        | PresetEvent::Menu { .. }
                        | PresetEvent::DropAt { .. }
                        | PresetEvent::Dropped { .. }
                        | PresetEvent::Reorder { .. }
                )
                .then(|| event.clone());
                edits.extend(event_edits(groups, &mut st, event, foreground, &words));
                events.extend(forwarded);
            }
            Some(Settled { before_state, st, used, edits, events })
        });
        // Once per pass, after the real run: the rename request and the edited state are stored, the edits reported.
        let (used, edits, events) = shown
            .settle(|cx, settled| {
                let Settled { before_state, mut st, used, edits, events } = settled.take()?;
                if let Some(key) = st.rename.take() {
                    PresetBrowser::request_rename(&ctx, browser_id, &key);
                }
                if st != before_state {
                    held_state.write(cx, &before_state, st, words.title.as_str());
                }
                Some((used, edits, events))
            })
            .flatten()
            .unwrap_or_default();
        SwatchesPanelOutput { response: shown.output, used, edits, events }
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
                SwatchNote::PaintsSkipped(n) => count(&words.paints_skipped, n),
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
