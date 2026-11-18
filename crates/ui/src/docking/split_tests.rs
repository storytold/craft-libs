//! A split tree of tab groups in an editing layout like FilmCraft's: Source | Program |
//! Properties over Project | Tools | Timeline | Audio Meters, every group an ordinary
//! resizable one. One test per docking rule, the stuck cases of the FilmCraft map, and a
//! property test over random drops.

use std::collections::HashMap;

use super::floating_tests::frame_style;
use super::render::{Drawn, Sizer};
use super::testing::DockDriver;
use super::*;
use crate::layout::{SplitAxis, SplitSize};
use egui::{Modifiers, Pos2, Rect, Vec2, pos2, vec2};

const SOURCE: [u32; 4] = [1, 11, 12, 13];
const PROGRAM: u32 = 2;
const PROPERTIES: [u32; 3] = [3, 31, 32];
const PROJECT: [u32; 7] = [4, 41, 42, 43, 44, 45, 46];
const TOOLS: u32 = 5;
const TIMELINE: u32 = 6;
const METERS: u32 = 7;
const INFO: u32 = 9;

const SCREEN: Vec2 = Vec2::new(1440.0, 860.0);

/// Panel bodies by their rects; `caps` are maximum widths a host asks for, `rules` the
/// permissions of panels that differ from the default.
#[derive(Default)]
struct Film {
    bodies: HashMap<u32, Rect>,
    caps: HashMap<u32, f32>,
    rules: HashMap<u32, Permissions>,
}

impl DockContent<u32> for Film {
    fn body(&mut self, ui: &mut egui::Ui, panel: &u32) {
        self.bodies.insert(*panel, ui.max_rect());
    }
    fn logical_label(&self, panel: &u32) -> String {
        format!("Panel {panel}")
    }
    fn limits(&self, panel: &u32) -> PanelLimits {
        match self.caps.get(panel) {
            Some(width) => PanelLimits { min: Vec2::ZERO, max: vec2(*width, 1_000_000.0) },
            None => PanelLimits::default(),
        }
    }
    fn permissions(&self, panel: &u32) -> Permissions {
        self.rules.get(panel).copied().unwrap_or_default()
    }
}

type Driver = DockDriver<u32, Film>;

fn tabs(panels: &[u32]) -> Node<u32> {
    Node::Tabs { panels: panels.to_vec(), active: 0 }
}

fn split(axis: SplitAxis, ratio: f32, first: Node<u32>, second: Node<u32>) -> Node<u32> {
    Node::Split { axis, size: SplitSize::Ratio(ratio), first: Box::new(first), second: Box::new(second) }
}

fn hsplit(ratio: f32, first: Node<u32>, second: Node<u32>) -> Node<u32> {
    split(SplitAxis::Horizontal, ratio, first, second)
}

/// The Editing workspace, with Info floating over Properties.
fn editing() -> Layout<u32> {
    let top = hsplit(0.69, hsplit(0.5, tabs(&SOURCE), tabs(&[PROGRAM])), tabs(&PROPERTIES));
    let row = hsplit(0.06, tabs(&[TOOLS]), hsplit(0.88, tabs(&[TIMELINE]), tabs(&[METERS])));
    let bottom = hsplit(0.29, tabs(&PROJECT), row);
    Layout {
        root: Some(split(SplitAxis::Vertical, 0.57, top, bottom)),
        floating: vec![Floating::single(vec![INFO], 0, [1120.0, 60.0, 280.0, 200.0])],
    }
}

fn driver(layout: Layout<u32>, content: Film) -> Driver {
    let mut d = DockDriver::new(layout, content).with_style(frame_style);
    d.screen = Rect::from_min_size(Pos2::ZERO, SCREEN);
    d.area = d.screen;
    d.settle();
    d
}

fn film() -> Driver {
    driver(editing(), Film::default())
}

/// A group's rect: its active body and its tab strip.
fn group(d: &Driver, panel: u32) -> Rect {
    let body = d.content.bodies.get(&panel).copied().unwrap_or_else(|| panic!("panel {panel} not drawn"));
    Rect::from_min_max(pos2(body.left(), body.top() - 26.0), body.max)
}

fn tab(d: &Driver, panel: u32) -> Pos2 {
    d.hit(&Hit::Tab(panel)).unwrap_or_else(|| panic!("tab {panel}")).center()
}

/// Drag a divider by `delta`, hovering it first as a pointer does.
fn drag_hit(d: &mut Driver, hit: Hit<u32>, delta: Vec2) {
    let at = d.hit(&hit).unwrap_or_else(|| panic!("{hit:?} in {:?}", d.hits())).center();
    d.move_to(at);
    d.drag(at, at + delta);
    d.settle();
}

fn divider(path: &[bool]) -> Hit<u32> {
    Hit::Divider(Divider::Split(path.to_vec()))
}

fn sizes(d: &Driver, panels: &[u32]) -> Vec<Vec2> {
    panels.iter().map(|p| group(d, *p).size()).collect()
}

fn close(a: Rect, b: Rect) -> bool {
    (a.min - b.min).length() < 1.0 && (a.max - b.max).length() < 1.0
}

#[test]
fn a_group_is_a_picture_frame_whose_edges_split_it_in_half_and_whose_middle_joins_it() {
    let base = film();
    let timeline = group(&base, TIMELINE);
    let others = [PROJECT[0], TOOLS, METERS, SOURCE[0], PROGRAM, PROPERTIES[0]];
    // Halfway into each 40 pt band of the body below the tab strip.
    let inset = |zone| {
        let (w, h) = (20.0, 20.0);
        match zone {
            Zone::Left => pos2(timeline.left() + w, timeline.center().y + 13.0),
            Zone::Right => pos2(timeline.right() - w, timeline.center().y + 13.0),
            Zone::Top => pos2(timeline.center().x, timeline.top() + 26.0 + h),
            Zone::Bottom => pos2(timeline.center().x, timeline.bottom() - h),
            Zone::Center => pos2(timeline.center().x, timeline.center().y + 13.0),
        }
    };
    for zone in [Zone::Left, Zone::Right, Zone::Top, Zone::Bottom] {
        let mut d = film();
        let before = sizes(&d, &others);
        let at = inset(zone);
        d.hold(tab(&d, INFO), at);
        assert!(d.filled_polygon(&zone_shape(timeline, 26.0, 40.0, zone).unwrap()), "{zone:?}: the preview fills that band");
        let out = d.release(at);
        assert_eq!(out.actions, vec![Action::Move { panel: INFO, anchor: TIMELINE, placement: Placement::Split(zone) }], "{zone:?}");
        d.settle();
        let (info, rest) = (group(&d, INFO), group(&d, TIMELINE));
        assert!(close(info.union(rest), timeline), "{zone:?}: the two halves fill the group: {info:?} {rest:?} in {timeline:?}");
        assert!((info.area() - rest.area()).abs() < info.area() * 0.05, "{zone:?}: about halves");
        assert_eq!(sizes(&d, &others), before, "{zone:?}: no other group changes");
    }
    // The middle and the tab row join the group.
    let mut d = film();
    let out = d.drag(tab(&d, INFO), inset(Zone::Center));
    assert_eq!(out.actions, vec![Action::Move { panel: INFO, anchor: TIMELINE, placement: Placement::Split(Zone::Center) }]);
    let mut d = film();
    let out = d.drag(tab(&d, INFO), pos2(timeline.center().x, timeline.top() + 10.0));
    assert!(matches!(&out.actions[..], [Action::Move { panel: INFO, placement: Placement::Tab { .. }, .. }]), "{:?}", out.actions);
    // The gap between two groups belongs to neither: the drag cancels.
    let mut d = film();
    let tools = group(&d, TOOLS);
    let gap = pos2(tools.right() + 2.0, tools.center().y);
    let out = d.drag(tab(&d, INFO), gap);
    assert!(out.actions.is_empty(), "{:?}", out.actions);
}

