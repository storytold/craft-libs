//! Shared numeric input state, measurement unit parsing, and expression evaluation.
//!
//! Provides unit parsing for `pt`, `mm`, `in`, `pc`, `Q`, `Ha`, arithmetic expression evaluation,
//! and label scrubbing drag state management that preserves application-specific command dispatch.

use std::borrow::Cow;
use std::ops::RangeInclusive;

use egui::{Context, CursorIcon, Id, Key, Rect, Response, Sense, Ui};

#[cfg(test)]
mod tests;

/// Maximum number of `+ - * /` signs allowed in one measure expression.
pub const MAX_MEASURE_OPERATORS: usize = 128;

/// Points of horizontal drag on a label per step.
pub const PX_PER_STEP: f32 = 2.0;

/// Widest gap between a label and the field it names.
const LABEL_GAP: f32 = 64.0;

/// Standard measurement units (display only; model values are in points).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MeasureUnit {
    #[default]
    Points,
    Picas,
    Inches,
    Millimeters,
    Centimeters,
    Pixels,
    FeetInches,
    Meters,
    Yards,
    Feet,
    /// Q (級), a quarter of a millimeter: Japanese typesetting unit for type size.
    Q,
    /// Ha (歯), a quarter of a millimeter: Japanese unit for spacing, leading, and positions.
    Ha,
}

impl MeasureUnit {
    /// Standard unit collection.
    pub const ALL: [MeasureUnit; 10] = [
        MeasureUnit::Points,
        MeasureUnit::Picas,
        MeasureUnit::Inches,
        MeasureUnit::Millimeters,
        MeasureUnit::Centimeters,
        MeasureUnit::Pixels,
        MeasureUnit::FeetInches,
        MeasureUnit::Meters,
        MeasureUnit::Yards,
        MeasureUnit::Feet,
    ];

    /// Units offered for East Asian typography: all of `ALL` plus `Q` and `Ha`.
    pub const EAST_ASIAN: [MeasureUnit; 12] = [
        MeasureUnit::Points,
        MeasureUnit::Picas,
        MeasureUnit::Inches,
        MeasureUnit::Millimeters,
        MeasureUnit::Centimeters,
        MeasureUnit::Pixels,
        MeasureUnit::FeetInches,
        MeasureUnit::Meters,
        MeasureUnit::Yards,
        MeasureUnit::Feet,
        MeasureUnit::Q,
        MeasureUnit::Ha,
    ];

    /// Number of points per unit.
    pub fn points(self) -> f64 {
        match self {
            MeasureUnit::Points | MeasureUnit::Pixels => 1.0,
            MeasureUnit::Picas => 12.0,
            MeasureUnit::Inches => 72.0,
            MeasureUnit::Millimeters => 72.0 / 25.4,
            MeasureUnit::Centimeters => 72.0 / 2.54,
            MeasureUnit::Q | MeasureUnit::Ha => 72.0 / 101.6,
            MeasureUnit::Meters => 72.0 / 0.0254,
            MeasureUnit::Yards => 72.0 * 36.0,
            MeasureUnit::Feet | MeasureUnit::FeetInches => 72.0 * 12.0,
        }
    }

