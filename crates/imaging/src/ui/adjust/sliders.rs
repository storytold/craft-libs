//! [`AdjustSliders`]: a table of slider rows for the adjustments that are only sliders (Exposure, Vibrance,
//! Posterize, VectorCraft's colour effects), and the row machinery every adjustment editor shares.
//!
//! An editor's value is one binding. Each row is a `SliderField` in the editor's `FieldGroup`; the
//! rows' edits are lifted onto the editor's value ([`Run`]), so one slider drag, one typed number or one reset is one
//! gesture of the whole value.

#[cfg(feature = "screen-testing")]
mod stories;

use std::ops::RangeInclusive;
use std::sync::Arc;

use craft_behavior::{EditEvent, EditEvents, Mixed, Value, decimals_for_step};
use craft_ui::contract::{Bind, BodyCx, EditResponse, Pattern, Props as _, Remember as _, Show as _, ShowContainer as _};
use craft_ui::form::{FieldGroup, SliderField};
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::style::DataColor;
use craft_ui::value_drag::TrackFace;
use egui::{Response, Ui};

use super::{AdjustContext, AdjustWords};
use crate::ui::curve::ToneRamp;

/// A slider row's track.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum AdjustTrack {
    /// The slider's own track.
    #[default]
    Plain,
    /// A colour track through these stops, start to end (an image tone, not chrome), drawn only while
    /// [`AdjustContext::dynamic_color_sliders`] is on; the plain track otherwise.
    Colors(Vec<DataColor>),
}

/// One row of an [`AdjustSliders`] table.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct AdjustField {
    /// The host's key for the value (the engine parameter's name); the library never reads it.
    pub key: &'static str,
    /// The row's caption.
    pub label: Label,
    /// The slider's value range, in the field's unit.
    pub range: RangeInclusive<f64>,
    /// The value a reset of the row restores.
    pub default: f64,
    /// What the number counts, none by default.
    pub unit_kind: Option<UnitKind>,
    /// The change of one arrow press and the grid typed values snap to; hundredths on a range 10 wide or less and
    /// whole numbers above.
    pub step: Option<f64>,
    /// The track behind the slider.
    pub track: AdjustTrack,
}

impl AdjustField {
    /// A row with a plain track, no unit and the range's own step.
    pub fn new(key: &'static str, label: impl Into<Label>, range: RangeInclusive<f64>, default: f64) -> Self {
        Self { key, label: label.into(), range, default, unit_kind: None, step: None, track: AdjustTrack::Plain }
    }

    /// What the number counts.
    #[must_use]
    pub fn unit_kind(mut self, kind: UnitKind) -> Self {
        self.unit_kind = Some(kind);
        self
    }

    /// The step of the row; a value the row reports is on it.
    #[must_use]
    pub fn step(mut self, step: f64) -> Self {
        self.step = Some(step);
        self
    }

    /// The row's track.
    #[must_use]
    pub fn track(mut self, track: AdjustTrack) -> Self {
        self.track = track;
        self
    }

    /// The value the row stores for `x`: rounded to the places of its step.
    pub fn round(&self, x: f64) -> f64 {
        let scale = 10f64.powi(decimals_for_step(self.stride()) as i32);
        (x * scale).round() / scale
    }

    /// The row's step: the one set, else 0.01 for a range 10 wide or less and 1 above.
    fn stride(&self) -> f64 {
        match self.step {
            Some(step) => step,
            None if self.range.end() - self.range.start() <= 10.0 => 0.01,
            None => 1.0,
        }
    }

    /// `x` held in the row's range.
    fn clamp(&self, x: f64) -> f64 {
        x.max(*self.range.start()).min(*self.range.end())
    }

