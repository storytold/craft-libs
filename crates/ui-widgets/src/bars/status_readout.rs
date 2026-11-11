//! `StatusReadout`: the document readout of a status bar: the zoom field, one chosen information item and the menu
//! that chooses it. A pattern over the children of a `Bar` with `BarKind::Status`.

use super::words::BarWords;
use crate::buttons::IconButton;
use crate::contract::{Bind, Entry, GlyphLabel, LabelKind, NoValue, Outcome, Props as _, Show as _, ShowContainer as _, SideLabel, Sizing};
use crate::display::{Elide, Text};
use crate::form::Field;
use crate::layout_components::Row;
use crate::runtime::commands::{self, CommandRef};
use crate::runtime::env::{Env, Label, Units};
use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::Size;
use craft_behavior::{CanvasView, EditEvents, IconId, Mixed, PixelSize, ZoomLadder};
use egui::{Response, Ui};
use std::time::Duration;

#[cfg(feature = "testing")]
mod stories;

/// Shown for an item whose value the host does not know.
const MISSING: &str = "\u{2014}";

/// The item the readout shows, chosen in its menu.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StatusItem {
    /// The flattened size and the size with layers.
    #[default]
    DocumentSizes,
    /// The document's colour profile name.
    DocumentProfile,
    /// The document's dimensions text.
    DocumentDimensions,
    /// The document's measurement scale text.
    MeasurementScale,
    /// Memory the app uses for open documents and what it may use.
    ScratchSizes,
    /// The share of time spent working rather than waiting on scratch.
    Efficiency,
    /// How long the last operation took.
    Timing,
    /// The name of the active tool.
    CurrentTool,
    /// The number of layers in the document.
    LayerCount,
}

/// The values the host knows this frame; an item without a value shows a dash.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct StatusValues {
    /// Flattened and layered sizes in bytes.
    pub document_sizes: Option<(u64, u64)>,
    /// The colour profile name, shown as given.
    pub document_profile: Option<String>,
    /// The document's size in pixels, shown in the user's unit.
    pub document_dimensions: Option<PixelSize>,
    /// The measurement scale text, shown as given.
    pub measurement_scale: Option<String>,
    /// Used and available bytes.
    pub scratch_sizes: Option<(u64, u64)>,
    /// 0 to 100.
    pub efficiency: Option<u8>,
    /// How long the last operation took.
    pub timing: Option<Duration>,
    /// The active tool's command; the readout shows its label as the rail and the menus do.
    pub current_tool: Option<CommandRef>,
    /// The number of layers.
    pub layer_count: Option<usize>,
}

impl StatusItem {
    /// Every item in the menu's order.
    pub const ALL: [StatusItem; 9] = [
        StatusItem::DocumentSizes,
        StatusItem::DocumentProfile,
        StatusItem::DocumentDimensions,
        StatusItem::MeasurementScale,
        StatusItem::ScratchSizes,
        StatusItem::Efficiency,
        StatusItem::Timing,
        StatusItem::CurrentTool,
        StatusItem::LayerCount,
    ];

    /// The item's name in the menu.
    fn name(self, words: &BarWords) -> Label {
        match self {
            StatusItem::DocumentSizes => words.readout_document_sizes.clone(),
            StatusItem::DocumentProfile => words.readout_document_profile.clone(),
            StatusItem::DocumentDimensions => words.readout_document_dimensions.clone(),
            StatusItem::MeasurementScale => words.readout_measurement_scale.clone(),
            StatusItem::ScratchSizes => words.readout_scratch_sizes.clone(),
            StatusItem::Efficiency => words.readout_efficiency.clone(),
            StatusItem::Timing => words.readout_timing.clone(),
            StatusItem::CurrentTool => words.readout_current_tool.clone(),
            StatusItem::LayerCount => words.readout_layer_count.clone(),
        }
    }

    /// The item's text for `values`, a dash when the value is missing.
    fn text(self, values: &StatusValues, words: &BarWords, env: &Env, ctx: &egui::Context) -> String {
        let units: Units = env.units();
        let given = |value: &Option<String>| value.clone().unwrap_or_else(|| MISSING.to_owned());
        match self {
            StatusItem::DocumentSizes => values.document_sizes.map_or_else(
                || MISSING.to_owned(),
                |(flat, layered)| words.readout_doc_sizes.fill(&[("flat", &units.file_size(flat)), ("layered", &units.file_size(layered))]),
            ),
            StatusItem::DocumentProfile => given(&values.document_profile),
            StatusItem::DocumentDimensions => values.document_dimensions.map_or_else(
                || MISSING.to_owned(),
                |size| {
                    words
                        .readout_dimensions_text
                        .fill(&[("width", &units.pixels(f64::from(size.width))), ("height", &units.pixels(f64::from(size.height)))])
                },
            ),
            StatusItem::MeasurementScale => given(&values.measurement_scale),
            StatusItem::ScratchSizes => values.scratch_sizes.map_or_else(
                || MISSING.to_owned(),
                |(used, available)| words.readout_scratch.fill(&[("used", &units.file_size(used)), ("available", &units.file_size(available))]),
            ),
            StatusItem::Efficiency => values
                .efficiency
                .map_or_else(|| MISSING.to_owned(), |n| words.readout_efficiency_text.fill(&[("value", &units.percent(f64::from(n.min(100))))])),
            StatusItem::Timing => values
                .timing
                .map_or_else(|| MISSING.to_owned(), |time| words.readout_timing_text.fill(&[("value", &units.duration(time.as_secs_f64()))])),
            StatusItem::CurrentTool => {
                values.current_tool.as_ref().map_or_else(|| MISSING.to_owned(), |tool| commands::resolve(ctx, tool.clone()).label.to_string())
            }
            StatusItem::LayerCount => {
                values.layer_count.map_or_else(|| MISSING.to_owned(), |n| words.readout_layers.text(n as u64, &env.format().integer(n as u64)))
            }
        }
    }
}

