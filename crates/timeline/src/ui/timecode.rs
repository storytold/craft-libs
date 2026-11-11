//! The timecode field, its codec and the Timecode panel.

#[cfg(feature = "screen-testing")]
mod stories;

use craft_behavior::nav::LARGE_STEPS;
use craft_behavior::{Caret, Codec, DisplayOrder, IconId, ItemSource, NavMove, NotAValue, PartKey, Selection, Sense, ValueDrag};
use craft_ui::buttons::IconButton;
use craft_ui::contract::{
    Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, Held, Input, MeasureCx, NoValue, Outcome, PartsCx, Props, Role, Scrub,
    SelectMode, Show as _, ShowCollection as _, ShowContainer as _, clicked,
};
use craft_ui::display::Divider;
use craft_ui::layout_components::Stack;
use craft_ui::list::{List, ListRow};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::primitives::{TextKey, TextOptions, TextRun};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::focus::KeyUse;
use craft_ui::runtime::memory::Remember as _;
use craft_ui::style::{Ink, StyleCx, TypeRole};
use craft_ui::text::FieldStyle;
use egui::{Rect, Response, Ui, Vec2};
use std::borrow::Cow;
use std::ops::Deref;

use crate::time::Tick;
use crate::timecode::{TimeDisplay, TimeFormat, TimecodeError, format_time, parse_time};

/// Frames the Up and Down keys step a field.
const STEP_FRAMES: i64 = 1;
/// The widest time a display writes, for the field's width: 99:59:59 in seconds.
const WIDEST_SECONDS: f64 = 359_999.0;
/// The part key of the field's one text part.
const TEXT: PartKey = PartKey::new("text");

craft_ui::words! {
    /// The words of the timecode field and the Timecode panel, English by default.
    #[non_exhaustive]
    pub struct TimecodeWords {
        timecode: "timeDisplay.timecode" = "Timecode";
        frames: "timeDisplay.frames" = "Frames";
        feet_16: "timeDisplay.feet16" = "Feet + Frames 16mm";
        feet_35: "timeDisplay.feet35" = "Feet + Frames 35mm";
        audio_samples: "timeDisplay.audioSamples" = "Audio Samples";
        seconds: "timeDisplay.seconds" = "Seconds";
        empty: "timecodeField.empty" = "Enter a time.";
        not_a_time: template "timecodeField.notATime" = "\u{201c}{input}\u{201d} is not a time.";
        current_time: "timecodePanel.currentTime" = "Current Time";
        media_time: "timecodePanel.mediaTime" = "Media Time";
        duration: "timecodePanel.duration" = "Duration";
        in_out_duration: "timecodePanel.inOutDuration" = "In/Out Duration";
        remaining: "timecodePanel.remaining" = "Remaining";
        program: "timecodePanel.program" = "Program";
        source: "timecodePanel.source" = "Source";
        active: "timecodePanel.active" = "Active Monitor";
        add_row: "timecodePanel.addRow" = "Add a timecode row";
        remove_row: "timecodePanel.removeRow" = "Remove Row";
        sequence_display: "timecodePanel.sequenceDisplay" = "Sequence Display";
        menu: "timecodePanel.menu" = "Row options";
        caption: template "timecodePanel.caption" = "{kind} \u{b7} {source}";
        spoken: template "timecodePanel.spoken" = "{caption}: {value}";
    }
}

impl TimecodeWords {
    /// The name of `display`.
    pub fn display_name(&self, display: TimeDisplay) -> &str {
        match display {
            TimeDisplay::Timecode => self.timecode.as_str(),
            TimeDisplay::Frames => self.frames.as_str(),
            TimeDisplay::Feet16 => self.feet_16.as_str(),
            TimeDisplay::Feet35 => self.feet_35.as_str(),
            TimeDisplay::AudioSamples => self.audio_samples.as_str(),
            TimeDisplay::Seconds => self.seconds.as_str(),
        }
    }

    /// The notice for typed text that is not a time.
    pub fn message(&self, error: &TimecodeError) -> String {
        match error {
            TimecodeError::Empty => self.empty.as_str().to_owned(),
            TimecodeError::NotATime(input) => self.not_a_time.fill(&[("input", input)]),
        }
    }

