//! [D] Iconic columns, flyouts and hit rects, driven through `testing::DockDriver`.

use super::testing::DockDriver;
use super::*;
use crate::layout::{SplitAxis, SplitSize};
use egui::{Key, Pos2, Rect, pos2, vec2};

struct Content;

impl DockContent<u32> for Content {
    fn body(&mut self, ui: &mut egui::Ui, _: &u32) {
        ui.label("Body");
    }
    fn logical_label(&self, panel: &u32) -> String {
        format!("Panel {panel}")
    }
    fn permissions(&self, panel: &u32) -> Permissions {
        if *panel == 0 { Permissions::PROTECTED } else { Permissions::default() }
    }
}

type Driver = DockDriver<u32, Content>;

/// A name and a test for one kind of [`Hit`].
type Check<'a> = (&'a str, &'a dyn Fn(&Hit<u32>) -> bool);

/// A left dock with one column of `groups`, iconic when `iconic`, beside canvas 0.
fn docked(groups: &[&[u32]], iconic: bool) -> Layout<u32> {
    let mut column = Column::new(groups.iter().map(|g| Group::new(g.to_vec(), 0)).collect(), 240.0);
    column.iconic = iconic;
    let dock = Node::Dock { side: Side::Start, columns: vec![column] };
    let canvas = Node::Tabs { panels: vec![0], active: 0 };
    Layout {
        root: Some(Node::Split { axis: SplitAxis::Horizontal, size: SplitSize::FixedFirst(240.0), first: Box::new(dock), second: Box::new(canvas) }),
        floating: Vec::new(),
    }
}

fn first_column(layout: &Layout<u32>) -> Column<u32> {
    match &layout.root {
        Some(Node::Split { first, .. }) => match first.as_ref() {
            Node::Dock { columns, .. } => columns.first().cloned().unwrap(),
            other => panic!("no dock: {other:?}"),
        },
        other => panic!("no split: {other:?}"),
    }
}

fn groups(layout: &Layout<u32>) -> Vec<Vec<u32>> {
    first_column(layout).groups.iter().map(|g| g.panels.clone()).collect()
}

fn hit(d: &Driver, hit: Hit<u32>) -> Rect {
    d.hit(&hit).unwrap_or_else(|| panic!("{hit:?} in {:?}", d.hits()))
}

fn open(d: &Driver) -> Option<u32> {
    flyout::<u32>(&d.ctx, d.id)
}

fn texts(d: &Driver) -> Vec<String> {
    d.shapes()
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
            _ => None,
        })
        .collect()
}

const CANVAS: Pos2 = pos2(600.0, 400.0);

#[test]
fn collapsed_column_shows_one_icon_per_panel_with_group_separators() {
    let d = DockDriver::new(docked(&[&[1, 2], &[3]], true), Content);
    let (one, two, three) = (hit(&d, Hit::Icon(1)), hit(&d, Hit::Icon(2)), hit(&d, Hit::Icon(3)));
    let (grip1, grip3) = (hit(&d, Hit::IconGrip(1)), hit(&d, Hit::IconGrip(3)));
    assert_eq!(one.width(), 36.0, "the column takes the style's iconic width");
    assert_eq!(one.bottom(), two.top(), "a group's icons follow each other");
    assert_eq!(three.top() - two.bottom(), 7.0, "groups are apart by the separator, with no room for a grip");
    assert_eq!((grip1.top(), grip1.bottom()), (one.top(), one.top() + 4.0), "the first group's band is its top edge");
    assert_eq!((grip3.top(), grip3.bottom()), (two.bottom(), three.top() + 4.0), "a later group's band covers the separator");
    assert!(d.hit(&Hit::Tab(1)).is_none(), "an iconic column draws no tabs");
}

#[test]
fn widening_an_iconic_column_shows_labels() {
    let mut d = DockDriver::new(docked(&[&[1, 2], &[3]], true), Content);
    assert!(!texts(&d).iter().any(|t| t == "Panel 1"));
    let edge = hit(&d, Hit::Divider(Divider::Column(1)));
    let out = d.drag(edge.center(), pos2(150.0, edge.center().y));
    assert!(out.error.is_none());
    let width = first_column(&d.layout).iconic_width.unwrap();
    assert!((100.0..=160.0).contains(&width), "{width}");
    d.settle();
    let labels = texts(&d);
    assert!(["Panel 1", "Panel 2", "Panel 3"].iter().all(|l| labels.iter().any(|t| t == l)), "{labels:?}");
    let edge = hit(&d, Hit::Divider(Divider::Column(1)));
    d.drag(edge.center(), pos2(20.0, edge.center().y));
    d.settle();
    assert_eq!(first_column(&d.layout).iconic_width, Some(36.0));
    assert!(!texts(&d).iter().any(|t| t == "Panel 1"));
}

