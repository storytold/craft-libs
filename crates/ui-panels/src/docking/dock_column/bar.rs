//! The collapse bar of a dock column: the chevron, a part of the column's run that collapses or expands it, and the
//! band beside it that moves the column and opens its menu.

use crate::buttons::IconButton;
use crate::buttons::icon_button::IconButtonStyleKey;
use crate::contract::overlay::Trigger;
use crate::contract::{Access, MemoProps, MemoView, Name, Outbox, PaintUi, Props as _, Role, Show as _, ShowContainer as _};
use crate::docking::group::GroupGrip;
use crate::docking::{ColumnArrow, DockWords, DragUnit, GroupDrag, PanelKey, PanelSpec};
use crate::menu::{GroupItem, Menu, MenuWords, PanelMenu, PanelRow};
use crate::runtime::env::Label;
use craft_behavior::PartKey;
use craft_behavior::dock::{Column, DockEdge};
use craft_theme::variant::Tone;
use craft_theme::{ControlSize, CustomVariant, Emphasis, States, Tokens, Variant, VariantLook};
use egui::{Rect, Ui, UiBuilder, Vec2};
use std::hash::Hash;

/// The chevron of a column's bar: a click collapses an expanded column to icons and expands an iconic one.
pub(in crate::docking) const CHEVRON: PartKey = PartKey::new("chevron");

/// What the user did on a column bar this pass, for the container to read when it finishes.
#[derive(Default)]
pub(in crate::docking) struct BarRecord {
    /// The user asked to collapse (`true`) or expand (`false`) the column.
    pub(in crate::docking) iconic: Option<bool>,
    /// The dock row picked in the column menu.
    pub(in crate::docking) picked: Option<GroupItem>,
}

/// What the user did on a column bar, as its view sends it.
enum BarEvent {
    /// A double click on the band: collapse (`true`) or expand (`false`) the column.
    Iconic(bool),
    /// A dock row of the column menu.
    Picked(GroupItem),
    /// A drag of the column started on the band.
    DragStarted,
}

/// The grip of a column bar's band: what a drag on it moves and shows.
pub(in crate::docking) struct ColumnGrip<P> {
    pub(in crate::docking) unit: DragUnit<P>,
    pub(in crate::docking) active: PanelKey,
    /// The column's extent where the drag starts and the name its drop region shows.
    pub(in crate::docking) ghost: (Vec2, Label),
    /// The band's name.
    pub(in crate::docking) name: Label,
}

/// The look of the collapse control: ghost, with the muted text until the pointer is over it, and no fill under the
/// pointer.
fn collapse_look(tokens: &Tokens) -> VariantLook {
    let ghost = VariantLook::of(Emphasis::Ghost, Tone::Neutral, tokens);
    let mut text = ghost.text;
    text.rest = tokens.color.text.muted;
    VariantLook::new(States::new(ghost.fill.rest, ghost.fill.rest, ghost.fill.rest, ghost.fill.rest), text)
}

const COLLAPSE: CustomVariant = CustomVariant::new::<IconButtonStyleKey>("craft_ui.dock-collapse", collapse_look);

/// Whether the column's double arrow points to the right of the screen: towards the edge it keeps to when a click sends
/// the column there (`towards_edge`), else away from it.
fn points_right(edge: DockEdge, towards_edge: bool, rtl: bool) -> bool {
    let at_left = (edge == DockEdge::Start) != rtl;
    at_left != towards_edge
}

/// The chevron's slot near the bar's outer end, the window edge the dock is on, as high as the bar and at least `least`
/// wide (the least hit size), so a click meant for the chevron does not land on the band beside it; and the band in the
/// rest of the bar. The slot stands half of `least` in from that edge, where the window's own resize border is, so a
/// press meant for the chevron does not resize the window and the other way round; that gap belongs to neither. A narrow
/// bar (an iconic column's) keeps at least half of itself for the band and gives the gap only what is left.
pub(in crate::docking) fn bar_slots(bar: Rect, edge: DockEdge, rtl: bool, least: f32) -> (Rect, Rect) {
    let side = bar.height().max(least.min(bar.width() / 2.0)).min(bar.width());
    let gap = (least / 2.0).min((bar.width() / 2.0 - side).max(0.0));
    if (edge == DockEdge::End) != rtl {
        let outer = bar.right() - gap;
        let split = outer - side;
        (Rect::from_x_y_ranges(split..=outer, bar.y_range()), Rect::from_x_y_ranges(bar.left()..=split, bar.y_range()))
    } else {
        let outer = bar.left() + gap;
        let split = outer + side;
        (Rect::from_x_y_ranges(outer..=split, bar.y_range()), Rect::from_x_y_ranges(split..=bar.right(), bar.y_range()))
    }
}

