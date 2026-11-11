//! [`StrokeEditor`]: the stroke editor's body, the one a popover or a docked panel hosts.

use std::borrow::Cow;
use std::ops::RangeInclusive;

use craft_behavior::{Axis, CancelReason, DisplayOrder, EditEvent, EditEvents, IconId, Input, ItemSource, Metrics, Mixed, Selection, Sense, Value};
use craft_theme::variant::Tone;
use craft_ui::buttons::Button;
use craft_ui::choice::{Checkbox, Segmented};
use craft_ui::contract::{
    Bind, Common, Component, Cx, Entry, MeasureCx, NoValue, Outcome, PaintUi, Props as _, Remember as _, Role, SelectMode, Show as _,
    ShowCollection as _, ShowContainer as _, clicked,
};
use craft_ui::display::{Divider, Shade, Text};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::{List, ListRow, ThumbAspect, ThumbSize};
use craft_ui::pickers::Select;
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::style::{ComponentStyle, StyleCx, SurfaceLook, TypeRole};
use craft_ui::text::NumberField;
use egui::{Response, Ui, Vec2};
use std::sync::Arc;

use super::{
    DashUnit, LineCap, LineJoin, LineStyle, StrokeAlign, StrokeChoiceRow, StrokeField, StrokeMixed, StrokePreview, StrokeSections, StrokeStyle,
    StrokeWords,
};

/// The width of a preset row's line sample as a multiple of its height.
const PRESET_ASPECT: (u16, u16) = (8, 1);
/// The tall sample's width as a multiple of its height, for the room it measures.
const SAMPLE_ASPECT: f32 = 6.0;

/// The preset rows, in list order.
const PRESETS: [LineStyle; 3] = [LineStyle::Solid, LineStyle::Dashed, LineStyle::Dotted];

/// What the stroke editor reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the editor's event"]
pub struct StrokeEditorOutput {
    /// The editor's whole area.
    pub response: Response,
    /// The valid changes, in order, each the whole new style: a preset row, a select, a segmented button, the check,
    /// Add pair and Remove pair are `Set`s; a number field's typed value is a `Set` and its scrub a gesture (Escape
    /// cancels). A style that fails [`StrokeStyle::validate`] is never reported. A binding that writes through has
    /// them already.
    pub events: EditEvents<StrokeStyle>,
    /// The control the events came from, so the host coalesces one field's edits into one undo step.
    pub field: Option<StrokeField>,
    /// A pick in a [`StrokeChoiceRow`]: the row's id and the choice's id.
    pub choice: Option<(String, String)>,
}

impl Outcome for StrokeEditorOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The stroke editor's body (PhotoCraft's Stroke Options popover body; VectorCraft's and DesignCraft's stroke panels
/// with [`StrokeSections::PANEL`]). It edits the host's [`StrokeStyle`] and reports every valid change at once; there
/// is no OK button (lead call 21).
///
/// - Presets ([`StrokeSections::PRESETS`]): Solid, Dashed (`[4, 2]`), Dotted (`[0, 2]`, round caps) in a list, each
///   with a line sample drawn at opacity 1; the row matching [`StrokeStyle::line_style`] is selected, none for Custom.
///   Picking one applies [`StrokeStyle::with_line_style`].
/// - Rows: Alignment (Inside, Center, Outside; enabled only when [`closed`](Self::closed)); Caps (Butt / Flat, Round,
///   Square; disabled for a solid closed path); Corners (Miter, Round, Bevel; enabled only with
///   [`corners`](Self::corners)); the app's [`rows`](Self::rows); Miter limit (1–500, `×`; enabled only for Miter
///   corners); Opacity ([`StrokeSections::OPACITY`], 0–100 %, stored 0..=1). Choices are selects, or segmented icon
///   buttons with [`StrokeSections::ICONS`]. Under the rows of an open path: "Open paths use centered strokes".
/// - Dashes ([`StrokeSections::DASHES`]): a "Dashed line" check (setting it applies the Dashed preset, clearing it
///   Solid); while dashed, a hint by [`StrokeStyle::dash_unit`], then one row per pair of Dash and Gap fields (0–10000,
///   `×`, or `pt` for document units); "Add pair" (enabled under 32 entries: appends `[4, 2]`, or `2` to complete an
///   odd pattern) and "Remove pair" (enabled over 2 entries: drops the last two); "Dash offset" (±1,000,000).
/// - Sample ([`StrokeSections::SAMPLE`]): a tall [`StrokePreview`], a rectangle when closed, a line otherwise. While
///   the style fails [`StrokeStyle::validate`], the invalid word shows under it and nothing is reported.
/// - [`mixed`](Self::mixed): a mixed field shows the mixed state and lights no button or preset; the first edit sets it
///   for all.
///
/// ```ignore
/// let out = StrokeEditor::new("Stroke", Bind::bound(&mut style)).sections(StrokeSections::PANEL).show(ui);
/// ```
#[must_use = "show the editor with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct StrokeEditor<'a> {
    label: Label,
    value: Bind<'a, StrokeStyle>,
    sections: StrokeSections,
    closed: bool,
    corners: bool,
    mixed: StrokeMixed,
    rows: &'a [StrokeChoiceRow<'a>],
}

