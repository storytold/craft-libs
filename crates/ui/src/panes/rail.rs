//! An icon rail: a vertical strip of icons, always shown at one edge, that opens one task pane at
//! a time beside it.
//!
//! The rail stores nothing. The app passes its items and the pane it has open every frame, shows
//! that pane with [`super::PaneHost`] ([`super::SidePolicy::OneSlot`], not movable), and runs the
//! [`RailRequest`] a click returns.

use std::fmt::Debug;
use std::hash::Hash;

use egui::{
    Align, EventFilter, Frame, Id, ImageSource, Key, Layout, Margin, Modifiers, Rect, Response, Role, Sense, Stroke, StrokeKind, Ui, WidgetInfo, vec2,
};

use super::Edge;
use crate::docking::chrome::ChromeStyle;

/// Default width of the rail.
pub const RAIL_WIDTH: f32 = 48.0;
/// Default width and height of one item.
pub const RAIL_BUTTON: f32 = 34.0;
/// Room between two items.
const SPACING: f32 = 4.0;
/// Room between an item's edge and its icon.
const ICON_INSET: f32 = 8.0;
/// Radius of the marker dot.
const MARKER_RADIUS: f32 = 3.0;
/// Largest width or item size the rail accepts; larger or non-finite values fall back.
const MAX_SIZE: f32 = 4096.0;

/// One icon of the rail, standing for the pane it opens.
#[derive(Clone, Debug)]
pub struct RailItem<P> {
    pub pane: P,
    /// Painted tinted with the chrome's text colors, like docking's panel icons.
    pub icon: ImageSource<'static>,
    /// Tooltip and accessible name. Translate it in the app.
    pub label: String,
    /// `false`: drawn disabled, not focusable, and a click asks for nothing.
    pub enabled: bool,
    /// A small dot telling that the pane has content (comments exist, the document has
    /// bookmarks). Drawn only while the pane is not open.
    pub marker: bool,
    /// The item's accessible description while the marker shows ("Has comments"). Translate it
    /// in the app.
    pub marker_label: Option<String>,
}

impl<P> RailItem<P> {
    /// An enabled item without a marker.
    pub fn new(pane: P, icon: ImageSource<'static>, label: impl Into<String>) -> Self {
        Self { pane, icon, label: label.into(), enabled: true, marker: false, marker_label: None }
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn marker(mut self, marker: bool) -> Self {
        self.marker = marker;
        self
    }

    pub fn marker_label(mut self, label: impl Into<String>) -> Self {
        self.marker_label = Some(label.into());
        self
    }
}

/// What a click on an item asks the app to do.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum RailRequest<P> {
    /// Open this pane in place of the open one.
    Open(P),
    /// Close this pane, which is the open one.
    Close(P),
}

/// What [`Rail::show`] returns.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct RailOutput<P> {
    /// The click of this frame, if any.
    pub request: Option<RailRequest<P>>,
    /// The item holding keyboard focus.
    pub focused: Option<P>,
    /// Where the rail was drawn.
    pub rect: Rect,
}

/// Draws the icon rail. Build it every frame.
pub struct Rail {
    id: Id,
    edge: Edge,
    rtl: bool,
    width: f32,
    button: f32,
    frame: Option<Frame>,
}

impl Rail {
    /// Creates a rail with a stable global ID; use [`Id::unique`] for string or tuple keys.
    pub fn new(id: impl Into<Id>) -> Self {
        Self { id: id.into(), edge: Edge::End, rtl: false, width: RAIL_WIDTH, button: RAIL_BUTTON, frame: None }
    }

    /// The edge the rail sits at. Default [`Edge::End`]. The rail is vertical, so
    /// [`Edge::Bottom`] draws it at the end edge.
    pub fn edge(mut self, edge: Edge) -> Self {
        self.edge = edge;
        self
    }

