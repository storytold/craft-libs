//! [D] Docks: columns, stacked groups, seams, column bars and their drops.

use super::*;
use crate::layout::{SplitAxis, SplitSize};
use egui::{Context, Event, Id, Modifiers, PointerButton, Pos2, RawInput, Rect, UiBuilder, pos2, vec2};

const AREA: &str = "dock-area";

fn group(panels: &[u32]) -> Group<u32> {
    Group::new(panels.to_vec(), 0)
}

/// A dock on `side` with `columns` (each a list of groups, 240 wide) beside canvas 0.
fn docked(side: Side, columns: &[&[&[u32]]]) -> Layout<u32> {
    let dock = Node::Dock { side, columns: columns.iter().map(|groups| Column::new(groups.iter().map(|g| group(g)).collect(), 240.0)).collect() };
    let canvas = Node::Tabs { panels: vec![0], active: 0 };
    let (first, second, size) = match side {
        Side::Start => (dock, canvas, SplitSize::FixedFirst(240.0)),
        Side::End => (canvas, dock, SplitSize::FixedSecond(240.0)),
    };
    Layout { root: Some(Node::Split { axis: SplitAxis::Horizontal, size, first: Box::new(first), second: Box::new(second) }), floating: Vec::new() }
}

fn with_floating(mut layout: Layout<u32>, windows: &[(u32, [f32; 4])]) -> Layout<u32> {
    layout.floating.extend(windows.iter().map(|(panel, rect)| Floating::single(vec![*panel], 0, *rect)));
    layout
}

fn columns(layout: &Layout<u32>, side: Side) -> Vec<Vec<Vec<u32>>> {
    fn find(node: &Node<u32>, side: Side) -> Option<Vec<Vec<Vec<u32>>>> {
        match node {
            Node::Dock { side: s, columns } if *s == side => {
                Some(columns.iter().map(|c| c.groups.iter().map(|g| g.panels.clone()).collect()).collect())
            }
            Node::Split { first, second, .. } => find(first, side).or_else(|| find(second, side)),
            _ => None,
        }
    }
    layout.root.as_ref().and_then(|root| find(root, side)).unwrap_or_default()
}

fn column(layout: &Layout<u32>, index: usize) -> Column<u32> {
    fn find(node: &Node<u32>) -> Option<&Vec<Column<u32>>> {
        match node {
            Node::Dock { columns, .. } => Some(columns),
            Node::Split { first, second, .. } => find(first).or_else(|| find(second)),
            _ => None,
        }
    }
    layout.root.as_ref().and_then(find).and_then(|c| c.get(index)).cloned().unwrap()
}

#[derive(Default)]
struct TestContent {
    min_height: f32,
}

impl DockContent<u32> for TestContent {
    fn body(&mut self, ui: &mut egui::Ui, _: &u32) {
        ui.label("Body");
    }
    fn permissions(&self, panel: &u32) -> Permissions {
        if *panel == 0 { Permissions::PROTECTED } else { Permissions::default() }
    }
    fn limits(&self, _: &u32) -> PanelLimits {
        PanelLimits { min: vec2(0.0, self.min_height), ..PanelLimits::default() }
    }
}

/// Renders a layout in a 600x600 area of an 800x600 window and applies what it emits.
struct Driver {
    ctx: Context,
    layout: Layout<u32>,
    content: TestContent,
    focused: bool,
    right_to_left: bool,
    shapes: Vec<egui::epaint::ClippedShape>,
    time: f64,
}

impl Driver {
    fn new(layout: Layout<u32>) -> Self {
        Self::with(layout, TestContent::default(), false)
    }

    fn with(layout: Layout<u32>, content: TestContent, right_to_left: bool) -> Self {
        Self::start(Self { ctx: Context::default(), layout, content, focused: true, right_to_left, shapes: Vec::new(), time: 0.0 })
    }

    /// A window egui never heard has focus (macOS can make it key without a `Focused` event).
    fn never_focused(layout: Layout<u32>) -> Self {
        Self::start(Self {
            ctx: Context::default(),
            layout,
            content: TestContent::default(),
            focused: false,
            right_to_left: false,
            shapes: Vec::new(),
            time: 0.0,
        })
    }

    /// Run three empty frames so floating areas finish their sizing pass, then
    /// return the driver.
    fn start(mut driver: Self) -> Self {
        // Floating areas settle after their sizing pass.
        for _ in 0..3 {
            driver.step(vec![]);
        }
        driver
    }

    /// One 800×600 frame with injected events and the driver's focus/right-to-left settings.
    /// Renders the dock in a 600×600 scope, clears texture deltas, records shapes, and returns
    /// `DockOutput`.
    fn frame(&mut self, events: Vec<Event>) -> DockOutput<u32> {
        let mut output = DockOutput::default();
        self.time += 1.0 / 60.0;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))),
            events,
            focused: self.focused,
            time: Some(self.time),
            ..Default::default()
        };
        let (layout, content, rtl) = (&self.layout, &mut self.content, self.right_to_left);
        let mut full = self.ctx.run_ui(input, |ui| {
            let mut style = DockStyle::from_ui(ui);
            style.right_to_left = rtl;
            ui.scope_builder(UiBuilder::new().max_rect(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 600.0))), |ui| {
                output = DockArea::new(Id::unique(AREA)).show_content(ui, layout, &style, content);
            });
        });
        full.textures_delta.clear();
        self.shapes = full.shapes;
        output
    }

    fn step(&mut self, events: Vec<Event>) -> DockOutput<u32> {
        let output = self.frame(events);
        assert!(output.error.is_none(), "{output:?}");
        let permissions = |p: &u32| self.content.permissions(p);
        for action in &output.actions {
            self.layout.apply_with_permissions(action.clone(), permissions).unwrap();
        }
        output
    }

    /// Read the response rectangle for the dock widget named `name` and scoped to
    /// `panel`.
    fn rect(&self, name: &str, panel: u32) -> Rect {
        self.ctx.read_response(Id::unique(AREA).with((name, Some(&panel)))).unwrap().rect
    }

    /// Read the response rectangle for `panel`'s tab.
    fn tab(&self, panel: u32) -> Rect {
        self.ctx.read_response(Id::unique(AREA).with(("tab", &panel))).unwrap().rect
    }

    fn hold(&mut self, from: Pos2, to: Pos2) -> DockOutput<u32> {
        self.step(vec![Event::PointerMoved(from)]);
        self.step(vec![button(from, true)]);
        self.step(vec![Event::PointerMoved(from + vec2(8.0, 6.0))]);
        self.step(vec![Event::PointerMoved(to)])
    }

    fn drag(&mut self, from: Pos2, to: Pos2) -> DockOutput<u32> {
        self.hold(from, to);
        self.step(vec![button(to, false)])
    }

    fn click(&mut self, at: Pos2) -> DockOutput<u32> {
        self.step(vec![Event::PointerMoved(at), button(at, true)]);
        self.step(vec![button(at, false)])
    }

    /// Two frames: egui reports a moved widget's rect a frame late.
    fn settle(&mut self) {
        self.step(vec![]);
        self.step(vec![]);
    }

    fn wait(&mut self) {
        for _ in 0..45 {
            self.step(vec![]);
        }
    }

    /// Whether the last frame painted the result preview of `rect`: a 2 pt outline with a light
    /// fill, inset 1 pt.
    fn previewed(&self, rect: Rect) -> bool {
        let rect = rect.shrink(1.0);
        let shapes = || {
            self.shapes.iter().filter_map(|c| match &c.shape {
                egui::Shape::Rect(r) if r.rect == rect => Some(r),
                _ => None,
            })
        };
        shapes()
            .filter(|r| r.fill == egui::Color32::TRANSPARENT && r.stroke.width == 2.0)
            .any(|outline| shapes().any(|r| r.stroke.width == 0.0 && r.fill == outline.stroke.color.gamma_multiply(0.08)))
    }

    /// The rect of the group whose strip starts with `panel`'s tab, in the column whose bar
    /// names `column`: down to the next group's tab, `next`, or the window's bottom.
    fn group_rect(&self, panel: u32, next: Option<u32>, column: u32) -> Rect {
        let tab = self.tab(panel);
        let bottom = next.map_or(600.0, |next| self.tab(next).top());
        Rect::from_x_y_ranges(self.column_of(column).x_range(), tab.top()..=bottom)
    }

    /// The rect of the dock column whose bar names `panel`.
    fn column_of(&self, panel: u32) -> Rect {
        let bar = self.rect("column-bar", panel).union(self.rect("column-collapse", panel));
        Rect::from_x_y_ranges(bar.x_range(), bar.top()..=600.0)
    }

    /// Whether the last frame painted the filled insertion line `line`.
    fn painted(&self, line: Rect) -> bool {
        self.shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Rect(shape) => shape.rect == line && shape.fill != egui::Color32::TRANSPARENT,
            _ => false,
        })
    }
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

