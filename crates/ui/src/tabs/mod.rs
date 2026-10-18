//! Tab fitting and accessible tab interactions without owning a dock or document model.
//!
//! The caller owns identity, localization, selection, painting, overflow menus and drag actions.
//! [`fit`] only assigns widths; [`Tab`] returns the complete egui response, including double-click
//! and drag phases, so a panel strip can collapse, reorder or tear off according to its app.

use egui::{Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};

/// Tabs visible on a strip and tabs that need an overflow menu, all in original order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TabFit {
    /// `(input index, width)` for visible tabs. A nonempty input always retains the selected tab
    /// (or the final tab if `selected` was out of bounds), even when its width must become zero.
    pub shown: Vec<(usize, f32)>,
    pub overflow: Vec<usize>,
    /// Reserved width of the overflow button, zero when no tabs overflow. This can be smaller
    /// than the requested width in a tiny strip; use it when laying out the button.
    pub overflow_width: f32,
}

/// Largest meaningful extent for one UI component, in logical points.
const MAX_EXTENT: f32 = 1_000_000.0;

fn extent(value: f32) -> f32 {
    if value.is_finite() { value.clamp(0.0, MAX_EXTENT) } else { 0.0 }
}

/// Fit tab widths into `available` points, shrinking the widest tabs first to a common cap.
/// Each tab normally retains at least `min(natural_width, minimum_width)`. If that cannot fit,
/// retain the selected tab first, then as many others as fit, and reserve an overflow button.
/// A selected tab in a very narrow strip takes the space left after that button.
///
/// Negative and non-finite dimensions become zero; finite dimensions are capped at one million
/// points. Empty input and an out-of-range selection are valid. The widths of all shown tabs plus
/// `overflow_width` never exceed the sanitized available width. Work and storage are linear in
/// the input count (with at most 32 fitting passes); callers keep their own tab-count policy.
pub fn fit(natural: &[f32], selected: usize, available: f32, minimum_width: f32, overflow_width: f32) -> TabFit {
    let available = extent(available);
    let minimum_width = extent(minimum_width);
    let natural: Vec<f32> = natural.iter().map(|&w| extent(w)).collect();
    let all: Vec<usize> = (0..natural.len()).collect();
    if natural.iter().map(|&w| f64::from(w)).sum::<f64>() <= f64::from(available) {
        return TabFit { shown: all.into_iter().zip(natural).collect(), ..Default::default() };
    }
    // One tab has nowhere to overflow; all available space belongs to it.
    if natural.len() == 1 {
        return TabFit { shown: vec![(0, available)], ..Default::default() };
    }
    let floor = |w: f32| w.min(minimum_width);
    if natural.iter().map(|&w| f64::from(floor(w))).sum::<f64>() <= f64::from(available) {
        return TabFit { shown: squeeze(&natural, &all, available, minimum_width), ..Default::default() };
    }
    let overflow_width = extent(overflow_width).min(available);
    let budget = available - overflow_width;
    let selected = selected.min(natural.len().saturating_sub(1));
    let mut keep = vec![false; natural.len()];
    let mut used = 0.0f64;
    for i in std::iter::once(selected).chain(all.iter().copied().filter(|&i| i != selected)) {
        let Some(&width) = natural.get(i) else { continue };
        let width = f64::from(floor(width));
        if i == selected || used + width <= f64::from(budget) {
            if let Some(keep) = keep.get_mut(i) {
                *keep = true;
            }
            used += width;
        }
    }
    let (shown, overflow): (Vec<usize>, Vec<usize>) = all.into_iter().partition(|&i| keep.get(i).copied().unwrap_or(false));
    let shown = squeeze(&natural, &shown, budget, minimum_width);
    TabFit { shown, overflow, overflow_width }
}

fn squeeze(natural: &[f32], indices: &[usize], total: f32, minimum: f32) -> Vec<(usize, f32)> {
    let width = |i: usize| natural.get(i).copied().unwrap_or(0.0);
    let sum_at = |cap: f32| indices.iter().map(|&i| f64::from(width(i).min(cap.max(width(i).min(minimum))))).sum::<f64>();
    let (mut lo, mut hi) = (minimum, indices.iter().map(|&i| width(i)).fold(minimum, f32::max));
    if sum_at(hi) > f64::from(total) {
        for _ in 0..32 {
            let mid = lo + (hi - lo) * 0.5;
            if sum_at(mid) <= f64::from(total) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
    } else {
        lo = hi;
    }
    let mut remaining = total;
    indices
        .iter()
        .map(|&i| {
            let natural = width(i);
            // Also handles a selected tab whose minimum alone exceeds the budget, and prevents
            // accumulated floating-point rounding from placing the overflow button past the end.
            let width = natural.min(lo.max(natural.min(minimum))).min(remaining);
            remaining = (remaining - width).max(0.0);
            (i, width)
        })
        .collect()
}

/// An accessible tab at a caller-supplied rectangle, with caller-supplied painting.
///
/// IDs must be unique in their UI scope and independent of translated labels. Selection remains
/// caller-owned. `Sense::click()` is the default; use [`Self::sense`] for draggable dock tabs.
pub struct Tab<'a> {
    id: Id,
    label: &'a str,
    selected: bool,
    sense: Sense,
    focus_stroke: Option<Stroke>,
}

impl<'a> Tab<'a> {
    pub fn new(id: Id, label: &'a str, selected: bool) -> Self {
        Self { id, label, selected, sense: Sense::click(), focus_stroke: None }
    }

    pub fn sense(mut self, sense: Sense) -> Self {
        self.sense = sense;
        self
    }

    /// Set the inset keyboard-focus outline using the application's theme. The default uses
    /// egui's selection stroke. An explicit `Stroke::NONE` disables the outline.
    pub fn focus_stroke(mut self, stroke: Stroke) -> Self {
        self.focus_stroke = Some(stroke);
        self
    }

    /// Register a tab, call its painter and return the unmodified click/drag response.
    ///
    /// This does not allocate space or alter the UI cursor. The painter receives the response
    /// for hover/press styling. Non-finite, empty, inverted or excessive rectangles yield an inert
    /// zero-size response and do not invoke the painter. Parent disabled state applies normally.
    pub fn show_at(self, ui: &mut Ui, rect: Rect, paint: impl FnOnce(&Ui, &Response)) -> Response {
        let size = rect.size();
        let valid = rect.is_finite() && size.x > 0.0 && size.y > 0.0 && size.x <= MAX_EXTENT && size.y <= MAX_EXTENT;
        let safe_rect = if valid { rect } else { Rect::from_min_size(egui::Pos2::ZERO, Vec2::ZERO) };
        let response = ui.interact(safe_rect, self.id, if valid { self.sense } else { Sense::hover() });
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, valid && ui.is_enabled(), self.selected, self.label));
        ui.ctx().accesskit_node_builder(self.id, |node| {
            node.set_role(egui::accesskit::Role::Tab);
            node.clear_toggled();
            node.set_selected(self.selected);
        });
        if !valid {
            return response;
        }
        paint(ui, &response);
        if response.has_focus() && rect.width() > 4.0 && rect.height() > 4.0 {
            let focus = rect.shrink(2.0);
            let stroke = self.focus_stroke.unwrap_or(ui.visuals().selection.stroke);
            if stroke.width.is_finite() && stroke.width > 0.0 {
                let stroke = Stroke::new(stroke.width.min(focus.width().min(focus.height()) * 0.5), stroke.color);
                ui.painter().rect_stroke(focus, 0.0, stroke, StrokeKind::Inside);
            }
        }
        response
    }
}

#[cfg(test)]
mod tests;
