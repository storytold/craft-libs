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
use craft_theme::variant::{Tone, Variant};
use craft_ui::buttons::Button;
use craft_ui::contract::{
    Bind, CollectionEvent, ColumnSpec, Entry, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, clicked,
};
use craft_ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, HeldAnswer, HeldOk, StackDialog, dialog_id};
use craft_ui::form::Field;
use craft_ui::layout_components::Row;
use craft_ui::list::ListRow;
use craft_ui::notices::{Notice, NoticeLayout};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label, Units};
use craft_ui::runtime::focus::{self, FocusOrigin, FocusTarget};
use craft_ui::runtime::words::CommonWords;
use craft_ui::sections::{DialogRole, FooterButton};
use craft_ui::table::Table;
use egui::Ui;
use std::borrow::Cow;
use std::cell::{Cell, RefCell};

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The OK footer button's index, in the order the buttons are given.
const OK: usize = 0;
/// Cancel's place among the bare dialog's buttons ([`TabStopsDialog::bare`]).
const CANCEL: usize = 1;
/// The bare dialog's width, and the room above its Cancel and OK.
const BARE_WIDTH: f32 = 420.0;
const BARE_GAP: f32 = 12.0;

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
        /// The empty tab stop list's text.
        no_stops: "tabStops.none" = "(none)";
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
///   selection) whose rows edit their stops in place: a position field in the unit, an alignment `Select` and a
///   leader `Select` whose options read as Word's list does ("1 ……"). Editing the position keeps the rows in order
///   until the edit ends, then [`TabStopsDialog::normalize`] restores position order and merges stops moved onto one
///   another (the edited row wins).
/// - The Tab stop position box shows the selected stop's position, as in Word; with none selected (as when the dialog
///   opens), the next free step, where Set puts a stop: the default interval when there are no stops. Set puts a stop at the box's position, or, when a stop already holds it (or the box is
///   empty), at the next free multiple of the default interval after the last stop; the new stop is left aligned, no
///   leader, selected. Clear moves the selected stop to "to be cleared"; Clear All clears every stop.
/// - Positions are clamped to ±[`MAX_POSITION`]; at most [`MAX_STOPS`] stops are read from a paragraph.
/// - Keys: a number field (Default tab stops, Tab stop position, a row's position) holds Enter and Escape from when the
///   user moves focus into it, clicks it or types in it until it takes one of them ([`FieldKeys`]). Enter in the Tab stop
///   position box is Set; in another field it takes the value. Otherwise Enter is OK and Escape is Cancel.
#[derive(craft_ui::contract::Pattern)]
pub struct TabStopsDialog {
    stops: Vec<TabStop>,
    /// The positions the paragraph's stops began at: one that no stop holds any more is cleared in the result.
    basis: Vec<f32>,
    cleared: Vec<f32>,
    default_interval: f32,
    selected: Option<usize>,
    edited: Option<usize>,
    unit: DocUnit,
    /// The row and position an in-place position edit began from, for Escape.
    draft: Option<(usize, f32)>,
    /// Which number field holds Enter and Escape.
    keys: FieldKeys,
    /// The selected row's Position field takes the focus the next time it shows: Set moves into the new stop.
    to_edit: bool,
    /// The Tab stop position box's value, points: Set or Enter there puts a stop at it, as in Word.
    typed_position: Option<f32>,
    /// Whether OK files its result for the app to judge instead of closing.
    hold: bool,
    /// Whether the dialog draws only its content and its Cancel and OK, for a window of its own that has the title and
    /// the close button (`DialogHost::run_windowed`).
    bare: bool,
    held: HeldAnswer<TabStopsResult>,
}

impl TabStopsDialog {
    /// The dialog over a paragraph's `stops` and the document's default interval (points).
    pub fn new(mut stops: Vec<TabStop>, default_interval: f32) -> Self {
        stops.retain(|stop| stop.position.is_finite());
        stops.truncate(MAX_STOPS);
        TabStopsDialog {
            basis: stops.iter().map(|stop| stop.position).collect(),
            stops,
            cleared: Vec::new(),
            default_interval,
            selected: None,
            edited: None,
            unit: DocUnit::default(),
            draft: None,
            keys: FieldKeys::default(),
            to_edit: false,
            typed_position: None,
            hold: false,
            bare: false,
            held: HeldAnswer::new(),
        }
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
        let Some(position) = self.next_step() else { return };
        self.stops.push(TabStop { position, align: TabAlign::Left, leader: TabLeader::None });
        self.stops.sort_by(|a, b| a.position.total_cmp(&b.position));
        self.select(self.stops.iter().position(|stop| (stop.position - position).abs() < SAME_POSITION));
        self.edited = None;
        self.to_edit = self.selected.is_some();
    }

