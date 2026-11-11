//! The point at which the suite presses a collection row to give it the lead or pick it: a point of the row's own
//! area (17.5). A row that holds child components (a switch, a select) is pressed where none of them is, so the
//! press reaches the row and not a child.

use super::nodes::subtree;
use crate::runtime::egui::access::Declared;
use crate::testing::fixture::{Fixture, Target};
use egui::{Pos2, Rect};

/// The point of `row` to press: its centre when no focusable or clickable descendant covers it, else the middle of the widest
/// stretch of the row's centre line that none covers, and the centre when the descendants cover the whole line.
pub(super) fn point(fixture: &Fixture, row: &Target) -> Pos2 {
    let tree = fixture.tree();
    let centre = row.rect.center();
    let mut covered: Vec<Rect> = subtree(tree, row.node)
        .into_iter()
        .skip(1)
        .filter(|node| tree.declares_key(*node, Declared::Focus) || tree.declares_key(*node, Declared::Click))
        .filter_map(|node| tree.bounds_of(node))
        .filter(|bounds| bounds.min.y <= centre.y && centre.y <= bounds.max.y)
        .collect();
    if !covered.iter().any(|bounds| bounds.contains(centre)) {
        return centre;
    }
    covered.sort_by(|a, b| a.min.x.total_cmp(&b.min.x));
    let mut widest: Option<(f32, f32)> = None;
    let mut free = row.rect.min.x;
    for (from, to) in covered.iter().map(|bounds| (bounds.min.x, bounds.max.x)).chain([(row.rect.max.x, row.rect.max.x)]) {
        if from > free && widest.is_none_or(|(start, end)| from - free > end - start) {
            widest = Some((free, from.min(row.rect.max.x)));
        }
        free = free.max(to);
    }
    widest.map_or(centre, |(start, end)| Pos2::new((start + end) / 2.0, centre.y))
}

/// Presses `row` at its own area.
pub(super) fn click(fixture: &mut Fixture, row: &Target) {
    let at = point(fixture, row);
    fixture.click_at(at);
}
