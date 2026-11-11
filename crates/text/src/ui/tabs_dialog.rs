//! [`TabStopsDialog`]: Word's and PowerPoint's Tabs dialog on the dialog stack: the default tab stops interval, the
//! list of stops (position, alignment, leader), the selected stop's fields, Set, Clear and Clear All; OK applies them
//! together.
//!
//! ```ignore
//! dialogs.open(TabStopsDialog::new(paragraph.tabs.clone(), doc.default_tab).unit(unit));
//! if let Some(DialogResult::Ok(tabs)) = dialogs.take::<TabStopsDialog>() {
//!     doc.set_tabs(tabs.stops, tabs.cleared, tabs.default_interval);
//! }
//! ```

use super::length::{DocUnit, length_field, shown};
use super::ruler::{MAX_POSITION, SAME_POSITION, TabAlign, TabLeader, TabStop};
use craft_behavior::{ColumnKey, ColumnWidth, DisplayOrder, EditEvent, ItemSource, Mixed, Selection, SelectionChange};
use craft_ui::buttons::Button;
use craft_ui::contract::{
    Bind, CollectionEvent, ColumnSpec, Entry, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, clicked,
};
use craft_ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft_ui::form::Field;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::ListRow;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Units};
use craft_ui::runtime::words::CommonWords;
use craft_ui::sections::{DialogRole, FooterButton};
use craft_ui::table::Table;
use egui::Ui;
use std::borrow::Cow;

#[cfg(feature = "screen-testing")]
mod stories;

const POSITION: ColumnKey = ColumnKey::new("position");
const ALIGNMENT: ColumnKey = ColumnKey::new("alignment");
const LEADER: ColumnKey = ColumnKey::new("leader");

craft_ui::words! {
    /// The dialog's words (Word's; PowerPoint's dialog uses the same).
    pub struct TabStopsWords {
        title: "tabs.title" = "Tabs";
        default_stops: "tabs.defaultStops" = "Default tab stops:";
        position: "tabs.position" = "Tab stop position:";
        /// The Position column's header (the field label above keeps its colon).
        position_column: "tabs.positionColumn" = "Position";
        alignment: "tabs.alignment" = "Alignment";
        leader: "tabs.leader" = "Leader";
        left: "tabs.left" = "Left";
        center: "tabs.center" = "Center";
        right: "tabs.right" = "Right";
        decimal: "tabs.decimal" = "Decimal";
        bar: "tabs.bar" = "Bar";
        leader_none: "tabs.leaderNone" = "None";
        /// A leader option: Word numbers the fills, "1 ……" (the number is the leader's access key in Word's list).
        leader_option: template "tabs.leaderOption" = "{n} {fill}";
        set: "tabs.set" = "Set";
        clear: "tabs.clear" = "Clear";
        clear_all: "tabs.clearAll" = "Clear All";
        to_be_cleared: "tabs.toBeCleared" = "Tab stops to be cleared:";
    }
}

/// What OK returns.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct TabStopsResult {
    /// The stops, sorted, none within [`SAME_POSITION`] of another.
    pub stops: Vec<TabStop>,
    /// Positions to clear (stops the paragraph inherits from its style, cleared here).
    pub cleared: Vec<f32>,
    /// The default interval, points.
    pub default_interval: f32,
}

/// The most stops the dialog reads from a paragraph (Word's 64).
pub const MAX_STOPS: usize = 64;

/// The interval Set steps by when the document's is not a positive number: Word's 0.5 in.
const FALLBACK_INTERVAL: f32 = 36.0;

/// The Tabs dialog.
///
/// - The stops are a `Table` (columns Position, Alignment, Leader; one Tab stop with the list keys and single
///   selection). The selected stop's fields sit under it: a position field in the unit, an alignment `Select` and a
///   leader `Select` whose options read as Word's list does ("1 ……"). Editing the position keeps the rows in order
///   until the edit ends, then [`TabStopsDialog::normalize`] restores position order and merges stops moved onto one
///   another (the edited row wins).
/// - Set adds a stop at the next free multiple of the default interval after the last stop, left aligned, no leader,
///   selected; Clear moves the selected stop to "to be cleared"; Clear All clears every stop.
/// - Positions are clamped to ±[`MAX_POSITION`]; at most [`MAX_STOPS`] stops are read from a paragraph.
/// - Keys: Enter is OK unless a field owns it, Escape is Cancel.
#[derive(craft_ui::contract::Pattern)]
pub struct TabStopsDialog {
    stops: Vec<TabStop>,
    cleared: Vec<f32>,
    default_interval: f32,
    selected: Option<usize>,
    edited: Option<usize>,
    unit: DocUnit,
    /// The row and position an in-place position edit began from, for Escape.
    draft: Option<(usize, f32)>,
}

