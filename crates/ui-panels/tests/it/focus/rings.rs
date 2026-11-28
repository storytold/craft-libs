//! The focus ring of the panel family's focusable parts: Tab to each one and the paint step rings it, as it rings a
//! button.

use craft_behavior::dock::{Column, DockEdge, Group};
use craft_behavior::{Axis, PartKey};
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft_ui::docking::stack::{StackSection, StackSections};
use craft_ui::docking::{
    DockColumn, DockGroup, DragUnit, FloatingGroup, GroupGrip, IconicColumn, PanelKey, PanelSpec, PanelTitle, SplitArea, Titles, WindowColumn,
};
use craft_ui::menu::PanelRow;
use craft_ui::sections::{HeaderAction, Panel, TitleRow};
use craft_ui::testing::memory::global;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Pos2, Rect, Shape, Vec2};

/// Presses Tab until a stop that `wanted` accepts holds the focus, and returns the rect of the focused part, or of
/// the focused component when the stop is a whole component. Panics when Tab never reaches one.
fn tab_to(fixture: &mut Fixture, wanted: impl Fn(Option<PartKey>) -> bool) -> Rect {
    for _ in 0..16 {
        fixture.key(Key::Tab, Modifiers::NONE).settle();
        let Some((owner, part)) = fixture.focused_stop() else { continue };
        if !wanted(part.map(|part| part.key)) {
            continue;
        }
        let rect = match part {
            Some(part) => fixture.part_rect(owner, part),
            None => fixture.node_of(owner).map(|target| target.rect),
        };
        return rect.unwrap_or_else(|| panic!("the focused stop {owner:?} {part:?} has a rect"));
    }
    panic!("Tab never reached the wanted stop");
}

/// Tab to the part `key`.
fn tab_to_part(fixture: &mut Fixture, key: &'static str) -> Rect {
    tab_to(fixture, |part| part == Some(PartKey::new(key)))
}

/// Whether the latest frame paints a focus ring (a rectangle stroke at least two points wide) around `target`.
fn ringed(fixture: &mut Fixture, target: Rect) -> bool {
    fixture.frame_shapes().iter().any(|shape| match shape {
        Shape::Rect(rect) if rect.stroke.width >= 2.0 => rect.rect.expand(1.0).contains_rect(target),
        _ => false,
    })
}

fn spec(panel: &u8) -> PanelSpec {
    PanelSpec::new(["Brushes", "Swatches", "History"].get(usize::from(*panel)).copied().unwrap_or("Panel"))
}

/// A title row's disclosure lead and its action buttons are rung when Tab reaches them.
#[trial]
fn a_title_rows_focused_lead_and_action_show_the_focus_ring() {
    let mut fixture = Fixture::new(|ui| {
        let mut open = true;
        TitleRow::new("Layers").disclosure(Bind::bound(&mut open)).show(ui);
    });
    fixture.settle();
    let lead = tab_to_part(&mut fixture, "lead");
    assert!(ringed(&mut fixture, lead), "the focused disclosure lead {lead:?} is ringed");
    let mut fixture = Fixture::new(|ui| {
        TitleRow::new("Layers").actions([HeaderAction::Close]).show(ui);
    });
    fixture.settle();
    let action = tab_to_part(&mut fixture, "action");
    assert!(ringed(&mut fixture, action), "the focused action {action:?} is ringed");
}

/// A group grip and a docked group's drag band and panel menu button are rung when Tab reaches them.
#[trial]
fn a_focused_group_grip_drag_band_and_panel_menu_button_show_the_focus_ring() {
    let mut fixture = Fixture::new(|ui| {
        let strip = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(60.0, 16.0));
        GroupGrip::new(DragUnit::Group(1usize), PanelKey::new(1)).label("Move tab group").show_at(ui, strip);
    });
    fixture.settle();
    let grip = tab_to(&mut fixture, |part| part.is_none());
    assert!(ringed(&mut fixture, grip), "the focused grip {grip:?} is ringed");
    let group = || {
        let panels = vec![PanelTitle::new(PanelKey::new(1), "Layers")];
        Fixture::new(move |ui| {
            let group = DockGroup::new(1usize, Titles::from_content(panels.clone()), panels[0].key).menu(Vec::<PanelRow>::new);
            let _ = group.show_at(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(300.0, 240.0)), |_, _| ());
        })
    };
    for part in ["grip", "panel_menu"] {
        let mut fixture = group();
        fixture.settle();
        let rect = tab_to_part(&mut fixture, part);
        assert!(ringed(&mut fixture, rect), "the group's focused {part} part {rect:?} is ringed");
    }
}

