//! The command table: command and shortcut columns over the lines `rows` builds, group headers that open and
//! close, and rows whose shortcut chips, add button and restore button are children of the row.

use std::borrow::Cow;

use craft_behavior::{ColumnKey, ColumnWidth, DisplayOrder, Flow, Insets, ItemSource, Metrics, Proposal, Selection, SelectionChange, SpanLayout};
use craft_keymap::chord::Chord;
use craft_keymap::keymap::{Binding, MAX_BINDINGS};
use craft_ui::buttons::{Button, IconButton};
use craft_ui::contract::{
    Bind, BindMut, Collection, CollectionEvent, CollectionProps, CollectionResponse, ColumnSpec, Common, Component, ContainerProps, Cx, ItemOutput,
    MeasureCx, Outcome, PaintUi, Props as _, Role, SelectMode, ShowCollection as _, Sizing, Styled as _, clicked,
};
use craft_ui::display::{Elide, Icon, Shade, Text};
use craft_ui::primitives::{IconSpec, Span, TextRun};
use craft_ui::style::{ComponentStyle, Ink, Stateful, StyleCx, TextLook, TypeRole};
use egui::{Pos2, Rect, Response, Ui, Vec2};

use crate::shortcuts::recorder::{ChordRecorder, RecorderOutput};
use crate::shortcuts::rows::{AreaId, Chip, FixedLine, Line, Row};
use crate::shortcuts::state::{DialogState, Env, Target};
use crate::shortcuts::view::{area_key, recording_text};

const COMMAND: ColumnKey = ColumnKey::new("command");
const SHORTCUT: ColumnKey = ColumnKey::new("shortcut");
/// The command column is this many times as wide as the shortcut column.
const COMMAND_SHARE: f32 = 2.0;
const SHORTCUT_SHARE: f32 = 1.0;
const PATH_SEPARATOR: &str = " \u{203a} ";

/// What a row reports besides what the collection reads from its pointer input.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RowAct {
    /// A chip (`Some`), or the add button (`None`), asked to record.
    Record(Option<usize>),
    /// The restore button was pressed.
    Reset,
    /// The recording cell took these chords this pass.
    Chords(Vec<Chord>),
    /// The recording cell lost focus.
    Stopped,
    /// The recording cell holds key focus.
    Started,
}

/// The lines as the table's source, keyed by position.
struct Lines<'a>(&'a [Line]);

impl DisplayOrder<usize> for Lines<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }

    fn has_children(&self, at: usize) -> bool {
        matches!(self.0[at], Line::Header { .. })
    }

    fn open(&self, at: usize) -> bool {
        matches!(&self.0[at], Line::Header { collapsed: false, .. })
    }

    fn is_section(&self, at: usize) -> bool {
        matches!(self.0[at], Line::Header { .. })
    }

    fn section_stop(&self, at: usize) -> bool {
        self.is_section(at)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(match &self.0[at] {
            Line::Header { name, .. } => name.as_str(),
            Line::Row(row) => row.label.as_str(),
            Line::Fixed(fixed) => fixed.label.as_str(),
        }))
    }
}

impl ItemSource<usize> for Lines<'_> {}

/// The style of the table: no look of its own.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub(crate) struct ShortcutTableStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// A table of commands with their shortcuts.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(collection)]
pub(crate) struct ShortcutTable<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    #[collection]
    items: CollectionProps,
    /// The selected row, as its position among the lines.
    #[prop]
    selection: Option<Bind<'a, Selection<usize>>>,
    /// Scrolls this line into view once.
    #[prop]
    reveal: Option<usize>,
}

impl<'a> ShortcutTable<'a> {
    fn new(label: impl Into<craft_ui::runtime::env::Label>, columns: Vec<ColumnSpec>) -> Self {
        ShortcutTable {
            common: Common::new(),
            props: ContainerProps::new().sizing(Sizing::Fill),
            items: CollectionProps::new().select(SelectMode::Single).columns(columns),
            selection: None,
            reveal: None,
        }
        .label(label)
    }
}

impl Component for ShortcutTable<'_> {
    const ROLE: Role = Role::TABLE;
    type Style = ShortcutTableStyle;
    type Output = CollectionResponse<usize, RowAct>;

    fn style(_: &StyleCx<'_>, _: ShortcutTableStyleKey) -> ShortcutTableStyle {
        ShortcutTableStyle::default()
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> Self::Output {
        cx.collection_response()
    }
}

