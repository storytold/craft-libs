//! Trials of `IconicColumn`: the letter of a panel without an icon, the names a wide column shows, the grips of its
//! groups, the resizes of its flyouts, and a flyout that a command leaves open.
#![allow(clippy::panic, clippy::expect_used, reason = "trials assert by panicking")]

use super::{CELL, GRIP};
use crate::buttons::Button;
use crate::contract::overlay::{self, OpenFilter};
use crate::contract::{Bind, Show as _, ShowContainer as _};
use crate::docking::group::GRIP_ABOVE;
use crate::docking::{
    DockHeights, DockSizes, DockSpot, DockTargets, DragUnit, FlyoutResize, GroupAt, GroupDrag, GroupGrip, IconGroupSeparator, IconicColumn, PanelKey,
    PanelSpec,
};
use crate::runtime::commands::{AppCatalog, Binding, CommandDef, CommandInput, Scope};
use crate::runtime::drag;
use crate::runtime::env::{Direction, with_env};
use crate::runtime::focus::{FocusTarget, request};
use crate::runtime::frame::take_output;
use crate::sections::Panel;
use crate::style::StyleCx;
use crate::tabs::PanelTabLook;
use crate::testing::memory::global;
use crate::testing::relations::{equals_token, grows_with};
use crate::testing::{Fixture, trial};
use craft_behavior::DockSide;
use craft_behavior::dock::{Action, Column, DockEdge, Group, Layout, Node};
use craft_behavior::overlay::Side;
use egui::{Id, Key, Modifiers, Pos2, Rect, UiBuilder, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The float slack of the grip comparisons: paint positions are sums of token values.
const EPS: f32 = 0.01;

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
        Self::with(groups, right, within, false)
    }

    /// [`Flyouts::new`] with Auto-Collapse Iconic Panels on when `auto_collapse`.
    fn with(groups: &[&[u8]], right: bool, within: Rect, auto_collapse: bool) -> Self {
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
                .opens(if right { Side::Start } else { Side::End })
                .within(within)
                .auto_collapse(auto_collapse)
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

    /// How far a resize grip reaches in from the edge it moves.
    fn grip_reach(&self) -> f32 {
        StyleCx::with_root(self.fixture.ctx(), |cx| crate::contract::grip_band(cx.tokens))
    }
}

/// A press outside the open flyout leaves it open; with Auto-Collapse Iconic Panels on it closes it.
#[trial]
fn a_press_outside_closes_the_flyout_only_with_auto_collapse() {
    for auto_collapse in [false, true] {
        let mut d = Flyouts::with(&[&[1, 2]], false, screen(), auto_collapse);
        d.open("Brushes");
        let away = d.rect().right_center() + Vec2::new(100.0, 0.0);
        d.fixture.click_at(away).settle();
        assert_eq!(d.shown.get().is_none(), auto_collapse, "auto-collapse {auto_collapse}: a press outside closes the flyout only with it on");
    }
}

/// The app command the trials bind to a key.
const NUDGE: &str = "flyoutTrial.nudge";

