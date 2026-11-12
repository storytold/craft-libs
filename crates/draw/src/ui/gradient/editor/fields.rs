//! The fields under the bar: the selected marker's Color well, Opacity, Location, Midpoint and Delete, each an
//! edit of the editor's stops.

use std::ops::RangeInclusive;
use std::sync::Arc;

use craft_behavior::{EditEvent, GestureId, IconId, Mixed};
use craft_color::ui::{ColorPicker, ColorPopover, PickedColor, ScreenPickGesture};
use craft_theme::Srgba;
use craft_ui::buttons::IconButton;
use craft_ui::choice::Swatch;
use craft_ui::contract::{Bind, BodyCx, Props as _, Remember as _, Show as _, ShowContainer as _, clicked};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::runtime::env::{Label, UnitKind};
use craft_ui::text::NumberField;
use egui::{Id, Ui};

use crate::ui::gradient::stops::{self, StopRules, locations, midpoint};
use crate::ui::gradient::value::{GradientStops, StopRef, StopRow, StopRows};
use crate::ui::gradient::words::GradientWords;

/// The key of the id of the first number field, which Enter or Space on an opacity stop or a midpoint focuses.
pub(super) const FIRST_FIELD: &str = "first-field";

/// The setup closure of the popover's picker.
pub(super) type PickerSetup<'a> = Box<dyn for<'b> Fn(ColorPicker<'b>) -> ColorPicker<'b> + 'a>;

/// Which number field of the fields row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Number {
    Opacity,
    Location,
    Midpoint,
}

/// A number field of one stop: which field, in which row, of which stop.
#[derive(Clone, Copy)]
struct At {
    field: Number,
    row: StopRow,
    i: usize,
}

/// The Location field's run under way: the stops as it began and the stop's index in them. Every value of the run
/// moves that stop from there, as a drag edits the stops at its press, so a run that passes a neighbour and comes
/// back leaves the stops as they were.
#[derive(Clone, Debug, Default)]
struct LocationRun {
    run: Option<(Option<GestureId>, StopRow, usize)>,
    base: GradientStops,
}

/// Whether the colour popover is open.
#[derive(Clone, Debug, Default)]
struct PopoverOpen(bool);

/// What the fields row decides in a pass.
#[derive(Default)]
pub(super) struct Report {
    /// The edits of the stops, in the order made.
    pub events: Vec<EditEvent<GradientStops>>,
    /// The selection the fields moved.
    pub select: Option<Option<StopRef>>,
    /// The colour stop whose colour the host is asked to edit.
    pub color_requested: Option<usize>,
    pub screen_pick: Option<ScreenPickGesture>,
}

/// What the row reads from the editor.
pub(super) struct Row<'r, 'a> {
    pub words: &'r GradientWords,
    pub rules: &'r StopRules,
    pub location_decimals: usize,
    pub enabled: bool,
    pub fields: bool,
    pub color_popover: bool,
    pub picker: &'r Option<PickerSetup<'a>>,
    pub sampled: Option<Srgba>,
    /// The colour request of the bar, when the bar asked for one.
    pub requested: Option<usize>,
}

/// A 0..=1 value in percent, without the binary fraction an `f32` adds (0.3 is 30, not 30.0000012).
fn percent(v: f32) -> f64 {
    (f64::from(v) * 1e6).round() / 1e4
}

/// A label of `word`.
fn label(word: &str) -> Label {
    Label::from(word.to_owned())
}

/// What `field` of stop `i` of `row` shows, in percent.
fn field_value(field: Number, stops: &GradientStops, row: StopRow, i: usize) -> f64 {
    percent(match (field, row) {
        (Number::Opacity, StopRow::Color) => stops.color[i].color.a,
        (Number::Opacity, StopRow::Opacity) => stops.opacity_stops()[i].opacity,
        (Number::Location, _) => locations(stops, row)[i],
        (Number::Midpoint, _) => midpoint(stops, row, i),
    })
}

/// `stops` with `field` of stop `i` of `row` at `v` percent, and the stop's index after it.
fn set_field(field: Number, stops: &GradientStops, row: StopRow, i: usize, v: f64, rules: &StopRules) -> Option<(GradientStops, usize)> {
    let v = (v / 100.0) as f32;
    match (field, row) {
        (Number::Opacity, StopRow::Color) => {
            let mut out = stops.clone();
            out.color[i].color.a = v;
            Some((out, i))
        }
        (Number::Opacity, StopRow::Opacity) => {
            let mut out = GradientStops::with_opacity(stops.color.clone(), stops.opacity_stops().into_owned());
            out.opacity[i].opacity = v;
            Some((out, i))
        }
        (Number::Location, _) => stops::move_to(stops, row, i, v, rules),
        (Number::Midpoint, _) => stops::set_midpoint(stops, row, i, v, rules),
    }
}

