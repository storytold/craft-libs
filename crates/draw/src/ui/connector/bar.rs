//! [`ConnectorBar`]: the connector part of an options bar.

use craft_behavior::{IconId, Mixed};
use craft_ui::choice::Segmented;
use craft_ui::contract::{Bind, Entry, Outcome, Show as _, ShowContainer as _};
use craft_ui::layout_components::Row;
use craft_ui::list::ThumbSize;
use craft_ui::pickers::Select;
use craft_ui::runtime::env::Env;
use egui::{Response, Ui};

use super::words::ConnectorBarWords;
use crate::geometry::connector::Routing;
use crate::stroke::StrokeStyle;
use crate::ui::stroke::{StrokeChoiceRow, StrokePreview};

/// A connector's line style as the bar's menu offers it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ConnectorLine {
    /// Dashed with [`DASH`](Self::DASH), else solid.
    pub dashed: bool,
    /// The stroke weight in points.
    pub weight: f64,
}

impl ConnectorLine {
    /// A line of `weight` points, dashed or solid.
    pub fn new(dashed: bool, weight: f64) -> Self {
        ConnectorLine { dashed, weight }
    }

    /// The weights the menu offers (FigCraft's `WEIGHTS`).
    pub const WEIGHTS: [f64; 4] = [1.0, 2.0, 4.0, 6.0];
    /// A dashed connector's dash and gap (FigCraft's `DASHES`).
    pub const DASH: [f64; 2] = [8.0, 6.0];
}

impl Default for ConnectorLine {
    /// Solid, 2 points.
    fn default() -> Self {
        ConnectorLine { dashed: false, weight: 2.0 }
    }
}

/// What [`ConnectorBar`] reported this pass: one field per control, `Some` when that control was
/// changed. Each change is one undo step.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the app applies each choice to the selected connectors"]
pub struct ConnectorBarOutput {
    /// The bar's response.
    pub response: Response,
    /// The routing button picked.
    pub routing: Option<Routing>,
    /// The start cap picked: the choice id from the app's start row.
    pub start_cap: Option<String>,
    /// The end cap picked: the choice id from the app's end row.
    pub end_cap: Option<String>,
    /// The line style menu's pick, as the whole new line.
    pub line: Option<ConnectorLine>,
}

impl Outcome for ConnectorBarOutput {
    type Value = craft_ui::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A row of the line style menu: the solid or dashed rows set the dashes, a weight row the weight.
#[derive(Clone, Copy, Debug, PartialEq)]
enum LinePick {
    Style(bool),
    Weight(usize),
}

/// A connector's line at `weight`, dashed or not, as the stroke preview paints it.
fn sample(weight: f64, dashed: bool) -> StrokeStyle {
    let dashes = if dashed { ConnectorLine::DASH.map(|n| n as f32).to_vec() } else { Vec::new() };
    StrokeStyle { width: weight as f32, dashes, ..StrokeStyle::default() }
}

/// The connector part of an options bar, in FigCraft's order: three routing buttons (Straight, Elbow,
/// Curved, drawn as icons; a radio group), the start and end cap menus (the app's
/// [`StrokeChoiceRow`]s from [`caps`](Self::caps), with their painted previews; the library holds no
/// arrowhead data; no menus without them), and the line style menu (Solid, Dash, then the
/// [weights](ConnectorLine::WEIGHTS)). Across a selection a value that differs shows as mixed, and
/// any pick applies to every connector. It is shown as a child of the host's options `Bar`; the
/// choices come back in the [`ConnectorBarOutput`].
#[must_use = "show the bar with `show`"]
#[derive(craft_ui::contract::Pattern)]
pub struct ConnectorBar<'a> {
    routing: Mixed<Routing>,
    line: Mixed<ConnectorLine>,
    caps: Option<(StrokeChoiceRow<'a>, StrokeChoiceRow<'a>)>,
}

impl<'a> ConnectorBar<'a> {
    /// The bar for connectors routed `routing` with `line`.
    pub fn new(routing: Mixed<Routing>, line: Mixed<ConnectorLine>) -> Self {
        ConnectorBar { routing, line, caps: None }
    }

