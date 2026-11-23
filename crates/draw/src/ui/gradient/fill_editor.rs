//! [`GradientFillEditor`]: the gradient fill rows of a shape or a Gradient Fill layer.

use std::sync::Arc;

use craft_behavior::{EditEvent, EditEvents, Mixed};
use craft_color_ui::ui::ScreenPickGesture;
use craft_color_ui::ui::gradient::{GradientEditor, GradientStops, StopColor, StopRef};
use craft_theme::Srgba;
use craft_ui::buttons::Button;
use craft_ui::choice::{Checkbox, Segmented};
use craft_ui::contract::{Bind, Entry, Outcome, Props as _, Show as _, ShowContainer as _, clicked};
use craft_ui::display::Text;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::patterns::presets::PresetState;
use craft_ui::runtime::env::{Env, Label, UnitKind};
use craft_ui::text::NumberField;
use craft_ui::value_drag::AngleDial;
use egui::{Response, Ui};

use super::palette::stops_of;
use super::{GradientFill, GradientGroup, GradientPalette, GradientRef, GradientStyle};

/// The range of the Angle rows, in degrees.
const ANGLE_RANGE: std::ops::RangeInclusive<f64> = -180.0..=180.0;
/// The range of the Scale field, in percent (Photoshop's).
const SCALE_RANGE: std::ops::RangeInclusive<f64> = 10.0..=150.0;

craft_ui::words! {
    /// The words of the gradient fill rows.
    pub struct GradientFillWords {
        linear: "gradientFill.linear" = "Linear";
        radial: "gradientFill.radial" = "Radial";
        angle_style: "gradientFill.angleStyle" = "Angle";
        reflected: "gradientFill.reflected" = "Reflected";
        diamond: "gradientFill.diamond" = "Diamond";
        fill: "gradientFill.fill" = "Gradient Fill";
        style: "gradientFill.style" = "Style";
        angle: "gradientFill.angle" = "Angle";
        scale: "gradientFill.scale" = "Scale";
        reverse: "gradientFill.reverse" = "Reverse";
        dither: "gradientFill.dither" = "Dither";
        align: "gradientFill.align" = "Align with layer";
        reset_alignment: "gradientFill.resetAlignment" = "Reset Alignment";
        options: "gradientFill.options" = "Gradient Options";
    }
}

/// The shape gradient tab and the fill layer's Gradient Fill rows: the palette, the stop editor, the
/// style, angle, scale, reverse, dither and align rows, and Reset Alignment. Each child's events
/// become events of the whole value; the editor keeps no state between passes, so a second pass in
/// one frame applies only the input of its own pass.
#[must_use = "show the editor with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct GradientFillEditor<'a> {
    label: Label,
    value: Bind<'a, GradientFill>,
    groups: &'a [GradientGroup],
    state: Bind<'a, PresetState<GradientRef>>,
    recent: &'a [GradientStops],
    sampled: Option<Srgba>,
}

/// What [`GradientFillEditor::show`] returns.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host applies the editor's edits"]
pub struct GradientFillEditorOutput {
    /// The editor.
    pub response: Response,
    /// The edits of the whole value.
    pub events: EditEvents<GradientFill>,
    /// A preset picked in the palette.
    pub picked: Option<GradientRef>,
    /// The colour stop whose colour the host is asked to edit.
    pub color_requested: Option<StopColor>,
    /// The popover picker's screen-pick press; the host answers with `sampled`. The pick is for the selected colour
    /// stop ([`selected`](Self::selected)).
    pub screen_pick: Option<ScreenPickGesture>,
    /// The stops editor's selection after this pass; `None` when the body did not run.
    pub selected: Option<StopRef>,
}

impl Outcome for GradientFillEditorOutput {
    type Value = GradientFill;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// `events` of one field as events of the whole fill: each carried value is `base` with the field set by `set`.
fn lift<T>(events: EditEvents<T>, base: &GradientFill, set: impl Fn(&mut GradientFill, T)) -> EditEvents<GradientFill> {
    events.map(|field| {
        let mut fill = base.clone();
        set(&mut fill, field);
        fill
    })
}

/// Adds `lifted` to the pass's `events`.
fn absorb(events: &mut EditEvents<GradientFill>, lifted: EditEvents<GradientFill>) {
    for event in lifted.iter() {
        events.push(event.clone());
    }
}

/// `stops` with every midpoint centred, as a preset gives them.
fn centred(stops: &GradientStops) -> GradientStops {
    let mut stops = stops.clone();
    for stop in &mut stops.color {
        stop.midpoint = 0.5;
    }
    for stop in &mut stops.opacity {
        stop.midpoint = 0.5;
    }
    stops
}

impl<'a> GradientFillEditor<'a> {
    /// The editor of `value`, offering the preset `groups` with their palette `state`.
    pub fn new(
        label: impl Into<Label>,
        value: Bind<'a, GradientFill>,
        groups: &'a [GradientGroup],
        state: Bind<'a, PresetState<GradientRef>>,
    ) -> Self {
        GradientFillEditor { label: label.into(), value, groups, state, recent: &[], sampled: None }
    }

