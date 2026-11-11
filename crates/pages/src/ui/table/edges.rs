//! `EdgePicker`: the diagram of a cell range's (or a paragraph's) six edges, as InDesign's Stroke proxy in the Table
//! and Cell Styles panels and Word's Borders dialog show it: top, bottom, left, right, inside horizontal and inside
//! vertical, each line on or off; and Word's border presets (All, None, Outside, Inside, one edge) as menu items.

use craft_behavior::{Metrics, NavShape, PartKey, PartRef, PressPhase, Roving, Sense};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Input, MeasureCx, Name, PaintUi, PartsCx, Pointer as _, Props as _,
    Remember as _, Role, SetPosition, Styled as _, WordSetCx as _,
};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::{ActivateKeys, KeyUse};
use craft_ui::style::{ComponentStyle, Flags, FocusLook, Stateful, StyleColor, StyleCx, SurfaceKind, SurfaceLook};
use egui::{CornerRadius, Pos2, Rect, Vec2, pos2};

const EDGE: PartKey = PartKey::new("edge");
const SQUARE: PartKey = PartKey::new("square");

craft_ui::words! {
    /// The edges' and presets' names (Word's border menu, InDesign's proxy tooltips).
    #[non_exhaustive]
    pub struct EdgePickerWords {
        top: "edgePicker.top" = "Top Border";
        bottom: "edgePicker.bottom" = "Bottom Border";
        left: "edgePicker.left" = "Left Border";
        right: "edgePicker.right" = "Right Border";
        inside_horizontal: "edgePicker.insideHorizontal" = "Inside Horizontal Border";
        /// The paragraph picker's name for the inside horizontal line (DesignCraft).
        between: "edgePicker.between" = "Between";
        inside_vertical: "edgePicker.insideVertical" = "Inside Vertical Border";
        all: "edgePicker.all" = "All Borders";
        none: "edgePicker.none" = "No Border";
        outside: "edgePicker.outside" = "Outside Borders";
        inside: "edgePicker.inside" = "Inside Borders";
    }
}

/// One of the six edges, in the picker's order (DesignCraft's `EDGE_NAMES`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EdgeKey {
    /// The top edge.
    Top,
    /// The bottom edge.
    Bottom,
    /// The left edge.
    Left,
    /// The right edge.
    Right,
    /// The line between rows.
    InsideHorizontal,
    /// The line between columns.
    InsideVertical,
}

impl EdgeKey {
    /// The six edges in the picker's order.
    pub const ALL: [EdgeKey; 6] = [EdgeKey::Top, EdgeKey::Bottom, EdgeKey::Left, EdgeKey::Right, EdgeKey::InsideHorizontal, EdgeKey::InsideVertical];

    /// The name commands take (`top`, `bottom`, `left`, `right`, `innerHorizontal`, `innerVertical`).
    pub fn id(self) -> &'static str {
        match self {
            EdgeKey::Top => "top",
            EdgeKey::Bottom => "bottom",
            EdgeKey::Left => "left",
            EdgeKey::Right => "right",
            EdgeKey::InsideHorizontal => "innerHorizontal",
            EdgeKey::InsideVertical => "innerVertical",
        }
    }
}

/// Which edges are chosen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BorderEdges([bool; 6]);

impl BorderEdges {
    /// Every edge chosen.
    pub const ALL: BorderEdges = BorderEdges([true; 6]);

    /// Whether `edge` is chosen.
    pub fn get(self, edge: EdgeKey) -> bool {
        self.0[edge as usize]
    }

    /// Chooses or clears `edge`.
    pub fn set(&mut self, edge: EdgeKey, on: bool) {
        self.0[edge as usize] = on;
    }
}

/// Word's border presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BorderPreset {
    /// Every edge.
    All,
    /// No edge.
    None,
    /// The four outer edges.
    Outside,
    /// The two inner lines.
    Inside,
    /// One edge, toggled.
    Edge(EdgeKey),
}

