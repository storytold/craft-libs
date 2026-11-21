//! Scrubby labels: a drag on a label moves the value field drawn next to it on the same row
//! (Photoshop's scrubby labels), on either side of the label — mirrored rows put the field left
//! of it. The linkage is pure rect arithmetic, direction-agnostic, and the drag semantics stay
//! physical: drag right increases the value in both directions.

use egui::{Id, Rect, Response, Sense, Ui};

/// A drag on a value field's label this frame, left by [`scrub_label`] (or
/// [`scrub_option_label`]/[`scrub_option_icon`]), taken by that field with [`take_label_scrub`].
#[derive(Clone, Copy, Debug)]
pub struct LabelScrub {
    pass: u64,
    /// The id whose drags move the field (the label itself, or an options caption's hit area).
    pub label: Id,
    /// The visible label's own id, which names the field it scrubs.
    pub name: Option<Id>,
    /// The visible label's rect.
    pub rect: Rect,
    /// How far on either side of the label the field may start (the whole row for a slider row).
    pub max_gap: f32,
    /// Options captions defer interaction until the adjacent field bounds are known.
    pub option_area: bool,
    /// An options icon's tooltip, shown by the drag area laid over it (which takes its hover).
    pub tip: Option<&'static str>,
    /// Whole steps to move the value (right positive; Shift ×10, Alt a tenth as fast).
    pub steps: f32,
    pub started: bool,
    pub dragging: bool,
    pub stopped: bool,
}

fn label_scrub_id() -> Id {
    Id::unique("craft_ui::label-scrub")
}

/// Makes `label` scrubby: dragging it left or right moves the value field drawn next to it on
/// the same row (one step a point, ×10 with Shift, a tenth as fast with Alt), with a
/// horizontal-resize cursor while linked. A label with no value field beside it stays a plain
/// label.
pub fn scrub_label(ui: &Ui, label: &Response) -> Response {
    scrub_label_within(ui, label, 24.0)
}

/// [`scrub_label`] with a custom `max_gap` (a slider row passes `f32::INFINITY`, so its caption
/// scrubs across the whole row).
pub fn scrub_label_within(ui: &Ui, label: &Response, max_gap: f32) -> Response {
    // Drag only, not focusable: Tab goes to the value field, not to its label as well.
    let resp = label.interact(Sense::DRAG);
    let scrub = label_scrub_response(ui, &resp, label.rect, max_gap, Some(label.id));
    ui.data_mut(|d| {
        d.insert_temp(label_scrub_id(), scrub);
        d.insert_temp(resp.id.with("scrub-linked"), false);
    });
    resp
}

/// A tool-options caption whose scrub target includes the adjacent field's outer box. Keep the
/// original label bounds for linking; register the hit area before the numeric editor.
pub fn scrub_option_label(ui: &Ui, label: &Response) {
    scrub_option_area(ui, label, None);
}

/// [`scrub_option_label`] for an icon with tooltip `tip`. The field's drag area is laid over the
/// icon after it, so egui hovers the area and not the icon: an `on_hover_text` on the icon never
/// shows.
pub fn scrub_option_icon(ui: &Ui, icon: &Response, tip: &'static str) {
    scrub_option_area(ui, icon, Some(tip));
}

fn scrub_option_area(ui: &Ui, label: &Response, tip: Option<&'static str>) {
    let scrub = LabelScrub {
        pass: ui.ctx().cumulative_pass_nr(),
        label: label.id.with("option-scrub"),
        name: Some(label.id),
        rect: label.rect,
        max_gap: 24.0,
        option_area: true,
        tip,
        steps: 0.0,
        started: false,
        dragging: false,
        stopped: false,
    };
    ui.data_mut(|d| d.insert_temp(label_scrub_id(), scrub));
}

