//! The rulers around one page of a [`PageView`](super::PageView): [`VerticalRuler`] with table row grips, and
//! [`PageRulers`], which joins a [`TextRuler`], a [`VerticalRuler`] and
//! [`MarginHandles`] around the page.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use craft_behavior::kurbo::Point;
use craft_behavior::{
    Axis, CursorIcon, GesturePhase, HitLevel, HitPriority, Metrics, NavMove, NavShape, PartKey, PointerButton, PressPhase, Roving, Sense,
};
use craft_keymap::Mods;
use craft_text::ui::DocUnit;
use craft_text::ui::ruler::{RulerValues, TabAlign, TextRuler, TextRulerOutput, nudge_size, ruler_text, snap_step, snap_stop};
use craft_ui::canvas::{RulerTick, Rulers, TickRule};
use craft_ui::contract::{
    Access, Bind, Common, Component, Cx, Focus as _, IdSource, Input, MeasureCx, Name, NoValue, Numeric, Outcome, PaintUi, PartsCx, Pointer as _,
    Props as _, Remember as _, Role, Show as _, Styled as _, WordSetCx as _,
};
use craft_ui::primitives::{PaintShape, RulerFace, StrokeWidth, TextRun, Tick, TickEdge, TickRank};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::focus::{ActivateKeys, FocusShape, KeyUse};
use craft_ui::style::{ComponentStyle, FocusLook, StyleColor, StyleCx, TextLook, TypeRole};
use egui::{AsIdSalt, CornerRadius, IdSalt, Pos2, Rangef, Rect, Response, Ui, Vec2, pos2, vec2};

use super::margins::{MarginHandles, MarginHandlesOutput, MarginRules, PageEdges};
use super::view::PlacedPage;

/// One table row border grip, `GRIP.at(i)` for the bottom border of row `i`.
const GRIP: PartKey = PartKey::new("row");

/// The three grip lines across a row grip, in grip-gap steps from its middle.
const GRIP_LINES: [f32; 3] = [-1.0, 0.0, 1.0];

craft_ui::words! {
    /// The names of the parts of the page rulers.
    pub struct PageRulersWords {
        row_border: "pageRulers.rowBorder" = "Table Row Border";
        vertical: template "pageRulers.vertical" = "{name} vertical";
        margins: template "pageRulers.margins" = "{name} margins";
    }
}

/// A table row grip dragged on a vertical ruler: one gesture per drag, reported with its phase.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct RowDrag {
    /// Where the gesture stands.
    pub phase: GesturePhase,
    /// The row whose bottom border the grip moves.
    pub index: usize,
    /// The position the pointer gives the border, points from the page's top.
    pub position: f32,
    /// The drag pass's modifiers.
    pub mods: Mods,
}

/// What a [`VerticalRuler`] did this pass.
#[non_exhaustive]
pub struct VerticalRulerOutput {
    /// The ruler's response.
    pub response: Response,
    /// A table row grip dragged ([`VerticalRuler::rows`]).
    pub row: Option<RowDrag>,
}

impl Outcome for VerticalRulerOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The style of the vertical ruler.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct VerticalRulerStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The face: the prop's, else the component set's.
    face: RulerFace,
    /// The strip round the centred face's bar.
    strip: StyleColor,
    /// The text range.
    fill: StyleColor,
    /// The margins above and below it.
    margin: StyleColor,
    /// The ticks.
    tick: StyleColor,
    /// The grips' lines.
    marker: StyleColor,
    /// The tick numbers.
    #[style(text)]
    label: TextLook,
    /// The strip's width.
    thickness: f32,
    /// The gap between a tick and its number.
    pad: f32,
    /// The height of a row grip.
    grip: f32,
    /// The gap between the grip lines.
    grip_gap: f32,
    /// How far a grip line reaches to each side of the grip's middle.
    grip_reach: f32,
    /// The least length of the ruler.
    length: f32,
}

/// The press that began a row grip drag: the grip and the border's position then.
#[derive(Clone, Copy, Debug, Default)]
struct Held(Option<(usize, f32)>);