#[test]
fn window_edges_add_a_full_column_or_row_without_taking_the_groups_edges() {
    let mut d = film();
    let project = group(&d, PROJECT[0]);
    let out = d.drag(tab(&d, INFO), pos2(3.0, project.center().y));
    assert_eq!(out.actions, vec![Action::Move { panel: INFO, anchor: SOURCE[0], placement: Placement::Edge(Zone::Left) }]);
    d.settle();
    let info = group(&d, INFO);
    assert_eq!((info.top(), info.bottom(), info.left()), (0.0, SCREEN.y, 0.0), "a full-height column at the left");
    assert!((info.width() - SCREEN.x * 0.25).abs() < 4.0, "{info:?}");

    for (at, zone) in [(pos2(SCREEN.x - 3.0, 300.0), Zone::Right), (pos2(700.0, 3.0), Zone::Top), (pos2(700.0, SCREEN.y - 3.0), Zone::Bottom)] {
        let mut d = film();
        let out = d.drag(tab(&d, INFO), at);
        assert!(matches!(&out.actions[..], [Action::Move { placement: Placement::Edge(z), .. }] if *z == zone), "{zone:?}: {:?}", out.actions);
    }
    // Past the window band, the group's own edge band splits that group.
    let mut d = film();
    let out = d.drag(tab(&d, INFO), pos2(project.left() + 20.0, project.center().y + 13.0));
    assert_eq!(out.actions, vec![Action::Move { panel: INFO, anchor: PROJECT[0], placement: Placement::Split(Zone::Left) }]);
}

#[test]
fn groups_have_one_small_minimum_and_no_maximum() {
    let mut d = film();
    let tools = group(&d, TOOLS).width();
    drag_hit(&mut d, divider(&[true, true]), vec2(400.0, 0.0));
    assert!((group(&d, TOOLS).width() - (tools + 400.0)).abs() < 1.0, "Tools grows as wide as it is dragged");
    drag_hit(&mut d, divider(&[true, true]), vec2(-2000.0, 0.0));
    assert_eq!(group(&d, TOOLS).width(), 40.0, "and shrinks to the minimum, no further");

    // At its minimum, with Project left of it at its own, it has no room for two halves and
    // takes a drop on its left edge as a tab.
    let tools = group(&d, TOOLS);
    assert_eq!(group(&d, PROJECT[0]).width(), 40.0);
    let out = d.drag(tab(&d, INFO), pos2(tools.left() + 4.0, tools.center().y + 30.0));
    assert!(
        matches!(&out.actions[..], [Action::Move { panel: INFO, anchor: TOOLS, placement: Placement::Split(Zone::Center) | Placement::Tab { .. } }]),
        "{:?}",
        out.actions
    );
}

#[test]
fn a_divider_resizes_only_the_groups_that_share_it() {
    // Timeline | Meters: Project and Tools keep their widths.
    let mut d = film();
    let (project, tools, timeline, meters) = (group(&d, PROJECT[0]), group(&d, TOOLS), group(&d, TIMELINE), group(&d, METERS));
    drag_hit(&mut d, divider(&[true, true, true]), vec2(-100.0, 0.0));
    assert_eq!((group(&d, PROJECT[0]), group(&d, TOOLS)), (project, tools));
    assert!((group(&d, TIMELINE).width() - (timeline.width() - 100.0)).abs() < 1.0);
    assert!((group(&d, METERS).width() - (meters.width() + 100.0)).abs() < 1.0);

    // Project | the rest: only Tools, on the other side, gives way; Timeline and Meters stay.
    let mut d = film();
    drag_hit(&mut d, divider(&[true]), vec2(15.0, 0.0));
    assert!((group(&d, PROJECT[0]).width() - (project.width() + 15.0)).abs() < 1.0);
    assert!((group(&d, TOOLS).width() - (tools.width() - 15.0)).abs() < 1.0);
    assert!(close(group(&d, TIMELINE), timeline) && close(group(&d, METERS), meters), "{:?}", d.layout);

    // Top row: Source | Program leaves Properties alone.
    let mut d = film();
    let properties = group(&d, PROPERTIES[0]);
    drag_hit(&mut d, divider(&[false, false]), vec2(80.0, 0.0));
    assert_eq!(group(&d, PROPERTIES[0]), properties);
}

#[test]
fn a_divider_dragged_past_a_groups_minimum_shrinks_the_next_group_along_the_row() {
    // A | B | C | D, 357 pt each, as (A | B) | (C | D): the cascade runs on past B's split.
    let row = hsplit(0.5, hsplit(0.5, tabs(&[1]), tabs(&[2])), hsplit(0.5, tabs(&[3]), tabs(&[4])));
    let mut d = driver(Layout { root: Some(row), floating: Vec::new() }, Film::default());
    let widths = |d: &Driver| [1, 2, 3, 4].map(|panel| group(d, panel).width().round());
    assert_eq!(widths(&d), [357.0, 357.0, 357.0, 357.0]);
    // B gives way down to its 40 pt minimum, then C.
    drag_hit(&mut d, divider(&[false]), vec2(400.0, 0.0));
    assert_eq!(widths(&d), [757.0, 40.0, 274.0, 357.0]);
    // The drag stops when B, C and D are all at their minimums.
    drag_hit(&mut d, divider(&[false]), vec2(670.0, 0.0));
    assert_eq!(widths(&d), [1308.0, 40.0, 40.0, 40.0]);
    // Back the other way, A gives way and only B, which touches the divider, grows.
    drag_hit(&mut d, divider(&[false]), vec2(-1300.0, 0.0));
    assert_eq!(widths(&d), [40.0, 1308.0, 40.0, 40.0]);
}