#[test]
/// An icon click activates its panel and opens a canvas-side flyout; another
/// icon switches it, and the same icon closes it even with auto-collapse off. Escape
/// and outside presses close it only with auto-collapse on; a programmatic raise opens it.
fn icon_click_opens_a_flyout_on_the_canvas_side_and_auto_collapse_closes_it() {
    let mut d = DockDriver::new(docked(&[&[1, 2], &[3]], true), Content);
    d.auto_collapse = true;
    let icon = hit(&d, Hit::Icon(2)).center();
    let out = d.click(icon);
    assert!(out.actions.contains(&Action::Activate { panel: 2 }));
    d.settle();
    assert_eq!(open(&d), Some(2));
    let column = hit(&d, Hit::Icon(1));
    let tab = hit(&d, Hit::Tab(2));
    assert!(tab.left() >= column.right(), "the flyout opens on the canvas side: {tab:?}");
    assert!(tab.top() <= hit(&d, Hit::IconGrip(1)).bottom(), "beside its group");
    assert_eq!(first_column(&d.layout).groups.first().map(|g| g.active), Some(1));

    // Another icon switches flyouts, the same icon closes.
    d.click(hit(&d, Hit::Icon(3)).center());
    d.settle();
    assert_eq!(open(&d), Some(3));
    assert!(d.hit(&Hit::Tab(2)).is_none());
    d.click(hit(&d, Hit::Icon(3)).center());
    d.settle();
    assert_eq!(open(&d), None);

    // Escape and a press outside close it.
    d.click(icon);
    d.settle();
    d.key(Key::Escape);
    d.settle();
    assert_eq!(open(&d), None);
    d.click(icon);
    d.settle();
    let inside = hit(&d, Hit::Tab(2)).center();
    d.click(inside);
    d.settle();
    assert_eq!(open(&d), Some(2), "a press inside keeps it");
    d.click(CANVAS);
    d.settle();
    assert_eq!(open(&d), None);

    // Auto-Collapse off: it stays until an icon closes it.
    d.auto_collapse = false;
    d.click(icon);
    d.settle();
    d.key(Key::Escape);
    d.click(CANVAS);
    d.settle();
    assert_eq!(open(&d), Some(2));
    d.click(icon);
    d.settle();
    assert_eq!(open(&d), None);

    // U3: raising an iconic panel, as the Window menu does, opens its flyout.
    d.raise = Some(3);
    d.settle();
    assert_eq!(open(&d), Some(3));
}