fn key(key: egui::Key) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
}

fn moved(actions: &[Action<u32>]) -> Option<(u32, &Target<u32>)> {
    actions.iter().find_map(|a| match a {
        Action::MoveGroup { panel, to } | Action::MoveColumn { panel, to } => Some((*panel, to)),
        _ => None,
    })
}

fn hline(y: f32, x: std::ops::RangeInclusive<f32>) -> Rect {
    Rect::from_x_y_ranges(x, (y - 2.0)..=(y + 2.0))
}

fn vline(x: f32, y: std::ops::RangeInclusive<f32>) -> Rect {
    Rect::from_x_y_ranges((x - 2.0)..=(x + 2.0), y)
}

/// A floating group's drag bar.
fn bar(d: &Driver, panel: u32) -> Pos2 {
    d.rect("drag-bar", panel).center()
}

// Geometry of `docked(Start, ..)`: the dock is 0..240 wide, its column bar 14 high, and two open
// groups share 14..600, meeting at y = 307.

#[test]
fn group_dropped_on_a_seam_inserts_between_and_previews_the_rect_it_takes() {
    let layout = with_floating(docked(Side::Start, &[&[&[2], &[3]]]), &[(5, [400.0, 300.0, 150.0, 150.0])]);
    let mut d = Driver::new(layout);
    assert_eq!(d.tab(3).top(), 307.0);
    let seam = pos2(120.0, 309.0);
    d.hold(bar(&d, 5), seam);
    let held = d.shapes.clone();
    let out = d.step(vec![button(seam, false)]);
    d.settle();
    let rect = d.group_rect(5, Some(3), 2);
    d.shapes = held;
    assert!(d.previewed(rect), "group 5's rect between groups 2 and 3: {rect:?}");
    assert_eq!(moved(&out.actions), Some((5, &Target::At { anchor: 2, placement: Placement::Group { below: true, height: None } })));
    assert_eq!(columns(&d.layout, Side::Start), vec![vec![vec![2], vec![5], vec![3]]]);
    assert!(d.layout.floating.is_empty());
}

#[test]
fn a_tab_dropped_on_a_seam_or_column_edge_moves_only_that_panel() {
    let mut d = Driver::new(docked(Side::Start, &[&[&[2, 3], &[4]]]));
    let out = d.drag(d.tab(3).center(), pos2(120.0, 309.0));
    assert_eq!(out.actions, vec![Action::Move { panel: 3, anchor: 2, placement: Placement::Group { below: true, height: None } }]);
    assert_eq!(columns(&d.layout, Side::Start), vec![vec![vec![2], vec![3], vec![4]]]);
    d.settle();
    d.drag(d.tab(4).center(), pos2(246.0, 200.0));
    assert_eq!(columns(&d.layout, Side::Start), vec![vec![vec![2], vec![3]], vec![vec![4]]]);
}

#[test]
fn group_dropped_on_a_top_or_bottom_band_stacks_above_or_below() {
    for (at, expected) in [
        // The top quarter of group 2's body, and the bottom quarter of group 3's.
        (pos2(120.0, 14.0 + 26.0 + 20.0), vec![vec![5], vec![2], vec![3]]),
        (pos2(120.0, 590.0), vec![vec![2], vec![3], vec![5]]),
        // The middle of a body joins as tabs.
        (pos2(120.0, 450.0), vec![vec![2], vec![3, 5]]),
    ] {
        let layout = with_floating(docked(Side::Start, &[&[&[2], &[3]]]), &[(5, [400.0, 300.0, 150.0, 150.0])]);
        let mut d = Driver::new(layout);
        d.drag(bar(&d, 5), at);
        assert_eq!(columns(&d.layout, Side::Start), vec![expected], "{at:?}");
    }
}

#[test]
fn a_dock_groups_top_and_bottom_thirds_stack_its_sides_make_columns_and_its_middle_joins() {
    // Group 3's body is 0..240 across and 333..600 down. Each drop beside previews the rect
    // group 5 then takes: in the column, below the group `Some` names, or a whole column.
    for (at, expected, result) in [
        (pos2(120.0, 360.0), vec![vec![vec![2], vec![5], vec![3]]], Some(Some(3))),
        (pos2(120.0, 580.0), vec![vec![vec![2], vec![3], vec![5]]], Some(None)),
        (pos2(30.0, 466.0), vec![vec![vec![5]], vec![vec![2], vec![3]]], None),
        (pos2(200.0, 466.0), vec![vec![vec![2], vec![3]], vec![vec![5]]], None),
        (pos2(120.0, 466.0), vec![vec![vec![2], vec![3, 5]]], None),
    ] {
        let layout = with_floating(docked(Side::Start, &[&[&[2], &[3]]]), &[(5, [400.0, 300.0, 150.0, 150.0])]);
        let mut d = Driver::new(layout);
        d.hold(bar(&d, 5), at);
        let held = d.shapes.clone();
        d.step(vec![button(at, false)]);
        assert_eq!(columns(&d.layout, Side::Start), expected, "{at:?}");
        d.settle();
        let rect = match result {
            Some(next) => d.group_rect(5, next, 2),
            None if expected.len() == 2 => d.column_of(5),
            None => continue,
        };
        d.shapes = held;
        assert!(d.previewed(rect), "{at:?}: the rect group 5 takes, {rect:?}");
    }
}