#[test]
fn a_divider_drag_keeps_the_splits_fixed_or_ratio_size() {
    let hsized = |size, first, second| Node::Split { axis: SplitAxis::Horizontal, size, first: Box::new(first), second: Box::new(second) };
    let start = |d: &Driver| match &d.layout.root {
        Some(Node::Split { size, .. }) => *size,
        other => panic!("{other:?}"),
    };
    let wider = |d: &mut Driver| {
        d.screen = Rect::from_min_size(Pos2::ZERO, SCREEN + vec2(400.0, 0.0));
        d.area = d.screen;
        d.settle();
    };
    let layout = |root| Layout { root: Some(root), floating: Vec::new() };

    // A fixed start panel keeps its dragged width when the window grows.
    let mut d = driver(layout(hsized(SplitSize::FixedFirst(240.0), tabs(&[1]), tabs(&[2]))), Film::default());
    drag_hit(&mut d, divider(&[]), vec2(60.0, 0.0));
    let SplitSize::FixedFirst(width) = start(&d) else { panic!("{:?}", d.layout) };
    assert!((width - 300.0).abs() < 1.0, "{width}");
    let panel = group(&d, 1).width();
    wider(&mut d);
    assert!((group(&d, 1).width() - panel).abs() < 1.0, "{} {panel}", group(&d, 1).width());

    // A fixed end panel too.
    let mut d = driver(layout(hsized(SplitSize::FixedSecond(240.0), tabs(&[1]), tabs(&[2]))), Film::default());
    drag_hit(&mut d, divider(&[]), vec2(60.0, 0.0));
    let SplitSize::FixedSecond(width) = start(&d) else { panic!("{:?}", d.layout) };
    assert!((width - 180.0).abs() < 1.0, "{width}");
    let panel = group(&d, 2).width();
    wider(&mut d);
    assert!((group(&d, 2).width() - panel).abs() < 1.0, "{} {panel}", group(&d, 2).width());

    // A ratio stays a ratio, and both sides grow with the window.
    let mut d = driver(layout(hsplit(0.5, tabs(&[1]), tabs(&[2]))), Film::default());
    drag_hit(&mut d, divider(&[]), vec2(60.0, 0.0));
    let SplitSize::Ratio(ratio) = start(&d) else { panic!("{:?}", d.layout) };
    assert!(ratio > 0.5 && ratio < 1.0, "{ratio}");
    let (first, second) = (group(&d, 1).width(), group(&d, 2).width());
    wider(&mut d);
    assert!(group(&d, 1).width() > first + 100.0 && group(&d, 2).width() > second + 100.0);

    // A drag that runs on past its own split into a fixed ancestor keeps the ancestor fixed.
    let mut d = driver(layout(hsized(SplitSize::FixedSecond(300.0), hsplit(0.5, tabs(&[1]), tabs(&[2])), tabs(&[3]))), Film::default());
    drag_hit(&mut d, divider(&[false]), vec2(600.0, 0.0));
    let SplitSize::FixedSecond(width) = start(&d) else { panic!("{:?}", d.layout) };
    assert!(width < 299.0, "{width}");
    let panel = group(&d, 3).width();
    wider(&mut d);
    assert!((group(&d, 3).width() - panel).abs() < 1.0, "{} {panel}", group(&d, 3).width());
}

#[test]
fn an_intersection_moves_both_dividers() {
    let mut d = film();
    let (project, tools, source) = (group(&d, PROJECT[0]), group(&d, TOOLS), group(&d, SOURCE[0]));
    let hit = Hit::Divider(Divider::Intersection(vec![], vec![true]));
    let at = d.hit(&hit).unwrap_or_else(|| panic!("{:?}", d.hits()));
    assert!(at.contains(pos2(project.right() + 2.0, project.top() - 2.0)), "{at:?} at Project's top right corner");
    drag_hit(&mut d, hit, vec2(15.0, -60.0));
    assert!((group(&d, PROJECT[0]).width() - (project.width() + 15.0)).abs() < 1.0);
    assert!((group(&d, PROJECT[0]).height() - (project.height() + 60.0)).abs() < 1.0);
    assert!((group(&d, TOOLS).width() - (tools.width() - 15.0)).abs() < 1.0);
    assert!((group(&d, SOURCE[0]).height() - (source.height() - 60.0)).abs() < 1.0);
    assert_eq!(group(&d, SOURCE[0]).width(), source.width());
}

#[test]
fn closing_a_group_gives_its_space_to_its_neighbours() {
    let mut d = film();
    let (tools, meters) = (group(&d, TOOLS), group(&d, METERS));
    d.layout.apply(Action::CloseGroup { panel: TIMELINE }).unwrap();
    d.settle();
    let meters_now = group(&d, METERS);
    assert_eq!(group(&d, TOOLS), tools);
    assert_eq!((meters_now.right(), meters_now.top(), meters_now.bottom()), (meters.right(), meters.top(), meters.bottom()));
    assert!((meters_now.left() - (tools.right() + 4.0)).abs() < 1.0, "the row has no hole: {meters_now:?}");
}

// The stuck cases of the FilmCraft map (map.md section 3).

#[test]
fn info_dropped_onto_a_capped_group_lifts_the_cap() {
    let content = Film { caps: HashMap::from([(METERS, 116.0)]), ..Film::default() };
    let mut d = driver(editing(), content);
    let meters = group(&d, METERS);
    d.drag(tab(&d, INFO), pos2(meters.center().x, meters.center().y + 13.0));
    d.settle();
    assert!(d.layout.location(&INFO).is_ok_and(|l| l.anchor == Some(METERS)), "{:?}", d.layout);
    // Info is the group's shown panel.
    let width = group(&d, INFO).width();
    drag_hit(&mut d, divider(&[true, true, true]), vec2(-200.0, 0.0));
    assert!((group(&d, INFO).width() - (width + 200.0)).abs() < 1.0, "the group grows past the cap of one of its panels");
    drag_hit(&mut d, divider(&[true, true, true]), vec2(150.0, 0.0));
    assert!((group(&d, INFO).width() - (width + 50.0)).abs() < 1.0, "and shrinks again");
}

#[test]
fn a_drop_beside_a_narrow_group_splits_that_group_only() {
    let mut d = film();
    drag_hit(&mut d, divider(&[true, true]), vec2(80.0, 0.0));
    let (tools, project, timeline, meters) = (group(&d, TOOLS), group(&d, PROJECT[0]), group(&d, TIMELINE), group(&d, METERS));
    assert!(tools.width() > 120.0, "{tools:?}");
    let at = pos2(tools.right() - 8.0, tools.center().y + 13.0);
    d.hold(tab(&d, INFO), at);
    assert!(d.filled_polygon(&zone_shape(tools, 26.0, 40.0, Zone::Right).unwrap()), "the preview fills Tools' right band");
    let out = d.release(at);
    assert_eq!(out.actions, vec![Action::Move { panel: INFO, anchor: TOOLS, placement: Placement::Split(Zone::Right) }]);
    d.settle();
    assert!(close(group(&d, TOOLS).union(group(&d, INFO)), tools));
    assert_eq!((group(&d, PROJECT[0]), group(&d, TIMELINE), group(&d, METERS)), (project, timeline, meters));
}

