//! Trials of `IconicColumn`: the letter of a panel without an icon, the names a wide column shows, and the resizes
//! of its flyouts.
#![allow(clippy::panic, clippy::expect_used, reason = "trials assert by panicking")]

use crate::contract::{Bind, Show as _, ShowContainer as _};
use crate::docking::{DockHeights, DockSizes, DragUnit, FlyoutResize, GroupAt, GroupGrip, IconicColumn, PanelKey, PanelSpec};
use crate::sections::Panel;
use crate::style::StyleCx;
use crate::testing::memory::global;
use crate::testing::{Fixture, trial};
use craft_behavior::dock::{Action, Column, DockEdge, Group, Layout, Node};
use egui::{Pos2, Rect, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

fn spec(panel: &u8) -> PanelSpec {
    PanelSpec::new(if *panel == 0 { "Brushes" } else { "Swatches" })
}

/// The width of the column that shows names beside its icons.
const WIDE_COLUMN: f32 = 200.0;

/// A column `width` wide over two icon-less panels, and the panel whose flyout is open.
fn column(width: f32) -> (Fixture, Rc<Cell<Option<u8>>>) {
    let open = Rc::new(Cell::new(None));
    let seen = Rc::clone(&open);
    let mut fixture = Fixture::new(move |ui| {
        let column = Column::new(vec![Group::new(vec![0, 1], 0)]);
        let mut now = seen.get();
        IconicColumn::new("Panels", global("iconic column", "trial"), &column, Bind::bound(&mut now), &spec).show_flyouts(ui, |_, at, ui, _| {
            let GroupAt::Show(rect) = at else { return None };
            Panel::new("Flyout").show_at(ui, rect, |_, _| ());
            None
        });
        seen.set(now);
    });
    fixture.room(Vec2::new(width, 300.0)).settle();
    (fixture, open)
}

/// A panel without an icon shows the first letter of its name; a column wider than the label width shows the names
/// too, and a click on a name opens that panel's flyout as a click on its icon does.
#[trial]
fn an_icon_less_panel_shows_its_initial_and_a_wide_column_shows_names_that_open_flyouts() {
    let painted = |fixture: &Fixture, text: &str| fixture.texts().iter().any(|run| run.text == text);
    let (narrow, _) = column(40.0);
    assert!(painted(&narrow, "B") && painted(&narrow, "S"), "each icon-less panel shows its initial");
    assert!(!painted(&narrow, "Brushes"), "a narrow column shows no names");

    let (mut wide, open) = column(WIDE_COLUMN);
    assert!(painted(&wide, "Brushes") && painted(&wide, "Swatches"), "a wide column shows the names");
    let icon = wide.by_label("Swatches").expect("the Swatches icon");
    assert!(icon.rect.width() < WIDE_COLUMN, "the icon keeps its square at the start: {:?}", icon.rect);
    // The name has no node of its own: the icon is its keyboard stop. Click on it, past the icon.
    wide.click_at(icon.rect.right_center() + Vec2::new(icon.rect.width(), 0.0)).idle(3);
    assert_eq!(open.get(), Some(1), "a click on the name opens its panel's flyout");
}

/// The room the flyout trials run in.
const SCREEN: Vec2 = Vec2::new(800.0, 600.0);
/// The width of the iconic column in the flyout trials.
const RAIL: f32 = 40.0;
/// The height of the tab strip a flyout's group shows at its top, which drags the group.
const STRIP: f32 = 24.0;

/// The panels of the flyout trials, each asking for more than the least height so a flyout can shrink.
fn named(panel: &u8) -> PanelSpec {
    let name = match panel {
        1 => "Brushes",
        2 => "Layers",
        3 => "Swatches",
        _ => "History",
    };
    PanelSpec::new(name).heights(DockHeights::new(160.0))
}

fn screen() -> Rect {
    Rect::from_min_size(Pos2::ZERO, SCREEN)
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1.0
}

/// A point just inside the flyout's top edge, `x` along it.
fn top_edge(rect: Rect, x: f32) -> Pos2 {
    Pos2::new(x, rect.top() + 0.5)
}

/// The dock's one column.
fn first_column(layout: &Layout<u8>) -> Column<u8> {
    match &layout.root {
        Some(Node::Dock { columns, .. }) => columns.first().cloned().expect("the dock has its column"),
        other => panic!("no dock: {other:?}"),
    }
}

/// What the dock area makes of a flyout's resize (its `column_work`): the group's flyout width and height, and the
/// top's offset when the top moved.
fn take(layout: &mut Layout<u8>, resize: FlyoutResize<u8>) {
    let panel = resize.panel;
    let mut actions = Vec::new();
    if let Some(width) = resize.width {
        actions.push(Action::resize_flyout_width(panel, Some(width)));
    }
    if let Some(height) = resize.height {
        actions.push(Action::resize_group(panel, Some(height)));
    }
    if resize.offset.is_some() {
        actions.push(Action::move_flyout_top(panel, resize.offset));
    }
    for action in actions {
        layout.apply(action).expect("the dock takes the flyout's size");
    }
}

/// An iconic column on one window edge over a layout the trial holds, which takes every flyout resize the column
/// reports as the dock area does. The flyout's group shows a strip at its top that drags the group.
struct Flyouts {
    fixture: Fixture,
    layout: Rc<RefCell<Layout<u8>>>,
    /// The rect the open flyout's group showed in, last pass.
    shown: Rc<Cell<Option<Rect>>>,
    /// A drag of the group started on its strip.
    strip_dragged: Rc<Cell<bool>>,
}

impl Flyouts {
    /// A column of `groups` on the right window edge when `right`, else the left one; its flyouts stay in `within`.
    fn new(groups: &[&[u8]], right: bool, within: Rect) -> Self {
        let mut column = Column::new(groups.iter().map(|panels| Group::new(panels.to_vec(), 0)).collect());
        column.iconic = true;
        let edge = if right { DockEdge::End } else { DockEdge::Start };
        let layout = Rc::new(RefCell::new(Layout::new(Some(Node::dock(edge, vec![column])), Vec::new())));
        let (shown, strip_dragged) = (Rc::new(Cell::new(None)), Rc::new(Cell::new(false)));
        let (held, seen, dragged, open) = (Rc::clone(&layout), Rc::clone(&shown), Rc::clone(&strip_dragged), Cell::new(None));
        let place = if right {
            Rect::from_min_max(Pos2::new(SCREEN.x - RAIL, 0.0), SCREEN.to_pos2())
        } else {
            Rect::from_min_size(Pos2::ZERO, Vec2::new(RAIL, SCREEN.y))
        };
        let mut fixture = Fixture::new(move |ui| {
            let column = first_column(&held.borrow());
            let mut now = open.get();
            let groups = RefCell::new(Vec::new());
            let output = IconicColumn::new("Panels", global("iconic column", "flyouts"), &column, Bind::bound(&mut now), &named)
                .edge(edge)
                .within(within)
                .show_flyouts_at(ui, place, |group, at, ui, _| {
                    let GroupAt::Show(rect) = at else { return None };
                    let panel = *group.active_panel()?;
                    groups.borrow_mut().push((panel, rect));
                    let strip = Rect::from_min_size(rect.min, Vec2::new(rect.width(), STRIP.min(rect.height())));
                    let grip = GroupGrip::new(DragUnit::Group(panel), PanelKey::of(&panel)).show_at(ui, strip);
                    dragged.set(dragged.get() || grip.drag_started);
                    None
                });
            // A flyout closed this pass still shows its group while it fades out: the open panel's group is the one.
            seen.set(groups.into_inner().into_iter().rev().find(|(panel, _)| Some(*panel) == now).map(|(_, rect)| rect));
            open.set(now);
            if let Some(resize) = output.flyout {
                take(&mut held.borrow_mut(), resize);
            }
        });
        fixture.room(SCREEN).settle();
        Flyouts { fixture, layout, shown, strip_dragged }
    }

    /// The open flyout's rect.
    fn rect(&self) -> Rect {
        self.shown.get().expect("a flyout is open")
    }

    /// The rect of the icon of the panel named `label`.
    fn icon_rect(&self, label: &str) -> Rect {
        self.fixture.by_label(label).expect("the panel's icon").rect
    }

    /// Clicks the icon of the panel named `label`.
    fn open(&mut self, label: &str) {
        let at = self.icon_rect(label).center();
        self.fixture.click_at(at).settle();
    }

    fn drag(&mut self, from: Pos2, by: Vec2) {
        self.fixture.drag_at(from, by).settle();
    }

    /// Applies `action` to the layout, as the host does for a change made elsewhere (the Window menu, a load).
    fn apply(&mut self, action: Action<u8>) {
        self.layout.borrow_mut().apply(action).expect("the action applies");
        self.fixture.settle();
    }

    /// The column's group `at` in the layout.
    fn group(&self, at: usize) -> Group<u8> {
        first_column(&self.layout.borrow()).groups.get(at).cloned().expect("the column has the group")
    }

    fn sizes(&self) -> DockSizes {
        StyleCx::with_root(self.fixture.ctx(), DockSizes::style)
    }

    /// Half the side of a resize grip, which straddles the edge it moves.
    fn grip_reach(&self) -> f32 {
        StyleCx::with_root(self.fixture.ctx(), |cx| cx.tokens.size.handle.sm) / 2.0
    }
}

/// The top edge and the free top corner grow the flyout upward on either window edge: the bottom and the column-side
/// edge stay, the column-side edge has no grip, and the new top is kept as an offset from the icons, so it survives
/// another group's flyout and reopening.
#[trial]
fn flyout_top_edge_and_free_top_corner_grow_it_upward() {
    for right in [false, true] {
        let mut d = Flyouts::new(&[&[1, 2], &[3]], right, screen());
        d.open("Swatches");
        let before = d.rect();
        assert!(before.top() > 40.0, "room above the second group's flyout: {before:?}");
        let from = top_edge(before, before.center().x);
        d.drag(from, Vec2::new(0.0, -30.0));
        let raised = d.rect();
        assert!(near(raised.top(), before.top() - 30.0), "right={right}: {before:?} -> {raised:?}");
        assert!(near(raised.bottom(), before.bottom()), "right={right}: {before:?} -> {raised:?}");
        assert!(near(raised.width(), before.width()), "right={right}: {before:?} -> {raised:?}");
        let group = d.group(1);
        assert!(group.height.is_some_and(|height| near(height, raised.height())), "{:?} for {raised:?}", group.height);
        assert!(group.flyout_offset.is_some_and(|offset| near(offset, -30.0)), "{:?}", group.flyout_offset);
        assert_eq!(group.flyout_width, None, "a vertical drag leaves the width following the column");
        assert_eq!(d.group(0).flyout_offset, None, "another group keeps its own placement");

        // The edge facing the column has no grip: a drag there leaves the flyout as it is.
        let (column_side, away) =
            if right { (raised.right_center() - Vec2::new(0.5, 0.0), 30.0) } else { (raised.left_center() + Vec2::new(0.5, 0.0), -30.0) };
        d.drag(column_side, Vec2::new(away, 0.0));
        assert_eq!(d.rect(), raised, "right={right}: the column-side edge does not resize");

        // The free top corner moves the canvas-side edge and the top together.
        let corner = if right { raised.left_top() + Vec2::new(2.0, 0.5) } else { raised.right_top() + Vec2::new(-2.0, 0.5) };
        let direction = if right { -1.0 } else { 1.0 };
        d.drag(corner, Vec2::new(direction * 40.0, -10.0));
        let grown = d.rect();
        assert!(near(grown.width(), raised.width() + 40.0), "right={right}: {raised:?} -> {grown:?}");
        assert!(near(grown.top(), raised.top() - 10.0), "right={right}: {raised:?} -> {grown:?}");
        assert!(near(grown.bottom(), before.bottom()), "right={right}: {raised:?} -> {grown:?}");
        let column_edge = |rect: Rect| if right { rect.right() } else { rect.left() };
        assert!(near(column_edge(grown), column_edge(before)), "right={right}: the column side stays");

        // Another group's flyout keeps its own placement; this one comes back where it was.
        d.open("Brushes");
        assert!(near(d.rect().top(), d.icon_rect("Brushes").top()), "another group stays aligned with its icons");
        d.open("Swatches");
        assert_eq!(d.rect(), grown, "right={right}: reopening gives the same rect");
    }
}

/// The top grip is the standard one, straddling the edge: a press inside it resizes; a press below it reaches the
/// tab strip and drags the group.
#[trial]
fn the_flyout_top_handle_is_a_band_that_leaves_the_strip_its_drags() {
    let mut d = Flyouts::new(&[&[1], &[3]], false, screen());
    d.open("Swatches");
    let before = d.rect();
    let reach = d.grip_reach();
    let from = Pos2::new(before.center().x, before.top() + reach - 1.0);
    d.drag(from, Vec2::new(0.0, -20.0));
    let raised = d.rect();
    assert!(near(raised.top(), before.top() - 20.0), "{before:?} -> {raised:?}");
    assert!(near(raised.bottom(), before.bottom()), "{before:?} -> {raised:?}");
    assert!(!d.strip_dragged.get(), "a press inside the grip leaves the strip alone");
    let below = Pos2::new(raised.center().x, raised.top() + reach + 2.0);
    assert!(below.y < raised.top() + STRIP, "the press lands on the strip");
    d.drag(below, Vec2::new(200.0, 100.0));
    assert!(d.strip_dragged.get(), "a press below the grip drags the group by its strip");
}

/// A saved offset that would put the flyout above the bounds or below them is clamped there, and a group without one
/// aligns its flyout with its icons.
#[trial]
fn a_saved_flyout_offset_stays_inside_the_bounds() {
    let within = screen();
    let mut d = Flyouts::new(&[&[1], &[3]], false, within);
    d.open("Swatches");
    let aligned = d.rect();
    assert!(near(aligned.top(), d.icon_rect("Swatches").top()), "with no offset the flyout aligns with its icons");
    for offset in [-100_000.0, 100_000.0] {
        d.apply(Action::move_flyout_top(3, Some(offset)));
        let rect = d.rect();
        assert!(within.contains_rect(rect), "{offset}: {rect:?} in {within:?}");
        assert!(near(rect.height(), aligned.height()), "{offset}: {aligned:?} -> {rect:?}");
    }
    d.apply(Action::move_flyout_top(3, None));
    assert_eq!(d.rect(), aligned);
}

/// Dragging the top edge down past the minimum stops at the minimum height with the bottom fixed; the saved offset
/// matches where the top stopped, and reopening keeps that rectangle.
#[trial]
fn a_top_edge_drag_down_stops_at_the_minimum_height() {
    let mut d = Flyouts::new(&[&[1], &[3]], false, screen());
    d.open("Swatches");
    let before = d.rect();
    let from = top_edge(before, before.center().x);
    d.drag(from, Vec2::new(0.0, 2000.0));
    let small = d.rect();
    let sizes = d.sizes();
    assert!(near(small.height(), sizes.strip + sizes.least), "{before:?} -> {small:?}");
    assert!(near(small.bottom(), before.bottom()), "{before:?} -> {small:?}");
    let group = d.group(1);
    let icons = d.icon_rect("Swatches").top();
    assert!(group.flyout_offset.is_some_and(|offset| near(offset, small.top() - icons)), "{:?} for {small:?}", group.flyout_offset);
    assert!(group.height.is_some_and(|height| near(height, small.height())), "{:?} for {small:?}", group.height);
    d.open("Brushes");
    d.open("Swatches");
    assert_eq!(d.rect(), small);
}

/// In bounds shorter than the least height, a flyout with a saved offset stays inside them, a top drag cannot push
/// it out, and its width still resizes.
#[trial]
fn a_flyout_with_an_offset_in_a_short_viewport_stays_inside_and_resizes_its_width() {
    for height in [20.0, 50.0] {
        let bounds = Rect::from_min_size(Pos2::ZERO, Vec2::new(SCREEN.x, height));
        let mut d = Flyouts::new(&[&[1]], false, bounds);
        d.apply(Action::move_flyout_top(1, Some(-300.0)));
        d.open("Brushes");
        let before = d.rect();
        assert!(bounds.contains_rect(before), "{height}: {before:?}");
        let from = top_edge(before, before.center().x);
        d.drag(from, Vec2::new(0.0, -500.0));
        assert!(bounds.contains_rect(d.rect()), "{height}: {:?}", d.rect());
        let corner = d.rect().right_bottom() - Vec2::new(2.0, 2.0);
        d.drag(corner, Vec2::new(60.0, 0.0));
        let after = d.rect();
        assert!(near(after.width(), before.width() + 60.0), "{height}: {before:?} -> {after:?}");
        assert!(bounds.contains_rect(after), "{height}: {after:?}");
    }
}
