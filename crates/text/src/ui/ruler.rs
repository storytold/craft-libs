//! [`TextRuler`]: the ruler over a paragraph's column with its tab stops and indents, as
//! Word's horizontal ruler, InDesign's and Illustrator's Tabs panels show them: the first-line,
//! left (hanging) and right indent markers, the stops with their alignment glyphs, a click on the
//! ruler to set a stop, drags to move them and off the ruler to clear them, and (Word) the tab
//! selector in the corner that picks the kind of stop a click sets.
//!
//! Stops and indents are the app's paragraph attributes; the ruler reports each gesture as one
//! edit of the whole [`RulerValues`], so one drag is one undo step.
//!
//! ```ignore
//! let out = TextRuler::new("ruler", Bind::given(values.clone()), column_width).unit(unit).kind(tab_kind).show_at(ui, strip);
//! if let Some(EditEvent::Commit { after, .. }) = out.edit.event { doc.set_paragraph_tabs(after); }
//! if let Some(kind) = out.kind { tab_kind = kind; }
//! if out.open_tabs_dialog { dialogs.open(TabStopsDialog::new(values.stops.clone(), default_tab)); }
//! ```

use super::length::DocUnit;
use craft_behavior::{Axis, GesturePhase, HitLevel, HitPriority, Metrics, Sense};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Input, MeasureCx, Name, Numeric, Outcome, PaintUi, PartsCx, Props as _,
    Role, WordSetCx as _,
};
use craft_ui::runtime::focus::{ActivateKeys, FocusShape, KeyUse};
use craft_ui::style::{ComponentStyle, Flags, FocusLook, Stateful, StyleColor, StyleCx, TextLook, TypeRole};
use egui::{Rect, Response, Vec2};

mod geometry;
mod items;
mod paint;
mod run;
#[cfg(feature = "screen-testing")]
mod stories;

use geometry::{Geometry, Shapes};
use items::{COLUMN, Item, SELECTOR, STRIP};

craft_ui::words! {
    /// The ruler's AccessKit names and readout (Word's).
    pub struct TextRulerWords {
        left_tab: "textRuler.leftTab" = "Left Tab";
        center_tab: "textRuler.centerTab" = "Center Tab";
        right_tab: "textRuler.rightTab" = "Right Tab";
        decimal_tab: "textRuler.decimalTab" = "Decimal Tab";
        bar_tab: "textRuler.barTab" = "Bar Tab";
        first_line_indent: "textRuler.firstLineIndent" = "First Line Indent";
        hanging_indent: "textRuler.hangingIndent" = "Hanging Indent";
        left_indent: "textRuler.leftIndent" = "Left Indent";
        right_indent: "textRuler.rightIndent" = "Right Indent";
        stop: template "textRuler.stop" = "{kind} at {position}";
        tab_selector: template "textRuler.tabSelector" = "Tab Selector: {kind}";
        table_column: "textRuler.tableColumn" = "Table Column Width";
    }
}

/// How a tab stop aligns the text after it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum TabAlign {
    /// Left.
    #[default]
    Left,
    /// Center.
    Center,
    /// Right.
    Right,
    /// On the decimal point (InDesign: on the character the app sets, "." by default).
    Decimal,
    /// A vertical bar at the stop, not a stop (Word).
    Bar,
}

impl TabAlign {
    /// The kinds the corner selector cycles through and the Tabs dialog lists, in Word's order.
    pub const ALL: [TabAlign; 5] = [TabAlign::Left, TabAlign::Center, TabAlign::Right, TabAlign::Decimal, TabAlign::Bar];

    /// The selector's next kind (`back`: the previous one), wrapping.
    pub fn next(self, back: bool) -> TabAlign {
        let count = TabAlign::ALL.len();
        let at = TabAlign::ALL.iter().position(|kind| *kind == self).unwrap_or(0);
        let to = if back { (at + count - 1) % count } else { (at + 1) % count };
        TabAlign::ALL.get(to).copied().unwrap_or_default()
    }
}

/// What fills the space before a stop.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum TabLeader {
    /// None.
    #[default]
    None,
    /// ……
    Dot,
    /// ------
    Hyphen,
    /// ______
    Underscore,
    /// ······
    MiddleDot,
}

