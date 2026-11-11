//! [`GradientEditor`]: a gradient's stop bar with the fields of the selected marker under it.

use std::marker::PhantomData;
use std::sync::Arc;

use craft_behavior::{EditEvent, EditEvents, Mixed};
use craft_color::ui::{ColorPicker, ScreenPickGesture};
use craft_theme::Srgba;
use craft_ui::contract::{Bind, EditResponse, Outcome, Pattern, Props as _, Remember as _, Show as _, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::requests::ColorRequest;
use egui::{Response, Ui};

use super::stops::{Sampler, StopRules};
use super::value::{GradientStops, StopColor, StopRef};

mod bar;
mod fields;
#[cfg(feature = "screen-testing")]
mod stories;

use bar::GradientBar;
pub use bar::StopDrop;
pub(crate) use bar::marker_texts;
use fields::{FIRST_FIELD, PickerSetup, Row, stop_color};

/// The key of the colour request a stop's colour well files.
const REQUEST_KEY: &str = "gradient-stop";

/// The selection the editor keeps when the host gives none: the first colour stop until set.
#[derive(Clone, Copy, Debug)]
struct Selection(Option<StopRef>);

impl Default for Selection {
    fn default() -> Self {
        Selection(Some(StopRef::Color(0)))
    }
}

/// What [`GradientEditor::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct GradientEditorOutput<P = ()> {
    /// This pass's edit of the stops: the bar's and the fields', folded into one event.
    pub edit: EditResponse<GradientStops>,
    /// The colour stop whose colour the host is asked to edit: a double click, Enter or Space on it, or a click on
    /// the Color well; never with `color_popover`.
    pub color_requested: Option<StopColor>,
    /// A payload dropped on the bar: where it landed and the payload.
    pub dropped: Option<(StopDrop, P)>,
    /// The popover picker's screen-pick press; the host answers with `sampled`.
    pub screen_pick: Option<ScreenPickGesture>,
    /// The selection after this pass.
    pub selected: Option<StopRef>,
}

impl<P> Outcome for GradientEditorOutput<P> {
    type Value = GradientStops;

    fn response(&self) -> &Response {
        &self.edit.response
    }
}

/// Edits a gradient's stops: `GradientEditor::new("Gradient", Bind::bound(&mut stops)).show(ui)`.
///
/// The bar fills the width offered. [`StopRows::Split`] rows, top to bottom: opacity markers (their midpoints just
/// above the ramp), the ramp, the colour midpoints, colour markers; [`StopRows::Combined`]: the midpoints, the ramp,
/// colour markers. The bar keeps its geometry right to left: the ramp is the gradient, not reading order.
///
/// Pointer: a press on a marker or diamond selects and grabs it without a jump; a drag moves it (stops pass their
/// neighbours and the selection follows) and is one gesture. A stop dragged more than the tear-off distance above or
/// below the ramp is removed while its row is above the minimum, and comes back on re-entry within the drag. Alt held
/// at the press drags a copy, which can't be torn off. A click on empty space adds a stop there (under `Split`, above
/// the ramp's centre an opacity stop) as one Set; a press and drag on empty space does nothing. A double click on a
/// colour stop asks for its colour.
///
/// Keys (one Tab stop; the selected marker is the active descendant): `+` (or `=`) and `-` select the next and previous
/// marker of the selected row in location order, midpoints between their stops; under `Split`, Up and Down select the
/// opacity or colour stop nearest the selected location; Left and Right move the selected stop or midpoint by 1 %,
/// 10 % with Shift; Delete and Backspace remove the selected stop and select the one that takes its index; Insert and
/// Cmd/Ctrl+Enter add a stop halfway in the selected row and select it; Enter and Space on a colour stop ask for its
/// colour, on an opacity stop or a midpoint they ask for its fields.
///
/// Fields (on by default): the selected marker's fields under the bar. A colour stop has a "Color" well (a click asks
/// for the colour, or opens the colour popover with `color_popover`) and, under `Combined`, an "Opacity"; an opacity
/// stop has an "Opacity"; a stop has a "Location" and a Delete button, disabled at the row's minimum; a midpoint has a
/// "Midpoint" in the rules' range. The number fields are percent, and their labels scrub them. A Location run that
/// passes a neighbour keeps the selection on its stop.
///
/// The selection is the host's `shared_selection` binding when given (the canvas gradient line shares it), else the editor's
/// own: the first colour stop until set.
#[must_use = "show the editor with `show`"]
#[derive(Pattern)]
#[pattern(probe(GradientEditor<'static, u8>))]
pub struct GradientEditor<'a, P: Clone + Send + Sync + 'static = ()> {
    label: Label,
    stops: Bind<'a, GradientStops>,
    rules: StopRules,
    midpoints: bool,
    opacity_midpoints: bool,
    sampler: Option<&'a Sampler>,
    location_decimals: usize,
    fields: bool,
    color_popover: bool,
    picker: Option<PickerSetup<'a>>,
    shared_selection: Option<Bind<'a, Option<StopRef>>>,
    sampled: Option<Srgba>,
    drop_target: bool,
    enabled: bool,
    payload: PhantomData<fn() -> P>,
}