#[test]
/// Choosing Auto-Collapse Iconic Panels from the iconic column's menu emits a
/// `DockEvent::ToggleAutoCollapse` event rather than mutating the layout directly.
fn auto_collapse_toggle_is_a_column_menu_event() {
    use egui_kittest::{Harness, kittest::Queryable};
    let layout = docked(&[&[1, 2]], true);
    let mut h = Harness::builder().with_size(vec2(800.0, 600.0)).build_ui_state(
        |ui, events: &mut Vec<DockEvent<u32>>| {
            let out = DockArea::new(egui::Id::unique("menu")).show_content(ui, &layout, &DockStyle::from_ui(ui), &mut Content);
            events.extend(out.events);
        },
        Vec::new(),
    );
    h.run_steps(2);
    let bar = h.ctx.read_response(egui::Id::unique("menu").with(("column-bar", Some(&1u32)))).unwrap().rect.center();
    h.input_mut().events.extend([
        egui::Event::PointerMoved(bar),
        egui::Event::PointerButton { pos: bar, button: egui::PointerButton::Secondary, pressed: true, modifiers: egui::Modifiers::NONE },
    ]);
    h.run_steps(1);
    h.input_mut().events.push(egui::Event::PointerButton {
        pos: bar,
        button: egui::PointerButton::Secondary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(2);
    h.get_by_label("Auto-Collapse Iconic Panels").click();
    h.run_steps(2);
    assert_eq!(h.state(), &vec![DockEvent::ToggleAutoCollapse]);
}

#[test]
fn dragging_a_flyout_strip_floats_its_group() {
    let mut d = DockDriver::new(docked(&[&[1, 2], &[3]], true), Content);
    d.click(hit(&d, Hit::Icon(1)).center());
    d.settle();
    // A one-line body: the flyout and the floated group take the least fitted height.
    let flyout = d.ctx.memory(|m| m.area_rect(d.id.with("flyout"))).unwrap();
    assert_eq!(flyout.height(), 120.0);
    let bar = hit(&d, Hit::DragBar(1));
    let out = d.drag(bar.center(), CANVAS);
    let [Action::MoveGroup { panel: 1, to: Target::Float { rect: Some(rect) } }] = out.actions.as_slice() else { panic!("{:?}", out.actions) };
    assert_eq!(rect[3], 120.0);
    assert_eq!(groups(&d.layout), vec![vec![3]]);
    assert_eq!(d.layout.floating.len(), 1);
    assert_eq!(d.layout.floating.first().map(|w| w.panels().copied().collect::<Vec<_>>()), Some(vec![1, 2]));
    d.settle();
    assert_eq!(open(&d), None, "the flyout closes with its group gone");

    // A tab dragged out of a flyout floats that panel alone.
    d.click(hit(&d, Hit::Icon(3)).center());
    d.settle();
    let tab = hit(&d, Hit::Tab(3));
    let out = d.drag(tab.center(), pos2(450.0, 200.0));
    assert!(matches!(out.actions.as_slice(), [Action::Float { panel: 3, .. }]), "{:?}", out.actions);
}

#[test]
fn icon_and_icon_group_grip_drags_move_a_panel_and_a_group() {
    let mut d = DockDriver::new(docked(&[&[1, 2], &[3]], true), Content);
    let before = d.layout.clone();
    // Escape, focus loss and cancel_drag end an icon drag with no change.
    let icon = hit(&d, Hit::Icon(2)).center();
    d.hold(icon, CANVAS);
    d.key(Key::Escape);
    d.release(CANVAS);
    d.hold(icon, CANVAS);
    d.focus(false);
    d.focus(true);
    d.release(CANVAS);
    d.hold(icon, CANVAS);
    cancel_drag::<u32>(&d.ctx, d.id);
    d.release(CANVAS);
    assert_eq!(d.layout, before);

    let out = d.drag(icon, CANVAS);
    let Some(Action::Float { panel: 2, rect }) = out.actions.first() else { panic!("{:?}", out.actions) };
    assert_eq!((rect[2], rect[3]), (240.0, 320.0), "a panel floated from its icon takes the column's expanded width");
    assert_eq!(groups(&d.layout), vec![vec![1], vec![3]]);

    let grip = hit(&d, Hit::IconGrip(3)).center();
    let out = d.drag(grip, pos2(450.0, 250.0));
    assert!(matches!(out.actions.as_slice(), [Action::MoveGroup { panel: 3, to: Target::Float { .. } }]), "{:?}", out.actions);
    assert_eq!(groups(&d.layout), vec![vec![1]]);
}

#[test]
fn group_dropped_on_an_iconic_column_joins_as_an_icon_group() {
    let mut layout = docked(&[&[1], &[2]], true);
    layout.floating.push(Floating::single(vec![5, 6], 0, [300.0, 200.0, 240.0, 200.0]));
    let mut d = DockDriver::new(layout, Content);
    let bar = hit(&d, Hit::DragBar(5)).center();
    let one = hit(&d, Hit::Icon(1));
    let target = pos2(one.center().x, one.bottom() - 2.0);
    d.hold(bar, target);
    let column = Rect::from_min_max(pos2(0.0, 14.0), pos2(36.0, 600.0));
    assert!(d.outlined(column.shrink(1.0)), "the column is outlined");
    let line = Rect::from_x_y_ranges(column.x_range(), (one.bottom() - 2.0)..=(one.bottom() + 2.0));
    assert!(d.filled(line), "with the insertion line under group 1");
    let out = d.release(target);
    assert!(
        out.actions
            .contains(&Action::MoveGroup { panel: 5, to: Target::At { anchor: 1, placement: Placement::Group { below: true, height: None } } }),
        "{:?}",
        out.actions
    );
    assert_eq!(groups(&d.layout), vec![vec![1], vec![5, 6], vec![2]]);
    assert!(first_column(&d.layout).iconic);
}

/// A group dropped on the window edge beside an iconic column at that edge docks in a new column
/// there: the preview is the edge line alone, without the column's outline.
#[test]
fn an_edge_drop_next_to_an_iconic_column_highlights_only_the_edge() {
    let mut layout = docked(&[&[1], &[2]], true);
    layout.floating.push(Floating::single(vec![5, 6], 0, [300.0, 200.0, 240.0, 200.0]));
    let mut d = DockDriver::new(layout, Content);
    let bar = hit(&d, Hit::DragBar(5)).center();
    let target = pos2(2.0, 300.0);
    d.hold(bar, target);
    let column = Rect::from_min_max(pos2(0.0, 14.0), pos2(36.0, 600.0));
    assert!(d.filled(Rect::from_x_y_ranges(0.0..=4.0, 0.0..=600.0)), "the edge line");
    assert!(!d.outlined(column.shrink(1.0)), "the iconic column is not outlined");
    d.release(target);
    let Some(Node::Split { first, .. }) = &d.layout.root else { panic!("{:?}", d.layout.root) };
    let Node::Dock { columns, .. } = first.as_ref() else { panic!("{first:?}") };
    let panels: Vec<Vec<u32>> = columns.iter().map(|c| c.groups.iter().flat_map(|g| g.panels.clone()).collect()).collect();
    assert_eq!(panels, vec![vec![5, 6], vec![1, 2]]);
    assert!(!columns[0].iconic && columns[1].iconic);
}

/// Every kind of gesture source has a rect, and dragging from it does something.
#[test]
fn hit_rects_cover_every_gesture_source() {
    let mut expanded = Column::new(vec![Group::new(vec![1, 2, 3, 4, 5, 6, 7], 0), Group::new(vec![8], 0)], 150.0);
    expanded.width = 150.0;
    let mut icons = Column::new(vec![Group::new(vec![9], 0)], 240.0);
    icons.iconic = true;
    let dock = Node::Dock { side: Side::Start, columns: vec![icons, expanded] };
    let stack = Node::Stack { entries: vec![StackEntry { panel: 10, open: true, height: None }, StackEntry { panel: 11, open: true, height: None }] };
    let frames = Node::Split {
        axis: SplitAxis::Vertical,
        size: SplitSize::Ratio(0.5),
        first: Box::new(Node::Tabs { panels: vec![0], active: 0 }),
        second: Box::new(stack),
    };
    let window = Floating::stack(vec![Group::new(vec![12], 0), Group::new(vec![13], 0)], [500.0, 60.0, 220.0, 300.0]).unwrap();
    let layout = Layout {
        root: Some(Node::Split { axis: SplitAxis::Horizontal, size: SplitSize::FixedFirst(200.0), first: Box::new(dock), second: Box::new(frames) }),
        floating: vec![window],
    };
    let d = DockDriver::new(layout.clone(), Content);
    let hits = d.hits();
    let kinds = |f: &dyn Fn(&Hit<u32>) -> bool| hits.iter().filter(|(h, _)| f(h)).count();
    let expected: [Check<'_>; 15] = [
        ("tab", &|h| matches!(h, Hit::Tab(_))),
        ("drag bar", &|h| matches!(h, Hit::DragBar(_))),
        ("border", &|h| matches!(h, Hit::Border(_))),
        ("overflow", &|h| matches!(h, Hit::Overflow(_))),
        ("column bar", &|h| matches!(h, Hit::ColumnBar(_))),
        ("collapse", &|h| matches!(h, Hit::Collapse(_))),
        ("stack toggle", &|h| matches!(h, Hit::StackToggle(_))),
        ("title", &|h| matches!(h, Hit::Title(12))),
        ("close", &|h| matches!(h, Hit::Close(12))),
        ("icon", &|h| matches!(h, Hit::Icon(9))),
        ("icon grip", &|h| matches!(h, Hit::IconGrip(9))),
        ("resize", &|h| matches!(h, Hit::Resize(12, _))),
        ("split", &|h| matches!(h, Hit::Divider(Divider::Split(_)))),
        ("column divider", &|h| matches!(h, Hit::Divider(Divider::Column(_)))),
        ("group divider", &|h| matches!(h, Hit::Divider(Divider::Group(_)))),
    ];
    for (name, kind) in expected {
        assert!(kinds(kind) > 0, "{name}: {hits:?}");
    }
    assert!(hits.iter().any(|(h, _)| matches!(h, Hit::Divider(Divider::Stack(10)))));
    assert_eq!(hits.iter().filter(|(h, _)| matches!(h, Hit::Resize(..))).count(), 8);
    for (hit, rect) in hits.iter().filter(|(h, _)| !matches!(h, Hit::Overflow(_) | Hit::Collapse(_) | Hit::StackToggle(_) | Hit::Close(_))) {
        let mut d = DockDriver::new(layout.clone(), Content);
        let from = rect.center();
        let moved = d.hold(from, from + vec2(-30.0, 40.0));
        let dragging = is_dragging::<u32>(&d.ctx, d.id);
        let released = d.release(from + vec2(-30.0, 40.0));
        assert!(dragging || !moved.actions.is_empty() || !released.actions.is_empty(), "{hit:?} at {rect:?} does nothing");
    }
    for (hit, rect) in hits.iter().filter(|(h, _)| matches!(h, Hit::Overflow(_) | Hit::Collapse(_) | Hit::StackToggle(_) | Hit::Close(_))) {
        let mut d = DockDriver::new(layout.clone(), Content);
        let out = d.click(rect.center());
        let opened = egui::Popup::is_any_open(&d.ctx);
        assert!(opened || !out.actions.is_empty(), "{hit:?} at {rect:?} does nothing");
    }
}

/// Paints panel 1's icon as a dot in code; panel 2 has neither a painted icon nor an image.
struct Painted;

impl DockContent<u32> for Painted {
    fn body(&mut self, _: &mut egui::Ui, _: &u32) {}
    fn logical_label(&self, panel: &u32) -> String {
        format!("Panel {panel}")
    }
    fn paint_icon(&mut self, painter: &egui::Painter, rect: Rect, panel: &u32, color: egui::Color32) -> bool {
        if *panel != 1 {
            return false;
        }
        painter.circle_filled(rect.center(), 3.0, color);
        true
    }
}

#[test]
fn an_icon_painted_in_code_replaces_the_initial_and_others_fall_back() {
    let d = DockDriver::new(docked(&[&[1, 2]], true), Painted);
    let (one, two) = (d.hit(&Hit::Icon(1)).unwrap(), d.hit(&Hit::Icon(2)).unwrap());
    let dots: Vec<Pos2> = d
        .shapes()
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Circle(circle) if circle.radius == 3.0 => Some(circle.center),
            _ => None,
        })
        .collect();
    assert_eq!(dots.len(), 1, "{dots:?}");
    assert!(one.contains(dots[0]) && !two.contains(dots[0]), "the dot sits in panel 1's cell: {dots:?}");
    let initials: Vec<Pos2> = d
        .shapes()
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.text() == "P" => Some(text.pos),
            _ => None,
        })
        .collect();
    assert_eq!(initials.len(), 1, "only panel 2 draws its initial: {initials:?}");
    assert!(two.contains(initials[0]));
}