#[test]
fn with_side_edge_zones_a_dock_groups_side_zones_are_40_pt_deep() {
    // Group 3's body is 0..240 across and 333..600 down. 60 pt in from its left edge and 57
    // below its top are in its outer thirds but past 40 pt: the middle, which joins it.
    for (at, expected) in [
        (pos2(30.0, 466.0), vec![vec![vec![5]], vec![vec![2], vec![3]]]),
        (pos2(120.0, 360.0), vec![vec![vec![2], vec![5], vec![3]]]),
        (pos2(60.0, 466.0), vec![vec![vec![2], vec![3, 5]]]),
        (pos2(120.0, 390.0), vec![vec![vec![2], vec![3, 5]]]),
    ] {
        let layout = with_floating(docked(Side::Start, &[&[&[2], &[3]]]), &[(5, [400.0, 300.0, 150.0, 150.0])]);
        let mut d = testing::DockDriver::new(layout, TestContent::default())
            .with_area(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 600.0)))
            .with_style(|s| s.side_edge_zones = true);
        let tab = d.hit(&Hit::Tab(5)).unwrap_or_else(|| panic!("tab 5")).center();
        d.drag(tab, at);
        assert_eq!(columns(&d.layout, Side::Start), expected, "{at:?}");
    }
}

#[test]
fn a_group_edge_scrolled_out_of_view_previews_its_line_at_the_views_edge() {
    // Groups 306 pt high from y = 14: group 3 spans 320..626 and its body's bottom third starts
    // at 533, above the window's bottom.
    let layout = with_floating(docked(Side::Start, &[&[&[2], &[3], &[4]]]), &[(5, [400.0, 300.0, 150.0, 150.0])]);
    let mut d = Driver::with(layout, TestContent { min_height: 280.0 }, false);
    let at = pos2(120.0, 580.0);
    d.hold(bar(&d, 5), at);
    assert!(d.painted(hline(598.0, 0.0..=240.0)), "the line at the view's bottom");
    d.step(vec![button(at, false)]);
    assert_eq!(columns(&d.layout, Side::Start), vec![vec![vec![2], vec![3], vec![5], vec![4]]]);
}

#[test]
fn in_a_scrolled_column_a_stack_previews_the_rect_at_the_scroll_offset() {
    // Groups 306 pt high from y = 14: group 3 spans 320..626 before scrolling.
    let layout = with_floating(docked(Side::Start, &[&[&[2], &[3], &[4]]]), &[(5, [400.0, 300.0, 150.0, 150.0])]);
    let mut d = Driver::with(layout, TestContent { min_height: 280.0 }, false);
    let over = d.tab(2).center();
    d.step(vec![
        Event::PointerMoved(over),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -200.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
            source: egui::MouseWheelSource::Unknown,
        },
    ]);
    d.wait();
    let group = d.tab(3);
    assert!(group.top() < 250.0, "scrolled up: {group:?}");
    // Group 3's top third stacks 5 above it, where 3 is now.
    let at = pos2(120.0, group.bottom() + 40.0);
    d.hold(bar(&d, 5), at);
    let held = d.shapes.clone();
    d.step(vec![button(at, false)]);
    assert_eq!(columns(&d.layout, Side::Start), vec![vec![vec![2], vec![5], vec![3], vec![4]]]);
    d.settle();
    let rect = d.group_rect(5, Some(3), 2);
    d.shapes = held;
    assert!(d.previewed(rect), "the rect group 5 takes, {rect:?}");
}

#[test]
fn a_stripless_dock_groups_top_third_stacks_above_it() {
    // Canvas 9 in the middle; protected panel 0 alone in the lower group draws no tab strip, so
    // its body is 307..600.
    let mut layout = with_floating(docked(Side::Start, &[&[&[2], &[0]]]), &[(5, [400.0, 300.0, 150.0, 150.0])]);
    if let Some(Node::Split { second, .. }) = &mut layout.root {
        **second = Node::Tabs { panels: vec![9], active: 0 };
    }
    let mut d = Driver::new(layout);
    d.drag(bar(&d, 5), pos2(120.0, 330.0));
    assert_eq!(columns(&d.layout, Side::Start), vec![vec![vec![2], vec![5], vec![0]]]);
}

#[test]
fn dock_bands_are_28_pt_wide() {
    let window = || with_floating(docked(Side::Start, &[&[&[2], &[3]]]), &[(5, [400.0, 300.0, 150.0, 150.0])]);
    let target = |layout: Layout<u32>, at: Pos2| {
        let mut d = Driver::new(layout);
        let out = d.drag(bar(&d, 5), at);
        moved(&out.actions).map(|(_, to)| to.clone())
    };
    let column_after = Target::At { anchor: 2, placement: Placement::Column { after: true } };
    // 14 pt either side of the dock's inner edge at 240; past it, the protected canvas.
    assert_eq!(target(window(), pos2(253.0, 300.0)), Some(column_after.clone()));
    assert_ne!(target(window(), pos2(255.0, 300.0)), Some(column_after));
    // 28 pt along the window edge without a dock.
    assert_eq!(target(window(), pos2(573.0, 300.0)), Some(Target::NewDock { side: Side::End }));
    assert_ne!(target(window(), pos2(571.0, 300.0)), Some(Target::NewDock { side: Side::End }));
    // 28 pt along the bottom of the part between the docks, which splits at the root.
    assert!(matches!(target(window(), pos2(420.0, 573.0)), Some(Target::At { placement: Placement::Edge(Zone::Bottom), .. })));
    assert!(!matches!(target(window(), pos2(420.0, 571.0)), Some(Target::At { placement: Placement::Edge(_), .. })));
    // 14 pt either side of a seam. In a 400 pt column, 16 pt in from the window edge and 13
    // above the seam at 307 is relatively nearer group 2's left than its bottom, but the seam
    // takes it; 15 above, the left third makes a column.
    let wide = || {
        let mut layout = with_floating(docked(Side::Start, &[&[&[2], &[3]]]), &[(5, [450.0, 300.0, 100.0, 150.0])]);
        if let Some(Node::Split { size, first, .. }) = &mut layout.root {
            *size = SplitSize::FixedFirst(400.0);
            if let Node::Dock { columns, .. } = &mut **first {
                columns.iter_mut().for_each(|c| c.width = 400.0);
            }
        }
        layout
    };
    assert_eq!(target(wide(), pos2(16.0, 294.0)), Some(Target::At { anchor: 2, placement: Placement::Group { below: true, height: None } }));
    assert_eq!(target(wide(), pos2(16.0, 292.0)), Some(Target::At { anchor: 2, placement: Placement::Column { after: false } }));
}