impl<'a> GradientEditor<'a> {
    /// An editor named `label` ("Gradient") of `stops`.
    pub fn new(label: impl Into<Label>, stops: Bind<'a, GradientStops>) -> Self {
        GradientEditor {
            label: label.into(),
            stops,
            rules: StopRules::default(),
            midpoints: true,
            opacity_midpoints: true,
            sampler: None,
            location_decimals: 0,
            fields: true,
            color_popover: false,
            picker: None,
            shared_selection: None,
            sampled: None,
            drop_target: false,
            enabled: true,
            payload: PhantomData,
        }
    }
}

impl<'a, P: Clone + Send + Sync + 'static> GradientEditor<'a, P> {
    /// The row layout, the fewest and most stops a row keeps and the range of a midpoint (PhotoCraft: two rows, 2 and
    /// 1024 stops, 5 % to 95 %).
    pub fn rules(mut self, rules: StopRules) -> Self {
        self.rules = rules;
        self
    }

    /// Whether the midpoint diamonds show and edit, on both rows; on by default. Off hides the opacity row's too.
    pub fn midpoints(mut self, midpoints: bool) -> Self {
        self.midpoints = midpoints;
        self
    }

    /// Whether the opacity row of a split bar has midpoint diamonds, for a host whose opacity stops carry none; on by
    /// default.
    pub fn opacity_midpoints(mut self, opacity_midpoints: bool) -> Self {
        self.opacity_midpoints = opacity_midpoints;
        self
    }

    /// Samples the gradient for the ramp and new stops instead of the default sampler.
    pub fn sampler(mut self, sampler: &'a Sampler) -> Self {
        self.sampler = Some(sampler);
        self
    }

    /// Decimals of a location in percent; 0 by default.
    pub fn location_decimals(mut self, decimals: usize) -> Self {
        self.location_decimals = decimals;
        self
    }

    /// Whether the selected marker's fields show under the bar; on by default.
    pub fn fields(mut self, fields: bool) -> Self {
        self.fields = fields;
        self
    }

    /// Whether the editor edits a colour stop's colour in a colour popover itself, anchored on the Color well (on the bar
    /// without the fields); off by default, when the request goes to the host in `color_requested`.
    pub fn color_popover(mut self, popover: bool) -> Self {
        self.color_popover = popover;
        self
    }

    /// Sets up the colour popover's picker (texts, constraint, models, eyedropper): `setup` gets the Compact picker, with
    /// alpha under `Combined`.
    pub fn picker(mut self, setup: impl for<'b> Fn(ColorPicker<'b>) -> ColorPicker<'b> + 'a) -> Self {
        self.picker = Some(Box::new(setup));
        self
    }

    /// The host's selection, shared with the canvas gradient line.
    pub fn shared_selection(mut self, selection: Bind<'a, Option<StopRef>>) -> Self {
        self.shared_selection = Some(selection);
        self
    }

    /// The colour the host's screen pick answered with, for the popover's picker.
    pub fn sampled(mut self, color: Srgba) -> Self {
        self.sampled = Some(color);
        self
    }

    /// Whether the editor takes input.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Takes drops of `Q` payloads: while one hovers, a line marks the place; the release is reported in
    /// `dropped` and the host applies it.
    pub fn drop_target<Q: Clone + Send + Sync + 'static>(self) -> GradientEditor<'a, Q> {
        let GradientEditor {
            label,
            stops,
            rules,
            midpoints,
            opacity_midpoints,
            sampler,
            location_decimals,
            fields,
            color_popover,
            picker,
            shared_selection,
            sampled,
            enabled,
            ..
        } = self;
        GradientEditor {
            label,
            stops,
            rules,
            midpoints,
            opacity_midpoints,
            sampler,
            location_decimals,
            fields,
            color_popover,
            picker,
            shared_selection,
            sampled,
            drop_target: true,
            enabled,
            payload: PhantomData,
        }
    }

    /// Shows the bar and the fields.
    pub fn show(self, ui: &mut Ui) -> GradientEditorOutput<P> {
        let GradientEditor {
            label,
            mut stops,
            rules,
            midpoints,
            opacity_midpoints,
            sampler,
            location_decimals,
            fields,
            color_popover,
            picker,
            shared_selection: mut selected,
            sampled,
            drop_target,
            enabled,
            ..
        } = self;
        let words = Env::of(ui).words::<super::words::GradientWords>();
        let mut output = None;
        let shown = Stack::new().fill_width().show(ui, |ui, cx| {
            let held = match selected.as_mut() {
                Some(bind) => bind.reborrow().read(cx).and_then(|held| held.same().copied()).unwrap_or(None),
                None => cx.memory::<Selection>().0,
            };
            let mut bar = GradientBar::<P>::new(label.clone(), stops.reborrowed())
                .rules(rules.clone())
                .midpoints(midpoints)
                .opacity_midpoints(opacity_midpoints)
                .location_decimals(location_decimals)
                .enabled(enabled);
            if let Some(held) = held {
                bar = bar.selected_marker(held);
            }
            if let Some(sampler) = sampler {
                bar = bar.sampler(sampler);
            }
            if drop_target {
                bar = bar.drop_target();
            }
            let bar = bar.show(ui);
            let mut edit = bar.edit;
            let mut now_selected = bar.select.unwrap_or(held);
            let mut color_requested = bar.color_requested;
            let mut screen_pick = None;
            if let Some(now) = stops.reborrow().read(cx).and_then(|stops| stops.same().cloned()) {
                let mut row = Row {
                    words: &words,
                    rules: &rules,
                    location_decimals,
                    enabled,
                    fields,
                    color_popover,
                    picker: &picker,
                    sampled,
                    requested: color_requested,
                };
                if bar.field_requested && fields {
                    craft_ui::runtime::focus::request(ui.ctx(), craft_ui::runtime::focus::FocusTarget::Id(cx.key(FIRST_FIELD)));
                }
                let report = row.show(ui, cx, &now, now_selected, &edit.response);
                color_requested = report.color_requested;
                screen_pick = report.screen_pick;
                if let Some(select) = report.select {
                    now_selected = select;
                }
                for event in report.events {
                    stops.reborrow().write(cx, &event);
                    edit.events.push(event);
                }
                color_requested = color_requested.filter(|i| *i < now.color.len());
                if let Some(i) = color_requested {
                    let request = ColorRequest::new(REQUEST_KEY, stop_color(&now, rules.rows, i)).title(label.clone());
                    output = Some(StopColor::new(i, now.color[i].tag, request));
                }
            }
            if now_selected != held {
                match selected.as_mut() {
                    Some(bind) => {
                        let event = EditEvent::set(Mixed::Same(held), now_selected, Arc::from("Select stop"), None);
                        bind.reborrow().write(cx, &event);
                    }
                    None => cx.memory::<Selection>().0 = now_selected,
                }
            }
            (edit, bar.dropped, screen_pick, now_selected)
        });
        match shown.inner {
            Some((edit, dropped, screen_pick, selected)) => GradientEditorOutput { edit, color_requested: output, dropped, screen_pick, selected },
            None => GradientEditorOutput {
                edit: EditResponse::lifted(shown.output, EditEvents::new(), None),
                color_requested: None,
                dropped: None,
                screen_pick: None,
                selected: None,
            },
        }
    }
}
