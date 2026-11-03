//! [D] Iconic columns: one icon per panel, groups apart, and the flyout an icon opens.
//!
//! Behavior follows VectorCraft `dock.rs` (icon column, flyouts beside it on the canvas side,
//! names when the column is widened) and PhotoCraft `dock.rs` (Auto-Collapse Iconic Panels).

use std::{fmt::Debug, hash::Hash};

use egui::{Context, Id, Key, Order, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

use super::{GroupKind, Renderer, dock::Edge};
use crate::docking::{
    Column, DockStyle, Group, GroupDrag, GroupGesture, begin_drag,
    chrome::{ChromeStyle, FloatingFrame},
    is_dragging,
    targets::{DockDrop, DockTarget, horizontal_line, usable},
};

/// How far an icon group's drag band reaches into its first icon. The band also covers the
/// separator room above the group; it reserves no height of its own.
const GROUP_BAND: f32 = 4.0;
/// Room between two icon groups, with the separator line in its middle.
const SEPARATOR: f32 = 7.0;
/// Height of a flyout, or of a panel floated from an icon, when nothing else sizes it.
const FLYOUT_HEIGHT: f32 = 320.0;

/// The panel whose icon opened the flyout of an area.
#[derive(Clone)]
struct Open<P>(P);

fn key(area: Id) -> Id {
    area.with("flyout")
}

/// [D] Open the flyout of the group holding `panel`, when that group is in an iconic column.
/// The flyout shows from the next frame on. [`crate::docking::DockArea::raise_panel`] does the
/// same for a panel the Window menu opens.
pub fn open_flyout<P: Clone + Send + Sync + 'static>(ctx: &Context, area_id: Id, panel: P) {
    ctx.data_mut(|d| d.insert_temp(key(area_id), Open(panel)));
}

/// [D] Close the area's flyout.
pub fn close_flyout<P: Clone + Send + Sync + 'static>(ctx: &Context, area_id: Id) {
    ctx.data_mut(|d| d.remove::<Open<P>>(key(area_id)));
}

/// [D] The panel whose icon opened the area's flyout, while one is open.
pub fn flyout<P: Clone + Send + Sync + 'static>(ctx: &Context, area_id: Id) -> Option<P> {
    ctx.data(|d| d.get_temp::<Open<P>>(key(area_id))).map(|open| open.0)
}

/// The flyout last reported by [`crate::docking::DockEvent::FlyoutChanged`].
#[derive(Clone)]
struct Reported<P>(Option<P>);

/// The flyout an iconic column asks for this frame.
struct Request<P> {
    group: Group<P>,
    /// The icons of the group, where the flyout's top aligns.
    icons: Rect,
    /// The iconic column, which a press does not close the flyout in.
    column: Rect,
    edge: Edge,
    /// The column's expanded width.
    width: f32,
}

/// Per-frame iconic state of a renderer.
pub(in crate::docking) struct Iconic<P> {
    auto_collapse: bool,
    /// Iconic column bodies drawn this frame, outlined by a drop into them (P9).
    columns: Vec<Rect>,
    /// Expanded column width of each panel drawn as an icon: a panel floated from its icon
    /// takes it.
    widths: Vec<(P, f32)>,
    request: Option<Request<P>>,
    /// The flyout drawn this frame; it occludes dock targets behind it.
    rect: Option<Rect>,
}

impl<P: PartialEq> Iconic<P> {
    pub(in crate::docking) fn new(auto_collapse: bool) -> Self {
        Self { auto_collapse, columns: Vec::new(), widths: Vec::new(), request: None, rect: None }
    }

    pub(super) fn covers(&self, pointer: Pos2) -> bool {
        self.rect.is_some_and(|rect| rect.contains(pointer))
    }

    /// Outline the iconic column an insertion line lies in.
    pub(super) fn outline(&self, painter: &egui::Painter, line: Rect, style: &DockStyle) {
        if let Some(column) = self.columns.iter().find(|column| column.shrink(1.0).contains(line.center())) {
            painter.rect_stroke(column.shrink(1.0), 0.0, Stroke::new(2.0, style.accent), StrokeKind::Inside);
        }
    }

    pub(super) fn float_width(&self, panel: &P) -> Option<f32> {
        self.widths.iter().find(|(p, _)| p == panel).map(|(_, width)| *width)
    }
}

impl<P: Clone + Eq + Hash + Debug + Send + Sync + 'static> Renderer<'_, P> {
    fn row_height(&self) -> f32 {
        self.style.icon_size + 10.0
    }