#[test]
fn chrome_has_focus_covers_every_drawn_control() {
    let mut layout = docked(&[&[1, 2], &[3]], true);
    layout.floating.push(Floating::single(vec![5, 6], 0, [400.0, 100.0, 240.0, 200.0]));
    let mut d = Driver::new(layout, Content);
    d.settle();
    let hits = d.hits();
    assert!(hits.iter().any(|(h, _)| matches!(h, Hit::Icon(_))), "{hits:?}");
    assert!(hits.iter().any(|(h, _)| matches!(h, Hit::IconGrip(_))), "{hits:?}");
    assert!(hits.iter().any(|(h, _)| matches!(h, Hit::Resize(_, ResizeEdge::Left))), "{hits:?}");
    for (hit, _) in hits {
        d.ctx.memory_mut(|m| m.request_focus(DockArea::<u32>::hit_id(d.id, &hit)));
        assert!(DockArea::<u32>::new(d.id).chrome_has_focus(&d.ctx, &d.layout), "{hit:?}");
    }
}

#[test]
fn floating_windows_stay_inside_the_float_bounds() {
    // The app's title bar strip (top 30 pt) is outside the bounds: a window saved there is
    // drawn below it, and a title bar drag towards it stops at its edge. Command keeps the
    // drag from docking.
    let mut layout = docked(&[&[1]], false);
    layout.floating.push(Floating::single(vec![5], 0, [400.0, 0.0, 240.0, 200.0]));
    let mut d = DockDriver::new(layout, Content);
    d.float_bounds = Some(Rect::from_min_max(pos2(0.0, 30.0), pos2(800.0, 600.0)));
    d.settle();
    let title = hit(&d, Hit::Title(5));
    assert!(title.top() >= 30.0, "{title:?}");
    d.modifiers = egui::Modifiers::COMMAND;
    d.drag(title.center(), pos2(title.center().x, 2.0));
    d.modifiers = egui::Modifiers::NONE;
    d.settle();
    let title = hit(&d, Hit::Title(5));
    assert!(title.top() >= 30.0, "{title:?}");
}
