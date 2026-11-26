//! `CurveEditor`: a tone curve's graph with its points, the host's rows, the Input and Output fields of the selected
//! point, the Corner check box and a hint line.

use super::graph::{CurveChannel, CurveGraph, CurveGraphOutput, CurveSampler};
use super::model::{Curve, CurveRules};
use super::words::CurveWords;
use crate::ui::histogram::HistogramData;
use craft_behavior::{EditEvent, EditEvents, Mixed};
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, BodyCx, EditResponse, Outcome, Pattern, Props, Remember as _, Show as _, ShowContainer as _, Sizing};
use craft_ui::display::{Shade, Text};
use craft_ui::form::Field;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::runtime::env::Label;
use craft_ui::style::TypeRole;
use craft_ui::text::NumberField;
use egui::Ui;

type CurveRowsFn<'a> = Box<dyn FnMut(&mut Ui, &mut BodyCx<'_>) + 'a>;

/// The host's rows between the graph and the Input and Output fields (the Contour Editor's Name row).
pub struct CurveRows<'a>(CurveRowsFn<'a>);

impl<'a> CurveRows<'a> {
    /// The rows `rows` shows in the editor's column.
    pub fn new(rows: impl FnMut(&mut Ui, &mut BodyCx<'_>) + 'a) -> Self {
        CurveRows(Box::new(rows))
    }
}

/// What the editor reports: the curve's edit, from the graph or from a field, and the selected point after the pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct CurveEditorOutput {
    /// The edit of the curve.
    pub edit: EditResponse<Curve>,
    /// The selected point after the pass; the host that keeps the selection stores it.
    pub selected: Option<usize>,
}

impl Outcome for CurveEditorOutput {
    type Value = Curve;

    fn response(&self) -> &egui::Response {
        &self.edit.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}

impl std::ops::Deref for CurveEditorOutput {
    type Target = EditResponse<Curve>;

    fn deref(&self) -> &Self::Target {
        &self.edit
    }
}

/// What the editor keeps: the channel the selection belongs to, the selection when the host keeps none, and the field
/// gesture under way.
#[derive(Clone, Debug, Default)]
struct Memory {
    channel: Option<usize>,
    selected: Option<usize>,
    /// The curve a field gesture began with and the point it edits.
    field: Option<(Curve, usize)>,
}

/// Edits one tone curve: `CurveEditor::new("Curve", Bind::bound(&mut curve)).show(ui)` returns a
/// [`CurveEditorOutput`].
///
/// The graph is a square plot (`surface.sunken`), a quarter grid, the diagonal of the unchanged curve, the curve in the
/// active channel's colour, its points and the tone bars under it and beside it. Under the graph the host's rows
/// ([`between`](CurveEditor::between)) show, then the Input and Output fields of the selected point, the Corner check
/// box when [`corners`](CurveEditor::corners) is on, and the hint line. The graph keeps its geometry right to left; the
/// fields follow the reading order.
///
/// Points follow Photoshop's rules: whole levels `0..=levels`, at most [`max_points`](CurveEditor::max_points), a new
/// point within two levels of another's input is that point, the end points stay (they move in both axes), and a point
/// stays strictly between its neighbours' inputs.
///
/// Pointer: a press on a point selects and drags it; a press on the empty plot adds a point there and drags it, as one
/// gesture. Dragged more than the handle reach off the plot an interior point is removed, and comes back on re-entry
/// within the drag. A right click or a Cmd/Ctrl press on a point deletes it.
///
/// Keys while the graph has focus: `+` (or `=`) and `-` select the next and previous point (the first and last with
/// nothing selected; they stop at the ends); arrows move the selected point one level, ten with Shift; Delete and
/// Backspace remove it; Insert, and Cmd/Ctrl+Enter, add a point on the curve halfway between the selected point and the
/// next (across the widest gap with nothing selected) and select it; Esc cancels a drag.
///
/// Corners: the Corner check box makes the selected point a corner point. The curve bends at a corner point: each run
/// of points from one corner to the next is interpolated on its own. A new point is smooth. With corners on, smooth
/// points are round and corner points square.
///
/// The selection is view state: the editor keeps it, or the host sets it with [`selection`](CurveEditor::selection) and
/// stores the output's. It clears when the [`channel`](CurveEditor::channel) changes.
#[must_use = "show the editor with `show`"]
#[derive(Pattern, Props)]
pub struct CurveEditor<'a> {
    label: Label,
    curve: Bind<'a, Curve>,
    /// The dialog's channels; the curve is the channel's. Without channels the curve is neutral and unnamed.
    #[prop]
    channels: &'a [CurveChannel],
    /// The active channel, an index into the channels.
    #[prop]
    channel: usize,
    levels: u32,
    /// The most points the curve holds.
    #[prop(quantity = "points")]
    max_points: usize,
    /// The interpolation of the curve through its points.
    #[prop(func = "Fn(&[[f32; 2]]) -> Vec<f32>")]
    sampler: Option<CurveSampler<'a>>,
    /// Whether the tone bars show under the plot and beside it.
    #[prop]
    bars: bool,
    /// Whether the Input and Output fields show.
    #[prop]
    fields: bool,
    /// The fields and accessible values in percent of the levels.
    #[prop(flag)]
    percent: bool,
    /// A line under the editor.
    #[prop(into)]
    hint: Option<Label>,
    /// Pencil mode: a drag draws one output per input level.
    #[prop(flag)]
    pencil: bool,
    /// Points can be corner points.
    #[prop(flag)]
    corners: bool,
    /// The active channel's histogram, drawn behind the curve.
    #[prop]
    histogram: Option<&'a HistogramData>,
    /// The curve of every channel, in the order of the channels: the other channels' curves draw behind the active one.
    #[prop]
    overlays: &'a [Curve],
    /// The selected point, kept by the host: the editor shows it and reports changes in the output.
    #[prop]
    selection: Option<Option<usize>>,
    /// The host's rows between the graph and the fields.
    #[prop(func = "FnMut(&mut Ui, &mut BodyCx<'_>)")]
    between: Option<CurveRows<'a>>,
}