    /// Primary suffix string representing this unit (e.g. `pt`, `mm`, `in`, `p`, `Q`, `H`).
    pub fn suffix(self) -> &'static str {
        match self {
            MeasureUnit::Points => "pt",
            MeasureUnit::Picas => "p",
            MeasureUnit::Inches => "in",
            MeasureUnit::Millimeters => "mm",
            MeasureUnit::Centimeters => "cm",
            MeasureUnit::Pixels => "px",
            MeasureUnit::FeetInches | MeasureUnit::Feet => "ft",
            MeasureUnit::Meters => "m",
            MeasureUnit::Yards => "yd",
            MeasureUnit::Q => "Q",
            MeasureUnit::Ha => "H",
        }
    }

    /// Whether this unit is a physical measurement unit (not points or pixels).
    pub fn is_physical(self) -> bool {
        self.svg_suffix().is_some()
            || matches!(
                self,
                MeasureUnit::FeetInches | MeasureUnit::Meters | MeasureUnit::Yards | MeasureUnit::Feet | MeasureUnit::Q | MeasureUnit::Ha
            )
    }

    /// Standard SVG length unit suffix if supported in SVG (`mm`, `cm`, `in`, `pc`).
    pub fn svg_suffix(self) -> Option<&'static str> {
        match self {
            MeasureUnit::Millimeters => Some("mm"),
            MeasureUnit::Centimeters => Some("cm"),
            MeasureUnit::Inches => Some("in"),
            MeasureUnit::Picas => Some("pc"),
            _ => None,
        }
    }

    /// Display label for preferences and menus.
    pub const fn label(self) -> &'static str {
        match self {
            MeasureUnit::Points => "Points",
            MeasureUnit::Picas => "Picas",
            MeasureUnit::Inches => "Inches",
            MeasureUnit::Millimeters => "Millimeters",
            MeasureUnit::Centimeters => "Centimeters",
            MeasureUnit::Pixels => "Pixels",
            MeasureUnit::FeetInches => "Feet & Inches",
            MeasureUnit::Meters => "Meters",
            MeasureUnit::Yards => "Yards",
            MeasureUnit::Q => "Q",
            MeasureUnit::Ha => "Ha",
            MeasureUnit::Feet => "Feet",
        }
    }

    /// Rulers' label step at `zoom` (screen pixels per point), in this unit.
    pub fn ruler_step(self, zoom: f64) -> f64 {
        const DECIMAL: [f64; 19] =
            [0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0];
        const BINARY: [f64; 15] = [1.0 / 16.0, 0.125, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 10000.0];
        let steps: &[f64] = if self.ruler_halves() { &BINARY } else { &DECIMAL };
        steps.iter().copied().find(|s| s * self.points() * zoom >= 50.0).unwrap_or(10000.0)
    }

    /// Does this unit's ruler divide a step in half rather than in ten? Inches do.
    pub fn ruler_halves(self) -> bool {
        matches!(self, MeasureUnit::Inches)
    }

    /// Unlabelled ticks between two labels `step` apart.
    pub fn ruler_minor(self, step: f64) -> f64 {
        if self.ruler_halves() { step / 8.0 } else { step / 10.0 }
    }

    /// `v` (points) on the nearest ruler tick at `zoom`.
    pub fn snap_to_ruler_tick(self, v: f64, zoom: f64) -> f64 {
        let tick = self.ruler_minor(self.ruler_step(zoom)) * self.points();
        if tick <= 0.0 || !tick.is_finite() {
            return v;
        }
        (v / tick).round() * tick
    }

    /// Convert from points to this unit.
    pub fn from_pt(self, v: f64) -> f64 {
        v / self.points()
    }

    /// Convert from this unit to points.
    pub fn to_pt(self, v: f64) -> f64 {
        v * self.points()
    }

    /// Format a point value with unit suffix (e.g. `12.5 pt`).
    pub fn format(self, pt: f64) -> String {
        format!("{} {}", self.number(pt), self.suffix())
    }

    /// Format a point value without the suffix (e.g. `12.5`).
    pub fn number(self, pt: f64) -> String {
        let decimals =
            if matches!(self, MeasureUnit::Points | MeasureUnit::Pixels | MeasureUnit::Picas | MeasureUnit::Q | MeasureUnit::Ha) { 3 } else { 4 };
        let s = format!("{:.*}", decimals, self.from_pt(pt));
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s == "-0" { "0".into() } else { s.into() }
    }

    /// Format with two decimals and suffix for canvas readout (e.g. `12.50 mm`).
    pub fn readout(self, pt: f64) -> String {
        let s = format!("{:.2}", self.from_pt(pt));
        let s = if s == "-0.00" { "0.00" } else { &s };
        format!("{} {}", s, self.suffix())
    }

    /// Key naming this unit in settings/preferences.
    pub const fn key(self) -> &'static str {
        match self {
            MeasureUnit::Points => "points",
            MeasureUnit::Picas => "picas",
            MeasureUnit::Inches => "inches",
            MeasureUnit::Millimeters => "millimeters",
            MeasureUnit::Centimeters => "centimeters",
            MeasureUnit::Pixels => "pixels",
            MeasureUnit::FeetInches => "feetInches",
            MeasureUnit::Meters => "meters",
            MeasureUnit::Yards => "yards",
            MeasureUnit::Feet => "feet",
            MeasureUnit::Q => "q",
            MeasureUnit::Ha => "ha",
        }
    }

    /// Find a unit in `ALL` matching `name` (label, suffix, or key case-insensitively).
    pub fn named(name: &str) -> Option<MeasureUnit> {
        Self::named_in(&Self::ALL, name)
    }

    /// Find a unit in `EAST_ASIAN` matching `name` (also `Q` and `Ha`).
    pub fn named_east_asian(name: &str) -> Option<MeasureUnit> {
        Self::named_in(&Self::EAST_ASIAN, name)
    }

    fn named_in(units: &[MeasureUnit], name: &str) -> Option<MeasureUnit> {
        let letters = |s: &'static str| s.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase());
        let given = || name.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase());
        units.iter().copied().find(|u| letters(u.label()).eq(given())).or_else(|| {
            units.iter().copied().find(|u| {
                if *u == MeasureUnit::Picas && (name.eq_ignore_ascii_case("p") || name.eq_ignore_ascii_case("pc")) {
                    return true;
                }
                if *u != MeasureUnit::FeetInches && u.suffix().chars().map(|c| c.to_ascii_lowercase()).eq(given()) {
                    return true;
                }
                if let Some(svg) = u.svg_suffix()
                    && svg.chars().map(|c| c.to_ascii_lowercase()).eq(given())
                {
                    return true;
                }
                false
            })
        })
    }

    /// Parse text into points, evaluating arithmetic expressions and unit suffixes.
    pub fn parse(self, s: &str) -> Option<f64> {
        if arithmetic_signs(s) > MAX_MEASURE_OPERATORS {
            return None;
        }
        let normalized = decimal_point(s);
        parse_measure(&times_as_asterisks(&normalized), self).filter(|v| v.is_finite())
    }
}