/// Properties and a narrow Tools frame side by side, Tools `tools` wide in an area `width`
/// wide, with Info floating at the left.
fn beside_tools(width: f32, tools: f32) -> Driver {
    let root = Node::Split {
        axis: SplitAxis::Horizontal,
        size: SplitSize::FixedSecond(tools),
        first: Box::new(tabs(&PROPERTIES)),
        second: Box::new(tabs(&[TOOLS])),
    };
    let layout = Layout { root: Some(root), floating: vec![Floating::single(vec![INFO], 0, [width + 100.0, 100.0, 280.0, 200.0])] };
    let mut d = driver(layout, Film::default());
    d.area = Rect::from_min_size(Pos2::ZERO, vec2(width, SCREEN.y));
    d.settle();
    d
}

#[test]
fn a_drop_beside_a_frame_too_narrow_to_halve_takes_room_from_its_neighbour() {
    let mut d = beside_tools(1000.0, 60.0);
    let (tools, properties, info) = (group(&d, TOOLS), group(&d, PROPERTIES[0]), group(&d, INFO));
    let left = zone_shape(tools, 26.0, 40.0, Zone::Left).unwrap();
    let at = pos2(tools.left() + 4.0, tools.center().y + 13.0);
    d.hold(tab(&d, INFO), at);
    assert!(d.filled_polygon(&left), "Tools' left zone is a target");
    let out = d.release(at);
    assert!(
        matches!(&out.actions[..], [Action::Move { panel: INFO, anchor: TOOLS, placement: Placement::SplitSized { zone: Zone::Left, .. } }]),
        "{:?}",
        out.actions
    );
    d.settle();
    let (new_tools, new_info, new_properties) = (group(&d, TOOLS), group(&d, INFO), group(&d, PROPERTIES[0]));
    assert!((new_tools.width() - tools.width()).abs() < 1.0, "Tools keeps its width: {new_tools:?}");
    assert!((new_info.width() - info.width()).abs() < 1.0, "Info keeps its floating width: {new_info:?}");
    assert!(new_info.right() < new_tools.left() && new_properties.right() < new_info.left());
    assert!(new_properties.width() < properties.width() - 200.0, "Properties gave the room");
}

/// Tools first in its row, at the area's left edge: its left side has no divider, so the room
/// comes from beyond its right one.
#[test]
fn a_drop_on_the_outer_side_of_a_frame_too_narrow_to_halve_takes_room_from_the_other_side() {
    let root = Node::Split {
        axis: SplitAxis::Horizontal,
        size: SplitSize::FixedFirst(60.0),
        first: Box::new(tabs(&[TOOLS])),
        second: Box::new(tabs(&PROPERTIES)),
    };
    let layout = Layout { root: Some(root), floating: vec![Floating::single(vec![INFO], 0, [1100.0, 100.0, 280.0, 200.0])] };
    let mut d = driver(layout, Film::default());
    d.area = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, SCREEN.y));
    d.settle();
    let (tools, properties, info) = (group(&d, TOOLS), group(&d, PROPERTIES[0]), group(&d, INFO));
    // Inside Tools' left zone, past the window edge band.
    let at = pos2(tools.left() + 12.0, tools.center().y + 13.0);
    d.hold(tab(&d, INFO), at);
    assert!(d.filled_polygon(&zone_shape(tools, 26.0, 40.0, Zone::Left).unwrap()), "Tools' left zone is a target");
    let out = d.release(at);
    assert!(
        matches!(&out.actions[..], [Action::Move { panel: INFO, anchor: TOOLS, placement: Placement::SplitSized { zone: Zone::Left, .. } }]),
        "{:?}",
        out.actions
    );
    d.settle();
    let (new_tools, new_info, new_properties) = (group(&d, TOOLS), group(&d, INFO), group(&d, PROPERTIES[0]));
    assert!((new_tools.width() - tools.width()).abs() < 1.0, "Tools keeps its width: {new_tools:?}");
    assert!((new_info.width() - info.width()).abs() < 1.0, "Info keeps its floating width: {new_info:?}");
    assert!(new_info.right() < new_tools.left() && new_tools.right() < new_properties.left());
    assert!(new_properties.width() < properties.width() - 200.0, "Properties gave the room");
}

#[test]
fn a_docked_panel_dropped_beside_a_frame_too_narrow_to_halve_takes_its_minimum_width() {
    // Info docked below Properties, as wide as it, beside a narrow Tools frame.
    let root = Node::Split {
        axis: SplitAxis::Horizontal,
        size: SplitSize::FixedSecond(60.0),
        first: Box::new(split(SplitAxis::Vertical, 0.6, tabs(&PROPERTIES), tabs(&[INFO]))),
        second: Box::new(tabs(&[TOOLS])),
    };
    let mut d = driver(Layout { root: Some(root), floating: Vec::new() }, Film::default());
    d.area = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, SCREEN.y));
    d.settle();
    let (tools, properties) = (group(&d, TOOLS), group(&d, PROPERTIES[0]));
    let out = d.drag(tab(&d, INFO), pos2(tools.left() + 4.0, tools.center().y + 13.0));
    assert!(
        matches!(&out.actions[..], [Action::Move { panel: INFO, anchor: TOOLS, placement: Placement::SplitSized { zone: Zone::Left, .. } }]),
        "{:?}",
        out.actions
    );
    d.settle();
    let (new_tools, new_info, new_properties) = (group(&d, TOOLS), group(&d, INFO), group(&d, PROPERTIES[0]));
    assert!((new_tools.width() - tools.width()).abs() < 1.0, "Tools keeps its width: {new_tools:?}");
    assert!(new_info.width() < 60.0, "Info takes its minimum, not its docked width: {new_info:?}");
    assert!(new_properties.width() > properties.width() - 60.0, "Properties gives only that: {new_properties:?}");
}

#[test]
fn a_drop_beside_a_narrow_frame_joins_it_when_no_neighbour_has_room() {
    let mut d = beside_tools(130.0, 60.0);
    let tools = group(&d, TOOLS);
    let at = pos2(tools.left() + 4.0, tools.center().y + 13.0);
    let out = d.drag(tab(&d, INFO), at);
    assert!(
        matches!(&out.actions[..], [Action::Move { panel: INFO, anchor: TOOLS, placement: Placement::Tab { .. } | Placement::Split(Zone::Center) }]),
        "{:?}",
        out.actions
    );
}

#[test]
fn every_divider_travels_after_drops() {
    let mut d = film();
    let tools = group(&d, TOOLS);
    d.drag(tab(&d, INFO), pos2(tools.right() - 8.0, tools.center().y + 13.0));
    d.settle();
    let meters = group(&d, METERS);
    d.drag(tab(&d, PROGRAM), pos2(meters.center().x, meters.center().y + 13.0));
    d.settle();
    let timeline = group(&d, TIMELINE);
    d.drag(tab(&d, PROPERTIES[0]), pos2(timeline.center().x, timeline.bottom() - 10.0));
    d.settle();
    assert_dividers_travel(&mut d);
}