    fn kind_name(&self, kind: TimecodeRowKind) -> &str {
        match kind {
            TimecodeRowKind::CurrentTime => self.current_time.as_str(),
            TimecodeRowKind::MediaTime => self.media_time.as_str(),
            TimecodeRowKind::Duration => self.duration.as_str(),
            TimecodeRowKind::InOutDuration => self.in_out_duration.as_str(),
            TimecodeRowKind::Remaining => self.remaining.as_str(),
        }
    }

    fn source_name(&self, source: TimecodeSource) -> &str {
        match source {
            TimecodeSource::Active => self.active.as_str(),
            TimecodeSource::Program => self.program.as_str(),
            TimecodeSource::Source => self.source.as_str(),
        }
    }
}

/// Writes a time in a display and reads typed text back with [`parse_time`], never before `min`. Text equal to the
/// shown text stands for the time itself, not the first tick of its frame.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TimecodeCodec {
    format: TimeFormat,
    min: Tick,
}

impl TimecodeCodec {
    /// A codec for times shown in `format`, from zero on.
    pub fn new(format: TimeFormat) -> Self {
        TimecodeCodec { format, min: Tick::ZERO }
    }

    /// The earliest time the codec reads, such as the sequence start.
    #[must_use]
    pub fn min(mut self, min: Tick) -> Self {
        self.min = min;
        self
    }
}

impl Codec for TimecodeCodec {
    type Value = Tick;

    fn format(&self, v: &Tick) -> String {
        format_time(*v, &self.format)
    }

    fn parse(&self, text: &str, start: &Tick) -> Result<Tick, NotAValue> {
        if text == self.format(start) {
            return Ok(*start);
        }
        parse_time(text, &self.format, *start).map(|time| time.max(self.min)).map_err(|_| NotAValue)
    }

    fn caret_on_focus(&self, _text: &str) -> Caret {
        Caret::All
    }

    fn sample(&self, current: &Tick) -> Cow<'_, str> {
        let widest = self.format(&Tick::from_seconds_f64(WIDEST_SECONDS));
        let shown = self.format(current);
        Cow::Owned(if shown.chars().count() > widest.chars().count() { shown } else { widest })
    }
}

/// What a [`TimecodeField`] did this pass.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the edit event"]
pub struct TimecodeFieldOutput {
    /// The edit: a changed time as an edit event.
    pub edit: EditResponse<Tick>,
    /// Typed text that is not a time, from Enter, in every pass the text stays marked invalid; word it with [`TimecodeWords::message`].
    /// Nothing is committed: the text stays in the field, marked invalid, until it is fixed or Esc restores the time.
    pub error: Option<TimecodeError>,
}

impl Deref for TimecodeFieldOutput {
    type Target = EditResponse<Tick>;

    fn deref(&self) -> &Self::Target {
        &self.edit
    }
}

impl Outcome for TimecodeFieldOutput {
    type Value = Tick;

    fn response(&self) -> &Response {
        &self.edit.response
    }
}

/// A time as text: Premiere's timecode field.
///
/// - Click (or Tab) to type: the text is selected; Enter or a click away commits through [`parse_time`] relative to
///   the current value; Esc cancels. A time that does not parse commits nothing and is reported on
///   [`TimecodeFieldOutput::error`].
/// - A drag on the field's side label scrubs one frame per point, Shift ten, as a gesture of `EditEvent<Tick>`s.
/// - Never before `min` (the sequence start). One Tab stop; Up and Down step one frame, Shift ten.
/// - AccessKit: a spin button whose text is the formatted time.
#[derive(Component, Props)]
pub struct TimecodeField<'a> {
    #[common]
    common: Common,
    value: Bind<'a, Tick>,
    format: TimeFormat,
    /// The earliest time the field accepts, such as the sequence start; zero by default.
    #[prop]
    min: Tick,
    options: TextOptions,
    /// The readout form: large accent text that scrubs under a drag on the field itself.
    large: bool,
}

