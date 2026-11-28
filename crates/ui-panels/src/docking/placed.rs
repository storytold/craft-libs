//! Where a drop puts the panels it moves: the layout after the drop, pruned as the dock area shows it and laid out by
//! the functions the dock draws with, so a drop's mark is the rect the moved panels take once dropped.

use super::frame::maximize;
use super::tree::{self, FrameRules};
use super::{ColumnBar, DockSizes, HidePanels, PanelSpec, column_cells, group_rects};
use craft_behavior::Flow;
use craft_behavior::dock::{Column, Layout, Node};
use egui::Rect;
use std::hash::Hash;

/// How a dock lays out its panels, which its drop target reads to mark where a drop puts them
/// (`DockDropTarget::placing`).
#[non_exhaustive]
pub struct Placing<'a, P> {
    /// The dock's sizes.
    pub sizes: DockSizes,
    /// Each panel's spec: its limits, heights and whether it is a canvas.
    pub spec: &'a dyn Fn(&P) -> PanelSpec,
    /// The panels that always show, such as a canvas.
    pub protected: &'a dyn Fn(&P) -> bool,
    /// The panels the area hides, as the shell's visibility leaves them.
    pub hide: HidePanels<'a>,
    /// The panel whose frame the area shows maximized.
    pub maximized: Option<&'a P>,
    /// The floating window whose column the dock lays out, by its id; `None` for a dock area's docked tree.
    pub window: Option<u64>,
}

impl<'a, P> Placing<'a, P> {
    /// The placing of a dock area's docked tree that hides nothing.
    pub fn new(sizes: DockSizes, spec: &'a dyn Fn(&P) -> PanelSpec, protected: &'a dyn Fn(&P) -> bool) -> Self {
        Placing { sizes, spec, protected, hide: HidePanels::None, maximized: None, window: None }
    }

    /// The area hides `hide` and shows the frame of `maximized` alone.
    #[must_use]
    pub fn shown(mut self, hide: HidePanels<'a>, maximized: Option<&'a P>) -> Self {
        self.hide = hide;
        self.maximized = maximized;
        self
    }

    /// The placing of the floating window `window` instead of the docked tree.
    #[must_use]
    pub fn window(mut self, window: u64) -> Self {
        self.window = Some(window);
        self
    }
}

/// Runs `f` with the rules a dock area lays its docked tree out by: each panel's limits from `spec`, a dock as wide as
/// its columns' natural widths ([`DockSizes::natural`]) and [`DockSizes::dock_gap`] from the node beside it, and no
/// strip on a canvas alone in its group.
pub fn with_area_rules<P, R>(sizes: &DockSizes, spec: &dyn Fn(&P) -> PanelSpec, f: impl FnOnce(FrameRules<'_, P>) -> R) -> R {
    let limits = |panel: &P| spec(panel).limits;
    let dock_width = |columns: &[Column<P>]| sizes.natural(columns, spec).iter().map(|(width, _)| *width).sum::<f32>();
    let stripless = |panel: &P| spec(panel).canvas;
    f(FrameRules::new(&limits).dock_width(&dock_width).stripless(&stripless).dock_gap(sizes.dock_gap))
}

/// The rect the panels `moved` take in `layout`, the layout after a drop, laid out in `room` (the room the dock
/// registered, `DockTargets::room`) as the dock lays it out, pruned as the area shows it: the union of the rects of the
/// groups that hold them, or the cell of a dock column they make up alone. `None` where one of them lands where the
/// layout gives it no rect of its own: hidden, in a collapsed column (icons) or in a stepped column (sized by its
/// content as it shows).
pub fn placed_rect<P: Clone + Eq + Hash>(layout: &Layout<P>, moved: &[P], room: Rect, placing: &Placing<'_, P>, rtl: bool) -> Option<Rect> {
    let view = maximize(layout, placing.maximized, placing.hide, placing.protected);
    let layout: &Layout<P> = &view.layout;
    let mut rects = Vec::new();
    match placing.window {
        Some(id) => {
            let window = layout.floating.iter().find(|window| window.id == id)?;
            column_rects(&window.column, room, ColumnBar::Window, placing, moved, &mut rects)?;
        }
        None => {
            let root = layout.root.as_ref()?;
            let flow = Flow::new(room, rtl);
            with_area_rules(&placing.sizes, placing.spec, |rules| {
                for frame in tree::frames(root, room, &placing.sizes, &rules, rtl).groups {
                    match frame.node {
                        Node::Dock { side, columns, .. } => {
                            for (column, cell) in columns.iter().zip(column_cells(columns, frame.rect, *side, flow, &placing.sizes, placing.spec)) {
                                column_rects(column, cell, ColumnBar::Own, placing, moved, &mut rects)?;
                            }
                        }
                        Node::Tabs { panels, .. } if panels.iter().any(|panel| moved.contains(panel)) => rects.push(frame.rect),
                        Node::Stack { entries, .. } if entries.iter().any(|entry| moved.contains(&entry.panel)) => rects.push(frame.rect),
                        _ => {}
                    }
                }
                Some(())
            })?;
        }
    }
    rects.into_iter().reduce(Rect::union)
}

/// Adds the rects the moved panels take in `column` at `cell` to `rects`: the cell when they make up the column, else
/// the rects of its groups that hold them. `None` when one lands in a collapsed or stepped column.
fn column_rects<P: PartialEq>(
    column: &Column<P>,
    cell: Rect,
    bar: ColumnBar,
    placing: &Placing<'_, P>,
    moved: &[P],
    rects: &mut Vec<Rect>,
) -> Option<()> {
    let moves = |panel: &P| moved.contains(panel);
    if !column.panels().any(moves) {
        return Some(());
    }
    if column.iconic || placing.sizes.stepped(column, placing.spec).is_some() {
        return None;
    }
    if column.panels().all(moves) {
        rects.push(cell);
        return Some(());
    }
    let groups = column.groups.iter().zip(group_rects(column, placing.spec, cell, bar, &placing.sizes));
    rects.extend(groups.filter(|(group, _)| group.panels.iter().any(moves)).map(|(_, rect)| rect));
    Some(())
}