impl TabStopsDialog {
    /// The dialog over a paragraph's `stops` and the document's default interval (points).
    pub fn new(mut stops: Vec<TabStop>, default_interval: f32) -> Self {
        stops.retain(|stop| stop.position.is_finite());
        stops.truncate(MAX_STOPS);
        TabStopsDialog { stops, cleared: Vec::new(), default_interval, selected: None, edited: None, unit: DocUnit::default(), draft: None }
    }

    /// Sets the unit the position fields show.
    #[must_use]
    pub fn unit(mut self, unit: DocUnit) -> Self {
        self.unit = unit;
        self
    }

    /// Set: a stop at the next free multiple of the default interval after the last stop.
    pub fn new_stop(&mut self) {
        if self.stops.len() >= MAX_STOPS {
            return;
        }
        let step = if self.default_interval.is_finite() && self.default_interval > 0.0 { self.default_interval } else { FALLBACK_INTERVAL };
        let taken = |at: f32| {
            self.stops.iter().any(|stop| (stop.position - at).abs() < SAME_POSITION)
                || self.cleared.iter().any(|gone| (gone - at).abs() < SAME_POSITION)
        };
        let last = self.stops.iter().map(|stop| stop.position).fold(0.0, f32::max);
        let mut position = (last / step).floor() * step + step;
        while taken(position) && position <= MAX_POSITION {
            position += step;
        }
        if !position.is_finite() || position > MAX_POSITION {
            return;
        }
        self.stops.push(TabStop { position, align: TabAlign::Left, leader: TabLeader::None });
        self.stops.sort_by(|a, b| a.position.total_cmp(&b.position));
        self.selected = self.stops.iter().position(|stop| (stop.position - position).abs() < SAME_POSITION);
        self.edited = None;
    }

    /// Position order after in-place edits, merging stops within [`SAME_POSITION`] (the edited
    /// row wins), the selection kept on its stop.
    pub fn normalize(&mut self) {
        let Some(edited) = self.edited.take() else { return };
        let followed = self.selected.and_then(|at| self.stops.get(at)).copied();
        let mut kept: Vec<(usize, TabStop)> = Vec::with_capacity(self.stops.len());
        for (at, stop) in self.stops.iter().enumerate() {
            match kept.iter().position(|(_, other)| (other.position - stop.position).abs() < SAME_POSITION) {
                // The edited row holds its ground; the row that drifted onto it goes.
                Some(held) if kept.get(held).is_some_and(|(was, _)| *was == edited) => {}
                Some(held) => {
                    kept.remove(held);
                    kept.push((at, *stop));
                }
                None => kept.push((at, *stop)),
            }
        }
        kept.sort_by(|a, b| a.1.position.total_cmp(&b.1.position));
        self.stops = kept.into_iter().map(|(_, stop)| stop).collect();
        self.selected = followed.and_then(|moved| self.stops.iter().position(|stop| (stop.position - moved.position).abs() < SAME_POSITION));
    }

    /// An in-place edit of the stop at `at`: clamped, and a cleared position it lands on is no
    /// longer cleared. The rows keep their order until [`normalize`](Self::normalize).
    pub fn edit_position(&mut self, at: usize, position: f32) {
        if !position.is_finite() {
            return;
        }
        let Some(stop) = self.stops.get_mut(at) else { return };
        stop.position = position.clamp(-MAX_POSITION, MAX_POSITION);
        let landed = stop.position;
        self.cleared.retain(|gone| (gone - landed).abs() >= SAME_POSITION);
        self.edited = Some(at);
    }

    /// Selects the stop at index `at`, or none.
    pub fn select(&mut self, at: Option<usize>) {
        self.selected = at.filter(|at| *at < self.stops.len());
    }

    /// Clear: the selected stop moves to "to be cleared".
    pub fn clear(&mut self) {
        let Some(at) = self.selected.filter(|at| *at < self.stops.len()) else { return };
        let stop = self.stops.remove(at);
        self.cleared.push(stop.position);
        self.selected = None;
        self.edited = None;
    }

    /// Clear All.
    pub fn clear_all(&mut self) {
        self.cleared.extend(self.stops.drain(..).map(|stop| stop.position));
        self.selected = None;
        self.edited = None;
    }

