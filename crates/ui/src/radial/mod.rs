//! A radial command menu: a ring of up to eight commands that opens around a point, usually the
//! pointer, with a close button in the middle.
//!
//! The ring knows nothing about commands. The app passes the items it wants in the ring, opens
//! the ring when its shortcut is pressed, and runs the id that [`RadialMenu::show`] returns as one
//! undo step, the same way it runs a menu pick. Whether the ring is on, which key opens it and
//! which commands fill it are the app's preferences.
//!
//! The menu keeps its own state (where it is open, whether the opening key is held) in egui's
//! memory under its id, so the app calls [`RadialMenu::show`] every frame and stores nothing.

use std::f32::consts::TAU;

use egui::{
    Align2, Area, Context, Event, EventFilter, FocusDirection, FontId, Id, ImageSource, Key, Modifiers, Order, Pos2, Rect, Response, Role, Sense, Ui,
    UiBuilder, Vec2, WidgetInfo, vec2,
};

#[cfg(test)]
mod tests;

/// Most items one ring shows. Items past this are not drawn.
pub const MAX_ITEMS: usize = 8;
/// Default distance from the ring's center to the center of each item.
pub const RADIUS: f32 = 72.0;
/// Default diameter of one item.
pub const BUTTON: f32 = 40.0;
/// Diameter of the close button in the middle.
const CENTER: f32 = 28.0;
/// Room under the ring for the highlighted item's caption.
const CAPTION: f32 = 24.0;
/// Room past the outer edge of the items that still counts as the ring.
const MARGIN: f32 = 8.0;
/// Room between an item's edge and its icon.
const ICON_INSET: f32 = 10.0;
/// Largest radius or item size the menu accepts; larger or non-finite values fall back.
const MAX_SIZE: f32 = 1024.0;

/// One command in the ring.
#[derive(Clone, Debug)]
pub struct RadialItem {
    /// What [`RadialEvent::Picked`] returns for this item. Never shown.
    pub id: String,
    /// Tooltip, caption and accessible name. Translate it in the app.
    pub label: String,
    /// Painted tinted with the text colors. Without one, the item shows the label's first letter.
    pub icon: Option<ImageSource<'static>>,
    /// `false`: drawn disabled, skipped by the keyboard, and a click picks nothing.
    pub enabled: bool,
}

impl RadialItem {
    /// An enabled item without an icon.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self { id: id.into(), label: label.into(), icon: None, enabled: true }
    }

    pub fn icon(mut self, icon: ImageSource<'static>) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// The two names the menu supplies itself. Translate them in the app.
#[derive(Clone, Debug)]
pub struct RadialLabels {
    /// The accessible name of the ring as a whole.
    pub menu: String,
    /// The accessible name and tooltip of the close button in the middle.
    pub close: String,
}

impl Default for RadialLabels {
    fn default() -> Self {
        Self { menu: "Commands".into(), close: "Close".into() }
    }
}

/// How an open ring ended this frame.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RadialEvent {
    /// The person picked the item with this id. Run it, then the ring is closed.
    Picked(String),
    /// The ring closed without a pick: Esc, the close button, a click outside, or the window
    /// lost focus.
    Closed,
}

/// Where a point lies relative to an open ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RingHit {
    /// Over the sector of the item with this index.
    Slot(usize),
    /// Over the close button in the middle.
    Center,
    /// Past the ring.
    Outside,
}

/// The angle of item `index` of `count`, in radians clockwise from straight up on screen. Item 0
/// is at the top and the items are spaced evenly.
pub fn slot_angle(index: usize, count: usize) -> f32 {
    if count == 0 { 0.0 } else { TAU * (index % count) as f32 / count as f32 }
}

/// The unit vector from the ring's center toward item `index` of `count`, in screen coordinates
/// (y grows downward).
pub fn slot_direction(index: usize, count: usize) -> Vec2 {
    let angle = slot_angle(index, count);
    vec2(angle.sin(), -angle.cos())
}

/// The angle of `direction` in radians clockwise from straight up, in `0..TAU`.
fn angle_of(direction: Vec2) -> f32 {
    let angle = direction.x.atan2(-direction.y);
    if angle < 0.0 { angle + TAU } else { angle }
}

