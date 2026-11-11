//! The outline bands of grouped rows and columns.

use craft_behavior::{Metrics, NavShape, PartKey, PartRef, Roving, Sense};
use craft_theme::{Stateful, TypeRole};
use craft_ui::contract::{
    Access, Common, Component, Cx, MeasureCx, Name, NoValue, Outcome, PaintUi, PartsCx, Props as _, Role, Styled as _, WordSetCx as _,
};
use craft_ui::primitives::{PaintShape, StrokeWidth, TextRun};
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, FocusLook, StyleColor, StyleCx, TextLook};
use egui::{CornerRadius, Pos2, Rect, Response, Vec2, pos2, vec2};

use super::grid::{CellAt, CellRange, SheetGeometry, column_letters};

#[cfg(feature = "screen-testing")]
mod stories;

craft_ui::words! {
    /// The band buttons' accessible names, English by default (Excel's).
    #[non_exhaustive]
    pub struct OutlineBandWords {
        level: template "outline.level" = "Level {n}";
        collapse_rows: template "outline.collapseRows" = "Hide Detail, rows {first}–{last}";
        expand_rows: template "outline.expandRows" = "Show Detail, rows {first}–{last}";
        collapse_columns: template "outline.collapseColumns" = "Hide Detail, columns {first}–{last}";
        expand_columns: template "outline.expandColumns" = "Show Detail, columns {first}–{last}";
    }
}

/// The part of a level button.
const LEVEL: PartKey = PartKey::new("level");
/// The part of a group's collapse or expand button.
const TOGGLE: PartKey = PartKey::new("toggle");

/// The least sheet zoom a lane's width follows; below it lanes stay at this share of the lane size. A proportion,
/// not a size.
pub const LANE_MIN_ZOOM: f32 = 0.75;

/// A group of lines `start..=end` at `level` (1 = outermost), with its summary line after it
/// (below or right; before it when `summary_before`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct OutlineGroup {
    /// The group's first line.
    pub start: u32,
    /// The group's last line.
    pub end: u32,
    /// The group's nesting level, 1 for the outermost.
    pub level: u8,
    /// Whether the group is collapsed.
    pub collapsed: bool,
    /// Whether the summary line comes before the group instead of after it.
    pub summary_before: bool,
}

/// One direction's outline: its deepest level and its groups.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct OutlineAxis {
    /// The deepest level.
    pub levels: u8,
    /// The groups of this direction.
    pub groups: Vec<OutlineGroup>,
}

/// What a band button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutlineAction {
    /// Show levels 1…`level` (the corner buttons 1…n+1).
    #[non_exhaustive]
    Level {
        /// True for the row outline, false for the column outline.
        rows: bool,
        /// The deepest level to show.
        level: u8,
    },
    /// Collapse or expand a group (its − / + button).
    #[non_exhaustive]
    Toggle {
        /// True for the row outline, false for the column outline.
        rows: bool,
        /// The group to collapse or expand.
        group: OutlineGroup,
    },
}

impl OutlineGroup {
    /// An expanded group of lines `start..=end` at `level` with its summary line after it.
    pub fn new(start: u32, end: u32, level: u8) -> Self {
        OutlineGroup { start, end, level, collapsed: false, summary_before: false }
    }

    /// Sets whether the group is collapsed.
    #[must_use]
    pub fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    /// Puts the summary line before the group.
    #[must_use]
    pub fn summary_before(mut self, before: bool) -> Self {
        self.summary_before = before;
        self
    }
}

impl OutlineAxis {
    /// An outline `levels` deep with `groups`.
    pub fn new(levels: u8, groups: Vec<OutlineGroup>) -> Self {
        OutlineAxis { levels, groups }
    }
}

impl OutlineAction {
    /// The corner button that shows levels 1…`level`.
    pub fn level(rows: bool, level: u8) -> Self {
        OutlineAction::Level { rows, level }
    }

    /// The button that collapses or expands `group`.
    pub fn toggle(rows: bool, group: OutlineGroup) -> Self {
        OutlineAction::Toggle { rows, group }
    }
}

/// A lane's width at `zoom` (sheet zoom, 1.0 = 100 %): `lane × max(zoom, LANE_MIN_ZOOM)`, rounded. `lane` is the
/// lane size the caller reads from its own style.
pub fn lane(lane: f32, zoom: f32) -> f32 {
    (lane * zoom.max(LANE_MIN_ZOOM)).round()
}

