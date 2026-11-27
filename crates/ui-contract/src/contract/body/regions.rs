//! Where the header, body, footer and end regions sit inside the container, and the rules between them.

use super::BodyHooks;
use super::scroll::scroll_of;
use crate::contract::cx::BodyCx;
use crate::contract::{Container, ContainerProps, FooterPlace, Gap, HeaderParts, LeastWidth, Mount, Pad, PadEdges, SectionRhythm, Sizing};
use crate::layout::{ColumnGroup, Columns, Reached};
use crate::primitives::{RectSide, Rule};
use crate::runtime::access::{self};
use crate::runtime::egui::{alloc, child, style};
use crate::runtime::env::{Env, RowLine};
use crate::runtime::memory::sub;
use crate::runtime::overlay::CloseReason;
use crate::style::{RuleKind, RuleLook, RuleWidth, UiTokens, rule_look};
use egui::{Align, Id, Layout, Pos2, Rect, Ui, UiBuilder, Vec2};

/// The slots of a container's own header parts at the edges of the header's line (`Container::header_parts`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct HeaderSlots {
    pub start: Option<Rect>,
    pub end: Option<Rect>,
}

/// Whether the footer sits in the end region: it asks to, and the container has one.
pub(super) fn footer_in_end(props: &ContainerProps) -> bool {
    props.end.is_some() && props.footer.is_some_and(|footer| footer.place.in_end())
}

/// Whether the body places the footer (`FooterPlace::Body`): the footer takes no band, and its size is inside the
/// body's.
pub(super) fn footer_in_body(props: &ContainerProps) -> bool {
    props.footer.is_some_and(|footer| footer.place == FooterPlace::Body)
}

/// Whether the end region takes the full height under the header: it asks to, or the footer sits in it.
pub(super) fn end_full_height(props: &ContainerProps) -> bool {
    props.end.is_some_and(|end| end.full_height) || footer_in_end(props)
}

/// The line a body shares with an inline header or an end region beside it, from what they took (`sizes`): as high
/// as the taller of the two. `None` for a body that shares no line.
pub(super) fn shared_line(props: &ContainerProps, sizes: RegionSizes) -> Option<f32> {
    let header = props.header.filter(|header| header.inline).and(sizes.header).map(|took| took.y);
    let end = props.end.filter(|_| !end_full_height(props)).and(sizes.end).map(|took| took.y);
    header.into_iter().chain(end).reduce(f32::max).filter(|line| *line > 0.0)
}

#[doc(hidden)]
pub fn pad_of(tokens: &UiTokens, pad: Pad) -> f32 {
    let pads = &tokens.tokens().space.pad;
    match pad {
        Pad::Tight => tokens.tokens().space.gap.tight,
        Pad::Control => pads.control,
        Pad::Regular => pads.regular,
        Pad::Dialog => pads.dialog,
        Pad::Roomy => pads.roomy,
        Pad::Page => pads.roomy * 2.5,
        _ => 0.0,
    }
}

/// A container's padding on each side along each axis: its tier on `edges`, none on the others.
pub(super) fn pad_vec(tokens: &UiTokens, pad: Pad, edges: PadEdges) -> Vec2 {
    let pad = pad_of(tokens, pad);
    match edges {
        PadEdges::All => Vec2::splat(pad),
        PadEdges::Block => Vec2::new(0.0, pad),
        PadEdges::Inline => Vec2::new(pad, 0.0),
    }
}

/// The room the rules between a container's regions take, in points: the divider rule's width on the display for each
/// rule the container draws, none for one it doesn't. Each rule lies in a band of its own, so it shares no pixel with
/// a region's content at any scale. Under `SectionRhythm::Spaced` the rule above a header has room around it.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct RuleBands {
    /// Before the rule above the header: what makes the gap the container's `Stack` leaves before it up to
    /// `space.gap.section`.
    pub(super) lead: f32,
    /// Above a header with `HeaderRegion::rule_above`.
    pub(super) above: f32,
    /// Between the rule above and the header: `space.gap.row`.
    pub(super) trail: f32,
    /// Under a stacked header with `HeaderRegion::rule`, before its gap.
    pub(super) under: f32,
    /// Between a footer with `FooterRegion::rule` and the content beside it: over the footer, or under it in the end
    /// region's top (`footer_rule_under`).
    pub(super) footer: f32,
    /// Between the body and an end region with `EndRegion::rule`, after the end gap.
    pub(super) end: f32,
}

