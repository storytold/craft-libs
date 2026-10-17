use egui::{Pos2, Rect, pos2};

use super::{Placement, Zone};

/// One visible group, supplied in back-to-front order to [`resolve_drop`]. An empty tab strip
/// or a group that rejects tabs may still accept side splits, e.g. a protected document canvas.
#[derive(Clone, Debug)]
pub struct DropTarget<P> {
    pub anchor: P,
    pub rect: Rect,
    pub tab_strip: Option<Rect>,
    pub tabs: Vec<(P, Rect)>,
    pub allow_splits: bool,
    pub accepts_tabs: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DropProposal<P> {
    pub anchor: P,
    pub placement: Placement<P>,
    pub preview: Rect,
    pub insertion: Option<Rect>,
    /// The pointer addresses the tab's current insertion slot.
    pub noop: bool,
}

pub(super) fn usable(rect: Rect) -> bool {
    rect.is_finite() && rect.width() > 0.0 && rect.height() > 0.0 && rect.width() <= 1_000_000.0 && rect.height() <= 1_000_000.0
}

/// Tab header is the center; otherwise the nearest quarter-width/height edge wins.
/// Outside, non-finite, empty and inverted rectangles have no target.
pub fn drop_zone(rect: Rect, tab_height: f32, pointer: Pos2) -> Option<Zone> {
    if !usable(rect) || !pointer.is_finite() || !rect.contains(pointer) || !tab_height.is_finite() || tab_height < 0.0 {
        return None;
    }
    if pointer.y < rect.top() + tab_height.min(rect.height()) {
        return Some(Zone::Center);
    }
    let x = (pointer.x - rect.left()) / rect.width();
    let y = (pointer.y - rect.top()) / rect.height();
    let distances = [(x, Zone::Left), (1.0 - x, Zone::Right), (y, Zone::Top), (1.0 - y, Zone::Bottom)];
    let (distance, zone) = distances.into_iter().min_by(|a, b| a.0.total_cmp(&b.0))?;
    Some(if distance > 0.25 { Zone::Center } else { zone })
}

/// Preview of the region occupied after a split or group merge.
pub fn zone_rect(rect: Rect, zone: Zone) -> Option<Rect> {
    if !usable(rect) {
        return None;
    }
    let center = rect.min + rect.size() * 0.5;
    Some(match zone {
        Zone::Center => rect,
        Zone::Left => Rect::from_min_max(rect.min, pos2(center.x, rect.bottom())),
        Zone::Right => Rect::from_min_max(pos2(center.x, rect.top()), rect.max),
        Zone::Top => Rect::from_min_max(rect.min, pos2(rect.right(), center.y)),
        Zone::Bottom => Rect::from_min_max(pos2(rect.left(), center.y), rect.max),
    })
}

/// Bounded insertion marker for a logical slot in a measured tab strip. Hidden tabs
/// retain their order; their slot uses the next visible boundary or the visible strip end.
pub fn tab_insertion_marker<P: Eq>(target: &DropTarget<P>, before: Option<&P>) -> Option<Rect> {
    if target.tabs.len() > super::MAX_PANELS || !target.accepts_tabs || !usable(target.rect) {
        return None;
    }
    let strip = target.tab_strip?.intersect(target.rect);
    if !usable(strip) {
        return None;
    }
    let index = before.and_then(|before| target.tabs.iter().position(|(panel, _)| panel == before));
    let x = index
        .and_then(|index| target.tabs.get(index..))
        .and_then(|tabs| tabs.iter().find(|(_, rect)| usable(*rect)).map(|(_, rect)| rect.left()))
        .unwrap_or_else(|| {
            target.tabs.iter().filter(|(_, rect)| usable(*rect)).map(|(_, rect)| rect.right()).fold(strip.left(), f32::max).min(strip.right())
        });
    let width = strip.width().min(2.0);
    let left = (x - width * 0.5).max(strip.left()).min((strip.right() - width).max(strip.left()));
    Some(Rect::from_min_max(pos2(left, strip.top()), pos2((left + width).min(strip.right()), strip.bottom())))
}

/// Resolve a pointer against visible groups. The frontmost containing group occludes groups
/// behind it even if it rejects the drop. A strip reorders by midpoint; the body of the source
/// group is a no-op at center. Floating groups normally set `allow_splits` to false.
pub fn resolve_drop<P: Clone + Eq>(panel: &P, pointer: Pos2, targets: &[DropTarget<P>]) -> Option<DropProposal<P>> {
    if !pointer.is_finite() || targets.len() > super::MAX_PANELS {
        return None;
    }
    let target = targets.iter().rev().find(|target| usable(target.rect) && target.rect.contains(pointer))?;
    if target.tabs.len() > super::MAX_PANELS {
        return None;
    }
    let anchor = if &target.anchor == panel { target.tabs.iter().find(|(p, _)| p != panel).map(|(p, _)| p).unwrap_or(panel) } else { &target.anchor };
    let strip = target.tab_strip.map(|strip| strip.intersect(target.rect)).filter(|r| usable(*r) && r.contains(pointer));
    if strip.is_some() {
        if !target.accepts_tabs {
            return None;
        }
        let before = target.tabs.iter().filter(|(_, r)| usable(*r)).find(|(_, r)| pointer.x < r.center().x);
        let before_panel = before.map(|(p, _)| p.clone());
        let insertion = tab_insertion_marker(target, before_panel.as_ref())?;
        let old_order: Vec<_> = target.tabs.iter().map(|(p, _)| p).collect();
        let mut order = old_order.clone();
        let noop = if before_panel.as_ref() == Some(panel) {
            old_order.contains(&panel)
        } else if old_order.contains(&panel) {
            order.retain(|p| *p != panel);
            let at = before_panel.as_ref().and_then(|before| order.iter().position(|p| *p == before)).unwrap_or(order.len());
            order.insert(at, panel);
            order == old_order
        } else {
            false
        };
        return Some(DropProposal {
            anchor: anchor.clone(),
            placement: Placement::Tab { before: before_panel },
            preview: target.rect,
            insertion: Some(insertion),
            noop,
        });
    }
    let zone = if target.allow_splits { drop_zone(target.rect, 0.0, pointer)? } else { Zone::Center };
    if zone == Zone::Center && target.accepts_tabs && target.tabs.iter().any(|(p, _)| p == panel) {
        return Some(DropProposal {
            anchor: anchor.clone(),
            placement: Placement::Tab { before: Some(panel.clone()) },
            preview: target.rect,
            insertion: None,
            noop: true,
        });
    }
    if anchor == panel {
        return None;
    }
    if zone == Zone::Center {
        if !target.accepts_tabs || target.tabs.iter().any(|(p, _)| p == panel) {
            return None;
        }
    } else if !target.allow_splits {
        return None;
    }
    Some(DropProposal {
        anchor: anchor.clone(),
        placement: Placement::Split(zone),
        preview: zone_rect(target.rect, zone)?,
        insertion: None,
        noop: false,
    })
}