    /// What OK returns: the stops normalized, the cleared positions and the default interval.
    pub fn result(&mut self) -> TabStopsResult {
        self.normalize();
        TabStopsResult { stops: self.stops.clone(), cleared: self.cleared.clone(), default_interval: self.default_interval }
    }
}

/// The stops as a collection source, keyed by position.
struct Stops<'a> {
    stops: &'a [TabStop],
    texts: &'a [String],
}

impl DisplayOrder<usize> for Stops<'_> {
    fn len(&self) -> usize {
        self.stops.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.stops.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.texts.get(at).map(|text| Cow::Borrowed(text.as_str()))
    }
}

impl ItemSource<usize> for Stops<'_> {}

impl StackDialog for TabStopsDialog {
    type Result = TabStopsResult;

    fn id(&self) -> DialogId {
        dialog_id("tab-stops")
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<TabStopsResult>> {
        let env = Env::of(ui);
        let words = env.words::<TabStopsWords>();
        let common = env.words::<CommonWords>();
        let units = env.units();
        let buttons = [FooterButton::new(common.ok.clone(), DialogRole::Accept), FooterButton::new(common.cancel.clone(), DialogRole::Cancel)];
        let shown = Dialog::new(words.title.clone()).buttons(buttons).show(ui, |ui, _| self.body(ui, &words, &units));
        shown.output.result(|| self.result())
    }
}

/// A change made in the selected stop's fields.
enum CellEdit {
    Position(EditEvent<Option<f64>>),
    Align(TabAlign),
    Leader(TabLeader),
}

impl TabStopsDialog {
    fn body(&mut self, ui: &mut Ui, words: &TabStopsWords, units: &Units) {
        Stack::new().show(ui, |ui, _| {
            self.default_row(ui, words);
            self.table(ui, words, units);
            self.fields(ui, words);
            self.buttons(ui, words);
            if !self.cleared.is_empty() {
                let positions: Vec<String> = self.cleared.iter().map(|gone| self.text(units, *gone)).collect();
                craft_ui::display::Text::new(format!("{} {}", words.to_be_cleared.as_str(), positions.join(", "))).show(ui);
            }
        });
    }

    /// `points` as the number in the unit with its suffix.
    fn text(&self, units: &Units, points: f32) -> String {
        units.show(self.unit.kind, shown(units, self.unit, f64::from(points)))
    }

    fn default_row(&mut self, ui: &mut Ui, words: &TabStopsWords) {
        let unit = self.unit;
        let points = Mixed::Same(Some(f64::from(self.default_interval)));
        let range = f64::from(SAME_POSITION)..=f64::from(MAX_POSITION);
        let edit = Field::new(words.default_stops.clone())
            .show(ui, |ui, _| length_field(unit, words.default_stops.clone(), points, range.clone()).show(ui))
            .inner;
        for event in edit.into_iter().flat_map(|out| out.edit.events) {
            if !matches!(event, EditEvent::Update { .. } | EditEvent::Commit { .. } | EditEvent::Set { .. }) {
                continue;
            }
            if let Some(Some(points)) = event.value() {
                self.default_interval = *points as f32;
            }
        }
    }

    /// The stops table; a selection change moves the selected stop.
    fn table(&mut self, ui: &mut Ui, words: &TabStopsWords, units: &Units) {
        let columns = vec![
            ColumnSpec::new(POSITION, words.position_column.clone(), ColumnWidth::fill(1.0)),
            ColumnSpec::new(ALIGNMENT, words.alignment.clone(), ColumnWidth::fill(1.0)),
            ColumnSpec::new(LEADER, words.leader.clone(), ColumnWidth::fill(1.0)),
        ];
        let texts: Vec<String> = self.stops.iter().map(|stop| self.text(units, stop.position)).collect();
        let selection = self.selected.map_or_else(Selection::default, Selection::single);
        let stops = self.stops.clone();
        let out = Table::<usize>::new(words.title.clone(), columns.clone())
            .select(SelectMode::Single)
            .natural_height(true)
            .selection(Bind::given(selection))
            .show_items(ui, &Stops { stops: &stops, texts: &texts }, &mut |key, icx| {
                let stop = &stops[*key];
                ListRow::new(texts[*key].clone())
                    .cells(icx.cells())
                    .columns(&columns)
                    .cell(ALIGNMENT, align_text(words, stop.align))
                    .cell(LEADER, leader_text(words, stop.leader))
            });
        for event in out.events {
            if let CollectionEvent::Selection(change) = event {
                match change {
                    SelectionChange::Set(keys) => self.select(keys.last().copied()),
                    SelectionChange::Clear => self.select(None),
                    _ => {}
                }
            }
        }
    }

