//! `GradientBar`: the stop bar of the gradient editor, a leaf with a part per marker. The editor shows it
//! above the fields row.

use std::marker::PhantomData;

use craft_behavior::part::PartRef;
use craft_behavior::{
    Axis, CursorIcon, Granularity, HitLevel, HitPriority, Intent, Metrics, NavShape, PartKey, PointerButton, PressPhase, Roving, Sense,
};
use craft_keymap::{Chord, Key, Mods};
use craft_theme::{Rgba, Srgba, Stateful, States};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, DropMark, DropOperation, DropTarget, Edit as _, EditResponse, Focus as _, MeasureCx, Name, Outcome,
    PaintUi, PartsCx, Pointer as _, Props as _, Remember as _, Role, Styled as _, WordSetCx as _,
};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::commands::FireSource;
use craft_ui::runtime::env::Units;
use craft_ui::runtime::focus::{ActivateKeys, FocusShape, KeyUse};
use craft_ui::style::{ComponentStyle, DataColor, Flags, FocusLook, StyleColor, StyleCx};
use egui::{Color32, CornerRadius, Pos2, Rect, Response, Vec2, pos2, vec2};

use crate::ui::gradient::ramp::GradientRamp;
use crate::ui::gradient::stops::{self, STOP_STEP, STOP_STEP_FAR, Sampler, StopRules, default_sampler, location, locations, midpoint, midpoint_at};
use crate::ui::gradient::value::{GradientStops, StopRef, StopRow, StopRows};
use crate::ui::gradient::words::GradientWords;
use craft_ui::runtime::env::UnitKind;

/// The part under the whole bar: a click on it adds a stop.
pub(super) const RAMP: PartKey = PartKey::new("ramp");
/// Colour stop `i`: `COLOR_STOP.at(i)`.
pub(super) const COLOR_STOP: PartKey = PartKey::new("color_stop");
/// Opacity stop `i`.
pub(super) const OPACITY_STOP: PartKey = PartKey::new("opacity_stop");
/// The midpoint after colour stop `i`.
pub(super) const COLOR_MIDPOINT: PartKey = PartKey::new("color_midpoint");
/// The midpoint after opacity stop `i`.
pub(super) const OPACITY_MIDPOINT: PartKey = PartKey::new("opacity_midpoint");

/// Stop locations as markers show them, closer than this, have no midpoint diamond between them.
const MIN_GAP: f32 = 0.001;
/// The tear-off distance as a multiple of the control height.
const TEAR_OFF_SHARE: f32 = 1.17;

/// The keys a focused bar keeps from the app's shortcuts. `+` is claimed both as itself and as `=`: many keyboards
/// type it with Shift.
const CLAIMS: [Chord; 5] =
    [Chord::plain(Key::Plus), Chord::plain(Key::Equals), Chord::plain(Key::Minus), Chord::plain(Key::Insert), Chord::new(Mods::CMD, Key::Enter)];

/// The part of a marker.
fn part_of(marker: StopRef) -> PartRef {
    match marker {
        StopRef::Color(i) => COLOR_STOP.at(i as u64),
        StopRef::Opacity(i) => OPACITY_STOP.at(i as u64),
        StopRef::Midpoint(StopRow::Color, i) => COLOR_MIDPOINT.at(i as u64),
        StopRef::Midpoint(StopRow::Opacity, i) => OPACITY_MIDPOINT.at(i as u64),
    }
}

/// The marker a part stands for.
fn marker_of(part: PartRef) -> Option<StopRef> {
    let i = usize::try_from(part.index?).ok()?;
    match part.key {
        k if k == COLOR_STOP => Some(StopRef::Color(i)),
        k if k == OPACITY_STOP => Some(StopRef::Opacity(i)),
        k if k == COLOR_MIDPOINT => Some(StopRef::Midpoint(StopRow::Color, i)),
        k if k == OPACITY_MIDPOINT => Some(StopRef::Midpoint(StopRow::Opacity, i)),
        _ => None,
    }
}

/// Where a payload dropped on the bar landed.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum StopDrop {
    /// On colour stop `i` (DesignCraft and VectorCraft recolour it).
    OnStop(usize),
    /// At a location of the bar, 0..=1, away from the colour stops (they add a stop there).
    At(f32),
}