impl<'a> StrokeEditor<'a> {
    /// The editor named `label` over `value`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, StrokeStyle>) -> Self {
        Self {
            label: label.into(),
            value,
            sections: StrokeSections::default(),
            closed: true,
            corners: true,
            mixed: StrokeMixed::default(),
            rows: &[],
        }
    }

    /// Which parts show; [`StrokeSections::POPOVER`] by default.
    pub fn sections(mut self, sections: StrokeSections) -> Self {
        self.sections = sections;
        self
    }

    /// Whether the stroked path is closed: alignment applies, caps need dashes, and the tall sample is a rectangle.
    /// True by default.
    pub fn closed(mut self, closed: bool) -> Self {
        self.closed = closed;
        self
    }

    /// Whether the path has corners for the Corners and Miter limit rows (an ellipse has none). True by default.
    pub fn corners(mut self, corners: bool) -> Self {
        self.corners = corners;
        self
    }

    /// The fields that differ across the host's selection.
    pub fn mixed(mut self, mixed: &StrokeMixed) -> Self {
        self.mixed = *mixed;
        self
    }

    /// The app's choice rows, after the Corners row.
    pub fn rows(mut self, rows: &'a [StrokeChoiceRow<'a>]) -> Self {
        self.rows = rows;
        self
    }

    /// Shows the editor.
    pub fn show(self, ui: &mut Ui) -> StrokeEditorOutput {
        let StrokeEditor { label, mut value, sections, closed, corners, mixed, rows } = self;
        let env = Env::of(ui);
        let words = env.words::<StrokeWords>();
        let name: Arc<str> = label.as_str().into();
        let mut found = Found::default();
        let shown = Stack::new().label(label).show(ui, |ui, cx| {
            found = Found::default();
            let held = match value.reborrow().read(cx) {
                Some(Mixed::Same(held)) => held,
                _ => StrokeStyle::default(),
            };
            let kept = cx.memory::<Pending>().clone();
            let style = match kept.0 {
                Some((base, draft)) if base == held => draft,
                _ => held.clone(),
            };
            let body = Body { words: &words, style: &style, sections, closed, corners, mixed, rows, name: name.clone() };
            body.show(ui, &mut found);
            if !found.events.is_empty() {
                for event in &found.events {
                    let _ = value.reborrow().write(cx, event);
                }
                *cx.memory::<Pending>() = Pending(None);
            } else if let Some(draft) = found.invalid.take() {
                *cx.memory::<Pending>() = Pending(Some((held, draft)));
            }
        });
        StrokeEditorOutput { response: shown.output, events: found.events, field: found.field, choice: found.choice }
    }
}

/// The invalid edit the user last made on `base`, kept until the host's value or a valid edit replaces it: the fields
/// show it and the message names it.
#[derive(Clone, Debug, Default, PartialEq)]
struct Pending(Option<(StrokeStyle, StrokeStyle)>);