impl TabLeader {
    /// Every leader, in the order a menu lists them.
    pub const ALL: [TabLeader; 5] = [TabLeader::None, TabLeader::Dot, TabLeader::Hyphen, TabLeader::Underscore, TabLeader::MiddleDot];

    /// The fill a closed leader box shows ("" for none): Word's "……", "------", "______",
    /// "······".
    pub fn sample(self) -> &'static str {
        match self {
            TabLeader::None => "",
            TabLeader::Dot => "……",
            TabLeader::Hyphen => "------",
            TabLeader::Underscore => "______",
            TabLeader::MiddleDot => "······",
        }
    }
}

/// A tab stop: its position from the paragraph's start edge (points; the right edge in a
/// right-to-left paragraph), alignment and leader. InDesign's leader is any text; the app maps
/// it to the nearest [`TabLeader`] or keeps its own in a field the ruler ignores.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct TabStop {
    /// Position in points from the paragraph's start edge.
    pub position: f32,
    /// The align.
    pub align: TabAlign,
    /// The leader.
    pub leader: TabLeader,
}

impl TabStop {
    /// A stop at `position` points with `align` and `leader`.
    #[must_use]
    pub const fn new(position: f32, align: TabAlign, leader: TabLeader) -> Self {
        TabStop { position, align, leader }
    }
}

impl ColumnBorder {
    /// Border `index` at `position` points from the column's start edge.
    #[must_use]
    pub const fn new(index: usize, position: f32) -> Self {
        ColumnBorder { index, position }
    }
}

/// Stops within this many points are the same stop: setting a stop there replaces it.
pub const SAME_POSITION: f32 = 0.5;

/// Stops are kept within ±1584 pt (22 in, Word's bound).
pub const MAX_POSITION: f32 = 1584.0;

/// What the ruler shows and edits for the caret's paragraph.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct RulerValues {
    /// Sorted by position.
    pub stops: Vec<TabStop>,
    /// The left (start) indent, points from the column's start edge.
    pub left_indent: f32,
    /// The first line's indent, relative to `left_indent` (negative hangs).
    pub first_line_indent: f32,
    /// The right (end) indent, points from the column's end edge.
    pub right_indent: f32,
}

/// A position for a stop: snapped to `step` (unless `free`, Alt held), within `0..=width`.
pub fn snap_stop(position: f32, step: f32, width: f32, free: bool) -> f32 {
    let position = if position.is_finite() { position } else { 0.0 };
    let snapped = if free || !step.is_finite() || step <= 0.0 { position } else { (position / step).round() * step };
    snapped.clamp(0.0, if width.is_finite() { width.max(0.0) } else { 0.0 })
}

/// Whether a pointer at `y` is far enough above or below the ruler `strip` to take a dragged stop
/// off: 18 screen points.
pub fn dropped_off(y: f32, strip: Rect) -> bool {
    y > strip.max.y + DROP_OFF || y < strip.min.y - DROP_OFF
}

/// How far (screen points) past the strip a dragged stop must be to come off.
const DROP_OFF: f32 = 18.0;

/// A table column border on the ruler: the table's left edge is
/// border 0, then the right edge of each cell of the caret's row.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ColumnBorder {
    /// Index of the border: 0 is the table's left edge.
    pub index: usize,
    /// Points from the column's start edge. The columns right of the border move along with it:
    /// only the column left of it changes width.
    pub position: f32,
}

/// A table column border drag: one gesture per drag, reported with its phase.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ColumnDrag {
    /// Where the gesture stands.
    pub phase: GesturePhase,
    /// The border and the position the pointer gives it.
    pub border: ColumnBorder,
}

/// What the ruler did this pass.
#[non_exhaustive]
pub struct TextRulerOutput {
    /// A placed, moved or cleared stop or a moved indent marker, as the whole new values.
    pub edit: EditResponse<RulerValues>,
    /// The stop the user clicked (the Tabs panel's position field edits it).
    pub picked: Option<usize>,
    /// A double-click on a stop (Word opens the Tabs dialog).
    pub open_tabs_dialog: bool,
    /// A table column border dragged ([`TextRuler::columns`]).
    pub column: Option<ColumnDrag>,
    /// The kind the tab selector was clicked to: the next kind, the previous one for a secondary click.
    pub kind: Option<TabAlign>,
}

