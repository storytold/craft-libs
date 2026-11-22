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
//!
//! While the ring is open it takes every key press, typed or composed text and clipboard event,
//! so a shortcut or a tool key does not reach the app's panels under it. Input that came before
//! the opening press in the pass that opened the ring is left for the app. Key releases still reach
//! the app, so it can report the opening key's release with [`RadialMenu::release`]. egui's
//! `key_down` still reports held keys; an app that acts on a held key reads
//! [`RadialMenu::is_open`] before calling [`RadialMenu::show`] and skips that for the rest of the
//! pass when the ring was open, since a pick or close in `show` ends the ring in that pass.
//!
//! In a right-to-left interface ([`crate::direction`]) the items run counterclockwise from the
//! top, and Tab follows them.

use std::f32::consts::{PI, TAU};

use egui::{
    Align2, Area, Context, Event, EventFilter, FocusDirection, FontId, Id, ImageSource, Key, KeyboardShortcut, Modifiers, Order, Pos2, Rect,
    Response, Role, Sense, TextStyle, Ui, UiBuilder, Vec2, WidgetInfo, vec2,
};

#[cfg(test)]
mod tests;

/// Most items one ring shows. Items past this are not drawn, and the menu logs a warning when it
/// opens with more.
pub const MAX_ITEMS: usize = 8;
/// Default distance from the ring's center to the center of each item.
pub const RADIUS: f32 = 72.0;
/// Default diameter of one item.
pub const BUTTON: f32 = 40.0;
/// Diameter of the close button in the middle.
const CENTER: f32 = 28.0;
/// Room past the ring's disc, below or above it, kept on screen for a one-line caption.
const CAPTION: f32 = 24.0;
/// Room past the outer edge of the items that still counts as the ring.
const MARGIN: f32 = 8.0;
/// Room between an item's edge and its icon, at most a quarter of the item's diameter.
const ICON_INSET: f32 = 10.0;
/// Largest radius or item size the menu accepts; larger or non-finite values fall back.
const MAX_SIZE: f32 = 1024.0;
/// Smallest item size the menu accepts.
const MIN_BUTTON: f32 = 16.0;

/// One command in the ring.
#[derive(Clone, Debug)]
pub struct RadialItem {
    /// What [`RadialEvent::Picked`] returns for this item. Never shown.
    pub id: String,
    /// Tooltip, caption and accessible name. Translate it in the app.
    pub label: String,
    /// A second line in the tooltip. For a disabled item, the reason it is disabled.
    pub detail: Option<String>,
    /// Painted tinted with the text colors. Without one, the item shows the label's first letter.
    /// An image rather than [`crate::menu::MenuItem`]'s glyph, since the ring shows no label
    /// beside it.
    pub icon: Option<ImageSource<'static>>,
    /// `false`: drawn disabled, skipped by the keyboard, and a click picks nothing.
    pub enabled: bool,
}

impl RadialItem {
    /// An enabled item without an icon.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self { id: id.into(), label: label.into(), detail: None, icon: None, enabled: true }
    }

    pub fn icon(mut self, icon: ImageSource<'static>) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Present but unavailable, with the reason in its tooltip.
    pub fn disabled(mut self, why: impl Into<String>) -> Self {
        self.enabled = false;
        self.detail = Some(why.into());
        self
    }
}

/// The two names the menu supplies itself. The defaults are English; translate them in the app.
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
/// is at the top and the items are spaced evenly. This and the other free functions here
/// describe a left-to-right ring; a right-to-left interface draws it mirrored, and
/// [`RadialMenu::hit_test`] tests an open ring as it is drawn.
pub fn slot_angle(index: usize, count: usize) -> f32 {
    if count == 0 { 0.0 } else { TAU * (index % count) as f32 / count as f32 }
}

/// The unit vector from the ring's center toward item `index` of `count`, in screen coordinates
/// (y grows downward).
pub fn slot_direction(index: usize, count: usize) -> Vec2 {
    let angle = slot_angle(index, count);
    vec2(angle.sin(), -angle.cos())
}

/// `direction` flipped left for right in a right-to-left interface. Applied to an item's place it
/// gives where the item is drawn; applied to a direction on screen it gives the direction among
/// the items' clockwise angles.
fn mirrored(direction: Vec2, rtl: bool) -> Vec2 {
    if rtl { vec2(-direction.x, direction.y) } else { direction }
}

/// Removes this pass's key presses, typed and composed text, clipboard events and assistive
/// technology requests for widgets other than `ring`'s, so the panels drawn after an open ring do
/// not see them. Key releases stay for the app, and so do the first `keep.0` raw events and the
/// first `keep.1` processed events: those that came before the press that opened the ring.
fn swallow_keys(ctx: &Context, ring: &[egui::accesskit::NodeId], keep: (usize, usize)) {
    ctx.input_mut(|input| {
        let modal = |e: &Event| match e {
            Event::AccessKitActionRequest(request) => !ring.contains(&request.target_node),
            _ => matches!(
                e,
                Event::Key { pressed: true, .. }
                    | Event::Text(_)
                    | Event::Copy
                    | Event::Cut
                    | Event::Paste(_)
                    | Event::PasteImage(_)
                    | Event::MiddleClickPaste { .. }
                    | Event::Ime(_)
            ),
        };
        let mut n = 0;
        input.events.retain(|e| {
            n += 1;
            n <= keep.1 || !modal(e)
        });
        let mut n = 0;
        input.raw.events.retain(|e| {
            n += 1;
            n <= keep.0 || !modal(e)
        });
    });
}

/// Where this pass's click with the primary button ended, or `fallback` without one: the last
/// primary release that egui counts as a click. egui's interaction position follows moves after
/// the release in the same pass. egui tracks one press for all buttons: each press, of any
/// button, starts it, and a move farther than egui's click distance from it before any button
/// goes up means that no release counts as a click until the next press. A release whose press
/// came in an earlier pass counts.
fn release_pos(ctx: &Context, fallback: Option<Pos2>) -> Option<Pos2> {
    primary_click(ctx).map(|(pos, _)| pos).or(fallback)
}