impl Collection for ShortcutTable<'_> {
    type Key = usize;
    type ItemLayout = SpanLayout;
    type Selection = Selection<usize>;
    type Item<'r> = CommandRow<'r>;
    type ItemEvent = RowAct;

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn selection(&mut self) -> Option<BindMut<'_, Selection<usize>>> {
        self.selection.as_mut().map(Bind::reborrow)
    }

    fn reveal(&self) -> Option<&usize> {
        self.reveal.as_ref()
    }

    /// A row is a control high.
    fn item_layout(&self, source: &dyn ItemSource<usize>, width: f32, cx: &StyleCx<'_>) -> SpanLayout {
        SpanLayout::sized(source, width, cx.control().height)
    }
}

/// The style of a row.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub(crate) struct CommandRowStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The command's name.
    #[style(text)]
    label: TextLook,
    /// Path parts, notes and unusable saved texts.
    #[style(text)]
    dim: TextLook,
    /// The row and control height.
    height: f32,
    pad: f32,
    gap: f32,
    /// The side of the conflict mark.
    icon: f32,
}

/// What a row returns.
#[non_exhaustive]
pub(crate) struct RowOut {
    /// The row's response.
    pub response: Response,
    /// What the row's children reported.
    pub act: Option<RowAct>,
}

impl Outcome for RowOut {
    type Value = craft_ui::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl ItemOutput<RowAct> for RowOut {
    fn into_event(self) -> Option<RowAct> {
        self.act
    }
}

/// What a row needs of the dialog this pass.
pub(crate) struct RowEnv<'r> {
    editable: bool,
    /// The cell the recorder replaces: the row's command, the chip it replaces (`None` adds one) and the text it
    /// shows.
    recording: Option<(&'r Target, String)>,
    /// The recording cell has not been shown yet: it takes key focus in the pass it first is.
    start: bool,
    words: &'r crate::shortcuts::words::ShortcutWords,
}

/// One row of the table: a command with its chips, or a read-only fixed row.
#[derive(craft_ui::contract::Component)]
pub(crate) struct CommandRow<'r> {
    #[common]
    common: Common,
    line: &'r Line,
    env: &'r RowEnv<'r>,
    cells: Vec<(ColumnKey, Rect)>,
    /// The width of every text the shortcut cell shows, measured in `measure` as the child that shows it (paint
    /// cannot lay out).
    widths: std::collections::BTreeMap<(Shown, String), f32>,
    act: Option<RowAct>,
}

/// What the shortcut cell shows a text as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Shown {
    Chip,
    Unparsed,
    Recording,
}

impl<'r> CommandRow<'r> {
    fn new(line: &'r Line, env: &'r RowEnv<'r>, cells: &[(ColumnKey, Rect)]) -> Self {
        let words = env.words;
        let name = match line {
            Line::Row(row) => {
                let mut name = row.label.clone();
                if row.cycles {
                    name = words.label_cycles.fill(&[("label", &name)]);
                }
                if row.changed {
                    name = words.label_changed.fill(&[("label", &name)]);
                }
                name
            }
            Line::Fixed(fixed) => match &fixed.now_used_by {
                Some(holder) => words.label_chord_now_used_by_holder.fill(&[("label", &fixed.label), ("chord", &fixed.keys), ("holder", holder)]),
                None => words.label_chord.fill(&[("label", &fixed.label), ("chord", &fixed.keys)]),
            },
            Line::Header { name, .. } => name.clone(),
        };
        let conflict = matches!(line, Line::Row(row) if row.conflict);
        let row = CommandRow { common: Common::new(), line, env, cells: cells.to_vec(), widths: std::collections::BTreeMap::new(), act: None };
        if conflict { row.description(words.shares_a_shortcut.as_str().to_owned()).label(name) } else { row.label(name) }
    }

    fn cell(&self, column: ColumnKey) -> Rect {
        self.cells.iter().find(|(key, _)| *key == column).map_or(Rect::NOTHING, |(_, rect)| *rect)
    }
}

/// A saved text that is not a shortcut, shown in the shortcut cell under `name`.
fn unparsed_note(text: &str, name: String) -> Text {
    Text::new(text.to_owned()).shade(Shade::Faint).elide(Elide::End).label(name)
}

/// The part of `row` that starts `at` from its leading edge and runs `length`, over the row's full height.
fn band(flow: &Flow, row: Rect, at: f32, length: f32) -> Rect {
    let span = flow.span(at, length);
    Rect::from_min_max(Pos2::new(span.left(), row.top()), Pos2::new(span.right(), row.bottom()))
}