    /// Where Set puts a stop when no position is typed: the next free multiple of the default interval after the last
    /// stop, past cleared positions; `None` past [`MAX_POSITION`]. An empty Tab stop position box shows it.
    fn next_step(&self) -> Option<f32> {
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
        (position.is_finite() && position <= MAX_POSITION).then_some(position)
    }

    /// Set with a position typed in the Tab stop position box: a stop there (Left, no leader), or the stop already
    /// there selected. A cleared position it lands on is no longer cleared.
    pub fn set_at(&mut self, position: f32) {
        if !position.is_finite() {
            return;
        }
        let position = position.clamp(-MAX_POSITION, MAX_POSITION);
        let at = |stops: &[TabStop]| stops.iter().position(|stop| (stop.position - position).abs() < SAME_POSITION);
        if at(&self.stops).is_none() {
            if self.stops.len() >= MAX_STOPS {
                return;
            }
            self.normalize();
            self.stops.push(TabStop { position, align: TabAlign::Left, leader: TabLeader::None });
            self.stops.sort_by(|a, b| a.position.total_cmp(&b.position));
            self.cleared.retain(|gone| (gone - position).abs() >= SAME_POSITION);
        }
        self.selected = at(&self.stops);
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

    /// Selects the stop at index `at`, or none. As in Word, the Tab stop position box shows a selected stop's position.
    pub fn select(&mut self, at: Option<usize>) {
        self.selected = at.filter(|at| *at < self.stops.len());
        if let Some(stop) = self.selected.map(|at| self.stops[at]) {
            self.typed_position = Some(stop.position);
        }
    }

    /// Clear: the selected stop moves to "to be cleared".
    pub fn clear(&mut self) {
        let Some(at) = self.selected.filter(|at| *at < self.stops.len()) else { return };
        let stop = self.stops.remove(at);
        self.cleared.push(stop.position);
        self.selected = None;
        self.typed_position = None;
        self.edited = None;
    }

    /// Clear All.
    pub fn clear_all(&mut self) {
        self.cleared.extend(self.stops.drain(..).map(|stop| stop.position));
        self.selected = None;
        self.typed_position = None;
        self.edited = None;
    }

    /// What OK returns: the stops normalized, the cleared positions and the default interval.
    pub fn result(&mut self) -> TabStopsResult {
        self.normalize();
        TabStopsResult { stops: self.stops.clone(), cleared: self.cleared_positions(), default_interval: self.default_interval }
    }

    /// The positions to clear: the ones cleared here, and the ones a stop began at that no stop holds now (a moved
    /// stop clears its old position).
    fn cleared_positions(&self) -> Vec<f32> {
        let mut cleared = self.cleared.clone();
        for at in &self.basis {
            let held = |position: f32| (position - at).abs() < SAME_POSITION;
            if !self.stops.iter().any(|stop| held(stop.position)) && !cleared.iter().any(|gone| held(*gone)) {
                cleared.push(*at);
            }
        }
        cleared
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

impl TabStopsDialog {
    /// OK files its result for the app to accept or refuse before the dialog closes.
    #[must_use]
    pub fn held_ok(mut self) -> Self {
        self.hold = true;
        self
    }

    /// Draws only the content and a row of Cancel and OK at its end, without the dialog's frame, title and close
    /// button, for a window of its own that has them (`DialogHost::run_windowed`). Enter is OK, Esc Cancel.
    #[must_use]
    pub fn bare(mut self, on: bool) -> Self {
        self.bare = on;
        self
    }

    /// The dialog's content alone, as a page of another dialog (the Paragraph dialog's Tabs page,
    /// [`crate::ui::character::ParagraphDialog::tabs_page`]).
    pub(crate) fn page(&mut self, ui: &mut Ui) {
        let env = Env::of(ui);
        let (words, units) = (env.words::<TabStopsWords>(), env.units());
        self.body(ui, &words, &units);
    }

    /// Whether a number field held Enter and Escape when this frame began: the keys this frame brought are that
    /// field's, not OK's or Cancel's. Asked before the body runs; the answer stays the same for the whole frame, so a
    /// pass egui runs again does not read the Enter a field took in the first as OK.
    pub(crate) fn field_holds_keys(&mut self, ctx: &egui::Context) -> bool {
        self.keys.at_frame_start(ctx.cumulative_frame_nr()).is_some()
    }

    /// Whether OK is the default button: no number field holds Enter.
    pub(crate) fn ok_is_default(&self) -> bool {
        self.keys.active.is_none()
    }

    /// Whether Set is the default button: drawn bare (a window of its own, or a bare dialog's page), while the Tab stop
    /// position box holds Enter, which is Set there.
    pub(crate) fn set_is_default(&self) -> bool {
        self.bare && self.keys.active == Some(NumberField::Position)
    }

    /// The bare content: the body and a refused answer's reason above Cancel and OK at its end; the button picked, if
    /// any.
    fn show_bare(
        &mut self,
        ui: &mut Ui,
        words: &TabStopsWords,
        units: &Units,
        labels: [Label; 2],
        waiting: bool,
        refusal: Option<Label>,
    ) -> Option<usize> {
        let mut picked = None;
        // Whose this frame's keys are is decided when it begins: the body may end the field's hold in this pass.
        let held = self.field_holds_keys(ui.ctx());
        ui.vertical(|ui| {
            ui.set_width(BARE_WIDTH);
            self.body(ui, words, units);
            if let Some(reason) = refusal {
                Notice::new(Tone::Bad, reason).layout(NoticeLayout::Inline).show(ui);
            }
            ui.add_space(BARE_GAP);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                // Right to left: OK at the end, Cancel before it.
                let [ok, cancel] = labels;
                let ok = Button::new(ok).enabled(!waiting);
                let ok = if self.ok_is_default() { ok.variant(Variant::PRIMARY) } else { ok };
                if ok.show(ui).clicks == 1 {
                    picked = Some(OK);
                }
                if Button::new(cancel).show(ui).clicks == 1 {
                    picked = Some(CANCEL);
                }
            });
        });
        let keys = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
        picked.or(match keys {
            _ if held => None,
            (_, true) => Some(CANCEL),
            (true, _) if !waiting => Some(OK),
            _ => None,
        })
    }

    /// The result OK filed, once, for the app to judge.
    pub fn take_ok(&mut self) -> Option<TabStopsResult> {
        self.held.take()
    }

    /// The app's answer to the filed result: `Ok` closes the dialog with it, `Err` shows the reason and enables OK.
    pub fn answer_ok(&mut self, answer: Result<(), craft_ui::runtime::env::Label>) {
        self.held.answer(answer);
    }

    /// Files `result` and keeps the dialog open when OK is held; a plain dialog closes with it.
    fn settle(&mut self, result: TabStopsResult) -> Option<TabStopsResult> {
        if !self.hold {
            return Some(result);
        }
        if !self.held.is_waiting() {
            self.held.file(result);
        }
        None
    }

    /// Whether the stops, the cleared positions and the interval are those of `result`.
    pub(crate) fn matches(&self, result: &TabStopsResult) -> bool {
        self.stops == result.stops && self.cleared_positions() == result.cleared && self.default_interval == result.default_interval
    }
}

impl StackDialog for TabStopsDialog {
    type Result = TabStopsResult;

