//! [`ArrowheadsPopover`]: the Line tool's arrowhead settings.

use std::sync::Arc;

use super::panel::NumberCell;
use super::{Arrowheads, ShapeWords};
use craft_behavior::{EditEvent, EditEvents, IconId, Mixed};
use craft_ui::buttons::IconButton;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, LabelScheme, PopupKind, Props as _, Show as _, ShowContainer as _, clicked};
use craft_ui::form::FieldGroup;
use craft_ui::layout_components::Row;
use craft_ui::popover::Popover;
use craft_ui::runtime::env::{Env, Units};
use craft_ui::text::RangeAlert;
use egui::{Response, Ui};

/// The arrowhead popover: which ends carry a head and the head's width, length and concavity. Its `open` flag is read
/// and written through the binding, so give it `Bind::bound`.
#[must_use = "show the popover with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct ArrowheadsPopover<'a> {
    value: Bind<'a, Arrowheads>,
    open: Bind<'a, bool>,
    pixels_per_inch: f64,
}

/// What [`ArrowheadsPopover::show`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct ArrowheadsOutput {
    /// The button's response.
    pub response: Response,
    /// The edits of this pass in order, each the whole value with one field replaced. A gesture's first pass carries
    /// its `Begin` and first `Update` together.
    pub edits: Vec<EditEvent<Arrowheads>>,
    /// A value typed past its field's range this pass, which the field clamped: the app answers with its range alert.
    pub out_of_range: Option<RangeAlert>,
}

impl<'a> ArrowheadsPopover<'a> {
    /// The popover of `value`, open while `open` is set, in a document of `pixels_per_inch`, which sizes a length typed
    /// in inches or points.
    pub fn new(value: Bind<'a, Arrowheads>, open: Bind<'a, bool>, pixels_per_inch: f64) -> Self {
        ArrowheadsPopover { value, open, pixels_per_inch }
    }

    /// Shows the button and, while open, the popover.
    pub fn show(self, ui: &mut Ui) -> ArrowheadsOutput {
        let ArrowheadsPopover { mut value, mut open, pixels_per_inch } = self;
        let env = Env::of(ui);
        let (words, units) = (env.words::<ShapeWords>(), env.units());
        let current = match value.reborrow().get() {
            Some(Mixed::Same(shown)) => shown,
            _ => Arrowheads::default(),
        };
        let was = matches!(open.reborrow().get(), Some(Mixed::Same(true)));
        let gear = IconButton::new(IconId::new("settings")).label(words.arrowheads.clone()).popup(PopupKind::Dialog).expanded(was).show(ui).response;
        if clicked(&gear) {
            // The popover's opener is manual: the gear's press is the pattern's to apply.
            let event = EditEvent::set(Mixed::Same(was), !was, Arc::from(words.arrowheads.as_str()), None);
            let _ = open.reborrow().apply(&event);
        }
        let (mut edits, mut out_of_range) = (Vec::new(), None);
        let _ = Popover::new(words.arrowheads.clone(), open.reborrowed()).opened_by(&gear).show(ui, |ui, _| {
            FieldGroup::new(words.arrowheads.clone()).labels(LabelScheme::Inspector).show(ui, |ui, _| {
                (edits, out_of_range) = arrow_rows(ui, &words, &units, pixels_per_inch, current, |_| {});
            });
        });
        ArrowheadsOutput { response: gear, edits, out_of_range }
    }
}

/// The arrowhead rows of `value`, shared by the popover and the Shape panel: the Start and End check boxes, then Width
/// and Length, then whatever `after_length` adds (the panel's Weight), then Concavity (-50 to 50 %, whole). Each edit is
/// the whole value with its one field replaced; the alert is that of a value typed past its field's range.
pub(super) fn arrow_rows(
    ui: &mut Ui,
    words: &ShapeWords,
    units: &Units,
    pixels_per_inch: f64,
    value: Arrowheads,
    after_length: impl FnOnce(&mut Ui),
) -> (Vec<EditEvent<Arrowheads>>, Option<RangeAlert>) {
    let (mut edits, mut alerts) = (Vec::new(), Vec::new());
    let (mut start, mut end) = (EditEvents::new(), EditEvents::new());
    Row::new().show(ui, |ui, _| {
        start = Checkbox::new(Bind::given(value.start)).label(words.arrow_start.clone()).show(ui).events;
        end = Checkbox::new(Bind::given(value.end)).label(words.arrow_end.clone()).show(ui).events;
    });
    edits.extend(start.map(|on| Arrowheads { start: on, ..value }));
    edits.extend(end.map(|on| Arrowheads { end: on, ..value }));
    let (width, alert) =
        NumberCell::pixels(units, pixels_per_inch, "arrow-width", &words.arrow_width, value.width, Arrowheads::ARROW_SIZE).labelled(ui);
    edits.extend(width.map(|width| Arrowheads { width, ..value }));
    alerts.push(alert);
    let (length, alert) =
        NumberCell::pixels(units, pixels_per_inch, "arrow-length", &words.arrow_length, value.length, Arrowheads::ARROW_SIZE).labelled(ui);
    edits.extend(length.map(|length| Arrowheads { length, ..value }));
    alerts.push(alert);
    after_length(ui);
    let (concavity, alert) =
        NumberCell::whole_percent(units, "arrow-concavity", &words.arrow_concavity, value.concavity, Arrowheads::ARROW_CONCAVITY).labelled(ui);
    edits.extend(concavity.map(|concavity| Arrowheads { concavity, ..value }));
    alerts.push(alert);
    (edits, alerts.into_iter().flatten().last())
}