/// Paints the chevron part of the `expanded` or collapsed column whose double arrow is `arrow`: a small ghost button
/// with the double chevron pointing where the column goes, named for what a click does.
pub(in crate::docking) fn paint_chevron(ui: &mut PaintUi<'_>, edge: DockEdge, arrow: ColumnArrow, expanded: bool, rtl: bool, words: &DockWords) {
    let chevrons = crate::collapse::chevrons(points_right(edge, arrow.towards_edge(expanded), rtl));
    ui.face(CHEVRON, IconButton::new(chevrons).size(ControlSize::Sm).variant(Variant::Custom(COLLAPSE)).label(arrow.name(words, expanded)));
}

/// The chevron's node.
pub(in crate::docking) fn describe_chevron(access: &mut Access, name: &Label) {
    access.node(CHEVRON, Role::BUTTON).name = Name::Text(name.as_str().to_owned());
}

/// The band of the column bar `bar` of a column that keeps to `edge`: the bar beside the chevron's slot.
pub(in crate::docking) fn bar_band(ui: &Ui, bar: Rect, edge: DockEdge) -> Rect {
    let env = crate::runtime::env::Env::of(ui);
    bar_slots(bar, edge, env.direction.is_rtl(), env.tokens.size.hit.target).1
}

/// Shows a column's bar in `band`: the band of a column bar ([`bar_band`]) and the column menu, as a memoized view
/// ([`column_bar`]) held to the band. A drag on the band moves the column, a double click collapses or expands it, and a
/// context request opens the column menu, which offers the collapse only where the column's double arrow (`arrow`)
/// collapses it. The chevron at the bar's outer end is the column's own part. `icons` is a collapsed column's bar.
/// Returns what the user did.
#[allow(clippy::too_many_arguments, reason = "one argument per input of the bar's view")]
pub(in crate::docking) fn show_bar<P: Clone + Hash + Send + Sync + 'static>(
    ui: &mut Ui,
    band: Rect,
    expanded: bool,
    auto_collapse: bool,
    arrow: ColumnArrow,
    icons: bool,
    grip: Option<ColumnGrip<P>>,
) -> BarRecord {
    let mut record = BarRecord::default();
    let Some(ColumnGrip { unit, active, ghost, name }) = grip.filter(|_| band.is_positive()) else { return record };
    let mut sent = Vec::new();
    ui.scope_builder(UiBuilder::new().max_rect(band), |ui| {
        let props = BarProps { expanded, auto_collapse, arrow, unit, active, ghost: ghost.1, name, sent: Outbox::new(&mut sent) };
        // Keyed by its column's scope id: unique among the bars of a scroll area's columns. As for the tab row, the env
        // around it holds only values its key follows, so it replays at rest.
        let _ = MemoView::new(ui.scope_id(), props, column_bar::<P>).show(ui);
    });
    for event in sent {
        match event {
            BarEvent::Iconic(iconic) => record.iconic = Some(iconic),
            BarEvent::Picked(item) => record.picked = Some(item),
            // The column's extent goes on the drag here, outside the view, whose measuring runs see no extent: a
            // collapsed column's drag floats it collapsed with its icons under the pointer.
            BarEvent::DragStarted => {
                crate::runtime::drag::amend::<GroupDrag<P>>(ui.ctx(), |drag| {
                    if icons {
                        drag.strip = Some(ghost.0);
                    } else {
                        drag.source = Some(ghost.0);
                    }
                });
            }
        }
    }
    record
}

/// The props of a column bar's view.
#[derive(MemoProps)]
struct BarProps<'a, P> {
    expanded: bool,
    auto_collapse: bool,
    arrow: ColumnArrow,
    #[memo_props(hash = "a dock's panels are plain values it keys by their hash")]
    unit: DragUnit<P>,
    active: PanelKey,
    /// The name the column's drop region shows.
    ghost: Label,
    name: Label,
    sent: Outbox<'a, BarEvent>,
}

