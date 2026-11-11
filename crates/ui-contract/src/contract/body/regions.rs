//! Where the header, body, footer and end regions sit inside the container, and the rules between them.

use super::BodyHooks;
use crate::contract::cx::BodyCx;
use crate::contract::{Container, ContainerProps, FooterPlace, Gap, LeastWidth, Pad, Sizing};
use crate::layout::ColumnGroup;
use crate::primitives::Rule;
use crate::runtime::access::{self};
use crate::runtime::egui::{alloc, child, style};
use crate::runtime::env::Env;
use crate::runtime::memory::sub;
use crate::runtime::overlay::CloseReason;
use crate::style::{RuleKind, UiTokens, rule_look};
use egui::{Align, Id, Layout, Pos2, Rect, Ui, UiBuilder, Vec2};

/// Whether the footer sits in the end region: it asks to, and the container has one.
pub(super) fn footer_in_end(props: &ContainerProps) -> bool {
    props.end.is_some() && props.footer.is_some_and(|footer| footer.place.in_end())
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
        Pad::Control => pads.control,
        Pad::Regular => pads.regular,
        Pad::Roomy => pads.roomy,
        _ => 0.0,
    }
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
    let gaps = &tokens.tokens().space.gap;
    match gap {
        Gap::None => 0.0,
        Gap::Tight => gaps.tight,
        Gap::Stack => gaps.stack,
        Gap::Item => gaps.item,
        Gap::Section => gaps.section,
        Gap::CardColumn => gaps.card_column,
        Gap::CardRow => gaps.card_row,
        _ => gaps.row,
    }
}

pub(super) fn least_of(tokens: &UiTokens, least: Option<LeastWidth>) -> f32 {
    match least {
        Some(LeastWidth::Popup) => tokens.tokens().size.popup_min,
        None => 0.0,
    }
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
}

/// The sizes of the header, footer and end region that the regions are placed from (zero or `None` for a region the
/// container lacks or that has not measured).
#[derive(Clone, Copy)]
pub(super) struct RegionSizes {
    pub(super) header: Option<Vec2>,
    pub(super) footer: Vec2,
    pub(super) end: Option<Vec2>,
}

/// What every region of a container shares: its id, direction, whether it is an AccessKit node and its env.
pub(super) struct RegionFrame<'e> {
    pub(super) id: Id,
    pub(super) rtl: bool,
    pub(super) node: bool,
    pub(super) env: &'e Env,
}

impl RegionFrame<'_> {
    /// The ui builder of the region `name` in `rect`.
    pub(super) fn region(&self, name: &'static str, rect: Rect) -> UiBuilder {
        let builder = UiBuilder::new().max_rect(rect).layout(child::column_layout(self.rtl)).scope_id(sub(self.id, name));
        if self.node { access::with_parent(builder, self.id) } else { builder }
    }
}