    fn icon_group_height(&self, group: &Group<P>) -> f32 {
        group.panels.len() as f32 * self.row_height()
    }

    /// The body of an iconic column: icon groups top to bottom, scrolling when they do not fit.
    pub(super) fn iconic_column(&mut self, ui: &mut Ui, column: &Column<P>, body: Rect, edge: Edge) {
        self.iconic.columns.push(body);
        for panel in column.groups.iter().flat_map(|g| g.panels.iter()) {
            self.iconic.widths.push((panel.clone(), column.width));
        }
        // Opening a panel from the Window menu opens its flyout (U3).
        if let Some(panel) = self.activation.as_ref().filter(|p| column.groups.iter().any(|g| g.panels.contains(p))) {
            open_flyout(ui.ctx(), self.id, panel.clone());
        }
        let total: f32 =
            column.groups.iter().map(|g| self.icon_group_height(g)).sum::<f32>() + column.groups.len().saturating_sub(1) as f32 * SEPARATOR;
        if total > body.height() + 0.5 {
            let Some(first) = column.groups.first().and_then(|g| g.panels.first()) else { return };
            let mut child =
                ui.new_child(UiBuilder::new().id_salt(("icon-column", first)).max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
            child.set_clip_rect(body.intersect(ui.clip_rect()));
            egui::ScrollArea::vertical()
                .id_salt(self.id.with(("icon-scroll", first)))
                .auto_shrink([false, false])
                .scroll_source(egui::scroll_area::ScrollSource::SCROLL_BAR | egui::scroll_area::ScrollSource::MOUSE_WHEEL)
                .show(&mut child, |ui| {
                    let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), total), Sense::hover());
                    self.icon_groups(ui, column, area, body, edge);
                });
        } else {
            self.icon_groups(ui, column, body, body, edge);
        }
    }

    /// Icon groups in `area` (the scrolled content when the column scrolls), their drop bands
    /// clipped to the visible `view`.
    fn icon_groups(&mut self, ui: &mut Ui, column: &Column<P>, area: Rect, view: Rect, edge: Edge) {
        let open = flyout::<P>(ui.ctx(), self.id);
        let labels = view.width() >= self.style.iconic_label_width;
        let xs = view.x_range();
        let mut top = area.top();
        let mut bands = Vec::new();
        for (index, group) in column.groups.iter().enumerate() {
            let Some(first) = group.panels.first() else { continue };
            let band_top = if index > 0 { top } else { area.top() };
            if index > 0 {
                let y = top + SEPARATOR * 0.5;
                ui.painter().with_clip_rect(view).hline(area.x_range().shrink(4.0), y, self.style.border);
                top += SEPARATOR;
            }
            let rect = Rect::from_min_size(pos2(area.left(), top), vec2(area.width(), self.icon_group_height(group)));
            let showing = open.as_ref().filter(|p| group.panels.contains(p));
            let mut y = rect.top();
            for panel in &group.panels {
                let cell = Rect::from_min_size(pos2(rect.left(), y), vec2(rect.width(), self.row_height()));
                self.icon(ui, panel, cell, labels, showing);
                y = cell.bottom();
            }
            // After the icons, so the band wins the top of the first icon.
            let band = Rect::from_min_max(pos2(rect.left(), band_top), pos2(rect.right(), (rect.top() + GROUP_BAND).min(rect.bottom())));
            self.icon_grip(ui, group, band, rect.top(), column.width);
            // The state again: a click above may have opened or closed this group's flyout.
            if flyout::<P>(ui.ctx(), self.id).is_some_and(|p| group.panels.contains(&p)) {
                self.iconic.request = Some(Request { group: group.clone(), icons: rect.intersect(view), column: view, edge, width: column.width });
            }
            bands.push((first.clone(), rect));
            top = rect.bottom();
        }
        // P9: the upper half of an icon group inserts above it, the lower half and the room
        // under the last group below it. The line sits on the group's edge.
        let count = bands.len();
        for (index, (anchor, rect)) in bands.into_iter().enumerate() {
            let middle = rect.center().y;
            let upper_top = if index == 0 { view.top() } else { rect.top() - SEPARATOR * 0.5 };
            let lower_bottom = if index + 1 == count { view.bottom() } else { rect.bottom() + SEPARATOR * 0.5 };
            for (below, ys, line) in [(false, upper_top..=middle, rect.top()), (true, middle..=lower_bottom, rect.bottom())] {
                let band = Rect::from_x_y_ranges(xs, ys).intersect(view);
                if usable(band) {
                    let width = self.style.drop_line_width;
                    let line = horizontal_line(line.clamp(view.top() + width * 0.5, view.bottom() - width * 0.5), xs, width);
                    self.dock_targets.push(DockDrop { target: DockTarget::GroupBand { anchor: anchor.clone(), below }, band, line, outer: false });
                }
            }
        }
    }

    /// The band on an icon group's top edge drags the whole group (S8). `top` is the group's
    /// first icon's top, where a floated group's source starts.
    fn icon_grip(&mut self, ui: &mut Ui, group: &Group<P>, grip: Rect, top: f32, width: f32) {
        let Some(first) = group.panels.first() else { return };
        let movable = group.panels.iter().all(|p| self.permissions(p).movable);
        let response = ui.interact(grip, self.id.with(("icon-grip", Some(first))), Sense::drag());
        let response = if movable { response.on_hover_cursor(egui::CursorIcon::Grab) } else { response };
        self.register_interaction(ui.ctx(), response.id);
        let height = group.height.filter(|h| h.is_finite()).unwrap_or(FLYOUT_HEIGHT);
        let source = Rect::from_min_size(pos2(grip.left(), top), vec2(width.max(self.style.min_pane), height.max(self.style.tab_height)));
        self.record_float(&group.panels, vec2(grip.width(), self.row_height()), source.size());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &self.style.move_group_label));
        if movable && response.drag_started() {
            let origin = ui.input(|i| i.pointer.press_origin()).or_else(|| ui.ctx().pointer_interact_pos()).unwrap_or(grip.center());
            let drag = GroupDrag {
                panel: first.clone(),
                original: [source.left(), source.top(), source.width(), source.height()],
                gesture: GroupGesture::Group,
                members: group.panels.clone(),
                origin,
                source,
            };
            ui.ctx().data_mut(|d| d.insert_temp(self.id.with("group-drag"), drag));
            close_flyout::<P>(ui.ctx(), self.id);
        }
    }

    /// One panel's icon: a click opens, switches or closes the flyout (M5); a drag moves the
    /// panel (S7). `showing` is the panel whose icon opened this group's flyout.
    fn icon(&mut self, ui: &mut Ui, panel: &P, cell: Rect, labels: bool, showing: Option<&P>) {
        let movable = self.permissions(panel).movable;
        let label = self.label_of(panel);
        let response = ui.interact(cell, self.id.with(("icon", panel)), if movable { Sense::click_and_drag() } else { Sense::click() });
        self.register_interaction(ui.ctx(), response.id);
        let current = showing == Some(panel);
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), current, &label));
        let painter = ui.painter().with_clip_rect(cell.intersect(ui.clip_rect()));
        if current || response.hovered() {
            painter.rect_filled(cell.shrink(2.0), 3.0, self.style.active_background);
        }
        if response.has_focus() {
            painter.rect_stroke(cell.shrink(1.0), 3.0, Stroke::new(1.0, self.style.accent), StrokeKind::Inside);
        }
        let size = self.style.icon_size;
        let inset = ((self.style.iconic_width - size) * 0.5).max(2.0);
        let icon = Rect::from_min_size(pos2(cell.left() + inset, cell.center().y - size * 0.5), vec2(size, size));
        let color = if current { self.style.text } else { self.style.inactive_text };
        if !self.content.paint_icon(&painter, icon, panel, color) {
            if let Some(source) = self.content.icon(panel) {
                egui::Image::new(source).tint(color).paint_at(ui, icon.intersect(cell));
            } else {
                let initial: String = label.chars().next().into_iter().flat_map(char::to_uppercase).collect();
                painter.rect_stroke(icon, 3.0, Stroke::new(1.0, color), StrokeKind::Inside);
                painter.text(icon.center(), egui::Align2::CENTER_CENTER, initial, self.style.font.clone(), color);
            }
        }
        if labels {
            let text = self.content.label(ui, panel, &label, &self.style.font, color);
            let galley =
                text.into_galley(ui, Some(egui::TextWrapMode::Truncate), (cell.right() - icon.right() - 12.0).max(0.0), self.style.font.clone());
            painter.galley(pos2(icon.right() + 6.0, cell.center().y - galley.size().y * 0.5), galley, color);
        } else if response.hovered() && !response.dragged() {
            response.clone().on_hover_text(&label);
        }
        if response.clicked() {
            if current {
                close_flyout::<P>(ui.ctx(), self.id);
            } else {
                if showing.is_none() {
                    ui.ctx().move_to_top(FloatingFrame::layer(self.id.with("flyout")));
                }
                open_flyout(ui.ctx(), self.id, panel.clone());
                self.output.actions.push(crate::docking::Action::Activate { panel: panel.clone() });
                self.output.focused = Some(panel.clone());
            }
        }
        if movable && response.drag_started() && begin_drag(ui.ctx(), self.id, panel.clone(), cell) {
            close_flyout::<P>(ui.ctx(), self.id);
        }
    }

    /// The Auto-Collapse Iconic Panels item of a column menu (U1): true when chosen.
    pub(super) fn auto_collapse_item(&mut self, ui: &mut Ui) -> bool {
        let logical = self.style.auto_collapse_label.clone();
        let text = self.content.action_label(ui, &logical, &self.style.font, self.style.text);
        let mut on = self.iconic.auto_collapse;
        let response = ui.add(egui::Checkbox::new(&mut on, text));
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), self.iconic.auto_collapse, &logical));
        if response.clicked() {
            ui.close();
        }
        response.clicked()
    }

    /// Draw the flyout an iconic column asked for, beside the column on the canvas side, or
    /// forget a flyout whose group is no longer iconic. With Auto-Collapse, Escape or a press
    /// outside the flyout and its column closes it.
    pub(in crate::docking) fn flyout(&mut self, ctx: &Context) {
        let Some(request) = self.iconic.request.take() else {
            close_flyout::<P>(ctx, self.id);
            return;
        };
        let Some(active) = request.group.active_panel().cloned() else { return };
        let id = self.id.with("flyout");
        if self.iconic.auto_collapse && self.flyout_dismissed(ctx, id, request.column) {
            close_flyout::<P>(ctx, self.id);
            return;
        }
        let bounds = self.workspace.intersect(self.float_bounds);
        let width = request.width.max(self.style.min_pane).min(bounds.width());
        let max = self
            .content
            .floating_size(&active)
            .map(|size| size.y)
            .or(request.group.height)
            .filter(|h| h.is_finite() && *h > 0.0)
            .unwrap_or(FLYOUT_HEIGHT);
        // A flyout fits the content its panel last drew, up to that height.
        let height = self
            .fitted_height(&active, self.style.tab_height, max)
            .unwrap_or(max)
            .max(self.style.tab_height + self.style.min_pane)
            .min(bounds.height());
        let left = match request.edge {
            Edge::Left => request.column.right(),
            Edge::Right => request.column.left() - width,
        };
        let top = request.icons.top().min(bounds.bottom() - height).max(bounds.top());
        let rect = Rect::from_min_size(pos2(left, top), vec2(width, height));
        let style = ChromeStyle::from_dock(self.style);
        let group = request.group;
        let response = FloatingFrame::new(id, rect).bounds(bounds).resizable(false).show(ctx, &style, |ui, rect| {
            self.group(ui, &group.panels, group.active, false, rect, GroupKind::Dock);
        });
        self.iconic.rect = Some(response.rect);
    }

    /// Report a flyout that differs from the last one reported.
    pub(in crate::docking) fn report_flyout(&mut self, ctx: &Context) {
        let shown = flyout::<P>(ctx, self.id);
        let key = self.id.with("flyout-reported");
        let reported = ctx.data(|d| d.get_temp::<Reported<P>>(key)).and_then(|r| r.0);
        if shown != reported {
            ctx.data_mut(|d| d.insert_temp(key, Reported(shown.clone())));
            self.output.events.push(crate::docking::DockEvent::FlyoutChanged { panel: shown });
        }
    }

    /// Escape (outside a drag) or a primary press outside the flyout, its column and any menu.
    fn flyout_dismissed(&self, ctx: &Context, id: Id, column: Rect) -> bool {
        let escape = !is_dragging::<P>(ctx, self.id) && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
        let press = ctx.input(|i| i.pointer.primary_pressed().then(|| i.pointer.press_origin()).flatten());
        let outside = press.is_some_and(|pos| {
            let inside = ctx.memory(|m| m.area_rect(id)).is_some_and(|r| r.contains(pos)) || column.contains(pos);
            let menu = ctx.layer_id_at(pos).is_some_and(|layer| matches!(layer.order, Order::Foreground | Order::Tooltip));
            !inside && !menu
        });
        escape || outside
    }
}
