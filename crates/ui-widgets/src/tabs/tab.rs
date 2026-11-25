//! `Tab`: one tab of a strip, a label with an optional icon, detail, modified mark, close button and progress bar.
//!
//! Selection, the lead, the focus ring and the drag ghost belong to the collection that shows the tab; the tab draws
//! the look the item step gives it and reports what its own parts do. The strip gives the tab a rect: the tab's
//! measure is its natural width (`Proposal::Min` the room for the first letters of its title beside its icon and close
//! button), and a narrower rect cuts the label first, then the detail, and shows the whole text as a tooltip.

use super::rules::{CloseSide, PanelTabLook, TabCloseLook, TabKind, close_side};
use super::words::TabWords;
use std::borrow::Cow;

/// The width, in digits of the label role, a document tab shows of its name at most.
const MAX_NAME_DIGITS: usize = 22;
use crate::buttons::IconButton;
use crate::buttons::icon_button::{MarkerStyle, paint_marker};
use crate::contract::{
    Access, Common, Component, Cx, ItemOutput, MeasureCx, Name, NoValue, Outcome, PaintUi, PartsCx, Pointer as _, Props as _, Role, SetPosition,
    Styled as _, WordSetCx as _,
};
use crate::display::Progress;
use crate::primitives::{IconSpec, PaintShape, RectSide, StrokeWidth, TextRun, Wrap};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, Flags, IconLook, RuleKind, Span, StyleColor, StyleCx, SurfaceLook, TextLook, UiTokens};
use craft_behavior::{Flow, IconId, Insets, Metrics, PartKey, Proposal, Sense};
use craft_theme::{Contrast, Rgba, Stateful, States, TypeRole};
use egui::{Pos2, Rect, Response, Vec2};

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The role of a tab in its strip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TabRole {
    /// An ordinary tab.
    #[default]
    Standard,
    /// A tab that appears with a selection: it takes the accent text colour, enters with motion and leaves through `Presence`.
    Contextual,
}

/// What a tab's own parts report, besides what the collection reads from the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TabPartEvent {
    /// The close button was pressed. A middle click is the strip's (`CollectionEvent::MiddleClick`).
    Close,
}

/// What a tab returns.
#[derive(Debug)]
#[non_exhaustive]
pub struct TabOutput {
    /// The tab's response.
    pub response: Response,
    /// What the tab's parts reported this pass.
    pub event: Option<TabPartEvent>,
}

impl Outcome for TabOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl ItemOutput<TabPartEvent> for TabOutput {
    fn into_event(self) -> Option<TabPartEvent> {
        self.event
    }
}

const TEXT: PartKey = PartKey::new("text");
const CLOSE: PartKey = PartKey::new("close");
const PROGRESS: PartKey = PartKey::new("progress");

/// How many of a title's first letters a shrunk tab still shows.
const MIN_LETTERS: usize = 3;

/// The modified mark of a tab whose look shows no marker dot for it.
const MARK: &str = "*";
const CLOSE_REASON: &str = "The Close command closes the focused tab";

/// The padding on each side of a ribbon tab's label: the strip insets its sliding underline by it, so the underline
/// runs under the label.
pub(super) fn ribbon_pad(tokens: &UiTokens) -> f32 {
    tokens.space.gap.item + tokens.space.gap.tight
}

/// The sizes a tab takes from its kind and the tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sizes {
    gap: f32,
    icon: f32,
    close_box: f32,
    /// Room after a text tab's contents.
    tail: f32,
    label_line: f32,
    detail_line: f32,
    underline_gap: f32,
    /// The height of the selected tab's indicator bar.
    indicator: f32,
    text_only: bool,
    /// Whether the label stands in the middle of the tab's height: a ribbon tab, whose strip is a bar.
    middle: bool,
}