#[test]
fn group_dropped_on_a_column_edge_makes_a_new_column() {
    // The dock's inner edge: a column after the existing one, previewed full height.
    let layout = with_floating(docked(Side::Start, &[&[&[2], &[3]]]), &[(5, [400.0, 300.0, 150.0, 150.0])]);
    let mut d = Driver::new(layout.clone());
    let edge = pos2(246.0, 200.0);
    d.hold(bar(&d, 5), edge);
    let held = d.shapes.clone();
    let out = d.step(vec![button(edge, false)]);
    d.settle();
    let rect = d.column_of(5);
    assert_eq!((rect.left(), rect.top(), rect.bottom()), (240.0, 0.0, 600.0));
    let shapes = std::mem::replace(&mut d.shapes, held);
    assert!(d.previewed(rect), "the new column's rect, the full height of the dock");
    d.shapes = shapes;
    assert_eq!(moved(&out.actions), Some((5, &Target::At { anchor: 2, placement: Placement::Column { after: true } })));
    assert_eq!(columns(&d.layout, Side::Start), vec![vec![vec![2], vec![3]], vec![vec![5]]]);
    assert_eq!(column(&d.layout, 1).width, 150.0, "the new column keeps the window's width");

    // The window edge of the outermost column: a column before it.
    let mut d = Driver::new(layout);
    d.drag(bar(&d, 5), pos2(3.0, 200.0));
    assert_eq!(columns(&d.layout, Side::Start), vec![vec![vec![5]], vec![vec![2], vec![3]]]);
}

#[test]
fn group_dropped_on_the_window_edge_without_a_dock_makes_a_new_dock() {
    for (side, at, new) in [(Side::Start, pos2(595.0, 200.0), Side::End), (Side::End, pos2(4.0, 200.0), Side::Start)] {
        let layout = with_floating(docked(side, &[&[&[2]]]), &[(5, [300.0, 300.0, 150.0, 150.0])]);
        let mut d = Driver::new(layout);
        d.hold(bar(&d, 5), at);
        let x = if new == Side::End { 598.0 } else { 2.0 };
        assert!(d.painted(vline(x, 0.0..=600.0)), "{new:?}: a vertical line at the window edge");
        let held = d.shapes.clone();
        let out = d.step(vec![button(at, false)]);
        d.settle();
        let rect = d.column_of(5);
        let shapes = std::mem::replace(&mut d.shapes, held);
        assert!(!d.previewed(rect), "{new:?}: no rect of the new dock's column");
        d.shapes = shapes;
        assert_eq!(moved(&out.actions), Some((5, &Target::NewDock { side: new })));
        assert_eq!(columns(&d.layout, new), vec![vec![vec![5]]]);
        assert_eq!(columns(&d.layout, side), vec![vec![vec![2]]]);
        // The new dock is drawn on its edge, its »/« button at the window edge.
        d.settle();
        let button = d.rect("column-collapse", 5);
        assert!(if new == Side::End { button.right() == 600.0 } else { button.left() == 0.0 }, "{button:?}");
    }
}

#[test]
fn tab_dragged_to_the_window_edge_without_a_dock_makes_a_dock_of_that_panel() {
    let mut d = testing::DockDriver::new(docked(Side::Start, &[&[&[2, 3]]]), TestContent::default());
    let tab = d.hit(&Hit::Tab(3)).unwrap_or_else(|| panic!("{:?}", d.hits()));
    let out = d.drag(tab.center(), pos2(795.0, 300.0));
    assert!(out.actions.contains(&Action::MovePanel { panel: 3, to: Target::NewDock { side: Side::End } }), "{:?}", out.actions);
    assert_eq!(columns(&d.layout, Side::End), vec![vec![vec![3]]]);
    assert_eq!(columns(&d.layout, Side::Start), vec![vec![vec![2]]]);
}

#[test]
/// A group dropped on a column's tab strip joins as a tab, on a group body
/// appends as its last tab, and in the empty space below starts a new group.
fn a_column_takes_groups_anywhere_in_it_and_as_tabs() {
    let windows = [(5, [400.0, 40.0, 150.0, 120.0]), (6, [400.0, 220.0, 150.0, 120.0]), (7, [400.0, 400.0, 150.0, 120.0])];
    let mut d = Driver::new(with_floating(docked(Side::Start, &[&[&[2], &[3]]]), &windows));
    // Onto group 3's tab strip: joins as a tab.
    d.drag(bar(&d, 5), pos2(d.tab(3).right() + 20.0, d.tab(3).center().y));
    assert_eq!(columns(&d.layout, Side::Start), vec![vec![vec![2], vec![3, 5]]]);
    // Onto the middle of group 2's body: joins as the last tab.
    d.drag(bar(&d, 6), pos2(120.0, 160.0));
    assert_eq!(columns(&d.layout, Side::Start), vec![vec![vec![2, 6], vec![3, 5]]]);
    // Into the room under the groups, every group minimized: below the last group.
    d.step(vec![Event::PointerMoved(pos2(700.0, 10.0))]);
    let mut minimized = d.layout.clone();
    for panel in [2, 3] {
        minimized.apply(Action::SetMinimized { panel, minimized: true }).unwrap();
    }
    d.layout = minimized;
    d.step(vec![]);
    d.drag(bar(&d, 7), pos2(120.0, 500.0));
    assert_eq!(columns(&d.layout, Side::Start), vec![vec![vec![2, 6], vec![3, 5], vec![7]]]);
}

#[test]
/// A column of five groups with a 160 pt minimum is taller than the window; scrolling up over its
/// tab strip moves the last tab more than 100 pt into view.
fn a_column_too_tall_for_the_window_scrolls() {
    let groups: [&[u32]; 5] = [&[2], &[3], &[4], &[5], &[6]];
    let mut d = Driver::with(docked(Side::Start, &[&groups]), TestContent { min_height: 160.0 }, false);
    let last = d.tab(6);
    assert!(last.top() > 600.0, "the last group starts below the window: {last:?}");
    let over = d.tab(2).center();
    for _ in 0..6 {
        d.step(vec![
            Event::PointerMoved(over),
            Event::MouseWheel {
                source: egui::MouseWheelSource::Unknown,
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -200.0),
                phase: egui::TouchPhase::Move,
                modifiers: Modifiers::NONE,
            },
        ]);
    }
    d.wait();
    let moved = d.tab(6);
    assert!(moved.top() < last.top() - 100.0, "scrolled up: {moved:?} from {last:?}");
}

#[test]
/// Dragging a column bar to the window edge moves the column first; dragging it
/// away from the dock floats every group in the column and shows a follow ghost.
fn column_bar_drag_moves_the_column_and_away_from_a_dock_floats_its_groups() {
    let mut d = Driver::new(docked(Side::Start, &[&[&[2], &[3]], &[&[5]]]));
    // The second column to the window edge: it becomes the first.
    let out = d.drag(d.rect("column-bar", 5).center(), pos2(3.0, 200.0));
    assert_eq!(moved(&out.actions), Some((5, &Target::At { anchor: 2, placement: Placement::Column { after: false } })));
    assert_eq!(columns(&d.layout, Side::Start), vec![vec![vec![5]], vec![vec![2], vec![3]]]);
    d.settle();
    // Away from the dock: a floating window holding the column's groups.
    let source = Rect::from_min_max(pos2(240.0, 0.0), pos2(480.0, 600.0));
    assert_eq!(d.rect("column-bar", 2).left(), 240.0 + 18.0);
    let grab = d.rect("column-bar", 2).center();
    d.hold(grab, pos2(560.0, 300.0));
    // The column's width; a docked column floats at most 320 pt tall.
    let size = vec2(source.width(), 320.0);
    let ghost = egui::Rect::from_min_size(pos2(560.0, 300.0) - (grab - source.min), size);
    let ghost = egui::Rect::from_min_size(ghost.min.min(pos2(800.0, 600.0) - size).max(Pos2::ZERO), size);
    assert!(
        d.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.rect == ghost && r.fill != egui::Color32::TRANSPARENT)),
        "a ghost of the column follows the pointer"
    );
    let out = d.step(vec![button(pos2(560.0, 300.0), false)]);
    assert!(matches!(moved(&out.actions), Some((2, Target::Float { .. }))), "{:?}", out.actions);
    assert_eq!(columns(&d.layout, Side::Start), vec![vec![vec![5]]]);
    assert_eq!(d.layout.floating[0].groups(), vec![group(&[2]), group(&[3])]);
}