/// A column bar's band, held to the view's room, and the column menu.
fn column_bar<P: Clone + Hash + Send + Sync + 'static>(props: &mut BarProps<'_, P>, ui: &mut Ui) {
    let band = ui.max_rect();
    let env = crate::runtime::env::Env::of(ui);
    let (words, menu_words) = (env.words::<DockWords>(), env.words::<MenuWords>());
    let toggle = ColumnArrow::Collapse.name(&words, props.expanded);
    let grip = GroupGrip::new(props.unit.clone(), props.active).ghost(props.ghost.clone()).label(props.name.clone());
    let held = grip.show_at(ui, band);
    if held.drag_started {
        props.sent.send(BarEvent::DragStarted);
    }
    if held.double_pressed {
        props.sent.send(BarEvent::Iconic(props.expanded));
    }
    let collapse = (props.arrow == ColumnArrow::Collapse).then(|| PanelRow::Item(crate::contract::Entry::new(GroupItem::CollapseToIcons, toggle)));
    let auto = PanelRow::Item(
        crate::contract::Entry::new(GroupItem::AutoCollapse, Label::from(menu_words.panel_auto_collapse.as_str().to_owned()))
            .checked(props.auto_collapse),
    );
    let rows: Vec<_> = collapse.into_iter().chain([auto]).collect();
    let picked = Menu::new(Label::from(words.panels.as_str().to_owned()))
        .opened_by(&held.response, Trigger::Context)
        .show(ui, |ui, _| PanelMenu::new(Vec::new(), rows.clone()).show(ui).picked);
    if let Some(item) = picked.inner.flatten() {
        props.sent.send(BarEvent::Picked(item));
    }
    // The grip takes no room in flow; the run covers the band, so the pointer over it runs it again.
    ui.expand_to_include_rect(band);
}

/// The name of a column's bar: the labels of its panels.
pub(in crate::docking) fn bar_name<P>(column: &Column<P>, spec: &dyn Fn(&P) -> PanelSpec, template: &crate::runtime::env::WordTemplate) -> String {
    template.fill(&[("name", panel_names(column, spec).as_str())])
}

/// The labels of a column's panels, in order.
pub(super) fn panel_names<P>(column: &Column<P>, spec: &dyn Fn(&P) -> PanelSpec) -> String {
    let names: Vec<String> = column.groups.iter().flat_map(|group| group.panels.iter()).map(|panel| spec(panel).label.as_str().to_owned()).collect();
    names.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    #[test]
    fn a_thin_bar_gives_the_chevron_the_least_hit_size() {
        let bar = Rect::from_min_max(pos2(0.0, 0.0), pos2(300.0, 8.0));
        for (edge, rtl) in [(DockEdge::End, false), (DockEdge::Start, false), (DockEdge::End, true)] {
            let (chevron, band) = bar_slots(bar, edge, rtl, 24.0);
            assert!(chevron.width() >= 24.0, "wider than the bar is high: {chevron:?}");
            assert!(!chevron.intersects(band.shrink2(egui::vec2(0.5, 0.0))), "the band starts where the chevron ends");
            assert!((chevron.width() + band.width() + 12.0 - bar.width()).abs() < 0.01, "the two and the gap fill the bar");
        }
    }

    #[test]
    fn the_chevron_stands_off_the_window_edge() {
        let bar = Rect::from_min_max(pos2(0.0, 0.0), pos2(300.0, 8.0));
        let (chevron, _) = bar_slots(bar, DockEdge::End, false, 24.0);
        assert!(bar.right() - chevron.right() >= 12.0, "half the hit size from the right edge: {chevron:?}");
        let (chevron, _) = bar_slots(bar, DockEdge::Start, false, 24.0);
        assert!(chevron.left() - bar.left() >= 12.0, "half the hit size from the left edge: {chevron:?}");
    }

    #[test]
    fn a_narrow_bar_keeps_half_for_the_band() {
        let bar = Rect::from_min_max(pos2(0.0, 0.0), pos2(30.0, 8.0));
        let (chevron, band) = bar_slots(bar, DockEdge::End, false, 24.0);
        assert!(band.width() >= chevron.width(), "the band keeps at least half: {chevron:?} {band:?}");
        assert!(chevron.width() > bar.height(), "still wider than the bar is high");
    }
}