/// Number of `+ - * /` signs in `s`, counting `x`, `X` or `×` multiplication signs as `*`.
pub fn arithmetic_signs(s: &str) -> usize {
    let mut prev = None;
    s.chars()
        .filter(|&c| {
            let sign = matches!(c, '+' | '-' | '*' | '/') || is_times(c, prev);
            prev = Some(c);
            sign
        })
        .count()
}

fn is_times(c: char, prev: Option<char>) -> bool {
    matches!(c, 'X' | '×') || (c == 'x' && prev != Some('p'))
}

fn times_as_asterisks(s: &str) -> Cow<'_, str> {
    let mut prev = None;
    let any = s.chars().any(|c| {
        let times = is_times(c, prev);
        prev = Some(c);
        times
    });
    if !any {
        return s.into();
    }
    let mut out = String::with_capacity(s.len());
    let mut prev = None;
    for c in s.chars() {
        out.push(if is_times(c, prev) { '*' } else { c });
        prev = Some(c);
    }
    out.into()
}

/// Convert decimal commas to periods if not written as thousands separators in an operand.
pub fn decimal_point(text: &str) -> Cow<'_, str> {
    if !text.contains(',') {
        return text.into();
    }
    let mut out = String::with_capacity(text.len());
    let mut operand_start = 0;
    let mut prev = None;
    let mut after_operator = true;

    for (i, c) in text.char_indices() {
        let is_leading = after_operator;
        let is_op = matches!(c, '*' | '/') || is_times(c, prev) || (matches!(c, '+' | '-') && !is_leading);

        if is_op {
            normalize_operand(&text[operand_start..i], &mut out);
            out.push(c);
            operand_start = i + c.len_utf8();
        }

        if !c.is_whitespace() {
            after_operator = matches!(c, '+' | '-' | '*' | '/') || is_times(c, prev);
        }
        prev = Some(c);
    }

    normalize_operand(&text[operand_start..], &mut out);
    out.into()
}

fn normalize_operand(operand: &str, out: &mut String) {
    if operand.contains(',') {
        if thousands(operand) {
            out.push_str(operand);
        } else {
            for c in operand.chars() {
                out.push(if c == ',' { '.' } else { c });
            }
        }
    } else {
        out.push_str(operand);
    }
}

fn thousands(text: &str) -> bool {
    let number = text.trim();
    let number = number.strip_prefix(['+', '-']).unwrap_or(number);
    let Some((whole, rest)) = number.split_once(',') else { return false };
    let (part, unit) = rest.split_at(rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len()));
    (1..=3).contains(&whole.len())
        && whole.bytes().all(|b| b.is_ascii_digit())
        && !whole.starts_with('0')
        && part.len() == 3
        && unit.trim().chars().all(|c| c.is_alphabetic() || c == '"' || c == '\'')
}

/// Whether length `pt` (points) is finite in every East Asian measurement unit.
pub fn finite_in_every_unit(pt: f64) -> bool {
    pt.is_finite() && MeasureUnit::EAST_ASIAN.iter().all(|u| u.from_pt(pt).is_finite())
}