/// The style of the stop bar.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct GradientBarStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The width a bar asks for when the layout offers none.
    length: f32,
    /// Height of the ramp.
    ramp_height: f32,
    /// Inset of the ramp at both ends of the bar.
    inset: f32,
    /// Side of a stop marker's square.
    marker: f32,
    /// Height of a stop marker's pointer.
    pointer: f32,
    /// Radius of a midpoint diamond.
    diamond: f32,
    /// How far from a marker and a diamond a press takes it.
    hit: f32,
    /// How far above or below the ramp a dragged stop is removed.
    tear_off: f32,
    /// Marker outlines, pointers and diamonds.
    edge: Stateful<StyleColor>,
    /// The outline of a hovered marker.
    hovered: StyleColor,
    /// The outline of the selected marker and the fill of the selected diamond.
    selected: StyleColor,
    /// The one-point edge round the ramp.
    ramp_edge: Stateful<StyleColor>,
    /// The line that marks a drop place.
    drop: StyleColor,
}

/// The selection and the drag the bar remembers.
#[derive(Clone, Debug, Default)]
struct Held {
    drag: Option<Drag>,
    /// The selection before the press, restored when the press is cancelled.
    before: Option<StopRef>,
}

/// The marker a press captured.
#[derive(Clone, Debug)]
struct Drag {
    marker: StopRef,
    /// The stops at the press: every frame of the drag edits them afresh, so a stop torn off comes back as it was.
    base: GradientStops,
    /// Where the press was and the marker's location then, so the marker doesn't jump.
    press: Pos2,
    grab: f32,
    /// Alt was held at the press: the drag moves a copy of the stop.
    copy: bool,
}

/// What the bar reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct GradientBarOutput<P = ()> {
    /// This pass's edit of the stops.
    pub edit: EditResponse<GradientStops>,
    /// The selection after this pass, when the bar changed it.
    pub select: Option<Option<StopRef>>,
    /// The colour stop whose colour is asked for: a double click on it, Enter or Space.
    pub color_requested: Option<usize>,
    /// Enter or Space on an opacity stop or a midpoint: its first field takes the focus.
    pub field_requested: bool,
    /// A payload dropped on the bar: where it landed.
    pub dropped: Option<(StopDrop, P)>,
}

impl<P> Outcome for GradientBarOutput<P> {
    type Value = GradientStops;

    fn response(&self) -> &Response {
        &self.edit.response
    }
}

/// The stop bar: opacity markers, ramp and colour markers, edited by pointer and keys.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(content_painter, unregistered)]
pub struct GradientBar<'a, P: Clone + Send + Sync + 'static = ()> {
    #[common]
    common: Common,
    value: Bind<'a, GradientStops>,
    /// The rows, the stop count and the midpoint range.
    #[prop]
    rules: StopRules,
    /// Whether the midpoint diamonds show and edit, on both rows.
    #[prop]
    midpoints: bool,
    /// Whether the opacity row of a split bar has midpoint diamonds.
    #[prop]
    opacity_midpoints: bool,
    /// Samples the gradient for the ramp and new stops instead of [`default_sampler`].
    #[prop]
    sampler: Option<&'a Sampler>,
    /// Decimals of a location in percent, for the accessible values.
    #[prop]
    location_decimals: usize,
    /// Registers the bar as a drop target for `P` payloads.
    #[prop(flag)]
    drop_target: bool,
    /// The selected marker.
    #[prop]
    selected_marker: Option<StopRef>,
    select: Option<Option<StopRef>>,
    color_requested: Option<usize>,
    field_requested: bool,
    dropped: Option<(StopDrop, P)>,
    payload: PhantomData<fn() -> P>,
}

impl<'a, P: Clone + Send + Sync + 'static> GradientBar<'a, P> {
    /// A bar named `label` over `value`.
    pub fn new(label: impl Into<craft_ui::runtime::env::Label>, value: Bind<'a, GradientStops>) -> Self {
        GradientBar {
            common: Common::new(),
            value,
            rules: StopRules::default(),
            midpoints: true,
            opacity_midpoints: true,
            sampler: None,
            location_decimals: 0,
            drop_target: false,
            selected_marker: None,
            select: None,
            color_requested: None,
            field_requested: false,
            dropped: None,
            payload: PhantomData,
        }
        .label(label)
        .ltr_axis()
    }

    fn rows(&self) -> StopRows {
        self.rules.rows
    }
}

/// Where the bar's parts are this pass.
#[derive(Clone, Copy, Debug)]
struct Geometry {
    ramp: Rect,
    rows: StopRows,
    midpoints: bool,
    opacity_midpoints: bool,
    marker: f32,
    pointer: f32,
    diamond: f32,
    hit: f32,
    tear_off: f32,
}