/// Colour stop `i` of `stops` in `color`; a split gradient's colour stops keep their own alpha, which nothing shows.
pub(super) fn with_color(stops: &GradientStops, rows: StopRows, i: usize, color: Srgba) -> GradientStops {
    let mut out = stops.clone();
    let stop = &mut out.color[i].color;
    *stop = match rows {
        StopRows::Combined => color,
        StopRows::Split => Srgba { a: stop.a, ..color },
        _ => Srgba { a: 1.0, ..color },
    };
    out
}

/// Colour stop `i`'s colour as the well shows it: opaque unless the alpha is the colour's.
pub(super) fn stop_color(stops: &GradientStops, rows: StopRows, i: usize) -> Srgba {
    let color = stops.color[i].color;
    if rows == StopRows::Combined { color } else { Srgba { a: 1.0, ..color } }
}

/// One `Set` of the stops, replacing `before`.
fn set_event(before: &GradientStops, after: GradientStops, name: &str) -> EditEvent<GradientStops> {
    EditEvent::set(Mixed::Same(before.clone()), after, Arc::from(name), None)
}

/// A number field in percent named `name`, which its label scrubs.
fn number(
    ui: &mut Ui,
    id: Option<Id>,
    name: &str,
    value: f64,
    range: RangeInclusive<f64>,
    decimals: usize,
    enabled: bool,
) -> craft_ui::contract::EditResponse<Option<f64>> {
    let shown = Field::new(label(name)).show(ui, |ui, _| {
        let field = NumberField::new(label(name), Bind::given(Some(value)))
            .unit_kind(UnitKind::Percent)
            .range(range.clone())
            .step(1.0)
            .commit_decimals(decimals)
            .scrub_by_label()
            .enabled(enabled);
        match id {
            Some(id) => field.id(id),
            None => field,
        }
        .show(ui)
        .edit
    });
    shown.inner.unwrap_or_else(|| craft_ui::contract::EditResponse::lifted(shown.output, craft_behavior::EditEvents::new(), None))
}

impl Row<'_, '_> {
    /// The selected marker's fields under the bar, as the rows of PhotoCraft's panel, and the colour popover. `now` are
    /// the stops as the pass leaves them; each edit is drawn from them and reported in the order made.
    pub(super) fn show(
        &mut self,
        ui: &mut Ui,
        cx: &mut BodyCx<'_>,
        now: &GradientStops,
        selected: Option<StopRef>,
        anchor: &egui::Response,
    ) -> Report {
        let mut report = Report { color_requested: self.requested.filter(|_| !self.color_popover), ..Report::default() };
        let selected = selected.filter(|m| match *m {
            StopRef::Midpoint(row, i) => i + 1 < now.len(row),
            m => m.index() < now.len(m.row()),
        });
        let Some(marker) = selected else {
            cx.memory::<PopoverOpen>().0 = false;
            return report;
        };
        if self.fields {
            FieldGroup::new(label(self.words.stop_fields.as_str())).show(ui, |ui, _| self.rows(ui, cx, now, marker, &mut report));
        } else if let StopRef::Color(i) = marker {
            if self.requested.is_some() {
                cx.memory::<PopoverOpen>().0 = true;
            }
            self.popover(ui, cx, now, i, anchor, &mut report);
        }
        if !matches!(marker, StopRef::Color(_)) {
            cx.memory::<PopoverOpen>().0 = false;
        }
        report
    }

    fn rows(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>, now: &GradientStops, marker: StopRef, report: &mut Report) {
        let rows = self.rules.rows;
        match marker {
            StopRef::Color(i) => {
                let well = Field::new(label(self.words.color.as_str()))
                    .show(ui, |ui, _| Swatch::new(label(self.words.color.as_str()), stop_color(now, rows, i)).enabled(self.enabled).show(ui))
                    .inner;
                if let Some(well) = well {
                    if self.color_popover {
                        let open = &mut cx.memory::<PopoverOpen>().0;
                        if well.clicked {
                            *open = !*open;
                        } else if self.requested.is_some() {
                            *open = true;
                        }
                        self.popover(ui, cx, now, i, &well.response, report);
                    } else if well.clicked {
                        report.color_requested = Some(i);
                    }
                }
                if rows == StopRows::Combined {
                    self.number(ui, cx, now, At { field: Number::Opacity, row: StopRow::Color, i }, report);
                }
                self.number(ui, cx, now, At { field: Number::Location, row: StopRow::Color, i }, report);
                self.delete(ui, now, StopRow::Color, i, report);
            }
            StopRef::Opacity(i) => {
                self.number(ui, cx, now, At { field: Number::Opacity, row: StopRow::Opacity, i }, report);
                self.number(ui, cx, now, At { field: Number::Location, row: StopRow::Opacity, i }, report);
                self.delete(ui, now, StopRow::Opacity, i, report);
            }
            StopRef::Midpoint(row, i) => self.number(ui, cx, now, At { field: Number::Midpoint, row, i }, report),
        }
    }