/// What one pass found: the events of the first control that reported, the field they came from, a choice pick and the
/// last invalid edit.
#[derive(Default)]
struct Found {
    events: EditEvents<StrokeStyle>,
    field: Option<StrokeField>,
    choice: Option<(String, String)>,
    invalid: Option<StrokeStyle>,
}

impl Found {
    /// Takes the events of the first control that reports in the pass; a field's gesture is the only thing that
    /// reports more than once.
    fn push(&mut self, field: StrokeField, events: impl Into<EditEvents<StrokeStyle>>) {
        for event in events.into() {
            if self.field.is_some_and(|first| first != field) {
                return;
            }
            if let Some(style) = event.value().filter(|style| style.validate().is_err()) {
                self.invalid = Some(style.clone());
            }
            if let Some(event) = checked(event) {
                self.events.push(event);
                self.field = Some(field);
            }
        }
    }
}

/// The event as the editor reports it: a style that fails [`StrokeStyle::validate`] is never reported, so a gesture
/// whose value turns invalid ends in a cancel.
fn checked(event: EditEvent<StrokeStyle>) -> Option<EditEvent<StrokeStyle>> {
    let valid = |style: &StrokeStyle| style.validate().is_ok();
    match event {
        EditEvent::Update { ref value, .. } if !valid(value) => None,
        EditEvent::Set { ref after, .. } if !valid(after) => None,
        EditEvent::Commit { id, before, ref after, .. } if !valid(after) => Some(EditEvent::cancel(id, CancelReason::Refused, Some(before))),
        event => Some(event),
    }
}

/// The preset rows as the list's item source: the position is the key.
struct PresetRows;

impl DisplayOrder<usize> for PresetRows {
    fn len(&self) -> usize {
        PRESETS.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn text(&self, _at: usize) -> Option<Cow<'_, str>> {
        None
    }
}

impl ItemSource<usize> for PresetRows {}

/// One pass of the editor: what it draws from.
struct Body<'a> {
    words: &'a StrokeWords,
    /// The host's value, or the pending invalid edit made on it.
    style: &'a StrokeStyle,
    sections: StrokeSections,
    closed: bool,
    corners: bool,
    mixed: StrokeMixed,
    rows: &'a [StrokeChoiceRow<'a>],
    /// The name of the edits, for the host's undo.
    name: Arc<str>,
}