impl Geometry {
    fn new(rect: Rect, rows: StopRows, midpoints: bool, opacity_midpoints: bool, style: &GradientBarStyle) -> Self {
        let top = Self::top(rows, midpoints, style);
        let ramp = Rect::from_min_size(
            pos2(rect.left() + style.inset, rect.top() + top),
            vec2((rect.width() - 2.0 * style.inset).max(1.0), style.ramp_height),
        );
        Geometry {
            ramp,

            rows,
            midpoints,
            opacity_midpoints,
            marker: style.marker,
            pointer: style.pointer,
            diamond: style.diamond,
            hit: style.hit,
            tear_off: style.tear_off,
        }
    }

    /// The height above the ramp.
    fn top(rows: StopRows, midpoints: bool, style: &GradientBarStyle) -> f32 {
        match rows {
            StopRows::Split => style.pointer + style.marker + 1.0,
            _ if midpoints => 3.0 * style.diamond + 1.0,
            _ => 1.0,
        }
    }

    /// The height of the bar.
    fn height(rows: StopRows, midpoints: bool, style: &GradientBarStyle) -> f32 {
        Self::top(rows, midpoints, style) + style.ramp_height + style.pointer + style.marker + 1.0
    }

    /// The rows the bar shows, top first.
    fn rows(&self) -> Vec<StopRow> {
        match self.rows {
            StopRows::Split => vec![StopRow::Opacity, StopRow::Color],
            _ => vec![StopRow::Color],
        }
    }

    fn x(&self, location: f32) -> f32 {
        self.ramp.left() + location.clamp(0.0, 1.0) * self.ramp.width()
    }

    /// The location under `x`, unclamped.
    fn location(&self, x: f32) -> f32 {
        (x - self.ramp.left()) / self.ramp.width().max(1.0)
    }

    /// Whether `m` is a marker of `stops` on this bar.
    fn valid(&self, stops: &GradientStops, m: StopRef) -> bool {
        let has = |row| row == StopRow::Color || self.rows == StopRows::Split;
        match m {
            StopRef::Midpoint(row, i) => self.shows_midpoints(row) && has(row) && i + 1 < stops.len(row),
            m => has(m.row()) && m.index() < stops.len(m.row()),
        }
    }

    fn shows_midpoints(&self, row: StopRow) -> bool {
        self.midpoints && (row == StopRow::Color || self.opacity_midpoints)
    }

    /// The midpoints of `row` the bar shows, with their locations: between stops more than [`MIN_GAP`] apart.
    fn diamonds(&self, stops: &GradientStops, row: StopRow) -> Vec<(usize, f32)> {
        if !self.shows_midpoints(row) {
            return Vec::new();
        }
        let at = locations(stops, row);
        (0..at.len().saturating_sub(1)).filter(|&i| at[i + 1] - at[i] > MIN_GAP).map(|i| (i, midpoint_at(stops, row, i))).collect()
    }

    /// The markers of `row` in location order, each midpoint between its stops.
    fn order(&self, stops: &GradientStops, row: StopRow) -> Vec<StopRef> {
        let diamonds = self.diamonds(stops, row);
        let mut out = Vec::new();
        for i in 0..stops.len(row) {
            out.push(StopRef::stop(row, i));
            if diamonds.iter().any(|(k, _)| *k == i) {
                out.push(StopRef::Midpoint(row, i));
            }
        }
        out
    }

    /// Every marker the bar shows, top row first.
    fn markers(&self, stops: &GradientStops) -> Vec<StopRef> {
        self.rows().into_iter().flat_map(|row| self.order(stops, row)).collect()
    }

    /// Where marker `m` is, 0..=1.
    fn location_of(&self, stops: &GradientStops, m: StopRef) -> f32 {
        match m {
            StopRef::Midpoint(row, i) => midpoint_at(stops, row, i),
            m => location(stops, m.row(), m.index()),
        }
    }

    fn diamond_center(&self, row: StopRow, at: f32) -> Pos2 {
        let d = 2.0 * self.diamond;
        let y = match (row, self.rows) {
            (StopRow::Color, StopRows::Split) => self.ramp.bottom() + d,
            _ => self.ramp.top() - d,
        };
        pos2(self.x(at), y)
    }

    /// The square of a stop marker of `row` at `at`. Colour markers hang below the ramp, opacity markers stand above it.
    fn body(&self, row: StopRow, at: f32) -> Rect {
        let top = match row {
            StopRow::Color => self.ramp.bottom() + self.pointer,
            StopRow::Opacity => self.ramp.top() - self.pointer - self.marker,
        };
        Rect::from_min_size(pos2(self.x(at) - self.marker / 2.0, top), vec2(self.marker, self.marker))
    }