/// Each split divider, in the docked layout and in floating windows, moves both ways when
/// dragged, from the layout as it is.
fn assert_dividers_travel(d: &mut Driver) {
    let start = d.layout.clone();
    let dividers: Vec<Hit<u32>> = d
        .hits()
        .into_iter()
        .filter_map(|(hit, _)| matches!(hit, Hit::Divider(Divider::Split(_) | Divider::WindowSplit(..))).then_some(hit))
        .collect();
    assert!(!dividers.is_empty());
    // A floating window may cover the dividers of the main area and of other windows.
    let covers: Vec<(Rect, &Floating<u32>)> = start
        .floating
        .iter()
        .map(|w| {
            let drawn = w.panels().filter(|p| d.content.bodies.contains_key(p)).map(|p| group(d, *p));
            let rect = Rect::from_min_size(pos2(w.rect[0], w.rect[1]), vec2(w.rect[2], w.rect[3]));
            (drawn.fold(rect, |a, b| a.union(b)), w)
        })
        .collect();
    for hit in dividers {
        let own = |w: &Floating<u32>| matches!(&hit, Hit::Divider(Divider::WindowSplit(key, _)) if w.contains(key));
        if d.hit(&hit).is_some_and(|r| covers.iter().any(|(c, w)| c.intersects(r) && !own(w))) {
            continue;
        }
        for step in [12.0, -24.0] {
            let Some(rect) = d.hit(&hit) else { continue };
            let delta = if rect.width() < rect.height() { vec2(step, 0.0) } else { vec2(0.0, step) };
            let before = d.layout.clone();
            drag_hit(d, hit.clone(), delta);
            assert_ne!(d.layout, before, "divider {hit:?} does not move by {step}");
        }
        d.layout = start.clone();
        d.settle();
    }
}

/// Every group at or above its minimum, and every divider with room both ways, in the docked
/// layout and in each floating window. Reads the bodies drawn last.
fn assert_sizes(d: &Driver, context: &str) {
    let mut style = DockStyle::default();
    frame_style(&mut style);
    let sizer = Sizer { style: &style, content: &d.content };
    let mut trees: Vec<(&Node<u32>, Rect)> = d.layout.root.iter().map(|root| (root, d.area)).collect();
    for window in d.layout.floating.iter().filter(|w| !w.minimized) {
        // The window's body is the union of its groups.
        let drawn = window.panels().filter(|p| d.content.bodies.contains_key(p)).map(|p| group(d, *p));
        if let Some(rect) = drawn.reduce(|a, b| a.union(b)) {
            trees.push((&window.root, rect));
        }
    }
    for (root, area) in trees {
        let mut groups: Vec<Drawn<'_, u32>> = Vec::new();
        sizer.groups(root, area, &mut groups);
        for g in &groups {
            let room = g.rect.size() - g.limits.min;
            assert!(room.x >= -0.01 && room.y >= -0.01, "{context}: {:?} is below its minimum {:?}", g.rect, g.limits.min);
        }
        let mut nodes = vec![(root, area, Vec::new())];
        while let Some((node, rect, path)) = nodes.pop() {
            let Some([a, b]) = sizer.children(node, rect) else { continue };
            let (lo, hi) = sizer.range((root, area), &path);
            assert!(lo < -0.5 && hi > 0.5, "{context}: a divider is stuck ({lo}, {hi}) at {rect:?} in {:?}", d.layout);
            for (second, (child, rect)) in [(false, a), (true, b)] {
                let mut path = path.clone();
                path.push(second);
                nodes.push((child, rect, path));
            }
        }
    }
}

/// A small deterministic generator, so a failing sequence repeats.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn unit(&mut self) -> f32 {
        (self.next() % 1000) as f32 / 1000.0
    }
}

#[test]
fn random_drops_leave_every_group_its_minimum_and_every_divider_travel() {
    // Drops land in the docked layout and in two floating windows, one of them split; tabs and
    // drag bars move panels and groups between all three, and Ctrl/Cmd floats some.
    let windows = || {
        let mut layout = editing();
        layout.floating.push(Floating::new(hsplit(0.5, tabs(&[8]), tabs(&[50, 51])), [80.0, 420.0, 640.0, 380.0]));
        layout
    };
    for seed in 1..=6u64 {
        let mut rng = Lcg(seed);
        let mut d = driver(windows(), Film::default());
        let all = panel_set(&d.layout);
        for step in 0..16 {
            // Only the bodies drawn now.
            d.content.bodies.clear();
            d.settle();
            let panels: Vec<u32> = d.layout.panels().into_iter().copied().collect();
            let panel = panels[rng.below(panels.len())];
            let bar = d.hit(&Hit::DragBar(panel)).filter(|_| rng.below(3) == 0);
            let Some(from) = bar.or_else(|| d.hit(&Hit::Tab(panel))).map(|r| r.center()) else { continue };
            let target = panels[rng.below(panels.len())];
            let Some(rect) = d.content.bodies.get(&target).copied() else { continue };
            let to = match rng.below(8) {
                // A window edge.
                0 => {
                    let edges = [
                        pos2(3.0, rect.center().y),
                        pos2(SCREEN.x - 3.0, rect.center().y),
                        pos2(rect.center().x, 3.0),
                        pos2(rect.center().x, SCREEN.y - 3.0),
                    ];
                    edges[rng.below(4)]
                }
                // The tab row.
                1 => pos2(rect.center().x, rect.top() - 13.0),
                // Anywhere in the body: an edge band or the middle.
                _ => rect.lerp_inside(vec2(0.02 + rng.unit() * 0.96, 0.02 + rng.unit() * 0.96)),
            };
            if rng.below(12) == 0 {
                d.modifiers = Modifiers::COMMAND;
            }
            d.drag(from, to);
            d.modifiers = Modifiers::NONE;
            d.content.bodies.clear();
            d.settle();
            let context = format!("seed {seed}, step {step}");
            assert_eq!(d.layout.validate(), Ok(()), "{context}");
            assert_eq!(panel_set(&d.layout), all, "{context}: a panel was lost");
            assert_sizes(&d, &context);
        }
        assert_dividers_travel(&mut d);
    }
}