impl BorderPreset {
    /// Word's menu order: All, No, Outside, Inside, Top, Bottom, Left, Right, Inside Horizontal, Inside Vertical.
    pub const MENU: [BorderPreset; 10] = [
        BorderPreset::All,
        BorderPreset::None,
        BorderPreset::Outside,
        BorderPreset::Inside,
        BorderPreset::Edge(EdgeKey::Top),
        BorderPreset::Edge(EdgeKey::Bottom),
        BorderPreset::Edge(EdgeKey::Left),
        BorderPreset::Edge(EdgeKey::Right),
        BorderPreset::Edge(EdgeKey::InsideHorizontal),
        BorderPreset::Edge(EdgeKey::InsideVertical),
    ];

    /// The edges after this preset: All, None, Outside and Inside set theirs (Outside and Inside leave the other group
    /// as it is); one edge toggles.
    // wordcraft: crates/ui-egui/src/dialogs/dialogs_table_style.rs:217 `pick_edges`.
    pub fn apply(self, edges: BorderEdges) -> BorderEdges {
        let mut out = edges;
        match self {
            BorderPreset::All => out = BorderEdges::ALL,
            BorderPreset::None => out = BorderEdges::default(),
            BorderPreset::Outside => OUTER.iter().for_each(|&e| out.set(e, true)),
            BorderPreset::Inside => INNER.iter().for_each(|&e| out.set(e, true)),
            BorderPreset::Edge(edge) => out.set(edge, !edges.get(edge)),
        }
        out
    }

    /// The preset's name in the border menu.
    pub fn label(self, words: &EdgePickerWords) -> &Label {
        match self {
            BorderPreset::All => &words.all,
            BorderPreset::None => &words.none,
            BorderPreset::Outside => &words.outside,
            BorderPreset::Inside => &words.inside,
            BorderPreset::Edge(edge) => EdgePicker::edge_label(words, edge, false),
        }
    }
}

const OUTER: [EdgeKey; 4] = [EdgeKey::Top, EdgeKey::Bottom, EdgeKey::Left, EdgeKey::Right];
const INNER: [EdgeKey; 2] = [EdgeKey::InsideHorizontal, EdgeKey::InsideVertical];

/// The style of an edge picker.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct EdgePickerStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(surface)]
    surface: SurfaceLook,
    #[style(focus)]
    focus: FocusLook,
    /// The side of the square.
    side: f32,
    /// How far the outer lines sit inside the square, and how near a line the pointer picks it.
    inset: f32,
    /// The area inside the lines.
    fill: StyleColor,
    /// A chosen line, and a line under the pointer.
    on: Stateful<StyleColor>,
    /// A line that is not chosen.
    off: Stateful<StyleColor>,
}

/// The picker, editing a [`BorderEdges`] in place.
///
/// - A square with a field border, filled inside the inset; the outer rectangle at the inset and the two inner lines
///   through its middle. Chosen lines are thick and in the accent colour, others thin and faint; the line under the
///   pointer is drawn as chosen (InDesign's proxy).
/// - The pointer picks the nearest line within the inset.
/// - Click toggles a line. Double-click on a line picks its group alone (an outer line: the four outer lines on, the
///   inner off; an inner line the other way round). Triple-click turns all on, or all off when all were on
///   (InDesign's proxy).
/// - [`paragraph`](Self::paragraph): no inside vertical line, and the inside horizontal is "Between" paragraphs
///   (Word's paragraph borders).
/// - Keys: one Tab stop over the six lines (check boxes); the arrows move between them as the diagram lies, Space
///   toggles. The edges are the panel's choice of which lines a stroke applies to (view state, not the document).
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub struct EdgePicker<'a> {
    #[common]
    common: Common,
    edges: Bind<'a, BorderEdges>,
    /// Paragraph borders: no inside vertical line, and the inside horizontal is the line between paragraphs.
    #[style_key]
    #[prop(flag)]
    paragraph: bool,
}

/// The click run on the picker: the line the last click hit and the choice before the run's first click.
#[derive(Clone, Copy, Debug, Default)]
struct ClickRun {
    last: Option<EdgeKey>,
    start: BorderEdges,
}