    /// What a press takes for marker `m`: a stop within `hit` across, from a diamond's radius away from the ramp's edge
    /// to the marker's far edge; a midpoint within `hit` of its diamond.
    fn hit_rect(&self, stops: &GradientStops, m: StopRef) -> Rect {
        match m {
            StopRef::Midpoint(row, i) => {
                Rect::from_center_size(self.diamond_center(row, midpoint_at(stops, row, i)), vec2(self.hit, 2.0 * self.diamond))
            }
            m => {
                let (row, x) = (m.row(), self.x(location(stops, m.row(), m.index())));
                let (near, far) = match row {
                    StopRow::Color => (self.ramp.bottom() + self.diamond, self.body(row, 0.0).bottom()),
                    StopRow::Opacity => (self.body(row, 0.0).top(), self.ramp.top() - self.diamond),
                };
                Rect::from_x_y_ranges((x - self.hit / 2.0)..=(x + self.hit / 2.0), near..=far)
            }
        }
    }

    /// The colour stop or location a payload dropped at `pos` lands on.
    fn drop_place(&self, stops: &GradientStops, pos: Pos2) -> StopDrop {
        let near = stops
            .color
            .iter()
            .enumerate()
            .map(|(i, s)| (i, (self.x(s.location) - pos.x).abs()))
            .filter(|(_, d)| *d <= self.hit)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        match near {
            Some((i, _)) => StopDrop::OnStop(i),
            None => StopDrop::At(stops::round_location(self.location(pos.x))),
        }
    }
}

/// The accessible name and value of marker `m` of `stops`, as the bar and the canvas gradient line's parts read them.
pub(crate) fn marker_texts(
    words: &GradientWords,
    units: &Units,
    rows: StopRows,
    stops: &GradientStops,
    m: StopRef,
    location_decimals: usize,
) -> (String, String) {
    let percent = |v: f32, decimals: usize| units.text(UnitKind::Percent, f64::from(v) * 100.0, decimals..=decimals);
    let location_text = |at: f32| percent(at, location_decimals);
    let count = locations(stops, m.row()).len();
    let (n, number) = (count.to_string(), |i: usize| (i + 1).to_string());
    match m {
        StopRef::Color(i) => {
            let stop = stops.color[i];
            let hex = Rgba::from(Srgba { a: 1.0, ..stop.color }).to_hex().to_uppercase();
            let (at, opacity) = (location_text(stop.location), percent(stop.color.a, 0));
            let value = match rows {
                StopRows::Combined => words.color_opacity_value.fill(&[("location", &at), ("hex", &hex), ("opacity", &opacity)]),
                _ => words.color_value.fill(&[("location", &at), ("hex", &hex)]),
            };
            (words.color_stop.fill(&[("i", &number(i)), ("n", &n)]), value)
        }
        StopRef::Opacity(i) => {
            let stop = stops.opacity_stops()[i];
            let value = words.opacity_value.fill(&[("location", &location_text(stop.location)), ("opacity", &percent(stop.opacity, 0))]);
            (words.opacity_stop.fill(&[("i", &number(i)), ("n", &n)]), value)
        }
        StopRef::Midpoint(row, i) => {
            let template = match row {
                StopRow::Color => &words.midpoint,
                StopRow::Opacity => &words.opacity_midpoint,
            };
            (template.fill(&[("i", &number(i))]), percent(midpoint(stops, row, i), 0))
        }
    }
}

/// A diamond of radius `r` round `center`.
fn diamond(center: Pos2, r: f32) -> PaintShape {
    PaintShape::Polygon(vec![center + vec2(0.0, -r), center + vec2(r, 0.0), center + vec2(0.0, r), center + vec2(-r, 0.0)])
}

/// Where a payload dropped on the bar lands: the bar's stops decide.
struct OnBar<'s, P> {
    geo: Geometry,
    stops: &'s GradientStops,
    live: bool,
    payload: PhantomData<fn() -> P>,
}

impl<P: Clone + Send + Sync + 'static> DropTarget<P> for OnBar<'_, P> {
    type Drop = (StopDrop, P);

    fn accept(&self, payload: &P, at: Pos2, _: DropOperation) -> Option<Self::Drop> {
        self.live.then(|| (self.geo.drop_place(self.stops, at), payload.clone()))
    }

    fn mark(&self, drop: &Self::Drop) -> DropMark {
        let x = match drop.0 {
            StopDrop::OnStop(i) => self.geo.x(self.stops.color[i].location),
            StopDrop::At(at) => self.geo.x(at),
        };
        DropMark::line(pos2(x, self.geo.ramp.top()), pos2(x, self.geo.ramp.bottom()))
    }
}