impl RuleBands {
    /// The bands of `props`' rules under `env` on a display of `pixels_per_point`, for a container whose flow start is
    /// `start` (`layout::flow_start`). A rule above separates the container from what precedes it in flow: right after
    /// a rule there is none, and at its flow's start nothing precedes it to stand off from.
    pub(super) fn of(props: &ContainerProps, env: &Env, pixels_per_point: f32, start: Option<bool>) -> Self {
        let tokens = &env.tokens;
        let width = rule_look(tokens, RuleKind::Divider).width.points(pixels_per_point);
        let on = |rule: bool| if rule { width } else { 0.0 };
        let above = on(start != Some(true) && props.header.is_some_and(|header| header.rule_above));
        let spaced = above > 0.0 && env.get::<SectionRhythm>() == SectionRhythm::Spaced;
        let gaps = &tokens.tokens().space.gap;
        let stack_gap = env.stack_gap.filter(|(body, _)| env.flow_body == Some(*body)).map_or(0.0, |(_, gap)| gap);
        RuleBands {
            lead: if spaced && start.is_none() { (gaps.section - stack_gap).max(0.0) } else { 0.0 },
            above,
            trail: if spaced { gaps.row } else { 0.0 },
            under: on(props.header.is_some_and(|header| header.rule && !header.inline)),
            footer: on(props.footer.is_some_and(|footer| footer.rule)),
            end: on(props.end.is_some_and(|end| end.rule)),
        }
    }

    /// The band of the rule above the header with the room around it.
    pub(super) fn above_band(&self) -> f32 {
        self.lead + self.above + self.trail
    }
}

/// Whether a ruled footer's rule lies under it: a footer at the top of the end region separates itself from the end
/// region's content below; every other footer from the content above it.
pub(super) fn footer_rule_under(props: &ContainerProps) -> bool {
    props.footer.is_some_and(|footer| footer.place == FooterPlace::EndTop) && footer_in_end(props)
}

/// The space `props`' stacked header leaves before the body: its `HeaderRegion::gap`, none without one.
pub(super) fn header_gap_of(props: &ContainerProps, tokens: &UiTokens) -> f32 {
    props.header.and_then(|header| header.gap).map_or(0.0, |gap| gap_of(tokens, gap))
}

impl Gap {
    /// The gap in points, in the tokens `ui` shows with: for a host that decides how many children fit in a row.
    pub fn points(self, ui: &Ui) -> f32 {
        gap_of(&Env::of(ui).tokens, self)
    }
}

pub(super) fn gap_of(tokens: &UiTokens, gap: Gap) -> f32 {
    let (gaps, page) = (&tokens.tokens().space.gap, &tokens.tokens().size.page.gap);
    match gap {
        Gap::None => 0.0,
        Gap::Tight => gaps.tight,
        Gap::Stack => gaps.stack,
        Gap::Item => gaps.item,
        Gap::Section => gaps.section,
        Gap::CardColumn => gaps.card_column,
        Gap::CardRow => gaps.card_row,
        Gap::Inline => gaps.inline,
        Gap::PageTitle => page.title,
        Gap::PageLead => page.lead,
        Gap::PageActions => page.actions,
        Gap::PageSection => page.section,
        Gap::PageHeading => page.heading,
        Gap::PageLink => page.link,
        _ => gaps.row,
    }
}

pub(super) fn least_of(tokens: &UiTokens, least: Option<LeastWidth>) -> f32 {
    least.map_or(0.0, |least| least.points(tokens.tokens()))
}

pub(super) fn rect_of(parts: [f32; 4]) -> Rect {
    Rect::from_min_max(Pos2::new(parts[0], parts[1]), Pos2::new(parts[2], parts[3]))
}

/// The regions of a container inside its rect.
pub(super) struct Regions {
    pub(super) header: Rect,
    pub(super) body: Rect,
    /// The end region: the line's end edge, as high as the line or, full height, from under the header to the
    /// container's bottom, less a footer placed in it.
    pub(super) end: Rect,
    footer: Rect,
    /// The width the body's line gives the regions beside the body: an inline header and the end region, with their
    /// gaps.
    pub(super) beside: f32,
    /// The bands of the rules between the regions (`RuleBands`), each with the side its rule runs along.
    pub(super) rules: [Option<(Rect, RectSide)>; 4],
}

impl Regions {
    /// The regions with the header's band, which may spread over the padding, below `top`: the bottom of the band a
    /// layer keeps clear for its top grip.
    pub(super) fn below(mut self, top: f32) -> Regions {
        self.header.min.y = self.header.min.y.max(top);
        self
    }
}