/// The look of a tab.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TabStyle {
    /// The fill of the selected tab and of a tab under the pointer; a text tab has none.
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
    /// The label: the text colour on the selected tab, muted on the others, also under the pointer. A ribbon tab's
    /// label is in the accent text colour when shown or contextual, and measures at the shown tab's semibold type.
    #[style(text)]
    label: TextLook,
    /// The label's type on a tab that is not shown: the ribbon's regular weight, the label's own type elsewhere.
    idle_role: TypeRole,
    /// The detail, which stays readable while the label is cut.
    #[style(text)]
    detail: TextLook,
    /// The icon before the label, in the label's ink.
    #[style(icon)]
    icon: IconLook,
    /// Under the selected text tab's label.
    underline: StyleColor,
    /// The bar along the selected tab's top edge: the accent in high contrast, none otherwise and on text tabs.
    indicator: Option<StyleColor>,
    /// A divider at the tab's trailing edge.
    divider: bool,
    /// The pill a panel tab in [`PanelTabLook::Pill`] shows under its label in place of the surface.
    pill: Option<Pill>,
    close_side: CloseSide,
    /// The close glyph's side and its ink at rest and under the pointer under [`TabCloseLook::Glyph`]; none for the icon
    /// button.
    close_glyph: Option<(f32, StyleColor, StyleColor)>,
    sizes: Sizes,
    /// Document tabs in [`DocumentTabLook::Shrinking`](super::DocumentTabLook::Shrinking): a name capped at
    /// `MAX_NAME_DIGITS` digits' width, a readable prefix at their least, and the marker dot for a modified tab.
    shrinking: bool,
    /// Document tabs in [`DocumentTabLook::Semibold`](super::DocumentTabLook::Semibold): the label and its `*` in the
    /// role's semibold face.
    strong: bool,
    /// The marker dot's diameter.
    marker: f32,
    /// The marker dot of a marked tab.
    marker_color: StyleColor,
    /// The ring around the marker dot.
    marker_edge: StyleColor,
}

/// The pill of a panel tab in [`PanelTabLook::Pill`]: its fill in each state, its corners and its height, in the
/// middle of the tab's height.
#[derive(Clone, Debug, PartialEq)]
struct Pill {
    fill: Stateful<StyleColor>,
    radius: egui::CornerRadius,
    height: f32,
}

/// `text` as a run of the label: in the role's semibold face where the look sets the label strong.
fn label_text<'t>(style: &TabStyle, text: impl Into<Cow<'t, str>>) -> TextRun<'t> {
    let span = Span::new(text);
    TextRun::spans(vec![if style.strong { span.strong() } else { span }])
}

/// Where the parts of a tab lie in its content rect.
#[derive(Clone, Copy, Debug)]
struct Placed {
    close: Option<Rect>,
    icon: Option<Rect>,
    label: Rect,
    detail: Option<Rect>,
    /// The modified mark `*`, right after the last text, where the look shows no dot for it.
    mark: Option<Rect>,
    /// The marker dot, after the last text: a modified tab's too where the look shows the dot for it.
    marker: Option<Rect>,
    /// The band the label, detail and marker share.
    band: Rect,
    /// Whether the band cuts the label or the detail.
    cut: bool,
}

/// One tab of a strip.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct Tab<'a> {
    #[common]
    common: Common,
    /// The kind of strip the tab is in: its height, label type, and whether it has a surface and a close button.
    #[style_key]
    #[prop]
    kind: TabKind,
    /// Whether the tab is a standard one or a contextual one; a contextual ribbon tab takes the accent text colour.
    #[style_key]
    #[prop]
    role: TabRole,
    /// Large Tabs: the strip's taller form with the body type.
    #[style_key]
    #[prop(flag)]
    large: bool,
    /// The look of a panel tab ([`TabKind::Panel`]); the other kinds have their own.
    #[style_key]
    #[prop]
    panel_look: PanelTabLook,
    /// Secondary text after the label (a colour mode, a zoom).
    #[prop(into)]
    detail: Option<Label>,
    /// An icon before the label.
    #[prop(into)]
    icon: Option<IconSpec<'a>>,
    /// Unsaved changes: a `*` after the detail, or after the label without one; the marker dot there under
    /// [`DocumentTabLook::Shrinking`](super::DocumentTabLook::Shrinking).
    #[prop(flag)]
    modified: bool,
    /// The length in bytes of the label's name part (a document's name before its zoom and colour mode): a modified
    /// tab under the `*` look shows the mark right after the name, inside the label.
    #[prop]
    name_len: Option<usize>,
    /// Whether the tab can be closed; by default whether the kind shows a close button.
    #[prop]
    closable: Option<bool>,
    /// A progress bar along the bottom edge (a document opening or saving).
    #[prop(quantity = "fraction")]
    progress: Option<f32>,
    /// The close button's name and tooltip in place of "Close {tab}" (a tab still opening names it "Cancel opening").
    #[prop(into)]
    close_label: Option<Label>,
    /// The marker dot after the last text: the tab's content has something to show.
    #[prop]
    marked: bool,
    /// The tab's place in its set when it is shown alone; inside a strip the item step gives it.
    #[prop]
    position: Option<SetPosition>,
    event: Option<TabPartEvent>,
}