    /// Right-to-left interface: `Start` is the right edge, and the marker dot sits at the
    /// item's top-left corner.
    pub fn rtl(mut self, rtl: bool) -> Self {
        self.rtl = rtl;
        self
    }

    /// Width of the rail. Default [`RAIL_WIDTH`].
    pub fn width(mut self, width: f32) -> Self {
        self.width = if width.is_finite() { width.clamp(0.0, MAX_SIZE) } else { RAIL_WIDTH };
        self
    }

    /// Width and height of one item. Default [`RAIL_BUTTON`]. The icon is 16 points smaller.
    pub fn button_size(mut self, size: f32) -> Self {
        self.button = if size.is_finite() { size.clamp(0.0, MAX_SIZE) } else { RAIL_BUTTON };
        self
    }

    /// Frame of the rail. Default: the panel fill and an 8 pt margin above and below. The panel's
    /// separator line draws the inner edge.
    pub fn frame(mut self, frame: Frame) -> Self {
        self.frame = Some(frame);
        self
    }

    /// The widget id of `pane`'s item, to give it keyboard focus (`Memory::request_focus`), for
    /// example when the app's F6 cycle reaches the rail.
    pub fn item_id<P: Hash + Debug>(&self, pane: &P) -> Id {
        self.id.with(("rail-item", pane))
    }

    fn left(&self) -> bool {
        matches!((self.edge, self.rtl), (Edge::Start, false) | (Edge::End | Edge::Bottom, true))
    }