impl Outcome for TextRulerOutput {
    type Value = RulerValues;

    fn response(&self) -> &Response {
        &self.edit.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}

/// What the ruler reports besides the edit.
#[derive(Clone, Debug, Default)]
pub(super) struct Report {
    pub(super) picked: Option<usize>,
    pub(super) open_tabs_dialog: bool,
    pub(super) column: Option<ColumnDrag>,
    pub(super) kind: Option<TabAlign>,
}

/// The style of the ruler: the strip's colours and the named sizes of its markers.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TextRulerStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The column.
    fill: StyleColor,
    /// The margins beside the column.
    margin: StyleColor,
    /// The ticks.
    tick: StyleColor,
    /// The markers' fill.
    marker_fill: StyleColor,
    /// A stop dragged off the strip.
    marker_off: StyleColor,
    /// The markers' lines; the picked stop takes the accent.
    marker: Stateful<StyleColor>,
    /// The tick numbers.
    #[style(text)]
    label: TextLook,
    /// The strip's depth.
    thickness: f32,
    /// The gap between a tick and its number.
    pad: f32,
    /// The width of an indent marker.
    marker_width: f32,
    /// The width of a table column marker.
    column_width: f32,
    /// The height of an indent triangle and of the left indent's box.
    marker_height: f32,
    /// The height of a stop's stem.
    stem: f32,
    /// The length of a stop's foot.
    foot: f32,
    /// The radius of a decimal stop's dot.
    dot: f32,
    /// The gap between the grip lines of a table column marker.
    grip_gap: f32,
    /// How far a grip line reaches above and below the marker's middle.
    grip_reach: f32,
    /// The least length of the ruler, which spans the width it is given.
    length: f32,
}

/// The ruler for one paragraph, shown in the strip the app lays out (a canvas ruler, or the Tabs panel's own
/// strip above a text frame).
///
/// - Ticks and numbers in the unit through the canvas rulers' tick rule (inches follow Word's sixteenths), from the
///   column's start edge; numbers keep clear of the markers. The column takes the ruler's fill, the margins the
///   margin shade.
/// - Indent markers (Word's): the first-line triangle down at the top, the hanging triangle up and the left box under
///   it at the bottom, the right triangle at the right. Dragging the box moves first-line and left together; Alt drags
///   freely (no snap).
/// - A click on the empty column sets a stop of the selector's kind at the snapped position (1/16 in, or the unit's
///   minor tick); dragging a stop moves it; dragging it off the ruler ([`dropped_off`]) clears it on release, the stop
///   drawn faded meanwhile. Each gesture is one edit of the whole values; Escape restores the start.
/// - The tab selector ([`kind`](Self::kind)) sits in the corner square at the interface's start edge of the rect:
///   click for the next kind, secondary click for the previous one.
/// - A table's column borders ([`columns`](Self::columns)): a grip marker on each border of the caret row; dragging
///   one reports a [`ColumnDrag`] gesture, snapped like a stop (Alt free). A border is a slider too: the arrows and the screen
///   reader's Increment, Decrement and SetValue report a lone [`GesturePhase::Commit`].
/// - Right-to-left paragraphs mirror the ruler: positions run from the right edge, and the stops' feet point the other
///   way.
/// - Keys: each marker and stop is a Tab stop. Page Down and Page Up move to the next or previous marker or stop, the
///   arrows nudge the focused one by the snap step (Shift larger, Alt finer), Delete and Backspace clear a stop and
///   focus the one before it. Each key is one edit. Each marker and stop is a slider named by its kind, with its value
///   in the unit; Increment, Decrement and SetValue act on it.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(content_painter)]
pub struct TextRuler<'a> {
    #[common]
    common: Common,
    value: Bind<'a, RulerValues>,
    width: f32,
    /// The caret row's table column borders, points from the column's start edge: the table's left edge first, then
    /// the right edge of each cell. Empty (the default) when the caret is not in a table.
    #[prop]
    columns: &'a [f32],
    /// The unit the markers' values and the ticks show.
    #[prop]
    unit: DocUnit,
    /// Shows Word's tab selector in the corner, with the kind a click sets. Without it a click sets a left stop.
    #[prop]
    kind: Option<TabAlign>,
    /// Screen points per point (the view's zoom when the ruler sits over a text frame on the canvas); without it the
    /// column fits into the strip.
    #[prop(quantity = "scale")]
    scale: Option<f32>,
    /// The paragraph's direction is right to left (`true`) or left to right (`false`): the ruler mirrors for right to
    /// left. Unset, the paragraph runs in the interface's direction.
    #[prop]
    right_to_left: Option<bool>,
    /// The stop shown picked.
    #[prop(name = "picked_stop")]
    selected: Option<usize>,
    report: Report,
}