/// A key that runs an app command, with a stop in the open flyout focused, leaves the flyout open and live in every
/// pass and the stop focused, with Auto-Collapse Iconic Panels off or on: the host's binding opens and closes a flyout,
/// and a command closes only the transient popups (menus, dropdowns, lists).
#[trial]
fn a_key_command_leaves_the_open_flyout_and_the_focus_in_it() {
    for auto_collapse in [false, true] {
        let (open, stop, faded) = (Rc::new(Cell::new(None)), Rc::new(Cell::new(None)), Rc::new(Cell::new(false)));
        let (seen, shown, fade) = (Rc::clone(&open), Rc::clone(&stop), Rc::clone(&faded));
        let place = Rect::from_min_size(Pos2::ZERO, Vec2::new(RAIL, SCREEN.y));
        let mut fixture = Fixture::new(move |ui| {
            let column = Column::new(vec![Group::new(vec![0, 1], 0)]);
            let mut now = seen.get();
            IconicColumn::new("Panels", global("iconic column", "commands"), &column, Bind::bound(&mut now), &spec)
                .within(screen())
                .auto_collapse(auto_collapse)
                .show_flyouts_at(ui, place, |_, at, ui, _| {
                    let GroupAt::Show(rect) = at else { return None };
                    shown.set(Some(ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| Button::new("Snap").show(ui).id).inner));
                    None
                });
            // Last pass the open flyout was not among the live layers that take keys: it played its exit.
            fade.set(fade.get() || (now.is_some() && !overlay::any_open(ui.ctx(), OpenFilter::Keys)));
            seen.set(now);
        });
        let binding = Binding::parse("F9").expect("the trial's chord parses");
        let catalog = AppCatalog::new(vec![CommandDef::new(NUDGE, NUDGE, Scope::app()).with_default(binding)], 1).expect("the catalog is taken");
        fixture.commands(CommandInput::new().catalog(catalog)).room(SCREEN).settle();
        let icon = fixture.by_label("Swatches").expect("the Swatches icon").rect.center();
        fixture.click_at(icon).settle();
        let button = stop.get().expect("the flyout shows its stop");
        request(fixture.ctx(), FocusTarget::Id(button));
        fixture.settle();
        assert_eq!(fixture.focused_stop().map(|(id, _)| id), Some(button), "auto-collapse {auto_collapse}: the flyout's stop has the focus");
        faded.set(false);
        take_output(fixture.ctx());
        fixture.key(Key::F9, Modifiers::NONE).settle();
        let fired = take_output(fixture.ctx()).fired;
        assert!(fired.iter().any(|fired| fired.command.id == NUDGE), "auto-collapse {auto_collapse}: the key runs its command");
        assert_eq!(open.get(), Some(1), "auto-collapse {auto_collapse}: the flyout stays open");
        assert!(!faded.get(), "auto-collapse {auto_collapse}: and never plays its exit");
        assert_eq!(fixture.focused_stop().map(|(id, _)| id), Some(button), "auto-collapse {auto_collapse}: and its stop keeps the focus");
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

/// The top grip is the standard one, inside the edge: a press inside it resizes; a press below it reaches the
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

/// The names of the panels of the tall-body trials, which set no heights.
const TALL: [&str; 8] = ["One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight"];
/// What the body of a flyout in the tall-body trial measures.
const TALL_BODY: f32 = 300.0;
/// The room below the last group's icon in the short-window trial.
const ROOM_BELOW: f32 = 200.0;
/// How much shorter than that room the body of the flyout that fits measures, and how much taller the one that does not.
const OVERFLOW: f32 = 40.0;

/// What [`tall_flyouts`] returns: the fixture, the panel whose flyout is open (the trial sets it) and the rect the open
/// flyout's body showed in, last pass.
type TallFlyouts = (Fixture, Rc<Cell<Option<u8>>>, Rc<Cell<Option<Rect>>>);

/// A column of the [`TALL`] panels, each in a group of its own, on the left window edge; its flyouts stay inside the
/// bounds `within` holds and their bodies measure the height `body` holds. Returns the fixture, the panel whose flyout
/// is open (the trial sets it) and the rect the open flyout's body showed in, last pass.
fn tall_flyouts(within: &Rc<Cell<Rect>>, body: &Rc<Cell<f32>>) -> TallFlyouts {
    let (open, shown) = (Rc::new(Cell::new(None)), Rc::new(Cell::new(None)));
    let (bounds, measures, now, seen) = (Rc::clone(within), Rc::clone(body), Rc::clone(&open), Rc::clone(&shown));
    let mut column = Column::new((0..TALL.len() as u8).map(|panel| Group::new(vec![panel], 0)).collect());
    column.iconic = true;
    let mut fixture = Fixture::new(move |ui| {
        let current = now.get();
        let mut opened = current;
        IconicColumn::new("Panels", global("iconic column", "tall flyouts"), &column, Bind::bound(&mut opened), &|panel: &u8| {
            PanelSpec::new(TALL[usize::from(*panel) % TALL.len()])
        })
        .edge(DockEdge::Start)
        .opens(Side::End)
        .within(bounds.get())
        .show_flyouts_at(ui, Rect::from_min_size(Pos2::ZERO, Vec2::new(RAIL, SCREEN.y)), |group, at, ui, _| {
            match at {
                GroupAt::Measure(_) => return Some(measures.get()),
                GroupAt::Show(rect) => {
                    if group.active_panel().is_some_and(|panel| Some(*panel) == current) {
                        seen.set(Some(rect));
                    }
                    Panel::new("Flyout").show_at(ui, rect, |_, _| ());
                }
            }
            None
        });
        now.set(opened);
    });
    fixture.room(SCREEN).settle();
    (fixture, open, shown)
}

/// Closes the open flyout of [`tall_flyouts`] and opens the last group's.
fn reopen_last(fixture: &mut Fixture, open: &Cell<Option<u8>>) {
    open.set(None);
    fixture.settle();
    open.set(Some(TALL.len() as u8 - 1));
    fixture.settle();
}

/// A flyout whose body measures taller than the room below its group's icons moves up to stay inside the bounds, as
/// tall as the body (or as the bounds when those are shorter): it is not cut to the room below its top with the rest
/// of its body laid out below the bounds. The panels set no heights, so the group's height does not give the room.
#[trial]
fn a_flyout_taller_than_the_room_below_its_icons_moves_up_inside_the_bounds() {
    let (within, body) = (Rc::new(Cell::new(screen())), Rc::new(Cell::new(TALL_BODY)));
    let (mut fixture, open, shown) = tall_flyouts(&within, &body);
    let last = TALL[TALL.len() - 1];
    let icon = fixture.by_label(last).expect("the last group's icon").rect;
    for height in [icon.bottom() + 8.0, 200.0] {
        // The bounds end just below the last icon, or are shorter than the body.
        let bounds = Rect::from_min_size(Pos2::ZERO, Vec2::new(SCREEN.x, height));
        within.set(bounds);
        reopen_last(&mut fixture, &open);
        let rect = shown.get().expect("the last group's flyout is open");
        let wanted = TALL_BODY.min(bounds.height());
        assert!(bounds.bottom() - icon.top() < TALL_BODY, "the bounds leave less room below the icon than the body takes: {icon:?} in {bounds:?}");
        assert!(bounds.contains_rect(rect), "{height}: the flyout {rect:?} stays inside {bounds:?}");
        assert!(rect.height() > wanted || near(rect.height(), wanted), "{height}: the flyout {rect:?} takes the body's height {wanted}");
        assert!(rect.top() < icon.top(), "{height}: the flyout {rect:?} moves up from its icon {icon:?}");
    }
}

/// On a short window a flyout whose body fits below its group's icons keeps its top at the icons, and one whose body
/// is taller than the room below them moves up only by what it overflows, its bottom on the bounds' bottom: neither
/// takes the whole height of the bounds over the column's other icons.
#[trial]
fn a_flyout_on_a_short_window_moves_up_only_by_its_overflow() {
    let (within, body) = (Rc::new(Cell::new(screen())), Rc::new(Cell::new(TALL_BODY)));
    let (mut fixture, open, shown) = tall_flyouts(&within, &body);
    let icon = fixture.by_label(TALL[TALL.len() - 1]).expect("the last group's icon").rect;
    let bounds = Rect::from_min_size(Pos2::ZERO, Vec2::new(SCREEN.x, icon.top() + ROOM_BELOW));
    within.set(bounds);
    let sizes = StyleCx::with_root(fixture.ctx(), DockSizes::style);
    let least = sizes.strip + sizes.least;
    assert!(icon.top() - OVERFLOW > bounds.top(), "the bounds leave room above the icon for the taller flyout to move up: {icon:?}");
    for over in [-OVERFLOW, OVERFLOW] {
        let height = ROOM_BELOW + over;
        assert!(height > least, "{over}: the body {height} measures taller than the flyout's least height {least}");
        body.set(height);
        reopen_last(&mut fixture, &open);
        let rect = shown.get().expect("the last group's flyout is open");
        // A body that fits stays at the icon's top; a taller one moves up by its overflow alone.
        let top = icon.top() - over.max(0.0);
        assert!(near(rect.height(), height), "{over}: the flyout {rect:?} takes its body's height {height}");
        assert!(near(rect.top(), top), "{over}: the flyout {rect:?} has its top at {top}, below the icon {icon:?} or up by its overflow");
        assert!(bounds.contains_rect(rect), "{over}: the flyout {rect:?} stays inside {bounds:?}");
    }
}

/// Each icon group's grip strip has its own room at the group's top: across the group, below the bar or the group
/// before it and above the group's first icon, overlapping no icon. A drag that starts on a grip moves its whole
/// group, as a drag on an icon moves its panel.
#[trial]
fn a_groups_grip_lies_above_its_icons_and_its_drag_moves_the_group() {
    let overlap = |a: Rect, b: Rect| {
        let both = a.intersect(b);
        both.width() > 0.0 && both.height() > 0.0
    };
    for direction in [Direction::Ltr, Direction::Rtl] {
        let mut fixture = Fixture::hosted(|ui| {
            let mut column = Column::new(vec![Group::new(vec![1, 2], 0), Group::new(vec![3], 0)]);
            column.iconic = true;
            let place = Rect::from_min_size(Pos2::ZERO, Vec2::new(RAIL, SCREEN.y));
            let output = IconicColumn::new("Panels", global("iconic column", "grips"), &column, Bind::kept(), &named)
                .within(screen())
                .show_flyouts_at(ui, place, |_, _, _, _| None);
            Some(output.into())
        });
        fixture.direction(direction).room(SCREEN).settle();
        let id = fixture.root_id().expect("the column shows");
        let grips = [0, 1].map(|group| fixture.part_rect(id, GRIP.at(group)).expect("each group has its grip"));
        let icon_rect = |label: &str| fixture.by_label(label).expect("the panel's icon").rect;
        let (brushes, layers, swatches) = (icon_rect("Brushes"), icon_rect("Layers"), icon_rect("Swatches"));
        assert!(grips[0].top() > 0.0, "{direction:?}: the first grip is under the bar: {grips:?}");
        assert!(grips[0].bottom() <= brushes.top(), "{direction:?}: the first grip is above its group's first icon");
        assert!(layers.bottom() <= grips[1].top(), "{direction:?}: the second grip is below the first group");
        assert!(grips[1].bottom() <= swatches.top(), "{direction:?}: the second grip is above its group's first icon");
        for grip in grips {
            assert!(grip.x_range().contains(brushes.center().x), "{direction:?}: the grip runs across the group: {grip:?}");
            for cell in [brushes, layers, swatches] {
                assert!(!overlap(grip, cell), "{direction:?}: {grip:?} overlaps the icon {cell:?}");
            }
        }
        assert!(!overlap(grips[0], grips[1]), "{direction:?}: the grips overlap");

        for (grip, anchor) in grips.into_iter().zip([1, 3]) {
            fixture.drag_hold_at(grip.center(), Vec2::new(RAIL * 2.0, 0.0));
            let unit = drag::active::<GroupDrag<u8>>(fixture.ctx()).map(|drag| drag.unit.clone());
            assert_eq!(unit, Some(DragUnit::Group(anchor)), "{direction:?}: a drag on the grip moves its group");
            fixture.release_at(grip.center() + Vec2::new(RAIL * 2.0, 0.0)).settle();
        }
    }
}

/// The icon groups take drops as an expanded column's groups do: the middle of a group joins its tabs, the separator
/// between two groups is the seam between them and wins there, the top of the first group stacks above it, and the
/// bottom of the last group, down through the column's empty foot, stacks below it.
#[trial]
fn icon_groups_take_the_drops_of_an_expanded_columns_groups() {
    let dock = global("iconic column", "spots");
    let probes: Rc<RefCell<Vec<Pos2>>> = Rc::default();
    let found: Rc<RefCell<Vec<Option<DockSpot<u8>>>>> = Rc::default();
    let (asked, seen) = (Rc::clone(&probes), Rc::clone(&found));
    let mut fixture = Fixture::hosted(move |ui| {
        let mut column = Column::new(vec![Group::new(vec![1, 2], 0), Group::new(vec![3], 0)]);
        column.iconic = true;
        let place = Rect::from_min_size(Pos2::ZERO, Vec2::new(RAIL, SCREEN.y));
        let output = IconicColumn::new("Panels", dock, &column, Bind::kept(), &named).within(screen()).show_flyouts_at(ui, place, |_, _, _, _| None);
        // The registry a drop reads is the previous pass's.
        *seen.borrow_mut() = asked.borrow().iter().map(|at| DockTargets::<u8>::resolve(ui.ctx(), dock, *at).map(|(spot, _)| spot)).collect();
        Some(output.into())
    });
    fixture.room(SCREEN).settle();
    let rect_of = |label: &str| fixture.by_label(label).expect("the panel's icon").rect;
    let (brushes, layers, swatches) = (rect_of("Brushes"), rect_of("Layers"), rect_of("Swatches"));
    let grip = |group: u64| fixture.root_id().and_then(|id| fixture.part_rect(id, GRIP.at(group))).expect("the group's grip");
    let (first, second) = (grip(0), grip(1));
    let x = brushes.center().x;
    *probes.borrow_mut() = vec![
        Pos2::new(x, brushes.bottom()),
        Pos2::new(x, (layers.bottom() + second.top()) / 2.0),
        Pos2::new(x, first.center().y),
        Pos2::new(x, swatches.center().y),
        Pos2::new(x, swatches.bottom() + 2.0 * swatches.height()),
    ];
    fixture.idle(2);
    let found = found.borrow().clone();
    assert_eq!(found[0], Some(DockSpot::group(1, DockSide::Center)), "the middle of a group joins its tabs");
    assert_eq!(found[1], Some(DockSpot::seam(1, 3)), "the separator is the seam between the groups");
    assert_eq!(found[2], Some(DockSpot::group(1, DockSide::Top)), "the first group's top stacks above it");
    assert_eq!(found[3], Some(DockSpot::group(3, DockSide::Center)), "a one-icon group keeps its middle for the join");
    assert_eq!(found[4], Some(DockSpot::group(3, DockSide::Bottom)), "the foot under the icons stacks below the last group");
}

/// The ticks of a grip strip in `shapes`: the quads of the one mesh that lies inside `strip`, and their colour.
pub(crate) fn grip_ticks(shapes: &[egui::Shape], strip: Rect) -> Option<(Vec<Rect>, egui::Color32)> {
    shapes.iter().find_map(|shape| {
        let egui::Shape::Mesh(mesh) = shape else { return None };
        let quads: Vec<Rect> = mesh.vertices.chunks(4).map(|quad| Rect::from_points(&quad.iter().map(|v| v.pos).collect::<Vec<_>>())).collect();
        let inside = !quads.is_empty() && quads.iter().all(|quad| strip.expand(EPS).contains_rect(*quad));
        inside.then(|| (quads, mesh.vertices.first().map_or(egui::Color32::TRANSPARENT, |v| v.color)))
    })
}

/// Checks the marks of a grip strip: the theme's `shape.grip` (photocraft#16426), so many marks of its width and height
/// (a tick's length, a dot's side) its gap apart, centred on the strip and in the theme's `grip` ink, which stands
/// apart from the panel `panel` (photocraft#16406).
pub(crate) fn assert_grip_ticks(
    ticks: &[Rect],
    ink: egui::Color32,
    strip: Rect,
    panel: craft_theme::Rgba,
    grip: craft_theme::Rgba,
    shape: craft_theme::GripShape,
    what: &str,
) {
    let height = if shape.mark == craft_theme::GripMark::Ticks { shape.length } else { shape.width };
    assert_eq!(ticks.len(), usize::from(shape.count), "{what}: {} marks: {ticks:?}", shape.count);
    for tick in ticks {
        assert!(
            (tick.width() - shape.width).abs() < EPS && (tick.height() - height).abs() < EPS,
            "{what}: a mark is {} x {height} points: {tick:?}",
            shape.width
        );
    }
    assert!(ticks.windows(2).all(|w| (w[1].min.x - w[0].max.x - shape.gap).abs() < EPS), "{what}: {} points apart: {ticks:?}", shape.gap);
    let run = ticks.iter().fold(Rect::NOTHING, |run, tick| run.union(*tick));
    assert!((run.center().x - strip.center().x).abs() <= 0.5, "{what}: centred across the strip: {run:?} in {strip:?}");
    assert!(
        (run.center().y - (strip.center().y + 1.0)).abs() <= 0.5,
        "{what}: a point below the strip's middle, keeping 2 points more above: {run:?} in {strip:?}"
    );
    let [r, g, b, a] = grip.0;
    assert_eq!(ink, egui::Color32::from_rgba_unmultiplied(r, g, b, a), "{what}: the ticks are in the theme's line.grip {grip:?}");
    assert_ne!(grip.0[..3], panel.0[..3], "{what}: the grip ink {grip:?} stands apart from the panel {panel:?}");
}

/// The grip above each icon group is the references' row of upright ticks, not a line: ten separate marks in the
/// theme's grip ink, the same under the pointer.
#[trial]
fn a_groups_grip_is_a_row_of_upright_ticks_in_the_grip_ink() {
    let mut fixture =
        Fixture::hosted(|ui| {
            let mut column = Column::new(vec![Group::new(vec![1, 2], 0), Group::new(vec![3], 0)]);
            column.iconic = true;
            let place = Rect::from_min_size(Pos2::ZERO, Vec2::new(RAIL, SCREEN.y));
            let output = IconicColumn::new("Panels", global("iconic column", "ticks"), &column, Bind::kept(), &named)
                .within(screen())
                .show_flyouts_at(ui, place, |_, _, _, _| None);
            Some(output.into())
        });
    fixture.room(SCREEN).settle();
    let id = fixture.root_id().expect("the column shows");
    let (panel, grip, shape, tight) = StyleCx::with_root(fixture.ctx(), |cx| {
        (cx.tokens.color.surface.panel, cx.tokens.color.line.grip, cx.tokens.shape.grip, cx.tokens.space.gap.tight)
    });
    assert_eq!(shape, craft_theme::GripShape::DEFAULT, "the fixture's theme sets no grip shape");
    for group in [0, 1] {
        let strip = fixture.part_rect(id, GRIP.at(group)).expect("each group has its grip");
        let shapes = fixture.frame_shapes();
        let (ticks, ink) = grip_ticks(&shapes, strip).expect("the grip paints its ticks");
        assert_grip_ticks(&ticks, ink, strip, panel, grip, shape, &format!("group {group}"));
        let run = ticks.iter().fold(Rect::NOTHING, |run, tick| run.union(*tick));
        let (above, below) = (run.top() - strip.top(), strip.bottom() - run.bottom());
        assert!(
            grows_with(tight, below) && equals_token(above - below, GRIP_ABOVE),
            "group {group}: the marks keep space.gap.tight ({tight}) off the strip's bottom and {GRIP_ABOVE} points more off its top: {run:?} in {strip:?}"
        );
        fixture.frame(vec![egui::Event::PointerMoved(strip.center())]);
        let hovered = fixture.frame_shapes();
        let (_, hover_ink) = grip_ticks(&hovered, strip).expect("the grip paints its ticks under the pointer");
        assert_eq!(hover_ink, ink, "group {group}: the ticks keep their ink under the pointer");
    }
}

/// The shapes a collapsed column of one group paints under a built-in theme whose `shape.grip` is `shape`, and its
/// group's grip strip.
fn themed_grip(shape: craft_theme::GripShape) -> (Vec<egui::Shape>, Rect) {
    let mut def = craft_theme::builtins().first().expect("a built-in theme").clone();
    let mut seed = def.shape.unwrap_or(craft_theme::ShapeSeed {
        radius: None,
        radius_offset: None,
        button_radius: None,
        edge: None,
        knob: None,
        wells: None,
        grip: None,
        locator: None,
    });
    seed.grip = Some(craft_theme::GripSeed {
        mark: Some(shape.mark),
        count: Some(shape.count),
        length: Some(shape.length),
        width: Some(shape.width),
        gap: Some(shape.gap),
    });
    def.shape = Some(seed);
    let def = std::sync::Arc::new(def);
    let mut fixture = Fixture::hosted(move |ui| {
        let mut out = None;
        crate::layout_components::Stack::new().theme(crate::runtime::env::ThemeScope::new().theme(std::sync::Arc::clone(&def))).show(ui, |ui, _| {
            let mut column = Column::new(vec![Group::new(vec![1, 2], 0)]);
            column.iconic = true;
            let place = Rect::from_min_size(Pos2::ZERO, Vec2::new(RAIL, SCREEN.y));
            let output = IconicColumn::new("Panels", global("iconic column", "themed grip"), &column, Bind::kept(), &named)
                .within(screen())
                .show_flyouts_at(ui, place, |_, _, _, _| None);
            out = Some(output.into());
        });
        out
    });
    fixture.room(SCREEN).settle();
    let id = fixture.root_id().expect("the column shows");
    let strip = fixture.part_rect(id, GRIP.at(0)).expect("the group has its grip");
    (fixture.frame_shapes(), strip)
}

/// A theme's `shape.grip` reaches the grips (photocraft#16426): six dots, two points on a side and three apart, in the
/// theme's grip ink.
#[trial]
fn a_themes_grip_shape_reaches_the_grips() {
    let shape = craft_theme::GripShape { mark: craft_theme::GripMark::Dots, count: 6, length: 5.0, width: 2.0, gap: 3.0 };
    let (shapes, strip) = themed_grip(shape);
    let (marks, ink) = grip_ticks(&shapes, strip).expect("the grip paints its marks");
    assert_eq!(marks.len(), 6, "the theme's six dots: {marks:?}");
    assert!(marks.iter().all(|m| equals_token(m.width(), shape.width) && equals_token(m.height(), shape.width)), "two-point dots: {marks:?}");
    assert!(marks.windows(2).all(|w| equals_token(w[1].min.x - w[0].max.x, shape.gap)), "three points apart: {marks:?}");
    assert_ne!(ink, egui::Color32::TRANSPARENT);
}

/// A theme's bar grip (VectorCraft's) is one level bar as long and as thick as the theme says, centred across the
/// strip, with no row of ticks.
#[trial]
fn a_themes_bar_grip_is_one_level_bar() {
    let shape = craft_theme::GripShape { mark: craft_theme::GripMark::Bar, count: 11, length: 18.0, width: 3.0, gap: 1.0 };
    let (shapes, strip) = themed_grip(shape);
    assert!(grip_ticks(&shapes, strip).is_none(), "no row of ticks");
    let bars: Vec<Rect> = shapes
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::Rect(rect) if strip.expand(EPS).contains_rect(rect.rect) && rect.fill != egui::Color32::TRANSPARENT => Some(rect.rect),
            _ => None,
        })
        .collect();
    let [bar] = bars.as_slice() else { panic!("one bar in the strip: {bars:?}") };
    assert!(bar.width() > bar.height(), "the bar lies level: {bar:?}");
    assert!(equals_token(bar.width(), shape.length) && equals_token(bar.height(), shape.width), "the theme's 18 by 3 bar: {bar:?}");
    assert!((bar.center().x - strip.center().x).abs() <= 0.5, "centred across the strip: {bar:?} in {strip:?}");
}