/// Where this pass's click with the primary button ended, as [`release_pos`] finds it, and
/// where the primary button went down for it when that was in this pass.
fn primary_click(ctx: &Context) -> Option<(Pos2, Option<Pos2>)> {
    let max = ctx.options(|o| o.input_options.max_click_dist);
    ctx.input(|i| {
        // Where the last press in this pass went down while no button has gone up since, and
        // whether the pointer moved too far from it.
        let mut origin: Option<Pos2> = None;
        let mut far = false;
        // Where the primary button went down in this pass, while it has not gone up since.
        let mut primary_press = None;
        let mut click = None;
        for e in &i.raw.events {
            match e {
                Event::PointerMoved(p) => {
                    if let Some(origin) = origin {
                        far |= origin.distance(*p) > max;
                    }
                }
                Event::PointerButton { pos, button, pressed: true, .. } => {
                    origin = Some(*pos);
                    far = false;
                    if *button == egui::PointerButton::Primary {
                        primary_press = Some(*pos);
                    }
                }
                Event::PointerButton { pos, button, pressed: false, .. } => {
                    if *button == egui::PointerButton::Primary {
                        if !far {
                            click = Some((*pos, primary_press));
                        }
                        primary_press = None;
                    }
                    origin = None;
                }
                _ => {}
            }
        }
        click
    })
}

/// Where the primary button last went down in this pass, or else where egui's current press began.
fn primary_press_pos(ctx: &Context) -> Option<Pos2> {
    ctx.input(|i| {
        i.raw
            .events
            .iter()
            .rev()
            .find_map(|e| match e {
                Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, .. } => Some(*pos),
                _ => None,
            })
            .or(i.pointer.press_origin())
    })
}

/// How many of `events`, which are `raw` with some events removed, come from `raw[..end]`. Each
/// event is matched to the latest raw event it can be, so an event equal to one before `end` and
/// one after it counts as after.
fn processed_before(raw: &[Event], events: &[Event], end: usize) -> usize {
    let mut until = raw.len();
    let mut after = 0;
    for e in events.iter().rev() {
        match raw[..until].iter().rposition(|r| r == e) {
            Some(at) if at >= end => {
                until = at;
                after += 1;
            }
            _ => break,
        }
    }
    events.len() - after
}

/// The four arrows in the order the ring reads them: up, down, left, right.
const ARROWS: [Key; 4] = [Key::ArrowUp, Key::ArrowDown, Key::ArrowLeft, Key::ArrowRight];

/// What a key event does to the arrows the ring reads.
enum ArrowStep {
    /// Arrow `n` (in [`ARROWS`] order) went down and the ring takes the press. `holds`: it stays
    /// part of a diagonal until it goes up.
    Press { n: usize, holds: bool },
    /// Arrow `n` went up.
    Release(usize),
}

/// What event `at` of a pass does to the arrows. The ring takes an arrow with no modifier or with
/// Shift. `hold` is the opening key while it is held, with the index of the event that ends its
/// hold: before that event the key is never part of a diagonal, and its repeats are not presses.
fn arrow_step(e: &Event, at: usize, hold: Option<(Key, usize)>) -> Option<ArrowStep> {
    let Event::Key { key, pressed, repeat, modifiers, .. } = e else {
        return None;
    };
    let n = ARROWS.iter().position(|k| k == key)?;
    // The opening key until its hold ends; a press after that is a new one.
    let opener = hold.is_some_and(|(k, end)| k == *key && at < end);
    if !*pressed {
        Some(ArrowStep::Release(n))
    } else if modifiers.alt || !modifiers.matches_logically(Modifiers::NONE) || (opener && *repeat) {
        None
    } else {
        Some(ArrowStep::Press { n, holds: !opener })
    }
}

/// `e` is a fresh press of `shortcut`: its key, not a repeat, with exactly its modifiers as
/// [`Modifiers::matches_exact`] reads them.
fn is_shortcut_press(e: &Event, shortcut: KeyboardShortcut) -> bool {
    matches!(e, Event::Key { key, pressed: true, repeat: false, modifiers, .. }
        if *key == shortcut.logical_key && modifiers.matches_exact(shortcut.modifiers))
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

/// Which item the caption under the ring names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaptionFor {
    /// The item with keyboard focus, after the arrows or Tab moved it.
    Focus,
    /// The item with this index, which releasing the held opening key would pick.
    Item(usize),
    /// The item under the pointer, or else the one with focus.
    Hover,
    /// No caption: the held key aims at a disabled item.
    Nothing,
}

/// How this pass draws the ring.
#[derive(Clone, Copy, Debug)]
struct View {
    /// The interface runs right to left.
    rtl: bool,
    /// The slots that hold another command than in the last pass.
    changed: [bool; MAX_ITEMS],
    /// The slots whose item was disabled when the primary button went down, in the last pass or
    /// this one, while it is down. A pointer click does not pick them.
    press_disabled: [bool; MAX_ITEMS],
    /// The primary button went down over other commands, or over the same commands drawn
    /// elsewhere: mirrored, moved or resized. A pointer click then picks nothing.
    press_moved: bool,
    /// The primary button went down over an item that was disabled then. A pointer click then
    /// picks nothing, even when it goes up over a neighbor.
    press_on_disabled: bool,
}

/// What the ring drew in one pass: each slot's command, whether it was enabled, and where the
/// slots were.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Layout {
    commands: [Option<Id>; MAX_ITEMS],
    enabled: [bool; MAX_ITEMS],
    rtl: bool,
    center: Pos2,
    /// The radius the ring is drawn at, [`RadialMenu::ring_radius`], not the one asked for.
    radius: f32,
    button: f32,
}

impl Layout {
    /// The same commands in the same places, whichever are enabled.
    fn same_places(&self, other: &Self) -> bool {
        Self { enabled: other.enabled, ..*self } == *other
    }

    /// Where `pointer` lay relative to the ring as this layout drew it.
    fn hit(&self, pointer: Pos2) -> RingHit {
        let count = self.commands.iter().filter(|c| c.is_some()).count();
        let outer = self.radius + self.button / 2.0 + MARGIN;
        hit(self.center, self.center + mirrored(pointer - self.center, self.rtl), count, CENTER / 2.0, outer)
    }
}

/// What the menu remembers between frames while it is open.
#[derive(Clone, Copy, Debug)]
struct State {
    /// Where the app asked the ring to open, before it is kept on screen.
    at: Pos2,
    /// The opening key is still down: releasing it over an item picks that item.
    held: bool,
    /// The shortcut that opened the ring, if the app named one. In the opening pass, only a press
    /// of its key with exactly its modifiers is an opening press. While `held`, the key's repeats are
    /// ignored, and its release, with any modifiers, ends the hold.
    opener: Option<KeyboardShortcut>,
    /// The pointer button whose press in the opening pass held the ring open, if one did. A
    /// release reported by the app picks from where that button went up in the same pass.
    button: Option<egui::PointerButton>,
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
    /// What the last pass drew; `None` before the first.
    drawn: Option<Layout>,
    /// The ring's widget that had focus at the end of the last pass.
    last_focus: Option<Id>,
    /// What was drawn when the primary pointer button went down, while it is down.
    press_layout: Option<Layout>,
    /// Where the primary pointer button went down, while it is down.
    press_at: Option<Pos2>,
    /// The arrows the ring took that are still down, in [`ARROWS`] order. An arrow that went down
    /// with Alt, Ctrl or Command, or before the ring opened, is not among them. Only the arrows
    /// among them repeat.
    arrows: [bool; 4],
    /// Tab went down while the ring was open, with no modifier or with Shift, and is still down.
    /// Only then do its repeats move the highlight.
    tab: bool,
    /// Where the pointer was at the end of the last pass.
    last_pointer: Option<Pos2>,
    /// The arrows or Tab moved the highlight. Releasing the opening key then picks nothing and
    /// leaves the ring open, so Enter or a click picks.
    keys_used: bool,
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

