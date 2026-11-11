//! `Tab`: one tab of a strip, a label with an optional icon, detail, modified mark, close button and progress bar.
//!
//! Selection, the lead, the focus ring and the drag ghost belong to the collection that shows the tab; the tab draws
//! the look the item step gives it and reports what its own parts do. The strip gives the tab a rect: the tab's
//! measure is its natural width (`Proposal::Min` the narrowest it shrinks to), and a narrower rect cuts the label
//! first, then the detail, and shows the whole text as a tooltip.

use super::rules::{CloseSide, TabKind, close_side};
use super::words::TabWords;
use crate::buttons::IconButton;
use crate::contract::{
    Access, Common, Component, Cx, ItemOutput, MeasureCx, Name, NoValue, Outcome, PaintUi, PartsCx, Pointer as _, Props as _, Role, SetPosition,
    Styled as _, WordSetCx as _,
};
use crate::display::Progress;
use crate::primitives::{IconSpec, PaintShape, StrokeWidth, TextRun};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, Corners, Flags, IconLook, RuleKind, StyleColor, StyleCx, SurfaceLook, TextLook, UiTokens};
use craft_behavior::{Flow, IconId, Insets, Metrics, PartKey, PointerButton, PressPhase, Proposal, Sense};
use craft_theme::{Rgba, RowStyle, Stateful, States, TypeRole};
use egui::{Pos2, Rect, Response, Vec2};

#[cfg(feature = "testing")]
mod stories;

/// What a tab's own parts report, besides what the collection reads from the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TabPartEvent {
    /// The close button was pressed, or the tab was middle-clicked where the kind closes on a middle click.
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

const CLOSE_REASON: &str = "The Close command closes the focused tab";

/// The sizes a tab takes from its kind and the tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sizes {
    gap: f32,
    icon: f32,
    close_box: f32,
    dot: f32,
    /// Room after a text tab's contents.
    tail: f32,
    /// The narrowest content width: what a shrinking tab keeps.
    min_content: f32,
    label_line: f32,
    detail_line: f32,
    underline_gap: f32,
    text_only: bool,
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
    /// The label: the text colour on the selected tab, muted on the others, also under the pointer.
    #[style(text)]
    label: TextLook,
    /// The detail, which stays readable while the label is cut.
    #[style(text)]
    detail: TextLook,
    /// The icon before the label, in the label's ink.
    #[style(icon)]
    icon: IconLook,
    /// Under the selected text tab's label.
    underline: StyleColor,
    /// A divider at the tab's trailing edge, in a theme that divides rows.
    divider: bool,
    close_side: CloseSide,
    sizes: Sizes,
}

/// Where the parts of a tab lie in its content rect.
#[derive(Clone, Copy, Debug)]
struct Placed {
    close: Option<Rect>,
    icon: Option<Rect>,
    label: Rect,
    detail: Option<Rect>,
    progress: Option<Rect>,
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
    /// Large Tabs: the strip's taller form with the body type.
    #[style_key]
    #[prop(flag)]
    large: bool,
    /// Secondary text after the label (a colour mode, a zoom).
    #[prop(into)]
    detail: Option<Label>,
    /// An icon before the label.
    #[prop(into)]
    icon: Option<IconSpec<'a>>,
    /// Unsaved changes: a dot where the close button is, until the pointer is over the tab.
    #[prop(flag)]
    modified: bool,
    /// Whether the tab can be closed; by default whether the kind shows a close button.
    #[prop]
    closable: Option<bool>,
    /// A progress bar along the bottom edge (a document opening or saving).
    #[prop(quantity = "fraction")]
    progress: Option<f32>,
    /// The close button's name and tooltip in place of "Close {tab}" (a tab still opening names it "Cancel opening").
    #[prop(into)]
    close_label: Option<Label>,
    /// The tab's place in its set when it is shown alone; inside a strip the item step gives it.
    #[prop]
    position: Option<SetPosition>,
    placed: Option<Placed>,
    event: Option<TabPartEvent>,
}

impl<'a> Tab<'a> {
    /// A tab named `label`, which is its text and its accessible name.
    pub fn new(label: impl Into<Label>) -> Self {
        Tab {
            common: Common::new(),
            kind: TabKind::Document,
            large: false,
            detail: None,
            icon: None,
            modified: false,
            closable: None,
            progress: None,
            close_label: None,
            position: None,
            placed: None,
            event: None,
        }
        .label(label)
    }