impl<'a> Tab<'a> {
    /// A tab named `label`, which is its text and its accessible name.
    pub fn new(label: impl Into<Label>) -> Self {
        Tab {
            common: Common::new(),
            kind: TabKind::Document,
            role: TabRole::default(),
            large: false,
            panel_look: PanelTabLook::Plain,
            detail: None,
            icon: None,
            modified: false,
            name_len: None,
            closable: None,
            progress: None,
            close_label: None,
            marked: false,
            position: None,
            event: None,
        }
        .label(label)
    }

    /// Whether the tab can be closed.
    fn can_close(&self) -> bool {
        self.closable.unwrap_or(self.kind.closes_by_default())
    }

    /// Whether the tab shows a close button.
    fn shows_close(&self) -> bool {
        self.can_close() && self.kind.shows_close()
    }

    fn detail_text(&self) -> Option<&str> {
        self.detail.as_ref().map(Label::as_str).filter(|text| !text.is_empty())
    }

    fn label_run<'t>(&self, style: &TabStyle, text: &'t str) -> TextRun<'t> {
        label_text(style, text).role(style.label.role)
    }

    /// Whether the marker dot shows: the tab's content has something to show, or the document is modified and the
    /// look marks a modified tab with the dot.
    fn dotted(&self, style: &TabStyle) -> bool {
        self.marked || (self.modified && style.shrinking)
    }

    /// Where in `label` the modified mark `*` goes: after the name part, for a modified tab whose look shows no dot for
    /// it.
    fn mark_at(&self, style: &TabStyle, label: &str) -> Option<usize> {
        self.name_len.filter(|at| self.modified && !style.shrinking && label.is_char_boundary(*at))
    }

    /// Whether the modified mark `*` shows after the text: a modified tab whose look shows no dot for it, and whose
    /// label holds no name part for the mark to follow.
    fn starred(&self, style: &TabStyle, label: &str) -> bool {
        self.modified && !style.shrinking && self.mark_at(style, label).is_none()
    }

    /// The text the tab shows for `label`: the label, with the modified mark after its name part where it has one.
    fn shown_label<'t>(&self, style: &TabStyle, label: &'t str) -> Cow<'t, str> {
        match self.mark_at(style, label) {
            Some(at) => Cow::Owned(format!("{}{MARK}{}", &label[..at], &label[at..])),
            None => Cow::Borrowed(label),
        }
    }

    /// The modified mark, in the label's type.
    fn mark_run(&self, style: &TabStyle) -> TextRun<'static> {
        label_text(style, MARK).role(style.label.role)
    }

    /// Where the parts lie in `content`, with `natural` the unbounded width of a run. `parts` registers its parts
    /// here and `paint` paints here, so a tab worn as a face, which runs no `parts`, paints the same.
    fn place(&self, style: &TabStyle, content: Rect, rtl: bool, label: &str, natural: impl Fn(&TextRun<'_>) -> f32) -> Placed {
        let sizes = style.sizes;
        let mut flow = Flow::new(content, rtl);
        let mut close = None;
        if self.shows_close() {
            let at_start = style.close_side == if rtl { CloseSide::Right } else { CloseSide::Left };
            let (slot, rest) = if at_start { flow.take_start(sizes.close_box) } else { flow.take_end(sizes.close_box) };
            // The glyph takes clicks in a square of the button's width, not the content's whole height.
            close = Some(if style.close_glyph.is_some() { Rect::from_center_size(slot.center(), Vec2::splat(sizes.close_box)) } else { slot });
            flow = if at_start { rest.take_start(sizes.gap).1 } else { rest.take_end(sizes.gap).1 };
        }
        flow = flow.take_end(sizes.tail).1;
        let mut icon = None;
        if self.icon.is_some() {
            let (slot, rest) = flow.take_start(sizes.icon);
            icon = Some(Rect::from_center_size(slot.center(), Vec2::splat(sizes.icon)));
            flow = rest.take_start(sizes.gap).1;
        }
        let band = flow.rect();
        let label_natural = natural(&self.label_run(style, label));
        let detail_natural = self.detail_text().map_or(0.0, |text| natural(&TextRun::new(text).role(style.detail.role)));
        // The marks keep their room; the label is cut first, then the detail.
        let mark_width = if self.starred(style, label) { natural(&self.mark_run(style)) } else { 0.0 };
        let marker_width = if self.dotted(style) { sizes.gap + style.marker } else { 0.0 };
        let marks = mark_width + marker_width;
        let detail_gap = if detail_natural > 0.0 { sizes.gap } else { 0.0 };
        let label_width = label_natural.min((band.width() - marks - detail_gap - detail_natural).max(0.0));
        let detail_width = detail_natural.min((band.width() - marks - label_width - detail_gap).max(0.0));
        let line = |rect: Rect, height: f32| {
            let top = if sizes.text_only && !sizes.middle { content.top() } else { content.center().y - height / 2.0 };
            Rect::from_x_y_ranges(rect.x_range(), top..=top + height)
        };
        let (label, rest) = flow.take_start(label_width);
        let (detail, after) = if detail_width > 0.0 {
            let (detail, after) = rest.take_start(detail_gap).1.take_start(detail_width);
            (Some(line(detail, sizes.detail_line)), after)
        } else {
            (None, rest)
        };
        let (mark, after) = if mark_width > 0.0 {
            let (mark, after) = after.take_start(mark_width);
            (Some(line(mark, sizes.label_line)), after)
        } else {
            (None, after)
        };
        let label = line(label, sizes.label_line);
        let marker = self.dotted(style).then(|| {
            let slot = after.take_start(sizes.gap).1.take_start(style.marker).0;
            Rect::from_center_size(Pos2::new(slot.center().x, label.center().y), Vec2::splat(style.marker))
        });
        let cut = label_width < label_natural || detail_width < detail_natural;
        Placed { close, icon, label, detail, mark, marker, band, cut }
    }

    fn name(&self, label: &str, words: &TabWords) -> String {
        let name = match self.detail_text() {
            Some(detail) => format!("{label}, {detail}"),
            None => label.to_owned(),
        };
        if self.modified { words.modified.fill(&[("tab", &name)]) } else { name }
    }
}