/// The item whose sector holds `direction`. Each of `count` items owns the
/// `TAU / count` wide sector centered on its angle. `None` for no items or a zero or non-finite
/// direction.
pub fn sector(direction: Vec2, count: usize) -> Option<usize> {
    if count == 0 || !direction.is_finite() || direction.length_sq() <= 0.0 {
        return None;
    }
    let step = TAU / count as f32;
    let index = ((angle_of(direction) + step / 2.0) / step).floor() as usize;
    Some(index % count)
}

/// The enabled item whose angle is nearest `direction`; on a tie, the lower index. `enabled`
/// holds one entry per item.
pub fn nearest_enabled(direction: Vec2, enabled: &[bool]) -> Option<usize> {
    if !direction.is_finite() || direction.length_sq() <= 0.0 {
        return None;
    }
    let want = angle_of(direction);
    let count = enabled.len();
    let mut best: Option<(usize, f32)> = None;
    for (index, _) in enabled.iter().enumerate().filter(|(_, on)| **on) {
        let gap = (slot_angle(index, count) - want).abs();
        let gap = gap.min(TAU - gap);
        if best.is_none_or(|(_, b)| gap < b - 1e-4) {
            best = Some((index, gap));
        }
    }
    best.map(|(index, _)| index)
}

/// Where `pointer` lies relative to a ring of `count` items centered at `center`. Within `inner`
/// of the center is the close button, past `outer` is outside, and between the two each item owns
/// its [`sector`]. With no items, everything outside the close button is outside.
pub fn hit(center: Pos2, pointer: Pos2, count: usize, inner: f32, outer: f32) -> RingHit {
    let offset = pointer - center;
    if !offset.is_finite() {
        return RingHit::Outside;
    }
    let distance = offset.length();
    if distance <= inner {
        return RingHit::Center;
    }
    if distance > outer {
        return RingHit::Outside;
    }
    sector(offset, count).map_or(RingHit::Outside, RingHit::Slot)
}

/// The center that keeps a ring reaching `reach` in every direction inside `screen`, as near
/// `at` as it can be. On a screen too small for the ring, the ring is centered on the screen
/// along that axis. A non-finite `at` gives the screen's center.
pub fn fit_center(at: Pos2, reach: f32, screen: Rect) -> Pos2 {
    let reach = if reach.is_finite() { reach.max(0.0) } else { 0.0 };
    let at = if at.is_finite() { at } else { screen.center() };
    let axis = |value: f32, low: f32, high: f32| {
        if high - low < 2.0 * reach { (low + high) / 2.0 } else { value.clamp(low + reach, high - reach) }
    };
    Pos2::new(axis(at.x, screen.min.x, screen.max.x), axis(at.y, screen.min.y, screen.max.y))
}

/// The caption's box: `size` centered on `anchor`, moved as little as needed to lie inside
/// `screen`. A box larger than the screen starts at the screen's top-left along that axis.
pub fn caption_rect(anchor: Pos2, size: Vec2, screen: Rect) -> Rect {
    let rect = Align2::CENTER_CENTER.anchor_size(anchor, size);
    let shift = |low: f32, high: f32, min: f32, max: f32| {
        if high - low > max - min || low < min {
            min - low
        } else if high > max {
            max - high
        } else {
            0.0
        }
    };
    rect.translate(vec2(shift(rect.min.x, rect.max.x, screen.min.x, screen.max.x), shift(rect.min.y, rect.max.y, screen.min.y, screen.max.y)))
}

/// What the menu remembers between frames while it is open.
#[derive(Clone, Copy, Debug)]
struct State {
    /// Where the app asked the ring to open, before it is kept on screen.
    at: Pos2,
    /// The opening key is still down: releasing it over an item picks that item.
    held: bool,
    /// The app reported the opening key's release; the next [`RadialMenu::show`] decides.
    released: bool,
    /// Where the pointer was when the ring opened.
    pointer_at_open: Option<Pos2>,
    /// The widget that held keyboard focus when the ring opened, given focus back on close.
    restore: Option<Id>,
    /// The ring widget to focus once the ring is drawn visibly. egui draws a new area invisibly
    /// on its first pass and drops focus from its widgets there.
    pending: Option<Id>,
    /// Focus has not yet been placed in the ring.
    fresh: bool,
    /// The pass the ring opened in. Enter and Space in that pass do not pick: the ring has not
    /// been shown yet, and the key may be the one that opened it.
    opened_pass: u64,
}