/// The band's thickness for an outline `levels` deep: `levels + 1` lanes plus `slack`; 0 without an outline.
fn thickness(levels: u8, zoom: f32, lane_size: f32, slack: f32) -> f32 {
    if levels == 0 {
        return 0.0;
    }
    (f32::from(levels) + 1.0) * lane(lane_size, zoom) + slack
}

/// The bands a grid reserves for outlines `(rows, columns)` levels deep: the row band's width and the column band's
/// height. Without headings the level buttons still need a corner: the band on the other axis is then at least a
/// lane and the slack thick. `lane_size` is the lane size of the caller's style (`size.icon.sm`) and `slack` the room
/// after the last lane (`space.gap.tight`).
pub fn bands((rows, columns): (u8, u8), headings: bool, zoom: f32, lane_size: f32, slack: f32) -> Vec2 {
    let corner = lane(lane_size, zoom) + slack;
    let mut band = Vec2::new(thickness(rows, zoom, lane_size, slack), thickness(columns, zoom, lane_size, slack));
    if !headings {
        if rows > 0 {
            band.y = band.y.max(corner);
        }
        if columns > 0 {
            band.x = band.x.max(corner);
        }
    }
    band
}

/// The style of the outline bands.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct OutlineBandStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The digit on a level button and the − or + on a group button.
    #[style(text)]
    text: TextLook,
    /// One lane at 100 % zoom.
    lane: f32,
    /// What a button's side is short of the lane.
    margin: f32,
    /// A button's least side.
    min_side: f32,
    /// The first lane's inset.
    inset: f32,
    /// How far a button's hit area reaches past its square.
    hit: f32,
    radius: CornerRadius,
    background: StyleColor,
    edge: StyleColor,
    button: StyleColor,
    ink: StyleColor,
}

/// What the bands did this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct OutlineBandOutput {
    /// The bands' response.
    pub response: Response,
    /// The buttons pressed this pass, in order.
    pub actions: Vec<OutlineAction>,
}

impl Outcome for OutlineBandOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// Excel's outline bands: a strip before the row headers and one above the column headers, a lane per level; each
/// expanded group a bracket along its lane from its first detail line to its button, with a tick at the start; − / +
/// on the summary line (before it when `summary_before`); the level buttons 1…n+1 in the corner.
///
/// The bands are placed by the sheet geometry, in logical coordinates mirrored with the sheet, and show over
/// [`OutlineBand::area`]. The buttons are parts in one roving group, level buttons first; a press reports its
/// [`OutlineAction`] in [`OutlineBandOutput::actions`], in order. Lanes follow the sheet zoom down to
/// [`LANE_MIN_ZOOM`]. A button is named by [`OutlineBandWords`] ("Level 2", "Hide Detail, rows 4–9").
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub struct OutlineBand<'a> {
    #[common]
    common: Common,
    rows: &'a OutlineAxis,
    columns: &'a OutlineAxis,
    geometry: &'a SheetGeometry,
    laid: Layout,
    actions: Vec<OutlineAction>,
}

impl<'a> OutlineBand<'a> {
    /// Bands named `label` for the `rows` and `columns` outlines, placed by `geometry`.
    pub fn new(label: impl Into<Label>, rows: &'a OutlineAxis, columns: &'a OutlineAxis, geometry: &'a SheetGeometry) -> Self {
        OutlineBand { common: Common::new(), rows, columns, geometry, laid: Layout::default(), actions: Vec::new() }.label(label)
    }

    /// The screen rect the bands show over: from the corner of the row and column bands to the end of the cells.
    pub fn area(geometry: &SheetGeometry) -> Rect {
        let cells = geometry.cells();
        let min = cells.min - geometry.header_size() - geometry.bands();
        geometry.mirror_rect(Rect::from_min_max(min, cells.max))
    }
}

impl Button {
    fn part(&self) -> PartRef {
        match self.action {
            OutlineAction::Level { rows, level } => LEVEL.at(u64::from(rows) << 8 | u64::from(level)),
            OutlineAction::Toggle { rows, group } => TOGGLE.at(u64::from(rows) << 48 | u64::from(group.start) << 16 | u64::from(group.level)),
        }
    }

    /// The button's face: the level digit, − or +.
    fn glyph(&self) -> String {
        match self.action {
            OutlineAction::Level { level, .. } => level.to_string(),
            OutlineAction::Toggle { group, .. } => if group.collapsed { "+" } else { "\u{2212}" }.to_string(),
        }
    }
}