/// Parse unitless expressions (percentages, degrees, counts) with `+ - * /` arithmetic.
pub fn parse_number(s: &str) -> Option<f64> {
    if !s.chars().all(|c| c.is_ascii_digit() || " .+-*/xX×,".contains(c)) {
        return None;
    }
    let s = s.trim();
    MeasureUnit::Points.parse(s.strip_prefix('+').unwrap_or(s))
}

/// Evaluate arithmetic expressions with unit conversions.
pub fn parse_measure(s: &str, default: MeasureUnit) -> Option<f64> {
    if arithmetic_signs(s) > MAX_MEASURE_OPERATORS {
        return None;
    }
    parse_measure_recursive(s, default)
}

fn parse_measure_recursive(s: &str, default: MeasureUnit) -> Option<f64> {
    let s = s.trim();
    for ops in [&['+', '-'][..], &['*', '/'][..]] {
        let skip = s.chars().next().map_or(0, char::len_utf8);
        let is_sign = |i: usize| {
            s.get(i..).and_then(|r| r.chars().next()).is_some_and(|c| matches!(c, '+' | '-'))
                && s.get(..i).and_then(|l| l.trim_end().chars().last()).is_some_and(|c| "+-*/".contains(c))
        };
        let found = s.char_indices().rev().find(|&(i, c)| i >= skip && ops.contains(&c) && !is_sign(i));
        if let Some(i) = found.map(|(i, _)| i) {
            let (l, r) = (&s[..i], &s[i + 1..]);
            if l.trim().is_empty() {
                continue;
            }
            let Some(op) = s[i..].chars().next() else { continue };
            return match op {
                '+' | '-' => {
                    let a = parse_measure_recursive(l, default)?;
                    let b = match percent(r) {
                        Some(k) => a * k,
                        None => parse_measure_recursive(r, default)?,
                    };
                    Some(if op == '+' { a + b } else { a - b })
                }
                _ => {
                    let (k, unit) = match percent(r) {
                        Some(k) => (k, None),
                        None => number_unit(r)?,
                    };
                    let a = parse_measure_recursive(l, unit.unwrap_or(default))?;
                    if op == '*' {
                        Some(a * k)
                    } else if k == 0.0 {
                        None
                    } else {
                        Some(a / k)
                    }
                }
            };
        }
    }

    let plain = |t: &str| t.trim().chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-') && t.trim().parse::<f64>().is_ok();
    if let Some((p, pt)) = s.split_once('p')
        && !p.is_empty()
        && plain(p)
        && (pt.is_empty() || plain(pt))
        && !s.ends_with("pt")
        && !s.ends_with("px")
    {
        let sign = if p.trim_start().starts_with('-') { -1.0 } else { 1.0 };
        let picas = p.trim().parse::<f64>().ok()?.abs();
        return Some(sign * (picas * 12.0 + pt.trim().parse::<f64>().unwrap_or(0.0)));
    }
    let (v, unit) = number_unit(s)?;
    Some(unit.unwrap_or(default).to_pt(v))
}

fn number_unit(s: &str) -> Option<(f64, Option<MeasureUnit>)> {
    let s = s.trim();
    let sign = usize::from(s.starts_with('+'));
    let num_end = s.get(sign..)?.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).map_or(s.len(), |i| i + sign);
    let v: f64 = s.get(..num_end)?.trim().parse().ok()?;
    let unit = match s.get(num_end..)?.trim() {
        "" => None,
        "pt" => Some(MeasureUnit::Points),
        "px" => Some(MeasureUnit::Pixels),
        "in" | "\"" => Some(MeasureUnit::Inches),
        "mm" => Some(MeasureUnit::Millimeters),
        "cm" => Some(MeasureUnit::Centimeters),
        "m" => Some(MeasureUnit::Meters),
        "ft" | "'" => Some(MeasureUnit::Feet),
        "yd" => Some(MeasureUnit::Yards),
        "p" | "pc" => Some(MeasureUnit::Picas),
        "q" | "Q" => Some(MeasureUnit::Q),
        "h" | "H" | "ha" | "Ha" => Some(MeasureUnit::Ha),
        _ => return None,
    };
    Some((v, unit))
}

fn percent(s: &str) -> Option<f64> {
    match number_unit(s.trim().strip_suffix('%')?)? {
        (p, None) => Some(p / 100.0),
        _ => None,
    }
}