/// The sizes of the header, footer and end region that the regions are placed from (zero or `None` for a region the
/// container lacks or that has not measured).
#[derive(Clone, Copy)]
pub(super) struct RegionSizes {
    pub(super) header: Option<Vec2>,
    pub(super) footer: Vec2,
    pub(super) end: Option<Vec2>,
}

/// What every region of a container shares: its id, direction, whether it is an AccessKit node, its env and the
/// container's body gap.
pub(super) struct RegionFrame<'e> {
    pub(super) id: Id,
    pub(super) rtl: bool,
    pub(super) node: bool,
    pub(super) env: &'e Env,
    pub(super) gap: Vec2,
}

impl RegionFrame<'_> {
    /// The ui of a region, an AccessKit child of the container when it is a node, its children spaced by `spacing`.
    /// The ui around a region spaces its own children differently in the measuring run and the real run, so a region
    /// sets its spacing itself: its measured size is then the size it takes.
    pub(super) fn open(&self, ui: &mut Ui, builder: UiBuilder, spacing: Vec2) -> Ui {
        let builder = if self.node { access::with_parent(builder, self.id) } else { builder };
        let mut region = alloc::child(ui, builder);
        style::set_item_spacing(&mut region, spacing);
        region
    }

    /// The column ui of the region `name` in `rect`, its children spaced by the container's body gap.
    pub(super) fn region(&self, ui: &mut Ui, name: &'static str, rect: Rect) -> Ui {
        let builder = UiBuilder::new().max_rect(rect).layout(child::column_layout(self.rtl)).scope_id(sub(self.id, name));
        self.open(ui, builder, self.gap)
    }
}

/// The divider along the inner edge of an edge pane with `edge_rule`, inside `rect`: the edge toward the window's
/// centre (`RectSide::toward_centre`).
pub(super) fn paint_edge_rule(ui: &mut Ui, props: &ContainerProps, env: &Env, rect: Rect) {
    let Mount::Pane(pane) = &props.mount else { return };
    let Some(side) = RectSide::toward_centre(pane.edge, env.direction.is_rtl()).filter(|_| props.edge_rule) else { return };
    let rule = rule_look(&env.tokens, RuleKind::Divider);
    ui.painter().extend(Rule::edge(&rule, rect, side, ui.ctx().pixels_per_point()));
}

/// The divider rules above and under the header, beside the footer and beside the end region, each in its band.
pub(super) fn paint_rules(ui: &mut Ui, env: &Env, regions: &Regions) {
    let rule = rule_look(&env.tokens, RuleKind::Divider);
    let ppp = ui.ctx().pixels_per_point();
    for (band, side) in regions.rules.iter().flatten() {
        ui.painter().extend(Rule::edge(&rule, *band, *side, ppp));
    }
}

/// The bracket of a linked `Row` (`Bracket::Linked`) from what its rows reported in `took`: a tick from the end of the
/// first and the last row's controls, `space.gap.tight` past them, to the middle child's centre line, and a stroke
/// along that line from each tick to the middle child's edge. `text.muted`, `stroke.thin` wide.
pub(super) fn paint_bracket(ui: &mut Ui, env: &Env, took: &crate::layout::BracketTook, rtl: bool) {
    let (Some(first), Some(last), Some(end), Some(middle)) = (took.first, took.last, took.end, took.middle) else { return };
    if last <= first {
        return;
    }
    let tokens = &env.tokens;
    let tight = tokens.tokens().space.gap.tight;
    let look = RuleLook { color: crate::style::rgba_color(tokens.color.text.muted), width: RuleWidth::Points(tokens.stroke.thin) };
    let ppp = ui.ctx().pixels_per_point();
    let (start, line) = if rtl { (-end - tight, middle.center().x) } else { (end + tight, middle.center().x) };
    let painter = ui.painter();
    for (from, to) in [
        (Pos2::new(start, first), Pos2::new(line, first)),
        (Pos2::new(start, last), Pos2::new(line, last)),
        (Pos2::new(line, first), Pos2::new(line, middle.top().max(first))),
        (Pos2::new(line, middle.bottom().min(last)), Pos2::new(line, last)),
    ] {
        painter.extend(Rule::shapes(&look, from, to, ppp));
    }
}