#[test]
/// Escape, focus loss, pointer exit and `cancel_drag` each end a held column drag without emitting
/// actions or a group drop; the release changes nothing and the layout equals the original.
fn column_drag_escape_focus_loss_pointer_exit_and_cancel_drag_leave_the_layout_unchanged() {
    let layout = docked(Side::Start, &[&[&[2], &[3]], &[&[5]]]);
    for case in ["escape", "focus", "exit", "cancel"] {
        let mut d = Driver::new(layout.clone());
        let target = pos2(3.0, 200.0);
        let held = d.hold(d.rect("column-bar", 5).center(), target);
        assert_eq!(held.group_drag, Some(5), "{case}");
        let out = match case {
            "escape" => d.step(vec![key(egui::Key::Escape)]),
            "focus" => {
                d.focused = false;
                let out = d.step(vec![]);
                d.focused = true;
                out
            }
            "exit" => d.step(vec![Event::PointerGone]),
            _ => {
                cancel_drag::<u32>(&d.ctx, Id::unique(AREA));
                d.step(vec![Event::PointerMoved(target + vec2(1.0, 0.0))])
            }
        };
        assert!(out.actions.is_empty(), "{case}: {out:?}");
        let out = d.step(vec![Event::PointerMoved(target), button(target, false)]);
        assert!(out.actions.is_empty() && out.group_drag.is_none(), "{case}: {out:?}");
        assert_eq!(d.layout, layout, "{case}");
        assert!(!is_dragging::<u32>(&d.ctx, Id::unique(AREA)), "{case}");
    }
}

#[test]
fn column_and_group_dividers_resize_and_minimized_groups_keep_their_height() {
    let mut d = Driver::new(docked(Side::Start, &[&[&[2], &[4]]]));
    // The dock's inner edge widens its column; the canvas gives way.
    d.drag(pos2(240.0, 300.0), pos2(280.0, 300.0));
    assert_eq!(column(&d.layout, 0).width, 280.0);
    d.settle();
    assert_eq!(d.rect("column-bar", 2).right(), 280.0);
    // The divider between the groups saves both heights.
    d.wait();
    d.drag(pos2(120.0, 307.0), pos2(120.0, 357.0));
    let groups = column(&d.layout, 0).groups;
    assert_eq!((groups[0].height, groups[1].height), (Some(343.0), Some(243.0)));
    d.settle();
    assert_eq!(d.tab(4).top(), 357.0);
    // Minimized, the lower group keeps its saved height and the upper one takes the room.
    d.layout.apply(Action::SetMinimized { panel: 4, minimized: true }).unwrap();
    d.settle();
    assert_eq!(column(&d.layout, 0).groups[1].height, Some(243.0));
    assert_eq!(d.tab(4).top(), 600.0 - 26.0);
}

#[test]
fn a_group_divider_dragged_past_the_lower_groups_minimum_shrinks_the_next_group() {
    let mut d = Driver::new(docked(Side::Start, &[&[&[2], &[3], &[4]]]));
    let divider = d.rect("group-resize", 2).center();
    d.drag(divider, divider + vec2(0.0, 500.0));
    // Groups 3 and 4 are at their minimum: a tab strip and a 40 pt body.
    let heights = column(&d.layout, 0).groups.iter().map(|g| g.height).collect::<Vec<_>>();
    assert_eq!(heights[1..], [Some(66.0), Some(66.0)], "{heights:?}");
}

#[test]
fn double_click_on_a_docked_tab_minimizes_and_clicking_another_tab_expands() {
    let mut d = Driver::new(docked(Side::Start, &[&[&[2, 3], &[4]]]));
    let minimized = |d: &Driver| column(&d.layout, 0).groups[0].minimized;
    let tab = d.tab(2).center();
    d.click(tab);
    d.click(tab);
    assert!(minimized(&d));
    d.settle();
    assert_eq!(d.tab(4).top(), 14.0 + 26.0, "the minimized group takes its strip");
    d.wait();
    d.click(d.tab(3).center());
    assert!(!minimized(&d));
    assert_eq!(column(&d.layout, 0).groups[0].active_panel(), Some(&3));
}

#[test]
fn collapse_button_and_column_bar_double_click_emit_set_iconic() {
    let mut d = Driver::new(docked(Side::Start, &[&[&[2], &[3]]]));
    let iconic = |d: &Driver| column(&d.layout, 0).iconic;
    let button = d.rect("column-collapse", 2).center();
    let out = d.click(button);
    assert_eq!(out.actions, vec![Action::SetIconic { panel: 2, iconic: true }]);
    assert!(iconic(&d));
    d.wait();
    d.click(d.rect("column-collapse", 2).center());
    assert!(!iconic(&d));
    d.wait();
    let bar = d.rect("column-bar", 2).center();
    d.click(bar);
    d.click(bar);
    assert!(iconic(&d));
}

#[test]
fn dock_targets_respect_occlusion_by_floating_windows() {
    // Window 5 covers the column's seam at x 150..240; window 6 is dragged.
    let windows = [(5, [150.0, 250.0, 200.0, 150.0]), (6, [400.0, 450.0, 150.0, 120.0])];
    let layout = with_floating(docked(Side::Start, &[&[&[2], &[3]]]), &windows);
    let mut d = Driver::new(layout.clone());
    let out = d.drag(bar(&d, 6), pos2(200.0, 309.0));
    assert!(matches!(moved(&out.actions), Some((6, Target::At { anchor: 5, .. }))), "joins the window in front: {:?}", out.actions);
    assert_eq!(columns(&d.layout, Side::Start), vec![vec![vec![2], vec![3]]]);
    // Left of the window, the seam takes it.
    let mut d = Driver::new(layout);
    d.drag(bar(&d, 6), pos2(100.0, 309.0));
    assert_eq!(columns(&d.layout, Side::Start), vec![vec![vec![2], vec![6], vec![3]]]);
}