    /// The row's slider over `value`, with its colour track while `context` shows tracks.
    fn slider(&self, value: f64, context: AdjustContext) -> SliderField<'_> {
        let mut slider = SliderField::new(self.label.clone(), Bind::given(value).reset(self.default), self.range.clone()).step(self.stride());
        if let Some(kind) = self.unit_kind {
            slider = slider.unit_kind(kind);
        }
        match &self.track {
            AdjustTrack::Colors(stops) if context.dynamic_color_sliders() => slider.track_face(TrackFace::new(ToneRamp::new(stops))),
            _ => slider,
        }
    }

    /// Exposure's rows: Exposure −20..20 (2 places), Offset −0.5..0.5 (4 places), Gamma Correction 0.01..9.99
    /// (default 1); keys `exposure`, `offset`, `gamma`.
    pub fn exposure(words: &AdjustWords) -> Vec<AdjustField> {
        vec![
            AdjustField::new("exposure", words.exposure.clone(), -20.0..=20.0, 0.0).step(0.01),
            AdjustField::new("offset", words.offset.clone(), -0.5..=0.5, 0.0).step(0.0001),
            AdjustField::new("gamma", words.gamma_correction.clone(), 0.01..=9.99, 1.0),
        ]
    }

    /// Vibrance's rows: Vibrance and Saturation, −100..100 %, default 0; keys `vibrance`, `saturation`.
    pub fn vibrance(words: &AdjustWords) -> Vec<AdjustField> {
        vec![
            AdjustField::new("vibrance", words.vibrance.clone(), -100.0..=100.0, 0.0).unit_kind(UnitKind::Percent),
            AdjustField::new("saturation", words.saturation.clone(), -100.0..=100.0, 0.0).unit_kind(UnitKind::Percent),
        ]
    }

    /// Posterize's row: Levels 2..255, default 4; key `levels`.
    pub fn posterize(words: &AdjustWords) -> Vec<AdjustField> {
        vec![AdjustField::new("levels", words.levels.clone(), 2.0..=255.0, 4.0)]
    }
}

/// What an editor keeps between passes: the value its running gesture began with.
#[derive(Clone, Debug, Default)]
struct Memory {
    began: Option<Vec<f64>>,
}

/// A table of slider rows over the host's values, one per [`AdjustField`] (same index).
///
/// - Each row: caption, slider and number field; the colour track when the field has one and the context's dynamic
///   tracks are on.
/// - A reset of a row (double-click on the slider, the caption's reset) sets the field's default, one `Set`.
/// - Every value a commit or set of the table reports is rounded with [`AdjustField::round`], and the edit carries
///   every value of the table, so the host stores it as it is.
/// - Keyboard and AccessKit: each row as `SliderField` has them.
#[must_use = "show the table with `show`"]
#[derive(Pattern)]
pub struct AdjustSliders<'a> {
    label: Label,
    fields: &'a [AdjustField],
    values: Bind<'a, Vec<f64>>,
    context: AdjustContext,
}

impl<'a> AdjustSliders<'a> {
    /// The table named `label` of `fields` over `values` (same length).
    pub fn new(label: impl Into<Label>, fields: &'a [AdjustField], values: Bind<'a, Vec<f64>>) -> Self {
        Self { label: label.into(), fields, values, context: AdjustContext::dialog(true) }
    }

    /// Dialog or Properties panel; a dialog by default.
    pub fn context(mut self, context: AdjustContext) -> Self {
        self.context = context;
        self
    }

    /// Shows the rows and returns this pass's edit of the table.
    pub fn show(self, ui: &mut Ui) -> EditResponse<Vec<f64>> {
        let AdjustSliders { label, fields, mut values, context } = self;
        let mut edit = None;
        let shown = FieldGroup::new(label).show(ui, |ui, cx| {
            let held = read(&mut values, cx).unwrap_or_default();
            let now: Vec<f64> = fields.iter().enumerate().map(|(i, f)| f.clamp(held.get(i).copied().unwrap_or(f.default))).collect();
            let mut run = Run::start(&mut values, context, now.clone(), cx.memory::<Memory>().began.clone());
            for (i, field) in fields.iter().enumerate() {
                run.slider_row(ui, cx, field, now[i], move |table: &Vec<f64>, x| {
                    let mut table = table.clone();
                    table[i] = x;
                    table
                });
            }
            cx.memory::<Memory>().began = run.began();
            edit = Some(run.finish());
        });
        lifted(edit, shown.output)
    }
}

/// The value of `bind` a body starts its pass from; `None` while it is mixed.
pub(super) fn read<V: Value>(bind: &mut Bind<'_, V>, cx: &mut BodyCx<'_>) -> Option<V> {
    match bind.reborrow().read(cx) {
        Some(Mixed::Same(value)) => Some(value),
        _ => None,
    }
}

/// The edit a body collected, answered with the container's response. A body that did not run (a parent's measuring
/// run reuses what it measured) collected none: the response carries no edit.
pub(super) fn lifted<V>(edit: Option<EditResponse<V>>, response: Response) -> EditResponse<V> {
    match edit {
        Some(mut edit) => {
            edit.response = response;
            edit
        }
        None => EditResponse::lifted(response, EditEvents::new(), None),
    }
}

/// One pass over an editor's parts: the value they edit, and their edits lifted onto it and written to the editor's
/// binding.
pub(super) struct Run<'b, 'a, V: Value> {
    bind: &'b mut Bind<'a, V>,
    context: AdjustContext,
    now: V,
    began: Option<V>,
    events: EditEvents<V>,
    scaffold: Option<EditResponse<V>>,
    submitted: bool,
    escaped: bool,
    unparsed: bool,
}