/// The vertical ruler of one page, with grips on the bottom borders of the caret row's table rows: the page's span is
/// shaded as margin and the text range filled, and a drag of a grip is one gesture.
///
/// - Ticks and numbers count from the page's top in the unit, through the canvas rulers' tick rule.
/// - The face ([`RulerFace`]) is the prop's, else the component set's: the edge face hangs the ticks from the strip's
///   inner edge with each number below its whole-unit tick; the centred face (Word's) draws a bar inset in the strip
///   with the ticks centred on it and the numbers in place of the whole-unit ticks, over the text range only.
/// - A grip drag reports a [`RowDrag`]: Begin, Update per move, Commit on release, Cancel on Escape. It snaps to the
///   ruler's step; Alt reports in `mods` that the drag is free of the snap and the position is not snapped.
/// - The grips are one Tab stop: Page Down and Page Up pick the next or previous grip, the arrows nudge the focused one
///   by the snap step (Shift larger, Alt finer) as a lone Commit.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(content_painter)]
#[must_use = "show the ruler with `show_at`"]
pub struct VerticalRuler<'a> {
    #[common]
    common: Common,
    page: &'a PlacedPage,
    /// The range the text occupies, points from the page's top.
    #[prop]
    text: Option<Rangef>,
    /// The unit the ticks and numbers show.
    #[prop]
    unit: DocUnit,
    /// The table row bottoms, points from the page's top, each a grip.
    #[prop]
    rows: &'a [f32],
    /// The face; unset, the scope's component set picks it ([`RulerFace::of`]).
    #[style_key]
    #[prop]
    face: Option<RulerFace>,
    report: Option<RowDrag>,
}

impl<'a> VerticalRuler<'a> {
    /// The ruler named `label` along `page`.
    pub fn new(label: impl Into<Label>, page: &'a PlacedPage) -> Self {
        VerticalRuler { common: Common::new(), page, text: None, unit: DocUnit::points(), rows: &[], face: None, report: None }.label(label)
    }

    /// Screen points per page point.
    fn scale(&self) -> f32 {
        (self.page.scale as f32).max(f32::EPSILON)
    }

    /// The screen y of the distance `d` from the page's top.
    fn y(&self, d: f32) -> f32 {
        self.page.rect.min.y + d * self.scale()
    }

    /// The page's height, points.
    fn page_height(&self) -> f32 {
        self.page.rect.height() / self.scale()
    }

    /// The grip of the border at `d`: the strip's width and one grip tall, centred on the border.
    fn grip_rect(&self, rect: Rect, style: &VerticalRulerStyle, d: f32) -> Rect {
        Rect::from_center_size(pos2(rect.center().x, self.y(d)), vec2(rect.width(), style.grip))
    }
}