/// A radial command menu. Build it every frame with the same id.
#[derive(Clone, Debug)]
pub struct RadialMenu {
    id: Id,
    radius: f32,
    button: f32,
}

impl RadialMenu {
    /// Creates a menu with a stable global ID; use [`Id::unique`] for string or tuple keys.
    pub fn new(id: impl Into<Id>) -> Self {
        Self { id: id.into(), radius: RADIUS, button: BUTTON }
    }

    /// Distance from the center to each item's center. Default [`RADIUS`].
    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = if radius.is_finite() { radius.clamp(0.0, MAX_SIZE) } else { RADIUS };
        self
    }

    /// Diameter of one item. Default [`BUTTON`].
    pub fn button_size(mut self, size: f32) -> Self {
        self.button = if size.is_finite() { size.clamp(0.0, MAX_SIZE) } else { BUTTON };
        self
    }

    /// The radius of the close button, inside which the pointer is over it.
    pub fn inner_radius(&self) -> f32 {
        CENTER / 2.0
    }

    /// The radius past which the pointer is outside the ring.
    pub fn outer_radius(&self) -> f32 {
        self.radius + self.button / 2.0 + MARGIN
    }

    /// How far the ring and its caption reach from the center in every direction.
    pub fn reach(&self) -> f32 {
        self.radius + self.button / 2.0 + CAPTION
    }

    /// The id of item `index`'s widget, for focus and accessibility.
    pub fn item_id(&self, index: usize) -> Id {
        self.id.with(("item", index))
    }

    /// The id of the close button's widget.
    pub fn close_id(&self) -> Id {
        self.id.with("close")
    }

    /// Opens the ring around `at`, kept on screen. `held`: the key that opened it is still down,
    /// and the app calls [`Self::release`] when it goes up. Opening a ring that is already open
    /// leaves it as it is, so a repeating key does not move it or reset its gesture.
    pub fn open(&self, ctx: &Context, at: Pos2, held: bool) {
        if self.is_open(ctx) {
            return;
        }
        let restore = ctx.memory(|m| m.focused()).filter(|id| !self.owns(*id));
        let state = State {
            at,
            held,
            released: false,
            pointer_at_open: ctx.pointer_latest_pos(),
            restore,
            pending: None,
            fresh: true,
            opened_pass: ctx.cumulative_pass_nr(),
        };
        ctx.data_mut(|d| d.insert_temp(self.id, state));
    }

    /// The key that opened the ring went up. On the next [`Self::show`], when the pointer has
    /// moved since the ring opened and lies off the close button, the enabled item in its
    /// direction is picked. Otherwise the ring stays open for a click or the keyboard.
    pub fn release(&self, ctx: &Context) {
        if let Some(mut state) = self.state(ctx)
            && state.held
        {
            state.held = false;
            state.released = true;
            ctx.data_mut(|d| d.insert_temp(self.id, state));
        }
    }

    /// Closes the ring without a pick, giving focus back as a close by the person does.
    pub fn close(&self, ctx: &Context) {
        if let Some(state) = self.state(ctx) {
            self.finish(ctx, state);
        }
    }

    pub fn is_open(&self, ctx: &Context) -> bool {
        self.state(ctx).is_some()
    }

    fn state(&self, ctx: &Context) -> Option<State> {
        ctx.data(|d| d.get_temp::<State>(self.id))
    }

    fn owns(&self, id: Id) -> bool {
        id == self.close_id() || (0..MAX_ITEMS).any(|i| id == self.item_id(i))
    }

    fn finish(&self, ctx: &Context, state: State) {
        ctx.data_mut(|d| d.remove::<State>(self.id));
        // egui hit-tests against the layers of the previous pass. Another pass right away drops
        // the ring's full-window layer, so the next click reaches what is underneath.
        ctx.request_repaint();
        ctx.memory_mut(|m| match state.restore {
            Some(id) => m.request_focus(id),
            None => {
                if let Some(id) = m.focused().filter(|id| self.owns(*id)) {
                    m.surrender_focus(id);
                }
            }
        });
    }

    /// Draws the ring while it is open and returns how it ended this frame, if it did. Call it
    /// every frame before the app's own panels, so the ring reads Esc, Enter, Tab and the arrows
    /// before a panel takes them. The ring draws on the foreground layer, above the panels, in
    /// either order.
    pub fn show(&self, ctx: &Context, items: &[RadialItem], labels: &RadialLabels) -> Option<RadialEvent> {
        let mut state = self.state(ctx)?;
        let items = &items[..items.len().min(MAX_ITEMS)];
        let count = items.len();
        let screen = ctx.content_rect();
        let center = fit_center(state.at, self.reach(), screen);
        let enabled: Vec<bool> = items.iter().map(|item| item.enabled).collect();
        // Keyboard stops: the enabled items clockwise, then the close button.
        let mut stops: Vec<Id> = (0..count).filter(|i| enabled[*i]).map(|i| self.item_id(i)).collect();
        stops.push(self.close_id());

        if crate::focus::lost(ctx) {
            self.finish(ctx, state);
            return Some(RadialEvent::Closed);
        }

        // Focus stays in the ring: on opening, and when the focused item was disabled or removed
        // while the ring is open, it goes to the first stop.
        state.pending = state.pending.filter(|id| stops.contains(id));
        let in_ring = ctx.memory(|m| m.focused()).is_some_and(|id| stops.contains(&id));
        if std::mem::take(&mut state.fresh) || (state.pending.is_none() && !in_ring) {
            state.pending = Some(stops[0]);
        }
        // Keys step from where focus is going; Enter and Space act only on the widget that has
        // focus now, never on one focus has not reached yet.
        let has_focus = ctx.memory(|m| m.focused());
        let focused = state.pending.or(has_focus);
        let focused_item = (0..count).find(|i| has_focus == Some(self.item_id(*i)));

        // Keys are read before the ring's widgets exist, and egui's own focus step is canceled,
        // so a key moves focus once and never out of the ring.
        let mut event = None;
        let mut move_to = None;
        let opening_pass = ctx.cumulative_pass_nr() == state.opened_pass;
        let held = state.held;
        ctx.input_mut(|input| {
            if input.consume_key(Modifiers::NONE, Key::Escape) {
                event = Some(RadialEvent::Closed);
                return;
            }
            // Only a fresh press activates: a key repeat, a press in the opening pass, or any
            // press while the opening key is held could be that key, and a held key picks on
            // release instead.
            let fresh_press = input.events.iter().any(|e| {
                matches!(e, Event::Key { key: Key::Enter | Key::Space, pressed: true, repeat: false, modifiers, .. }
                    if modifiers.matches_logically(Modifiers::NONE))
            });
            let enter = input.consume_key(Modifiers::NONE, Key::Enter);
            let space = input.consume_key(Modifiers::NONE, Key::Space);
            if (enter || space) && fresh_press && !opening_pass && !held {
                event = match focused_item {
                    Some(i) if enabled[i] => Some(RadialEvent::Picked(items[i].id.clone())),
                    Some(_) => None,
                    None if has_focus == Some(self.close_id()) => Some(RadialEvent::Closed),
                    None => None,
                };
                return;
            }
            let at = focused.and_then(|id| stops.iter().position(|s| *s == id));
            // Shift+Tab first: a plain Tab check also matches Tab with Shift held.
            if input.consume_key(Modifiers::SHIFT, Key::Tab) {
                move_to = Some(at.map_or(stops.len() - 1, |at| (at + stops.len() - 1) % stops.len()));
                return;
            }
            if input.consume_key(Modifiers::NONE, Key::Tab) {
                move_to = Some(at.map_or(0, |at| (at + 1) % stops.len()));
                return;
            }
            // An arrow pressed this frame counts even if it was released in the same frame, and an
            // arrow still held counts too, so two arrows give a diagonal.
            let mut arrows = [(Key::ArrowUp, false), (Key::ArrowDown, false), (Key::ArrowLeft, false), (Key::ArrowRight, false)];
            let mut pressed = false;
            for (key, on) in &mut arrows {
                let now = input.consume_key(Modifiers::NONE, *key);
                pressed |= now;
                *on = now || input.key_down(*key);
            }
            if pressed {
                let [up, down, left, right] = arrows.map(|(_, on)| if on { 1.0 } else { 0.0 });
                let direction = vec2(right - left, down - up);
                move_to = nearest_enabled(direction, &enabled).and_then(|i| stops.iter().position(|s| *s == self.item_id(i)));
            }
        });
        ctx.memory_mut(|m| m.move_focus(FocusDirection::None));
        if let Some(event) = event {
            self.finish(ctx, state);
            return Some(event);
        }
        if let Some(stop) = move_to.and_then(|i| stops.get(i).copied()) {
            state.pending = Some(stop);
        }

        let pointer = ctx.pointer_latest_pos();
        let moved_since_open = |p: Pos2| state.pointer_at_open.is_none_or(|open| open.distance(p) > self.inner_radius());
        // While the key is held, the direction the pointer moved from where it was when the ring
        // opened aims, at any distance, so a flick picks the same item when the ring had to move
        // to stay on screen. Otherwise, the item under the pointer.
        let aimed = |p: Pos2, held: bool| {
            let slot = if held {
                let origin = state.pointer_at_open.unwrap_or(center);
                if origin.distance(p) > self.inner_radius() { sector(p - origin, count) } else { None }
            } else {
                match hit(center, p, count, self.inner_radius(), self.outer_radius()) {
                    RingHit::Slot(i) => Some(i),
                    _ => None,
                }
            };
            slot.filter(|i| enabled[*i])
        };
        if std::mem::take(&mut state.released)
            && let Some(i) = pointer.filter(|p| moved_since_open(*p)).and_then(|p| aimed(p, true))
        {
            let picked = items[i].id.clone();
            self.finish(ctx, state);
            return Some(RadialEvent::Picked(picked));
        }
        let pointer_moved = ctx.input(|i| i.pointer.delta() != Vec2::ZERO);
        if pointer_moved && let Some(i) = pointer.and_then(|p| aimed(p, state.held)) {
            state.pending = Some(self.item_id(i));
        }

        let mut ended = None;
        let mut visible = false;
        Area::new(self.id.with("area")).order(Order::Foreground).fixed_pos(screen.min).movable(false).constrain(false).show(ctx, |ui| {
            // A see-through layer over the whole window takes the clicks outside the ring, so
            // they close it instead of reaching what is underneath.
            visible = ui.is_visible();
            let backdrop = ui.interact(screen, self.id.with("backdrop"), Sense::click());
            // The area covers the window too, so `Context::layer_id_at` finds the ring's layer
            // for an app that reads the pointer itself.
            ui.expand_to_include_rect(screen);
            ui.scope_builder(UiBuilder::new().max_rect(screen).accessibility_role(Role::Menu).accessibility_label(labels.menu.clone()), |ui| {
                ended = self.draw(ui, center, items, labels)
            });
            if ended.is_none() && backdrop.clicked() {
                ended = match backdrop.interact_pointer_pos().map(|p| hit(center, p, count, self.inner_radius(), self.outer_radius())) {
                    Some(RingHit::Slot(i)) if enabled[i] => Some(RadialEvent::Picked(items[i].id.clone())),
                    Some(RingHit::Slot(_)) => None,
                    _ => Some(RadialEvent::Closed),
                };
            }
        });
        if ended.is_some() {
            self.finish(ctx, state);
        } else {
            if visible && let Some(id) = state.pending.take() {
                ctx.memory_mut(|m| m.request_focus(id));
            }
            ctx.data_mut(|d| d.insert_temp(self.id, state));
            // Tab and the arrows stay with the ring's item that has focus.
            if let Some(id) = ctx.memory(|m| m.focused()).filter(|id| self.owns(*id)) {
                ctx.memory_mut(|m| {
                    m.set_focus_lock_filter(id, EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true })
                });
            }
        }
        ended
    }

    /// Draws the items, the close button and the caption; returns a click's result.
    fn draw(&self, ui: &mut Ui, center: Pos2, items: &[RadialItem], labels: &RadialLabels) -> Option<RadialEvent> {
        let count = items.len();
        let visuals = ui.visuals().clone();
        let painter = ui.painter().clone();
        let disc = self.radius + self.button / 2.0 + MARGIN / 2.0;
        painter.circle(center, disc, visuals.window_fill, visuals.window_stroke);

        let mut ended = None;
        let mut caption = None;
        for (index, item) in items.iter().enumerate() {
            let rect = Rect::from_center_size(center + slot_direction(index, count) * self.radius, Vec2::splat(self.button));
            let sense = if item.enabled { Sense::click() } else { Sense::hover() };
            let response = ui.interact(rect, self.item_id(index), sense);
            let (label, enabled) = (item.label.clone(), item.enabled);
            response.widget_info(|| WidgetInfo::labeled(Role::MenuItem, enabled, label.as_str()));
            let lit = item.enabled && (response.has_focus() || response.hovered());
            if lit {
                caption = Some(item.label.as_str());
            }
            self.paint_item(ui, &response, item, lit);
            let response = response.on_hover_text(item.label.as_str());
            if item.enabled && response.clicked() {
                ended = Some(RadialEvent::Picked(item.id.clone()));
            }
        }

        let rect = Rect::from_center_size(center, Vec2::splat(CENTER));
        let close = ui.interact(rect, self.close_id(), Sense::click());
        let name = labels.close.clone();
        close.widget_info(|| WidgetInfo::labeled(Role::Button, true, name.as_str()));
        let state = if close.hovered() || close.has_focus() { &visuals.widgets.hovered } else { &visuals.widgets.inactive };
        painter.circle(center, CENTER / 2.0, state.bg_fill, state.bg_stroke);
        let arm = CENTER / 5.0;
        for (a, b) in [(vec2(-arm, -arm), vec2(arm, arm)), (vec2(-arm, arm), vec2(arm, -arm))] {
            painter.line_segment([center + a, center + b], state.fg_stroke);
        }
        if close.has_focus() {
            painter.circle_stroke(center, CENTER / 2.0 + 2.0, visuals.selection.stroke);
            caption = Some(labels.close.as_str());
        }
        let close = close.on_hover_text(labels.close.as_str());
        if close.clicked() {
            ended = Some(RadialEvent::Closed);
        }

        if let Some(text) = caption {
            let at = center + vec2(0.0, self.radius + self.button / 2.0 + CAPTION / 2.0);
            let screen = ui.max_rect();
            let wrap = (screen.width() - 12.0).max(1.0);
            let galley = painter.layout(text.to_owned(), FontId::proportional(13.0), visuals.text_color(), wrap);
            let back = caption_rect(at, galley.size() + vec2(12.0, 4.0), screen);
            painter.rect(back, 4.0, visuals.window_fill, visuals.window_stroke, egui::StrokeKind::Inside);
            painter.galley(back.min + vec2(6.0, 2.0), galley, visuals.text_color());
        }
        ended
    }

    fn paint_item(&self, ui: &Ui, response: &Response, item: &RadialItem, lit: bool) {
        let rect = response.rect;
        let radius = rect.width() / 2.0;
        if radius <= 0.0 {
            return;
        }
        let visuals = ui.visuals();
        let state = if lit { &visuals.widgets.hovered } else { &visuals.widgets.inactive };
        let tint = if item.enabled { state.fg_stroke.color } else { visuals.widgets.noninteractive.fg_stroke.color.gamma_multiply(0.5) };
        let painter = ui.painter();
        painter.circle(rect.center(), radius, state.bg_fill, state.bg_stroke);
        if response.has_focus() {
            painter.circle_stroke(rect.center(), radius + 2.0, visuals.selection.stroke);
        }
        match &item.icon {
            Some(icon) => {
                let inner = rect.shrink(ICON_INSET);
                if inner.is_positive() {
                    egui::Image::new(icon.clone()).tint(tint).paint_at(ui, inner);
                }
            }
            None => {
                let letter: String = item.label.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
                painter.text(rect.center(), Align2::CENTER_CENTER, letter, FontId::proportional(radius * 0.8), tint);
            }
        }
    }
}