/// The icons of the two groups of a collapsed column `width` wide whose groups stand apart by `separator`: the fixture,
/// the column's id and the panel whose flyout is open.
fn separated(separator: IconGroupSeparator, width: f32) -> (Fixture, Id, Rc<Cell<Option<u8>>>) {
    let open = Rc::new(Cell::new(None));
    let seen = Rc::clone(&open);
    let mut fixture = Fixture::hosted(move |ui| {
        let mut column = Column::new(vec![Group::new(vec![1, 2], 0), Group::new(vec![3], 0)]);
        column.iconic = true;
        let place = Rect::from_min_size(Pos2::ZERO, Vec2::new(width, SCREEN.y));
        let mut now = seen.get();
        let output =
            with_env(
                ui,
                |overrides| {
                    overrides.values.set(separator);
                },
                |ui| {
                    IconicColumn::new("Panels", global("iconic column", "rule"), &column, Bind::bound(&mut now), &named)
                        .within(screen())
                        .show_flyouts_at(ui, place, |_, _, _, _| None)
                },
            );
        seen.set(now);
        Some(output.into())
    });
    fixture.room(SCREEN).settle();
    let id = fixture.root_id().expect("the column shows");
    (fixture, id, open)
}

/// An app's `IconGroupSeparator::Rule` (VectorCraft's): no grip strips, so the first icon stands higher than under grip
/// strips; one level edge rule lies in the middle of the room between the two groups, short of the column's sides, and
/// none over the first group. Each group keeps a bare grip band, as production's: the first group's over its first
/// icon's top edge, a later group's over the room above it and its first icon's top edge. A drag from a band moves the
/// group, from an icon below the band, the first included, only that icon's panel; a click in the band opens no flyout
/// (the band takes the press there, as production's did), a click below it does, in a narrow and in a wide column.
#[trial]
fn icon_groups_apart_by_a_rule_show_one_level_rule_between_them() {
    let (gripped, gripped_id, _) = separated(IconGroupSeparator::Grip, RAIL);
    let gripped_first = gripped.part_rect(gripped_id, CELL.at(0)).expect("the first icon under grips");
    let (mut fixture, id, _) = separated(IconGroupSeparator::Rule, RAIL);
    let border = StyleCx::with_root(fixture.ctx(), |cx| cx.tokens.color.line.border);
    let shapes = fixture.frame_shapes();
    let level: Vec<(Pos2, Pos2, egui::Color32)> = shapes
        .iter()
        .filter_map(|shape| match shape {
            egui::Shape::LineSegment { points: [a, b], stroke } if (a.y - b.y).abs() < EPS => Some((*a, *b, stroke.color)),
            _ => None,
        })
        .collect();
    let cell = |at: u64| fixture.part_rect(id, CELL.at(at)).expect("the icon shows");
    let band = |group: u64| fixture.part_rect(id, GRIP.at(group)).expect("each group keeps a grip band");
    let (first, last, next) = (cell(0), cell(1), cell(2));
    let (top, lower) = (band(0), band(1));
    assert!(first.top() < gripped_first.top(), "the first icon {first:?} stands higher than under a grip strip {gripped_first:?}");
    for (group, band) in [top, lower].into_iter().enumerate() {
        assert!(grip_ticks(&shapes, band).is_none(), "group {group}: the band shows no marks");
    }
    assert!(
        near(top.top(), first.top()) && top.bottom() > first.top() && top.bottom() < first.center().y,
        "the first band {top:?} is the top edge of {first:?}"
    );
    assert!(
        near(lower.top(), last.bottom()) && lower.bottom() > next.top() && lower.bottom() < next.center().y,
        "the later band {lower:?} covers the room from {last:?} and the top edge of {next:?}"
    );
    assert!(!level.iter().any(|(a, _, _)| a.y < first.bottom()), "no rule over the first group: {level:?}");
    let between: Vec<_> = level.iter().filter(|(a, _, _)| a.y > last.bottom() && a.y < next.top()).collect();
    let [(a, b, ink)] = between.as_slice() else { panic!("one rule between the groups: {level:?}") };
    let middle = (last.bottom() + next.top()) / 2.0;
    assert!(near(a.y, middle), "the rule {a:?} lies in the middle of the room from {last:?} to {next:?}");
    let (left, right) = (a.x.min(b.x), a.x.max(b.x));
    let span = right - left;
    assert!(left > 0.0 && right < RAIL && span > RAIL - span, "the rule reaches across the column, short of its sides: {a:?} to {b:?}");
    let [r, g, bl, al] = border.0;
    assert_eq!(*ink, egui::Color32::from_rgba_unmultiplied(r, g, bl, al), "the rule is in line.border");
    for width in [RAIL, WIDE_COLUMN] {
        let (mut fixture, id, open) = separated(IconGroupSeparator::Rule, width);
        let cell = |at: u64| fixture.part_rect(id, CELL.at(at)).expect("the icon shows");
        let band = |group: u64| fixture.part_rect(id, GRIP.at(group)).expect("each group keeps a grip band");
        let (first, last, next, top, lower) = (cell(0), cell(1), cell(2), band(0), band(1));
        let in_top = Pos2::new(first.center().x, top.center().y);
        let drags = [
            (in_top, DragUnit::Group(1)),
            (Pos2::new(next.center().x, (last.bottom() + next.top()) / 2.0), DragUnit::Group(3)),
            (Pos2::new(next.center().x, (next.top() + lower.bottom()) / 2.0), DragUnit::Group(3)),
            (first.center(), DragUnit::Panel(1)),
            (last.center(), DragUnit::Panel(2)),
            (next.center(), DragUnit::Panel(3)),
        ];
        for (from, unit) in drags {
            fixture.drag_hold_at(from, Vec2::new(SCREEN.x / 2.0, 0.0));
            let dragged = drag::active::<GroupDrag<u8>>(fixture.ctx()).map(|drag| drag.unit.clone());
            assert_eq!(dragged, Some(unit.clone()), "{width} wide: a drag from {from:?} moves {unit:?}");
            fixture.release_at(from + Vec2::new(SCREEN.x / 2.0, 0.0)).settle();
        }
        fixture.click_at(in_top).idle(3);
        assert_eq!(open.get(), None, "{width} wide: a click in the band opens no flyout");
        fixture.click_at(first.center()).idle(3);
        assert_eq!(open.get(), Some(1), "{width} wide: a click on the first icon below the band opens its flyout");
    }
}