impl<'a> TextRuler<'a> {
    /// The ruler named `label` over `value`, for a column `width` points wide.
    pub fn new(label: impl Into<craft_ui::runtime::env::Label>, value: Bind<'a, RulerValues>, width: f32) -> Self {
        TextRuler {
            common: Common::new(),
            value,
            width: if width.is_finite() { width.max(0.0) } else { 0.0 },
            columns: &[],
            unit: DocUnit::points(),
            kind: None,
            scale: None,
            right_to_left: None,
            selected: None,
            report: Report::default(),
        }
        .label(label)
    }

    /// The corner square, the strip and the markers' sizes inside `rect`.
    fn layout(&self, rect: Rect, style: &TextRulerStyle, interface_rtl: bool) -> Layout {
        let corner_width = if self.kind.is_some() { style.thickness.min(rect.width()) } else { 0.0 };
        let (corner, strip) = if interface_rtl {
            (
                Rect::from_min_max(egui::pos2(rect.max.x - corner_width, rect.min.y), rect.max),
                Rect::from_min_max(rect.min, egui::pos2(rect.max.x - corner_width, rect.max.y)),
            )
        } else {
            (
                Rect::from_min_max(rect.min, egui::pos2(rect.min.x + corner_width, rect.max.y)),
                Rect::from_min_max(egui::pos2(rect.min.x + corner_width, rect.min.y), rect.max),
            )
        };
        let fit = if self.width > 0.0 { strip.width() / self.width } else { 1.0 };
        let scale = self.scale.filter(|scale| scale.is_finite() && *scale > 0.0).unwrap_or(fit).max(f32::EPSILON);
        Layout {
            g: Geometry { strip, scale, right_to_left: self.right_to_left.unwrap_or(interface_rtl), width: self.width },
            shapes: Shapes { width: style.marker_width, column: style.column_width, tip: style.marker_height, stem: style.stem, foot: style.foot },
            corner,
        }
    }

    fn tab_name(words: &TextRulerWords, kind: TabAlign) -> &str {
        match kind {
            TabAlign::Left => words.left_tab.as_str(),
            TabAlign::Center => words.center_tab.as_str(),
            TabAlign::Right => words.right_tab.as_str(),
            TabAlign::Decimal => words.decimal_tab.as_str(),
            TabAlign::Bar => words.bar_tab.as_str(),
        }
    }

    fn item_name<'w>(words: &'w TextRulerWords, item: Item, values: &RulerValues) -> &'w str {
        match item {
            Item::FirstLine => words.first_line_indent.as_str(),
            Item::Hanging => words.hanging_indent.as_str(),
            Item::LeftBox => words.left_indent.as_str(),
            Item::Right => words.right_indent.as_str(),
            Item::Stop(at) => Self::tab_name(words, values.stops.get(at).map(|stop| stop.align).unwrap_or_default()),
        }
    }
}

/// The corner, the strip with its column and the marker sizes.
pub(super) struct Layout {
    g: Geometry,
    shapes: Shapes,
    corner: Rect,
}

