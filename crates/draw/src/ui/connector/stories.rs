//! Stories of the Connector tool, run live over three boxes, of its routes between every pair of sides, and of the
//! connector handles and the connector bar.

use craft_behavior::kurbo::{BezPath, Point, Rect, Size, Vec2};
use craft_behavior::{CanvasView, EditEvent, Mixed};
use craft_ui::bars::{Bar, BarKind};
use craft_ui::canvas::RasterSource as _;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::runtime::env::Env;
use craft_ui::testing::canvas_story::{OUTLINE, StoryDocument, StoryPicture, StoryShape, accent, on_picture, paint_shapes};
use craft_ui::testing::{Drive, EditKind, Grip, StoryButton, StoryData, StoryOutput, ToolStory, story, tool_story};
use egui::{Color32, Ui, vec2};
use std::hash::{DefaultHasher, Hash as _, Hasher as _};

use super::{
    ConnectionSite, ConnectionTarget, ConnectorBar, ConnectorEnd, ConnectorEvent, ConnectorHandles, ConnectorItem, ConnectorLine, ConnectorTool,
};
use crate::geometry::connector::{ConnectionSide, RouteEnd, Routing, route, route_along};
use crate::ui::stroke::{StrokeChoice, StrokeChoiceRow};

craft_ui::words! {
    /// The status line of the Connector tool story.
    #[non_exhaustive]
    pub(crate) struct ConnectorStoryWords {
        /// The status line: the last thing the tool or the handles did.
        status: template "connectorStory.status" = "Last event: {event}";
        /// Nothing done so far.
        idle: "connectorStory.idle" = "none";
        /// A connector being drawn.
        drawing: "connectorStory.drawing" = "drawing a connector";
        /// A connector drawn.
        drew: "connectorStory.drew" = "drew a connector";
        /// A drawing given up.
        cancelled: "connectorStory.cancelled" = "drawing cancelled";
        /// The start end glued elsewhere.
        reattached_start: "connectorStory.reattachedStart" = "moved the start";
        /// The end glued elsewhere.
        reattached_end: "connectorStory.reattachedEnd" = "moved the end";
        /// A bend moved.
        reshaped: "connectorStory.reshaped" = "reshaped the connector";
    }
}

fn targets() -> Vec<ConnectionTarget> {
    vec![
        ConnectionTarget::new(1, Rect::new(170.0, 190.0, 250.0, 250.0)).named("Rectangle 1"),
        ConnectionTarget::new(2, Rect::new(390.0, 150.0, 470.0, 210.0)).named("Rectangle 2"),
    ]
}

fn glued(target: u64, side: ConnectionSide) -> ConnectorEnd {
    ConnectorEnd::Glued { target, side, site: None, at: None }
}

/// The story document with `targets` drawn on it as boxes in the accent.
fn boxes(ui: &Ui, targets: &[ConnectionTarget]) -> StoryPicture<impl Fn(Point) -> Color32 + use<>> {
    let (fill, line) = (accent(ui, 0.15), accent(ui, 1.0));
    let shapes: Vec<StoryShape> = targets.iter().map(|target| StoryShape::filled(&target.bounds, fill).outlined(OUTLINE, line)).collect();
    let size = StoryDocument::STANDARD.size();
    StoryPicture::new(size.width, size.height, move |p| paint_shapes(&shapes, Color32::TRANSPARENT, p))
}

/// Shows the handles `handles` builds and shows as the only canvas control of a canvas over `targets`. The canvas body
/// runs more than once per pass, so `handles` builds them on every run.
fn on_canvas<R: Into<StoryOutput>>(ui: &mut Ui, targets: &[ConnectionTarget], mut handles: impl FnMut(&mut Ui, egui::Rect) -> R) -> StoryOutput {
    let picture = boxes(ui, targets);
    on_picture(ui, &picture, |ui, at, _| handles(ui, at))
}

#[story(ConnectorHandles<'static>, covers = [Default], drive = Drive::Drag(Grip::part(super::handles::END.at(0)).toward(vec2(0.0, 1.0))))]
fn connector_glued_at_both_ends(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let targets = targets();
    let item = data.value(
        "item",
        ConnectorItem {
            id: 9,
            from: ConnectorEnd::Glued { target: 1, side: ConnectionSide::Right, site: None, at: Some([1.0, 0.5]) },
            to: glued(2, ConnectionSide::Left),
            routing: Routing::Elbow,
            path: None,
        },
    );
    on_canvas(ui, &targets, |ui, at| ConnectorHandles::new("Connector", Bind::bound(&mut *item), &targets).show_at(ui, at))
}

