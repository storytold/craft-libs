//! ListRow as a component: its parts, input, access and paint pass.

use super::ListRowStyleKey;
use super::output::{RowEvent, RowOutput};
use super::paint::draw_clip_arrow;
use super::placement::{Sizes, lead_run};
use super::style::{FillKey, ListRowStyle, RowForm, fill_of, row_ink_of, selected_outline};
use super::thumb::{RowThumb, target_at};
use super::trailing::{HostedSize, TrailingFace, TrailingItem, TrailingPart, below_key, cell_key, field_key};
use super::{ListRow, RowDragSpot, RowInk};
use crate::contract::{
    Access, Bind, Component, Cx, DragPreview, Focus as _, MeasureCx, Name, OpenChange, PaintUi, PartsCx, Pointer as _, Props as _, Role, RowPressed,
    Styled as _, WordSetCx as _,
};
use crate::controls::disclosure::Disclosure;
use crate::display::Progress;
use crate::list::toggle::{ToggleFace, ToggleState};
use crate::list::words::ListWords;
use crate::primitives::{RectSide, RuleKind, StrokeWidth, TextRun, Wrap, glyph};
use crate::runtime::commands::FireSource;
use crate::runtime::env::{Label, WordTemplate};
use crate::style::{Flags, IconLook, StyleCx, SurfaceLook, TextLook, UiTokens};
use craft_behavior::{ColumnKey, Metrics, NavShape, PartKey, PartRef, PressPhase, Proposal, Roving, Sense};
use craft_keymap::Mods;
use craft_theme::variant::Tone;
use craft_theme::{Rgba, RowStyle, Stateful, TypeRole};
use egui::{CornerRadius, Pos2, Rect, Vec2};

pub(super) const TOGGLE: PartKey = PartKey::new("toggle");
pub(super) const TEXT_CELL: PartKey = PartKey::new("text-cell");
const TOGGLE_CELL: PartKey = PartKey::new("toggle-cell");
const LABEL_CELL: PartKey = PartKey::new("label-cell");
const DISCLOSURE: PartKey = PartKey::new("disclosure");

const LINK: PartKey = PartKey::new("link");
const PROGRESS: PartKey = PartKey::new("progress");
const LINES: PartKey = PartKey::new("lines");
pub(super) const TRAILING: PartKey = PartKey::new("trailing");
const TRAILING_ICON: PartKey = PartKey::new("trailing-icon");
const TRAILING_TEXT: PartKey = PartKey::new("trailing-text");
pub(super) const THUMB: PartKey = PartKey::new("thumb");
const LEAD: PartKey = PartKey::new("lead");

/// The name a pattern with `{row}` gives for the row labelled `row`.
pub(super) fn named(pattern: &Label, row: &str) -> String {
    WordTemplate::new(pattern.as_str()).fill(&[("row", row)])
}