    fn id(&self) -> DialogId {
        dialog_id("tab-stops")
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<TabStopsResult>> {
        if let Some(result) = self.held.finish() {
            return Some(DialogResult::Ok(result));
        }
        let env = Env::of(ui);
        let words = env.words::<TabStopsWords>();
        let common = env.words::<CommonWords>();
        let units = env.units();
        let waiting = self.held.is_waiting();
        let refused = self.held.refusal().map(|(reason, result)| (reason.clone(), result.clone()));
        let unchanged = Cell::new(true);
        let buttons = [
            FooterButton::new(common.ok.clone(), DialogRole::Accept).stays_open(self.hold),
            FooterButton::new(common.cancel.clone(), DialogRole::Cancel),
        ];
        if self.bare {
            let notice = refused.as_ref().filter(|(_, result)| self.matches(result)).map(|(reason, _)| reason.clone());
            let picked = self.show_bare(ui, &words, &units, [common.ok.clone(), common.cancel.clone()], waiting, notice);
            // A refused result stands until the stops change, as in the dialog.
            if refused.as_ref().is_some_and(|(_, result)| !self.matches(result)) {
                self.held.clear_refusal();
            }
            return match picked {
                Some(OK) => {
                    let result = self.result();
                    self.settle(result).map(DialogResult::Ok)
                }
                Some(_) => Some(DialogResult::Cancel),
                None => None,
            };
        }
        let shown = Dialog::new(words.title.clone())
            .buttons(buttons)
            .settle_buttons(|_, buttons| {
                if let Some(ok) = buttons.first_mut() {
                    ok.set_enabled(!waiting);
                }
            })
            .refusal(|| refused.as_ref().filter(|_| unchanged.get()).map(|(reason, _)| reason.clone()))
            .show(ui, |ui, _| {
                self.body(ui, &words, &units);
                if let Some((_, result)) = &refused {
                    unchanged.set(self.matches(result));
                }
            });
        if refused.is_some() && !unchanged.get() {
            self.held.clear_refusal();
        }
        // A held OK stays open: its pick files the answer, and only Cancel closes.
        if self.hold && shown.output.picked == Some(OK) {
            let result = self.result();
            return self.settle(result).map(DialogResult::Ok);
        }
        match shown.output.result(|| self.result()) {
            Some(DialogResult::Ok(result)) => self.settle(result).map(DialogResult::Ok),
            other => other,
        }
    }

    fn confirm(&mut self, _: &egui::Context) -> Option<TabStopsResult> {
        if let Some(result) = self.held.finish() {
            return Some(result);
        }
        let result = self.result();
        self.settle(result)
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }
}

/// A change made in a stop's row.
enum CellEdit {
    Position(EditEvent<Option<f64>>),
    Align(TabAlign),
    Leader(TabLeader),
}

impl TabStopsDialog {
    fn body(&mut self, ui: &mut Ui, words: &TabStopsWords, units: &Units) {
        let mut fields = Vec::new();
        self.default_row(ui, words, &mut fields);
        self.position_row(ui, words, &mut fields);
        self.table(ui, words, units, &mut fields);
        self.keys.follow(ui.ctx(), &fields);
        self.buttons(ui, words);
        if !self.cleared.is_empty() {
            let positions: Vec<String> = self.cleared.iter().map(|gone| self.text(units, *gone)).collect();
            craft_ui::display::Text::new(format!("{} {}", words.to_be_cleared.as_str(), positions.join(", "))).show(ui);
        }
    }