impl Component for CommandRow<'_> {
    const ROLE: Role = Role::ROW;
    type Style = CommandRowStyle;
    type Output = RowOut;

    fn style(cx: &StyleCx<'_>, _: CommandRowStyleKey) -> CommandRowStyle {
        CommandRowStyle {
            metrics: Metrics::new(),
            label: TextLook::new(TypeRole::Label, Stateful::new(cx.ink(Ink::OnPanel))),
            dim: TextLook::new(TypeRole::Label, Stateful::new(cx.color(cx.tokens.color.text.muted))),
            height: cx.control().height,
            pad: cx.control().pad_x,
            gap: cx.tokens.space.gap.inline,
            icon: cx.control().icon,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        self.widths.clear();
        if let Line::Row(line) = self.line {
            for chip in &line.chips {
                let width = cx.child(("chip", &chip.text), &mut Button::new(chip.text.clone()), Proposal::Ideal).x;
                self.widths.insert((Shown::Chip, chip.text.clone()), width);
            }
            for text in &line.unparsed {
                let width = cx.child(("unparsed", text), &mut unparsed_note(text, String::new()), Proposal::Ideal).x;
                self.widths.insert((Shown::Unparsed, text.clone()), width);
            }
            if let Some((Target::Cell { command, .. }, shown)) = &self.env.recording
                && *command == line.command
            {
                let width = ChordRecorder::new(String::new(), true, shown.clone()).width(cx, ("record", &line.key));
                self.widths.insert((Shown::Recording, shown.clone()), width);
            }
        }
        Vec2::new(0.0, cx.style.height)
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let rtl = cx.env.direction().is_rtl();
        let command = Flow::new(self.cell(COMMAND), rtl).inset(Insets::horizontal(cx.style.pad));
        match self.line {
            Line::Row(line) => {
                self.paint_label(ui, cx, command, line);
                self.paint_shortcuts(ui, cx, rtl, line);
            }
            Line::Fixed(fixed) => self.paint_fixed(ui, cx, rtl, command, fixed),
            Line::Header { .. } => {}
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> RowOut {
        RowOut { response: cx.response.clone(), act: self.act }
    }
}

impl CommandRow<'_> {
    /// The command's name after its dimmed menu path, with the clash mark before and the note of a shared tool key
    /// after.
    fn paint_label(&self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>, area: Rect, line: &Row) {
        let words = self.env.words;
        let mut area = area;
        if line.conflict {
            let flow = Flow::new(area, cx.env.direction().is_rtl());
            let side = cx.style.icon;
            let slot = flow.span(0.0, side);
            ui.child(
                "conflict",
                Rect::from_center_size(slot.center(), Vec2::splat(side)),
                Icon::new(IconSpec::svg(craft_behavior::IconId::new("warning"))),
            );
            area = band(&flow, area, side + cx.style.gap, (area.width() - side - cx.style.gap).max(0.0));
        }
        let mut spans = Vec::new();
        for part in &line.prefix {
            spans.push(Span::new(part.as_str()).muted());
            spans.push(Span::new(PATH_SEPARATOR).muted());
        }
        spans.push(Span::new(line.label.as_str()));
        if line.cycles {
            spans.push(Span::new(" ").muted());
            spans.push(Span::new(words.cycles.as_str()).muted());
        }
        let run = TextRun::spans(spans).role(cx.style.label.role).color(cx.resolve(&cx.style.label.color)).elide();
        ui.draw().text(area, &run);
    }

    fn paint_fixed(&self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>, rtl: bool, area: Rect, fixed: &FixedLine) {
        let taken = fixed.now_used_by.is_some();
        let color = if taken { &cx.style.dim.color } else { &cx.style.label.color };
        let run = TextRun::new(fixed.label.as_str()).role(cx.style.label.role).color(cx.resolve(color)).elide();
        ui.draw().text(area, &run);
        let keys = Flow::new(self.cell(SHORTCUT), rtl).inset(Insets::horizontal(cx.style.pad));
        let run = TextRun::new(fixed.keys.as_str()).role(cx.style.label.role).color(cx.resolve(color)).elide();
        ui.draw().text(keys, &run);
        if let Some(holder) = &fixed.now_used_by {
            let tip = self.env.words.now_used_by_label.fill(&[("label", holder)]);
            ui.child("note", self.cell(SHORTCUT), Text::new(craft_ui::runtime::env::Label::Empty).tooltip(tip));
        }
    }

    /// The chips in a row, the recording cell in place of the chip it replaces, the add button after them and the
    /// restore button at the end of the cell.
    fn paint_shortcuts(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>, rtl: bool, line: &Row) {
        let words = self.env.words;
        let cell = self.cell(SHORTCUT);
        let flow = Flow::new(cell, rtl);
        let (gap, height) = (cx.style.gap, cx.style.height);
        let editable = self.env.editable;
        let widths = &self.widths;
        let width_of = |kind: Shown, text: &str| widths.get(&(kind, text.to_owned())).copied().unwrap_or(0.0);
        let recording = self.env.recording.as_ref().and_then(|(target, shown)| match target {
            Target::Cell { command, old } if *command == line.command => Some((old.clone(), shown.clone())),
            _ => None,
        });
        let reset_at = (cell.width() - height).max(0.0);
        let mut at = 0.0;
        let mut act = None;
        let mut cells: Vec<(Option<&Chip>, bool)> =
            line.chips.iter().map(|chip| (Some(chip), recording.as_ref().is_some_and(|(old, _)| old.as_ref() == Some(&chip.binding)))).collect();
        if matches!(&recording, Some((None, _))) {
            cells.push((None, true));
        }
        for (n, (chip, recording_here)) in cells.iter().enumerate() {
            let shown = match (&recording, recording_here) {
                (Some((_, shown)), true) => shown.clone(),
                _ => chip.map_or_else(String::new, |chip| chip.text.clone()),
            };
            let kind = if *recording_here { Shown::Recording } else { Shown::Chip };
            let width = width_of(kind, &shown).min((reset_at - at).max(0.0));
            let rect = band(&flow, cell, at, width);
            at += width + gap;
            if *recording_here {
                let label = words.label_press_keys.fill(&[("label", &line.label)]);
                let start = self.env.start;
                let out: RecorderOutput = ChordRecorder::new(label, true, shown).child(ui, ("record", line.key.as_str()), rect);
                if !out.chords.is_empty() {
                    act = Some(RowAct::Chords(out.chords));
                } else if out.response.lost_focus() {
                    act = Some(RowAct::Stopped);
                } else if start {
                    act = Some(RowAct::Started);
                }
                continue;
            }
            let Some(chip) = chip else { continue };
            let name = words.label_chord.fill(&[("label", &line.label), ("chord", &chip.text)]);
            let button = Button::new(chip.text.clone()).id_salt(("chip", line.key.as_str(), n)).label(name).enabled(editable);
            if clicked(&ui.child(("chip", line.key.as_str(), n), rect, button)) {
                act = Some(RowAct::Record(Some(n)));
            }
        }
        if cells.is_empty() {
            let rect = band(&flow, cell, 0.0, reset_at.max(height));
            let none = words.none.as_str();
            let name = words.label_chord.fill(&[("label", &line.label), ("chord", none)]);
            let button = Button::new(none.to_owned()).id_salt(("chip", line.key.as_str(), 0)).label(name).enabled(editable);
            if clicked(&ui.child(("chip", line.key.as_str(), 0), rect, button)) {
                act = Some(RowAct::Record(None));
            }
            at = reset_at;
        }
        for (n, text) in line.unparsed.iter().enumerate() {
            let width = width_of(Shown::Unparsed, text).min((reset_at - at).max(0.0));
            let rect = band(&flow, cell, at, width);
            at += width + gap;
            let name = words.label_chord.fill(&[("label", &line.label), ("chord", text)]);
            let note = unparsed_note(text, name).tooltip(words.this_saved_text_is_not_a.as_str().to_owned());
            ui.child(("unparsed", line.key.as_str(), n), rect, note);
        }
        let adding = matches!(&recording, Some((None, _)));
        if editable && line.chips.len() < MAX_BINDINGS && !adding && at + height <= reset_at {
            let rect = band(&flow, cell, at, height);
            let name = words.add_a_shortcut_to_label.fill(&[("label", &line.label)]);
            let add = IconButton::new(IconSpec::svg(craft_behavior::IconId::new("plus")))
                .id_salt(("add", line.key.as_str()))
                .label(name.clone())
                .tooltip(name);
            if clicked(&ui.child(("add", line.key.as_str()), rect, add)) {
                act = Some(RowAct::Record(None));
            }
        }
        if editable {
            let rect = band(&flow, cell, reset_at, height);
            let name = words.restore_the_default_for_label.fill(&[("label", &line.label)]);
            let reset = IconButton::new(IconSpec::svg(craft_behavior::IconId::new("undo")))
                .id_salt(("reset", line.key.as_str()))
                .label(name.clone())
                .tooltip(name)
                .enabled(line.changed);
            if clicked(&ui.child(("reset", line.key.as_str()), rect, reset)) {
                act = Some(RowAct::Reset);
            }
        }
        if act.is_some() {
            self.act = act;
        }
    }
}

/// The table for one pass.
///
/// Rows come from `state.lines(env)`; a press on a chip or the add button starts a recording on that cell, Enter
/// records into the selected row, Delete removes its current chip, and a group header opens and closes.
pub(super) fn show(ui: &mut Ui, now: f64, env: &mut Env<'_>, state: &mut DialogState) {
    let words = env.text.words();
    let built = state.lines(env).clone();
    let position = state.selected.as_ref().and_then(|command| built.position(command));
    let editable = env.options.editable;
    let shown = recording_text(env, state);
    let start = state.recording.as_ref().is_some_and(|recording| !recording.started);
    let row_env = RowEnv { editable, recording: state.recording.as_ref().map(|recording| (&recording.target, shown.clone())), start, words };
    let columns = vec![
        ColumnSpec::new(COMMAND, words.command.clone(), ColumnWidth::fill(COMMAND_SHARE)),
        ColumnSpec::new(SHORTCUT, words.shortcut.clone(), ColumnWidth::fill(SHORTCUT_SHARE)),
    ];
    let mut selection = Selection::new(position.into_iter().collect(), position, position);
    let reveal = if state.reveal { position } else { None };
    let source = Lines(&built.lines);
    let stamp = stamp_of(&state.area, &state.query, &format!("{:?}", state.filter));
    let mut table = ShortcutTable::new(words.commands.clone(), columns)
        .id_salt("table")
        .selection(Bind::bound(&mut selection))
        .empty(words.no_matching_commands.clone())
        .rows_stamp(stamp)
        .deletable(editable);
    if let Some(at) = reveal {
        table = table.reveal(at);
    }
    let out = table.show_items(ui, &source, &mut |key, icx| CommandRow::new(&built.lines[*key], &row_env, icx.cells()));
    apply(env, state, now, &built.lines, out);
}

/// Applies what the table reported, in the order it happened.
fn apply(env: &mut Env<'_>, state: &mut DialogState, now: f64, lines: &[Line], out: CollectionResponse<usize, RowAct>) {
    for event in out.events {
        match event {
            CollectionEvent::Selection(SelectionChange::Set(keys)) => {
                if let Some(Line::Row(row)) = keys.last().and_then(|key| lines.get(*key)) {
                    state.select_row(&row.command);
                }
            }
            CollectionEvent::Activate(key) => match lines.get(key) {
                Some(Line::Header { name, .. }) => state.toggle_group(name),
                Some(Line::Row(row)) if env.options.editable => {
                    state.select_row(&row.command);
                    let chip = state.chip.min(row.chips.len().saturating_sub(1));
                    let old = row.chips.get(chip).map(|chip| chip.binding.clone());
                    state.record(env, Target::Cell { command: row.command.clone(), old });
                    state.chip = chip;
                }
                _ => {}
            },
            CollectionEvent::Open(change) => {
                if let Some(Line::Header { name, collapsed }) = lines.get(change.key)
                    && change.open == *collapsed
                {
                    state.toggle_group(name);
                }
            }
            CollectionEvent::Delete(keys) if env.options.editable => {
                for key in keys {
                    let Some(Line::Row(row)) = lines.get(key) else { continue };
                    let chip = state.chip.min(row.chips.len().saturating_sub(1));
                    if let Some(binding) = row.chips.get(chip).map(|chip| chip.binding.clone()) {
                        state.remove(env, &row.command, &binding);
                        state.chip = chip.saturating_sub(1);
                    }
                }
            }
            CollectionEvent::Item(key, act) => {
                let Some(Line::Row(row)) = lines.get(key) else { continue };
                match act {
                    RowAct::Record(chip) => {
                        let old: Option<Binding> = chip.and_then(|n| row.chips.get(n)).map(|chip| chip.binding.clone());
                        state.select_row(&row.command);
                        state.record(env, Target::Cell { command: row.command.clone(), old });
                        state.chip = chip.unwrap_or(row.chips.len());
                    }
                    RowAct::Reset => {
                        state.select_row(&row.command);
                        state.use_default(env, &row.command);
                    }
                    RowAct::Chords(chords) => {
                        state.feed(env, &chords, now);
                    }
                    RowAct::Stopped => state.stop_recording(),
                    RowAct::Started => {
                        if let Some(recording) = state.recording.as_mut() {
                            recording.started = true;
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Identifies what the rows are, so the lead starts over when the area, the filter or the query changes.
fn stamp_of(area: &AreaId, query: &str, filter: &str) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    area_key(area).hash(&mut hasher);
    query.hash(&mut hasher);
    filter.hash(&mut hasher);
    hasher.finish()
}