    /// The recently used gradients, shown first in the palette.
    pub fn recent(mut self, recent: &'a [GradientStops]) -> Self {
        self.recent = recent;
        self
    }

    /// The colour a screen pick sampled.
    pub fn sampled(mut self, sampled: Option<Srgba>) -> Self {
        self.sampled = sampled;
        self
    }

    /// Shows the editor.
    pub fn show(self, ui: &mut Ui) -> GradientFillEditorOutput {
        let GradientFillEditor { label, mut value, groups, mut state, recent, sampled } = self;
        let env = Env::of(ui);
        let words = env.words::<GradientFillWords>();
        let label_text: Arc<str> = Arc::from(words.fill.as_str());
        let shown = Stack::new().fill_width().show(ui, |ui, cx| {
            let mut events = EditEvents::<GradientFill>::new();
            let mut picked = None;
            let mut color_requested = None;
            let mut screen_pick = None;
            let Some(now) = value.reborrow().read(cx).and_then(|held| held.same().cloned()) else {
                return (events, picked, color_requested, screen_pick, None);
            };
            if !groups.is_empty() || !recent.is_empty() {
                let palette = GradientPalette::new(label.clone(), groups, state.reborrowed()).recent(recent).show(ui);
                picked = palette.picked;
                if let Some(key) = picked
                    && let Some(stops) = stops_of(groups, recent, key)
                {
                    let mut next = now.clone();
                    next.stops = if matches!(key, GradientRef::Recent(_)) { stops.clone() } else { centred(stops) };
                    if next != now {
                        events.push(EditEvent::set(Mixed::Same(now.clone()), next, label_text.clone(), None));
                    }
                }
            }
            let mut editor = GradientEditor::new(label.clone(), Bind::given(now.stops.clone()));
            if let Some(color) = sampled {
                editor = editor.sampled(color);
            }
            let stops = editor.show(ui);
            color_requested = stops.color_requested;
            screen_pick = stops.screen_pick;
            let selected = stops.selected;
            absorb(&mut events, lift(stops.edit.events, &now, |fill, stops| fill.stops = stops));

            Text::new(words.options.clone()).show(ui);
            Row::new().show(ui, |ui, _| {
                Text::new(words.style.clone()).show(ui);
                let entries: Vec<Entry<GradientStyle>> =
                    GradientStyle::ALL.iter().map(|style| Entry::new(*style, style.label(&words)).icon(style.icon())).collect();
                let pick = Segmented::new(words.style.clone(), Bind::given(now.style), &entries).show(ui);
                absorb(&mut events, lift(pick.events, &now, |fill, style| fill.style = style));
            });
            Row::new().show(ui, |ui, _| {
                Text::new(words.angle.clone()).show(ui);
                let dial = AngleDial::new(Bind::given(now.angle)).label(words.angle.clone()).range(ANGLE_RANGE).show(ui);
                absorb(&mut events, lift(dial.events, &now, |fill, angle| fill.angle = angle));
                let field =
                    NumberField::new(words.angle.clone(), Bind::given(Some(now.angle))).unit_kind(UnitKind::Degrees).range(ANGLE_RANGE).show(ui);
                absorb(&mut events, lift(field.edit.events.filter_map(|angle| angle), &now, |fill, angle| fill.angle = angle));
            });
            Row::new().show(ui, |ui, _| {
                Text::new(words.scale.clone()).show(ui);
                let field = NumberField::new(words.scale.clone(), Bind::given(Some(now.scale * 100.0))).range(SCALE_RANGE).unit("%").show(ui);
                absorb(&mut events, lift(field.edit.events.filter_map(|percent| percent), &now, |fill, percent| fill.scale = percent / 100.0));
            });
            Row::new().show(ui, |ui, _| {
                let reverse = Checkbox::new(Bind::given(now.reverse)).label(words.reverse.clone()).show(ui);
                absorb(&mut events, lift(reverse.events, &now, |fill, on| fill.reverse = on));
                let dither = Checkbox::new(Bind::given(now.dither)).label(words.dither.clone()).show(ui);
                absorb(&mut events, lift(dither.events, &now, |fill, on| fill.dither = on));
            });
            let align = Checkbox::new(Bind::given(now.align)).label(words.align.clone()).show(ui);
            absorb(&mut events, lift(align.events, &now, |fill, on| fill.align = on));
            let reset = Button::new(words.reset_alignment.clone()).enabled(now.offset != kurbo::Vec2::ZERO).show(ui);
            if clicked(&reset) {
                let mut centred = now.clone();
                centred.offset = kurbo::Vec2::ZERO;
                events.push(EditEvent::set(Mixed::Same(now.clone()), centred, Arc::from(words.reset_alignment.as_str()), None));
            }

            for event in events.iter() {
                value.reborrow().write(cx, event);
            }
            (events, picked, color_requested, screen_pick, selected)
        });
        let (events, picked, color_requested, screen_pick, selected) = shown.inner.unwrap_or_default();
        GradientFillEditorOutput { response: shown.output, events, picked, color_requested, screen_pick, selected }
    }
}

#[cfg(feature = "suite")]
pub(crate) mod stories;