fn caps() -> Vec<StrokeChoice> {
    vec![StrokeChoice::new("none", "None"), StrokeChoice::new("arrow", "Arrow"), StrokeChoice::new("dot", "Dot")]
}

#[story(ConnectorBar<'static>, covers = [Default], drive = Drive::Never)]
fn connector_bar_for_one_connector(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let choices = caps();
    Bar::new("Options", BarKind::Options)
        .show(ui, |ui, _| {
            let start = StrokeChoiceRow::new("start", "Start", &choices).selected(Mixed::Same("none"));
            let end = StrokeChoiceRow::new("end", "End", &choices).selected(Mixed::Same("arrow"));
            let _ = ConnectorBar::new(Mixed::Same(Routing::Elbow), Mixed::Same(ConnectorLine::default())).caps(start, end).show(ui);
        })
        .output
        .into()
}

#[story(ConnectorBar<'static>, covers = [Mixed], drive = Drive::Never)]
fn connector_bar_for_a_mixed_selection(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let choices = caps();
    Bar::new("Options", BarKind::Options)
        .show(ui, |ui, _| {
            let start = StrokeChoiceRow::new("start", "Start", &choices).selected(Mixed::Mixed);
            let end = StrokeChoiceRow::new("end", "End", &choices).selected(Mixed::Same("arrow"));
            let _ = ConnectorBar::new(Mixed::Mixed, Mixed::Mixed).caps(start, end).show(ui);
        })
        .output
        .into()
}

/// A box at shares of the story document: its left, top, width and height.
fn share(x: f64, y: f64, w: f64, h: f64) -> Rect {
    let page = StoryDocument::STANDARD.size();
    let (width, height) = (f64::from(page.width), f64::from(page.height));
    Rect::from_origin_size((width * x, height * y), Size::new(width * w, height * h))
}

/// Three boxes; the third has six numbered sites (two on each long side and one on each short side), as DeckCraft's
/// shapes have.
fn board_targets() -> Vec<ConnectionTarget> {
    let third = share(0.38, 0.62, 0.26, 0.28);
    let (quarter, mid) = (third.width() / 4.0, third.center());
    let sites = vec![
        ConnectionSite::new(Point::new(third.x0 + quarter, third.y0), ConnectionSide::Top, 0),
        ConnectionSite::new(Point::new(third.x1 - quarter, third.y0), ConnectionSide::Top, 1),
        ConnectionSite::new(Point::new(third.x1, mid.y), ConnectionSide::Right, 2),
        ConnectionSite::new(Point::new(third.x1 - quarter, third.y1), ConnectionSide::Bottom, 3),
        ConnectionSite::new(Point::new(third.x0 + quarter, third.y1), ConnectionSide::Bottom, 4),
        ConnectionSite::new(Point::new(third.x0, mid.y), ConnectionSide::Left, 5),
    ];
    vec![
        ConnectionTarget::new(1, share(0.08, 0.12, 0.24, 0.26)).named("Rectangle 1"),
        ConnectionTarget::new(2, share(0.66, 0.1, 0.26, 0.26)).named("Rectangle 2"),
        ConnectionTarget::new(3, third).named("Rectangle 3").sites(sites),
    ]
}

/// Where `end` of a connector meets the boxes `targets`.
fn route_end(targets: &[ConnectionTarget], end: &ConnectorEnd) -> Option<RouteEnd> {
    match *end {
        ConnectorEnd::Free(point) => Some(RouteEnd::Point(point)),
        ConnectorEnd::Glued { target, side, site, at, .. } => {
            let target = targets.iter().find(|candidate| candidate.id == target)?;
            let bounds = target.bounds;
            if let Some([x, y]) = at {
                return Some(RouteEnd::On(bounds, side, Point::new(bounds.x0 + x * bounds.width(), bounds.y0 + y * bounds.height())));
            }
            Some(match site.and_then(|index| target.side_sites().into_iter().find(|site| site.index == index)) {
                Some(site) => RouteEnd::On(bounds, side, site.point),
                None => RouteEnd::Box(bounds, side),
            })
        }
    }
}