impl BodyHooks<'_> {
    /// The footer's rect and how far its left edge lies past the container's, under the control column of a
    /// label-scoped container; along an axis the room leaves unbounded, the footer follows the body and the end region.
    pub(super) fn place_footer(&self, props: &ContainerProps, regions: &mut Regions, inner: Rect, body_rect: Rect, rtl: bool) -> (Rect, f32) {
        // Along an axis the room leaves unbounded (a measuring run) the footer follows the body and the end region
        // it ran after; along a bounded axis it sits at the far edge.
        if !inner.max.y.is_finite() && !footer_in_end(props) {
            let mut bottom = regions.body.min.y + self.body_size.map_or(0.0, |size| size.y);
            if props.end.is_some() && !end_full_height(props) {
                bottom = bottom.max(regions.end.min.y + self.end_size.map_or(0.0, |size| size.y));
            }
            regions.footer.min.y = bottom;
            regions.footer.max.y = bottom + self.measured_footer.y;
        }
        let mut footer_rect = regions.footer;
        // The footer's start edge moves to the control column's, never past its own end edge.
        if props.label.is_some() {
            if rtl {
                footer_rect.max.x = body_rect.max.x.min(footer_rect.max.x).max(footer_rect.min.x);
            } else {
                footer_rect.min.x = body_rect.min.x.max(footer_rect.min.x).min(footer_rect.max.x);
            }
        }
        let offset = if rtl { regions.footer.max.x - footer_rect.max.x } else { footer_rect.min.x - regions.footer.min.x };
        (footer_rect, offset)
    }
}

impl<'b> BodyHooks<'b> {
    /// The header region (B1): the trailing controls take the end edge first (in call order, as a box at the edge)
    /// and the header fills what they leave. Records the header's size and returns the close it asked for.
    pub(super) fn header_region<C: Container>(
        &mut self,
        comp: &mut C,
        ui: &mut Ui,
        bcx: &mut BodyCx<'_>,
        frame: &RegionFrame<'_>,
        header: Rect,
        (trailing, inset, parts): (bool, f32, HeaderParts),
    ) -> Option<CloseReason> {
        let ctx = ui.ctx().clone();
        let (id, rtl) = (frame.id, frame.rtl);
        let (line, _) = self.header_line(header, inset, parts, rtl);
        let mut header_rect = line;
        let header = line;
        let mut controls = Vec2::ZERO;
        if trailing {
            // The controls keep their call order (first at the start of their box); the box sits at the end edge.
            let layout = if rtl { Layout::right_to_left(Align::Center) } else { Layout::left_to_right(Align::Center) };
            // The box is as wide as the measuring run found it, at the end edge; in the measuring run it spans the header.
            let room = match self.measured_controls.map(|measured| measured.x).filter(|_| header.width().is_finite()) {
                Some(width) if rtl => Rect::from_min_max(header.min, Pos2::new((header.min.x + width).min(header.max.x), header.max.y)),
                Some(width) => Rect::from_min_max(Pos2::new((header.max.x - width).max(header.min.x), header.min.y), header.max),
                None => header,
            };
            let builder = UiBuilder::new().max_rect(room).layout(layout).scope_id(sub(id, "controls"));
            let mut controls_ui = frame.open(ui, builder, Vec2::splat(gap_of(&frame.env.tokens, Gap::Item)));
            let installed = Env::install_for(&controls_ui, frame.env.clone());
            comp.controls(&mut controls_ui, bcx);
            Env::restore(&ctx, installed);
            controls = controls_ui.min_rect().size();
            self.controls_size = Some(controls);
            if controls.x > 0.0 {
                let cut = controls.x + self.region_gap;
                if rtl {
                    header_rect.min.x = (header_rect.min.x + cut).min(header_rect.max.x);
                } else {
                    header_rect.max.x = (header_rect.max.x - cut).max(header_rect.min.x);
                }
            }
        }
        let mut header_ui = frame.region(ui, "header", header_rect);
        let installed = Env::install_for(&header_ui, frame.env.clone());
        comp.header(&mut header_ui, bcx);
        Env::restore(&ctx, installed);
        let mut took = self.region_size(&header_ui);
        if controls.x > 0.0 {
            took.x += controls.x + self.region_gap;
        }
        took.y = took.y.max(controls.y);
        let slot = |width: f32| if width > 0.0 { width + self.region_gap } else { 0.0 };
        took.x += 2.0 * inset + slot(parts.start) + slot(parts.end);
        self.header_size = Some(took);
        bcx.close_requested()
    }