impl<P: Clone + Send + Sync + 'static> GradientBar<'_, P> {
    fn geometry(&self, rect: Rect, style: &GradientBarStyle) -> Geometry {
        Geometry::new(rect, self.rows(), self.midpoints, self.opacity_midpoints, style)
    }

    /// Selects `marker`, as the bar's report to the editor.
    fn choose(&mut self, marker: Option<StopRef>) {
        self.selected_marker = marker;
        self.select = Some(marker);
    }

    /// The drag's stops with the marker at the pointer: moved, copied, or torn off.
    fn drag_to(&mut self, cx: &mut Cx<'_, Self>, geo: &Geometry, drag: &Drag, pos: Pos2) {
        let Drag { marker, ref base, press, grab, copy, .. } = *drag;
        let to = grab + geo.location(pos.x) - geo.location(press.x);
        let moved = pos.x != press.x;
        let off = pos.y > geo.ramp.bottom() + geo.tear_off || pos.y < geo.ramp.top() - geo.tear_off;
        let rules = &self.rules;
        let (stops, selected) = match marker {
            StopRef::Midpoint(row, i) => {
                let at = locations(base, row);
                let m = (to - at[i]) / (at[i + 1] - at[i]).max(MIN_GAP);
                match moved {
                    true => stops::set_midpoint(base, row, i, m, rules).map_or((base.clone(), Some(marker)), |(s, _)| (s, Some(marker))),
                    false => (base.clone(), Some(marker)),
                }
            }
            m => {
                let (row, i, at) = (m.row(), m.index(), stops::round_location(to));
                // At the row's minimum a stop dragged off the bar stays and moves.
                let torn = (off && !copy).then(|| stops::remove(base, row, i, rules)).flatten().map(|(s, _)| (s, None));
                let edited = torn.or_else(|| match (moved, copy) {
                    (false, _) => None,
                    (true, true) => stops::copy(base, row, i, at, rules).map(|(s, j)| (s, Some(StopRef::stop(row, j)))),
                    (true, false) => stops::move_to(base, row, i, at, rules).map(|(s, j)| (s, Some(StopRef::stop(row, j)))),
                });
                edited.unwrap_or_else(|| (base.clone(), Some(marker)))
            }
        };
        self.choose(selected);
        cx.propose(stops);
    }

    /// The press, drag and release of the marker `marker`.
    fn pointer(&mut self, cx: &mut Cx<'_, Self>, geo: &Geometry, stops: &GradientStops, marker: StopRef) {
        let part = part_of(marker);
        let press = cx.part(part).press;
        match press.phase {
            PressPhase::Down { .. } => {
                cx.memory::<Held>().before = self.selected_marker;
                self.choose(Some(marker));
                cx.focus_part(part);
                if press.double_pressed()
                    && let StopRef::Color(i) = marker
                {
                    self.color_requested = Some(i);
                }
            }
            PressPhase::DragStart { start, pos, .. } if start.button == PointerButton::Primary => {
                let drag = Drag {
                    marker,
                    base: stops.clone(),
                    press: start.origin,
                    grab: geo.location_of(stops, marker),
                    copy: press.mods.alt && marker.is_stop(),
                };
                self.choose(Some(marker));
                cx.begin();
                self.drag_to(cx, geo, &drag, pos);
                cx.memory::<Held>().drag = Some(drag);
            }
            PressPhase::Drag { pos, .. } => {
                if let Some(drag) = cx.memory::<Held>().drag.clone() {
                    self.drag_to(cx, geo, &drag, pos);
                }
            }
            PressPhase::Release { pos, start, .. } => match cx.memory::<Held>().drag.take() {
                Some(drag) => {
                    self.drag_to(cx, geo, &drag, pos);
                    cx.commit();
                }
                None if start.button == PointerButton::Secondary => self.choose(Some(marker)),
                None => {}
            },
            PressPhase::Cancel { .. } => {
                let held = cx.memory::<Held>();
                held.drag = None;
                let before = held.before;
                if self.selected_marker != before {
                    self.choose(before);
                }
            }
            _ => {}
        }
    }