#[test]
fn ctrl_or_cmd_floats_from_a_tab_or_a_drag_bar_and_the_preview_says_so() {
    // The hint names the platform's key, as `ctx.os()` reports it in tests.
    let key = if egui::os::OperatingSystem::from_target_os() == egui::os::OperatingSystem::Mac { "⌘" } else { "Ctrl" };
    let hint = format!("Hold {key} to undock");
    let floated = |actions: &[Action<u32>], panel: u32| match actions {
        [Action::Float { panel: p, .. }] => *p == panel,
        [Action::MoveGroup { panel: p, to: Target::Float { .. } }] => *p == panel,
        _ => false,
    };
    fn texts(d: &Driver) -> Vec<String> {
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
                egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
                _ => {}
            }
        }
        let mut out = Vec::new();
        d.shapes().iter().for_each(|clipped| walk(&clipped.shape, &mut out));
        out
    }
    let d = film();
    let over = group(&d, TIMELINE).center() + vec2(0.0, 13.0);
    let from_tab = tab(&d, PROJECT[0]);
    let from_bar = d.hit(&Hit::DragBar(PROJECT[0])).unwrap().center();
    for from in [from_tab, from_bar] {
        // Without the modifier, a release over a group docks there; the preview names the modifier.
        let mut d = film();
        d.hold(from, over);
        assert!(texts(&d).contains(&hint), "{from:?}: {:?}", texts(&d));
        let out = d.release(over);
        assert!(!out.actions.is_empty() && !floated(&out.actions, PROJECT[0]), "{from:?}: {:?}", out.actions);
        // Held, it floats, and the hint is gone.
        let mut d = film();
        d.modifiers = Modifiers::COMMAND;
        d.hold(from, over);
        assert!(!texts(&d).contains(&hint), "{from:?}");
        let out = d.release(over);
        assert!(floated(&out.actions, PROJECT[0]), "{from:?}: {:?}", out.actions);
    }
}

/// The Editing workspace under a 24 pt menu bar and above a 20 pt status bar.
fn film_in_window() -> Driver {
    let mut d = DockDriver::new(editing(), Film::default()).with_style(frame_style);
    d.screen = Rect::from_min_size(Pos2::ZERO, SCREEN);
    d.area = Rect::from_min_max(pos2(0.0, 24.0), pos2(SCREEN.x, SCREEN.y - 20.0));
    d.settle();
    d
}

#[test]
fn a_release_over_no_target_inside_the_window_cancels_the_drag() {
    let d = film_in_window();
    let (source, program) = (group(&d, SOURCE[0]), group(&d, PROGRAM));
    let gap = pos2((source.right() + program.left()) * 0.5, source.center().y);
    let from_tab = tab(&d, PROJECT[0]);
    let from_bar = d.hit(&Hit::DragBar(PROJECT[0])).unwrap().center();
    for from in [from_tab, from_bar] {
        for to in [pos2(600.0, 10.0), pos2(600.0, SCREEN.y - 8.0), gap] {
            let mut d = film_in_window();
            let before = d.layout.clone();
            let out = d.drag(from, to);
            d.settle();
            assert!(out.actions.is_empty(), "{from:?} to {to:?}: {:?}", out.actions);
            assert_eq!(d.layout, before, "{from:?} to {to:?}");
        }
        // Ctrl/Cmd held floats it there.
        let mut d = film_in_window();
        d.modifiers = Modifiers::COMMAND;
        d.drag(from, pos2(600.0, 10.0));
        d.settle();
        assert!(d.layout.floating.iter().any(|w| w.contains(&PROJECT[0])), "{from:?}: {:?}", d.layout.floating);
    }
}

#[test]
fn a_release_outside_the_window_floats() {
    let d = film_in_window();
    let from_tab = tab(&d, PROJECT[0]);
    let from_bar = d.hit(&Hit::DragBar(PROJECT[0])).unwrap().center();
    for from in [from_tab, from_bar] {
        let mut d = film_in_window();
        d.hold(from, pos2(600.0, 10.0));
        d.leave();
        d.settle();
        let window = d.layout.floating.iter().find(|w| w.contains(&PROJECT[0]));
        assert!(window.is_some(), "{from:?}: {:?}", d.layout);
        // A tab takes one panel; a drag bar takes the group.
        assert_eq!(window.unwrap().contains(&PROJECT[1]), from == from_bar, "{from:?}");
    }
}

#[test]
fn the_title_bar_moves_a_floating_window_without_docking() {
    let mut d = film();
    let title = d.hit(&Hit::Title(INFO)).unwrap().center();
    let timeline = group(&d, TIMELINE);
    d.drag(title, timeline.center());
    d.settle();
    let window = d.layout.floating.iter().find(|w| w.contains(&INFO)).expect("still floating");
    assert!(window.rect[0] < 1000.0 && window.rect[1] > 400.0, "moved towards the drop: {:?}", window.rect);
    assert_eq!(d.layout.root, editing().root);
}

fn panel_set(layout: &Layout<u32>) -> Vec<u32> {
    let mut panels: Vec<u32> = layout.panels().into_iter().copied().collect();
    panels.sort_unstable();
    panels
}

fn window_of(layout: &Layout<u32>, panel: u32) -> &Floating<u32> {
    layout.floating.iter().find(|w| w.contains(&panel)).unwrap_or_else(|| panic!("{panel} does not float: {layout:?}"))
}

#[test]
fn tabs_and_drag_bars_dock_into_a_floating_window_and_out_of_it() {
    let mut d = film();
    let all = panel_set(&d.layout);
    let info = group(&d, INFO);
    let out = d.drag(tab(&d, PROGRAM), pos2(info.left() + 10.0, info.center().y + 13.0));
    assert_eq!(out.actions, vec![Action::Move { panel: PROGRAM, anchor: INFO, placement: Placement::Split(Zone::Left) }]);
    d.settle();
    assert!(matches!(window_of(&d.layout, INFO).root, Node::Split { axis: SplitAxis::Horizontal, .. }), "{:?}", d.layout.floating);
    let (program, info) = (group(&d, PROGRAM), group(&d, INFO));
    assert!(program.right() < info.left() && (program.width() - info.width()).abs() < 2.0, "{program:?} {info:?}");

    // A drag bar inside the window moves its group to a group of the main area.
    let timeline = group(&d, TIMELINE);
    let bar = d.hit(&Hit::DragBar(INFO)).unwrap().center();
    let out = d.drag(bar, timeline.center() + vec2(0.0, 13.0));
    assert_eq!(out.actions, vec![Action::MoveGroup { panel: INFO, to: Target::At { anchor: TIMELINE, placement: Placement::Split(Zone::Center) } }]);
    d.settle();
    assert_eq!(window_of(&d.layout, PROGRAM).root, tabs(&[PROGRAM]));
    assert_eq!(panel_set(&d.layout), all);
}

