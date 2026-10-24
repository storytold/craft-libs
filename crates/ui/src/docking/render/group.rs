use std::{fmt::Debug, hash::Hash, sync::Arc};

use egui::{Color32, Galley, Id, Key, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

use super::{Renderer, clamp_float, stripless};
use crate::docking::{Action, DropTarget, GroupDrag, GroupGesture, Placement, Target, begin_drag, surface::TAB_PADDING};
use crate::tabs;

/// Where a group is drawn. It decides the drop targets the group offers and its gestures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::docking) enum GroupKind {
    /// A `Tabs` node of the split tree: side drops split it.
    Frame,
    /// A group in a dock column.
    Dock,
    /// A `Tabs` node of the floating window drawn at `window`: side drops split it.
    Floating { window: Rect },
}

/// The group a tab belongs to, for its menu and minimize gestures.
pub(super) struct TabGroup<'a, P> {
    pub(super) panels: &'a [P],
    pub(super) selected: &'a P,
    pub(super) minimized: bool,
    pub(super) kind: GroupKind,
    pub(super) rect: Rect,
}

/// The top band of a strip that drags the whole group.
const BORDER_BAND: f32 = 4.0;

/// Width of the "…" button that lists the panels a strip has no room for.
const OVERFLOW_WIDTH: f32 = 22.0;

/// Points of smooth (trackpad) scrolling that make one wheel notch.
const WHEEL_NOTCH: f32 = 40.0;