    /// The start and end cap menus: the app's rows of arrowheads, each with its picked choice
    /// (mixed across a selection) and its preview painter.
    pub fn caps(mut self, start: StrokeChoiceRow<'a>, end: StrokeChoiceRow<'a>) -> Self {
        self.caps = Some((start, end));
        self
    }

    /// Shows the bar.
    pub fn show(self, ui: &mut Ui) -> ConnectorBarOutput {
        let ConnectorBar { routing, line, caps } = self;
        let words = Env::of(ui).words::<ConnectorBarWords>();
        let mut routing_pick = None;
        let (mut start_cap, mut end_cap, mut line_pick) = (None, None, None);
        let shown = Row::new().show(ui, |ui, _| {
            let routes = [
                Entry::new(Routing::Straight, words.straight.clone()).icon(IconId::new("route_straight")),
                Entry::new(Routing::Elbow, words.elbow.clone()).icon(IconId::new("route_elbow")),
                Entry::new(Routing::Curved, words.curved.clone()).icon(IconId::new("route_curved")),
            ];
            routing_pick = Segmented::new(words.routing.clone(), Bind::given(routing), &routes).show(ui).value().copied();

            if let Some((start, end)) = &caps {
                for (row, name, slot) in [(start, &words.start, &mut start_cap), (end, &words.end, &mut end_cap)] {
                    let entries: Vec<Entry<String>> = row.choices.iter().map(|c| Entry::new(c.id.clone(), c.label.clone())).collect();
                    let current = match row.selected {
                        Some(Mixed::Same(id)) => Mixed::Same(id.to_owned()),
                        _ => Mixed::Mixed,
                    };
                    let mut select = Select::new(name.clone(), Bind::given(current), &entries);
                    if let Some(paint) = &row.paint {
                        let choices = row.choices;
                        select = select.decorate(move |id: &String, list_row| match choices.iter().find(|c| &c.id == id) {
                            Some(choice) => list_row.thumb(ThumbSize::Small, move |cx, rect| paint.paint(cx, rect, choice)),
                            None => list_row,
                        });
                    }
                    if let Some(id) = select.show(ui).value().cloned() {
                        *slot = Some(id);
                    }
                }
            }

            let mut entries = vec![
                Entry::new(LinePick::Style(false), words.solid.clone()),
                Entry::new(LinePick::Style(true), words.dashed.clone()),
                Entry::separator(),
            ];
            for (index, weight) in ConnectorLine::WEIGHTS.iter().enumerate() {
                entries.push(Entry::new(LinePick::Weight(index), words.weight.fill(&[("weight", &weight.to_string())])));
            }
            let current = match line {
                Mixed::Same(held) => Mixed::Same(LinePick::Style(held.dashed)),
                Mixed::Mixed => Mixed::Mixed,
            };
            let held = match line {
                Mixed::Same(held) => held,
                Mixed::Mixed => ConnectorLine::default(),
            };
            let select = Select::new(words.line_style.clone(), Bind::given(current), &entries).decorate(move |pick: &LinePick, list_row| {
                let (dashed, weight) = match *pick {
                    LinePick::Style(dashed) => (dashed, held.weight),
                    LinePick::Weight(index) => (held.dashed, ConnectorLine::WEIGHTS[index]),
                };
                list_row.thumb(ThumbSize::Small, move |cx, rect| StrokePreview::new(&sample(weight, dashed), weight as f32).paint(cx, rect))
            });
            if let Some(pick) = select.show(ui).value().copied() {
                line_pick = Some(match pick {
                    LinePick::Style(dashed) => ConnectorLine { dashed, ..held },
                    LinePick::Weight(index) => ConnectorLine { weight: ConnectorLine::WEIGHTS[index], ..held },
                });
            }
        });
        ConnectorBarOutput { response: shown.output, routing: routing_pick, start_cap, end_cap, line: line_pick }
    }
}