/// A docked column's chevron and the divider between its groups are rung when Tab reaches them.
#[trial]
fn a_dock_columns_focused_chevron_and_divider_show_the_focus_ring() {
    for part in ["chevron", "divider"] {
        let mut fixture = Fixture::new(|ui| {
            let column = Column::new(vec![Group::new(vec![0, 1], 0), Group::new(vec![2], 0)]);
            DockColumn::new("Panels", global("dock column", "focus ring trial"), &column, &spec).show_groups(ui, |_, rect, ui, _| {
                Panel::new("Panel").show_at(ui, rect, |_, _| ());
            });
        });
        fixture.room(Vec2::new(240.0, 400.0)).settle();
        let rect = tab_to_part(&mut fixture, part);
        assert!(ringed(&mut fixture, rect), "the column's focused {part} part {rect:?} is ringed");
    }
}

/// A dock window's chevron is rung when Tab reaches it.
#[trial]
fn a_dock_windows_focused_chevron_shows_the_focus_ring() {
    let mut fixture = Fixture::new(|ui| {
        let rect = Rect::from_min_size(Pos2::new(40.0, 40.0), Vec2::new(240.0, 180.0));
        FloatingGroup::new("Layers", rect).column(WindowColumn::expanded(DockEdge::Start)).show_group(ui, |_, _| ());
    });
    fixture.settle();
    let chevron = tab_to_part(&mut fixture, "chevron");
    assert!(ringed(&mut fixture, chevron), "the window's focused chevron {chevron:?} is ringed");
}

/// A collapsed column's chevron and icon cells are rung when Tab reaches them.
#[trial]
fn an_iconic_columns_focused_chevron_and_cell_show_the_focus_ring() {
    for part in ["chevron", "cell"] {
        let mut fixture = Fixture::new(|ui| {
            let column = Column::new(vec![Group::new(vec![0, 1], 0)]);
            let mut open = None;
            IconicColumn::new("Panels", global("iconic column", "focus ring trial"), &column, Bind::bound(&mut open), &spec)
                .show_flyouts(ui, |_, _, _, _| None);
        });
        fixture.room(Vec2::new(48.0, 300.0)).settle();
        let rect = tab_to_part(&mut fixture, part);
        assert!(ringed(&mut fixture, rect), "the column's focused {part} part {rect:?} is ringed");
    }
}

/// The handle between two docked areas and the divider between two stacked sections are rung when Tab reaches them.
#[trial]
fn a_focused_split_handle_and_stack_divider_show_the_focus_ring() {
    use craft_behavior::dock::{Layout, Node, SplitSize};
    use craft_ui::contract::SizeLimits;
    use craft_ui::docking::tree::FrameRules;
    let mut fixture = Fixture::new(|ui| {
        let root = Node::split(Axis::X, SplitSize::Ratio(0.5), Node::tabs(vec![0usize], 0), Node::tabs(vec![1usize], 0));
        let mut layout = Layout::new(Some(root), Vec::new());
        let limits = |_: &usize| SizeLimits::default();
        SplitArea::new("Panels", Bind::bound(&mut layout), FrameRules::new(&limits), 0).show(ui, |_, _| ());
    });
    fixture.room(Vec2::new(400.0, 300.0)).settle();
    let handle = tab_to_part(&mut fixture, "handle");
    assert!(ringed(&mut fixture, handle), "the focused split handle {handle:?} is ringed");
    let mut fixture = Fixture::new(|ui| {
        StackSections::new(vec![StackSection::new(true), StackSection::new(true)]).label("Sections").show(ui, |_, _| ());
    });
    fixture.room(Vec2::new(240.0, 300.0)).settle();
    let divider = tab_to_part(&mut fixture, "divider");
    assert!(ringed(&mut fixture, divider), "the focused stack divider {divider:?} is ringed");
}
