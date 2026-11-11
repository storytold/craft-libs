//! The overlay's keys while its group has focus: the picked part moved, removed, added after or asked for its colour.

use craft_behavior::kurbo::Vec2;
use craft_behavior::{Granularity, Intent};
use craft_ui::canvas::{NUDGE, NUDGE_FAR};
use craft_ui::contract::{Cx, Edit as _, Focus as _};

use super::parts::line_part;
use super::parts::{part_ref, selection_of};
use super::{CMD_ENTER, ENTER, GradientOverlay, INSERT, SPACE};
use crate::ui::gradient::stops::{self, STOP_STEP, STOP_STEP_FAR, round_location};
use crate::ui::gradient::{GradientLine, LineGradient, LinePart, Sampler, StopRow, default_sampler};

impl GradientOverlay<'_> {
    /// Selection follows the focus. While focused: the keys of the picked part (the line when none is picked).
    pub(super) fn run_keys(&mut self, cx: &mut Cx<'_, Self>, shown: &LineGradient) {
        let active = cx.active_part();
        let picked = active.and_then(line_part);
        // Focus arriving at a part selects it, unless a press holds the part (the press selects by its own rule).
        if let Some(part) = cx.focus_arrived_by_key()
            && let Some(selected) = line_part(part).and_then(selection_of)
        {
            self.select(cx, Some(selected));
        }
        if !cx.is_focused() {
            return;
        }
        let mut part = picked.unwrap_or(LinePart::Line);
        let mut value = shown.clone();
        let default = default_sampler(self.rules.rows);
        let sampler: &Sampler = self.sampler.unwrap_or(&default);

        let add = cx.take_claimed(INSERT) | cx.take_claimed(CMD_ENTER);
        if add {
            let after = match part {
                LinePart::Stop(i) | LinePart::Midpoint(i) => Some(i),
                _ => None,
            };
            let at = stops::halfway(&value.stops, StopRow::Color, after);
            if let Some((added, i)) = stops::add(&value.stops, StopRow::Color, at, sampler, &self.rules) {
                value.stops = added;
                part = LinePart::Stop(i);
            }
        }
        // Enter and Space are claimed only where they ask for a colour; elsewhere they are the host's.
        let request = self.color_request(shown, part);
        if request.is_some() {
            cx.claim(ENTER);
            cx.claim(SPACE);
        }
        if cx.take_claimed(ENTER) | cx.take_claimed(SPACE) {
            self.report.color_requested = request;
        }
        let delete =
            cx.intents().take_exact(&Intent::delete(true, Granularity::Unit)) | cx.intents().take_exact(&Intent::delete(false, Granularity::Unit));
        let nudge = cx.intents().take_nudge();
        match part {
            LinePart::Stop(i) | LinePart::Midpoint(i) if i < value.stops.color.len() => {
                if delete && matches!(part, LinePart::Stop(_)) {
                    if let Some((removed, j)) = stops::remove(&value.stops, StopRow::Color, i, &self.rules) {
                        value.stops = removed;
                        part = LinePart::Stop(j);
                    }
                } else if let Some((direction, mods)) = nudge.filter(|(direction, _)| direction.x != 0.0) {
                    let by = direction.x.signum() * if mods.shift { STOP_STEP_FAR } else { STOP_STEP };
                    let moved = match part {
                        LinePart::Midpoint(_) => {
                            let at = value.stops.color[i].midpoint + by;
                            stops::set_midpoint(&value.stops, StopRow::Color, i, at, &self.rules).map(|(s, _)| (s, part))
                        }
                        _ => {
                            let at = round_location(value.stops.color[i].location + by);
                            stops::move_to(&value.stops, StopRow::Color, i, at, &self.rules).map(|(s, j)| (s, LinePart::Stop(j)))
                        }
                    };
                    if let Some((s, p)) = moved {
                        value.stops = s;
                        part = p;
                    }
                }
            }
            LinePart::Line | LinePart::Start | LinePart::End => {
                if let Some((direction, mods)) = nudge {
                    let step = if mods.shift { NUDGE_FAR } else { NUDGE };
                    let by = Vec2::new(f64::from(direction.x), f64::from(direction.y)) * step;
                    let GradientLine { start, end } = value.line;
                    value.line = match part {
                        LinePart::Start => GradientLine { start: start + by, end },
                        LinePart::End => GradientLine { start, end: end + by },
                        _ => GradientLine { start: start + by, end: end + by },
                    };
                }
            }
            _ => {}
        }
        if value != *shown {
            cx.set(value);
            if let Some(selected) = selection_of(part) {
                self.select(cx, Some(selected));
            }
            if part != picked.unwrap_or(LinePart::Line) {
                cx.focus_part(part_ref(part));
            }
        }
    }
}