impl<P: Clone + Eq + Hash + Debug + Send + Sync + 'static> Renderer<'_, P> {
    pub(super) fn group(&mut self, ui: &mut Ui, panels: &[P], active: usize, minimized: bool, rect: Rect, kind: GroupKind) {
        let Some(selected) = panels.get(active) else { return };
        self.record_float(panels, rect.size(), rect.size());
        let policy = self.permissions(selected);
        let splits = matches!(kind, GroupKind::Frame | GroupKind::Floating { .. }) && policy.accepts_splits;
        let rtl = self.style.right_to_left;
        if stripless(panels, &*self.content) {
            self.contents(ui, selected, rect);
            self.targets.push(DropTarget {
                anchor: selected.clone(),
                rect,
                tab_strip: None,
                tabs: vec![(selected.clone(), Rect::NOTHING)],
                allow_splits: splits,
                accepts_tabs: policy.accepts_tabs,
                right_to_left: rtl,
            });
            return;
        }
        let movable = panels.iter().all(|p| self.permissions(p).movable);
        let strip = Rect::from_min_max(rect.min, pos2(rect.right(), (rect.top() + self.style.tab_height).min(rect.bottom())));
        ui.painter().rect_filled(strip, 0.0, self.style.tab_background);
        let floating = matches!(kind, GroupKind::Floating { .. });
        // Laid out left to right, then mirrored in a right-to-left layout: tabs from the right
        // edge, the header (the host's ≡) at the left.
        let mirror = |r: Rect| {
            if rtl {
                Rect::from_x_y_ranges((strip.left() + strip.right() - r.right())..=(strip.left() + strip.right() - r.left()), r.y_range())
            } else {
                r
            }
        };
        let requested = self.content.group_header_width(panels, selected, floating, strip.width());
        let header_width = if requested.is_finite() { requested.clamp(0.0, strip.width()) } else { 0.0 };
        if header_width > 0.0 {
            let header = mirror(Rect::from_min_max(pos2(strip.right() - header_width, strip.top()), strip.max));
            let mut child = ui.new_child(UiBuilder::new().id(self.id.with(("header", selected))).max_rect(header));
            child.set_clip_rect(header.intersect(ui.clip_rect()));
            self.content.group_header(&mut child, panels, selected, floating);
            // A press in the header focuses the shown panel, as a press on its tab does.
            if child.rect_contains_pointer(header) && child.input(|i| i.pointer.any_pressed()) {
                self.output.focused = Some(selected.clone());
            }
        }
        // The drag bar keeps its reserved width: tabs shrink, then overflow, before it narrows.
        let bar_width = self.style.drag_bar_width.clamp(0.0, (strip.width() - header_width).max(0.0));
        let tabs_width = (strip.width() - header_width - bar_width).max(0.0);
        let widths: Vec<f32> = panels
            .iter()
            .map(|p| {
                self.content
                    .label(ui, p, &self.label_of(p), &self.style.font, self.style.text)
                    .into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, self.style.font.clone())
                    .size()
                    .x
                    + TAB_PADDING
            })
            .collect();
        // The widest tabs shrink first, to a common width no narrower than `min_tab_width`; only
        // when every tab at that width cannot fit do the rest go to the overflow menu. A tab
        // narrower than its name cuts it with "…" and names it in a tooltip.
        let fit = tabs::fit(&widths, active, tabs_width, self.style.min_tab_width, OVERFLOW_WIDTH);
        let owner = TabGroup { panels, selected, minimized, kind, rect };
        let mut x = strip.left();
        let mut shown = Vec::new();
        for (index, width) in &fit.shown {
            let Some(panel) = panels.get(*index) else { continue };
            let tab = mirror(Rect::from_min_size(pos2(x, strip.top()), vec2(*width, strip.height())));
            self.tab(ui, panel, tab, *index == active, rect, Some(&owner));
            shown.push((panel.clone(), tab));
            x += width;
        }
        if !fit.overflow.is_empty() && fit.overflow_width > 0.0 {
            let overflow = mirror(Rect::from_min_size(pos2(x, strip.top()), vec2(fit.overflow_width, strip.height())));
            let response = ui.interact(overflow, self.id.with(("overflow", selected)), Sense::click());
            self.register_interaction(ui.ctx(), response.id);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &self.style.panels_label));
            ui.painter().with_clip_rect(overflow).text(overflow.center(), egui::Align2::CENTER_CENTER, "…", self.style.font.clone(), self.style.text);
            egui::Popup::menu(&response).show(|ui| {
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    for index in &fit.overflow {
                        if let Some(panel) = panels.get(*index)
                            && self.menu_label(ui, panel).clicked()
                        {
                            self.output.actions.push(Action::Activate { panel: panel.clone() });
                            self.output.focused = Some(panel.clone());
                            ui.close();
                        }
                    }
                });
            });
            x += fit.overflow_width;
        }
        let bar =
            mirror(Rect::from_min_max(pos2(x.min(strip.right() - header_width), strip.top()), pos2(strip.right() - header_width, strip.bottom())));
        self.group_handle(ui, "drag-bar", bar, &owner, movable);
        // Registered last, so the band wins over the tabs below it.
        let border = mirror(Rect::from_min_max(strip.min, pos2(strip.right() - header_width, (strip.top() + BORDER_BAND).min(strip.bottom()))));
        self.group_handle(ui, "drag-border", border, &owner, movable);
        if self.style.wheel_selects_tabs {
            let tabs_area = mirror(Rect::from_min_max(strip.min, pos2(strip.right() - header_width, strip.bottom())));
            self.wheel_tabs(ui, tabs_area, panels, active);
        }
        // Logical order, hidden tabs included without a hit area, for no-op detection.
        let tabs =
            panels.iter().map(|panel| (panel.clone(), shown.iter().find(|(p, _)| p == panel).map_or(Rect::NOTHING, |(_, rect)| *rect))).collect();
        self.targets.push(DropTarget {
            anchor: selected.clone(),
            rect,
            tab_strip: Some(strip),
            tabs,
            allow_splits: splits,
            accepts_tabs: policy.accepts_tabs,
            right_to_left: rtl,
        });
        if !minimized {
            self.contents(ui, selected, Rect::from_min_max(pos2(rect.left(), strip.bottom()), rect.max));
        }
        ui.painter().rect_stroke(rect, 0.0, self.style.border, StrokeKind::Inside);
    }

    /// The drag bar (`drag-bar`: drag, double-click, and the group menu on a right click, Enter
    /// or Space) or the top border band (`drag-border`: drag only). Both move the whole group;
    /// a left click on the bar only focuses it.
    fn group_handle(&mut self, ui: &mut Ui, name: &'static str, hit: Rect, owner: &TabGroup<'_, P>, movable: bool) {
        if !crate::docking::targets::usable(hit) {
            return;
        }
        let bar = name == "drag-bar";
        let sense = if bar { Sense::click_and_drag() } else { Sense::drag() };
        let response = ui.interact(hit, self.id.with((name, owner.panels.first())), sense);
        let response = if movable { response.on_hover_cursor(egui::CursorIcon::Grab) } else { response };
        self.register_interaction(ui.ctx(), response.id);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &self.style.move_group_label));
        if movable && response.drag_started() {
            self.start_group_drag(ui, owner.panels, self.drag_source(owner));
        }
        if !bar {
            return;
        }
        if self.minimizes(owner) && response.double_clicked() {
            self.output.actions.push(Action::SetMinimized { panel: owner.selected.clone(), minimized: !owner.minimized });
        }
        let active = response.has_focus() || response.hovered();
        if active {
            ui.painter().with_clip_rect(hit).rect_stroke(
                hit.shrink(1.0),
                0.0,
                Stroke::new(1.0, self.style.accent.gamma_multiply(0.5)),
                StrokeKind::Inside,
            );
        }
        let keyboard = response.has_focus() && ui.input(|i| i.key_pressed(Key::Enter) || i.key_pressed(Key::Space));
        let command = if response.secondary_clicked() || keyboard {
            Some(egui::SetOpenCommand::Bool(true))
        } else if response.clicked() {
            Some(egui::SetOpenCommand::Bool(false))
        } else {
            None
        };
        egui::Popup::menu(&response).open_memory(command).show(|ui| self.group_menu(ui, owner, owner.selected));
    }

    /// The wheel over a group's strip: down selects the next tab, up the previous, one tab per
    /// notch, stopping at the ends. Trackpad scrolling adds up to a notch.
    fn wheel_tabs(&mut self, ui: &Ui, strip: Rect, panels: &[P], active: usize) {
        let key = self.id.with(("tab-wheel", panels.first()));
        if !ui.rect_contains_pointer(strip) {
            ui.ctx().data_mut(|d| d.remove::<f32>(key));
            return;
        }
        let notches: f32 = ui.input(|i| {
            i.events
                .iter()
                .map(|event| match event {
                    egui::Event::MouseWheel { unit, delta, modifiers, .. } if !modifiers.command && !modifiers.ctrl => match unit {
                        egui::MouseWheelUnit::Point => delta.y / WHEEL_NOTCH,
                        egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => delta.y,
                    },
                    _ => 0.0,
                })
                .sum()
        });
        if notches == 0.0 || !notches.is_finite() {
            return;
        }
        let total = ui.ctx().data(|d| d.get_temp::<f32>(key)).unwrap_or(0.0) + notches;
        let steps = total.trunc();
        ui.ctx().data_mut(|d| d.insert_temp(key, total - steps));
        // A wheel turned down scrolls content up: a negative delta selects the next tab.
        let to = (active as i64).saturating_sub(steps as i64).clamp(0, panels.len().saturating_sub(1) as i64) as usize;
        if to != active
            && let Some(panel) = panels.get(to)
        {
            self.output.actions.push(Action::Activate { panel: panel.clone() });
            self.output.focused = Some(panel.clone());
        }
    }

    fn start_group_drag(&mut self, ui: &Ui, panels: &[P], rect: Rect) {
        let Some(first) = panels.first() else { return };
        let origin = ui.input(|i| i.pointer.press_origin()).or_else(|| ui.ctx().pointer_interact_pos()).unwrap_or(rect.center());
        let drag = GroupDrag {
            panel: first.clone(),
            original: [rect.left(), rect.top(), rect.width(), rect.height()],
            gesture: GroupGesture::Group,
            members: panels.to_vec(),
            origin,
            source: rect,
        };
        ui.ctx().data_mut(|d| d.insert_temp(self.id.with("group-drag"), drag));
    }

    /// The group items of the menu tail: Close Tab Group, Minimize or Expand, Float Tab Group
    /// and Move Group To. `panel` is the panel whose menu this is.
    fn group_menu(&mut self, ui: &mut Ui, owner: &TabGroup<'_, P>, panel: &P) {
        let all = |check: &dyn Fn(crate::docking::Permissions) -> bool| owner.panels.iter().all(|p| check(self.permissions(p)));
        let closable = self.style.close_menu_items && all(&|p| p.closable);
        let movable = all(&|p| p.movable);
        let floatable = movable && all(&|p| p.floatable);
        if closable && self.menu_action(ui, &self.style.close_group_label.clone()) {
            self.output.actions.push(Action::CloseGroup { panel: panel.clone() });
        }
        if self.minimizes(owner) {
            let label = if owner.minimized { self.style.expand_label.clone() } else { self.style.minimize_label.clone() };
            if self.menu_action(ui, &label) {
                self.output.actions.push(Action::SetMinimized { panel: panel.clone(), minimized: !owner.minimized });
            }
        }
        if owner.kind == GroupKind::Dock && self.menu_action(ui, &self.style.collapse_to_icons_label.clone()) {
            self.output.actions.push(Action::SetIconic { panel: panel.clone(), iconic: true });
        }
        let whole_window = matches!(owner.kind, GroupKind::Floating { .. }) && self.whole_window(panel);
        if floatable && !whole_window && self.menu_action(ui, &self.style.float_group_label.clone()) {
            let size = self.group_float_size(owner.panels, owner.rect.size());
            let rect = clamp_float(Rect::from_min_size(owner.rect.min + vec2(24.0, 24.0), size), self.float_bounds);
            self.output.actions.push(Action::MoveGroup {
                panel: panel.clone(),
                to: Target::Float { rect: Some([rect.left(), rect.top(), rect.width(), rect.height()]) },
            });
        }
        let targets = if movable { self.group_targets(owner.panels) } else { Vec::new() };
        if !targets.is_empty() {
            let logical = self.style.move_group_to_label.clone();
            let text = self.content.action_label(ui, &logical, &self.style.font, self.style.text);
            let response = ui.menu_button(text, |ui| {
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    for target in &targets {
                        if self.menu_label(ui, target).clicked() {
                            self.output.actions.push(Action::MoveGroup {
                                panel: panel.clone(),
                                to: Target::At { anchor: target.clone(), placement: Placement::Tab { before: None } },
                            });
                            ui.close();
                        }
                    }
                });
            });
            response.response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &logical));
        }
        self.frame_items(ui, owner, panel);
    }

    /// A stock menu button with shaped text and its logical accessibility label; true when
    /// clicked, after closing the menu.
    pub(super) fn menu_action(&mut self, ui: &mut Ui, logical: &str) -> bool {
        let text = self.content.action_label(ui, logical, &self.style.font, self.style.text);
        let response = ui.button(text);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), logical));
        if response.clicked() {
            ui.close();
        }
        response.clicked()
    }

    /// One anchor per other group that accepts a whole group as tabs, in layout order.
    fn group_targets(&self, members: &[P]) -> Vec<P> {
        let Some(first) = members.first() else { return Vec::new() };
        let mut places = Vec::new();
        let mut anchors = Vec::new();
        for anchor in self.layout.move_targets(first, |p| self.permissions(p)) {
            if members.contains(anchor) {
                continue;
            }
            let Some(place) = self.place(anchor) else { continue };
            if !places.contains(&place) {
                places.push(place);
                anchors.push(anchor.clone());
            }
        }
        anchors
    }

    pub(super) fn menu_label(&mut self, ui: &mut Ui, panel: &P) -> egui::Response {
        let logical = self.label_of(panel);
        let text = self.content.label(ui, panel, &logical, &self.style.font, self.style.text);
        let response = ui.button(text);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &logical));
        response
    }

    /// A tab's label, cut at the end with '…' to fit `width`.
    pub(super) fn tab_label(&mut self, ui: &Ui, panel: &P, label: &str, color: Color32, width: f32) -> Arc<Galley> {
        let text = self.content.label(ui, panel, label, &self.style.font, color);
        crate::docking::chrome::truncated(ui, text, width, self.style.font.clone())
    }

    /// True while a tab's focus came from the pointer, which shows no focus outline. Tab or an
    /// arrow key (keyboard navigation) clears it.
    fn pointer_focus(&self, ui: &Ui, tab: Id) -> bool {
        let key = tab.with("pointer-focus");
        let navigated = ui.input(|i| [Key::Tab, Key::ArrowLeft, Key::ArrowRight, Key::ArrowUp, Key::ArrowDown].iter().any(|k| i.key_pressed(*k)));
        if navigated {
            ui.ctx().data_mut(|d| d.remove_temp::<bool>(key));
        }
        ui.ctx().data(|d| d.get_temp::<bool>(key)).unwrap_or(false)
    }

    pub(super) fn tab(&mut self, ui: &mut Ui, panel: &P, rect: Rect, active: bool, group: Rect, owner: Option<&TabGroup<'_, P>>) {
        let minimized = owner.filter(|owner| self.minimizes(owner)).map(|owner| owner.minimized);
        let policy = self.permissions(panel);
        let label = self.label_of(panel);
        let color = if active { self.style.text } else { self.style.inactive_text };
        let galley = self.tab_label(ui, panel, &label, color, (rect.width() - TAB_PADDING).max(0.0));
        let elided = galley.elided;
        let tab_id = self.id.with(("tab", panel));
        let pointer_focus = self.pointer_focus(ui, tab_id);
        let response = tabs::Tab::new(tab_id, &label, active)
            .sense(if policy.movable { Sense::click_and_drag() } else { Sense::click() })
            .focus_stroke(if pointer_focus { Stroke::NONE } else { Stroke::new(1.0, self.style.accent) })
            .show_at(ui, rect, |ui, response| {
                let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
                painter.rect_filled(rect, 0.0, if active || response.hovered() { self.style.active_background } else { self.style.tab_background });
                let at = pos2(rect.left() + TAB_PADDING * 0.5, rect.center().y - galley.size().y * 0.5);
                painter.galley(at, galley.clone(), color);
            });
        let response = if elided { response.on_hover_text(&label) } else { response };
        self.register_interaction(ui.ctx(), response.id);
        if response.is_pointer_button_down_on() || response.clicked() {
            ui.ctx().data_mut(|d| d.insert_temp(tab_id.with("pointer-focus"), true));
        }
        // The click that expands a minimized group is not the first half of a double-click.
        let expand_key = self.id.with(("expanded-by-click", owner.map(|owner| owner.panels.first())));
        if response.clicked() {
            self.output.actions.push(Action::Activate { panel: panel.clone() });
            self.output.focused = Some(panel.clone());
            if minimized == Some(true) {
                // Activate already expands the group of its active tab.
                if !active {
                    self.output.actions.push(Action::SetMinimized { panel: panel.clone(), minimized: false });
                }
                let now = ui.input(|i| i.time);
                ui.ctx().data_mut(|d| d.insert_temp(expand_key, now));
            }
        }
        if let Some(minimized) = minimized
            && response.double_clicked()
        {
            let (now, window) = (ui.input(|i| i.time), ui.ctx().options(|o| o.input_options.max_double_click_delay));
            let expanded_by_first_click = ui.ctx().data_mut(|d| d.remove_temp::<f64>(expand_key)).is_some_and(|t| now - t <= window);
            if !expanded_by_first_click {
                self.output.actions.push(Action::SetMinimized { panel: panel.clone(), minimized: !minimized });
            }
        }
        if self.style.middle_click_closes_tabs && policy.closable && response.clicked_by(egui::PointerButton::Middle) {
            self.output.actions.push(Action::Close { panel: panel.clone() });
        }
        if policy.movable && response.drag_started() && begin_drag(ui.ctx(), self.id, panel.clone(), group) {
            self.latch_undock(ui.ctx(), owner.map(|owner| owner.kind));
        }
        response.context_menu(|ui| {
            self.content.panel_menu(ui, panel);
            if policy.movable && policy.floatable {
                let logical = self.style.float_label.clone();
                let text = self.content.action_label(ui, &logical, &self.style.font, self.style.text);
                let response = ui.button(text);
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &logical));
                if response.clicked() {
                    let size = self.float_size(panel, group.size());
                    let rect = clamp_float(Rect::from_min_size(group.min + vec2(24.0, 24.0), size), self.float_bounds);
                    self.output.actions.push(Action::Float { panel: panel.clone(), rect: [rect.left(), rect.top(), rect.width(), rect.height()] });
                    ui.close();
                }
            }
            let targets: Vec<P> =
                if policy.movable { self.layout.move_targets(panel, |p| self.permissions(p)).into_iter().cloned().collect() } else { Vec::new() };
            if !targets.is_empty() {
                let text = self.content.action_label(ui, &self.style.move_panel_to_label, &self.style.font, self.style.text);
                let response = ui.menu_button(text, |ui| {
                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                        for target in &targets {
                            if self.menu_label(ui, target).clicked() {
                                self.output.actions.push(Action::Move {
                                    panel: panel.clone(),
                                    anchor: target.clone(),
                                    placement: Placement::Tab { before: None },
                                });
                                ui.close();
                            }
                        }
                    });
                });
                response
                    .response
                    .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &self.style.move_panel_to_label));
            }
            if policy.closable && self.style.close_menu_items {
                let text = self.content.action_label(ui, &self.style.close_label, &self.style.font, self.style.text);
                let response = ui.button(text);
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &self.style.close_label));
                if response.clicked() {
                    self.output.actions.push(Action::Close { panel: panel.clone() });
                    ui.close();
                }
            }
            if let Some(owner) = owner {
                ui.separator();
                self.group_menu(ui, owner, panel);
            }
        });
    }
}