/// The surface shapes painted around the open flyout of a column that is `framed` (a flush dock's): the rects whose
/// rect holds the flyout's body.
fn flyout_surfaces(framed: bool) -> Vec<egui::epaint::RectShape> {
    let shown: Rc<Cell<Option<Rect>>> = Rc::default();
    let seen = Rc::clone(&shown);
    let mut fixture = Fixture::new(move |ui| {
        let column = Column::new(vec![Group::new(vec![0, 1], 0)]);
        let mut open = Some(0_u8);
        IconicColumn::new("Panels", global("iconic column", "docked flyout"), &column, Bind::bound(&mut open), &spec).framed(framed).show_flyouts(
            ui,
            |_, at, ui, _| {
                let GroupAt::Show(rect) = at else { return None };
                seen.set(Some(rect));
                Panel::new("Flyout").show_at(ui, rect, |_, _| ());
                None
            },
        );
    });
    fixture.room(Vec2::new(RAIL, 300.0)).settle();
    let body = shown.get().expect("the flyout shows");
    fixture
        .frame_shapes()
        .into_iter()
        .filter_map(|shape| match shape {
            egui::Shape::Rect(rect) if rect.rect.expand(EPS).contains_rect(body) && rect.rect.width() < body.width() + RAIL => Some(rect),
            _ => None,
        })
        .collect()
}