#[test]
fn right_to_left_mirrors_docks_their_edges_and_new_docks() {
    let layout = with_floating(docked(Side::Start, &[&[&[2], &[3]]]), &[(5, [100.0, 300.0, 150.0, 150.0])]);
    let mut d = Driver::with(layout.clone(), TestContent::default(), true);
    // The start dock is on the right, its »/« button at the window edge.
    let column_bar = d.rect("column-bar", 2);
    assert_eq!((column_bar.left(), column_bar.right()), (360.0, 600.0 - 18.0));
    // Its inner edge is on the left: dragging it left widens the column.
    d.drag(pos2(360.0, 300.0), pos2(320.0, 300.0));
    assert_eq!(column(&d.layout, 0).width, 280.0);
    // The left window edge has no dock: a new end dock.
    d.wait();
    let out = d.drag(bar(&d, 5), pos2(4.0, 200.0));
    assert_eq!(moved(&out.actions), Some((5, &Target::NewDock { side: Side::End })));
    d.settle();
    assert_eq!(d.rect("column-collapse", 5).left(), 0.0);
}

#[test]
/// Dropping a column's first tab just past its own edge anchors it after the
/// other column's panel; a panel alone in its column stays in place.
fn a_column_s_first_tab_dropped_on_its_own_column_edge_anchors_on_another_panel() {
    // Two columns, 0..240 and 240..480; the dragged tab is the first panel of the last one.
    let wide = |columns: &[&[&[u32]]]| {
        let mut layout = docked(Side::Start, columns);
        if let Some(Node::Split { size, .. }) = &mut layout.root {
            *size = SplitSize::FixedFirst(480.0);
        }
        layout
    };
    let mut d = Driver::new(wide(&[&[&[2]], &[&[3, 4]]]));
    let out = d.drag(d.tab(3).center(), pos2(484.0, 300.0));
    assert_eq!(out.actions, vec![Action::Move { panel: 3, anchor: 4, placement: Placement::Column { after: true } }]);
    assert_eq!(columns(&d.layout, Side::Start), vec![vec![vec![2]], vec![vec![4]], vec![vec![3]]]);

    // Alone in its column, the panel stays where it is.
    let layout = wide(&[&[&[2]], &[&[3]]]);
    let mut d = Driver::new(layout.clone());
    let out = d.drag(d.tab(3).center(), pos2(484.0, 300.0));
    assert_eq!(out.actions, vec![]);
    assert_eq!(d.layout, layout);
}

#[test]
/// A driver that never reported window focus can still drag tab 3 to a column edge (moving it after
/// panel 4). When focus arrives and is then lost mid-drag, releasing emits no action and restores the
/// original layout.
fn a_window_that_never_reported_focus_drags_tabs_and_losing_focus_cancels_a_drag() {
    let mut layout = docked(Side::Start, &[&[&[2]], &[&[3, 4]]]);
    if let Some(Node::Split { size, .. }) = &mut layout.root {
        *size = SplitSize::FixedFirst(480.0);
    }
    let to = pos2(484.0, 300.0);
    let mut d = Driver::never_focused(layout.clone());
    let out = d.drag(d.tab(3).center(), to);
    assert_eq!(out.actions, vec![Action::Move { panel: 3, anchor: 4, placement: Placement::Column { after: true } }]);

    // Focus that arrives during the drag and goes again ends it.
    let mut d = Driver::never_focused(layout.clone());
    d.hold(d.tab(3).center(), to);
    d.focused = true;
    d.step(vec![Event::WindowFocused(true)]);
    assert!(is_dragging::<u32>(&d.ctx, Id::unique(AREA)));
    d.focused = false;
    d.step(vec![Event::WindowFocused(false)]);
    let out = d.step(vec![button(to, false)]);
    assert!(out.actions.is_empty(), "{out:?}");
    assert_eq!(d.layout, layout);
    assert!(!is_dragging::<u32>(&d.ctx, Id::unique(AREA)));
}

#[test]
fn a_dock_wider_than_the_window_shrinks_to_fit_and_resizes() {
    let widths = render::fit_widths(&[(600.0, Some(40.0)), (600.0, Some(40.0)), (48.0, None)], 800.0);
    assert!((widths.iter().sum::<f32>() - 800.0).abs() < 0.01 && widths[2] == 48.0, "{widths:?}");
    assert_eq!(render::fit_widths(&[(600.0, Some(300.0)), (600.0, Some(300.0))], 400.0), vec![300.0, 300.0]);

    // Three 600-wide columns saved on a larger screen, in an 800-wide window.
    let mut layout = docked(Side::Start, &[&[&[1]], &[&[2]], &[&[3]]]);
    if let Some(Node::Split { first, .. }) = &mut layout.root
        && let Node::Dock { columns, .. } = &mut **first
    {
        for column in columns {
            column.width = 600.0;
        }
    }
    let mut d = testing::DockDriver::new(layout, TestContent::default());
    for panel in [1, 2, 3] {
        let divider = d.hit(&Hit::Divider(Divider::Column(panel))).unwrap();
        assert!(d.hit(&Hit::Tab(panel)).is_some_and(|tab| tab.width() > 0.0), "{panel}");
        assert!(divider.right() < 800.0, "{panel}: {divider:?}");
    }
    let divider = d.hit(&Hit::Divider(Divider::Column(1))).unwrap().center();
    d.press(divider);
    d.move_to(divider + vec2(-8.0, 0.0));
    let out = d.move_to(divider + vec2(-30.0, 0.0));
    assert!(out.error.is_none() && !out.actions.is_empty(), "{out:?}");
    for action in &out.actions {
        assert!(matches!(action, Action::ResizeColumn { width, .. } if *width > 0.0), "{action:?}");
    }
    d.release(divider + vec2(-30.0, 0.0));
    let total: f32 = (0..3).map(|i| column(&d.layout, i).width).sum();
    assert!(total < 800.0, "{total}");
    assert!((d.hit(&Hit::Divider(Divider::Column(1))).unwrap().center().x - (divider.x - 30.0)).abs() < 1.0);
}

#[test]
fn screen_reader_values_follow_the_column_width_on_either_edge() {
    use egui::accesskit::{Action as A, ActionData, ActionRequest, TreeId};
    for (side, right_to_left) in [(Side::End, false), (Side::Start, true), (Side::Start, false)] {
        let mut d = testing::DockDriver::new(docked(side, &[&[&[2]]]), TestContent::default()).with_style(move |s| s.right_to_left = right_to_left);
        d.ctx.enable_accesskit();
        d.settle();
        let id = DockArea::<u32>::hit_id(d.id, &Hit::Divider(Divider::Column(2)));
        let request =
            |action, data| Event::AccessKitActionRequest(ActionRequest { action, target_tree: TreeId::ROOT, target_node: id.accesskit_id(), data });
        let case = format!("{side:?}, right to left {right_to_left}");
        let out = d.frame(vec![request(A::SetValue, Some(ActionData::NumericValue(300.0)))]);
        assert_eq!(out.actions, vec![Action::ResizeColumn { panel: 2, width: 300.0 }], "{case}");
        let out = d.frame(vec![request(A::Increment, None)]);
        assert_eq!(out.actions, vec![Action::ResizeColumn { panel: 2, width: 248.0 }], "{case}");
        // Arrow keys stay physical: right moves the divider right.
        d.ctx.memory_mut(|m| m.request_focus(id));
        d.frame(vec![]);
        let out = d.frame(vec![key(egui::Key::ArrowRight)]);
        let wider = d.hit(&Hit::Divider(Divider::Column(2))).unwrap().center().x < 400.0;
        let width = if wider { 248.0 } else { 232.0 };
        assert_eq!(out.actions, vec![Action::ResizeColumn { panel: 2, width }], "{case}");
    }
}