impl<'a> CurveEditor<'a> {
    /// An editor named `label` of `curve`.
    pub fn new(label: impl Into<Label>, curve: Bind<'a, Curve>) -> Self {
        CurveEditor {
            label: label.into(),
            curve,
            channels: &[],
            channel: 0,
            levels: 255,
            max_points: 16,
            sampler: None,
            bars: true,
            fields: true,
            percent: false,
            hint: None,
            pencil: false,
            corners: false,
            histogram: None,
            overlays: &[],
            selection: None,
            between: None,
        }
    }

    /// The top level of input and output.
    pub fn levels(mut self, levels: u32) -> Self {
        self.levels = levels.max(1);
        self
    }

    /// Shows the editor and returns this pass's edit.
    pub fn show(self, ui: &mut Ui) -> CurveEditorOutput {
        let CurveEditor {
            label,
            mut curve,
            channels,
            channel,
            levels,
            max_points,
            mut sampler,
            bars,
            fields,
            percent,
            hint,
            pencil,
            corners,
            histogram,
            overlays,
            selection: host_selection,
            mut between,
        } = self;
        let words = craft_ui::runtime::env::Env::of(ui).words::<CurveWords>();
        let rules = CurveRules { levels: levels as f32, max_points: max_points.max(2) };
        let mut out: Option<(EditResponse<Curve>, Option<usize>)> = None;
        let name = label.clone();
        let shown = Stack::new().label(label).show(ui, |ui, root| {
            let memory = root.memory::<Memory>();
            let cleared = memory.channel.is_some_and(|seen| seen != channel);
            if cleared {
                memory.selected = None;
            }
            memory.channel = Some(channel);
            let selected = match host_selection {
                Some(_) if cleared => None,
                Some(host) => host,
                None => memory.selected,
            };
            let mut graph = CurveGraph::new(name.clone(), curve.reborrowed())
                .channels(channels)
                .channel(channel)
                .levels(levels)
                .max_points(max_points)
                .bars(bars)
                .overlays(overlays);
            if let Some(data) = histogram {
                graph = graph.histogram(data);
            }
            if let Some(sampler) = sampler.take() {
                graph = graph.sampler(sampler);
            }
            if let Some(at) = selected {
                graph = graph.selection(at);
            }
            if pencil {
                graph = graph.pencil();
            }
            if corners {
                graph = graph.corners();
            }
            if percent {
                graph = graph.percent();
            }
            let CurveGraphOutput { mut edit, selected } = graph.show(ui);
            if let Some(rows) = between.as_mut() {
                (rows.0)(ui, root);
            }
            let now = match &edit.shown {
                Some(Mixed::Same(shown)) => Some((**shown).clone()),
                _ => None,
            };
            if let Some(now) = now.filter(|now| fields && !pencil && now.points().is_some()) {
                let at = selected.filter(|&i| now.points().is_some_and(|points| i < points.len()));
                let row = FieldsRow { words: &words, rules, now: &now, at, percent, corners };
                let lifted = row.show(ui, root, &mut curve);
                if edit.events.is_empty() {
                    edit.events = lifted;
                }
            }
            root.memory::<Memory>().selected = selected;
            if let Some(hint) = &hint {
                Text::new(hint.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
            }
            out = Some((edit, selected));
        });
        // A parent's measuring run reuses what it measured and does not run the body.
        let (edit, selected) = out.unwrap_or_else(|| (EditResponse::lifted(shown.output, EditEvents::new(), None), None));
        CurveEditorOutput { edit, selected }
    }
}

/// The Input and Output fields of the selected point `at` of `now`, and the Corner check box.
struct FieldsRow<'p> {
    words: &'p CurveWords,
    rules: CurveRules,
    now: &'p Curve,
    at: Option<usize>,
    percent: bool,
    corners: bool,
}