/// The divider rules above and under the header and above the footer.
pub(super) fn paint_rules(ui: &mut Ui, props: &ContainerProps, env: &Env, inner: Rect, regions: &Regions) {
    let rule = rule_look(&env.tokens, RuleKind::Divider);
    let ppp = ui.ctx().pixels_per_point();
    let draw_rule = |from: f32, to: f32, y: f32| ui.painter().extend(Rule::shapes(&rule, Pos2::new(from, y), Pos2::new(to, y), ppp));
    if props.header.is_some_and(|header| header.rule_above) {
        draw_rule(inner.min.x, inner.max.x, regions.header.min.y);
    }
    if props.header.is_some_and(|header| header.rule && !header.inline) {
        draw_rule(inner.min.x, inner.max.x, regions.header.max.y);
    }
    // A footer across the container rules the whole width; one beside or in the end region rules its own.
    match props.footer.filter(|footer| footer.rule) {
        Some(footer) if footer.place == FooterPlace::EndTop && footer_in_end(props) => {
            draw_rule(regions.footer.min.x, regions.footer.max.x, regions.footer.max.y);
        }
        Some(_) if end_full_height(props) => draw_rule(regions.footer.min.x, regions.footer.max.x, regions.footer.min.y),
        Some(_) => draw_rule(inner.min.x, inner.max.x, regions.footer.min.y),
        None => {}
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
        if props.label.is_some() {
            if rtl {
                footer_rect.max.x = body_rect.max.x.clamp(footer_rect.min.x, footer_rect.max.x);
            } else {
                footer_rect.min.x = body_rect.min.x.clamp(footer_rect.min.x, footer_rect.max.x);
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
        trailing: bool,
        inset: f32,
    ) -> Option<CloseReason> {
        let ctx = ui.ctx().clone();
        let (id, rtl) = (frame.id, frame.rtl);
        let mut header_rect = header.shrink2(Vec2::new(inset, 0.0));
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
            let builder = if frame.node { access::with_parent(builder, id) } else { builder };
            let mut controls_ui = alloc::child(ui, builder);
            // The same spacing in the measuring run and the placing run, whatever the parent's item spacing is.
            style::set_item_spacing(&mut controls_ui, Vec2::splat(gap_of(&frame.env.tokens, Gap::Item)));
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
        let mut header_ui = alloc::child(ui, frame.region("header", header_rect));
        let installed = Env::install_for(&header_ui, frame.env.clone());
        comp.header(&mut header_ui, bcx);
        Env::restore(&ctx, installed);
        let mut took = self.region_size(&header_ui);
        if controls.x > 0.0 {
            took.x += controls.x + self.region_gap;
        }
        took.y = took.y.max(controls.y);
        took.x += 2.0 * inset;
        self.header_size = Some(took);
        bcx.close_requested()
    }

    /// The end region: its content runs in reading order, so its roving order is its order on the line; the region
    /// is as wide as it measured, so the last child sits at the end edge. On the line it is centred; a full-height
    /// region starts at its top. Records the region's size and returns the close it asked for.
    pub(super) fn end_region<C: Container>(
        &mut self,
        comp: &mut C,
        ui: &mut Ui,
        bcx: &mut BodyCx<'_>,
        frame: &RegionFrame<'_>,
        props: &ContainerProps,
        end: Rect,
    ) -> Option<CloseReason> {
        let cross = if end_full_height(props) { Align::Min } else { Align::Center };
        let layout = if frame.rtl { Layout::right_to_left(cross) } else { Layout::left_to_right(cross) };
        let builder = UiBuilder::new().max_rect(end).layout(layout).scope_id(sub(frame.id, "end"));
        let builder = if frame.node { access::with_parent(builder, frame.id) } else { builder };
        let mut end_ui = alloc::child(ui, builder);
        let installed = Env::install_for(&end_ui, frame.env.clone());
        comp.end(&mut end_ui, bcx);
        Env::restore(ui.ctx(), installed);
        self.end_size = Some(self.region_size(&end_ui));
        bcx.close_requested()
    }

    /// The space under a stacked header `header_h` high: `HeaderRegion::gap` while the body shows, scaled by its presence.
    pub(super) fn header_space(&self, header_h: f32) -> f32 {
        if header_h > 0.0 { self.header_gap * self.presence.t } else { 0.0 }
    }

    /// The regions inside `inner` (the rect minus padding and the padding tier), placed from `sizes`. A header with a
    /// band spreads over the `bleed` the padding leaves around `inner` at the top and the sides.
    pub(super) fn regions(&self, props: &ContainerProps, inner: Rect, bleed: f32, room: Vec2, rtl: bool, sizes: RegionSizes) -> Regions {
        let inline = props.header.is_some_and(|header| header.inline);
        let (in_end, full) = (footer_in_end(props), end_full_height(props));
        let header_h = if props.header.is_some() { sizes.header.unwrap_or(Vec2::ZERO).y } else { 0.0 };
        let footer_h = if props.footer.is_some() { sizes.footer.y } else { 0.0 };
        // A footer in the end region takes no band across the container: it stacks with the end region's content.
        let across_h = if in_end { 0.0 } else { footer_h };
        let end_h = sizes.end.map_or(0.0, |end| end.y) + if in_end { footer_h } else { 0.0 };
        // An inline header takes no height of its own: the line is as high as the taller of header and body.
        let stacked_h = if inline { 0.0 } else { header_h + self.header_space(header_h) };
        let top = inner.min.y + stacked_h;
        // Along an axis the container is not given, its body is offered the room the container was offered and the
        // container takes what the body took.
        let content_sized = self.sizing(props) == Sizing::Content;
        let content_y = content_sized || ((self.sizing(props) == Sizing::Fill || !inner.max.y.is_finite()) && !self.given.1);
        let (bottom, footer_top) = if content_y {
            let body_room = (room.y - stacked_h - across_h).max(0.0);
            // A body that fills records the band it was given, which may predate the footer's measure: the line is
            // never taller than the room less header and footer, so the footer stays inside the room.
            let body_h = (self.measured_body.map_or(0.0, |size| size.y) * self.presence.t).min(body_room);
            let line_h = if inline { body_h.max(header_h) } else { body_h };
            // A full-height end region stands beside the footer too, so only the part of it under the line counts.
            let end_line = if full { (end_h - across_h).max(0.0) } else { end_h };
            (top + body_room, top + line_h.max(end_line).min(body_room))
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
            inner.width()
        };
        let (left, right) = if rtl { (inner.max.x - width, inner.max.x) } else { (inner.min.x, inner.min.x + width) };
        let band = |top: f32, bottom: f32| Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, bottom.max(top)));
        let header = band(inner.min.y, if props.header.is_some() { inner.min.y + header_h.max(0.0) } else { inner.min.y });
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
        // The end region keeps the width it measured (with a footer in it, the wider of the two) and takes the end
        // edge; the body takes the rest. Before its first measure it spans the line and the body keeps its width.
        let measured = props.end.and(sizes.end).map(|end| {
            let footer_w = if in_end { sizes.footer.x } else { 0.0 };
            end.x.max(footer_w).min(width)
        });
        let taken = measured.unwrap_or(0.0);
        let cut = if taken > 0.0 { taken + self.region_gap } else { 0.0 };
        let end_w = measured.unwrap_or(width);
        let end_bottom = if full { footer_top + across_h } else { footer_top };
        let mut end = if rtl {
            Rect::from_min_max(Pos2::new(left, top), Pos2::new(left + end_w, end_bottom.max(top)))
        } else {
            Rect::from_min_max(Pos2::new(right - end_w, top), Pos2::new(right, end_bottom.max(top)))
        };
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
            Rect::from_min_max(header.min - Vec2::splat(bleed), Pos2::new(header.max.x + bleed, header.max.y - bleed))
        } else {
            header
        };
        Regions { header, body, end, footer }
    }
}