#[test]
fn an_unanchored_open_prefers_the_end_dock_and_an_edge_frame_takes_a_quarter() {
    let start = Node::Dock { side: Side::Start, columns: vec![Column::new(vec![group(&[1])], 240.0)] };
    let end = docked(Side::End, &[&[&[2]]]).root.unwrap();
    let mut layout = Layout {
        root: Some(Node::Split { axis: SplitAxis::Horizontal, size: SplitSize::FixedFirst(240.0), first: Box::new(start), second: Box::new(end) }),
        floating: Vec::new(),
    };
    layout.apply(Action::Open { panel: 9, anchor: None }).unwrap();
    assert_eq!(columns(&layout, Side::End), vec![vec![vec![2, 9]]]);
    assert_eq!(columns(&layout, Side::Start), vec![vec![vec![1]]]);
    // A minimized group and an iconic column are passed over: the panel would not show there.
    let start = Node::Dock { side: Side::Start, columns: vec![Column::new(vec![group(&[1])], 240.0)] };
    let mut end = docked(Side::End, &[&[&[2]]]).root.unwrap();
    if let Node::Split { second, .. } = &mut end
        && let Node::Dock { columns, .. } = second.as_mut()
    {
        columns[0].groups[0].minimized = true;
    }
    let mut layout = Layout {
        root: Some(Node::Split { axis: SplitAxis::Horizontal, size: SplitSize::FixedFirst(240.0), first: Box::new(start), second: Box::new(end) }),
        floating: Vec::new(),
    };
    let mut iconic = layout.clone();
    layout.apply(Action::Open { panel: 9, anchor: None }).unwrap();
    assert_eq!(columns(&layout, Side::Start), vec![vec![vec![1, 9]]]);
    if let Some(Node::Split { first, .. }) = &mut iconic.root
        && let Node::Dock { columns, .. } = first.as_mut()
    {
        columns[0].iconic = true;
    }
    iconic.apply(Action::Open { panel: 9, anchor: None }).unwrap();
    assert!(matches!(iconic.place(&9), Some(dock_model::Place::Node(_))), "{iconic:?}");

    // No dock: a frame at the right edge, a quarter of the width.
    let mut layout = Layout { root: Some(Node::Tabs { panels: vec![0], active: 0 }), floating: Vec::new() };
    layout.apply(Action::Open { panel: 9, anchor: None }).unwrap();
    let Some(Node::Split { axis: SplitAxis::Horizontal, size, second, .. }) = &layout.root else { panic!("{layout:?}") };
    assert_eq!(*size, SplitSize::Ratio(0.75));
    assert_eq!(**second, Node::Tabs { panels: vec![9], active: 0 });
}

/// Panels 1-3 as a creative app's Color, Properties and Layers: Layers fills the column and asks
/// for room for about ten rows.
struct Creative;

impl DockContent<u32> for Creative {
    fn body(&mut self, ui: &mut egui::Ui, _: &u32) {
        ui.label("Body");
    }
    fn permissions(&self, panel: &u32) -> Permissions {
        if *panel == 0 { Permissions::PROTECTED } else { Permissions::default() }
    }
    fn limits(&self, panel: &u32) -> PanelLimits {
        PanelLimits { min: vec2(0.0, if *panel == 3 { 120.0 } else { 60.0 }), ..PanelLimits::default() }
    }
    fn dock_heights(&self, panel: &u32) -> Option<DockHeights> {
        match panel {
            1 => Some(DockHeights::new(160.0).compact(100.0)),
            2 => Some(DockHeights::new(220.0).compact(130.0)),
            3 => Some(DockHeights::new(300.0).fill(480.0)),
            _ => None,
        }
    }
}

/// Color, Properties and Layers docked in a window `height` tall, with saved `heights`.
fn sized_column(height: f32, heights: [Option<f32>; 3]) -> testing::DockDriver<u32, Creative> {
    let mut layout = docked(Side::Start, &[&[&[1], &[2], &[3]]]);
    if let Some(Node::Split { first, .. }) = &mut layout.root
        && let Node::Dock { columns, .. } = first.as_mut()
    {
        columns[0].groups.iter_mut().zip(heights).for_each(|(g, h)| g.height = h);
    }
    let mut d = testing::DockDriver::new(layout, Creative);
    d.screen = Rect::from_min_size(Pos2::ZERO, vec2(1440.0, height));
    d.area = d.screen;
    d.settle();
    d
}

/// Heights of the column's groups as drawn, from their tabs' tops.
fn drawn_heights(d: &testing::DockDriver<u32, Creative>) -> [f32; 3] {
    let top = |p| d.hit(&Hit::Tab(p)).unwrap().top();
    [top(2) - top(1), top(3) - top(2), d.area.bottom() - top(3)]
}

#[test]
fn a_default_column_gives_the_filler_its_fill_height() {
    let strip = DockStyle::from_visuals(&egui::Visuals::dark()).tab_height;
    // A 900 pt window: Properties, nearest Layers, gives way towards its compact height so
    // Layers gets the height it asks for; Color keeps its default.
    let d = sized_column(900.0, [None; 3]);
    let [color, properties, layers] = drawn_heights(&d);
    assert!((layers - (480.0 + strip)).abs() < 0.5, "{:?}", drawn_heights(&d));
    assert_eq!(color, 160.0 + strip, "{:?}", drawn_heights(&d));
    assert!(properties < 220.0 + strip && properties >= 130.0 + strip, "{:?}", drawn_heights(&d));
    // A tall window: both at their defaults, Layers takes the rest.
    let d = sized_column(1200.0, [None; 3]);
    let [color, properties, layers] = drawn_heights(&d);
    assert_eq!((color, properties), (160.0 + strip, 220.0 + strip));
    assert!(layers > 480.0 + strip);
    // Nothing is saved until the user resizes a group.
    assert!(column(&d.layout, 0).groups.iter().all(|g| g.height.is_none()));
}

#[test]
fn shared_groups_with_different_minimums_fit_an_exact_room() {
    let open = |min: f32, content: Option<DockHeights>| render::GroupSize { minimized: false, saved: None, min, content };
    let column = [open(50.0, Some(DockHeights::new(186.0))), open(86.0, None), open(146.0, None)];
    assert_eq!(render::group_heights(&column, 26.0, 418.0), vec![186.0, 86.0, 146.0]);
    assert_eq!(render::group_heights(&column, 26.0, 518.0), vec![186.0, 166.0, 166.0]);
}

#[test]
fn a_group_asking_for_a_fill_fills_the_column_beside_groups_without_dock_heights() {
    let open = |min: f32, content: Option<DockHeights>| render::GroupSize { minimized: false, saved: None, min, content };
    let column = [open(86.0, Some(DockHeights::new(186.0))), open(86.0, None), open(146.0, Some(DockHeights::new(326.0).fill(506.0)))];
    assert_eq!(render::group_heights(&column, 26.0, 1000.0), vec![186.0, 86.0, 728.0]);
}