impl<'a> EdgePicker<'a> {
    /// A picker named `label` editing `edges`.
    pub fn new(label: impl Into<Label>, edges: Bind<'a, BorderEdges>) -> Self {
        EdgePicker { common: Common::new(), edges, paragraph: false }.label(label).ltr_axis()
    }

    /// The name of `edge`.
    pub fn edge_label(words: &EdgePickerWords, edge: EdgeKey, paragraph: bool) -> &Label {
        match edge {
            EdgeKey::Top => &words.top,
            EdgeKey::Bottom => &words.bottom,
            EdgeKey::Left => &words.left,
            EdgeKey::Right => &words.right,
            EdgeKey::InsideHorizontal if paragraph => &words.between,
            EdgeKey::InsideHorizontal => &words.inside_horizontal,
            EdgeKey::InsideVertical => &words.inside_vertical,
        }
    }

    /// The lines drawn: all six, without the inside vertical in a paragraph.
    pub fn visible(paragraph: bool) -> Vec<EdgeKey> {
        EdgeKey::ALL.into_iter().filter(|&k| !(paragraph && k == EdgeKey::InsideVertical)).collect()
    }

    /// The edges after a click run's latest click on the lines `keys`: `hit` is the line it landed on, `last` the line
    /// of the click before it and `start` the choice before the run's first click. A double click counts only on the
    /// line of the click before it; a triple click decides from `start`. `None` when the click changes nothing.
    pub fn after_click(
        edges: BorderEdges,
        start: BorderEdges,
        keys: &[EdgeKey],
        hit: Option<EdgeKey>,
        last: Option<EdgeKey>,
        clicks: u32,
    ) -> Option<BorderEdges> {
        let mut out = edges;
        if clicks >= 3 {
            let all = keys.iter().all(|&k| start.get(k));
            keys.iter().for_each(|&k| out.set(k, !all));
            return Some(out);
        }
        let hit = hit?;
        if clicks == 2 && last == Some(hit) {
            let outer = OUTER.contains(&hit);
            for &k in keys {
                out.set(k, OUTER.contains(&k) == outer);
            }
        } else {
            out.set(hit, !edges.get(hit));
        }
        Some(out)
    }

    fn part(key: EdgeKey) -> PartRef {
        EDGE.at(key as u64)
    }

    fn band(lines: &[(Pos2, Pos2); 6], key: EdgeKey, inset: f32) -> Rect {
        let (a, b) = lines[key as usize];
        Rect::from_two_pos(a, b).expand(inset / 2.0)
    }
}

/// The six lines (start, end) in [`EdgeKey::ALL`] order: the outer rectangle at `inset` and the two inner lines
/// through its middle.
fn edge_lines(rect: Rect, inset: f32) -> [(Pos2, Pos2); 6] {
    let i = rect.shrink(inset);
    let c = i.center();
    [
        (i.left_top(), i.right_top()),
        (i.left_bottom(), i.right_bottom()),
        (i.left_top(), i.left_bottom()),
        (i.right_top(), i.right_bottom()),
        (pos2(i.left(), c.y), pos2(i.right(), c.y)),
        (pos2(c.x, i.top()), pos2(c.x, i.bottom())),
    ]
}

/// The line nearest `p` among `keys`, if one is within `reach`.
fn line_at(lines: &[(Pos2, Pos2); 6], keys: &[EdgeKey], p: Pos2, reach: f32) -> Option<EdgeKey> {
    keys.iter()
        .map(|&key| {
            let (a, b) = lines[key as usize];
            let t = ((p - a).dot(b - a) / (b - a).length_sq()).clamp(0.0, 1.0);
            (key, p.distance(a + (b - a) * t))
        })
        .filter(|&(_, distance)| distance <= reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(key, _)| key)
}