    /// The selected stop's position, alignment and leader.
    fn fields(&mut self, ui: &mut Ui, words: &TabStopsWords) {
        let Some((at, stop)) = self.selected.and_then(|at| self.stops.get(at).map(|stop| (at, *stop))) else { return };
        let unit = self.unit;
        let aligns: Vec<Entry<TabAlign>> = TabAlign::ALL.into_iter().map(|align| Entry::new(align, align_text(words, align))).collect();
        let leaders: Vec<Entry<TabLeader>> = TabLeader::ALL.into_iter().map(|leader| Entry::new(leader, leader_text(words, leader))).collect();
        let mut edits = Vec::new();
        Row::new().show(ui, |ui, _| {
            let range = -f64::from(MAX_POSITION)..=f64::from(MAX_POSITION);
            let field = Field::new(words.position.clone())
                .show(ui, |ui, _| length_field(unit, words.position.clone(), Mixed::Same(Some(f64::from(stop.position))), range.clone()).show(ui))
                .inner;
            edits.extend(field.into_iter().flat_map(|field| field.edit.events).map(CellEdit::Position));
            let align = Select::new(words.alignment.clone(), Bind::given(stop.align), &aligns).show(ui);
            edits.extend(align.value().copied().map(CellEdit::Align));
            let leader = Select::new(words.leader.clone(), Bind::given(stop.leader), &leaders).show(ui);
            edits.extend(leader.value().copied().map(CellEdit::Leader));
        });
        for edit in edits {
            match edit {
                CellEdit::Position(event) => {
                    let event = event.map(|points| points.map(|p| p as f32));
                    self.apply_position(at, event);
                }
                CellEdit::Align(align) => self.stops[at].align = align,
                CellEdit::Leader(leader) => self.stops[at].leader = leader,
            }
        }
    }

    /// An in-place position edit: rows keep their order until the edit ends. Escape restores the position the edit
    /// began from.
    pub fn apply_position(&mut self, at: usize, event: EditEvent<Option<f32>>) {
        let Some(current) = self.stops.get(at).map(|stop| stop.position) else { return };
        match event {
            EditEvent::Begin { .. } => self.draft = Some((at, current)),
            EditEvent::Update { value: Some(position), .. } => {
                self.draft.get_or_insert((at, current));
                self.edit_position(at, position);
            }
            EditEvent::Commit { after: Some(position), .. } | EditEvent::Set { after: Some(position), .. } => {
                self.draft = None;
                self.edit_position(at, position);
                self.normalize();
            }
            EditEvent::Cancel { .. } => {
                if let Some((row, start)) = self.draft.take() {
                    self.edit_position(row, start);
                    self.normalize();
                }
            }
            _ => {}
        }
    }

    fn buttons(&mut self, ui: &mut Ui, words: &TabStopsWords) {
        Row::new().show(ui, |ui, _| {
            if clicked(&Button::new(words.set.clone()).enabled(self.stops.len() < MAX_STOPS).show(ui)) {
                self.new_stop();
            }
            if clicked(&Button::new(words.clear.clone()).enabled(self.selected.is_some()).show(ui)) {
                self.clear();
            }
            if clicked(&Button::new(words.clear_all.clone()).enabled(!self.stops.is_empty()).show(ui)) {
                self.clear_all();
            }
        });
    }
}

fn align_text(words: &TabStopsWords, align: TabAlign) -> String {
    match align {
        TabAlign::Left => words.left.as_str(),
        TabAlign::Center => words.center.as_str(),
        TabAlign::Right => words.right.as_str(),
        TabAlign::Decimal => words.decimal.as_str(),
        TabAlign::Bar => words.bar.as_str(),
    }
    .to_owned()
}

/// A leader option's text as Word's list reads: "None", or the fill after its number.
pub fn leader_text(words: &TabStopsWords, kind: TabLeader) -> String {
    match TabLeader::ALL.iter().position(|other| *other == kind) {
        Some(n) if kind != TabLeader::None => words.leader_option.fill(&[("n", &n.to_string()), ("fill", kind.sample())]),
        _ => words.leader_none.as_str().to_owned(),
    }
}