/// Scrubbing drag lifecycle phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrubPhase {
    Idle,
    Dragging,
    /// Released on this frame.
    Released,
    /// Cancelled via Escape on this frame.
    Cancelled,
}

#[derive(Clone, Copy, Debug)]
struct ScrubState {
    zone: Id,
    field: Id,
    start: f64,
    pending: f64,
    step: f64,
    moved: bool,
    cancelled: bool,
}

#[derive(Clone, Copy, Debug)]
struct RegisteredLabel {
    layer: egui::LayerId,
    rect: Rect,
    pass: u64,
    link: Option<Id>,
    tab_stop: bool,
}

/// Parameters for scrubbing a numeric field.
#[derive(Clone, Copy, Debug)]
pub struct FieldScrubParams<'a> {
    pub id: Id,
    pub rect: Rect,
    pub text: &'a str,
    pub value: Option<f64>,
    pub decimals: i32,
    pub bounds: &'a RangeInclusive<f64>,
}

impl<'a> FieldScrubParams<'a> {
    pub fn new(id: Id, rect: Rect, text: &'a str, value: Option<f64>, decimals: i32, bounds: &'a RangeInclusive<f64>) -> Self {
        Self { id, rect, text, value, decimals, bounds }
    }
}

/// Label scrubbing drag state controller.
pub struct Scrubber;

impl Scrubber {
    fn state_id() -> Id {
        Id::unique("craft-ui-numeric-scrub")
    }

    fn label_id() -> Id {
        Id::unique("craft-ui-numeric-label")
    }

    fn enabled_id() -> Id {
        Id::unique("craft-ui-numeric-scrub-enabled")
    }

    /// Turn scrubbing on or off globally in context data.
    pub fn set_enabled(ctx: &Context, on: bool) {
        ctx.data_mut(|d| d.insert_temp(Self::enabled_id(), on));
    }

    /// Whether scrubbing is currently enabled in the context.
    pub fn is_enabled(ctx: &Context) -> bool {
        ctx.data(|d| d.get_temp(Self::enabled_id())).unwrap_or(true)
    }

    /// Get current scrub phase.
    pub fn phase(ctx: &Context) -> ScrubPhase {
        let Some(s) = ctx.data(|d| d.get_temp::<ScrubState>(Self::state_id())) else {
            return ScrubPhase::Idle;
        };
        if ctx.drag_stopped_id() == Some(s.zone) {
            if s.cancelled { ScrubPhase::Cancelled } else { ScrubPhase::Released }
        } else if ctx.is_being_dragged(s.zone) {
            ScrubPhase::Dragging
        } else {
            ScrubPhase::Idle
        }
    }

    /// Note a label drawn at `rect` for the numeric field immediately following it.
    pub fn note_label(ui: &Ui, rect: Rect) {
        Self::note(ui, RegisteredLabel { layer: ui.layer_id(), rect, pass: 0, link: None, tab_stop: false });
    }

    /// Draw a link label that scrubs the subsequent field when dragged, while retaining click dispatch.
    pub fn link_label(ui: &mut Ui, text: egui::RichText) -> Response {
        let link = ui
            .scope(|ui| {
                ui.style_mut().interaction.selectable_labels = false;
                ui.link(text)
            })
            .inner;
        Self::note(ui, RegisteredLabel { layer: ui.layer_id(), rect: link.rect, pass: 0, link: Some(link.id), tab_stop: true });
        link
    }

    /// Draw a panel-opening label that scrubs when dragged and retains click dispatch.
    pub fn panel_label(ui: &mut Ui, text: egui::RichText) -> Response {
        let label = ui.add(egui::Label::new(text).selectable(false).sense(Sense::CLICK));
        if label.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        Self::note(ui, RegisteredLabel { layer: ui.layer_id(), rect: label.rect, pass: 0, link: Some(label.id), tab_stop: false });
        label
    }

    fn note(ui: &Ui, label: RegisteredLabel) {
        let pass = ui.ctx().cumulative_pass_nr();
        let layer = ui.layer_id();
        ui.data_mut(|d| d.insert_temp(Self::label_id(), RegisteredLabel { pass, layer, ..label }));
    }