    /// The room the container's own header parts take this pass (`Container::header_parts`). The faces it measures
    /// are a question about the parts, not content of the body around: no transcript keeps or checks them.
    pub(super) fn header_parts<C: Container>(comp: &C, ui: &Ui, env: &Env, cx: &crate::contract::Cx<'_, C>) -> HeaderParts {
        let input = crate::contract::steps::MeasureIn { ui, env, proposal: craft_behavior::Proposal::Ideal, room: None };
        let measure = crate::contract::steps::measure_cx::<C>(&input, cx.style, cx.core.resolved, cx.shown.as_deref());
        crate::runtime::layout_memory::boundary(ui.ctx(), true, || comp.header_parts(&measure))
    }

    /// The header's line inside its inset, less the slots of the container's own header parts at its two edges: the
    /// rect the header's children get, and the slots. An end edge that narrows (`HeaderParts::end_narrows`) gives up
    /// room, down to its least, until the children keep their room.
    pub(super) fn header_line(&self, header: Rect, inset: f32, parts: HeaderParts, rtl: bool) -> (Rect, HeaderSlots) {
        let mut flow = craft_behavior::Flow::new(header.shrink2(Vec2::new(inset, 0.0)), rtl);
        let mut slots = HeaderSlots::default();
        if parts.start > 0.0 {
            let (slot, rest) = flow.take_start(parts.start);
            slots.start = Some(slot);
            flow = rest.take_start(self.region_gap).1;
        }
        let end = match parts.end_least {
            Some(least) => parts.end.min((flow.rect().width() - self.region_gap - parts.keep).max(least)),
            None => parts.end,
        };
        if end > 0.0 {
            let (slot, rest) = flow.take_end(end);
            slots.end = Some(slot);
            flow = rest.take_end(self.region_gap).1;
        }
        (flow.rect(), slots)
    }

    /// The end region: its content runs in reading order, so its roving order is its order on the line; the region
    /// is as wide as it measured, so the last child sits at the end edge. Beside a body with a members' line (`line`,
    /// from the region's top) each child sits on that line by the rule members follow (`layout::place_in_cell`);
    /// without one it is centred, and a full-height region starts at its top. Records the region's size and returns
    /// the close it asked for.
    #[allow(clippy::too_many_arguments, reason = "the region's own rect and line beside what every region gets")]
    pub(super) fn end_region<C: Container>(
        &mut self,
        comp: &mut C,
        ui: &mut Ui,
        bcx: &mut BodyCx<'_>,
        frame: &RegionFrame<'_>,
        props: &ContainerProps,
        end: Rect,
        line: Option<RowLine>,
    ) -> Option<CloseReason> {
        let cross = if end_full_height(props) || line.is_some() { Align::Min } else { Align::Center };
        let layout = if frame.rtl { Layout::right_to_left(cross) } else { Layout::left_to_right(cross) };
        let builder = UiBuilder::new().max_rect(end).layout(layout).scope_id(sub(frame.id, "end"));
        let mut end_ui = frame.open(ui, builder, frame.gap);
        let env = match line {
            Some(line) => frame.env.derive_local(|scoped| scoped.row_line = Some(line)),
            None => frame.env.clone(),
        };
        let installed = Env::install_for(&end_ui, env);
        comp.end(&mut end_ui, bcx);
        Env::restore(ui.ctx(), installed);
        self.end_size = Some(self.region_size(&end_ui));
        bcx.close_requested()
    }

    /// The space under a stacked header `header_h` high: the band of the rule under it, then `HeaderRegion::gap` while
    /// the body shows, scaled by its presence.
    pub(super) fn header_space(&self, header_h: f32) -> f32 {
        self.rules.under + if header_h > 0.0 { self.header_gap * self.presence.t } else { 0.0 }
    }