    /// The range and decimals of `field`, in percent.
    fn range(&self, field: Number) -> (RangeInclusive<f64>, usize) {
        match field {
            Number::Opacity => (0.0..=100.0, 0),
            Number::Location => (0.0..=100.0, self.location_decimals),
            Number::Midpoint => (percent(*self.rules.midpoint.start())..=percent(*self.rules.midpoint.end()), 0),
        }
    }

    /// The number field `field` of stop `i` of `row`, and its edit mapped onto the stops.
    fn number(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>, now: &GradientStops, at: At, report: &mut Report) {
        let At { field, row, i } = at;
        let name = match field {
            Number::Opacity => self.words.opacity.as_str(),
            Number::Location => self.words.location.as_str(),
            Number::Midpoint => self.words.midpoint_field.as_str(),
        };
        let (range, decimals) = self.range(field);
        let id = (field != Number::Location && !(row == StopRow::Color && field == Number::Opacity)).then(|| cx.key(FIRST_FIELD));
        let shown = number(ui, id, name, field_value(field, now, row, i), range, decimals, self.enabled);
        let rules = self.rules.clone();
        for event in shown.events {
            if field != Number::Location {
                report.events.push(event.map(|v| v.and_then(|v| set_field(field, now, row, i, v, &rules)).map_or_else(|| now.clone(), |(s, _)| s)));
                continue;
            }
            // Each value of a run moves the stop from where the run began; the selection follows it past a neighbour.
            let run = cx.memory::<LocationRun>();
            let id = event.id();
            if run.run.map(|(id, ..)| id) != Some(id) {
                *run = LocationRun { run: Some((id, row, i)), base: now.clone() };
            }
            let (base, from) = (run.base.clone(), run.run.map_or(i, |(_, _, from)| from));
            if let Some(v) = event.value().copied().flatten()
                && let Some((_, j)) = stops::move_to(&base, row, from, (v / 100.0) as f32, &rules)
            {
                report.select = Some(Some(StopRef::stop(row, j)));
            }
            if event.phase().is_none_or(|phase| matches!(phase, craft_behavior::GesturePhase::Commit | craft_behavior::GesturePhase::Cancel)) {
                *cx.memory::<LocationRun>() = LocationRun::default();
            }
            report.events.push(
                event.map(|v| v.and_then(|v| stops::move_to(&base, row, from, (v / 100.0) as f32, &rules)).map_or_else(|| now.clone(), |(s, _)| s)),
            );
        }
    }

    /// The Delete button, disabled at the row's minimum.
    fn delete(&mut self, ui: &mut Ui, now: &GradientStops, row: StopRow, i: usize, report: &mut Report) {
        let enabled = self.enabled && now.len(row) > self.rules.min;
        let name = label(self.words.delete.as_str());
        let button = IconButton::new(IconId::new("trash")).label(name).enabled(enabled).show(ui);
        if clicked(&button)
            && let Some((out, j)) = stops::remove(now, row, i, self.rules)
        {
            report.select = Some(Some(StopRef::stop(row, j)));
            report.events.push(set_event(now, out, self.words.delete.as_str()));
        }
    }

    /// The colour popover of colour stop `i`, hung from `opener`, while it is open.
    fn popover(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>, now: &GradientStops, i: usize, opener: &egui::Response, report: &mut Report) {
        let rows = self.rules.rows;
        let host = self.picker.as_deref();
        let sampled = self.sampled;
        let mut open = cx.memory::<PopoverOpen>().0;
        let shown = ColorPopover::new(label(self.words.popover.as_str()), Bind::given(PickedColor::rgb(stop_color(now, rows, i))))
            .picker(move |picker| {
                let picker = picker.alpha(rows == StopRows::Combined);
                let picker = match sampled {
                    Some(color) => picker.sampled(color),
                    None => picker,
                };
                match host {
                    Some(host) => host(picker),
                    None => picker,
                }
            })
            .anchor(opener, Bind::bound(&mut open))
            .show(ui);
        cx.memory::<PopoverOpen>().0 = open && shown.open;
        report.screen_pick = shown.screen_pick;
        for event in shown.events {
            report.events.push(event.map(|picked| with_color(now, rows, i, picked.color)));
        }
    }
}