#[test]
fn a_floating_window_tree_resizes_by_its_dividers_and_intersections() {
    let mut layout = editing();
    let tree = hsplit(0.5, tabs(&[INFO]), split(SplitAxis::Vertical, 0.5, tabs(&[50]), tabs(&[51])));
    layout.floating = vec![Floating::new(tree, [700.0, 100.0, 600.0, 500.0])];
    let mut d = driver(layout, Film::default());
    let ratios = |d: &Driver| match &d.layout.floating[0].root {
        Node::Split { size: SplitSize::Ratio(outer), second, .. } => match &**second {
            Node::Split { size: SplitSize::Ratio(inner), .. } => (*outer, *inner),
            other => panic!("{other:?}"),
        },
        other => panic!("{other:?}"),
    };
    let outer = Hit::Divider(Divider::WindowSplit(INFO, Vec::new()));
    let inner = Hit::Divider(Divider::WindowSplit(INFO, vec![true]));
    for (hit, delta) in [(&outer, vec2(30.0, 0.0)), (&outer, vec2(-60.0, 0.0)), (&inner, vec2(0.0, 30.0)), (&inner, vec2(0.0, -60.0))] {
        let before = ratios(&d);
        drag_hit(&mut d, hit.clone(), delta);
        let after = ratios(&d);
        let moved = if hit == &outer { (after.0 - before.0) * delta.x } else { (after.1 - before.1) * delta.y };
        assert!(moved > 0.0, "{hit:?} by {delta:?}: {before:?} to {after:?}");
        assert_eq!(d.layout.root, editing().root, "the main area keeps its sizes");
    }
    // The four-way handle where the inner divider meets the outer one moves both.
    let before = ratios(&d);
    drag_hit(&mut d, Hit::Divider(Divider::WindowIntersection(INFO, Vec::new(), vec![true])), vec2(25.0, 25.0));
    let after = ratios(&d);
    assert!(after.0 > before.0 && after.1 > before.1, "{before:?} to {after:?}");
}

#[test]
fn moves_between_windows_are_one_action_and_lose_no_panel() {
    let mut layout = editing();
    layout.floating.push(Floating::single(vec![50, 51], 0, [200.0, 520.0, 320.0, 260.0]));
    let mut d = driver(layout, Film::default());
    let all = panel_set(&d.layout);
    // Floating to floating: a whole group, by its drag bar, beside Info.
    let info = group(&d, INFO);
    let bar = d.hit(&Hit::DragBar(50)).unwrap().center();
    let out = d.drag(bar, pos2(info.right() - 10.0, info.center().y + 13.0));
    assert_eq!(out.actions, vec![Action::MoveGroup { panel: 50, to: Target::At { anchor: INFO, placement: Placement::Split(Zone::Right) } }]);
    d.settle();
    assert_eq!(d.layout.floating.len(), 1);
    assert_eq!(panel_set(&d.layout), all);
    // Main to floating: a tab joins the window's group.
    let fifty = group(&d, 50);
    let out = d.drag(tab(&d, SOURCE[1]), fifty.center() + vec2(0.0, 13.0));
    assert_eq!(out.actions, vec![Action::Move { panel: SOURCE[1], anchor: 50, placement: Placement::Split(Zone::Center) }]);
    d.settle();
    assert!(window_of(&d.layout, 50).contains(&SOURCE[1]));
    assert_eq!(panel_set(&d.layout), all);
    assert_eq!(d.layout.validate(), Ok(()));
}

#[test]
fn a_closed_floating_window_closes_its_panels_and_each_reopens_where_it_was() {
    let mut layout = editing();
    let tree = hsplit(0.4, tabs(&[INFO]), tabs(&[50]));
    layout.floating = vec![Floating::new(tree.clone(), [700.0, 100.0, 600.0, 400.0])];
    let mut d = driver(layout, Film::default());
    let locations: Vec<_> = [INFO, 50].into_iter().map(|p| (p, d.layout.location(&p).unwrap())).collect();
    let close = d.hit(&Hit::Close(INFO)).unwrap().center();
    let out = d.click(close);
    assert_eq!(out.actions, vec![Action::CloseWindow { panel: INFO }]);
    assert!(d.layout.floating.is_empty() && !d.layout.contains(&50));
    for (panel, location) in &locations {
        d.layout.restore(*panel, location).unwrap();
    }
    let window = &d.layout.floating[0];
    assert_eq!((&window.root, window.rect), (&tree, [700.0, 100.0, 600.0, 400.0]));
}

/// The zone outlines of the last frame: closed 1 pt paths without a fill.
fn zone_outlines(d: &Driver) -> Vec<Vec<Pos2>> {
    d.shapes()
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Path(path) if path.closed && path.fill == egui::Color32::TRANSPARENT && path.stroke.width == 1.0 => {
                Some(path.points.clone())
            }
            _ => None,
        })
        .collect()
}

fn same_polygon(a: &[Pos2], b: &[Pos2]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.distance(*b) <= 0.5)
}

fn corners(rect: Rect) -> Vec<Pos2> {
    vec![rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()]
}

#[test]
fn the_preview_fills_the_zone_under_the_pointer_and_no_result_rect() {
    let d = film();
    let timeline = group(&d, TIMELINE);
    let strip = Rect::from_min_size(timeline.min, vec2(timeline.width(), 26.0));
    // A side band: its trapezoid, without the half the panel takes.
    for (zone, at) in
        [(Zone::Left, pos2(timeline.left() + 8.0, timeline.center().y + 13.0)), (Zone::Bottom, pos2(timeline.center().x, timeline.bottom() - 20.0))]
    {
        let mut d = film();
        d.hold(tab(&d, INFO), at);
        assert!(d.filled_polygon(&zone_shape(timeline, 26.0, 40.0, zone).unwrap()), "{zone:?}");
        assert!(!d.outlined(zone_rect(timeline, zone).unwrap().shrink(1.0)), "{zone:?}");
        d.key(egui::Key::Escape);
    }
    // The middle and the tab row: the center rectangle and the strip.
    for at in [timeline.center() + vec2(0.0, 13.0), strip.center()] {
        let mut d = film();
        d.hold(tab(&d, INFO), at);
        assert!(d.filled_polygon(&zone_shape(timeline, 26.0, 40.0, Zone::Center).unwrap()), "{at:?}");
        assert!(d.filled_polygon(&corners(strip)), "{at:?}");
        assert!(!d.outlined(timeline.shrink(1.0)), "{at:?}");
        d.key(egui::Key::Escape);
    }
    // A group's drag bar shows the same.
    let mut d = film();
    let bar = d.hit(&Hit::DragBar(PROJECT[0])).unwrap().center();
    d.hold(bar, pos2(timeline.left() + 8.0, timeline.center().y + 13.0));
    assert!(d.filled_polygon(&zone_shape(timeline, 26.0, 40.0, Zone::Left).unwrap()));
    assert_eq!(zone_outlines(&d).len(), 6);
    d.key(egui::Key::Escape);

    // The dock-and-canvas arrangement shows the rect the panel takes there instead.
    let mut d = DockDriver::new(editing(), Film::default());
    d.screen = Rect::from_min_size(Pos2::ZERO, SCREEN);
    d.area = d.screen;
    d.settle();
    let at = pos2(timeline.left() + 8.0, timeline.center().y + 13.0);
    d.hold(tab(&d, INFO), at);
    assert!(!d.filled_polygon(&zone_shape(timeline, 26.0, 40.0, Zone::Left).unwrap()));
    assert!(zone_outlines(&d).is_empty());
    assert!(!d.filled(zone_rect(timeline, Zone::Left).unwrap().shrink(1.0)));
    let shapes = d.shapes().to_vec();
    d.release(at);
    d.settle();
    let info = group(&d, INFO);
    assert_eq!(info.left(), timeline.left());
    let painted = |fill: egui::Color32, stroke: f32| {
        shapes.iter().any(|c| matches!(&c.shape, egui::Shape::Rect(r) if r.rect == info.shrink(1.0) && r.fill == fill && r.stroke.width == stroke))
    };
    let accent = DockStyle::default().accent;
    assert!(painted(accent.gamma_multiply(0.08), 0.0) && painted(egui::Color32::TRANSPARENT, 2.0), "Info's rect beside the timeline");
}