/// The extent a label takes from `body` by placing the control at `control`. The label takes its column either by
/// narrowing the control's rect or by shifting it along the row, which a content-sized row (unbounded to the end)
/// does; the extent grows by whichever of the two is finite.
pub(super) fn label_extra_of(body: Rect, control: Rect) -> Vec2 {
    let taken = |amount: f32| if amount.is_finite() { amount.max(0.0) } else { 0.0 };
    (body.size() - control.size()).max(Vec2::ZERO).max(Vec2::new(
        taken(control.min.x - body.min.x).max(taken(body.max.x - control.max.x)),
        taken(control.min.y - body.min.y).max(taken(body.max.y - control.max.y)),
    ))
}

/// Runs `body` in a column group named `labels` under the container `id` when `shared` is set, so the labelled
/// rows directly inside share the widest label. Otherwise the body runs behind a barrier: the labelled rows of a
/// nested container read no group of the container around it. In a measuring run, `extent` takes the width the
/// group's rows take at the widest label (`ColumnGroup::scope_measured`).
pub(super) fn shared_labels<R>(ui: &mut Ui, id: Id, shared: bool, room: f32, extent: &mut Option<f32>, body: impl FnOnce(&mut Ui) -> R) -> R {
    if shared {
        let (result, took) = ColumnGroup::scope_measured(ui, sub(id, "labels"), |ui, group| {
            group.offer(room);
            body(ui)
        });
        *extent = took;
        result
    } else {
        ColumnGroup::barrier(ui, body)
    }
}