impl Component for ListRow<'_> {
    const ROLE: Role = Role::LIST_ITEM;
    const OVERLAYS: bool = false;
    const ALT_ROLES: &'static [Role] = &[Role::TREE_ITEM, Role::LIST_BOX_OPTION];
    type Style = ListRowStyle;
    type Output = RowOutput;

    fn style(cx: &StyleCx<'_>, key: ListRowStyleKey) -> ListRowStyle {
        let tokens = cx.tokens;
        let colors = &tokens.color;
        let text = &colors.text;
        let clear = Rgba::TRANSPARENT;
        let c = |rgba: Rgba| cx.color(rgba);
        let form = if key.option { RowForm::Option } else { RowForm::List };
        let fill = fill_of(cx, FillKey { striped: key.striped, layer: key.layer, form });
        // An option sits inset in its popup with rounded corners whatever the theme's row form.
        let rounded = key.option || tokens.shape.rows == RowStyle::Rounded;
        let radius = if rounded { UiTokens::corner(tokens.shape.radius.control) } else { CornerRadius::ZERO };
        let mut surface = SurfaceLook::new(fill).radius(radius);
        if key.layer {
            surface = surface.border(Stateful::new(c(clear)).when(Flags::SELECTED, c(selected_outline(cx))), tokens.stroke.thin);
        }
        if key.option {
            let edge = Stateful::new(c(clear)).when(Flags::SELECTED, c(colors.accent.text)).when(Flags::LEAD, c(colors.line.control_edge));
            surface = surface.border(edge, tokens.stroke.thin);
        }
        let ink = |base: Rgba| row_ink_of(cx, c(base), form);
        let base = match key.ink {
            RowInk::Normal => text.normal,
            RowInk::Muted => text.muted,
            RowInk::Dimmed => text.faint,
        };
        let weight = key.strong || key.heading;
        let role = key.role.unwrap_or(if weight { TypeRole::BodyStrong } else { TypeRole::Body });
        let sizes = Sizes::of(cx, role, key.rows);
        let muted = if key.ink == RowInk::Dimmed { text.faint } else { text.muted };
        ListRowStyle {
            surface,
            metrics: Metrics::new().min_size(Vec2::new(0.0, sizes.row)).gap(sizes.gap),
            text: TextLook::new(role, ink(base)),
            icon: IconLook::new(
                sizes.icon,
                match key.tone {
                    Tone::Neutral => ink(base),
                    tone => row_ink_of(cx, cx.tone(tone), form),
                },
            ),
            trailing_icon: IconLook::new(
                sizes.icon,
                match key.trailing_tone {
                    Tone::Neutral => ink(base),
                    tone => row_ink_of(cx, cx.tone(tone), form),
                },
            ),
            detail: TextLook::new(TypeRole::Caption, ink(text.faint)),
            cell: TextLook::new(role, ink(base)),
            cell_muted: TextLook::new(role, ink(muted)),
            thumb_edge: c(colors.line.border),
            thumb_radius: UiTokens::corner(tokens.shape.radius.sm),
            sizes,
            dividers: tokens.shape.rows == RowStyle::Divided,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        self.adopt_facts(&cx.env, cx.resolved.id);
        let sizes = &cx.style.sizes;
        if self.spacer {
            return Vec2::ZERO;
        }
        let above = if self.space_above { sizes.gap_row } else { 0.0 };
        let height = sizes.height(self.thumb, self.detail_line());
        let rtl = cx.env.direction().is_rtl();
        let room = cx.room.unwrap_or(0.0);
        let columns = self.toggles.iter().map(|toggle| &toggle.column).chain(self.texts.iter().map(|text| &text.column));
        let toggles: f32 = columns.filter_map(|key| self.cells.iter().find(|(column, _)| column == key)).map(|(_, cell)| cell.width()).sum();
        self.measure_hosted(cx);
        let widths = self.widths(sizes, &|run| cx.layout_at(run, f32::INFINITY).size().x);
        let probe = self.place_in(&[], sizes, &widths, Rect::from_min_size(Pos2::ZERO, Vec2::new(room, height)), rtl);
        let probe_min = probe.min_width + toggles;
        let band = cx.room.map_or(f32::INFINITY, |_| (probe.label.width() - toggles).max(0.0));
        let wrapped = self.lines.as_ref().map_or(0.0, |text| {
            let run = TextRun::new(text.as_str()).role(cx.style.detail.role).wrap(self.lines_wrap());
            sizes.gap + cx.layout_at(&run, band).size().y
        });
        let detail = if self.detail_line() { sizes.detail_line } else { 0.0 };
        let text = sizes.label_line + detail + self.progress_line(sizes) + wrapped + self.below_height(sizes);
        let height = if self.lines.is_some() || self.progress.is_some() || !self.below.is_empty() {
            height.max(text + sizes.gap)
        } else {
            height.max(sizes.label_line)
        };
        let height = self
            .texts
            .iter()
            .filter(|cell| cell.wrap)
            .filter_map(|cell| self.cells.iter().find(|(column, _)| *column == cell.column).map(|(_, rect)| (cell, rect)))
            .fold(height, |height, (cell, rect)| {
                let run = TextRun::new(cell.text.as_str()).role(cx.style.cell.role).wrap(Wrap::Wrap);
                height.max(cx.layout_at(&run, sizes.cell_room(rect.width())).size().y + sizes.cell_pad)
            });
        let height = height + above;
        if cx.proposal == Proposal::Min {
            return Vec2::new(probe_min, height);
        }
        let label = cx.layout(&TextRun::new(cx.resolved.label).role(cx.style.text.role)).size().x;
        // The label band holds the detail line under the label, so it is as wide as the wider of the two.
        let label = match self.detail.as_ref().filter(|_| self.detail_line()) {
            Some(detail) => label.max(cx.layout(&TextRun::new(detail.as_str()).role(cx.style.detail.role)).size().x),
            None => label,
        };
        let label = label.max(self.below_width(sizes));
        let own = probe.ideal_width - sizes.min_label + label.max(sizes.min_label) + self.indent_full(sizes);
        if cx.proposal == Proposal::Ideal {
            self.report_cell_widths(cx, own);
        }
        Vec2::new(own + toggles, height)
    }

    fn sense(&self) -> Sense {
        if self.spacer { Sense::empty() } else { Sense::CLICK | Sense::FOCUS }
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        self.record_toggle_names(cx);
        if self.spacer {
            return;
        }
        self.adopt_facts(&cx.env, cx.resolved.id);
        self.measure_hosted(cx);
        let widths = self.widths(&cx.style.sizes, &|run| cx.layout_at(run, f32::INFINITY).size().x);
        let placed = self.place(&cx.style.sizes, &widths, rect, cx.flow.is_rtl());
        for (at, cell) in &placed.toggles {
            let toggle = &self.toggles[*at];
            let mut flags = Flags::empty();
            flags.set(Flags::CHECKED, toggle.state == ToggleState::On);
            flags.set(Flags::MIXED, toggle.state == ToggleState::Mixed);
            let shape = if matches!(toggle.face, ToggleFace::Check) {
                Rect::from_center_size(cell.center(), Vec2::splat(cx.style.sizes.icon))
            } else {
                *cell
            };
            let key = self.toggle_keys.iter().find(|(column, _)| *column == toggle.column).map(|(_, chord)| *chord);
            let tip = match (&toggle.tooltip, key) {
                (Some(tip), None) => Some(tip.clone()),
                (tip, Some(chord)) => {
                    let base = match tip {
                        Some(tip) => cx.env.resolve(tip).as_str().to_owned(),
                        None => named(toggle.name_pattern(&cx.words::<ListWords>()), cx.resolved.label),
                    };
                    Some(Label::from(crate::contract::tooltip::with_chord(&base, &chord, crate::runtime::windows::os_of(cx.ui.ctx()))))
                }
                (None, None) => None,
            };
            let held = match &self.pressed {
                Some(RowPressed::Cell(column)) => *column == toggle.column,
                Some(RowPressed::Row) => toggle.row_press,
                _ => false,
            };
            let part = cx.part(TOGGLE.at(*at as u64), shape).flags(flags).held(held);
            let _ = match tip {
                Some(tip) => part.tooltip(tip),
                None => part,
            };
        }
        for (at, position, _) in &placed.texts {
            if self.cell_selects(*at) {
                cx.selectable_text(TEXT_CELL.at(*position as u64), &self.cell_run(cx.style, *at));
            }
        }
        if let Some(disclosure) = placed.disclosure.filter(|_| !self.disclosure_by_host) {
            let words = cx.words::<ListWords>();
            let tip = if self.group == Some(true) { words.collapse_tip.clone() } else { words.expand_tip.clone() };
            cx.part(DISCLOSURE, disclosure).sense(Sense::CLICK).pointer_only("Left and Right open and close the row").tooltip(tip);
        }
        if self.progress.is_some() {
            cx.part(PROGRESS, self.progress_band(placed.label, &cx.style.sizes));
        }
        // Clamped lines that the clamp cuts show whole in a tooltip of their own, which a cut label's tooltip on the
        // row does not hide.
        if let Some(text) = self.lines.as_ref().filter(|_| self.line_limit.is_some()) {
            let sizes = &cx.style.sizes;
            let run = TextRun::new(text.as_str()).role(cx.style.detail.role).wrap(self.lines_wrap());
            let layout = cx.layout_at(&run, self.text_block(placed.label, sizes, 0.0).0.width());
            if layout.elided {
                let (band, top) = self.text_block(placed.label, sizes, sizes.gap + layout.size().y);
                let detail = if self.detail_line() { sizes.detail_line } else { 0.0 };
                let lines_top = top + sizes.label_line + detail + sizes.gap;
                let rect = Rect::from_x_y_ranges(band.x_range(), lines_top..=lines_top + layout.size().y);
                cx.part(LINES, rect).tooltip(text.clone());
            }
        }
        let mut focusable = false;
        for (at, (_, link)) in placed.extras.iter().enumerate() {
            if let Some(link) = link {
                cx.part(LINK.at(at as u64), *link).sense(Sense::CLICK | Sense::FOCUS).focusable();
                focusable = true;
            }
        }
        let side = cx.style.sizes.icon;
        for (at, (trailing, slot)) in self.trailing.iter().zip(&placed.items).enumerate() {
            let Some(slot) = slot else { continue };
            let shape = Rect::from_center_size(slot.center(), Vec2::splat(side));
            match &trailing.item {
                TrailingItem::Text { name: None, .. } | TrailingItem::Hosted(_) => {}
                TrailingItem::Text { name: Some(name), .. } => {
                    cx.part(TRAILING_TEXT.at(at as u64), *slot).tooltip(name.clone());
                }
                TrailingItem::Icon { name, .. } => {
                    cx.part(TRAILING_ICON.at(at as u64), shape).tooltip(name.clone());
                }
                TrailingItem::Part { part, face, name, enabled, .. } => {
                    // A text face takes its whole slot; the other faces the icon square in it.
                    let shape = if matches!(face, TrailingFace::Strong(_)) { *slot } else { shape };
                    let key = TRAILING.at(u64::from(part.0));
                    if self.drag_at(RowDragSpot::Trailing(*part)).is_some() {
                        cx.part(key, shape)
                            .sense(Sense::CLICK | Sense::DRAG | Sense::PICKS_ITEM)
                            .pointer_only("A drag handle is a pointer target; a click on it is a click on the row")
                            .tooltip(name.clone());
                    } else if *enabled {
                        let name = self.part_name(*part, name, &cx.words::<ListWords>(), cx.resolved.label);
                        cx.part(key, shape).sense(Sense::CLICK | Sense::FOCUS).focusable().tooltip(name);
                        focusable = true;
                    } else {
                        cx.part(key, shape).tooltip(name.clone());
                    }
                }
            }
        }
        if focusable {
            cx.roving(Roving::new(NavShape::Flat));
        }
        let thumbs = placed.first.into_iter().chain(placed.extras.iter().map(|(thumb, _)| *thumb));
        for (at, thumb) in thumbs.enumerate() {
            let target = target_at(at);
            let sense =
                if self.drag_at(RowDragSpot::Thumb(target)).is_some() { Sense::CLICK | Sense::DRAG } else { Sense::CLICK } | Sense::PICKS_ITEM;
            let part = cx.part(THUMB.at(at as u64), thumb).sense(sense).pointer_only("A thumbnail is a pointer target; the keys act on the row");
            if let Some((_, cursor)) = self.cursors.iter().find(|(held, _)| *held == target) {
                part.cursor(cursor.clone());
            }
        }
        if let (Some(slot), Some(_)) = (placed.icon, &self.lead_text) {
            let rect = Rect::from_x_y_ranges(slot.x_range(), rect.y_range());
            cx.part(LEAD, rect).sense(Sense::CLICK | Sense::PICKS_ITEM).pointer_only("The leading text is a pointer target; the keys act on the row");
        }
    }

    fn flags(&self) -> Flags {
        if self.highlight { Flags::EMPHASIZED } else { Flags::empty() }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if self.spacer {
            return;
        }
        if let Some(open) = self.group {
            // The router hands an AccessKit Expand or Collapse on the disclosure to the row only when it changes the
            // state, so it asks what a click does.
            let disclosure: PartRef = DISCLOSURE.into();
            let by_access = cx
                .intents()
                .take_access_where(|request| {
                    request.target == Some(disclosure)
                        && matches!(request.action, craft_behavior::AccessAction::Expand | craft_behavior::AccessAction::Collapse)
                })
                .is_some();
            if by_access || cx.part_activated(DISCLOSURE).is_some() {
                let change = OpenChange::new((), !open);
                self.open = Some(if !by_access && cx.part(DISCLOSURE).press.mods.alt { change.all() } else { change });
            }
        }
        for (at, thumb) in self.extras.iter().enumerate() {
            let part = LINK.at(at as u64);
            if let Some((linked, _)) = &thumb.link
                && cx.part_activated(part).is_some()
            {
                self.event = Some(RowEvent::LinkToggled { thumb: at, linked: !linked });
            }
        }
        for at in 0..=self.extras.len() {
            let Some(result) = cx.part_if_declared(THUMB.at(at as u64)) else { continue };
            let target = target_at(at);
            let phase = result.press.phase;
            if let (PressPhase::DragStart { .. }, Some(payload)) = (phase, self.drag_at(RowDragSpot::Thumb(target))) {
                let preview = DragPreview::frame(self.thumb_size(&cx.style.sizes), None);
                crate::runtime::drag::start(&cx.ctx, cx.id, payload.clone(), None, preview);
                self.event = Some(RowEvent::PartDrag(RowDragSpot::Thumb(target)));
            } else if let Some(source) = cx.part_activated(THUMB.at(at as u64)) {
                let clicks = part_clicks(source, phase);
                self.event = Some(RowEvent::ThumbClicked { target, mods: part_mods(cx, THUMB.at(at as u64), source), clicks });
            }
        }
        if self.lead_text.is_some()
            && let Some(source) = cx.part_activated(LEAD)
        {
            self.event =
                Some(RowEvent::LeadClicked { mods: part_mods(cx, LEAD.into(), source), clicks: part_clicks(source, cx.part(LEAD).press.phase) });
        }
        for trailing in &self.trailing {
            let TrailingItem::Part { part, .. } = &trailing.item else { continue };
            let key = TRAILING.at(u64::from(part.0));
            if let Some(payload) = self.drag_at(RowDragSpot::Trailing(*part)) {
                // A handle's click is the row's: only its drag is its own.
                if let Some(result) = cx.part_if_declared(key)
                    && matches!(result.press.phase, PressPhase::DragStart { .. })
                {
                    let preview = DragPreview::frame(result.rect.size(), None);
                    crate::runtime::drag::start(&cx.ctx, cx.id, payload.clone(), None, preview);
                    self.event = Some(RowEvent::PartDrag(RowDragSpot::Trailing(*part)));
                }
                continue;
            }
            if let Some(source) = cx.part_activated(key) {
                if *part == TrailingPart::MENU {
                    self.menu = Some(cx.part(key).rect);
                } else {
                    let clicks = part_clicks(source, cx.part(key).press.phase);
                    let was_lead = match (source, &self.pressed) {
                        (FireSource::Pointer, Some(RowPressed::Part { was_lead, .. })) => *was_lead,
                        (FireSource::Pointer, _) => false,
                        _ => true,
                    };
                    self.event = Some(RowEvent::Trailing { part: *part, mods: part_mods(cx, key, source), clicks, was_lead });
                }
            }
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        if !self.node {
            access.role(Role::None);
            return;
        }
        if self.spacer {
            access.name(Name::Decorative);
            return;
        }
        let words = cx.words::<ListWords>();
        let label = cx.resolved.label;
        let name = match (&self.accessible_name, &self.detail) {
            (Some(name), _) => Name::Text(name.clone()),
            (None, Some(detail)) => Name::Joined(vec![label.to_owned(), detail.as_str().to_owned()]),
            (None, None) => Name::FromLabel,
        };
        access.name(name);
        // A drag handle is no button: its name is part of the row's description, as a mark's is.
        let handle = |part: &TrailingPart| self.drag_at(RowDragSpot::Trailing(*part)).is_some();
        let icons = self.trailing.iter().filter_map(|trailing| match &trailing.item {
            TrailingItem::Icon { name, .. } | TrailingItem::Text { name: Some(name), .. } => Some(name.as_str().to_owned()),
            TrailingItem::Part { part, name, .. } if handle(part) => Some(name.as_str().to_owned()),
            _ => None,
        });
        let percent = self.progress.map(|fraction| cx.format().percent(f64::from(Progress::clamped(fraction)), 0));
        let described: Vec<String> = self
            .clipped
            .then(|| words.clipped.as_str().to_owned())
            .into_iter()
            .chain(self.common.description.clone())
            .chain(self.lines.as_ref().filter(|_| self.line_limit.is_some()).map(|text| text.as_str().to_owned()))
            .chain(percent)
            .chain(icons)
            .collect();
        if !described.is_empty() {
            access.description(described.join(", "));
        }
        if self.depth.is_some() {
            access.role(Role::TREE_ITEM);
        } else if self.option {
            access.role(Role::LIST_BOX_OPTION);
        }
        if self.progress.is_some() {
            access.node(PROGRESS, Role::PROGRESS_INDICATOR).name = Name::Text(label.to_owned());
        }
        if let Some(open) = self.group {
            access.expanded(open);
            if !self.disclosure_by_host {
                let pattern = if open { &words.collapse } else { &words.expand };
                access.node(DISCLOSURE, Role::BUTTON).name = Name::Text(named(pattern, label));
            }
        }
        let selected =
            cx.resolved.selected == Some(true) || cx.env.facts_of(cx.resolved.id).is_some_and(|facts| facts.flags.contains(Flags::SELECTED));
        // The item step adds the row index; the column index is the cell's position among the row's columns.
        let column_of = |key: &ColumnKey| self.cells.iter().position(|(column, _)| column == key);
        if let Some(column) = self.label_column(&self.cells) {
            let node = access.node(LABEL_CELL, Role::CELL);
            node.name = Name::Text(label.to_owned());
            node.grid.column_index = column_of(column);
            node.flags.set(Flags::SELECTED, selected);
        }
        for (at, toggle) in self.toggles.iter().enumerate() {
            let cell = access.node(TOGGLE_CELL.at(at as u64), Role::CELL);
            cell.name = Name::Text(label.to_owned());
            cell.grid.column_index = column_of(&toggle.column);
            cell.flags.set(Flags::SELECTED, selected);
            let role = if toggle.check_box { Role::CHECK_BOX } else { Role::BUTTON };
            let node = access.node(TOGGLE.at(at as u64), role);
            node.name = Name::Text(named(toggle.name_pattern(&words), label));
            node.under(TOGGLE_CELL.at(at as u64));
        }
        for text in &self.texts {
            let Some(position) = self.cells.iter().position(|(column, _)| *column == text.column) else { continue };
            let node = access.node(TEXT_CELL.at(position as u64), Role::CELL);
            node.name = Name::Text(text.text.as_str().to_owned());
            node.grid.column_index = Some(position);
            node.flags.set(Flags::SELECTED, selected);
        }
        for (at, thumb) in self.extras.iter().enumerate() {
            if let Some((_, name)) = &thumb.link {
                access.node(LINK.at(at as u64), Role::BUTTON).name = Name::Text(named(name, label));
            }
        }
        for (at, trailing) in self.trailing.iter().enumerate() {
            match &trailing.item {
                TrailingItem::Text { .. } | TrailingItem::Hosted(_) => {}
                TrailingItem::Part { part, .. } if handle(part) => {}
                TrailingItem::Icon { name, .. } => {
                    access.node(TRAILING_ICON.at(at as u64), Role::IMAGE).name = Name::Text(name.as_str().to_owned());
                }
                TrailingItem::Part { part, face, name, .. } => {
                    let name = self.part_name(*part, name, &words, label);
                    let node = access.node(TRAILING.at(u64::from(part.0)), Role::BUTTON);
                    node.name = Name::Text(name.as_str().to_owned());
                    if let TrailingFace::Disclosure { open } = face {
                        node.expanded(*open);
                    }
                }
            }
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        if self.spacer {
            return;
        }
        let style = cx.style;
        let sizes = &style.sizes;
        let rtl = cx.env.direction().is_rtl();
        let rect = cx.rect;
        let widths = self.widths(sizes, &|run| ui.draw().layout(run, f32::INFINITY).size().x);
        let placed = self.place(sizes, &widths, rect, rtl);
        self.label_rect = placed.label;
        {
            let draw = ui.draw();
            if self.rule || (self.layer && style.dividers) {
                draw.edge_rule(RuleKind::Divider, rect, RectSide::Bottom);
            }
            if self.layer && style.dividers {
                for x in &placed.dividers {
                    draw.rule(RuleKind::Divider, Pos2::new(*x, rect.top()), Pos2::new(*x, rect.bottom()));
                }
            }
        }
        let lead_painted = self.lead_paint.is_some() || self.lead_text.is_some();
        self.paint_toggles(ui, cx, &placed);
        let ink = cx.resolve(&style.text.color);
        if let (Some(slot), Some((text, _))) = (placed.icon, &self.lead_text) {
            let draw = ui.draw();
            let run = lead_run(text, self.lead_role).color(cx.resolve(&style.detail.color));
            let shown = draw.layout(&run, slot.width());
            let x = if rtl { slot.left() } else { slot.right() - shown.size().x };
            draw.galley(Pos2::new(x, slot.center().y - shown.size().y / 2.0), &shown);
        } else if let (Some(slot), Some(paint)) = (placed.icon, self.lead_paint.take()) {
            let _ = ui.child("lead", Rect::from_x_y_ranges(slot.x_range(), rect.y_range()), RowThumb::new(paint));
        }
        if let Some(open) = self.group
            && placed.disclosure.is_some()
            && !self.disclosure_by_host
        {
            ui.face(DISCLOSURE, Disclosure::new(Bind::given(open)).node(false).triangle(true).label(cx.resolved.label.to_owned()));
        }
        {
            let draw = ui.draw();
            if let Some(slot) = placed.marker {
                draw_clip_arrow(&draw, slot, rtl, ink);
            }
            if let (Some(slot), Some(true)) = (placed.check, self.check) {
                draw.stroke(&glyph::check(Rect::from_center_size(slot.center(), Vec2::splat(sizes.icon))), StrokeWidth::Medium, ink);
            }
            if let Some(slot) = placed.icon
                && let Some(icon) = self.icon.as_mut()
                && !lead_painted
            {
                icon.color.get_or_insert(cx.resolve(&style.icon.color));
                draw.icon(Rect::from_center_size(slot.center(), Vec2::splat(sizes.icon)), icon);
            }
        }
        self.paint_thumbs(ui, cx, &placed, ink);
        let text_end = self.paint_text(&ui.draw(), cx, placed.label);
        let mut from = 0.0;
        for (at, below) in self.below.iter_mut().enumerate() {
            let (Some(item), Some(size)) = (below.item.take(), below.size) else { continue };
            let width = size.ideal.x.min((placed.label.width() - from).max(0.0));
            let x = if rtl { placed.label.right() - from - width } else { placed.label.left() + from };
            item.show(ui, below_key(at), Rect::from_min_size(Pos2::new(x, text_end + sizes.gap), Vec2::new(width, size.ideal.y)));
            from += width + sizes.gap;
        }
        if let Some(fraction) = self.progress {
            ui.face(PROGRESS, Progress::new(fraction));
        }
        self.paint_cells(ui, cx, &placed);
        self.paint_cell_fields(ui, cx, &placed);
        self.paint_trailing(ui, cx, &placed);
    }

    fn finish(self, cx: Cx<'_, Self>) -> RowOutput {
        RowOutput { response: cx.response.clone(), event: self.event, open: self.open, menu: self.menu, label: self.label_rect }
    }
}

/// The press count of an activation: the click's count for a pointer click, 1 for a key or AccessKit activation.
fn part_clicks(source: FireSource, phase: PressPhase) -> u32 {
    match (source, phase) {
        (FireSource::Pointer, PressPhase::Release { start, click: true, .. }) => start.clicks,
        _ => 1,
    }
}

/// The modifiers of an activation of `part`: those held at a click, none for a key or AccessKit activation.
fn part_mods(cx: &Cx<'_, ListRow<'_>>, part: PartRef, source: FireSource) -> Mods {
    match source {
        FireSource::Pointer => cx.part(part).press.mods,
        _ => Mods::NONE,
    }
}

impl ListRow<'_> {
    /// Reports the width of the row's content in each of its columns (`MeasureCx::cell_width`): `own` for the label
    /// column, which holds everything but the toggle and text cells, less the empty toggle columns its band spans,
    /// and a text cell's line with its insets. A toggle cell's column has its own width.
    fn report_cell_widths(&self, cx: &MeasureCx<'_, Self>, own: f32) {
        if let Some((at, first, last)) = self.label_span(&self.cells) {
            let spanned: f32 = (first..=last).filter(|position| *position != at).map(|position| self.cells[position].1.width()).sum();
            cx.cell_width(&self.cells[at].0, (own - spanned).max(0.0));
        }
        let inset = 2.0 * cx.style.sizes.cell_inset;
        for text in &self.texts {
            if self.cells.iter().any(|(column, _)| *column == text.column) {
                let line = cx.layout_at(&TextRun::new(text.text.as_str()).role(cx.style.cell.role), f32::INFINITY).size().x;
                cx.cell_width(&text.column, line + inset);
            }
        }
    }

    /// Measures each hosted component not yet measured in this pass, for its slot.
    fn measure_hosted(&mut self, cx: &MeasureCx<'_, Self>) {
        for (at, trailing) in self.trailing.iter_mut().enumerate() {
            if let TrailingItem::Hosted(hosted) = &mut trailing.item
                && hosted.size.is_none()
                && let Some(item) = hosted.item.as_mut()
            {
                let ideal = item.measure(cx, field_key(at), Proposal::Ideal);
                let least = item.measure(cx, field_key(at), Proposal::Min).x;
                hosted.size = Some(HostedSize { ideal, least });
            }
        }
        for (at, below) in self.below.iter_mut().enumerate() {
            if below.size.is_none()
                && let Some(item) = below.item.as_mut()
            {
                let ideal = item.measure(cx, below_key(at), Proposal::Ideal);
                let least = item.measure(cx, below_key(at), Proposal::Min).x;
                below.size = Some(HostedSize { ideal, least });
            }
        }
        for (at, (_, field)) in self.cell_fields.iter_mut().enumerate() {
            if field.size.is_none()
                && let Some(item) = field.item.as_mut()
            {
                let ideal = item.measure(cx, cell_key(at), Proposal::Ideal);
                let least = item.measure(cx, cell_key(at), Proposal::Min).x;
                field.size = Some(HostedSize { ideal, least });
            }
        }
    }

    /// The name of the trailing part `part`: the one given, or "More ({row})" for the row's menu button.
    fn part_name(&self, part: TrailingPart, name: &Label, words: &ListWords, row: &str) -> Label {
        if part == TrailingPart::MENU { Label::from(named(&words.more, row)) } else { name.clone() }
    }
}