/// A route's line: a polyline, or a curve's cubic.
fn route_line(points: &[Point], routing: Routing) -> BezPath {
    let mut line = BezPath::new();
    match (routing, points) {
        (Routing::Curved, &[a, b, c, d]) => {
            line.move_to(a);
            line.curve_to(b, c, d);
        }
        (_, [first, rest @ ..]) => {
            line.move_to(*first);
            for point in rest {
                line.line_to(*point);
            }
        }
        _ => {}
    }
    line
}

/// The story document with `targets` drawn as boxes in the accent and `lines` over them in `ink`, at `revision`.
fn board_picture(
    ui: &Ui,
    targets: &[ConnectionTarget],
    lines: &[BezPath],
    ink: Color32,
    revision: u64,
) -> StoryPicture<impl Fn(Point) -> Color32 + use<>> {
    let (fill, edge) = (accent(ui, 0.15), accent(ui, 1.0));
    let mut shapes: Vec<StoryShape> = targets.iter().map(|target| StoryShape::filled(&target.bounds, fill).outlined(OUTLINE, edge)).collect();
    shapes.extend(lines.iter().map(|line| StoryShape::line(line, OUTLINE, ink)));
    let size = StoryDocument::STANDARD.size();
    StoryPicture::new(size.width, size.height, move |p| paint_shapes(&shapes, Color32::TRANSPARENT, p)).revision(revision)
}

/// The connectors the Connector tool story keeps as its document, and the one selected.
#[derive(Clone, Debug, PartialEq)]
struct Connectors {
    items: Vec<ConnectorItem>,
    selected: Option<usize>,
}

impl Connectors {
    /// Two connectors: box 1 to box 2, and box 2 to site 2 of box 3; the first selected.
    fn new(routing: Routing) -> Self {
        let first = ConnectorItem::new(1, glued(1, ConnectionSide::Right), glued(2, ConnectionSide::Left), routing);
        let second = ConnectorItem::new(2, glued(2, ConnectionSide::Bottom), glued(3, ConnectionSide::Right).site(2), routing);
        Connectors { items: vec![first, second], selected: Some(0) }
    }
}

/// The kept Connector tool, the routing it last drew with, and the last thing it or the handles did.
#[derive(Default)]
struct ConnectorKept {
    tool: ConnectorTool,
    routing: Option<Routing>,
    note: Option<String>,
}

/// The Connector tool live over three boxes, the third with numbered sites: a drag from a box draws a connector with
/// the routing the story picks, a hovered box shows its sites, and the new connector is selected, so its handles
/// reattach its ends and reshape its bends. Select Next moves the selection; the other connectors are drawn muted.
#[story(ConnectorHandles<'static>, covers = [], drive = Drive::Never)]
fn connector_tool(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<ConnectorStoryWords>();
    let routings = [("Straight", Routing::Straight), ("Elbow", Routing::Elbow), ("Curved", Routing::Curved)];
    let routing = *data.choice("routing", Routing::Elbow, &routings);
    let targets = board_targets();
    let connectors = data.value("document", Connectors::new(routing)).clone();
    let lines: Vec<BezPath> = connectors
        .items
        .iter()
        .enumerate()
        .filter(|(index, _)| Some(*index) != connectors.selected)
        .filter_map(|(_, item)| {
            let points = route_along(route_end(&targets, &item.from)?, route_end(&targets, &item.to)?, item.routing, item.path.as_ref())?;
            Some(route_line(&points, item.routing))
        })
        .collect();
    let mut hasher = DefaultHasher::new();
    format!("{:?}", connectors.items).hash(&mut hasher);
    let muted = craft_ui::style::rgba_color(Env::of(ui).tokens.color.text.muted);
    let picture = board_picture(ui, &targets, &lines, muted, hasher.finish());
    let body = |ui: &mut Ui, at: egui::Rect, _: &CanvasView, _: &mut ConnectorKept, connectors: &Connectors| {
        let index = connectors.selected?;
        let mut item = connectors.items.get(index)?.clone();
        let shown = ConnectorHandles::new("Connector", Bind::bound(&mut item), &targets).show_at(ui, at);
        let edits: Vec<EditKind> = shown.edit.events.iter().filter_map(EditKind::of).collect();
        Some((index, item, shown.event, edits))
    };
    tool_story(
        ui,
        data,
        ToolStory::new("Document", &picture).buttons(&[StoryButton::new("Select Next")]),
        ConnectorKept::default,
        Connectors::new(routing),
        body,
        |run| {
            let (connectors, kept) = (run.document, run.tool);
            let mut output = StoryOutput::default();
            // A routing picked in the story applies to the selected connector too, as the app's bar does.
            if kept.routing.is_some_and(|was| was != routing)
                && let Some(item) = connectors.selected.and_then(|index| connectors.items.get_mut(index))
            {
                item.routing = routing;
                item.path = None;
            }
            kept.routing = Some(routing);
            if let Some((index, item, event, edits)) = run.shown.flatten() {
                if let Some(stored) = connectors.items.get_mut(index) {
                    *stored = item;
                }
                kept.note = event
                    .map(|event| match event {
                        ConnectorEvent::Reattach { end: super::ConnectorEndKind::Start, .. } => words.reattached_start.as_str().to_owned(),
                        ConnectorEvent::Reattach { .. } => words.reattached_end.as_str().to_owned(),
                        _ => words.reshaped.as_str().to_owned(),
                    })
                    .or(kept.note.take());
                output.edits = edits;
            }
            kept.tool.targets(&targets).routing(routing);
            let _ = run.host.run(&mut kept.tool);
            for event in kept.tool.take_events() {
                let ConnectorEvent::Draw(edit) = event else { continue };
                output.edits.extend(EditKind::of(&edit));
                kept.note = Some(match &edit {
                    EditEvent::Commit { after, .. } => {
                        let id = connectors.items.iter().map(|item| item.id).max().unwrap_or(0) + 1;
                        connectors.items.push(ConnectorItem::new(id, after.from, after.to, after.routing));
                        connectors.selected = Some(connectors.items.len() - 1);
                        words.drew.as_str().to_owned()
                    }
                    EditEvent::Cancel { .. } => words.cancelled.as_str().to_owned(),
                    _ => words.drawing.as_str().to_owned(),
                });
            }
            if run.pressed.is_some() && !connectors.items.is_empty() {
                connectors.selected = Some(connectors.selected.map_or(0, |index| (index + 1) % connectors.items.len()));
                output.clicked = true;
            }
            let event = kept.note.clone().unwrap_or_else(|| words.idle.as_str().to_owned());
            output.readout(craft_i18n::fmt(words.status.as_str(), &[("event", event.as_str())]))
        },
    )
}