    /// Draw the rail as the outermost panel on its edge: call it before [`super::PaneHost::show`]
    /// (and after the app's top and bottom bars), so the open pane sits between the document and
    /// the rail. `items` are drawn top-down and scroll in a short window; `foot` draws below
    /// them, always visible (page navigation, zoom), and may draw nothing. `open` is the pane
    /// the app has open.
    pub fn show<P: Clone + Hash + Debug>(self, ui: &mut Ui, items: &[RailItem<P>], open: Option<&P>, foot: impl FnOnce(&mut Ui)) -> RailOutput<P> {
        let style = ChromeStyle::from_ui(ui);
        let panel_id = self.id.with("rail-panel");
        let panel = if self.left() { egui::Panel::left(panel_id) } else { egui::Panel::right(panel_id) };
        let frame = self.frame.unwrap_or_else(|| Frame::NONE.fill(ui.visuals().panel_fill).inner_margin(Margin::symmetric(0, 8)));
        let shown = panel.exact_size(self.width).resizable(false).frame(frame).show(ui, |ui| {
            // The foot is a panel of its own, so that in a short window the items scroll instead
            // of running into it.
            egui::Panel::bottom(self.id.with("rail-foot")).frame(Frame::NONE).show_separator_line(false).resizable(false).show(ui, |ui| {
                ui.with_layout(Layout::top_down(Align::Center), foot);
            });
            egui::ScrollArea::vertical()
                .id_salt(self.id.with("rail-items"))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.with_layout(Layout::top_down(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.y = SPACING;
                        self.items(ui, items, open, &style)
                    })
                    .inner
                })
                .inner
        });
        let (request, focused) = shown.inner;
        RailOutput { request, focused, rect: shown.response.rect }
    }

    fn items<P: Clone + Hash + Debug>(
        &self,
        ui: &mut Ui,
        items: &[RailItem<P>],
        open: Option<&P>,
        style: &ChromeStyle,
    ) -> (Option<RailRequest<P>>, Option<P>) {
        let mut request = None;
        let mut focused = None;
        let open = open.map(|pane| self.item_id(pane));
        let mut drawn: Vec<Id> = Vec::new();
        // Focus order for Up and Down: the enabled items with their place in `drawn`.
        let mut enabled: Vec<(usize, Id, &P)> = Vec::new();
        let mut focus_index = None;
        // An item disabled while focused, by its place. egui drops a disabled widget's focus as
        // it draws it, so this is read before.
        let mut lost_focus = None;
        for item in items {
            let id = self.item_id(&item.pane);
            // A pane listed twice would give two widgets one id.
            if drawn.contains(&id) {
                continue;
            }
            let place = drawn.len();
            drawn.push(id);
            let selected = open == Some(id);
            let had_focus = ui.memory(|m| m.has_focus(id));
            let response = ui
                .add_enabled_ui(item.enabled, |ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(self.button, self.button), Sense::hover());
                    let response = ui.interact(rect, id, Sense::click());
                    response.widget_info(|| WidgetInfo::selected(Role::Button, ui.is_enabled(), selected, &item.label));
                    if item.marker
                        && !selected
                        && let Some(label) = &item.marker_label
                    {
                        ui.ctx().accesskit_node_builder(id, |node| node.set_description(label.as_str()));
                    }
                    self.paint(ui, &response, item, selected, style);
                    response
                })
                .inner;
            let response = if item.enabled { response.on_hover_text(&item.label) } else { response.on_disabled_hover_text(&item.label) };
            if !item.enabled {
                if had_focus {
                    lost_focus = Some(place);
                }
                continue;
            }
            if response.gained_focus() {
                response.scroll_to_me(None);
            }
            if response.has_focus() {
                focus_index = Some(enabled.len());
                focused = Some(item.pane.clone());
            }
            enabled.push((place, id, &item.pane));
            if response.clicked() {
                request = Some(if selected { RailRequest::Close(item.pane.clone()) } else { RailRequest::Open(item.pane.clone()) });
            }
        }
        if let Some(place) = lost_focus
            && focus_index.is_none()
        {
            // Focus moves to the nearest enabled item below, else above, else leaves the rail.
            let next = enabled.iter().find(|(p, ..)| *p > place).or_else(|| enabled.iter().rev().find(|(p, ..)| *p < place));
            if let Some(&(_, next, pane)) = next {
                ui.memory_mut(|m| m.request_focus(next));
                focused = Some(pane.clone());
            }
        }
        if let Some(index) = focus_index
            && let Some(&(_, id, _)) = enabled.get(index)
        {
            // Up and Down move between the items, never out of the rail by egui's geometric focus.
            ui.memory_mut(|m| m.set_focus_lock_filter(id, EventFilter { vertical_arrows: true, ..Default::default() }));
            let next = ui.input_mut(|input| {
                if input.consume_key(Modifiers::NONE, Key::ArrowDown) {
                    Some(index.checked_add(1))
                } else if input.consume_key(Modifiers::NONE, Key::ArrowUp) {
                    Some(index.checked_sub(1))
                } else {
                    None
                }
            });
            if let Some(&(_, next, _)) = next.flatten().and_then(|i| enabled.get(i)) {
                ui.memory_mut(|m| m.request_focus(next));
            }
        }
        (request, focused)
    }

    fn paint<P>(&self, ui: &Ui, response: &Response, item: &RailItem<P>, selected: bool, style: &ChromeStyle) {
        let rect = response.rect;
        if !rect.is_positive() {
            return;
        }
        let painter = ui.painter();
        let hovered = response.hovered() && ui.is_enabled();
        if selected || hovered {
            painter.rect_filled(rect.shrink(1.0), 4.0, style.active_background);
        }
        if response.has_focus() {
            painter.rect_stroke(rect.shrink(1.0), 4.0, Stroke::new(1.0, style.accent), StrokeKind::Inside);
        }
        let tint = if selected || hovered { style.text } else { style.weak_text };
        let icon = rect.shrink(ICON_INSET);
        if icon.is_positive() {
            egui::Image::new(item.icon.clone()).tint(tint).paint_at(ui, icon);
        }
        if item.marker && !selected {
            let corner = if self.rtl { rect.left_top() + vec2(6.0, 6.0) } else { rect.right_top() + vec2(-6.0, 6.0) };
            painter.circle_filled(corner, MARKER_RADIUS, style.accent);
        }
    }
}