#[test]
fn a_drag_over_a_group_outlines_all_its_zones_and_fills_the_one_drop_zone_names() {
    let d = film();
    let timeline = group(&d, TIMELINE);
    let strip = Rect::from_min_size(timeline.min, vec2(timeline.width(), 26.0));
    let shape = |zone| zone_shape(timeline, 26.0, 40.0, zone).unwrap();
    let all: Vec<Vec<Pos2>> =
        [Zone::Left, Zone::Right, Zone::Top, Zone::Bottom, Zone::Center].map(shape).into_iter().chain([corners(strip)]).collect();
    let body = Rect::from_min_max(pos2(timeline.left(), strip.bottom()), timeline.max);
    // Each band, near its corners too, the middle, and the tab row.
    let points = [
        pos2(body.left() + 30.0, body.center().y),
        pos2(body.left() + 30.0, body.top() + 34.0),
        pos2(body.right() - 2.0, body.bottom() - 12.0),
        pos2(body.center().x, body.top() + 5.0),
        pos2(body.left() + 50.0, body.bottom() - 30.0),
        body.center(),
        pos2(body.left() + 45.0, body.top() + 45.0),
        strip.center(),
    ];
    for at in points {
        let mut d = film();
        d.hold(tab(&d, INFO), at);
        let outlines = zone_outlines(&d);
        assert_eq!(outlines.len(), all.len(), "{at:?}: four sides, the center and the strip");
        assert!(all.iter().all(|zone| outlines.iter().any(|o| same_polygon(o, zone))), "{at:?}: only Timeline's zones");
        let zone = drop_zone(timeline, 26.0, 40.0, at).unwrap();
        assert!(d.filled_polygon(&shape(zone)), "{at:?}: {zone:?} is filled");
        let filled = all.iter().filter(|z| d.filled_polygon(z)).count();
        assert_eq!(filled, if zone == Zone::Center { 2 } else { 1 }, "{at:?}: only {zone:?}");
        d.key(egui::Key::Escape);
    }
}

#[test]
fn zones_a_group_does_not_take_are_not_drawn() {
    let timeline = group(&film(), TIMELINE);
    let strip = Rect::from_min_size(timeline.min, vec2(timeline.width(), 26.0));
    let shape = |zone| zone_shape(timeline, 26.0, 40.0, zone).unwrap();
    let at = pos2(timeline.left() + 8.0, timeline.center().y + 13.0);
    let content = |rules: Permissions| Film { rules: HashMap::from([(TIMELINE, rules)]), ..Film::default() };
    // No splits: the center and the strip only, and the left band under the pointer neither
    // outlined nor filled.
    let mut d = driver(editing(), content(Permissions { accepts_splits: false, ..Permissions::default() }));
    d.hold(tab(&d, INFO), at);
    let outlines = zone_outlines(&d);
    assert_eq!(outlines.len(), 2, "{outlines:?}");
    assert!(outlines.iter().any(|o| same_polygon(o, &shape(Zone::Center))) && outlines.iter().any(|o| same_polygon(o, &corners(strip))));
    assert!(!d.filled_polygon(&shape(Zone::Left)));
    d.key(egui::Key::Escape);
    // No tabs: the four sides only.
    let mut d = driver(editing(), content(Permissions { accepts_tabs: false, ..Permissions::default() }));
    d.hold(tab(&d, INFO), at);
    let outlines = zone_outlines(&d);
    assert_eq!(outlines.len(), 4, "{outlines:?}");
    assert!([Zone::Left, Zone::Right, Zone::Top, Zone::Bottom].into_iter().all(|z| outlines.iter().any(|o| same_polygon(o, &shape(z)))));
    assert!(d.filled_polygon(&shape(Zone::Left)));
    d.key(egui::Key::Escape);
    // A tab over its own single-tab group: nothing to drop on.
    let mut d = film();
    d.hold(tab(&d, TIMELINE), timeline.center() + vec2(0.0, 13.0));
    assert!(zone_outlines(&d).is_empty());
}

#[test]
fn a_narrow_group_splits_on_every_side_and_its_neighbours_give_the_room() {
    // Tools at 80 pt: two halves need 84, so a side split takes the rest from Timeline.
    let narrow = || {
        let mut d = film();
        let width = group(&d, TOOLS).width();
        drag_hit(&mut d, divider(&[true, true]), vec2(80.0 - width, 0.0));
        d
    };
    let d = narrow();
    let (tools, timeline) = (group(&d, TOOLS), group(&d, TIMELINE));
    assert_eq!(tools.width().round(), 80.0);
    let body_top = tools.top() + 26.0;
    for (zone, at) in [
        (Zone::Left, pos2(tools.left() + 5.0, tools.center().y)),
        (Zone::Right, pos2(tools.right() - 5.0, tools.center().y)),
        (Zone::Top, pos2(tools.center().x, body_top + 5.0)),
        (Zone::Bottom, pos2(tools.center().x, tools.bottom() - 15.0)),
    ] {
        let mut d = narrow();
        d.hold(tab(&d, INFO), at);
        assert_eq!(zone_outlines(&d).len(), 6, "{zone:?}: four sides, the center and the strip");
        assert!(d.filled_polygon(&zone_shape(tools, 26.0, 40.0, zone).unwrap()), "{zone:?} is filled");
        let out = d.release(at);
        assert_eq!(out.actions, vec![Action::Move { panel: INFO, anchor: TOOLS, placement: Placement::Split(zone) }]);
        d.settle();
        let (info, split) = (group(&d, INFO), group(&d, TOOLS));
        let beside = match zone {
            Zone::Left => info.right() < split.left(),
            Zone::Right => info.left() > split.right(),
            Zone::Top => info.bottom() < split.top(),
            _ => info.top() > split.bottom(),
        };
        assert!(beside, "{zone:?}: Info {info:?}, Tools {split:?}");
        if matches!(zone, Zone::Left | Zone::Right) {
            assert_eq!((info.width(), split.width()), (40.0, 40.0), "{zone:?}");
            assert!((group(&d, TIMELINE).width() - (timeline.width() - 4.0)).abs() < 1.0, "{zone:?}: Timeline gives the room");
        }
        assert_sizes(&d, &format!("{zone:?}"));
    }
}