/// Accumulates `resp`'s drag into whole steps (physical: right positive; Shift ×10, Alt ÷10)
/// and builds the [`LabelScrub`] left for the field drawn next.
pub fn label_scrub_response(ui: &Ui, resp: &Response, original_rect: Rect, max_gap: f32, name: Option<Id>) -> LabelScrub {
    // Whether a field took this label's drags last frame: only then is it scrubby.
    let linked = ui.data(|d| d.get_temp::<bool>(resp.id.with("scrub-linked"))).unwrap_or(false);
    if linked && resp.enabled() && (resp.hovered() || resp.dragged()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    let acc_id = resp.id.with("scrub-acc");
    let mut steps = 0.0;
    if resp.dragged() || resp.drag_stopped() {
        let mut acc: f32 = if resp.drag_started() { 0.0 } else { ui.data(|d| d.get_temp(acc_id)).unwrap_or(0.0) };
        let mods = ui.input(|i| i.modifiers);
        let rate = if mods.shift {
            10.0
        } else if mods.alt {
            0.1
        } else {
            1.0
        };
        // The drag starts once the pointer has moved a few points: count those too.
        let dx = match (resp.drag_started(), ui.input(|i| i.pointer.press_origin()), resp.interact_pointer_pos()) {
            (true, Some(origin), Some(pos)) => pos.x - origin.x,
            _ => resp.drag_delta().x,
        };
        acc += dx * rate;
        if !acc.is_finite() {
            acc = 0.0;
        }
        steps = acc.trunc();
        ui.data_mut(|d| d.insert_temp(acc_id, acc - steps));
    }
    LabelScrub {
        pass: ui.ctx().cumulative_pass_nr(),
        label: resp.id,
        name,
        rect: original_rect,
        max_gap,
        option_area: false,
        tip: None,
        steps,
        started: resp.drag_started(),
        dragging: resp.dragged(),
        stopped: resp.drag_stopped(),
    }
}

/// The label drag left for the field at `rect` this frame, if its label is just beside it on
/// its row — on either side, since mirrored rows put the field left of its label. Taken either
/// way: a label only scrubs the next field drawn.
pub fn take_label_scrub(ui: &Ui, rect: Rect) -> Option<LabelScrub> {
    let s = ui.data_mut(|d| {
        let s = d.get_temp::<LabelScrub>(label_scrub_id());
        d.remove::<LabelScrub>(label_scrub_id());
        s
    })?;
    let same_row = rect.top() < s.rect.bottom() && rect.bottom() > s.rect.top();
    // The field may sit on either side of its label: accept the side in band.
    let gap_right = rect.left() - s.rect.right(); // field right of label (LTR rows)
    let gap_left = s.rect.left() - rect.right(); // field left of label (RTL rows)
    let in_band = |g: f32| g >= -1.0 && g <= s.max_gap;
    let linked = s.pass == ui.ctx().cumulative_pass_nr() && same_row && (in_band(gap_right) || in_band(gap_left));
    if linked {
        ui.data_mut(|d| d.insert_temp(s.label.with("scrub-linked"), true));
    }
    linked.then_some(s)
}

/// The visible caption left for the control drawn next this frame, if any, consumed so that a
/// non-scrubbing control (a dropdown) can still take its name and the caption cannot leak on.
pub fn take_label_scrub_name(ui: &Ui, rect: Rect) -> Option<Id> {
    let s = ui.data_mut(|d| {
        let s = d.get_temp::<LabelScrub>(label_scrub_id());
        d.remove::<LabelScrub>(label_scrub_id());
        s
    })?;
    if s.pass != ui.ctx().cumulative_pass_nr() {
        return None;
    }
    // Only a caption on the control's own row names it; one left over from an earlier row does not.
    let same_row = rect.top() < s.rect.bottom() && rect.bottom() > s.rect.top();
    same_row.then_some(s.name).flatten()
}

#[cfg(test)]
mod tests {
    use egui::{Align, Context, Modifiers, Pos2, RawInput, Rect, pos2, vec2};

    use super::*;

    fn frame(ctx: &Context, add: impl FnMut(&mut Ui)) {
        let raw = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 400.0))), ..Default::default() };
        ctx.run_ui(raw, add).textures_delta.clear();
    }

    /// The link is accepted on either side of the label and refused past `max_gap`, whatever the
    /// direction: the rect arithmetic is direction-agnostic.
    #[test]
    fn the_scrub_gap_is_accepted_on_either_side() {
        let ctx = Context::default();
        let label_rect = Rect::from_min_size(pos2(300.0, 100.0), vec2(40.0, 20.0));
        let plant = |ui: &mut Ui| {
            let pass = ui.ctx().cumulative_pass_nr();
            ui.data_mut(|d| {
                d.insert_temp(
                    label_scrub_id(),
                    LabelScrub {
                        pass,
                        label: Id::unique("l"),
                        name: None,
                        rect: label_rect,
                        max_gap: 24.0,
                        option_area: false,
                        tip: None,
                        steps: 0.0,
                        started: false,
                        dragging: false,
                        stopped: false,
                    },
                );
            });
        };
        let mut taken = None;
        // A field to the right of the label (LTR row shape) links.
        frame(&ctx, |ui| {
            plant(ui);
            taken = take_label_scrub(ui, Rect::from_min_size(pos2(label_rect.right() + 4.0, 100.0), vec2(80.0, 20.0)));
        });
        assert!(taken.is_some(), "field right of the label links");
        // A field to the left of the label (mirrored row shape) links too.
        frame(&ctx, |ui| {
            plant(ui);
            taken = take_label_scrub(ui, Rect::from_min_size(pos2(label_rect.left() - 4.0 - 80.0, 100.0), vec2(80.0, 20.0)));
        });
        assert!(taken.is_some(), "field left of the label links (mirrored rows)");
        // A gap past max_gap on the mirrored side does not link.
        frame(&ctx, |ui| {
            plant(ui);
            taken = take_label_scrub(ui, Rect::from_min_size(pos2(label_rect.left() - 60.0 - 80.0, 100.0), vec2(80.0, 20.0)));
        });
        assert!(taken.is_none(), "a gap past max_gap does not link on the mirrored side either");
    }

    /// Dragging a label right of its field scrubs that field, and the drag stays physical:
    /// drag right still increases the value.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn dragging_a_label_scrubs_the_field_left_of_it_in_rtl() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable as _;

        use crate::direction::{Direction, apply};

        let mut h = Harness::new_ui_state(
            |ui, s: &mut (f32, f32)| {
                apply(ui.ctx(), Direction::Rtl);
                // Same draw order as the LTR twin; the mirrored row places the label right of the field.
                ui.with_layout(egui::Layout::right_to_left(Align::Min), |ui| {
                    ui.add_space(40.0); // keep the label clear of the window's right edge
                    let l = ui.label("Size:");
                    scrub_label(ui, &l);
                    let (r, _) = ui.allocate_exact_size(vec2(80.0, 20.0), Sense::hover());
                    if let Some(scrub) = take_label_scrub(ui, r) {
                        let unit = 1.0;
                        let v = ((s.0 / unit).round() + scrub.steps) * unit;
                        let v = (v * 1e4).round() / 1e4;
                        if v.is_finite() && (0.0..=1000.0).contains(&v) {
                            s.0 = v;
                        }
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(Align::Min), |ui| {
                    ui.add_space(40.0);
                    let l = ui.label("Mode:");
                    scrub_label(ui, &l);
                    ui.add_space(60.0);
                    let (r, _) = ui.allocate_exact_size(vec2(80.0, 20.0), Sense::hover());
                    if let Some(scrub) = take_label_scrub(ui, r) {
                        let v = s.1.round() + scrub.steps;
                        if v.is_finite() && (0.0..=1000.0).contains(&v) {
                            s.1 = v;
                        }
                    }
                });
            },
            (100.0, 100.0),
        );
        h.run();
        let drag = |h: &mut Harness<'static, (f32, f32)>, label: &str, dx: f32, mods: Modifiers| {
            let at = h.get_by_label(label).rect().center();
            h.hover_at(at);
            h.run();
            h.drag_at(at);
            h.run();
            for i in 1..=4 {
                h.event(egui::Event::PointerMoved(at + egui::vec2(dx * i as f32 / 4.0, 0.0)));
                h.event(egui::Event::ModifiersChanged(mods));
                h.run();
            }
            h.drop_at(at + egui::vec2(dx, 0.0));
            h.run();
        };
        drag(&mut h, "Size:", 20.0, Modifiers::NONE);
        assert_eq!(h.state().0, 120.0, "the field left of its label scrubs too");
        drag(&mut h, "Size:", -20.0, Modifiers::NONE);
        assert_eq!(h.state().0, 100.0);
        drag(&mut h, "Mode:", 20.0, Modifiers::NONE);
        assert_eq!(h.state().1, 100.0, "a gap past max_gap doesn't link on the mirrored side either");
    }
}