#[test]
fn defaults_give_way_to_the_filler_but_user_heights_stay() {
    let open = |saved: Option<f32>, min: f32, content: Option<DockHeights>| render::GroupSize { minimized: false, saved, min, content };
    let color = Some(DockHeights::new(186.0).compact(126.0));
    let properties = Some(DockHeights::new(246.0).compact(156.0));
    let layers = Some(DockHeights::new(326.0).fill(506.0));
    let column = [open(None, 86.0, color), open(None, 86.0, properties), open(None, 146.0, layers)];
    // Tall: defaults, the filler takes the rest.
    assert_eq!(render::group_heights(&column, 26.0, 1200.0), vec![186.0, 246.0, 768.0]);
    // Short: the default nearest the filler gives way first; Color keeps its default.
    assert_eq!(render::group_heights(&column, 26.0, 863.0), vec![186.0, 171.0, 506.0]);
    // Shorter: Properties is at its compact height before Color gives way.
    assert_eq!(render::group_heights(&column, 26.0, 818.0), vec![156.0, 156.0, 506.0]);
    // Shorter than the compact heights and the fill: every group gives way towards its minimum.
    let hs = render::group_heights(&column, 26.0, 400.0);
    assert!(hs.iter().zip(&column).all(|(h, g)| *h >= g.min - 0.01), "{hs:?}");
    assert!((hs.iter().sum::<f32>() - 400.0).abs() < 0.01 && hs[0] < 126.0 && hs[1] < 156.0, "{hs:?}");
    // Heights the user set stay; only the defaults give way.
    let mine = [open(Some(200.0), 86.0, color), open(None, 86.0, properties), open(None, 146.0, layers)];
    assert_eq!(render::group_heights(&mine, 26.0, 838.0), vec![200.0, 156.0, 482.0]);
    // In a drawn column too: Layers gets less than it asks for rather than the user's heights.
    let d = sized_column(900.0, [Some(200.0), Some(340.0), None]);
    assert_eq!(drawn_heights(&d), [200.0, 340.0, 900.0 - 14.0 - 540.0]);
}

/// Dock columns left and right of protected canvas 0, as in a creative app: group [1, 7] above
/// [2] at the left, [3] above [4] at the right, each column 200 wide, and a floating window
/// holding group [5, 6].
fn creative() -> testing::DockDriver<u32, TestContent> {
    let dock = |side, groups: &[&[u32]]| Node::Dock { side, columns: vec![Column::new(groups.iter().map(|g| group(g)).collect(), 200.0)] };
    let canvas = Node::Tabs { panels: vec![0], active: 0 };
    let inner = Node::Split {
        axis: SplitAxis::Horizontal,
        size: SplitSize::FixedSecond(200.0),
        first: Box::new(canvas),
        second: Box::new(dock(Side::End, &[&[3], &[4]])),
    };
    let root = Node::Split {
        axis: SplitAxis::Horizontal,
        size: SplitSize::FixedFirst(200.0),
        first: Box::new(dock(Side::Start, &[&[1, 7], &[2]])),
        second: Box::new(inner),
    };
    let layout = Layout { root: Some(root), floating: vec![Floating::single(vec![5, 6], 0, [300.0, 200.0, 200.0, 160.0])] };
    testing::DockDriver::new(layout, TestContent::default()).with_style(|s| {
        s.side_edge_zones = false;
        s.new_dock_zones = true;
        s.float_on_empty_drop = true;
        s.title_bar_docks = true;
    })
}

/// The rect of the result preview the last frame painted: a 2 pt accent outline with a light
/// fill.
fn result_preview<C: DockContent<u32>>(d: &testing::DockDriver<u32, C>) -> Option<Rect> {
    let fill = DockStyle::default().accent.gamma_multiply(0.08);
    d.shapes()
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(shape) if shape.fill == fill && shape.stroke.width == 0.0 => Some(shape.rect),
            _ => None,
        })
        .filter(|rect| d.outlined(*rect))
}

#[test]
fn a_group_dropped_on_the_window_edge_makes_a_column_there_and_previews_the_edge_line() {
    // Group [1, 7] heads the left column, so the column's window edge is named by panel 1: the
    // drop names the column by panel 2 instead and moves the whole group out.
    let mut d = creative();
    let at = pos2(3.0, 300.0);
    d.hold(d.hit(&Hit::DragBar(1)).unwrap().center(), at);
    assert!(d.filled(Rect::from_x_y_ranges(0.0..=4.0, 0.0..=600.0)), "the line along the window edge");
    assert_eq!(result_preview(&d), None);
    let out = d.release(at);
    assert_eq!(out.actions, vec![Action::MoveGroup { panel: 1, to: Target::At { anchor: 2, placement: Placement::Column { after: false } } }]);
    assert_eq!(columns(&d.layout, Side::Start), vec![vec![vec![1, 7]], vec![vec![2]]]);

    // A floating window's group at the right window edge.
    let mut d = creative();
    let at = pos2(797.0, 300.0);
    d.hold(d.hit(&Hit::DragBar(5)).unwrap().center(), at);
    assert!(d.filled(Rect::from_x_y_ranges(796.0..=800.0, 0.0..=600.0)), "the line along the window edge");
    assert_eq!(result_preview(&d), None);
    let out = d.release(at);
    assert_eq!(out.actions, vec![Action::MoveGroup { panel: 5, to: Target::At { anchor: 3, placement: Placement::Column { after: false } } }]);
    assert_eq!(columns(&d.layout, Side::End), vec![vec![vec![5, 6]], vec![vec![3], vec![4]]]);

    // The bottom edge band between the docks: the line along the canvas's bottom.
    let mut d = creative();
    let at = pos2(400.0, 597.0);
    d.hold(d.hit(&Hit::DragBar(5)).unwrap().center(), at);
    assert!(d.filled(Rect::from_x_y_ranges(204.0..=596.0, 596.0..=600.0)), "the line along the canvas's bottom edge");
    assert_eq!(result_preview(&d), None);
    let out = d.release(at);
    assert!(
        matches!(&out.actions[..], [Action::MoveGroup { panel: 5, to: Target::At { placement: Placement::Edge(Zone::Bottom), .. } }]),
        "{:?}",
        out.actions
    );
}

#[test]
fn a_tab_dropped_beside_a_dock_group_previews_the_rect_it_takes() {
    // The bottom third of group [2], the left column's last group: a new group below it.
    let mut d = creative();
    let at = pos2(100.0, 560.0);
    d.hold(d.hit(&Hit::Tab(6)).unwrap().center(), at);
    let preview = result_preview(&d);
    let out = d.release(at);
    assert_eq!(out.actions, vec![Action::Move { panel: 6, anchor: 2, placement: Placement::Group { below: true, height: None } }]);
    d.settle();
    let rect = Rect::from_x_y_ranges(0.0..=200.0, d.hit(&Hit::Tab(6)).unwrap().top()..=600.0);
    assert_eq!(preview, Some(rect.shrink(1.0)), "group 6's rect below group 2");
}