impl FieldsRow<'_> {
    /// A level as the fields write it: whole levels, or a whole percent of them.
    fn to_field(&self, level: f32) -> f64 {
        match self.percent {
            true => (f64::from(level) / f64::from(self.rules.levels) * 100.0).round(),
            false => f64::from(level),
        }
    }

    fn to_level(&self, value: f64) -> f32 {
        match self.percent {
            true => (value / 100.0 * f64::from(self.rules.levels)).round() as f32,
            false => value as f32,
        }
    }

    /// The range of the `axis` field. The input stays strictly between the neighbours' inputs, so the field never shows
    /// what the curve can't take.
    fn range(&self, axis: usize) -> std::ops::RangeInclusive<f64> {
        let points = self.now.points().unwrap_or_default();
        match (axis, self.at) {
            (0, Some(i)) => {
                let lo = i.checked_sub(1).map_or(0.0, |j| points[j][0] + 1.0);
                let hi = points.get(i + 1).map_or(self.rules.levels, |p| p[0] - 1.0);
                self.to_field(lo)..=self.to_field(hi.max(lo))
            }
            _ => 0.0..=self.to_field(self.rules.levels),
        }
    }

    /// Shows the fields and the check box, and writes what the user did with them into `curve` as the edits of the
    /// one curve. Returns the edit this pass made.
    fn show(&self, ui: &mut Ui, root: &mut BodyCx<'_>, curve: &mut Bind<'_, Curve>) -> EditEvents<Curve> {
        let points = self.now.points().unwrap_or_default();
        let mut reports: Vec<(usize, EditResponse<Option<f64>>)> = Vec::new();
        Row::new().show(ui, |ui, _| {
            for (axis, name) in [(0, self.words.input.clone()), (1, self.words.output.clone())] {
                let value = Some(self.to_field(self.at.map_or(0.0, |i| points[i][axis])));
                let mut field = NumberField::new(name.clone(), Bind::given(Mixed::Same(value)))
                    .range(self.range(axis))
                    .step(1.0)
                    .commit_decimals(0)
                    .enabled(self.at.is_some());
                if self.percent {
                    field = field.unit(self.words.percent_unit.clone());
                }
                let mut field = Some(field);
                let shown = Field::new(name).sizing(Sizing::Content).show(ui, |ui, _| field.take().map(|field| field.show(ui)));
                if let Some(Some(output)) = shown.inner {
                    reports.push((axis, output.edit));
                }
            }
        });
        let mut lifted = EditEvents::new();
        for (axis, report) in reports {
            let make = |base: &Curve, i: usize, value: f64| {
                let mut to = *base.points()?.get(i)?;
                to[axis] = self.to_level(value);
                self.rules.moved(base, i, to)
            };
            for event in &report.events {
                if let Some(event) = lift(event, self.now, root.memory::<Memory>(), self.at, make) {
                    curve.reborrow().write(root, &event);
                    lifted.push(event);
                }
            }
        }
        if self.corners {
            let mut on = self.at.is_some_and(|i| self.now.is_corner(i));
            let check = Checkbox::new(Bind::bound(&mut on)).label(self.words.corner.clone()).enabled(self.at.is_some()).show(ui);
            for event in &check.events {
                if let (EditEvent::Set { label, doc_key, .. }, Some(i)) = (event, self.at)
                    && let Some(after) = CurveRules::cornered(self.now, i, on)
                {
                    let event = EditEvent::set(Mixed::Same(self.now.clone()), after, label.clone(), *doc_key);
                    curve.reborrow().write(root, &event);
                    lifted.push(event);
                }
            }
        }
        lifted
    }
}

/// A field's event as the curve's: the same gesture, with the value `make` builds of the point `i` from the curve the
/// gesture began with. `None` when the event has no curve to carry (an empty field).
fn lift(
    event: &EditEvent<Option<f64>>,
    now: &Curve,
    memory: &mut Memory,
    at: Option<usize>,
    make: impl Fn(&Curve, usize, f64) -> Option<Curve>,
) -> Option<EditEvent<Curve>> {
    match event {
        EditEvent::Begin { id, label, doc_key, .. } => {
            memory.field = Some((now.clone(), at?));
            Some(EditEvent::begin(*id, Mixed::Same(now.clone()), label.clone(), *doc_key))
        }
        EditEvent::Update { id, .. } => {
            let (base, i) = memory.field.as_ref()?;
            let value = (*event.value()?)?;
            Some(EditEvent::update(*id, make(base, *i, value)?))
        }
        EditEvent::Commit { id, .. } => {
            let (base, i) = memory.field.take()?;
            let after = event.value().and_then(|value| make(&base, i, (*value)?)).unwrap_or_else(|| base.clone());
            Some(EditEvent::commit(*id, Mixed::Same(base), after))
        }
        EditEvent::Cancel { id, reason, .. } => {
            let began = memory.field.take().map(|(base, _)| Mixed::Same(base));
            Some(EditEvent::cancel(*id, *reason, began))
        }
        EditEvent::Set { label, doc_key, .. } => {
            let value = (*event.value()?)?;
            let after = make(now, at?, value)?;
            Some(EditEvent::set(Mixed::Same(now.clone()), after, label.clone(), *doc_key))
        }
        _ => None,
    }
}