    /// A stop added where a click on empty space was: above the ramp's centre in the opacity row of a split bar, else in
    /// the colour row.
    fn add_at(&mut self, cx: &mut Cx<'_, Self>, geo: &Geometry, stops: &GradientStops, pos: Pos2) {
        let row = match geo.rows {
            StopRows::Split if pos.y < geo.ramp.center().y => StopRow::Opacity,
            _ => StopRow::Color,
        };
        let default = default_sampler(self.rows());
        let sampler: &Sampler = self.sampler.unwrap_or(&default);
        if let Some((added, j)) = stops::add(stops, row, stops::round_location(geo.location(pos.x)), sampler, &self.rules) {
            self.choose(Some(StopRef::stop(row, j)));
            cx.set(added);
        }
    }

    /// Selects the next or previous marker of the selected row; the first or last with nothing selected.
    fn step(&mut self, cx: &mut Cx<'_, Self>, geo: &Geometry, stops: &GradientStops, forward: bool) {
        let row = self.selected_marker.map_or(StopRow::Color, StopRef::row);
        let order = geo.order(stops, row);
        let Some(last) = order.len().checked_sub(1) else { return };
        let k = match self.selected_marker.and_then(|s| order.iter().position(|m| *m == s)) {
            None if forward => 0,
            None => last,
            Some(k) if forward => (k + 1).min(last),
            Some(k) => k.saturating_sub(1),
        };
        self.choose(Some(order[k]));
        cx.focus_part(part_of(order[k]));
    }

    /// Selects the stop of `row` nearest the selected marker's location.
    fn switch(&mut self, cx: &mut Cx<'_, Self>, geo: &Geometry, stops: &GradientStops, row: StopRow) {
        if self.selected_marker.is_some_and(|s| s.row() == row) {
            return;
        }
        let at = self.selected_marker.map_or(0.0, |s| geo.location_of(stops, s));
        let nearest = locations(stops, row).into_iter().enumerate().min_by(|a, b| (a.1 - at).abs().total_cmp(&(b.1 - at).abs())).map(|(i, _)| i);
        if let Some(i) = nearest {
            self.choose(Some(StopRef::stop(row, i)));
            cx.focus_part(part_of(StopRef::stop(row, i)));
        }
    }

    /// Selection, moves, deletes and additions while the bar has focus and no drag is under way. Each pass's changes are
    /// one Set.
    fn keys(&mut self, cx: &mut Cx<'_, Self>, geo: &Geometry, shown: &GradientStops) {
        let mut stops = shown.clone();
        while cx.take_claimed(Chord::plain(Key::Plus)) | cx.take_claimed(Chord::plain(Key::Equals)) {
            self.step(cx, geo, &stops, true);
        }
        while cx.take_claimed(Chord::plain(Key::Minus)) {
            self.step(cx, geo, &stops, false);
        }
        while cx.take_claimed(Chord::plain(Key::Insert)) | cx.take_claimed(Chord::new(Mods::CMD, Key::Enter)) {
            let row = self.selected_marker.map_or(StopRow::Color, StopRef::row);
            let after = self.selected_marker.map(StopRef::index).filter(|i| *i < stops.len(row));
            let default = default_sampler(self.rows());
            let sampler: &Sampler = self.sampler.unwrap_or(&default);
            if let Some((added, j)) = stops::add(&stops, row, stops::halfway(&stops, row, after), sampler, &self.rules) {
                stops = added;
                self.choose(Some(StopRef::stop(row, j)));
                cx.focus_part(part_of(StopRef::stop(row, j)));
            }
        }
        // Enter on a focused marker activates that part (Intent::Activate is not delivered while a part is active).
        let activated = self.selected_marker.is_some_and(|m| matches!(cx.part_activated(part_of(m)), Some(FireSource::Key | FireSource::Access)));
        if activated {
            match self.selected_marker {
                Some(StopRef::Color(i)) => self.color_requested = Some(i),
                Some(_) => self.field_requested = true,
                None => {}
            }
        }
        if let Some(m) = self.selected_marker.filter(|m| geo.valid(&stops, *m)) {
            let removed = [true, false].into_iter().any(|forward| cx.intents().take_exact(&Intent::delete(forward, Granularity::Unit)));
            if removed
                && m.is_stop()
                && let Some((out, j)) = stops::remove(&stops, m.row(), m.index(), &self.rules)
            {
                stops = out;
                self.choose(Some(StopRef::stop(m.row(), j)));
                cx.focus_part(part_of(StopRef::stop(m.row(), j)));
            }
        }
        while let Some((dir, mods)) = cx.intents().take_nudge() {
            if mods.alt || mods.ctrl || mods.cmd {
                continue;
            }
            if dir.y != 0.0 && geo.rows == StopRows::Split {
                self.switch(cx, geo, &stops, if dir.y < 0.0 { StopRow::Opacity } else { StopRow::Color });
                continue;
            }
            let by = dir.x.signum() * if mods.shift { STOP_STEP_FAR } else { STOP_STEP };
            let Some(m) = self.selected_marker.filter(|m| geo.valid(&stops, *m)).filter(|_| dir.x != 0.0) else { continue };
            let moved = match m {
                StopRef::Midpoint(row, i) => stops::set_midpoint(&stops, row, i, midpoint(&stops, row, i) + by, &self.rules).map(|(s, _)| (s, m)),
                _ => stops::move_to(&stops, m.row(), m.index(), location(&stops, m.row(), m.index()) + by, &self.rules)
                    .map(|(s, j)| (s, m.with_index(j))),
            };
            if let Some((out, now)) = moved {
                stops = out;
                self.choose(Some(now));
                cx.focus_part(part_of(now));
            }
        }
        if stops != *shown {
            cx.set(stops);
        }
    }
}