impl Component for VerticalRuler<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_moves().with_activate(ActivateKeys::Neither);
    type Style = VerticalRulerStyle;
    type Output = VerticalRulerOutput;

    fn style(cx: &StyleCx<'_>, key: VerticalRulerStyleKey) -> VerticalRulerStyle {
        let tokens = cx.tokens;
        let colors = &tokens.color;
        VerticalRulerStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            face: key.face.unwrap_or(RulerFace::of(cx.set())),
            strip: cx.color(colors.surface.chrome),
            // As `TextRuler`'s: the text range is the light well, the margins are shaded as the desk round the page.
            fill: cx.color(colors.surface.sunken),
            margin: cx.color(colors.surface.canvas),
            tick: cx.color(colors.text.muted),
            marker: cx.color(colors.text.normal),
            label: TextLook::new(TypeRole::Caption, craft_ui::style::Stateful::new(cx.color(colors.text.muted))),
            thickness: tokens.typography.get(TypeRole::Caption).line_height,
            pad: tokens.space.gap.tight,
            grip: tokens.size.handle.md,
            grip_gap: tokens.space.gap.tight,
            grip_reach: tokens.size.handle.sm,
            length: tokens.size.length.track,
        }
    }

    fn fills(&self) -> Option<Axis> {
        Some(Axis::Y)
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(cx.style.thickness, cx.style.length)
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let handle = HitPriority::new(HitLevel::Handle, 1);
        for (index, row) in self.rows.iter().enumerate() {
            cx.part(GRIP.at(index as u64), self.grip_rect(rect, cx.style, *row))
                .sense(Sense::DRAG)
                .priority(handle)
                .cursor(CursorIcon::ResizeVertical)
                .focusable()
                .nudges()
                .focus_shape(FocusShape::Rect);
        }
        cx.roving(Roving::new(NavShape::Flat).page(Some(1)).home_end(false));
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let scale = self.scale();
        let height = self.page_height();
        let numbers = cx.env.units();
        let step = snap_step(&ruler_text(self.unit, &numbers), self.unit);
        let mut held = *cx.memory::<Held>();
        for (index, from_rows) in self.rows.iter().copied().enumerate() {
            let press = cx.part(GRIP.at(index as u64)).press;
            // A border stays below the one above it and within the page.
            let above = index.checked_sub(1).and_then(|before| self.rows.get(before)).copied().unwrap_or(0.0);
            let target = |from: f32, y: f32, origin: f32| {
                let to = from + (y - origin) / scale;
                if to == from { from } else { snap_stop(to, step, height, press.mods.alt).max(above.min(height)) }
            };
            let report = |phase: GesturePhase, position: f32| Some(RowDrag { phase, index, position, mods: press.mods });
            match press.phase {
                PressPhase::DragStart { start, .. } if start.button == PointerButton::Primary => {
                    held = Held(Some((index, from_rows)));
                    self.report = report(GesturePhase::Begin, from_rows);
                    cx.focus_own();
                }
                PressPhase::Drag { start, pos, .. } if matches!(held.0, Some((at, _)) if at == index) => {
                    let from = held.0.map_or(from_rows, |(_, from)| from);
                    self.report = report(GesturePhase::Update, target(from, pos.y, start.origin.y));
                }
                PressPhase::Release { start, pos, click: false, .. } if matches!(held.0, Some((at, _)) if at == index) => {
                    let from = held.0.map_or(from_rows, |(_, from)| from);
                    self.report = report(GesturePhase::Commit, target(from, pos.y, start.origin.y));
                    held = Held(None);
                }
                PressPhase::Cancel { .. } if matches!(held.0, Some((at, _)) if at == index) => {
                    self.report = report(GesturePhase::Cancel, from_rows);
                    held = Held(None);
                }
                _ => {}
            }
        }
        if held.0.is_none()
            && let Some(part) = cx.active_part()
            && part.key == GRIP
            && let Some(index) = part.index.map(|index| index as usize)
            && let Some(position) = self.rows.get(index).copied()
        {
            let mut by = 0.0;
            let mut mods = Mods::NONE;
            while let Some((mv, moved)) = cx.intents().take_move() {
                mods.alt |= moved.fine;
                mods.shift |= moved.extend && !moved.fine;
                let size = nudge_size(step, moved.fine, moved.extend);
                match mv {
                    NavMove::Forward | NavMove::Down => by += size,
                    NavMove::Back | NavMove::Up => by -= size,
                    _ => {}
                }
            }
            if let Some((direction, nudge)) = cx.intents().take_nudge() {
                let size = nudge_size(step, nudge.alt, nudge.shift);
                mods = nudge;
                by += direction.y * size;
            }
            if by != 0.0 {
                let position = (position + by).clamp(0.0, height);
                self.report = Some(RowDrag { phase: GesturePhase::Commit, index, position, mods });
            }
        }
        *cx.memory::<Held>() = held;
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<PageRulersWords>();
        let numbers = cx.env.units();
        let text = ruler_text(self.unit, &numbers);
        let step = f64::from(snap_step(&text, self.unit));
        let range = 0.0..=f64::from(self.page_height());
        for (index, row) in self.rows.iter().enumerate() {
            let at = f64::from(*row);
            let node = access.node(GRIP.at(index as u64), Role::SPLITTER);
            node.name = Name::Text(words.row_border.as_str().to_owned());
            node.numeric = Some(Numeric::new(at, range.clone()).step(step).text(text.format_value(&numbers, at, 0.0)));
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style: &VerticalRulerStyle = cx.style;
        let rect = cx.rect;
        let scale = self.scale();
        let numbers = cx.env.units();
        let text = ruler_text(self.unit, &numbers);
        let per_doc = text.units.per_doc;
        let shape = |rect: Rect| PaintShape::Rect(rect, CornerRadius::ZERO);
        let face = style.face;
        let bar = ui.content_painter(cx).ruler_bar(rect, TickEdge::Right, face);
        let page = Rect::from_x_y_ranges(rect.x_range(), self.y(0.0)..=self.y(self.page_height())).intersect(rect);
        let draw = ui.draw();
        // The edge face shades the whole strip; the centred face shades the page's span of its bar on the strip's own
        // surface.
        match face {
            RulerFace::Edge => draw.fill(&shape(rect), style.margin),
            RulerFace::Centred => {
                draw.fill(&shape(rect), style.strip);
                draw.fill(&shape(Rect::from_x_y_ranges(bar.x_range(), page.y_range())), style.margin);
            }
        }
        let text_band = self.text.map(|range| Rect::from_x_y_ranges(bar.x_range(), self.y(range.min)..=self.y(range.max)).intersect(page));
        if let Some(band) = text_band.filter(|band| band.is_positive()) {
            draw.fill(&shape(band), style.fill);
        }
        let grips: Vec<Rect> = self.rows.iter().map(|row| self.grip_rect(rect, style, *row)).collect();
        let (lo, hi) = (((page.min.y - self.y(0.0)) / scale).max(0.0), ((page.max.y - self.y(0.0)) / scale).max(0.0));
        let ticks: Vec<RulerTick> =
            Rulers::ticks(&text.units, TickRule::of_unit(self.unit.kind), f64::from(scale), (f64::from(lo) * per_doc)..=(f64::from(hi) * per_doc));
        let draw = ui.draw();
        let color: StyleColor = cx.resolve(&style.label.color);
        // The centred face numbers the text range only, as a word processor's vertical ruler does.
        let numbered_span = match face {
            RulerFace::Edge => page,
            RulerFace::Centred => text_band.unwrap_or(page),
        };
        let mut runs = Vec::new();
        for tick in ticks.iter().filter(|tick| matches!(tick.rank, TickRank::Major)) {
            let y = self.y((tick.value / per_doc) as f32);
            let run = TextRun::new(text.tick_label(&numbers, tick.value, tick.step)).role(style.label.role).color(color);
            let size = draw.layout(&run, f32::INFINITY).size();
            let boxed = match face {
                RulerFace::Edge => Rect::from_min_size(pos2(rect.min.x, y + style.pad), vec2(rect.width(), size.y)),
                RulerFace::Centred => RulerFace::centred_number(bar, TickEdge::Right, y, size),
            };
            let span = Rect::from_x_y_ranges(rect.x_range(), numbered_span.y_range());
            if span.contains_rect(boxed) && size.x <= bar.width() && !grips.iter().any(|grip| grip.intersects(boxed)) {
                runs.push((boxed, run));
            }
        }
        let marks: Vec<Tick> = ticks
            .iter()
            .map(|tick| (self.y((tick.value / per_doc) as f32), tick))
            .filter(|(y, _)| (page.min.y..=page.max.y).contains(y))
            .map(|(y, tick)| Tick::new(y - bar.min.y, tick.rank))
            .collect();
        let boxes: Vec<Rect> = runs.iter().map(|(boxed, _)| *boxed).collect();
        let marks = face.beside_numbers(TickEdge::Right, bar.min.y, marks, &boxes, style.pad);
        ui.content_painter(cx).ticks(bar, TickEdge::Right, face, &marks);
        let draw = ui.draw();
        for (boxed, run) in &runs {
            draw.text(*boxed, run);
        }
        for grip in &grips {
            draw.fill(&shape(*grip), style.margin);
            draw.stroke(&shape(*grip), StrokeWidth::Thin, style.marker);
            let middle = grip.center();
            for step in GRIP_LINES {
                let at: Pos2 = pos2(middle.x, middle.y + step * style.grip_gap);
                draw.stroke(
                    &PaintShape::Path(vec![at - vec2(style.grip_reach, 0.0), at + vec2(style.grip_reach, 0.0)]),
                    StrokeWidth::Thin,
                    style.marker,
                );
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> VerticalRulerOutput {
        VerticalRulerOutput { response: cx.response.clone(), row: self.report }
    }
}

/// What [`PageRulers`] did this pass.
#[non_exhaustive]
pub struct PageRulersOutput {
    /// The horizontal ruler's output.
    pub ruler: TextRulerOutput,
    /// The margin handles' output.
    pub margins: MarginHandlesOutput,
    /// A table row grip dragged on the vertical ruler.
    pub row: Option<RowDrag>,
}

/// The rulers around a page: the text ruler over the top band, the vertical ruler and the margin handles last, so
/// their edges win. It shows no guides, no zero point and no units menu; `Rulers` is unchanged.
///
/// The text ruler counts from the text column's start edge and is cut to the page's width; the vertical ruler counts
/// from the page's top and shades the margins. The strips are as thick as the canvas rulers' strips. Both rulers take
/// the face given here, else the scope's component set's.
#[must_use = "show the rulers with `show_at`"]
#[derive(craft_ui::contract::Pattern)]
pub struct PageRulers<'a> {
    label: Label,
    page: &'a PlacedPage,
    edges: Bind<'a, PageEdges>,
    ruler: Bind<'a, RulerValues>,
    column: Option<(f32, f32)>,
    unit: DocUnit,
    kind: Option<TabAlign>,
    right_to_left: Option<bool>,
    columns: &'a [f32],
    gaps: &'a [(f32, f32)],
    rows: &'a [f32],
    gutter: f64,
    mirrored_even: bool,
    id: IdSource,
    gaps_draggable: bool,
    face: Option<RulerFace>,
}

impl<'a> PageRulers<'a> {
    /// The rulers named `label` around `page`, editing its margin `edges` and the paragraph's `ruler` values.
    pub fn new(label: impl Into<Label>, page: &'a PlacedPage, edges: Bind<'a, PageEdges>, ruler: Bind<'a, RulerValues>) -> Self {
        PageRulers {
            label: label.into(),
            page,
            edges,
            ruler,
            column: None,
            unit: DocUnit::points(),
            kind: None,
            right_to_left: None,
            columns: &[],
            gaps: &[],
            rows: &[],
            gutter: 0.0,
            mirrored_even: false,
            id: IdSource::Auto,
            gaps_draggable: true,
            face: None,
        }
    }

    /// The face of both rulers ([`TextRuler::face`], [`VerticalRuler::face`]); unset, the scope's component set picks it.
    pub fn face(mut self, face: RulerFace) -> Self {
        self.face = Some(face);
        self
    }

    /// Salts the text ruler's id with `salt` (the caret paragraph's key): a new salt is a new ruler, so a drag open on
    /// the ruler of another paragraph ends with no commit.
    pub fn id_salt(mut self, salt: impl AsIdSalt) -> Self {
        self.id = IdSource::Salt(IdSalt::new(salt));
        self
    }

    /// Whether the column gaps' edges take a drag (the default): [`TextRuler::gaps_draggable`].
    pub fn gaps_draggable(mut self, on: bool) -> Self {
        self.gaps_draggable = on;
        self
    }

    /// The text column's start and end, points from the page's left. Without it the column is the text area between the
    /// margins.
    pub fn column(mut self, column: (f32, f32)) -> Self {
        self.column = Some(column);
        self
    }

    /// The unit the ticks and numbers show.
    pub fn unit(mut self, unit: DocUnit) -> Self {
        self.unit = unit;
        self
    }

    /// Shows the tab selector in the canvas's corner, with the kind a click sets.
    pub fn kind(mut self, kind: Option<TabAlign>) -> Self {
        self.kind = kind;
        self
    }

    /// The paragraph's direction is right to left (`true`) or left to right (`false`).
    pub fn right_to_left(mut self, right_to_left: Option<bool>) -> Self {
        self.right_to_left = right_to_left;
        self
    }

    /// The caret row's table column borders, points from the column's start edge.
    pub fn columns(mut self, columns: &'a [f32]) -> Self {
        self.columns = columns;
        self
    }

    /// The text column gaps as (start, end) points.
    pub fn gaps(mut self, gaps: &'a [(f32, f32)]) -> Self {
        self.gaps = gaps;
        self
    }

    /// The table row bottoms, points from the page's top.
    pub fn rows(mut self, rows: &'a [f32]) -> Self {
        self.rows = rows;
        self
    }

    /// Binding room added to the inside margin, points.
    pub fn gutter(mut self, gutter: f64) -> Self {
        self.gutter = gutter;
        self
    }

    /// The page is an even page of mirrored margins.
    pub fn mirrored_even(mut self, mirrored_even: bool) -> Self {
        self.mirrored_even = mirrored_even;
        self
    }

    /// Shows the rulers over the canvas at `canvas_rect`.
    pub fn show_at(self, ui: &mut Ui, canvas_rect: Rect) -> PageRulersOutput {
        let PageRulers {
            label,
            page,
            mut edges,
            ruler,
            column,
            unit,
            kind,
            right_to_left,
            columns,
            gaps,
            rows,
            gutter,
            mirrored_even,
            id,
            gaps_draggable,
            face,
        } = self;
        let (words, interface_rtl) = {
            let env = Env::of(ui);
            (env.words::<PageRulersWords>(), env.view().direction().is_rtl())
        };
        let (top_strip, left_strip, corner) = Rulers::strips(ui, canvas_rect);
        let page_size = (f64::from(page.rect.width()) / page.scale, f64::from(page.rect.height()) / page.scale);
        let shown = edges.reborrow().get().and_then(|edges| edges.same().copied()).unwrap_or_default();
        let rules = MarginRules::new(page_size, shown).gutter(gutter).mirrored_even(mirrored_even);
        let bands = MarginHandles::spans(&rules, Point::ZERO);
        let (left, right) = (bands.horizontal.first().map_or(0.0, |band| band.end), bands.horizontal.get(1).map_or(page_size.0, |band| band.start));
        let (top, bottom) = (bands.vertical.first().map_or(0.0, |band| band.end), bands.vertical.get(1).map_or(page_size.1, |band| band.start));
        let column = column.unwrap_or((left as f32, right as f32));
        let scale = (page.scale as f32).max(f32::EPSILON);

        let rtl = right_to_left.unwrap_or(interface_rtl);
        // The ruler counts from the column's start edge and covers the page's width; both are given as screen x. With the
        // tab selector it lies along the top strip and the canvas's corner, which holds the selector.
        let origin = page.rect.min.x + (if rtl { column.1 } else { column.0 }) * scale;
        let extent = Rangef::new(page.rect.min.x, page.rect.max.x);
        let mut horizontal = TextRuler::new(label.clone(), ruler, (column.1 - column.0).max(0.0))
            .unit(unit)
            .columns(columns)
            .gaps(gaps)
            .gaps_draggable(gaps_draggable)
            .origin(origin)
            .extent(extent)
            .scale(scale)
            .right_to_left(rtl)
            .id_source(id);
        let mut band = top_strip;
        if let Some(kind) = kind {
            horizontal = horizontal.kind(kind).corner(corner);
            band = top_strip.union(corner);
        }
        if let Some(face) = face {
            horizontal = horizontal.face(face);
        }
        let ruler_out = horizontal.show_at(ui, band);

        let vertical_label = Label::from(words.vertical.fill(&[("name", label.as_str())]));
        let mut vertical = VerticalRuler::new(vertical_label, page).text(Rangef::new(top as f32, bottom as f32)).unit(unit).rows(rows);
        if let Some(face) = face {
            vertical = vertical.face(face);
        }
        let vertical = vertical.show_at(ui, left_strip);

        let margins_label = Label::from(words.margins.fill(&[("name", label.as_str())]));
        let margins =
            MarginHandles::new(margins_label, page, edges).gutter(gutter).mirrored_even(mirrored_even).unit(unit.kind).show_at(ui, canvas_rect);
        PageRulersOutput { ruler: ruler_out, margins, row: vertical.row }
    }
}