/// What the readout reports.
#[non_exhaustive]
pub struct StatusReadoutOutput {
    /// The readout row's response.
    pub response: Response,
    /// The zoom field's response; `None` when the row did not run.
    pub zoom: Option<Response>,
    /// The view changes of this pass (a typed or scrubbed zoom about the view's centre), for a binding that does not
    /// write through.
    pub view: EditEvents<CanvasView>,
    /// The item chosen in the menu this pass; the host stores it.
    pub item: Option<StatusItem>,
}

impl Outcome for StatusReadoutOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        crate::testing::StoryOutput::of_edit(&self.response, &self.view)
    }
}

/// The zoom field, the chosen item's text and the menu that chooses it, shown as children of a status `Bar`.
///
/// - The field shows the bound view's zoom in percent of device pixels, as the Navigator's zoom row does. A typed or
///   scrubbed zoom is clamped to the document's zoom range and written into the view about its centre.
/// - The item is the caller's value, and a pick comes back in the output.
#[derive(crate::contract::Pattern)]
pub struct StatusReadout<'a> {
    view: Bind<'a, CanvasView>,
    document: Size,
    item: StatusItem,
    values: &'a StatusValues,
    tips: Vec<(StatusItem, Label)>,
}

impl<'a> StatusReadout<'a> {
    /// A readout of the zoom of `view` over a document of `document` units that shows `item` from `values`.
    pub fn new(view: Bind<'a, CanvasView>, document: Size, item: StatusItem, values: &'a StatusValues) -> Self {
        StatusReadout { view, document, item, values, tips: Vec::new() }
    }

    /// A tooltip for the info text while `item` is the one shown.
    #[must_use]
    pub fn tooltip(mut self, item: StatusItem, text: impl Into<Label>) -> Self {
        self.tips.retain(|(shown, _)| *shown != item);
        self.tips.push((item, text.into()));
        self
    }

    /// Shows the readout in a row: the zoom field, whose glyph label scrubs the zoom, the info text and the menu
    /// button.
    pub fn show(mut self, ui: &mut Ui) -> StatusReadoutOutput {
        let env = Env::of(ui);
        let words = env.words::<BarWords>();
        let text = self.item.text(self.values, &words, &env, ui.ctx());
        let pixels_per_point = env.scale.0;
        let document = self.document;
        let mut events = EditEvents::new();
        let shown = Row::new().show(ui, |ui, cx| {
            let current = match self.view.reborrow().read(cx) {
                Some(Mixed::Same(current)) => Some(current),
                _ => None,
            };
            let device = current.as_ref().map(|view| zoom::device(view, pixels_per_point));
            let glyph = SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::icon(IconId::new("zoom_in")))).scrub(true);
            let field = Field::new(words.readout_zoom.clone()).side(glyph).sizing(Sizing::Content).show(ui, |ui, _| {
                let mut number = crate::text::NumberField::new(words.readout_zoom.clone(), Bind::given(device.map(|device| device * 100.0)))
                    .codec(env.units().zoom_codec())
                    .scrub_by_label()
                    .enabled(current.is_some());
                if let Some(device) = device {
                    number = number.range(ZoomLadder::percent_range(device, document));
                }
                number.show(ui)
            });
            let typed = field.inner.as_ref().map(|zoom| zoom.edit.events.clone()).unwrap_or_default();
            events = match current {
                Some(current) => {
                    typed.map(|percent| percent.map_or(current, |percent| zoom::with_device(&current, percent / 100.0, document, pixels_per_point)))
                }
                None => EditEvents::new(),
            };
            for edit in &events {
                self.view.reborrow().write(cx, edit);
            }
            let mut info = Text::new(text.clone()).elide(Elide::End);
            if let Some((_, tip)) = self.tips.iter().find(|(item, _)| *item == self.item) {
                info = info.tooltip(tip.clone());
            }
            info.show(ui);
            let more = IconButton::new(IconId::new("chevron_right")).label(words.readout_menu.clone()).show(ui);
            let entries: Vec<Entry<StatusItem>> =
                StatusItem::ALL.iter().map(|&item| Entry::new(item, item.name(&words)).checked(item == self.item)).collect();
            let menu = crate::menu::pick_values_from(ui, words.readout_menu.clone(), &more, &entries);
            (field.inner.map(|zoom| zoom.edit.response), menu.picked)
        });
        let (zoom, item) = shown.inner.unzip();
        StatusReadoutOutput { response: shown.output, zoom: zoom.flatten(), view: events, item: item.flatten() }
    }
}