    /// The regions inside `inner` (the rect minus padding and the padding tier), placed from `sizes`, in a real run when
    /// `real` (not a measuring run: the `Ui` is no sizing pass). A header with a band spreads over the `bleed` the
    /// padding leaves around `inner` at the top and the sides.
    #[allow(clippy::too_many_arguments, reason = "the regions' rect, room and sizes beside the run they are placed for")]
    pub(super) fn regions(&self, props: &ContainerProps, inner: Rect, bleed: Vec2, room: Vec2, rtl: bool, sizes: RegionSizes, real: bool) -> Regions {
        let inline = props.header.is_some_and(|header| header.inline);
        let (in_end, full) = (footer_in_end(props), end_full_height(props));
        let header_h = if props.header.is_some() { sizes.header.unwrap_or(Vec2::ZERO).y } else { 0.0 };
        let footer_h = if props.footer.is_some() && !footer_in_body(props) { sizes.footer.y } else { 0.0 };
        // A footer in the end region takes no band across the container: it stacks with the end region's content.
        let across_h = if in_end { 0.0 } else { footer_h };
        let end_h = sizes.end.map_or(0.0, |end| end.y) + if in_end { footer_h } else { 0.0 };
        // An inline header takes no height of its own: the line is as high as the taller of header and body. A rule
        // above the header takes its band either way.
        let stacked_h = self.rules.above_band() + if inline { 0.0 } else { header_h + self.header_space(header_h) };
        let top = inner.min.y + stacked_h;
        // Along an axis the container is not given, its body is offered the room the container was offered and the
        // container takes what the body took.
        // `content_sized` is the width's rule (`FillHeight` takes its content's width); the height has its own.
        let content_sized = self.sizing(props).content_width();
        let content_y = self.sizing(props) == Sizing::Content || ((self.sizing(props).fills_height() || !inner.max.y.is_finite()) && !self.given.1);
        let (bottom, footer_top) = if content_y {
            let body_room = (room.y - stacked_h - across_h).max(0.0);
            // A body that fills records the band it was given, which may predate the footer's measure: the line is
            // never taller than the room less header and footer, so the footer stays inside the room.
            let body_h = (self.measured_body.map_or(0.0, |size| size.y) * self.presence.t).min(body_room);
            let line_h = if inline { body_h.max(header_h) } else { body_h };
            // A full-height end region stands beside the footer too, so only the part of it under the line counts.
            let end_line = if full { (end_h - across_h).max(0.0) } else { end_h };
            let line_bottom = top + line_h.max(end_line).min(body_room);
            // A scrolling body offered no bound (a content-sized container in a body that scrolls) shows all of its
            // content: in the real run its view is the line this pass measured, never an unbounded band. Its measuring
            // run keeps the unbounded band, in which the content takes its whole extent.
            let unbounded_view = real && !body_room.is_finite() && scroll_of(props).is_some();
            let bottom = if unbounded_view { line_bottom } else { top + body_room };
            (bottom, line_bottom)
        } else {
            // A fill-width body is offered the room the parent gives this pass, never the height its content took last
            // pass: a collection that scrolls takes its rows up to that room, and from zero it would stay at zero.
            // An inline header shares the line with the body, so the body is offered that line: as high as the tallest of
            // header, end region and the body this pass measured, at most the room.
            let body_room = (room.y - stacked_h - across_h).max(0.0);
            let offered = (self.sizing(props) == Sizing::FillWidth && room.y.is_finite()).then(|| {
                let line = if inline && self.measured_body.is_some() {
                    header_h.max(end_h).max(self.measured_body.map_or(0.0, |size| size.y)).min(body_room)
                } else {
                    body_room
                };
                top + line
            });
            (offered.unwrap_or(inner.max.y - across_h), inner.max.y - across_h)
        };
        let width = if content_sized {
            self.shared_width(room.x, props)
        } else if !self.given.0 {
            self.shared_width(room.x, props).max(inner.width())
        } else {
            // A container given less width than its padding has an inverted inner rect: its regions take no width,
            // at their start edge.
            inner.width().max(0.0)
        };
        // A measure with no room limit along x offers an unbounded width. Left to right the body runs from its start
        // edge to +inf as egui allows; right to left its start edge is the right one, and an unbounded width would put
        // its left edge at -inf and every rect laid out from it at NaN. It is as wide as a scrolling region there, as
        // the measuring rect is (`measure_run`).
        let width = if rtl && !width.is_finite() { crate::contract::MAX_EXTENT } else { width };
        let (left, right) = if rtl { (inner.max.x - width, inner.max.x) } else { (inner.min.x, inner.min.x + width) };
        let band = |top: f32, bottom: f32| Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, bottom.max(top)));
        let header_top = inner.min.y + self.rules.above_band();
        let header = band(header_top, if props.header.is_some() { header_top + header_h.max(0.0) } else { header_top });
        let mut body = band(top, bottom);
        let header = if inline {
            // The header keeps the width it measured, none when it measured nothing; before its first measure it
            // takes the whole line. The body takes the rest.
            let head_w = sizes.header.map_or(width, |took| took.x.min(width));
            let cut = if head_w > 0.0 { (head_w + self.region_gap).min(width) } else { 0.0 };
            if rtl {
                body.max.x = right - cut;
                Rect::from_min_max(Pos2::new(right - head_w, header.min.y), header.max)
            } else {
                body.min.x = left + cut;
                Rect::from_min_max(header.min, Pos2::new(left + head_w, header.max.y))
            }
        } else {
            header
        };
        let across = |top: f32, width: f32| (width > 0.0).then(|| (Rect::from_x_y_ranges(inner.x_range(), top..=top + width), RectSide::Top));
        let above = across(inner.min.y + self.rules.lead, self.rules.above);
        let under = if inline { None } else { across(header.max.y, self.rules.under) };
        // The end region keeps the width it measured (with a footer in it, the wider of the two) and takes the end
        // edge; the body takes the rest. Before its first measure it spans the line and the body keeps its width.
        let measured = props.end.and(sizes.end).map(|end| {
            let footer_w = if in_end { sizes.footer.x } else { 0.0 };
            end.x.max(footer_w).min(width)
        });
        let taken = measured.unwrap_or(0.0);
        let cut = if taken > 0.0 { taken + self.end_gap + self.rules.end } else { 0.0 };
        let end_bottom = if full { footer_top + across_h } else { footer_top };
        // The line's own edges before the first measure: a line of unbounded room (a content-sized container measured
        // in an unbounded flow) has no finite width to take from its other edge.
        let (end_left, end_right) = match measured {
            None => (left, right),
            Some(end_w) if rtl => (left, left + end_w),
            Some(end_w) => (right - end_w, right),
        };
        let mut end = Rect::from_min_max(Pos2::new(end_left, top), Pos2::new(end_right, end_bottom.max(top)));
        if rtl {
            body.min.x = (body.min.x + cut).min(body.max.x);
        } else {
            body.max.x = (body.max.x - cut).max(body.min.x);
        }
        let footer = match props.footer.map(|footer| footer.place) {
            Some(FooterPlace::EndTop) if in_end => {
                let footer = Rect::from_min_max(end.min, Pos2::new(end.max.x, (end.min.y + footer_h).min(end.max.y)));
                end.min.y = footer.max.y;
                footer
            }
            _ if in_end => {
                let footer = Rect::from_min_max(Pos2::new(end.min.x, (end.max.y - footer_h).max(end.min.y)), end.max);
                end.max.y = footer.min.y;
                footer
            }
            // Beside a full-height end region the footer lies under the body only.
            _ if full && props.end.is_some() => Rect::from_min_max(Pos2::new(body.min.x, footer_top), Pos2::new(body.max.x, footer_top + footer_h)),
            _ => band(footer_top, footer_top + footer_h),
        };
        let header = if props.header.is_some_and(|header| header.fill.is_some()) {
            Rect::from_min_max(header.min - bleed, Pos2::new(header.max.x + bleed.x, header.max.y - bleed.y))
        } else {
            header
        };
        let footer_rule = (self.rules.footer > 0.0).then(|| {
            // A footer across the container rules the whole width; one beside or in the end region rules its own.
            let x = if full { footer.x_range() } else { inner.x_range() };
            let w = self.rules.footer;
            if footer_rule_under(props) {
                (Rect::from_x_y_ranges(x, footer.max.y - w..=footer.max.y), RectSide::Bottom)
            } else {
                (Rect::from_x_y_ranges(x, footer.min.y..=footer.min.y + w), RectSide::Top)
            }
        });
        // The end region's rule runs along its edge toward the body, beside a footer placed in it too.
        let end_rule = (self.rules.end > 0.0 && end.width() > 0.0).then(|| {
            let (top, bottom) = if in_end { (end.min.y.min(footer.min.y), end.max.y.max(footer.max.y)) } else { (end.min.y, end.max.y) };
            let x = if rtl { end.max.x } else { end.min.x - self.rules.end };
            (Rect::from_x_y_ranges(x..=x + self.rules.end, top..=bottom), RectSide::end(rtl))
        });
        Regions { header, body, end, footer, beside: width - body.width(), rules: [above, under, footer_rule, end_rule] }
    }
}