/// The flyout of a flush dock's column (`DockConfig::flush_docks`, VectorCraft's) stands as a docked column does:
/// square, with no shadow; a column of another dock opens a raised popup with rounded corners.
#[trial]
fn a_flush_docks_flyout_stands_square_with_no_shadow() {
    let raised = flyout_surfaces(false);
    assert!(raised.iter().any(|rect| rect.corner_radius != egui::CornerRadius::ZERO), "the popup flyout is rounded: {raised:?}");
    let docked = flyout_surfaces(true);
    assert!(!docked.is_empty(), "the docked flyout paints its surface");
    for rect in &docked {
        assert_eq!(rect.corner_radius, egui::CornerRadius::ZERO, "the docked flyout's surface is square: {rect:?}");
        assert!(rect.blur_width <= 0.0, "the docked flyout casts no shadow: {rect:?}");
    }
}

/// The fills painted on the open panel's icon of a column beside tabs in `look`, with the colour of `surface.control`.
fn open_icon_fills(look: PanelTabLook) -> (Vec<egui::Color32>, egui::Color32) {
    let mut fixture = Fixture::hosted(move |ui| {
        let column = Column::new(vec![Group::new(vec![0, 1], 0)]);
        let mut open = Some(0_u8);
        let output = IconicColumn::new("Panels", global("iconic column", "open icon"), &column, Bind::bound(&mut open), &spec)
            .tab_look(look)
            .show_flyouts_at(ui, Rect::from_min_size(Pos2::ZERO, Vec2::new(RAIL, 300.0)), |_, _, _, _| None);
        Some(output.into())
    });
    fixture.room(SCREEN).settle();
    let id = fixture.root_id().expect("the column shows");
    let cell = fixture.part_rect(id, CELL.at(0)).expect("the open panel's icon");
    let control = StyleCx::with_root(fixture.ctx(), |cx| cx.tokens.color.surface.control).0;
    let fills = fixture
        .frame_shapes()
        .into_iter()
        .filter_map(|shape| match shape {
            egui::Shape::Rect(rect) if cell.expand(EPS).contains_rect(rect.rect) && rect.fill != egui::Color32::TRANSPARENT => Some(rect.fill),
            _ => None,
        })
        .collect();
    (fills, egui::Color32::from_rgba_unmultiplied(control[0], control[1], control[2], control[3]))
}

/// Beside block tabs (`PanelTabLook::Block`, VectorCraft's) the open panel's icon shows on `surface.control`, as the
/// shown tab does; beside the other looks it keeps the accent's on look.
#[trial]
fn beside_block_tabs_the_open_icon_shows_on_the_control_surface() {
    let (block, control) = open_icon_fills(PanelTabLook::Block);
    assert!(block.contains(&control), "the open icon shows surface.control {control:?}: {block:?}");
    let (plain, _) = open_icon_fills(PanelTabLook::Plain);
    assert!(!plain.is_empty() && !plain.contains(&control), "beside plain tabs the open icon keeps its own fill: {plain:?}");
}