impl Component for Tab<'_> {
    const ROLE: Role = Role::TAB;
    const OVERLAYS: bool = false;
    type Style = TabStyle;
    type Output = TabOutput;

    fn style(cx: &StyleCx<'_>, key: TabStyleKey) -> TabStyle {
        let tokens = cx.tokens;
        let (colors, space, size, stroke) = (&tokens.color, &tokens.space, &tokens.size, &tokens.stroke);
        let text_only = key.kind.text_only();
        let ribbon = key.kind == TabKind::Ribbon;
        let c = |rgba: Rgba| cx.color(rgba);
        let clear = Rgba::TRANSPARENT;
        // A ribbon tab measures at the shown tab's semibold type, so showing a tab does not move its neighbours.
        let role = if ribbon {
            TypeRole::BodyStrong
        } else if text_only || key.large {
            TypeRole::Body
        } else {
            TypeRole::Label
        };
        let idle_role = if ribbon { TypeRole::Body } else { role };
        let line = |role: TypeRole| tokens.typography.get(role).line_height;
        // A ribbon tab is a bar's tab, its label in the middle of the old ribbon's 28-point strip (compact sizes) and
        // padded by an item and a tight gap on each side (10 points in the compact sizes, as the old ribbon padded them).
        let height = if ribbon {
            size.bar.tab + space.gap.tight
        } else if text_only {
            line(role) + space.gap.tight + stroke.thick
        } else if key.large {
            size.bar.tab_large
        } else if key.kind == TabKind::Panel {
            size.panel_header
        } else {
            size.bar.tab
        };
        let pad_x = if ribbon {
            ribbon_pad(tokens)
        } else if text_only {
            0.0
        } else {
            2.0 * space.pad.control
        };
        let panel = colors.surface.panel;
        let hover = c(colors.state.hover);
        let fill = if ribbon {
            Stateful::new(c(clear)).off(States::new(c(clear), hover, c(clear), c(clear))).on(States::new(c(clear), c(clear), c(clear), c(clear)))
        } else if text_only {
            Stateful::new(c(clear))
        } else if key.kind == TabKind::Panel && key.panel_look == PanelTabLook::Block {
            let control = c(colors.surface.control);
            Stateful::new(c(clear)).off(States::new(c(clear), hover, hover, c(clear))).on(States::new(control, control, control, control))
        } else {
            Stateful::new(c(clear)).off(States::new(c(clear), c(panel), c(panel), c(clear))).on(States::new(c(panel), c(panel), c(panel), c(panel)))
        };
        // A pill panel tab shows its fill as the pill instead of the surface.
        let pill = (key.kind == TabKind::Panel && key.panel_look == PanelTabLook::Pill).then(|| {
            let control = c(colors.surface.control);
            Pill {
                fill: Stateful::new(c(clear))
                    .off(States::new(c(clear), control, control, c(clear)))
                    .on(States::new(control, control, control, control)),
                radius: UiTokens::corner(tokens.shape.radius.control),
                height: size.control.sm,
            }
        });
        let fill = if pill.is_some() { Stateful::new(c(clear)) } else { fill };
        let surface = if ribbon { SurfaceLook::new(fill).radius(UiTokens::corner(tokens.shape.radius.sm)) } else { SurfaceLook::new(fill) };
        let disabled = c(colors.text.disabled);
        let ink = |off: Rgba, on: Rgba| {
            Stateful::new(c(off)).off(States::new(c(off), c(off), c(off), disabled)).on(States::new(c(on), c(on), c(on), disabled))
        };
        let label_ink = match (ribbon, key.role) {
            (true, TabRole::Contextual) => ink(colors.accent.text, colors.accent.text),
            // A ribbon look may rest its tabs in the normal text colour (`RibbonLook::tabs`).
            (true, _) if cx.env::<crate::buttons::RibbonLook>().tab_ink => ink(colors.text.normal, colors.accent.text),
            (true, _) => ink(colors.text.muted, colors.accent.text),
            _ => ink(colors.text.muted, colors.text.normal),
        };
        let sizes = Sizes {
            gap: space.gap.inline,
            icon: size.icon.sm,
            close_box: size.icon.md,
            tail: if text_only && !ribbon { space.gap.section } else { 0.0 },
            label_line: line(role),
            detail_line: line(TypeRole::Caption),
            underline_gap: space.gap.tight,
            indicator: stroke.thick,
            text_only,
            middle: ribbon,
        };
        let marker = MarkerStyle::of(cx);
        TabStyle {
            surface,
            metrics: Metrics::new().padding(Insets::new(pad_x, pad_x, 0.0, 0.0)).min_size(Vec2::new(0.0, height)).gap(sizes.gap),
            label: TextLook::new(role, label_ink.clone()),
            idle_role,
            detail: TextLook::new(TypeRole::Caption, ink(colors.text.faint, colors.text.faint)),
            icon: IconLook::new(sizes.icon, label_ink),
            underline: c(colors.text.normal),
            indicator: (!text_only && cx.contrast() == Contrast::High).then(|| c(colors.accent.solid)),
            divider: key.kind.trailing_divider(),
            pill,
            close_side: close_side(cx.os, cx.direction.is_rtl()),
            close_glyph: (cx.env::<TabCloseLook>() == TabCloseLook::Glyph).then(|| (size.icon.sm, c(colors.text.muted), c(colors.text.normal))),
            sizes,
            shrinking: key.kind.shrinks_in(cx.env::<super::DocumentTabLook>()),
            strong: key.kind == TabKind::Document && cx.env::<super::DocumentTabLook>() == super::DocumentTabLook::Semibold,
            marker: marker.side,
            marker_color: marker.color,
            marker_edge: marker.edge,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let sizes = &cx.style.sizes;
        let shown = self.shown_label(cx.style, cx.resolved.label);
        let label: &str = &shown;
        // What `place` takes before the label: the tail, the icon and the close button, each with its gap.
        let mut parts = sizes.tail;
        if self.icon.is_some() {
            parts += sizes.icon + sizes.gap;
        }
        if self.shows_close() {
            parts += sizes.gap + sizes.close_box;
        }
        // The first letters of the title: the least a shrunk tab shows.
        let letters = || {
            let first: String = label.chars().take(MIN_LETTERS).collect();
            cx.layout_at(&self.label_run(cx.style, &first), f32::INFINITY).size().x
        };
        if cx.proposal == Proposal::Min && !cx.style.shrinking {
            return Vec2::new(parts + letters(), sizes.label_line);
        }
        let width = |run: &TextRun<'_>| cx.layout_at(run, f32::INFINITY).size().x;
        let label_width = width(&self.label_run(cx.style, label));
        // A document tab never takes more than a name of `MAX_NAME_DIGITS` digits' width, as production's tabs cut a long
        // name short so the other tabs and the Open control keep their room; the label is cut with an ellipsis.
        let capped = if cx.style.shrinking { label_width.min(width(&self.label_run(cx.style, &"0".repeat(MAX_NAME_DIGITS)))) } else { label_width };
        let mut natural = parts + capped;
        if let Some(detail) = self.detail_text() {
            natural += sizes.gap + width(&TextRun::new(detail).role(cx.style.detail.role));
        }
        if self.starred(cx.style, label) {
            natural += width(&self.mark_run(cx.style));
        }
        if self.dotted(cx.style) {
            natural += sizes.gap + cx.style.marker;
        }
        let measured = if cx.proposal == Proposal::Min {
            let end = label.char_indices().nth(12).map_or(label.len(), |(index, _)| index);
            let prefix = label.get(..end).unwrap_or(label);
            let readable = natural - capped + width(&self.label_run(cx.style, prefix));
            readable.max(parts + letters()).min(natural)
        } else {
            natural
        };
        Vec2::new(measured, sizes.label_line)
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let shown = self.shown_label(cx.style, cx.resolved.label);
        let placed = self.place(cx.style, cx.content_rect, cx.flow.is_rtl(), &shown, |run| cx.layout_at(run, f32::INFINITY).size().x);
        if placed.cut {
            let full = match self.detail_text() {
                Some(detail) => format!("{}, {detail}", cx.resolved.label),
                None => cx.resolved.label.to_owned(),
            };
            // The text part's tooltip covers the tab's own over the text, so the tab's tooltip follows the full label
            // there; an uncut tab shows the tab's own tooltip alone.
            let full = match cx.resolved.tooltip_text(&cx.env) {
                Some(tip) if !tip.is_empty() => format!("{full}\n{tip}"),
                _ => full,
            };
            let _ = cx.part(TEXT, placed.band).tooltip(full);
        }
        if let Some(slot) = placed.close {
            let name = self.close_label.clone();
            let part = cx.part(CLOSE, slot).sense(Sense::CLICK).pointer_only(CLOSE_REASON);
            if let Some(name) = name {
                let _ = part.tooltip(name);
            }
        }
        let sizes = cx.style.sizes;
        let progress = self.progress.map(|_| {
            let bar = cx.face(Progress::new(0.0)).y.min(rect.height());
            Rect::from_min_max(
                Pos2::new(rect.left() + sizes.gap.min(rect.width() / 2.0), rect.bottom() - bar),
                Pos2::new(rect.right() - sizes.gap.min(rect.width() / 2.0), rect.bottom()),
            )
        });
        if let Some(bar) = progress {
            let _ = cx.part(PROGRESS, bar);
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if self.shows_close() && cx.part_activated(CLOSE).is_some() {
            self.event = Some(TabPartEvent::Close);
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<TabWords>();
        let label = cx.resolved.label;
        access.name(Name::Text(self.name(label, &words)));
        if let Some(position) = self.position {
            access.position(position);
        }
        if self.shows_close() {
            let name = match &self.close_label {
                Some(name) => name.as_str().to_owned(),
                None => words.close.fill(&[("tab", label)]),
            };
            access.node(CLOSE, Role::BUTTON).name = Name::Text(name);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (style, rect) = (cx.style, cx.rect);
        let sizes = style.sizes;
        let selected = cx.resolved.selected == Some(true) || cx.state().flags.contains(Flags::SELECTED);
        let draw = ui.draw();
        let shown = self.shown_label(style, cx.resolved.label);
        let placed = self.place(style, cx.content_rect, cx.flow.is_rtl(), &shown, |run| draw.layout(run, f32::INFINITY).size().x);
        if style.divider {
            draw.edge_rule(RuleKind::Divider, rect, RectSide::end(cx.flow.is_rtl()));
        }
        if let Some(pill) = &style.pill {
            let band = Rect::from_center_size(rect.center(), Vec2::new(rect.width(), pill.height.min(rect.height())));
            draw.fill(&PaintShape::Rect(band, pill.radius), cx.resolve(&pill.fill));
        }
        if let (Some(color), true) = (style.indicator, selected) {
            let bar = Rect::from_min_size(rect.min, Vec2::new(rect.width(), sizes.indicator.min(rect.height())));
            draw.fill(&PaintShape::Rect(bar, egui::CornerRadius::ZERO), color);
        }
        // A strip that slides its underline paints the one underline itself.
        if sizes.text_only && selected && !self.kind.slides_underline() {
            let y = placed.label.bottom() + sizes.underline_gap;
            let line = PaintShape::Path(vec![Pos2::new(placed.label.left(), y), Pos2::new(placed.label.right(), y)]);
            draw.stroke(&line, StrokeWidth::Thick, style.underline);
        }
        let role = if selected { style.label.role } else { style.idle_role };
        let label = label_text(style, &*shown).role(role).color(cx.resolve(&style.label.color));
        draw.text(placed.label, &if self.kind.clips_label() { label.wrap(Wrap::Clip) } else { label.elide() });
        if let (Some(at), Some(detail)) = (placed.detail, self.detail_text()) {
            draw.text(at, &TextRun::new(detail).role(style.detail.role).color(cx.resolve(&style.detail.color)).elide());
        }
        if let Some(at) = placed.mark {
            draw.text(at, &self.mark_run(style).color(cx.resolve(&style.label.color)));
        }
        if let Some(at) = placed.marker {
            paint_marker(&draw, at.center(), &MarkerStyle { side: style.marker, color: style.marker_color, edge: style.marker_edge });
        }
        if let (Some(at), Some(icon)) = (placed.icon, &mut self.icon) {
            if icon.color.is_none() {
                icon.color = Some(cx.resolve(&style.icon.color));
            }
            ui.draw().icon(at, icon);
        }
        if let (Some(at), Some((side, rest, hover))) = (placed.close, style.close_glyph) {
            let mut glyph = IconSpec::from(IconId::new("close"));
            glyph.color = Some(if cx.hover_pos(CLOSE).is_some() { hover } else { rest });
            ui.draw().icon(Rect::from_center_size(at.center(), Vec2::splat(side)), &glyph);
        } else if placed.close.is_some() {
            let words = cx.words::<TabWords>();
            let name = match &self.close_label {
                Some(name) => name.as_str().to_owned(),
                None => words.close.fill(&[("tab", cx.resolved.label)]),
            };
            ui.face(CLOSE, IconButton::new(IconId::new("close")).label(name));
        }
        if let Some(fraction) = self.progress {
            ui.face(PROGRESS, Progress::new(fraction).label(cx.resolved.label.to_owned()));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> TabOutput {
        TabOutput { response: cx.response.clone(), event: self.event }
    }
}