/// The extent a label placed at `label` takes from `body` by placing the control at `control`. A label beside or above
/// takes its column either by narrowing the control's rect or by shifting it along the row, which a content-sized row
/// (unbounded to the end) does; the extent grows by whichever of the two is finite. A label after the control takes
/// the gap and its own width past the control's end edge, not the rest of the row, and the indent before the control
/// when the label step moved it into the control column.
pub(super) fn label_extra_of(body: Rect, control: Rect, label: Rect, rtl: bool) -> Vec2 {
    let taken = |amount: f32| if amount.is_finite() { amount.max(0.0) } else { 0.0 };
    let after = label.is_positive() && if rtl { label.max.x <= control.min.x } else { label.min.x >= control.max.x };
    if after {
        let reach = control.union(label);
        let indent = taken(if rtl { body.max.x - control.max.x } else { control.min.x - body.min.x });
        return Vec2::new(reach.width() - control.width() + indent, taken(reach.height() - control.height()));
    }
    (body.size() - control.size()).max(Vec2::ZERO).max(Vec2::new(
        taken(control.min.x - body.min.x).max(taken(body.max.x - control.max.x)),
        taken(control.min.y - body.min.y).max(taken(body.max.y - control.max.y)),
    ))
}

/// How a container's body takes part in the label columns of the group around it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LabelColumns {
    /// The body is a group of its own (`label_column`): its labelled rows share the widest label.
    Own,
    /// The body's rows read the group around the container, as the rows directly in the group do. `sized`: the
    /// container shows at the width it measures (`ColumnGroup::through`).
    Through { sized: bool },
    /// A labelled container's body: its rows read no label column of the group around, and share its other columns
    /// (`ColumnGroup::field`).
    Field,
    /// The body lays out apart from the group around it, and its rows read none.
    Apart,
}