    fn take_label(ui: &Ui, field_rect: Rect) -> Option<RegisteredLabel> {
        let l = ui.data_mut(|d| {
            let l = d.get_temp::<RegisteredLabel>(Self::label_id());
            d.remove::<RegisteredLabel>(Self::label_id());
            l
        })?;
        let gap = field_rect.left() - l.rect.right();
        (l.layer == ui.layer_id()
            && l.pass == ui.ctx().cumulative_pass_nr()
            && (-1.0..=LABEL_GAP).contains(&gap)
            && field_rect.y_range().contains(l.rect.center().y))
        .then_some(l)
    }

    /// Scrub a numeric field, stepping its value based on horizontal drag on its associated label.
    ///
    /// Preserves app-specific command dispatch by returning the new stepped value (`Option<f64>`)
    /// and calling `focus_field` if clicked.
    pub fn field<F>(ui: &Ui, params: FieldScrubParams<'_>, focus_field: F) -> Option<f64>
    where
        F: FnOnce(&Ui, Id, &str),
    {
        let label = Self::take_label(ui, params.rect)?;
        let value = params.value.filter(|v| v.is_finite())?;
        if !Self::is_enabled(ui.ctx()) || !ui.is_enabled() {
            return None;
        }

        let resp = match label.link {
            Some(link) if label.tab_stop => ui.interact(label.rect, link, Sense::drag()),
            Some(link) => ui.interact(label.rect, link, Sense::DRAG),
            None => ui.interact(label.rect, params.id.with("craft-ui-scrub-label"), Sense::CLICK | Sense::DRAG),
        };

        Self::drag(ui, &resp, params, value, label.link.is_some(), focus_field).map(|v| v.max(*params.bounds.start()).min(*params.bounds.end()))
    }

    fn drag<F>(ui: &Ui, resp: &Response, params: FieldScrubParams<'_>, value: f64, is_link: bool, focus_field: F) -> Option<f64>
    where
        F: FnOnce(&Ui, Id, &str),
    {
        if (resp.hovered() && !is_link) || resp.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        if resp.clicked() {
            focus_field(ui, params.id, params.text);
            return None;
        }
        if resp.drag_started() {
            ui.memory_mut(|m| m.stop_text_input());
            let s = ScrubState { zone: resp.id, field: params.id, start: value, pending: 0.0, step: 0.0, moved: false, cancelled: false };
            ui.data_mut(|d| d.insert_temp(Self::state_id(), s));
        }
        if !(resp.dragged() || resp.drag_stopped()) {
            return None;
        }
        let mut s = ui.data(|d| d.get_temp::<ScrubState>(Self::state_id())).filter(|s| s.zone == resp.id && s.field == params.id && !s.cancelled)?;
        let escape = ui.input_mut(|i| {
            if let Some(pos) = i.events.iter().position(|e| matches!(e, egui::Event::Key { key: Key::Escape, pressed: true, .. })) {
                if let egui::Event::Key { modifiers, .. } = i.events[pos] { i.consume_key(modifiers, Key::Escape) } else { false }
            } else {
                false
            }
        });
        let out = if escape {
            s.cancelled = true;
            (s.field == params.id && s.moved).then_some(s.start)
        } else if resp.drag_stopped() {
            if !s.moved && !is_link {
                focus_field(ui, params.id, params.text);
            }
            None
        } else {
            Self::step(ui, resp, &mut s, value, params.decimals)
        };
        ui.data_mut(|d| d.insert_temp(Self::state_id(), s));
        out
    }

    fn step(ui: &Ui, resp: &Response, s: &mut ScrubState, value: f64, decimals: i32) -> Option<f64> {
        let mods = ui.input(|i| i.modifiers);
        let size = if mods.command {
            0.1
        } else if mods.shift {
            10.0
        } else {
            1.0
        };
        let step = f64::max(size, 10f64.powi(-decimals));
        if (step - s.step).abs() > 1e-9 {
            s.step = step;
            s.pending = 0.0;
        }
        s.pending += f64::from(resp.drag_delta().x / PX_PER_STEP) * size;
        let n = (s.pending / step + s.pending.signum() * 1e-9).trunc();
        // The drag's first frame only starts it: host apps open their undo group on the next frame.
        if n == 0.0 || resp.drag_started() {
            return None;
        }
        s.pending -= n * step;
        s.moved = true;
        Some(round_to(value + n * step, decimals))
    }
}

/// Round `val` to `decimals` decimal places.
pub fn round_to(val: f64, decimals: i32) -> f64 {
    let factor = 10f64.powi(decimals);
    (val * factor).round() / factor
}