impl Component for EdgePicker<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_activate(ActivateKeys::Both);
    type Style = EdgePickerStyle;
    type Output = EditResponse<BorderEdges>;

    fn style(cx: &StyleCx<'_>, _: EdgePickerStyleKey) -> EdgePickerStyle {
        let sunken = cx.surface(SurfaceKind::Sunken);
        let colors = &cx.tokens.color;
        let edge = Stateful::new(cx.color(colors.line.control_edge));
        EdgePickerStyle {
            metrics: Metrics::new(),
            surface: SurfaceLook::new(Stateful::new(cx.color(colors.surface.sunken)))
                .kind(SurfaceKind::Sunken)
                .radius(sunken.radius)
                .border(edge, cx.tokens.stroke.thin),
            focus: cx.focus_look(),
            side: cx.tokens.size.thumb.sm,
            inset: cx.tokens.space.pad.regular,
            fill: cx.color(colors.surface.sunken),
            on: Stateful::new(cx.color(colors.accent.solid)),
            off: Stateful::new(cx.color(colors.text.faint)),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(cx.style.side)
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let inset = cx.style.inset;
        let lines = edge_lines(rect, inset);
        cx.part(SQUARE, rect).sense(Sense::CLICK).pointer_only("each line is a part of its own: the arrows move between them and Space toggles");
        let chosen = cx.value().copied().unwrap_or_default();
        for key in Self::visible(self.paragraph) {
            let part = cx.part(Self::part(key), Self::band(&lines, key, inset)).sense(Sense::CLICK | Sense::FOCUS).focusable();
            if chosen.get(key) {
                part.flags(Flags::CHECKED);
            }
        }
        cx.roving(Roving::new(NavShape::Spatial));
    }

    fn bind(&mut self) -> Option<BindMut<'_, BorderEdges>> {
        Some(self.edges.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(&current) = cx.value() else { return };
        let keys = Self::visible(self.paragraph);
        let lines = edge_lines(cx.rect, cx.style.inset);
        // A click lands on a line's part or, off every line, on the square; the line it picks is the nearest.
        let click =
            std::iter::once(PartRef::from(SQUARE)).chain(keys.iter().map(|&key| Self::part(key))).find_map(|part| match cx.part(part).press.phase {
                PressPhase::Release { start, pos, click: true, .. } => Some((start.clicks, line_at(&lines, &keys, pos, cx.style.inset))),
                _ => None,
            });
        let next = if let Some((clicks, hit)) = click {
            let run = *cx.memory::<ClickRun>();
            let start = if clicks >= 2 { run.start } else { current };
            *cx.memory::<ClickRun>() = ClickRun { last: hit, start };
            Self::after_click(current, start, &keys, hit, run.last, clicks)
        } else {
            keys.iter().find(|&&key| cx.part_activated(Self::part(key)).is_some()).map(|&key| {
                let mut edges = current;
                edges.set(key, !current.get(key));
                edges
            })
        };
        if let Some(edges) = next.filter(|edges| *edges != current) {
            cx.set(edges);
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<EdgePickerWords>();
        let keys = Self::visible(self.paragraph);
        let lines = edge_lines(cx.rect, cx.style.inset);
        for (at, &key) in keys.iter().enumerate() {
            let node = access.node(Self::part(key), Role::CHECK_BOX);
            node.name = Name::Text(cx.env.resolve(Self::edge_label(&words, key, self.paragraph)).as_str().to_owned());
            node.position = Some(SetPosition::new(at).of(keys.len()));
            node.bounds(Self::band(&lines, key, cx.style.inset));
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(&edges) = cx.value() else { return };
        let keys = Self::visible(self.paragraph);
        let lines = edge_lines(cx.rect, cx.style.inset);
        let draw = ui.draw();
        draw.fill(&PaintShape::Rect(cx.rect.shrink(cx.style.inset), CornerRadius::ZERO), cx.style.fill);
        let hovered = cx.hover_pos(SQUARE).and_then(|p| line_at(&lines, &keys, p, cx.style.inset));
        for key in keys {
            let (a, b) = lines[key as usize];
            let (width, color) = if edges.get(key) || hovered == Some(key) {
                (StrokeWidth::Thick, cx.resolve(&cx.style.on))
            } else {
                (StrokeWidth::Thin, cx.resolve(&cx.style.off))
            };
            draw.stroke(&PaintShape::Path(vec![a, b]), width, color);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<BorderEdges> {
        cx.edit_response()
    }
}