impl<'a> TimecodeField<'a> {
    /// A field named `label` that shows and edits `value` in `format`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, Tick>, format: TimeFormat) -> Self {
        let mut options = TextOptions::new();
        options.mono = true;
        options.select_on_focus = true;
        options.submits = true;
        options.steps_value = true;
        TimecodeField { common: Common::new(), value, format, min: Tick::ZERO, options, large: false }.label(label)
    }

    /// The large readout: the prominent text role in the accent ink. A drag on the field itself scrubs the time, as a
    /// drag on the side label does for the plain field.
    #[must_use]
    pub fn large(mut self) -> Self {
        self.options.mono = false;
        self.options.role = TypeRole::BodyLarge;
        self.options.ink = Some(Ink::Accent);
        self.large = true;
        self
    }

    fn codec(&self) -> TimecodeCodec {
        TimecodeCodec::new(self.format).min(self.min)
    }

    /// The time `frames` frames from `from`, never before `min`.
    fn stepped(&self, from: Tick, frames: i64) -> Tick {
        let rate = self.format.rate;
        rate.tick_of(rate.frame_at(from).saturating_add(frames)).max(self.min)
    }
}

impl Component for TimecodeField<'_> {
    const ROLE: Role = Role::SPIN_BUTTON;
    const INPUT: Input = Input::Text;
    const KEYS: KeyUse = KeyUse::of(Input::Text).with_moves();
    const OVERLAYS: bool = false;
    type Style = FieldStyle;
    type Output = TimecodeFieldOutput;

    fn style(cx: &StyleCx<'_>, _: TimecodeFieldStyleKey) -> FieldStyle {
        FieldStyle::at(cx)
    }

    fn sense(&self) -> Sense {
        if self.large { Sense::CLICK | Sense::DRAG | Sense::FOCUS | Sense::TAKE_FOCUS } else { Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let codec = self.codec();
        let sample = codec.sample(cx.value().unwrap_or(&Tick::ZERO));
        let role = if self.options.mono { TypeRole::Mono } else { self.options.role };
        cx.layout(&TextRun::new(sample.as_ref()).role(role)).size()
    }

    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let codec = self.codec();
        let rect = cx.content_rect;
        if let Some(value) = cx.value() {
            cx.text(TextKey::new(TEXT), rect, &codec, value, &self.options);
        }
    }

    fn bind(&mut self) -> Option<BindMut<'_, Tick>> {
        Some(self.value.reborrow())
    }

    /// A drag on the side label moves the time a frame per point.
    fn scrub(&self) -> Option<Scrub<Tick>> {
        let (rate, min) = (self.format.rate, self.min);
        let frames = rate.frame_at(min) as f64..=rate.frame_at(Tick::MAX) as f64;
        let drag = ValueDrag::relative(1.0, frames).warp(true).hide_cursor(true).coarse(LARGE_STEPS);
        let scrub = Scrub::new(drag, move |time: &Tick| rate.frame_at(*time) as f64, move |frames: f64| rate.tick_of(frames.round() as i64).max(min));
        Some(if self.large { scrub.on_field() } else { scrub })
    }

    /// Up and Down step the time by a frame, with Shift by ten.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        while let Some((mv, mods)) = cx.intents().take_move() {
            let frames = STEP_FRAMES * if mods.extend { LARGE_STEPS as i64 } else { 1 };
            let from = cx.value().copied().unwrap_or(Tick::ZERO);
            match mv {
                NavMove::Up => cx.set(self.stepped(from, frames)),
                NavMove::Down => cx.set(self.stepped(from, -frames)),
                _ => {}
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> TimecodeFieldOutput {
        let edit = cx.edit_response();
        let draft = cx.text(&TextKey::<Tick>::new(TEXT)).and_then(|text| text.draft.as_deref());
        // The edit decides what is not a time (a rejected Enter keeps the draft and marks it unparsed); the parse here
        // only words the reason.
        let error = match draft {
            Some(draft) if edit.unparsed => parse_time(draft, &self.format, cx.value().copied().unwrap_or(Tick::ZERO)).err(),
            _ => None,
        };
        TimecodeFieldOutput { edit, error }
    }
}

/// What a Timecode panel row shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TimecodeRowKind {
    /// The playhead time.
    #[default]
    CurrentTime,
    /// The source timecode of the clip under the playhead.
    MediaTime,
    /// The length of the content.
    Duration,
    /// The length between the In and Out marks.
    InOutDuration,
    /// The time left after the playhead.
    Remaining,
}