impl<P: Clone + Send + Sync + 'static> Component for GradientBar<'_, P> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::CANVAS.with_activate(ActivateKeys::Both).with_deletes().with_chrome_key(false).with_claims(&CLAIMS);
    type Style = GradientBarStyle;
    type Output = GradientBarOutput<P>;

    fn style(cx: &StyleCx<'_>, _: GradientBarStyleKey<P>) -> GradientBarStyle {
        let tokens = cx.tokens;
        let (size, stroke, colors) = (&tokens.size, &tokens.stroke, &tokens.color);
        let (rest, disabled) = (cx.color(colors.text.muted), cx.color(colors.text.disabled));
        let (ramp_rest, ramp_disabled) = (cx.color(colors.line.control_edge), cx.color(colors.text.disabled));
        GradientBarStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            length: size.length.track,
            ramp_height: cx.control().height,
            inset: tokens.space.pad.control,
            marker: size.handle.md,
            pointer: size.handle.sm / 2.0 + stroke.thin,
            diamond: size.handle.sm,
            hit: size.hit.target,
            tear_off: cx.control().height * TEAR_OFF_SHARE,
            edge: Stateful::new(rest).off(States::new(rest, rest, rest, disabled)),
            hovered: cx.color(colors.text.normal),
            selected: cx.color(colors.accent.solid),
            ramp_edge: Stateful::new(ramp_rest).off(States::new(ramp_rest, ramp_rest, ramp_rest, ramp_disabled)),
            drop: cx.color(colors.accent.solid),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let width = cx.room.filter(|room| room.is_finite()).unwrap_or(cx.style.length);
        Vec2::new(width, Geometry::height(self.rows(), self.midpoints, cx.style))
    }

    fn fills(&self) -> Option<Axis> {
        Some(Axis::X)
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn bind(&mut self) -> Option<BindMut<'_, GradientStops>> {
        Some(self.value.reborrow())
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(stops) = cx.value().cloned() else { return };
        let geo = self.geometry(rect, cx.style);
        cx.part(RAMP, rect).sense(Sense::CLICK | Sense::TAKE_FOCUS).pointer_only("Insert adds a stop after the selected marker");
        let markers = geo.markers(&stops);
        for marker in &markers {
            let priority = HitPriority::new(HitLevel::Handle, u8::from(marker.is_stop()));
            let part = cx
                .part(part_of(*marker), geo.hit_rect(&stops, *marker))
                .sense(Sense::DRAG | Sense::CLICK | Sense::CONTEXT_CLICK | Sense::TAKE_FOCUS)
                .priority(priority)
                .cursor(CursorIcon::ResizeHorizontal)
                .focusable()
                .nudges()
                .focus_shape(FocusShape::Rect);
            if self.selected_marker == Some(*marker) {
                part.selected().flags(Flags::SELECTED);
            }
        }
        if !markers.is_empty() {
            cx.roving(Roving::new(NavShape::Flat));
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(stops) = cx.value().cloned() else { return };
        let geo = self.geometry(cx.rect, cx.style);
        let enabled = cx.resolved.enabled;
        let was = self.selected_marker;
        self.selected_marker = self.selected_marker.filter(|m| geo.valid(&stops, *m));
        if enabled {
            let focused = cx.active_part().and_then(marker_of).filter(|m| geo.valid(&stops, *m));
            // Focus arriving at a marker selects it, unless a press holds it (the press selects by its own rule).
            if let Some(part) = cx.focus_arrived_by_key()
                && let Some(m) = marker_of(part).filter(|m| geo.valid(&stops, *m))
            {
                self.choose(Some(m));
            }
            for marker in geo.markers(&stops) {
                self.pointer(cx, &geo, &stops, marker);
            }
            let ramp = cx.part(RAMP).press;
            if let PressPhase::Release { pos, start, click: true, .. } = ramp.phase
                && start.button == PointerButton::Primary
                && cx.memory::<Held>().drag.is_none()
            {
                self.add_at(cx, &geo, &stops, pos);
            }
            if focused.is_some() && cx.memory::<Held>().drag.is_none() && cx.gesture().is_none() {
                self.keys(cx, &geo, &stops);
            }
            if self.drop_target {
                let target = OnBar::<P> { geo, stops: &stops, live: true, payload: PhantomData };
                self.dropped = cx.drop_target::<P, _>(&target);
            }
        }
        if self.selected_marker != was && self.select.is_none() {
            self.select = Some(self.selected_marker);
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let Some(stops) = cx.value() else { return };
        let words = cx.words::<GradientWords>();
        let units = cx.env.units();
        let geo = self.geometry(cx.rect, cx.style);
        let count = |n: usize| (n, n.to_string());
        let value = match self.rows() {
            StopRows::Split => {
                let (colors, opacities) = (count(stops.color.len()), count(stops.opacity_stops().len()));
                words.split_value.fill(&[
                    ("colors", &words.color_stops.text(colors.0 as u64, &colors.1)),
                    ("opacities", &words.opacity_stops.text(opacities.0 as u64, &opacities.1)),
                ])
            }
            _ => {
                let n = count(stops.color.len());
                words.bar_stops.text(n.0 as u64, &n.1)
            }
        };
        access.value(value);
        for marker in geo.markers(stops) {
            let (name, value) = marker_texts(&words, &units, self.rows(), stops, marker, self.location_decimals);
            let node = access.node(part_of(marker), Role::GRAPHICS_OBJECT);
            node.name = Name::Text(name);
            node.value(value);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(stops) = cx.value() else { return };
        let style = cx.style;
        let geo = self.geometry(cx.rect, style);
        let default = default_sampler(self.rows());
        let sampler: &Sampler = self.sampler.unwrap_or(&default);
        GradientRamp::paint(&ui.content_painter(cx), geo.ramp, &GradientRamp::colors(stops, sampler));
        let dragging = cx.recall::<Held>().drag.is_some();
        let draw = ui.draw();
        draw.stroke(&PaintShape::Rect(geo.ramp, CornerRadius::ZERO), StrokeWidth::Thin, cx.resolve(&style.ramp_edge));
        let edge_of = |m: StopRef| {
            if self.selected_marker == Some(m) {
                style.selected
            } else if !dragging && cx.hover_pos(part_of(m)).is_some() {
                style.hovered
            } else {
                cx.resolve(&style.edge)
            }
        };
        let mut markers = geo.markers(stops);
        // The selected marker on top of the others.
        markers.sort_by_key(|m| self.selected_marker == Some(*m));
        for m in markers {
            let edge = edge_of(m);
            match m {
                StopRef::Midpoint(row, i) => {
                    let shape = diamond(geo.diamond_center(row, midpoint_at(stops, row, i)), geo.diamond);
                    if self.selected_marker == Some(m) {
                        draw.fill(&shape, style.selected);
                    }
                    draw.stroke(&shape, StrokeWidth::Thin, edge);
                }
                m => {
                    let (row, at) = (m.row(), location(stops, m.row(), m.index()));
                    let content = match row {
                        StopRow::Color => Color32::from(Srgba { a: 1.0, ..stops.color[m.index()].color }),
                        StopRow::Opacity => Color32::from_gray((stops.opacity_stops()[m.index()].opacity.clamp(0.0, 1.0) * 255.0).round() as u8),
                    };
                    let (body, x, half) = (geo.body(row, at), geo.x(at), geo.marker / 2.0);
                    let (tip, base) = match row {
                        StopRow::Color => (geo.ramp.bottom() + 1.0, body.top()),
                        StopRow::Opacity => (geo.ramp.top() - 1.0, body.bottom()),
                    };
                    draw.fill(&PaintShape::Polygon(vec![pos2(x, tip), pos2(x + half, base), pos2(x - half, base)]), edge);
                    let square = PaintShape::Rect(body, CornerRadius::ZERO);
                    draw.data_fill(&square, DataColor::new(content));
                    draw.stroke(&square, StrokeWidth::Thin, edge);
                }
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> GradientBarOutput<P> {
        GradientBarOutput {
            edit: cx.edit_response(),
            select: self.select,
            color_requested: self.color_requested,
            field_requested: self.field_requested,
            dropped: self.dropped,
        }
    }
}