impl LabelColumns {
    /// The part `props`'s body takes. A labelled container's body is its field's control, whose labels stay its own
    /// while its other columns are the group's. A roving group's body (one control over its members: a radio group),
    /// a body on a mount of its own (a popup, a dialog, a pane), one with a scroll region (a view of its own, as a
    /// bar's) and a grid (whose columns align its cells) lay out apart. Every other body (a section, a box, a stack, a
    /// row) passes the group through.
    pub(super) fn of(props: &ContainerProps, sized: bool) -> Self {
        let apart = props.roving.is_some()
            || props.mount != Mount::Inline
            || props.scroll.is_some()
            || matches!(props.layout, crate::layout::BodyLayout::Grid { .. });
        if props.label_column {
            LabelColumns::Own
        } else if apart {
            LabelColumns::Apart
        } else if props.label.is_some() {
            LabelColumns::Field
        } else {
            LabelColumns::Through { sized }
        }
    }
}

/// What a measuring run of a body found of the label columns its rows share.
#[derive(Default)]
pub(super) struct LabelsFound {
    /// The width the rows of the body's own group take at its widest label (`ColumnGroup::scope_measured`).
    pub(super) extent: Option<f32>,
    /// How far the rows of a `Through` body reached past the columns of the group around (`ColumnGroup::through`).
    pub(super) reached: Option<Reached>,
    /// The widths the body's own group settled on, when the rows have to run again at them
    /// (`ColumnGroup::scope_measured`).
    pub(super) unsettled: Option<Columns>,
}

/// Runs `body`, offered `room`, under the label columns `columns`: in a column group named `labels` under the container
/// `id`, so the labelled rows directly inside share the widest label; through to the group around; as a field's
/// control, which shares the group's columns other than its labels; or behind a barrier.
/// An own group opens at the `seed` widths a first run of the rows settled.
pub(super) fn shared_labels<R>(
    ui: &mut Ui,
    id: Id,
    columns: LabelColumns,
    room: f32,
    seed: Option<Columns>,
    found: &mut LabelsFound,
    body: impl FnOnce(&mut Ui) -> R,
) -> R {
    match columns {
        LabelColumns::Own => own_labels(ui, id, (room, seed), found, body),
        LabelColumns::Through { sized } => through_labels(ui, id, (room, sized), found, body),
        LabelColumns::Field => ColumnGroup::field(ui, body),
        LabelColumns::Apart => ColumnGroup::barrier(ui, body),
    }
}

/// The body's rows in a label group of their own, offered `room` and opened at the `seed` widths.
fn own_labels<R>(ui: &mut Ui, id: Id, (room, seed): (f32, Option<Columns>), found: &mut LabelsFound, body: impl FnOnce(&mut Ui) -> R) -> R {
    let (result, scoped) = ColumnGroup::scope_measured(ui, sub(id, "labels"), seed, |ui, group| {
        group.offer(room);
        body(ui)
    });
    found.extent = scoped.extent;
    // A later sweep of the same run may take a row's measure from the earlier sweep without reading the column again
    // (a stack's sweep at the width its first sweep found): that measure is still the one taken at the narrower
    // column, so the rows run again whenever any sweep found one.
    found.unsettled = scoped.unsettled.or(found.unsettled.take());
    result
}

/// The body's rows in the label group around the container.
fn through_labels<R>(ui: &mut Ui, id: Id, (room, sized): (f32, bool), found: &mut LabelsFound, body: impl FnOnce(&mut Ui) -> R) -> R {
    let (result, reached) = ColumnGroup::through(ui, sub(id, "through"), room, sized, body);
    found.reached = reached;
    result
}