/// Which monitor a row reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TimecodeSource {
    /// The Source monitor when it has the focus and an item, else the Program.
    #[default]
    Active,
    /// The Program monitor.
    Program,
    /// The Source monitor.
    Source,
}

/// One row of the Timecode panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct TimecodeRow {
    /// What the row shows.
    pub kind: TimecodeRowKind,
    /// Which monitor the row reads.
    pub source: TimecodeSource,
    /// `None`: the sequence's own display.
    pub display: Option<TimeDisplay>,
}

impl TimecodeRow {
    /// A row of `kind` on the Active monitor in the sequence's display.
    pub fn new(kind: TimecodeRowKind) -> Self {
        TimecodeRow { kind, ..TimecodeRow::default() }
    }

    /// The monitor the row reads.
    #[must_use]
    pub fn source(mut self, source: TimecodeSource) -> Self {
        self.source = source;
        self
    }

    /// The display the row shows; `None` follows the sequence.
    #[must_use]
    pub fn display(mut self, display: Option<TimeDisplay>) -> Self {
        self.display = display;
        self
    }
}

/// The Timecode panel's rows, saved by the app with its panel state.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct TimecodePanelSettings {
    /// The rows, the first one emphasised.
    pub rows: Vec<TimecodeRow>,
}

impl TimecodePanelSettings {
    /// Settings with `rows`.
    pub fn new(rows: Vec<TimecodeRow>) -> Self {
        TimecodePanelSettings { rows }
    }
}

/// Gives the text of a [`TimecodeRow`]: the app knows the clip under the playhead.
pub struct TimecodeText<'a>(Box<dyn Fn(&TimecodeRow) -> String + 'a>);

impl<'a> TimecodeText<'a> {
    /// The text `text` gives for a row.
    pub fn new(text: impl Fn(&TimecodeRow) -> String + 'a) -> Self {
        TimecodeText(Box::new(text))
    }

    fn of(&self, row: &TimecodeRow) -> String {
        (self.0)(row)
    }
}

/// What the Timecode panel did this pass.
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub struct TimecodePanelOutput {
    /// The panel's response, for hover and focus.
    pub response: Response,
    /// A row was added, removed or changed; the app saves [`TimecodePanelSettings`].
    pub settings_changed: bool,
}

impl Outcome for TimecodePanelOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// What a row's menu does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowCommand {
    Kind(TimecodeRowKind),
    Source(TimecodeSource),
    Display(Option<TimeDisplay>),
    Remove,
}

/// What a row shows and says.
struct RowText {
    value: String,
    caption: String,
    spoken: String,
}

/// The rows as a collection source, keyed by position.
struct Rows<'a> {
    texts: &'a [RowText],
}

impl DisplayOrder<usize> for Rows<'_> {
    fn len(&self) -> usize {
        self.texts.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.texts[at].value.as_str()))
    }
}

impl ItemSource<usize> for Rows<'_> {}

/// The panel's session state: the row the menu acts on.
#[derive(Clone, Debug, Default)]
struct PanelMemory {
    selection: Selection<usize>,
}

/// A row's menu: its kind, its monitor, its display, and Remove Row while another row is left.
fn row_menu(ui: &mut Ui, words: &TimecodeWords, row: &TimecodeRow, removable: bool) -> Option<RowCommand> {
    let mut ran = None;
    let mut item = |ui: &mut Ui, label: &str, on: bool, command: RowCommand| {
        if MenuItem::new(label.to_owned()).radio_mark(on).show(ui).ran {
            ran = Some(command);
        }
    };
    for kind in [
        TimecodeRowKind::CurrentTime,
        TimecodeRowKind::MediaTime,
        TimecodeRowKind::Duration,
        TimecodeRowKind::InOutDuration,
        TimecodeRowKind::Remaining,
    ] {
        item(ui, words.kind_name(kind), row.kind == kind, RowCommand::Kind(kind));
    }
    Divider::horizontal().show(ui);
    for source in [TimecodeSource::Active, TimecodeSource::Program, TimecodeSource::Source] {
        item(ui, words.source_name(source), row.source == source, RowCommand::Source(source));
    }
    Divider::horizontal().show(ui);
    item(ui, words.sequence_display.as_str(), row.display.is_none(), RowCommand::Display(None));
    for display in TimeDisplay::ALL {
        item(ui, words.display_name(display), row.display == Some(display), RowCommand::Display(Some(display)));
    }
    if removable {
        Divider::horizontal().show(ui);
        item(ui, words.remove_row.as_str(), false, RowCommand::Remove);
    }
    ran
}