impl Component for OutlineBand<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    type Style = OutlineBandStyle;
    type Output = OutlineBandOutput;

    fn style(cx: &StyleCx<'_>, _: OutlineBandStyleKey) -> OutlineBandStyle {
        let tokens = cx.tokens;
        let (colors, size, space) = (&tokens.color, &tokens.size, &tokens.space);
        let ink = cx.color(colors.text.muted);
        OutlineBandStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            text: TextLook::new(TypeRole::Caption, Stateful::new(ink)),
            lane: size.icon.sm,
            margin: space.gap.tight,
            min_side: size.handle.sm,
            inset: space.gap.tight,
            hit: size.hit.slop,
            radius: CornerRadius::same(tokens.shape.radius.sm.round() as u8),
            background: cx.color(colors.surface.background),
            edge: cx.color(colors.line.border),
            button: cx.color(colors.surface.chrome),
            ink,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Self::area(self.geometry).size()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    /// The level buttons, then the group buttons, in one roving group.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let frame = Frame::new(self.geometry, self.rows, self.columns, cx.style);
        self.laid = frame.layout();
        let mut buttons: Vec<&Button> = self.laid.buttons.iter().collect();
        buttons.sort_by_key(|button| matches!(button.action, OutlineAction::Toggle { .. }));
        for button in &buttons {
            let rect = self.geometry.mirror_rect(button.rect.expand(frame.hit));
            cx.part(button.part(), rect).sense(Sense::CLICK | Sense::FOCUS).focusable();
        }
        if !buttons.is_empty() {
            cx.roving(Roving::new(NavShape::Flat));
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        for button in &self.laid.buttons {
            if cx.part_activated(button.part()).is_some() {
                self.actions.push(button.action);
            }
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<OutlineBandWords>();
        let format = cx.format();
        let number = |n: u32| format.integer(u64::from(n));
        for button in &self.laid.buttons {
            let name = match button.action {
                OutlineAction::Level { level, .. } => words.level.fill(&[("n", &number(u32::from(level)))]),
                OutlineAction::Toggle { rows, group } => {
                    let (word, first, last) = match (rows, group.collapsed) {
                        (true, false) => (&words.collapse_rows, number(group.start + 1), number(group.end + 1)),
                        (true, true) => (&words.expand_rows, number(group.start + 1), number(group.end + 1)),
                        (false, false) => (&words.collapse_columns, column_letters(group.start), column_letters(group.end)),
                        (false, true) => (&words.expand_columns, column_letters(group.start), column_letters(group.end)),
                    };
                    word.fill(&[("first", &first), ("last", &last)])
                }
            };
            access.node(button.part(), Role::BUTTON).name = Name::Text(name);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (style, geometry) = (cx.style, self.geometry);
        let frame = Frame::new(geometry, self.rows, self.columns, style);
        if frame.row_band.width() <= 0.0 && frame.column_band.height() <= 0.0 {
            return;
        }
        let draw = ui.draw();
        let line = |a: Pos2, b: Pos2| PaintShape::Path(vec![geometry.mirror_pos(a), geometry.mirror_pos(b)]);
        for band in [frame.row_band, frame.column_band] {
            if band.width() > 0.0 && band.height() > 0.0 {
                draw.fill(&PaintShape::Rect(geometry.mirror_rect(band), CornerRadius::ZERO), style.background);
            }
        }
        if frame.row_band.width() > 0.0 {
            let band = frame.row_band;
            draw.stroke(&line(pos2(band.right(), band.top()), pos2(band.right(), band.bottom())), StrokeWidth::Hairline, style.edge);
        }
        if frame.column_band.height() > 0.0 {
            let band = frame.column_band;
            draw.stroke(&line(pos2(band.left(), band.bottom()), pos2(band.right(), band.bottom())), StrokeWidth::Hairline, style.edge);
        }
        for bracket in &self.laid.brackets {
            draw.stroke(&line(bracket.line[0], bracket.line[1]), StrokeWidth::Thin, style.ink);
            if let Some(tick) = bracket.tick {
                draw.stroke(&line(tick[0], tick[1]), StrokeWidth::Thin, style.ink);
            }
        }
        for button in &self.laid.buttons {
            let face = geometry.mirror_rect(button.rect);
            let shape = PaintShape::Rect(face, style.radius);
            draw.fill(&shape, style.button);
            draw.stroke(&shape, StrokeWidth::Thin, style.ink);
            let run = TextRun::new(button.glyph()).role(style.text.role).color(cx.resolve(&style.text.color));
            let layout = draw.layout(&run, face.width());
            draw.galley(face.center() - layout.size() / 2.0, &layout);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> OutlineBandOutput {
        OutlineBandOutput { response: cx.response.clone(), actions: self.actions }
    }
}

/// A button in the band.
struct Button {
    rect: Rect,
    action: OutlineAction,
}

/// An expanded group's bracket: a line along its lane from the first detail line to its button, with a tick at the
/// start (none when the start is scrolled out of view), both cut to the band.
struct Bracket {
    line: [Pos2; 2],
    tick: Option<[Pos2; 2]>,
}

#[derive(Default)]
struct Layout {
    buttons: Vec<Button>,
    brackets: Vec<Bracket>,
}

/// `segment`, an axis-aligned line, cut to `clip`; `None` when nothing of it is inside.
fn cut(segment: [Pos2; 2], clip: Rect) -> Option<[Pos2; 2]> {
    let kept = Rect::from_two_pos(segment[0], segment[1]).intersect(clip);
    (kept.min.x <= kept.max.x && kept.min.y <= kept.max.y).then_some([kept.min, kept.max])
}

/// One pass's geometry, in logical coordinates: the bands and the two axes.
struct Frame<'a> {
    geometry: &'a SheetGeometry,
    rows: &'a OutlineAxis,
    columns: &'a OutlineAxis,
    /// The lane's width, a button's side and the first lane's inset.
    step: f32,
    side: f32,
    inset: f32,
    /// How far a button's hit area reaches past its square.
    hit: f32,
    row_band: Rect,
    column_band: Rect,
}

impl<'a> Frame<'a> {
    fn new(geometry: &'a SheetGeometry, rows: &'a OutlineAxis, columns: &'a OutlineAxis, style: &OutlineBandStyle) -> Self {
        let step = lane(style.lane, geometry.zoom());
        let band = geometry.bands();
        let cells = geometry.cells();
        let min = cells.min - geometry.header_size() - band;
        Self {
            geometry,
            rows,
            columns,
            step,
            side: (step - style.margin).max(style.min_side),
            inset: style.inset,
            hit: style.hit,
            row_band: Rect::from_min_max(min, pos2(min.x + band.x, cells.bottom())),
            column_band: Rect::from_min_max(min, pos2(cells.right(), min.y + band.y)),
        }
    }

    /// Brackets and group buttons stay out of the corner, where the level buttons are.
    fn clip(&self, rows: bool) -> Rect {
        if rows {
            Rect::from_min_max(pos2(self.row_band.left(), self.geometry.cells().top()), self.row_band.max)
        } else {
            Rect::from_min_max(pos2(self.geometry.cells().left(), self.column_band.top()), self.column_band.max)
        }
    }

    /// Where `line` is on its axis, clipped to its pane: `None` when hidden or off screen.
    fn extent(&self, rows: bool, line: u32) -> Option<(f32, f32)> {
        let cell = CellAt::new(if rows { line } else { 0 }, if rows { 0 } else { line });
        let (rect, pane) = (self.geometry.cell_rect(cell), *self.geometry.range_panes(&CellRange::cell(cell)).first()?);
        let (a, b, top, bottom) =
            if rows { (rect.top(), rect.bottom(), pane.top(), pane.bottom()) } else { (rect.left(), rect.right(), pane.left(), pane.right()) };
        let (from, to) = (a.max(top), b.min(bottom));
        (b > a && to > from).then_some((from, to))
    }

    fn start(&self, rows: bool, line: u32) -> f32 {
        if rows { self.geometry.y(line) } else { self.geometry.x(line) }
    }

    fn end(&self, rows: bool, line: u32) -> f32 {
        let cell = self.geometry.cell_rect(CellAt::new(if rows { line } else { 0 }, if rows { 0 } else { line }));
        if rows { cell.bottom() } else { cell.right() }
    }

    /// The first on-screen line of `start..=end`: `start` itself, else the first shown line of the group's frozen
    /// lines, else the lines from the first scrolling line on, so a long group isn't scanned from its far-off start.
    fn first_visible(&self, rows: bool, start: u32, end: u32) -> Option<(u32, (f32, f32))> {
        if let Some(extent) = self.extent(rows, start) {
            return Some((start, extent));
        }
        let (frozen_rows, frozen_columns) = self.geometry.frozen();
        let frozen = if rows { frozen_rows } else { frozen_columns };
        let ((first_row, _), (first_column, _)) = self.geometry.visible();
        let scrolled = start.max(if rows { first_row } else { first_column });
        let in_frozen = start..=end.min(frozen.saturating_sub(1));
        let mut in_scroll = scrolled..=end;
        let shown = |line: u32| self.extent(rows, line).map(|extent| (line, extent));
        if start < frozen { in_frozen.chain(in_scroll).find_map(shown) } else { in_scroll.find_map(shown) }
    }

    fn point(rows: bool, along: f32, across: f32) -> Pos2 {
        if rows { pos2(across, along) } else { pos2(along, across) }
    }

    fn square(rows: bool, along: f32, across: f32, side: f32) -> Rect {
        Rect::from_center_size(Self::point(rows, along, across), vec2(side, side))
    }

    /// Lane `level`'s centre across (1-based).
    fn lane(&self, rows: bool, level: u8) -> f32 {
        let origin = if rows { self.row_band.left() } else { self.column_band.top() };
        origin + self.inset + (f32::from(level) - 0.5) * self.step
    }

    /// The buttons and brackets that are on screen. A group button the band cuts is left out.
    fn layout(&self) -> Layout {
        let mut out = Layout::default();
        let cells = self.geometry.cells();
        let header = self.geometry.header_size();
        // The level buttons sit along the heading strip beside the corner, or the band without headings.
        let corner = Vec2::new(
            if header.x > 0.0 { header.x } else { self.row_band.width() },
            if header.y > 0.0 { header.y } else { self.column_band.height() },
        );
        for (rows, axis) in [(true, self.rows), (false, self.columns)] {
            if axis.levels == 0 {
                continue;
            }
            let level_along = if rows { cells.top() - corner.y / 2.0 } else { cells.left() - corner.x / 2.0 };
            for level in 1..=axis.levels + 1 {
                let rect = Self::square(rows, level_along, self.lane(rows, level), self.side);
                out.buttons.push(Button { rect, action: OutlineAction::Level { rows, level } });
            }
            let clip = self.clip(rows);
            for group in &axis.groups {
                let across = self.lane(rows, group.level);
                let summary = if group.summary_before { group.start.saturating_sub(1) } else { group.end.saturating_add(1) };
                let has_summary = !(group.start..=group.end).contains(&summary);
                let button = has_summary.then(|| self.extent(rows, summary).map(|(a, b)| (a + b) / 2.0)).flatten();
                if let Some(mid) = button {
                    let rect = Self::square(rows, mid, across, self.side);
                    if clip.contains_rect(rect) {
                        out.buttons.push(Button { rect, action: OutlineAction::Toggle { rows, group: *group } });
                    }
                }
                if !group.collapsed {
                    self.bracket(&mut out, rows, group, button, across);
                }
            }
        }
        out
    }

    /// The bracket of one group's detail over the part that is on screen, and its tick.
    fn bracket(&self, out: &mut Layout, rows: bool, group: &OutlineGroup, button: Option<f32>, across: f32) {
        let Some((first_line, (from, _))) = self.first_visible(rows, group.start, group.end) else { return };
        let cells = self.geometry.cells();
        let hi = if rows { cells.bottom() } else { cells.right() };
        let clip = self.clip(rows);
        let tick_length = self.side / 2.0;
        let mut push = |line: [Pos2; 2], tick: Option<[Pos2; 2]>| {
            if let Some(line) = cut(line, clip) {
                out.brackets.push(Bracket { line, tick: tick.and_then(|tick| cut(tick, clip)) });
            }
        };
        if group.summary_before {
            // From the button on to the last detail line, with the tick there.
            let from = button.map_or(from, |mid| mid + self.side / 2.0);
            let end = self.end(rows, group.end);
            let to = end.min(hi);
            if to <= from {
                return;
            }
            let tick = (end <= hi).then(|| [Self::point(rows, to, across), Self::point(rows, to, across + tick_length)]);
            push([Self::point(rows, from, across), Self::point(rows, to, across)], tick);
            return;
        }
        let to = match button {
            Some(mid) => mid - self.side / 2.0,
            None => self.start(rows, group.end.saturating_add(1)).min(hi),
        };
        if to <= from {
            return;
        }
        let start_shown = first_line == group.start && (from - self.start(rows, group.start)).abs() < self.step / 2.0;
        let tick = start_shown.then(|| [Self::point(rows, from, across), Self::point(rows, from, across + tick_length)]);
        push([Self::point(rows, from, across), Self::point(rows, to, across)], tick);
    }
}