    /// `points` as the number in the unit with its suffix.
    fn text(&self, units: &Units, points: f32) -> String {
        units.show(self.unit.kind, shown(units, self.unit, f64::from(points)))
    }

    fn default_row(&mut self, ui: &mut Ui, words: &TabStopsWords, fields: &mut Vec<FieldPass>) {
        let unit = self.unit;
        let points = Mixed::Same(Some(f64::from(self.default_interval)));
        let range = f64::from(SAME_POSITION)..=f64::from(MAX_POSITION);
        let edit = Field::new(words.default_stops.clone())
            .show(ui, |ui, _| length_field(unit, words.default_stops.clone(), points, range.clone()).show(ui))
            .inner;
        if let Some(out) = &edit {
            fields.push(FieldPass::of(ui.ctx(), NumberField::Default, out.response.id, &out.edit));
        }
        for event in edit.into_iter().flat_map(|out| out.edit.events) {
            if !matches!(event, EditEvent::Update { .. } | EditEvent::Commit { .. } | EditEvent::Set { .. }) {
                continue;
            }
            if let Some(Some(points)) = event.value() {
                self.default_interval = *points as f32;
            }
        }
    }

    /// The Tab stop position box: Enter there is Set, a stop at the typed position.
    fn position_row(&mut self, ui: &mut Ui, words: &TabStopsWords, fields: &mut Vec<FieldPass>) {
        // An empty box shows where Set would put a stop (the next free step), and follows it until a value is typed.
        let next = self.next_step();
        let points = Mixed::Same(self.typed_position.or(next).map(f64::from));
        let range = -f64::from(MAX_POSITION)..=f64::from(MAX_POSITION);
        let unit = self.unit;
        let Some(out) =
            Field::new(words.position.clone()).show(ui, |ui, _| length_field(unit, words.position.clone(), points, range.clone()).show(ui)).inner
        else {
            return;
        };
        fields.push(FieldPass::of(ui.ctx(), NumberField::Position, out.response.id, &out.edit));
        for event in &out.edit.events {
            let typed = matches!(event, EditEvent::Update { .. });
            // Leaving an empty box commits the step it shows, which is no position typed.
            let kept = matches!(event, EditEvent::Commit { .. } | EditEvent::Set { .. })
                && (self.typed_position.is_some() || event.value() != Some(&next.map(f64::from)));
            if (typed || kept)
                && let Some(Some(points)) = event.value()
            {
                self.typed_position = Some(*points as f32);
            }
        }
        // Enter in the box is Set at the position it shows: the one typed, or the next free step.
        if out.edit.submitted
            && let Some(position) = self.typed_position.or(next)
        {
            self.set_at(position);
        }
    }