impl Component for TextRuler<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_moves().with_deletes().with_activate(ActivateKeys::Neither).with_value(true);
    type Style = TextRulerStyle;
    type Output = TextRulerOutput;

    fn style(cx: &StyleCx<'_>, _: TextRulerStyleKey) -> TextRulerStyle {
        let tokens = cx.tokens;
        let colors = &tokens.color;
        TextRulerStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            fill: cx.color(colors.surface.chrome),
            margin: cx.color(colors.surface.sunken),
            tick: cx.color(colors.text.muted),
            marker_fill: cx.color(colors.surface.raised),
            marker_off: cx.color(colors.text.faint),
            marker: Stateful::new(cx.color(colors.text.normal)).when(Flags::SELECTED, cx.color(colors.accent.strong)),
            label: TextLook::new(TypeRole::Caption, Stateful::new(cx.color(colors.text.muted))),
            thickness: tokens.typography.get(TypeRole::Caption).line_height,
            pad: tokens.space.gap.tight,
            marker_width: tokens.size.handle.md,
            column_width: tokens.size.handle.md,
            marker_height: tokens.size.handle.sm,
            stem: tokens.size.handle.sm,
            foot: tokens.size.handle.sm,
            dot: tokens.stroke.thick,
            grip_gap: tokens.space.gap.tight,
            grip_reach: tokens.size.handle.sm,
            length: tokens.size.length.track,
        }
    }

    fn fills(&self) -> Option<Axis> {
        Some(Axis::X)
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(cx.style.length, cx.style.thickness)
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn bind(&mut self) -> Option<BindMut<'_, RulerValues>> {
        Some(self.value.reborrow())
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let layout = self.layout(rect, cx.style, cx.flow.is_rtl());
        let values = cx.value().cloned().unwrap_or_default();
        let handle = HitPriority::new(HitLevel::Handle, 0);
        cx.part(STRIP, layout.g.strip).sense(Sense::CLICK | Sense::DRAG).pointer_only("the Tabs dialog sets a stop by number");
        for item in Item::all(&values) {
            let part = cx
                .part(item.part(), layout.shapes.rect(&layout.g, item, &values))
                .sense(Sense::DRAG | Sense::CLICK)
                .priority(handle)
                .cursor(craft_behavior::CursorIcon::ResizeHorizontal)
                .focusable()
                .focus_shape(FocusShape::Rect)
                .value_part();
            if matches!(item, Item::Stop(at) if self.selected == Some(at)) {
                part.selected().flags(Flags::SELECTED);
            }
        }
        for (index, position) in self.columns.iter().enumerate() {
            let marker = layout.shapes.column_marker(layout.g.strip, layout.g.x(*position));
            cx.part(COLUMN.at(index as u64), marker)
                .sense(Sense::DRAG)
                .priority(handle)
                .cursor(craft_behavior::CursorIcon::ResizeHorizontal)
                .focusable()
                .focus_shape(FocusShape::Rect)
                .value_part();
        }
        if self.kind.is_some() {
            cx.part(SELECTOR, layout.corner)
                .sense(Sense::CLICK | Sense::CONTEXT_CLICK)
                .priority(handle)
                .pointer_only("the Tabs dialog sets the alignment of a stop");
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.run(cx);
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<TextRulerWords>();
        let values = cx.value().cloned().unwrap_or_default();
        let numbers = cx.env.units();
        let text = run::ruler_text(self.unit, &numbers);
        let step = f64::from(run::snap_step(&text, self.unit));
        let range = 0.0..=f64::from(self.width);
        for item in Item::all(&values) {
            let at = f64::from(item.position(&values, self.width));
            let shown = text.format_value(&numbers, at, 0.0);
            let node = access.node(item.part(), Role::SLIDER);
            let kind = Self::item_name(&words, item, &values);
            node.name = Name::Text(if matches!(item, Item::Stop(_)) {
                words.stop.fill(&[("kind", kind), ("position", shown.as_str())])
            } else {
                kind.to_owned()
            });
            node.numeric = Some(Numeric::new(at, range.clone()).step(step).text(shown));
        }
        for (index, position) in self.columns.iter().enumerate() {
            let at = f64::from(*position);
            let node = access.node(COLUMN.at(index as u64), Role::SPLITTER);
            node.name = Name::Text(words.table_column.as_str().to_owned());
            node.numeric = Some(Numeric::new(at, range.clone()).step(step).text(text.format_value(&numbers, at, 0.0)));
        }
        if let Some(kind) = self.kind {
            let name = Self::tab_name(&words, kind);
            let node = access.node(SELECTOR, Role::BUTTON);
            node.name = Name::Text(words.tab_selector.fill(&[("kind", name)]));
            node.description = Some(name.to_owned());
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        self.paint_ruler(ui, cx);
    }

    fn finish(self, cx: Cx<'_, Self>) -> TextRulerOutput {
        let Report { picked, open_tabs_dialog, column, kind } = self.report;
        TextRulerOutput { edit: cx.edit_response(), picked, open_tabs_dialog, column, kind }
    }
}