    /// Whether the tab can be closed.
    fn can_close(&self) -> bool {
        self.closable.unwrap_or(self.kind.shows_close())
    }

    /// Whether the tab shows a close button.
    fn shows_close(&self) -> bool {
        self.can_close() && self.kind.shows_close()
    }

    /// Whether the end slot (the close button's place) is used: by the button or by the modified dot.
    fn has_slot(&self) -> bool {
        self.shows_close() || self.modified
    }

    fn detail_text(&self) -> Option<&str> {
        self.detail.as_ref().map(Label::as_str).filter(|text| !text.is_empty())
    }

    fn label_run<'t>(&self, style: &TabStyle, text: &'t str) -> TextRun<'t> {
        TextRun::new(text).role(style.label.role)
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
        let c = |rgba: Rgba| cx.color(rgba);
        let clear = Rgba::TRANSPARENT;
        let role = if text_only || key.large { TypeRole::Body } else { TypeRole::Label };
        let line = |role: TypeRole| tokens.typography.get(role).line_height;
        let height = if text_only {
            line(role) + space.gap.tight + stroke.thick
        } else if key.large {
            size.bar.title
        } else if key.kind == TabKind::Panel {
            size.panel_header
        } else {
            size.bar.tab
        };
        let pad_x = if text_only { 0.0 } else { 2.0 * space.pad.control };
        let panel = colors.surface.panel;
        let fill = if text_only {
            Stateful::new(c(clear))
        } else {
            Stateful::new(c(clear)).off(States::new(c(clear), c(panel), c(panel), c(clear))).on(States::new(c(panel), c(panel), c(panel), c(panel)))
        };
        let rounded = !text_only && tokens.shape.rows == RowStyle::Rounded;
        let mut surface = SurfaceLook::new(fill);
        if rounded {
            surface = surface.radius(UiTokens::corner(tokens.shape.radius.control)).corners(Corners::Top);
        }
        let disabled = c(colors.text.disabled);
        let ink = |off: Rgba, on: Rgba| {
            Stateful::new(c(off)).off(States::new(c(off), c(off), c(off), disabled)).on(States::new(c(on), c(on), c(on), disabled))
        };
        let sizes = Sizes {
            gap: space.gap.inline,
            icon: size.icon.sm,
            close_box: size.icon.md,
            dot: size.handle.sm,
            tail: if text_only { space.gap.section } else { 0.0 },
            min_content: size.icon.md,
            label_line: line(role),
            detail_line: line(TypeRole::Caption),
            underline_gap: space.gap.tight,
            text_only,
        };
        TabStyle {
            surface,
            metrics: Metrics::new().padding(Insets::new(pad_x, pad_x, 0.0, 0.0)).min_size(Vec2::new(0.0, height)).gap(sizes.gap),
            label: TextLook::new(role, ink(colors.text.muted, colors.text.normal)),
            detail: TextLook::new(TypeRole::Caption, ink(colors.text.faint, colors.text.faint)),
            icon: IconLook::new(sizes.icon, ink(colors.text.muted, colors.text.normal)),
            underline: c(colors.text.normal),
            divider: key.kind.compact_dividers() && tokens.shape.rows == RowStyle::Divided,
            close_side: close_side(cx.os, cx.direction.is_rtl()),
            sizes,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let sizes = &cx.style.sizes;
        if cx.proposal == Proposal::Min {
            return Vec2::new(sizes.min_content, sizes.label_line);
        }
        let width = |run: &TextRun<'_>| cx.layout_at(run, f32::INFINITY).size().x;
        let mut natural = sizes.tail + width(&self.label_run(cx.style, cx.resolved.label));
        if self.icon.is_some() {
            natural += sizes.icon + sizes.gap;
        }
        if let Some(detail) = self.detail_text() {
            natural += sizes.gap + width(&TextRun::new(detail).role(cx.style.detail.role));
        }
        if self.has_slot() {
            natural += sizes.gap + sizes.close_box;
        }
        Vec2::new(natural, sizes.label_line)
    }

    fn sense(&self) -> Sense {
        if self.can_close() && self.kind.middle_click_closes() {
            Sense::CLICK | Sense::MIDDLE_CLICK | Sense::FOCUS
        } else {
            Sense::CLICK | Sense::FOCUS
        }
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let sizes = cx.style.sizes;
        let rtl = cx.flow.is_rtl();
        let content = cx.content_rect;
        let mut flow = Flow::new(content, rtl);
        let mut close = None;
        if self.has_slot() {
            let at_start = cx.style.close_side == if rtl { CloseSide::Right } else { CloseSide::Left };
            let (slot, rest) = if at_start { flow.take_start(sizes.close_box) } else { flow.take_end(sizes.close_box) };
            close = Some(slot);
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
        let natural = |run: &TextRun<'_>| cx.layout_at(run, f32::INFINITY).size().x;
        let label_natural = natural(&self.label_run(cx.style, cx.resolved.label));
        let detail_natural = self.detail_text().map_or(0.0, |text| natural(&TextRun::new(text).role(cx.style.detail.role)));
        let detail_gap = if detail_natural > 0.0 { sizes.gap } else { 0.0 };
        let label_width = label_natural.min((band.width() - detail_gap - detail_natural).max(0.0));
        let detail_width = detail_natural.min((band.width() - label_width - detail_gap).max(0.0));
        let line = |rect: Rect, height: f32| {
            let top = if sizes.text_only { content.top() } else { content.center().y - height / 2.0 };
            Rect::from_x_y_ranges(rect.x_range(), top..=top + height)
        };
        let (label, rest) = flow.take_start(label_width);
        let detail = (detail_width > 0.0).then(|| line(rest.take_start(detail_gap).1.take_start(detail_width).0, sizes.detail_line));
        let label = line(label, sizes.label_line);
        let cut = label_width < label_natural || detail_width < detail_natural;
        if cut {
            let full = match self.detail_text() {
                Some(detail) => format!("{}, {detail}", cx.resolved.label),
                None => cx.resolved.label.to_owned(),
            };
            let _ = cx.part(TEXT, band).tooltip(full);
        }
        if let Some(slot) = close.filter(|_| self.shows_close()) {
            let name = self.close_label.clone();
            let part = cx.part(CLOSE, slot).sense(Sense::CLICK).pointer_only(CLOSE_REASON);
            if let Some(name) = name {
                let _ = part.tooltip(name);
            }
        }
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
        self.placed = Some(Placed { close, icon, label, detail, progress });
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let pressed = self.shows_close() && cx.part_activated(CLOSE).is_some();
        let middle = self.can_close()
            && self.kind.middle_click_closes()
            && matches!(cx.press().phase, PressPhase::Release { start, click: true, .. } if start.button == PointerButton::Middle);
        if pressed || middle {
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
        let Some(placed) = self.placed.take() else { return };
        let (style, rect) = (cx.style, cx.rect);
        let sizes = style.sizes;
        let selected = cx.resolved.selected == Some(true) || cx.state().flags.contains(Flags::SELECTED);
        let draw = ui.draw();
        if style.divider {
            let x = cx.flow.end_x();
            draw.rule(RuleKind::Divider, Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom()));
        }
        if sizes.text_only && selected {
            let y = placed.label.bottom() + sizes.underline_gap;
            let line = PaintShape::Path(vec![Pos2::new(placed.label.left(), y), Pos2::new(placed.label.right(), y)]);
            draw.stroke(&line, StrokeWidth::Thick, style.underline);
        }
        let label = self.label_run(style, cx.resolved.label).color(cx.resolve(&style.label.color)).elide();
        draw.text(placed.label, &label);
        if let (Some(at), Some(detail)) = (placed.detail, self.detail_text()) {
            draw.text(at, &TextRun::new(detail).role(style.detail.role).color(cx.resolve(&style.detail.color)).elide());
        }
        if let (Some(at), Some(icon)) = (placed.icon, &mut self.icon) {
            if icon.color.is_none() {
                icon.color = Some(cx.resolve(&style.icon.color));
            }
            ui.draw().icon(at, icon);
        }
        if let Some(slot) = placed.close {
            let over = cx.response.hovered();
            if self.shows_close() && (over || !self.modified) {
                let words = cx.words::<TabWords>();
                let name = match &self.close_label {
                    Some(name) => name.as_str().to_owned(),
                    None => words.close.fill(&[("tab", cx.resolved.label)]),
                };
                ui.face(CLOSE, IconButton::new(IconId::new("close")).label(name));
            } else {
                let dot = PaintShape::Circle(slot.center(), sizes.dot / 2.0);
                ui.draw().fill(&dot, cx.resolve(&style.label.color));
            }
        }
        if let (Some(_), Some(fraction)) = (placed.progress, self.progress) {
            ui.face(PROGRESS, Progress::new(fraction).label(cx.resolved.label.to_owned()));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> TabOutput {
        TabOutput { response: cx.response.clone(), event: self.event }
    }
}