    /// The stops table: each row edits its stop in place (a position field, an alignment and a leader `Select`); a
    /// selection change moves the selected stop.
    fn table(&mut self, ui: &mut Ui, words: &TabStopsWords, units: &Units, fields: &mut Vec<FieldPass>) {
        let columns = vec![
            // Wide enough for its header with no rows, and for the position field.
            ColumnSpec::new(POSITION, words.position_column.clone(), ColumnWidth::Controls(4.0)),
            ColumnSpec::new(ALIGNMENT, words.alignment.clone(), ColumnWidth::fill(1.0)),
            ColumnSpec::new(LEADER, words.leader.clone(), ColumnWidth::fill(1.0)),
        ];
        let texts: Vec<String> = self.stops.iter().map(|stop| self.text(units, stop.position)).collect();
        let selection = self.selected.map_or_else(Selection::default, Selection::single);
        let stops = self.stops.clone();
        let unit = self.unit;
        let aligns: Vec<Entry<TabAlign>> = TabAlign::ALL.into_iter().map(|align| Entry::new(align, align_text(words, align))).collect();
        // The closed box shows the fill alone, as Word's does; the list reads "1 ……".
        let leaders: Vec<Entry<TabLeader>> = TabLeader::ALL
            .into_iter()
            .map(|leader| Entry::new(leader, leader_text(words, leader)).closed_label(leader_face(words, leader)))
            .collect();
        let range = -f64::from(MAX_POSITION)..=f64::from(MAX_POSITION);
        let edits: RefCell<Vec<(usize, CellEdit)>> = RefCell::new(Vec::new());
        let position_ids: RefCell<Vec<(usize, egui::Id)>> = RefCell::new(Vec::new());
        let noted = RefCell::new(Vec::new());
        let ctx = ui.ctx().clone();
        let out = Table::<usize>::new(words.title.clone(), columns.clone())
            .empty(words.no_stops.clone())
            .select(SelectMode::Single)
            .min_rows(1)
            .max_rows(6)
            .selection(Bind::given(selection))
            .show_items(ui, &Stops { stops: &stops, texts: &texts }, &mut |key, icx| {
                let (at, stop) = (*key, stops[*key]);
                let (edits, position_ids, noted, ctx) = (&edits, &position_ids, &noted, &ctx);
                let points = Mixed::Same(Some(f64::from(stop.position)));
                // Named after its column: "Tab stop position:" is the box above the table.
                let position = length_field(unit, words.position_column.clone(), points, range.clone());
                let align = Select::new(words.alignment.clone(), Bind::given(stop.align), &aligns);
                let leader = Select::new(words.leader.clone(), Bind::given(stop.leader), &leaders);
                ListRow::new(texts[at].clone())
                    .cells(icx.cells())
                    .cell_field(POSITION, position, move |out| {
                        position_ids.borrow_mut().push((at, out.response.id));
                        noted.borrow_mut().push(FieldPass::of(ctx, NumberField::Row, out.response.id, &out.edit));
                        edits.borrow_mut().extend(out.edit.events.into_iter().map(|event| (at, CellEdit::Position(event))));
                    })
                    .cell_field(ALIGNMENT, align, move |out| {
                        edits.borrow_mut().extend(out.value().copied().map(|align| (at, CellEdit::Align(align))))
                    })
                    .cell_field(LEADER, leader, move |out| {
                        edits.borrow_mut().extend(out.value().copied().map(|leader| (at, CellEdit::Leader(leader))));
                    })
            });
        fields.extend(noted.into_inner());
        if self.to_edit
            && let Some((_, id)) = position_ids.borrow().iter().find(|(at, _)| Some(*at) == self.selected)
        {
            self.to_edit = !focus::request(ui.ctx(), FocusTarget::Id(*id));
        }
        for (at, edit) in edits.into_inner() {
            match edit {
                CellEdit::Position(event) => {
                    let event = event.map(|points| points.map(|p| p as f32));
                    self.apply_position(at, event);
                }
                CellEdit::Align(align) => self.stops[at].align = align,
                CellEdit::Leader(leader) => self.stops[at].leader = leader,
            }
        }
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
            let set = Button::new(words.set.clone()).enabled(self.stops.len() < MAX_STOPS);
            let set = if self.set_is_default() { set.variant(Variant::PRIMARY) } else { set };
            if clicked(&set.show(ui)) {
                // The box shows the selected stop: Set with a position no stop holds puts one there, otherwise it adds
                // the next free step, so Set always adds a stop.
                let held = |at: f32| self.stops.iter().any(|stop| (stop.position - at).abs() < SAME_POSITION);
                match self.typed_position.filter(|at| !held(*at)) {
                    Some(position) => self.set_at(position),
                    None => self.new_stop(),
                }
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

/// The page's number fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NumberField {
    /// Default tab stops: Enter takes the interval.
    Default,
    /// Tab stop position: Enter is Set.
    Position,
    /// A stop row's position: Enter takes the position.
    Row,
}

/// What one number field reported in a pass.
struct FieldPass {
    field: NumberField,
    /// The component, which focus is on when it is on the field's text.
    id: egui::Id,
    /// It took an Enter or an Escape, which ended its edit.
    ended: bool,
    /// A value was typed in it.
    typed: bool,
    /// The pointer was pressed on it.
    pressed: bool,
}

impl FieldPass {
    fn of<T>(ctx: &egui::Context, field: NumberField, id: egui::Id, edit: &craft_ui::contract::EditResponse<T>) -> Self {
        let typed = edit.events.iter().any(|event| matches!(event, EditEvent::Update { .. }));
        let rect = edit.response.rect;
        let pressed = ctx.input(|i| i.pointer.any_pressed() && i.pointer.press_origin().is_some_and(|at| rect.contains(at)));
        FieldPass { field, id, ended: edit.submitted || edit.escaped, typed, pressed }
    }
}

/// Which number field holds Enter and Escape: the focused one, from when the user moves focus into it (a click, Tab,
/// Set moving into a new row), clicks it or types a value in it, until it takes Enter or Escape. After that the keys
/// are OK's and Cancel's again, though focus stays in the field. Focus the dialog puts on its first field when it
/// opens is not the user's ([`FocusOrigin::Open`]): Enter there is OK, as in Word.
#[derive(Default)]
struct FieldKeys {
    /// The component focus was on at the end of the last pass, when it was a number field.
    focused: Option<egui::Id>,
    /// The field holding the keys.
    active: Option<NumberField>,
    /// The frame and the field that held the keys when it began.
    frame: Option<(u64, Option<NumberField>)>,
}

impl FieldKeys {
    /// The field that held the keys when frame `frame` began.
    fn at_frame_start(&mut self, frame: u64) -> Option<NumberField> {
        match self.frame {
            Some((at, field)) if at == frame => field,
            _ => {
                self.frame = Some((frame, self.active));
                self.active
            }
        }
    }

    /// Follows focus and edits over a pass's `fields`.
    fn follow(&mut self, ctx: &egui::Context, fields: &[FieldPass]) {
        let focused = fields.iter().find(|pass| focus::has(ctx, pass.id));
        match focused {
            Some(pass) if Some(pass.id) != self.focused => {
                let by_user = matches!(focus::arrival(ctx, pass.id), Some(FocusOrigin::Press | FocusOrigin::Request));
                self.active = by_user.then_some(pass.field);
            }
            Some(_) => {}
            None => self.active = None,
        }
        self.focused = focused.map(|pass| pass.id);
        // A click into the field that already has focus (the one the dialog opened on) or typing there.
        if let Some(pass) = focused.filter(|pass| pass.typed || pass.pressed) {
            self.active = Some(pass.field);
        }
        // Enter or Escape in the focused field (a field Tab leaves ends its edit too, but the keys go with focus).
        if focused.is_some_and(|pass| pass.ended) {
            self.active = None;
        }
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

/// A leader's text in its closed box: "None", or the fill alone.
pub fn leader_face(words: &TabStopsWords, kind: TabLeader) -> String {
    if kind == TabLeader::None { words.leader_none.as_str().to_owned() } else { kind.sample().to_owned() }
}

/// A leader option's text as Word's list reads: "None", or the fill after its number.
pub fn leader_text(words: &TabStopsWords, kind: TabLeader) -> String {
    match TabLeader::ALL.iter().position(|other| *other == kind) {
        Some(n) if kind != TabLeader::None => words.leader_option.fill(&[("n", &n.to_string()), ("fill", kind.sample())]),
        _ => words.leader_none.as_str().to_owned(),
    }
}