impl Body<'_> {
    /// Reports the one-step change to `next`.
    fn set(&self, found: &mut Found, field: StrokeField, next: StrokeStyle) {
        let event = EditEvent::set(Mixed::Same(self.style.clone()), next, self.name.clone(), None);
        found.push(field, event);
    }

    fn show(&self, ui: &mut Ui, found: &mut Found) {
        if self.sections.contains(StrokeSections::PRESETS) {
            self.presets(ui, found);
            Divider::horizontal().show(ui);
        }
        self.grid(ui, found);
        if !self.closed {
            Text::new(self.words.open_paths.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
        }
        if self.sections.contains(StrokeSections::DASHES) {
            Divider::horizontal().show(ui);
            self.dashes(ui, found);
        }
        if self.sections.contains(StrokeSections::SAMPLE) {
            Sample { common: Common::new(), value: self.style, closed: self.closed }.label(self.words.sample.clone()).show(ui);
        }
        if found.invalid.is_some() || self.style.validate().is_err() {
            Text::new(self.words.invalid.clone()).role(TypeRole::Caption).tone(Tone::Bad).show(ui);
        }
    }

    fn preset_name(&self, preset: LineStyle) -> Label {
        match preset {
            LineStyle::Solid => self.words.solid.clone(),
            LineStyle::Dashed => self.words.dashed.clone(),
            LineStyle::Dotted => self.words.dotted.clone(),
            LineStyle::Custom => self.words.custom.clone(),
        }
    }

    fn presets(&self, ui: &mut Ui, found: &mut Found) {
        let current = if self.mixed.dashes { None } else { PRESETS.iter().position(|preset| *preset == self.style.line_style()) };
        // The rows show their pattern whatever the opacity is.
        let samples: Vec<StrokeStyle> = PRESETS
            .iter()
            .map(|preset| {
                let mut sample = self.style.with_line_style(*preset);
                sample.opacity = 1.0;
                sample
            })
            .collect();
        let mut selection = current.map_or_else(Selection::default, Selection::single);
        List::new(self.words.title.clone())
            .id_salt("presets")
            .select(SelectMode::Single)
            .natural_height(true)
            .selection(Bind::bound(&mut selection))
            .show_items(ui, &PresetRows, &mut |at, _| {
                let sample = &samples[*at];
                ListRow::new(self.preset_name(PRESETS[*at])).thumb_aspect(
                    ThumbSize::Small,
                    ThumbAspect::new(PRESET_ASPECT.0, PRESET_ASPECT.1),
                    move |cx, rect| StrokePreview::sized(sample, rect).paint(cx, rect),
                )
            });
        let picked = selection.items.first().copied().filter(|at| Some(*at) != current);
        if let Some(at) = picked {
            self.set(found, StrokeField::Preset, self.style.with_line_style(PRESETS[at]));
        }
    }

    fn grid(&self, ui: &mut Ui, found: &mut Found) {
        let (style, mixed, words) = (self.style, self.mixed, self.words);
        let solid_closed = self.closed && style.dashes.is_empty() && !mixed.dashes;
        let icons = self.sections.contains(StrokeSections::ICONS);
        FieldGroup::new(words.title.clone()).show(ui, |ui, _| {
            let align = [
                (StrokeAlign::Inside, "stroke_align_inside", &words.inside),
                (StrokeAlign::Center, "stroke_align_center", &words.center),
                (StrokeAlign::Outside, "stroke_align_outside", &words.outside),
            ];
            let cap = [
                (LineCap::Butt, "stroke_cap_butt", &words.butt),
                (LineCap::Round, "stroke_cap_round", &words.round_cap),
                (LineCap::Square, "stroke_cap_square", &words.square),
            ];
            let join = [
                (LineJoin::Miter, "stroke_join_miter", &words.miter),
                (LineJoin::Round, "stroke_join_round", &words.round_join),
                (LineJoin::Bevel, "stroke_join_bevel", &words.bevel),
            ];
            let event = choice(ui, &words.alignment, "align", held(mixed.align, style.align), &align, self.closed, icons);
            found.push(StrokeField::Align, event.map(|align| StrokeStyle { align, ..style.clone() }));
            let event = choice(ui, &words.caps, "cap", held(mixed.cap, style.cap), &cap, !solid_closed, icons);
            found.push(StrokeField::Cap, event.map(|cap| StrokeStyle { cap, ..style.clone() }));
            let event = choice(ui, &words.corners, "join", held(mixed.join, style.join), &join, self.corners, icons);
            found.push(StrokeField::Join, event.map(|join| StrokeStyle { join, ..style.clone() }));
            self.app_rows(ui, found);
            let miter = self.corners && (mixed.join || style.join == LineJoin::Miter);
            let event = number(
                ui,
                &words.miter_limit,
                "miter-limit",
                f64::from(style.miter_limit),
                mixed.miter_limit,
                wide(StrokeStyle::MITER_LIMIT_RANGE),
                UnitKind::Times,
                miter,
            );
            found.push(StrokeField::MiterLimit, event.map(|v| StrokeStyle { miter_limit: v as f32, ..style.clone() }));
            if self.sections.contains(StrokeSections::OPACITY) {
                let event =
                    number(ui, &words.opacity, "opacity", f64::from(style.opacity) * 100.0, mixed.opacity, 0.0..=100.0, UnitKind::Percent, true);
                found.push(StrokeField::Opacity, event.map(|v| StrokeStyle { opacity: v as f32 / 100.0, ..style.clone() }));
            }
        });
    }

    /// The app's rows: each a select of the row's choices with painted previews.
    fn app_rows(&self, ui: &mut Ui, found: &mut Found) {
        for row in self.rows {
            let current = match row.selected {
                Some(Mixed::Same(id)) => Mixed::Same(id.to_owned()),
                Some(Mixed::Mixed) => Mixed::Mixed,
                None => Mixed::Same(String::new()),
            };
            let entries: Vec<Entry<String>> = row.choices.iter().map(|choice| Entry::new(choice.id.clone(), choice.label.clone())).collect();
            let mut picked = None;
            Field::new(row.label.clone()).id_salt(("row", row.id.as_str())).show(ui, |ui, _| {
                let mut select = Select::new(row.label.clone(), Bind::given(current.clone()), &entries);
                if let Some(painter) = &row.paint {
                    select = select.decorate(move |id: &String, entry| match row.choices.iter().find(|choice| &choice.id == id) {
                        Some(choice) => entry.thumb(ThumbSize::Small, move |cx, rect| painter.paint(cx, rect, choice)),
                        None => entry,
                    });
                }
                picked = select.show(ui).value().cloned();
            });
            if let Some(id) = picked
                && found.choice.is_none()
            {
                found.choice = Some((row.id.clone(), id));
            }
        }
    }

    fn dash_unit(&self) -> UnitKind {
        match self.style.dash_unit {
            DashUnit::Widths => UnitKind::Times,
            DashUnit::Document => UnitKind::Points,
        }
    }

    fn dashes(&self, ui: &mut Ui, found: &mut Found) {
        let (style, words) = (self.style, self.words);
        let state = if self.mixed.dashes { Mixed::Mixed } else { Mixed::Same(!style.dashes.is_empty()) };
        if let Some(on) = Checkbox::new(Bind::given(state)).label(words.dashed_line.clone()).id_salt("dashed").show(ui).value().cloned() {
            let preset = if on { LineStyle::Dashed } else { LineStyle::Solid };
            self.set(found, StrokeField::Dashed, style.with_line_style(preset));
        }
        if self.mixed.dashes || style.dashes.is_empty() {
            return;
        }
        let hint = match style.dash_unit {
            DashUnit::Widths => &words.dash_widths,
            DashUnit::Document => &words.dash_document,
        };
        Text::new(hint.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
        for (pair, lengths) in style.dashes.chunks(2).enumerate() {
            Row::new().id_salt(("pair", pair)).show(ui, |ui, _| {
                for (k, &length) in lengths.iter().enumerate() {
                    let (index, name) = (pair * 2 + k, if k == 0 { &words.dash } else { &words.gap });
                    Text::new(name.clone()).shade(Shade::Muted).id_salt(("name", index)).show(ui);
                    let bind = Bind::<Option<f64>>::given(Some(f64::from(length)));
                    let out = NumberField::new(name.clone(), bind)
                        .id_salt(("dash", index))
                        .range(wide(StrokeStyle::DASH_RANGE))
                        .unit_kind(self.dash_unit())
                        .show(ui);
                    let with = |v: Option<f64>| {
                        let mut next = style.clone();
                        next.dashes[index] = v.map_or(length, |v| v as f32);
                        next
                    };
                    found.push(StrokeField::Dash(index), out.edit.events.map(with));
                }
            });
        }
        Row::new().id_salt("pair-buttons").show(ui, |ui, _| {
            let add = Button::new(words.add_pair.clone()).id_salt("add-pair").enabled(style.dashes.len() < StrokeStyle::MAX_DASHES).show(ui);
            if clicked(&add) {
                let mut next = style.clone();
                if next.dashes.len() % 2 == 1 {
                    next.dashes.push(StrokeStyle::DASHED[1]);
                } else {
                    next.dashes.extend(StrokeStyle::DASHED);
                }
                self.set(found, StrokeField::AddPair, next);
            }
            let remove = Button::new(words.remove_pair.clone()).id_salt("remove-pair").enabled(style.dashes.len() > 2).show(ui);
            if clicked(&remove) {
                let mut next = style.clone();
                next.dashes.truncate(style.dashes.len().saturating_sub(2));
                self.set(found, StrokeField::RemovePair, next);
            }
        });
        FieldGroup::new(words.dash_offset.clone()).id_salt("offset").show(ui, |ui, _| {
            let range = -f64::from(StrokeStyle::DASH_OFFSET_MAX)..=f64::from(StrokeStyle::DASH_OFFSET_MAX);
            let event =
                number(ui, &words.dash_offset, "dash-offset", f64::from(style.dash_offset), self.mixed.dash_offset, range, self.dash_unit(), true);
            found.push(StrokeField::DashOffset, event.map(|v| StrokeStyle { dash_offset: v as f32, ..style.clone() }));
        });
    }
}

/// `value` as a field's binding value: mixed while the field is.
fn held<T: Value>(mixed: bool, value: T) -> Mixed<T> {
    if mixed { Mixed::Mixed } else { Mixed::Same(value) }
}

fn wide(range: RangeInclusive<f32>) -> RangeInclusive<f64> {
    f64::from(*range.start())..=f64::from(*range.end())
}

/// A row of choices as a select, or as a segmented set with icons; the event of a pick. A mixed field lights nothing.
fn choice<T: Value + 'static>(
    ui: &mut Ui,
    label: &Label,
    salt: &'static str,
    current: Mixed<T>,
    items: &[(T, &'static str, &Label)],
    enabled: bool,
    icons: bool,
) -> EditEvents<T> {
    let entries: Vec<Entry<T>> = items
        .iter()
        .map(|(item, icon, name)| {
            let entry = Entry::new(item.clone(), (*name).clone());
            if icons { entry.icon(IconSpec::svg(IconId::new(icon))) } else { entry }
        })
        .collect();
    let mut event = EditEvents::new();
    Field::new(label.clone()).id_salt(salt).enabled(enabled).show(ui, |ui, _| {
        let bind = Bind::given(current.clone());
        event = if icons {
            Segmented::new(label.clone(), bind, &entries).show(ui).events
        } else {
            Select::new(label.clone(), bind, &entries).show(ui).events
        };
    });
    event
}

/// A number row; the event of an edit, in the field's own units. A mixed field shows the mixed state.
#[expect(clippy::too_many_arguments, reason = "one call per field")]
fn number(
    ui: &mut Ui,
    label: &Label,
    salt: &'static str,
    value: f64,
    mixed: bool,
    range: RangeInclusive<f64>,
    kind: UnitKind,
    enabled: bool,
) -> EditEvents<f64> {
    let mut event = EditEvents::new();
    Field::new(label.clone()).id_salt(salt).enabled(enabled).show(ui, |ui, _| {
        let bind = if mixed { Bind::<Option<f64>>::given(Mixed::Mixed) } else { Bind::<Option<f64>>::given(Some(value)) };
        let out = NumberField::new(label.clone(), bind).id_salt("field").range(range.clone()).unit_kind(kind).show(ui);
        event = out.edit.events.map(|v| v.unwrap_or(value));
    });
    event
}

/// The style of the tall sample: the field well and its height.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
struct SampleStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
    /// The sample's height.
    height: f32,
}

/// The tall line sample under the rows: a [`StrokePreview`] on a sunken well, a rectangle when closed, a line
/// otherwise. It is the stroke document art the editor shows, so it paints with the content painter.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(content_painter, unregistered)]
struct Sample<'a> {
    #[common]
    common: Common,
    value: &'a StrokeStyle,
    closed: bool,
}

impl Component for Sample<'_> {
    const ROLE: Role = Role::IMAGE;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const INPUT: Input = Input::Display;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = SampleStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: SampleStyleKey) -> SampleStyle {
        SampleStyle { surface: cx.field(), metrics: Metrics::new(), height: cx.tokens.size.thumb.lg }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn fills(&self) -> Option<Axis> {
        Some(Axis::X)
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(cx.style.height * SAMPLE_ASPECT, cx.style.height)
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let painter = ui.content_painter(cx);
        StrokePreview::sized(self.value, cx.content_rect).closed(self.closed).paint(&painter, cx.content_rect);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}