/// The routes between two boxes for every pair of sides: a cell per pair (rows are the first box's side, columns the
/// second's), the boxes placed by the layout the story picks.
#[story(ConnectorHandles<'static>, covers = [], drive = Drive::Never)]
fn routes(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let routings = [("Straight", Routing::Straight), ("Elbow", Routing::Elbow), ("Curved", Routing::Curved)];
    let routing = *data.choice("routing", Routing::Elbow, &routings);
    type Place = (f64, f64, f64, f64);
    let layouts: [(&str, (Place, Place)); 3] = [
        ("Diagonal", ((0.08, 0.1, 0.3, 0.3), (0.6, 0.6, 0.3, 0.3))),
        ("Beside", ((0.06, 0.3, 0.26, 0.4), (0.68, 0.3, 0.26, 0.4))),
        ("Overlapping", ((0.2, 0.08, 0.5, 0.42), (0.3, 0.4, 0.5, 0.42))),
    ];
    let (first, second) = *data.choice("layout", layouts[0].1, &layouts);
    let page = StoryDocument::STANDARD.size();
    let cell = Size::new(f64::from(page.width) / 4.0, f64::from(page.height) / 4.0);
    let place = |origin: Vec2, (x, y, w, h): Place| {
        Rect::from_origin_size((origin.x + cell.width * x, origin.y + cell.height * y), Size::new(cell.width * w, cell.height * h))
    };
    let (mut targets, mut lines) = (Vec::new(), Vec::new());
    for (row, &from) in ConnectionSide::ALL.iter().enumerate() {
        for (column, &to) in ConnectionSide::ALL.iter().enumerate() {
            let origin = Vec2::new(cell.width * column as f64, cell.height * row as f64);
            let (a, b) = (place(origin, first), place(origin, second));
            targets.push(ConnectionTarget::new(targets.len() as u64, a));
            targets.push(ConnectionTarget::new(targets.len() as u64, b));
            lines.extend(route(RouteEnd::Box(a, from), RouteEnd::Box(b, to), routing).map(|points| route_line(&points, routing)));
        }
    }
    let ink = craft_ui::style::rgba_color(Env::of(ui).tokens.color.text.normal);
    let picture = board_picture(ui, &targets, &lines, ink, 1);
    on_picture(ui, &picture, |_, _, _| StoryOutput::default())
}