/// Premiere's Timecode panel: each row's text with its caption ("Current Time \u{b7} Program"), the first row
/// emphasised. The app gives each row's text. A secondary click, the Menu key or Shift+F10 on a row opens its menu
/// (kind, source, display with "Sequence Display" to follow the sequence again, Remove Row); "+" adds a row. The rows
/// are one list: one Tab stop, the arrows move.
#[derive(craft_ui::contract::Pattern)]
pub struct TimecodePanel<'a> {
    label: Label,
    settings: Bind<'a, TimecodePanelSettings>,
    text: TimecodeText<'a>,
}

impl<'a> TimecodePanel<'a> {
    /// A panel named `label` over `settings`, with `text` giving each row's text.
    pub fn new(label: impl Into<Label>, settings: Bind<'a, TimecodePanelSettings>, text: TimecodeText<'a>) -> Self {
        TimecodePanel { label: label.into(), settings, text }
    }

    /// Shows the panel and reports whether the settings changed.
    pub fn show(self, ui: &mut Ui) -> TimecodePanelOutput {
        let TimecodePanel { label, settings, text } = self;
        let words = Env::of(ui).words::<TimecodeWords>();
        let name = label.as_str().to_owned();
        let mut changed = false;
        let mut held = Held::new(settings);
        let shown = Stack::new().label(label.clone()).show(ui, |ui, root| {
            let before = held.read(root).unwrap_or_default();
            let mut next = before.clone();
            let texts: Vec<RowText> = before
                .rows
                .iter()
                .map(|row| {
                    let value = text.of(row);
                    let caption = words.caption.fill(&[("kind", words.kind_name(row.kind)), ("source", words.source_name(row.source))]);
                    let spoken = words.spoken.fill(&[("caption", &caption), ("value", &value)]);
                    RowText { value, caption, spoken }
                })
                .collect();
            let mut selection = root.keep::<PanelMemory>().selection.clone();
            let source = Rows { texts: &texts };
            let out = List::<usize>::new(label.clone()).select(SelectMode::Single).selection(Bind::bound(&mut selection)).show_items(
                ui,
                &source,
                &mut |key, _| {
                    let row = &texts[*key];
                    ListRow::new(row.value.clone()).detail(row.caption.clone()).strong(*key == 0).accessible_name(row.spoken.clone())
                },
            );
            let target = selection.lead.or_else(|| selection.items.first().copied());
            root.keep::<PanelMemory>().selection = selection;
            let removable = before.rows.len() > 1;
            let ran = Menu::context(words.menu.clone(), &out.response)
                .show(ui, |ui, _| target.and_then(|at| before.rows.get(at)).and_then(|row| row_menu(ui, &words, row, removable)))
                .inner
                .flatten();
            if let (Some(command), Some(at)) = (ran, target)
                && let Some(row) = next.rows.get_mut(at)
            {
                match command {
                    RowCommand::Kind(kind) => row.kind = kind,
                    RowCommand::Source(source) => row.source = source,
                    RowCommand::Display(display) => row.display = display,
                    RowCommand::Remove => {
                        next.rows.remove(at);
                    }
                }
            }
            let add = IconButton::new(IconId::new("plus")).label(words.add_row.clone()).show(ui);
            if clicked(&add) {
                let from = next.rows.last().copied().unwrap_or_default();
                next.rows.push(TimecodeRow { kind: TimecodeRowKind::Duration, ..from });
            }
            if next != before {
                changed = true;
                held.write(root, &before, next, &name);
            }
        });
        TimecodePanelOutput { response: shown.output, settings_changed: changed }
    }
}