    /// Distance from the center to each item's center. Default [`RADIUS`]. A radius too small
    /// for eight items to clear the close button and each other is raised to the smallest that
    /// does; see [`Self::ring_radius`].
    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = if radius.is_finite() { radius.clamp(0.0, MAX_SIZE) } else { RADIUS };
        self
    }

    /// Diameter of one item, from 16 to 1024 pt. Default [`BUTTON`].
    pub fn button_size(mut self, size: f32) -> Self {
        self.button = if size.is_finite() { size.clamp(MIN_BUTTON, MAX_SIZE) } else { BUTTON };
        self
    }

    /// The distance from the center to each item's center that the ring uses: the radius asked
    /// for, or the smallest at which eight items clear the close button and each other.
    pub fn ring_radius(&self) -> f32 {
        let clear_center = CENTER / 2.0 + self.button / 2.0 + MARGIN / 2.0;
        let clear_neighbors = self.button / (2.0 * (PI / MAX_ITEMS as f32).sin());
        self.radius.max(clear_center).max(clear_neighbors)
    }

    /// This menu as an open ring draws it on `screen`: when the ring's disc is wider or taller
    /// than the screen, the item size and the radius are scaled down together until it fits, but
    /// never below an item size of 16 pt.
    fn fitted(&self, screen: Rect) -> Self {
        let half = screen.width().min(screen.height()) / 2.0;
        if !half.is_finite() || self.outer_radius() <= half {
            return self.clone();
        }
        // Each bound on the ring radius in `ring_radius` is `a * scale + c`; the disc reaches half
        // an item and the margin past the largest of them.
        let button = self.button;
        let bounds = [(self.radius, 0.0), (button / 2.0, CENTER / 2.0 + MARGIN / 2.0), (button / (2.0 * (PI / MAX_ITEMS as f32).sin()), 0.0)];
        let room = half - MARGIN;
        let scale = bounds.iter().map(|(a, c)| (room - c) / (a + button / 2.0)).fold(1.0_f32, f32::min).max(0.0);
        let button = (button * scale).max(MIN_BUTTON).min(button);
        // An item size held at the smallest leaves less room for the radius than `scale` gave.
        Self { id: self.id, radius: (self.radius * scale).min(room - button / 2.0).max(0.0), button }
    }

    /// The radius of the close button, inside which the pointer is over it.
    pub fn inner_radius(&self) -> f32 {
        CENTER / 2.0
    }

    /// The radius past which the pointer is outside the ring. The ring's disc is drawn to here.
    pub fn outer_radius(&self) -> f32 {
        self.ring_radius() + self.button / 2.0 + MARGIN
    }

    /// How far the ring and its caption reach from the center in every direction.
    pub fn reach(&self) -> f32 {
        self.outer_radius() + CAPTION
    }

    /// The center of item `index`'s round button.
    fn item_center(&self, center: Pos2, index: usize, count: usize, rtl: bool) -> Pos2 {
        center + mirrored(slot_direction(index, count), rtl) * self.ring_radius()
    }

    /// The bounding square of item `index`'s round button: its widget and accessible bounds.
    /// Neighboring squares can overlap at their corners, so what a click picks comes from
    /// [`hit`], never from which square took it.
    fn item_rect(&self, center: Pos2, index: usize, count: usize, rtl: bool) -> Rect {
        Rect::from_center_size(self.item_center(center, index, count, rtl), Vec2::splat(self.button))
    }

    /// Where `pointer` lies relative to this ring while it is open with `count` items, tested as
    /// the ring tests clicks: around the center it was moved to on screen, and mirrored in a
    /// right-to-left interface. [`RingHit::Outside`] while the ring is closed.
    pub fn hit_test(&self, ctx: &Context, pointer: Pos2, count: usize) -> RingHit {
        let Some(state) = self.state(ctx) else {
            return RingHit::Outside;
        };
        let screen = ctx.content_rect();
        let ring = self.fitted(screen);
        let center = fit_center(state.at, ring.reach(), screen);
        ring.ring_hit(center, pointer, count.min(MAX_ITEMS), crate::direction::is_rtl(ctx))
    }

    fn ring_hit(&self, center: Pos2, pointer: Pos2, count: usize, rtl: bool) -> RingHit {
        hit(center, center + mirrored(pointer - center, rtl), count, self.inner_radius(), self.outer_radius())
    }

    /// The bounding square of the close button: its widget and accessible bounds.
    fn close_rect(&self, center: Pos2) -> Rect {
        Rect::from_center_size(center, Vec2::splat(CENTER))
    }

    /// The id of item `index`'s widget, for focus and accessibility.
    pub fn item_id(&self, index: usize) -> Id {
        self.id.with(("item", index))
    }

    /// The id of the close button's widget.
    pub fn close_id(&self) -> Id {
        self.id.with("close")
    }

    /// Opens the ring around `at`, kept on screen. `held`: the input that opened it, such as a
    /// pen or mouse button, is still down, and the app calls [`Self::release`] when it ends. A
    /// held gesture aims from the press nearest `at` among the buttons that went down in this
    /// pass and are still down; pass the opening button's press position as `at`. A
    /// pointer button that opens the ring also reaches whatever egui assigned the press to
    /// before the ring existed, so bind the ring to a button that no widget takes, or a key.
    /// Opening a ring that is already open leaves it as it is, so a repeating key does not move it
    /// or reset its gesture.
    pub fn open(&self, ctx: &Context, at: Pos2, held: bool) {
        self.open_with(ctx, at, held, None);
    }

    /// Opens the ring around `at` from a fresh press of `shortcut` with exactly its modifiers,
    /// whose key is still down. In this pass the ring reads only such a press as the opening press,
    /// as [`Modifiers::matches_exact`] reads them, so open it on the same test. Until the key goes
    /// up, the ring ignores its repeats, and its release does what [`Self::release`] does.
    pub fn open_held_key(&self, ctx: &Context, at: Pos2, shortcut: KeyboardShortcut) {
        self.open_with(ctx, at, true, Some(shortcut));
    }

    fn open_with(&self, ctx: &Context, at: Pos2, held: bool, opener: Option<KeyboardShortcut>) {
        if self.is_open(ctx) {
            return;
        }
        let restore = ctx.memory(|m| m.focused()).filter(|id| !self.owns(*id));
        let pass_start = self.pass_start_pointer(ctx);
        let mut button = None;
        // With a named shortcut, the gesture starts where the pointer was at its last press in
        // this pass: the pointer's last move before the press, or where it was when the pass
        // began. A held ring opened by [`Self::open`] starts at the press, among the buttons
        // pressed in this pass that are still down, nearest `at`, or else where the pointer is.
        let pointer_at_open = match opener {
            Some(shortcut) => ctx.input(|i| {
                let press = i.raw.events.iter().rposition(|e| is_shortcut_press(e, shortcut));
                let before = press.map_or(&[][..], |at| &i.raw.events[..at]);
                let moved = before.iter().rev().find_map(|e| match e {
                    Event::PointerMoved(p) | Event::PointerButton { pos: p, .. } => Some(Some(*p)),
                    Event::PointerGone => Some(None),
                    _ => None,
                });
                match (press, moved) {
                    (Some(_), Some(at)) => at,
                    (Some(_), None) => pass_start,
                    (None, _) => i.pointer.latest_pos(),
                }
            }),
            None => {
                let press = ctx.input(|i| {
                    // The buttons pressed in this pass that are still down.
                    let mut down: Vec<(egui::PointerButton, Pos2)> = Vec::new();
                    for e in &i.raw.events {
                        if let Event::PointerButton { pos, button, pressed, .. } = e {
                            down.retain(|(b, _)| b != button);
                            if *pressed {
                                down.push((*button, *pos));
                            }
                        }
                    }
                    down.into_iter().filter(|_| held).min_by(|a, b| a.1.distance_sq(at).total_cmp(&b.1.distance_sq(at)))
                });
                button = press.map(|(b, _)| b);
                press.map(|(_, pos)| pos).or_else(|| ctx.pointer_latest_pos())
            }
        };
        let state = State {
            at,
            held,
            opener,
            button,
            released: false,
            pointer_at_open,
            restore,
            pending: None,
            fresh: true,
            drawn: None,
            last_focus: None,
            press_layout: None,
            press_at: None,
            arrows: [false; 4],
            tab: false,
            last_pointer: pass_start,
            keys_used: false,
            opened_pass: ctx.cumulative_pass_nr(),
        };
        ctx.data_mut(|d| d.insert_temp(self.id, state));
    }

    /// The input that opened the ring ended. [`Self::show`] does the same itself when the key passed
    /// to [`Self::open_held_key`] goes up. On the next [`Self::show`], when the pointer has
    /// moved since the ring opened and lies off the close button, the enabled item in its
    /// direction is picked. Otherwise, and always once the arrows or Tab have moved the
    /// highlight, the ring stays open for Enter or a click.
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

    /// Where the pointer was when this pass began: where [`Self::show`] saw it in the last pass,
    /// or else where egui's pointer movement in this pass started from.
    fn pass_start_pointer(&self, ctx: &Context) -> Option<Pos2> {
        let pass = ctx.cumulative_pass_nr();
        match ctx.data(|d| d.get_temp::<(u64, Option<Pos2>)>(self.id.with("pointer"))) {
            Some((seen, at)) if seen + 1 == pass => at,
            _ => ctx.input(|i| i.pointer.latest_pos().map(|p| p - i.pointer.delta())),
        }
    }

    fn owns(&self, id: Id) -> bool {
        id == self.close_id() || (0..MAX_ITEMS).any(|i| id == self.item_id(i))
    }

    fn finish(&self, ctx: &Context, state: State) {
        ctx.data_mut(|d| d.remove::<State>(self.id));
        // egui hit-tests against the layers of the previous pass. Another pass right away drops
        // the ring's full-window layer, so the next click reaches what is underneath.
        ctx.request_repaint();
        // A click that closed the ring also ends this pass's focus handling, which can clear the
        // restored focus; the next [`Self::show`] requests it again.
        if let Some(id) = state.restore {
            ctx.data_mut(|d| d.insert_temp(self.id.with("restore"), id));
        }
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
    /// before a panel takes them, and call [`Self::open`] and [`Self::release`] for the opening
    /// key before it. Consume the press that opens the ring, so it neither reaches the panels nor
    /// moves the ring's highlight. The ring draws on the foreground layer, above the panels, in
    /// either order. On a window too small for the ring, the items and their distance from the
    /// center shrink together until the ring fits, down to an item size of 16 pt.
    pub fn show(&self, ctx: &Context, items: &[RadialItem], labels: &RadialLabels) -> Option<RadialEvent> {
        // A ring opened in the next pass starts its gesture from here when no pointer event in
        // that pass comes before the opening press.
        let seen = (ctx.cumulative_pass_nr(), ctx.pointer_latest_pos());
        ctx.data_mut(|d| d.insert_temp(self.id.with("pointer"), seen));
        self.fitted(ctx.content_rect()).show_ring(ctx, items, labels)
    }

    fn show_ring(&self, ctx: &Context, items: &[RadialItem], labels: &RadialLabels) -> Option<RadialEvent> {
        let restore_key = self.id.with("restore");
        if let Some(id) = ctx.data_mut(|d| {
            let id = d.get_temp::<Id>(restore_key);
            d.remove::<Id>(restore_key);
            id
        }) {
            // Only when focus was cleared: a widget the picked command focused keeps it.
            ctx.memory_mut(|m| {
                if m.focused().is_none_or(|f| self.owns(f)) {
                    m.request_focus(id);
                }
            });
        }
        let mut state = self.state(ctx)?;
        if state.fresh && items.len() > MAX_ITEMS {
            log::warn!("radial menu: {} items passed, the first {MAX_ITEMS} are shown", items.len());
        }
        let items = &items[..items.len().min(MAX_ITEMS)];
        let count = items.len();
        let rtl = crate::direction::is_rtl(ctx);
        let screen = ctx.content_rect();
        let center = fit_center(state.at, self.reach(), screen);
        let ring_hit = |p: Pos2| self.ring_hit(center, p, count, rtl);
        let enabled: Vec<bool> = items.iter().map(|item| item.enabled).collect();
        // Keyboard stops: the enabled items clockwise, then the close button.
        let mut stops: Vec<Id> = (0..count).filter(|i| enabled[*i]).map(|i| self.item_id(i)).collect();
        stops.push(self.close_id());
        // The widgets an assistive technology may still act on while the ring is open.
        let ring_nodes: Vec<egui::accesskit::NodeId> = stops.iter().map(Id::accesskit_id).collect();
        let opening_pass = ctx.cumulative_pass_nr() == state.opened_pass;
        // In the opening pass, the first and the last press of the shortcut or button that opened
        // the ring, among the raw events. A press of the shortcut's key with other modifiers is
        // not one of them. Input before the first came before the ring existed: the ring does not
        // read it and leaves it for the app. The hold starts at the last. Input between two such
        // presses goes to the ring, since the ring cannot tell which press the app opened it on.
        let open_presses = ctx.input(|i| {
            let is_open_press = |e: &Event| match (state.opener, state.button) {
                (Some(shortcut), _) => is_shortcut_press(e, shortcut),
                (None, Some(button)) => matches!(e, Event::PointerButton { button: b, pressed: true, .. } if *b == button),
                (None, None) => false,
            };
            Some((i.raw.events.iter().position(is_open_press)?, i.raw.events.iter().rposition(is_open_press)?))
        });
        let open_presses = open_presses.filter(|_| opening_pass);
        let open_press = open_presses.map(|(first, _)| first);
        // The processed events before the first opening press, and the first processed event
        // after it.
        let (before_open, first_event) = open_press
            .map_or((0, 0), |at| ctx.input(|i| (processed_before(&i.raw.events, &i.events, at), processed_before(&i.raw.events, &i.events, at + 1))));
        let keep = (open_press.unwrap_or(0), before_open);
        let swallow_keys = |ctx: &Context| swallow_keys(ctx, &ring_nodes, keep);
        // Each slot's command, to tell when the app put another command in a slot.
        let mut commands = [None; MAX_ITEMS];
        for (slot, item) in commands.iter_mut().zip(items) {
            *slot = Some(self.id.with(("command", item.id.as_str())));
        }
        let mut enabled_slots = [false; MAX_ITEMS];
        for (slot, item) in enabled_slots.iter_mut().zip(items) {
            *slot = item.enabled;
        }
        let layout = Layout { commands, enabled: enabled_slots, rtl, center, radius: self.ring_radius(), button: self.button };

        // A loss and a return of window focus between two passes both count as a loss.
        if crate::focus::lost(ctx) || ctx.input(|i| i.raw.events.iter().any(|e| matches!(e, Event::WindowFocused(false)))) {
            swallow_keys(ctx);
            self.finish(ctx, state);
            return Some(RadialEvent::Closed);
        }
        // The opening key's hold, and where it ends among this pass's events: at a release of that
        // key, but in the opening pass only one after the press that opened the ring.
        let hold_key = state.opener.map(|shortcut| shortcut.logical_key).filter(|_| state.held);
        let mut hold_ends = None;
        // Where the pointer was when the opening key went up: its last move before the release,
        // or where it was at the end of the last pass. `None` when the app reported the release.
        let mut pointer_at_release: Option<Option<Pos2>> = None;
        if let Some(opener) = hold_key {
            let is_release = |e: &Event| matches!(e, Event::Key { key, pressed: false, .. } if *key == opener);
            let release = ctx.input(|i| {
                let from = open_presses.map_or(0, |(_, last)| last + 1);
                let at = i.raw.events.iter().skip(from).position(is_release)? + from;
                // `Some(None)`: the pointer left the window before the release.
                let moved = i.raw.events[..at].iter().rev().find_map(|e| match e {
                    Event::PointerMoved(p) | Event::PointerButton { pos: p, .. } => Some(Some(*p)),
                    Event::PointerGone => Some(None),
                    _ => None,
                });
                // The same release among the processed events: releases of the key are never
                // consumed, so it is the one with as many before it.
                let earlier = i.raw.events[..at].iter().filter(|e| is_release(e)).count();
                let ends = i.events.iter().enumerate().filter(|(_, e)| is_release(e)).nth(earlier).map(|(n, _)| n);
                Some((ends, moved.unwrap_or(state.last_pointer)))
            });
            if let Some((ends, at)) = release {
                hold_ends = ends;
                pointer_at_release = Some(at);
                state.held = false;
                state.released = true;
            }
        }

        // Focus stays in the ring: on opening, and when the focused item was disabled or removed
        // while the ring is open, it goes to the first stop.
        state.pending = state.pending.filter(|id| stops.contains(id));
        let in_ring = ctx.memory(|m| m.focused()).is_some_and(|id| stops.contains(&id));
        // A focus request from outside the ring, such as an assistive technology's for a control
        // under it, sends focus back to the ring's widget that last had it.
        if std::mem::take(&mut state.fresh) {
            state.pending = Some(stops[0]);
        } else if state.pending.is_none() && !in_ring {
            state.pending = Some(state.last_focus.filter(|id| stops.contains(id)).unwrap_or(stops[0]));
        }
        // Keys step from where focus is going; Enter and Space act only on the widget that has
        // focus now, never on one focus has not reached yet.
        let has_focus = ctx.memory(|m| m.focused());
        let focused = state.pending.or(has_focus);
        // When the app changed the ring's commands since the last pass, no item is activated in
        // this pass: the person aimed at or focused what was drawn before, and a new item moves
        // every sector.
        let layout_changed = state.drawn.is_some_and(|drawn| drawn.commands != commands);
        let changed = [layout_changed; MAX_ITEMS];
        // A pointer click whose press was made over the earlier commands, or over the same
        // commands drawn elsewhere (mirrored, moved or resized), picks nothing either, and a
        // press over an item that was disabled then does not pick that item. A press in this pass
        // landed on what the last pass drew; the press of this pass's click decides, when the
        // button went up and down again. Keys and assistive clicks do not depend on the press.
        let pressed_now = match primary_click(ctx) {
            Some((_, here)) => here,
            None if state.press_layout.is_none() && ctx.input(|i| i.pointer.primary_pressed()) => primary_press_pos(ctx),
            None => None,
        };
        let (at_press, press_at) = match pressed_now {
            Some(at) => (Some(state.drawn.unwrap_or(layout)), Some(at)),
            None => (state.press_layout, state.press_at),
        };
        let press_moved = at_press.is_some_and(|at_press| !at_press.same_places(&layout));
        let press_disabled = at_press.map_or([false; MAX_ITEMS], |at_press| at_press.enabled.map(|on| !on));
        // The press itself landed on an item that was disabled, as that pass drew the ring.
        let press_on_disabled = at_press.zip(press_at).is_some_and(|(drawn, at)| matches!(drawn.hit(at), RingHit::Slot(i) if !drawn.enabled[i]));

        let pointer = ctx.pointer_latest_pos();
        let moved_since_open = |p: Pos2| state.pointer_at_open.is_none_or(|open| open.distance(p) > self.inner_radius());
        // While the key is held, the direction the pointer moved from where it was when the ring
        // opened aims, at any distance, so a flick picks the same item when the ring had to move
        // to stay on screen. Otherwise, the item under the pointer.
        let aimed_slot = |p: Pos2, held: bool| {
            if held {
                let origin = state.pointer_at_open.unwrap_or(center);
                if origin.distance(p) > self.inner_radius() { sector(mirrored(p - origin, rtl), count) } else { None }
            } else {
                match ring_hit(p) {
                    RingHit::Slot(i) => Some(i),
                    _ => None,
                }
            }
        };
        let aimed = |p: Pos2, held: bool| aimed_slot(p, held).filter(|i| enabled[*i]);

        // The pick that the opening key's release this pass makes when no key acted before it in
        // an earlier pass or in this one. The key loop below reads the release in order, so only
        // an Esc, Tab or arrow that the loop acts on before the release keeps the flick from
        // picking, and keys after the release do not.
        let keys_used_before = state.keys_used;
        let flick = || {
            let at = pointer_at_release.filter(|_| !keys_used_before)?;
            let i = at.filter(|p| moved_since_open(*p)).and_then(|p| aimed(p, true)).filter(|i| !changed[*i])?;
            Some(RadialEvent::Picked(items[i].id.clone()))
        };
        // A release found among the raw events but not among the processed ones comes first.
        if hold_ends.is_none()
            && let Some(picked) = flick()
        {
            swallow_keys(ctx);
            self.finish(ctx, state);
            return Some(picked);
        }

        // Keys are read before the ring's widgets exist, and egui's own focus step is canceled,
        // so a key moves focus once and never out of the ring.
        let mut event = None;
        let mut move_to = None;
        let held = state.held;
        let (mut arrows_after, mut tab_after) = (state.arrows, state.tab);
        ctx.input(|input| {
            // Only a fresh press activates: a key repeat, a press in the opening pass, or any
            // press while the opening key is held could be that key, and a held key picks on
            // release instead. A press before the opening key's release in this pass came while
            // it was held.
            let after_hold = |at: usize| match hold_key {
                Some(_) => hold_ends.is_some_and(|end| at > end),
                None => !held,
            };
            let activates = |at: usize, e: &Event| {
                !opening_pass
                    && after_hold(at)
                    && matches!(e, Event::Key { key: Key::Enter | Key::Space, pressed: true, repeat: false, modifiers, .. }
                        if modifiers.matches_logically(Modifiers::NONE))
            };
            // What Enter or Space does to `target`: an enabled item whose slot holds the command it
            // held in the last pass is picked, and the close button closes the ring.
            let activate = |target: Option<Id>| match (0..count).find(|i| target == Some(self.item_id(*i))) {
                Some(i) if enabled[i] && !changed[i] => Some(RadialEvent::Picked(items[i].id.clone())),
                Some(_) => None,
                None if target == Some(self.close_id()) => Some(RadialEvent::Closed),
                None => None,
            };
            let hold = hold_key.map(|key| (key, hold_ends.unwrap_or(usize::MAX)));
            // The keys act in the order they came, up to the first that ends the ring: Esc, or
            // Enter or Space on a target it acts on. Enter or Space acts on the highlight as it is
            // at that press: where Tab and the arrows before it moved it, or else the widget that
            // has focus now. One that does not act leaves the keys after it to act.
            //
            // An arrow pressed this pass counts even if it went up again, together with the other
            // arrows pressed since the last Tab and the arrows the ring took that are still held,
            // so two arrows give a diagonal. An arrow held as the opening key, or pressed with
            // Alt, Ctrl or Command, is not the ring's. A repeat of Tab or an arrow moves the
            // highlight only when the ring took its press: one that went down before the ring
            // opened, or with a modifier the ring does not take, repeats without moving it.
            let len = stops.len();
            let mut at = focused.and_then(|id| stops.iter().position(|s| *s == id));
            let mut run = [false; 4];
            let mut held_now = state.arrows;
            let mut tab_held = state.tab;
            for (n_event, e) in input.events.iter().enumerate() {
                // The opening key went up here, and no key before it moved the highlight: the
                // flick it ends picks now, before the keys after it.
                if hold_ends == Some(n_event)
                    && move_to.is_none()
                    && let Some(picked) = flick()
                {
                    event = Some(picked);
                    break;
                }
                if n_event < first_event {
                    continue;
                }
                // Repeats of the opening key during its hold, up to its release, are not input to
                // the ring. Repeats after the release come from a new press.
                let opener_repeat = |key: Key, repeat: bool| repeat && hold.is_some_and(|(k, end)| k == key && n_event < end);
                if let Event::Key { key: Key::Escape, pressed: true, repeat, modifiers, .. } = e
                    && modifiers.matches_logically(Modifiers::NONE)
                    && !opener_repeat(Key::Escape, *repeat)
                {
                    event = Some(RadialEvent::Closed);
                    break;
                }
                if activates(n_event, e) {
                    event = activate(move_to.map_or(has_focus, |stop: usize| stops.get(stop).copied()));
                    if event.is_some() {
                        break;
                    }
                    continue;
                }
                match arrow_step(e, n_event, hold) {
                    Some(ArrowStep::Release(n)) => held_now[n] = false,
                    Some(ArrowStep::Press { n, .. }) if matches!(e, Event::Key { repeat: true, .. }) && !held_now[n] => {}
                    Some(ArrowStep::Press { n, holds }) => {
                        held_now[n] |= holds;
                        run[n] = true;
                        let on = |n: usize| if run[n] || held_now[n] { 1.0 } else { 0.0 };
                        let direction = mirrored(vec2(on(3) - on(2), on(1) - on(0)), rtl);
                        if let Some(stop) = nearest_enabled(direction, &enabled).and_then(|i| stops.iter().position(|s| *s == self.item_id(i))) {
                            at = Some(stop);
                            move_to = Some(stop);
                        }
                    }
                    None => match e {
                        Event::Key { key: Key::Tab, pressed: false, .. } => tab_held = false,
                        // Each Tab steps forward and each Shift+Tab back. Without focus in the
                        // ring, Tab starts at the first stop and Shift+Tab at the last.
                        Event::Key { key: Key::Tab, pressed: true, repeat, modifiers, .. }
                            if !modifiers.alt
                                && modifiers.matches_logically(Modifiers::NONE)
                                && !opener_repeat(Key::Tab, *repeat)
                                && (!*repeat || tab_held) =>
                        {
                            tab_held = true;
                            let stop = match (at, modifiers.shift) {
                                (Some(from), false) => (from + 1) % len,
                                (Some(from), true) => (from + len - 1) % len,
                                (None, false) => 0,
                                (None, true) => len - 1,
                            };
                            at = Some(stop);
                            move_to = Some(stop);
                            run = [false; 4];
                        }
                        _ => {}
                    },
                }
            }
            (arrows_after, tab_after) = (held_now, tab_held);
        });
        (state.arrows, state.tab) = (arrows_after, tab_after);
        // Tab, the arrows, Esc, Enter and Space were read above; this removes their presses.
        swallow_keys(ctx);
        ctx.memory_mut(|m| m.move_focus(FocusDirection::None));
        if let Some(event) = event {
            self.finish(ctx, state);
            return Some(event);
        }
        // Once the arrows or Tab have moved the highlight, a release never picks, and Enter picks
        // the highlighted item.
        if let Some(stop) = move_to.and_then(|i| stops.get(i).copied()) {
            state.pending = Some(stop);
            state.keys_used = true;
        }

        // A release the app reported for the pointer button that held the ring picks from where
        // that button last went up in this pass; a move after it does not change the pick.
        let button_up = || {
            let button = state.button?;
            ctx.input(|i| {
                i.raw.events.iter().rev().find_map(|e| match e {
                    Event::PointerButton { pos, button: b, pressed: false, .. } if *b == button => Some(*pos),
                    _ => None,
                })
            })
        };
        if std::mem::take(&mut state.released)
            && !state.keys_used
            && let Some(i) = pointer_at_release
                .unwrap_or_else(|| button_up().or(pointer))
                .filter(|p| moved_since_open(*p))
                .and_then(|p| aimed(p, true))
                .filter(|i| !changed[*i])
        {
            let picked = items[i].id.clone();
            self.finish(ctx, state);
            return Some(RadialEvent::Picked(picked));
        }
        // A key that moved the highlight this pass keeps it over a pointer move in the same pass.
        let pointer_moved = move_to.is_none() && ctx.input(|i| i.pointer.delta() != Vec2::ZERO);
        if pointer_moved && let Some(i) = pointer.and_then(|p| aimed(p, state.held)) {
            state.pending = Some(self.item_id(i));
        }

        // The caption names what Enter or a release would pick: the focused item once the keys
        // moved it, or while the key is held, the item the pointer aims at. Aimed at a disabled
        // item, which a release does not pick, it shows nothing rather than another item.
        let caption_for = if state.keys_used {
            CaptionFor::Focus
        } else if let Some(i) = pointer.filter(|p| state.held && moved_since_open(*p)).and_then(|p| aimed_slot(p, true)) {
            if enabled[i] { CaptionFor::Item(i) } else { CaptionFor::Nothing }
        } else if pointer.is_some_and(moved_since_open) {
            CaptionFor::Hover
        } else {
            // A pointer that has not moved since the ring opened rests on the close button by
            // chance; the caption names the focused item that Enter would pick.
            CaptionFor::Focus
        };
        let mut ended = None;
        let mut visible = false;
        Area::new(self.id.with("area")).order(Order::Foreground).fixed_pos(screen.min).movable(false).constrain(false).show(ctx, |ui| {
            // A see-through layer over the whole window takes the clicks outside the ring, so
            // they close it instead of reaching what is underneath.
            visible = ui.is_visible();
            // Clicks only: the backdrop is not a control a screen reader or Tab should reach.
            let backdrop = ui.interact(screen, self.id.with("backdrop"), Sense::CLICK);
            // The area covers the window too, so `Context::layer_id_at` finds the ring's layer
            // for an app that reads the pointer itself.
            ui.expand_to_include_rect(screen);
            ui.scope_builder(UiBuilder::new().max_rect(screen).accessibility_role(Role::Menu).accessibility_label(labels.menu.clone()), |ui| {
                ended = self.draw(ui, center, items, labels, View { rtl, changed, press_disabled, press_moved, press_on_disabled }, caption_for)
            });
            if ended.is_none() && backdrop.clicked() {
                ended = match release_pos(ctx, backdrop.interact_pointer_pos()).map(ring_hit) {
                    Some(RingHit::Slot(i)) if enabled[i] && !changed[i] && !press_moved && !press_on_disabled && !press_disabled[i] => {
                        Some(RadialEvent::Picked(items[i].id.clone()))
                    }
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
            state.last_pointer = pointer;
            // A press lands on what was drawn in the last pass.
            let drawn = state.drawn.unwrap_or(layout);
            (state.press_layout, state.press_at) = match ctx.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down())) {
                (true, _) => (Some(drawn), primary_press_pos(ctx)),
                (false, true) => (state.press_layout, state.press_at),
                (false, false) => (None, None),
            };
            state.drawn = Some(layout);
            state.last_focus = ctx.memory(|m| m.focused()).filter(|id| self.owns(*id)).or(state.last_focus);
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
    fn draw(
        &self,
        ui: &mut Ui,
        center: Pos2,
        items: &[RadialItem],
        labels: &RadialLabels,
        view: View,
        caption_for: CaptionFor,
    ) -> Option<RadialEvent> {
        let rtl = view.rtl;
        let count = items.len();
        let visuals = ui.visuals().clone();
        let painter = ui.painter().clone();
        // The disc covers exactly the area where a click picks an item.
        painter.circle(center, self.outer_radius(), visuals.window_fill, visuals.window_stroke);

        let mut ended = None;
        let (mut hover_caption, mut focus_caption, mut item_caption) = (None, None, None);
        for (index, item) in items.iter().enumerate() {
            let rect = self.item_rect(center, index, count, rtl);
            let sense = if item.enabled { Sense::click() } else { Sense::hover() };
            let response = ui.interact(rect, self.item_id(index), sense);
            let (label, enabled) = (item.label.clone(), item.enabled);
            response.widget_info(|| WidgetInfo::labeled(Role::MenuItem, enabled, label.as_str()));
            if let Some(detail) = &item.detail {
                ui.ctx().accesskit_node_builder(response.id, |node| node.set_description(detail.as_str()));
            }
            // The response covers the button's bounding square, which can overlap a neighbor's at
            // the corners; the pointer is over the item only inside its circle.
            let over = ui.ctx().pointer_hover_pos().is_some_and(|p| p.distance(rect.center()) <= self.button / 2.0);
            let lit = item.enabled && (response.has_focus() || over);
            let tip = match &item.detail {
                Some(detail) => format!("{}\n{detail}", item.label),
                None => item.label.clone(),
            };
            if over {
                hover_caption = Some(tip.clone());
            }
            if response.has_focus() {
                focus_caption = Some(tip.clone());
            }
            if caption_for == CaptionFor::Item(index) {
                item_caption = Some(tip.clone());
            }
            self.paint_item(ui, &response, item, lit);
            let response = if over { response.on_hover_text(tip) } else { response };
            if response.clicked() {
                ended = self.click(center, items, view, &response).or(ended);
            }
        }

        let close = ui.interact(self.close_rect(center), self.close_id(), Sense::click());
        let name = labels.close.clone();
        close.widget_info(|| WidgetInfo::labeled(Role::Button, true, name.as_str()));
        let close_over = ui.ctx().pointer_hover_pos().is_some_and(|p| p.distance(center) <= CENTER / 2.0);
        if close_over {
            hover_caption = Some(labels.close.clone());
        }
        let state = if close_over || close.has_focus() { &visuals.widgets.hovered } else { &visuals.widgets.inactive };
        painter.circle(center, CENTER / 2.0, state.bg_fill, state.bg_stroke);
        let arm = CENTER / 5.0;
        for (a, b) in [(vec2(-arm, -arm), vec2(arm, arm)), (vec2(-arm, arm), vec2(arm, -arm))] {
            painter.line_segment([center + a, center + b], state.fg_stroke);
        }
        if close.has_focus() {
            painter.circle_stroke(center, CENTER / 2.0 + 2.0, visuals.selection.stroke);
            focus_caption = Some(labels.close.clone());
        }
        let close = if close_over { close.on_hover_text(labels.close.as_str()) } else { close };
        if close.clicked() {
            ended = self.click(center, items, view, &close).or(ended);
        }

        let caption = match caption_for {
            CaptionFor::Focus => focus_caption.or(hover_caption),
            CaptionFor::Item(_) => item_caption.or(hover_caption),
            CaptionFor::Hover => hover_caption.or(focus_caption),
            CaptionFor::Nothing => None,
        };
        if let Some(text) = caption {
            let screen = ui.max_rect();
            let pad = ui.spacing().button_padding;
            let wrap = (screen.width() - 2.0 * pad.x).max(1.0);
            let font = TextStyle::Body.resolve(ui.style());
            // Below the ring's disc, or above it, so the caption leaves the buttons uncovered.
            // When neither side has room for the detail line, the caption shows the label alone,
            // and the tooltip still has the detail. Without room for the label, there is no
            // caption.
            let place = |size: Vec2| {
                let edge = self.outer_radius() + size.y / 2.0;
                let (below, above) = (center + vec2(0.0, edge), center - vec2(0.0, edge));
                if below.y + size.y / 2.0 <= screen.max.y {
                    Some(below)
                } else if above.y - size.y / 2.0 >= screen.min.y {
                    Some(above)
                } else {
                    None
                }
            };
            let mut galley = painter.layout(text.clone(), font.clone(), visuals.text_color(), wrap);
            let mut at = place(galley.size() + 2.0 * pad);
            if at.is_none()
                && let Some((label, _)) = text.split_once('\n')
            {
                galley = painter.layout(label.to_owned(), font, visuals.text_color(), wrap);
                at = place(galley.size() + 2.0 * pad);
            }
            if let Some(at) = at {
                let back = caption_rect(at, galley.size() + 2.0 * pad, screen);
                painter.rect(
                    back,
                    visuals.widgets.noninteractive.corner_radius,
                    visuals.window_fill,
                    visuals.window_stroke,
                    egui::StrokeKind::Inside,
                );
                painter.galley(back.min + pad, galley, visuals.text_color());
            }
        }
        ended
    }

    /// What a click on an item's or the close button's widget does. A pointer click acts on
    /// where it landed, as [`hit`] reads it: the bounding squares overlap at their corners. A
    /// click with no pointer, from an assistive technology, acts on the widget itself, and on an
    /// item only while its slot holds the command it held in the last pass.
    fn click(&self, center: Pos2, items: &[RadialItem], view: View, response: &Response) -> Option<RadialEvent> {
        let own = (0..items.len()).find(|i| response.id == self.item_id(*i));
        let pointer = release_pos(&response.ctx, response.interact_pointer_pos()).filter(|_| response.clicked_by(egui::PointerButton::Primary));
        let at = match pointer {
            Some(p) => self.ring_hit(center, p, items.len(), view.rtl),
            None if own.is_some_and(|i| view.changed[i]) => return None,
            None => own.map_or(RingHit::Center, RingHit::Slot),
        };
        // Only a pointer click depends on its press.
        let press_stale = |i: usize| pointer.is_some() && (view.press_moved || view.press_on_disabled || view.press_disabled[i]);
        match at {
            RingHit::Slot(i) if items[i].enabled && !view.changed[i] && !press_stale(i) => Some(RadialEvent::Picked(items[i].id.clone())),
            RingHit::Slot(_) => None,
            RingHit::Center | RingHit::Outside => Some(RadialEvent::Closed),
        }
    }

    fn paint_item(&self, ui: &Ui, response: &Response, item: &RadialItem, lit: bool) {
        let rect = response.rect;
        let radius = self.button / 2.0;
        let visuals = ui.visuals();
        let state = if lit { &visuals.widgets.hovered } else { &visuals.widgets.inactive };
        let tint = if item.enabled { state.fg_stroke.color } else { visuals.weak_text_color() };
        let painter = ui.painter();
        if item.enabled {
            painter.circle(rect.center(), radius, state.bg_fill, state.bg_stroke);
        } else {
            // A disabled item is an outline without a fill, so the difference does not rest on
            // color alone.
            painter.circle_stroke(rect.center(), radius, visuals.widgets.noninteractive.bg_stroke);
        }
        if response.has_focus() {
            painter.circle_stroke(rect.center(), radius + 2.0, visuals.selection.stroke);
        }
        match &item.icon {
            Some(icon) => {
                let inner = Rect::from_center_size(rect.center(), Vec2::splat(self.button)).shrink(ICON_INSET.min(self.button / 4.0));
                if inner.is_positive() {
                    egui::Image::new(icon.clone()).tint(tint).paint_at(ui, inner);
                }
            }
            None => {
                // The theme's button font, at least 0.8 of the item's radius and at most its full
                // radius, so a larger text size also enlarges the letter.
                let letter: String = item.label.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
                let theme = TextStyle::Button.resolve(ui.style());
                let font = FontId::new(theme.size.clamp(radius * 0.8, radius), theme.family);
                painter.text(rect.center(), Align2::CENTER_CENTER, letter, font, tint);
            }
        }
    }
}