impl<'b, 'a, V: Value> Run<'b, 'a, V> {
    /// A pass over `now`, with the value a running gesture began with.
    pub(super) fn start(bind: &'b mut Bind<'a, V>, context: AdjustContext, now: V, began: Option<V>) -> Self {
        Run { bind, context, now, began, events: EditEvents::new(), scaffold: None, submitted: false, escaped: false, unparsed: false }
    }

    /// The value a gesture that runs on began with, for the editor's memory.
    pub(super) fn began(&self) -> Option<V> {
        self.began.clone()
    }

    /// Lifts a part's `edit` onto the editor's value: the same gesture, with the value `make` builds of the value the
    /// gesture began with and the part's value.
    pub(super) fn push<T: Value>(&mut self, cx: &mut BodyCx<'_>, edit: EditResponse<T>, make: impl Fn(&V, &T) -> V) {
        self.submitted |= edit.submitted;
        self.escaped |= edit.escaped;
        self.unparsed |= edit.unparsed;
        for event in &edit.events {
            if let Some(event) = self.lift(event, &make) {
                let _ = self.bind.reborrow().write(cx, &event);
                self.events.push(event);
            }
        }
        if self.scaffold.is_none() {
            let now = self.now.clone();
            self.scaffold = Some(edit.map(move |_| now.clone()));
        }
    }

    /// A one-step change the editor makes itself (a Default button): `after` as one `Set` named `label`.
    pub(super) fn set(&mut self, cx: &mut BodyCx<'_>, label: &Label, after: V) {
        if after == self.now {
            return;
        }
        let event = EditEvent::set(Mixed::Same(self.now.clone()), after, Arc::from(label.as_str()), None);
        let _ = self.bind.reborrow().write(cx, &event);
        self.events.push(event);
    }

    fn lift<T>(&mut self, event: &EditEvent<T>, make: &impl Fn(&V, &T) -> V) -> Option<EditEvent<V>> {
        match event {
            EditEvent::Begin { id, label, doc_key, .. } => {
                self.began = Some(self.now.clone());
                Some(EditEvent::begin(*id, Mixed::Same(self.now.clone()), label.clone(), *doc_key))
            }
            EditEvent::Update { id, value, .. } => {
                let base = self.began.as_ref().unwrap_or(&self.now);
                Some(EditEvent::update(*id, make(base, value)))
            }
            EditEvent::Commit { id, after, .. } => {
                let base = self.began.take().unwrap_or_else(|| self.now.clone());
                let after = make(&base, after);
                Some(EditEvent::commit(*id, Mixed::Same(base), after))
            }
            EditEvent::Cancel { id, reason, .. } => Some(EditEvent::cancel(*id, *reason, self.began.take().map(Mixed::Same))),
            EditEvent::Set { after, label, doc_key, .. } => {
                Some(EditEvent::set(Mixed::Same(self.now.clone()), make(&self.now, after), label.clone(), *doc_key))
            }
            _ => None,
        }
    }

    /// One row: `field`'s slider showing `value`, its edit lifted onto the editor's value by `set` with
    /// the number rounded to the field's step.
    pub(super) fn slider_row(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>, field: &AdjustField, value: f64, set: impl Fn(&V, f64) -> V) {
        let context = self.context;
        let mut edit = field.slider(field.clamp(value), context).id_salt(field.key).show(ui);
        // A drag proposes the slider's own stops and the slider compares what it is shown with what it proposed, so
        // the running gesture carries them as they are; the value that ends the edit is rounded to the field's places.
        edit.events = std::mem::take(&mut edit.events)
            .into_iter()
            .map(|event| match event {
                EditEvent::Commit { .. } | EditEvent::Set { .. } => event.map(|x| field.round(x)),
                running => running,
            })
            .collect();
        self.push(cx, edit, |now, x| set(now, *x));
    }

    /// The pass's edit of the value. The response is the first part's until the caller puts the container's in
    /// (`lifted`).
    pub(super) fn finish(self) -> EditResponse<V> {
        let mut edit = self.scaffold.expect("an editor shows at least one part");
        let shown = match self.events.value() {
            Some(value) => value.clone(),
            None => self.now,
        };
        edit.shown = Some(Mixed::Same(Arc::new(shown)));
        edit.events = self.events;
        edit.submitted = self.submitted;
        edit.escaped = self.escaped;
        edit.unparsed = self.unparsed;
        edit
    }
}

/// The words of the installed language.
pub(super) fn words(ui: &Ui) -> Arc<AdjustWords> {
    Env::of(ui).words::<AdjustWords>()
}
