//! How an open menu of the bar behaves like a native one: a menu or submenu taller than the
//! window scrolls (wheel, and the ▲ / ▼ arrows at its top and bottom, which scroll while the
//! pointer rests on them), and the keyboard drives it: ↑ / ↓ move the highlight (and keep it in
//! view), → opens a submenu or the next menu, ← closes the submenu or opens the previous menu,
//! ↩ / Space run the highlighted item, Esc closes.
//!
//! The highlight is view state in egui memory, per top-level menu. It follows the pointer too, so
//! the mouse and the keyboard share one highlighted row, as in a native menu.
//!
//! An open submenu stays open while the pointer heads for it: crossing the rows between its row
//! and the submenu, on a diagonal, a curved path or with a pause on the way, neither closes it
//! nor opens the submenu of a row crossed. The pointer resting on another row, or moving away
//! from the submenu, switches as usual.
//!
//! The bar's own keys (Alt and F10 focusing it, mnemonics; [`super::keys`]) drive the same
//! state: the open menu ([`Nav::top`], through the popup ids in [`Nav::tops`]), the level the
//! arrows move in ([`Nav::level`]), its rows' mnemonics ([`Nav::row_with_mnemonic`]) and
//! [`Nav::open_sub_row`] to go into a submenu.

use egui::containers::menu::{MenuState, SubMenu, find_menu_root};
use egui::{Id, Key, Modifiers, Pos2, Rect, Sense, Ui, pos2, vec2};

use craft_keymap::CommandRef;

/// Height of a scroll-arrow strip.
pub(crate) const ARROW: f32 = 16.0;
/// Gap kept between a menu and the window edge.
const EDGE: f32 = 6.0;
/// How fast resting on a scroll arrow scrolls, in points per second.
const SPEED: f32 = 600.0;
/// How far above and below an open submenu the pointer may aim and still be heading for it.
const AIM_SLACK: f32 = 24.0;
/// How long the pointer may rest on another row, heading for an open submenu, before that row
/// takes over (macOS waits about as long).
const AIM_REST: f32 = 0.3;

/// The pointer heading for the open submenu of a row.
#[derive(Clone, Debug)]
struct Aim {
    /// The row whose submenu is open, and its index in its level.
    row: Id,
    index: usize,
    /// Where the pointer last was on that row.
    from: Pos2,
    /// The pointer was off the row with its submenu still open in the last frame.
    held: bool,
}

/// One row of a menu level: a command or a submenu.
#[derive(Clone, Debug)]
pub(crate) struct Row {
    /// The row's widget id (a submenu's popup id derives from it).
    pub widget: Id,
    pub enabled: bool,
    /// The command it runs; `None` for a submenu.
    pub command: Option<CommandRef>,
    /// Where it was drawn (scrolled out of view when outside its level's `views` rect).
    #[cfg_attr(not(test), allow(dead_code))]
    pub rect: Rect,
    /// The letter that runs it from the keyboard, upper case.
    pub mnemonic: Option<char>,
}

/// Keyboard and highlight state of the open menu.
#[derive(Clone, Debug, Default)]
pub(crate) struct Nav {
    /// The open top-level menu (index into the bar's titles).
    pub top: Option<usize>,
    /// Popup ids of the top-level menus, from the last frame.
    pub tops: Vec<Id>,
    /// The level the arrow keys move in (0 = the top-level menu).
    pub level: usize,
    /// Highlighted row per level.
    hi: Vec<Option<usize>>,
    /// This frame's rows per level (deeper levels are dropped when their parent redraws).
    pub rows: Vec<Vec<Row>>,
    /// Menu-root id per level (the owner of egui's `MenuState` for that level).
    roots: Vec<Id>,
    /// Underline the rows' mnemonics this frame.
    pub underline: bool,
    /// Bottom of the menu bar: where top-level menus hang from.
    pub bar_bottom: Option<f32>,
    /// The visible (scrolled) part of each level.
    pub views: Vec<Rect>,
    /// The full height of each level's rows (taller than its view when it scrolls).
    pub contents: Vec<f32>,
    /// Per level: the row its open submenu hangs from (level 0 has none). From the frame
    /// before, since a submenu is drawn while its row is.
    anchors: Vec<Option<Rect>>,
    /// Scroll the highlighted row of `level` into view when it is next drawn.
    reveal: bool,
    /// Highlight the first enabled row of this level once it has been drawn (a keyboard-opened
    /// submenu).
    first_at: Option<usize>,
    /// Per level: the pointer heading for the open submenu of one of its rows.
    aims: Vec<Option<Aim>>,
    /// A held submenu of this level let go: the row under the pointer takes the highlight even
    /// if the pointer rests.
    follow: Option<usize>,
}

fn open_id() -> Id {
    Id::unique("craft-menu-bar-open")
}

/// Whether one of the bar's menus is open. An app leaves the keys to it meanwhile: its
/// shortcuts wait until the menu closes.
pub fn is_open(ctx: &egui::Context) -> bool {
    // Only while the bar is still drawn: an app that hides it with a menu open (full screen)
    // gets its keys back.
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<u64>(open_id())).is_some_and(|seen| seen + 2 >= pass)
}

impl Nav {
    /// Last frame's state, reset when another top-level menu (or none) is open now.
    pub fn load(ctx: &egui::Context, id: Id) -> Nav {
        let mut nav: Nav = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
        let top = nav.tops.iter().position(|id| egui::Popup::is_id_open(ctx, *id));
        if top != nav.top {
            nav = Nav { top, tops: std::mem::take(&mut nav.tops), ..Default::default() };
        }
        nav
    }

    /// The state as the last frame left it.
    #[cfg(test)]
    pub fn current(ctx: &egui::Context, id: Id) -> Nav {
        ctx.data(|d| d.get_temp(id)).unwrap_or_default()
    }

    /// The highlighted row of `level`, if any.
    pub fn highlighted(&self, level: usize) -> Option<usize> {
        self.hi.get(level).copied().flatten()
    }

    fn set_hi(&mut self, level: usize, row: Option<usize>) {
        if self.hi.len() <= level {
            self.hi.resize(level + 1, None);
        }
        self.hi[level] = row;
    }

    /// Start drawing `level`: forget its rows from before, and every deeper level's.
    fn begin_level(&mut self, level: usize, root: Id) {
        self.rows.truncate(level);
        self.rows.resize(level + 1, Vec::new());
        self.roots.truncate(level);
        self.roots.resize(level + 1, root);
        self.views.truncate(level);
        self.views.resize(level + 1, Rect::NOTHING);
        self.contents.truncate(level);
        self.contents.resize(level + 1, 0.0);
    }

    /// Draw a row of `level` with `add`, lit like a hovered row when the keyboard (or the
    /// pointer) is on it, and register it for the keys. `add` returns the row's response. A row
    /// without a `command` opens a submenu; `mnemonic` is the letter that runs or opens it.
    pub fn row<R>(
        &mut self,
        ui: &mut Ui,
        level: usize,
        enabled: bool,
        command: Option<&CommandRef>,
        mnemonic: Option<char>,
        add: impl FnOnce(&mut Ui, &mut Nav) -> (egui::Response, R),
    ) -> R {
        let i = self.rows.get(level).map_or(usize::MAX, Vec::len);
        // While a submenu is kept open for the pointer heading to it, its row stays lit and the
        // rows crossed on the way don't light up.
        let held = self.aims.get(level).and_then(Option::as_ref).filter(|a| a.held).map(|a| a.index);
        let lit = enabled && (held == Some(i) || held.is_none() && self.highlighted(level) == Some(i));
        let saved = (ui.visuals().widgets.inactive, ui.visuals().widgets.hovered);
        if lit {
            ui.visuals_mut().widgets.inactive = ui.visuals().widgets.hovered;
        } else if held.is_some() {
            ui.visuals_mut().widgets.hovered = ui.visuals().widgets.inactive;
        }
        let root = self.roots.get(level).copied().filter(|_| command.is_none());
        let open = root.and_then(|root| MenuState::from_id(ui.ctx(), root, |s| s.open_item));
        let (r, out) = add(ui, self);
        (ui.visuals_mut().widgets.inactive, ui.visuals_mut().widgets.hovered) = saved;
        if let Some(root) = root {
            self.aim(level, i, root, open, &r);
        }
        if let Some(rows) = self.rows.get_mut(level) {
            rows.push(Row { widget: r.id, enabled, command: command.cloned(), rect: r.rect, mnemonic });
            if let Some(index) = held {
                self.set_hi(level, Some(index));
            } else {
                self.after_row(level, i, lit, &r);
            }
        }
        out
    }

    /// After a submenu row is drawn: if egui just closed its submenu because the pointer is over
    /// another row of this menu, keep it open while the pointer heads for it. `open` was the
    /// menu's open submenu before the row was drawn.
    fn aim(&mut self, level: usize, index: usize, root: Id, open: Option<Id>, r: &egui::Response) {
        if self.aims.len() <= level {
            self.aims.resize(level + 1, None);
        }
        let Some(p) = r.ctx.pointer_hover_pos() else { return };
        let was_held = self.aims.get(level).and_then(Option::as_ref).is_some_and(|a| a.row == r.id && a.held);
        let held = self.hold(level, index, root, open, p, r);
        if was_held && !held {
            // Let go: the row under the pointer takes the highlight, moving or not.
            self.follow = Some(level);
        }
    }

    /// [`Self::aim`]: is this row's submenu open with the pointer off the row (held) now?
    fn hold(&mut self, level: usize, index: usize, root: Id, open: Option<Id>, p: Pos2, r: &egui::Response) -> bool {
        let ctx = &r.ctx;
        let sub = SubMenu::id_from_widget_id(r.id);
        let target = self.views.get(level + 1).copied();
        let Some(slot) = self.aims.get_mut(level) else { return false };
        if open != Some(sub) {
            if slot.as_ref().is_some_and(|a| a.row == r.id) {
                *slot = None;
            }
            return false;
        }
        if r.interact_rect.contains(p) {
            *slot = Some(Aim { row: r.id, index, from: p, held: false });
            return false;
        }
        let Some(aim) = slot.as_mut().filter(|a| a.row == r.id) else { return false };
        // Off its row with the submenu still open (egui keeps it while the pointer moves straight
        // at it): the row stays lit.
        aim.held = true;
        if MenuState::from_id(ctx, root, |s| s.open_item) == Some(sub) {
            return true;
        }
        aim.held = false;
        // A click or a key closes menus as usual; so does leaving the submenu's direction, or
        // resting on another row.
        let (acted, rest) =
            ctx.input(|i| (i.pointer.any_pressed() || i.pointer.any_click() || !i.keys_down.is_empty(), i.pointer.time_since_last_movement()));
        let Some(target) = target else { return false };
        if acted || rest >= AIM_REST || !heading(aim.from, p, target) {
            return false;
        }
        MenuState::from_id(ctx, root, |s| s.open_item = Some(sub));
        aim.held = true;
        ctx.request_repaint_after_secs(AIM_REST - rest);
        true
    }

    /// After a row is drawn: the pointer moving onto it highlights it; a keyboard move scrolls
    /// the highlighted row into view.
    fn after_row(&mut self, level: usize, i: usize, lit: bool, r: &egui::Response) {
        let moved = r.ctx.input(|inp| inp.pointer.delta() != egui::Vec2::ZERO);
        let follow = self.follow == Some(level);
        if (moved || follow) && r.hovered() && !lit {
            if follow {
                self.follow = None;
            }
            self.set_hi(level, Some(i));
            self.level = level;
            r.ctx.request_repaint();
        }
        if lit && self.reveal && level == self.level {
            r.scroll_to_me(None);
            self.reveal = false;
        }
    }

    fn enabled(&self, level: usize) -> Vec<usize> {
        self.rows.get(level).map(|rows| rows.iter().enumerate().filter(|(_, r)| r.enabled).map(|(i, _)| i).collect()).unwrap_or_default()
    }

    /// ↑ / ↓: the next enabled row (wrapping), or the first / last one.
    fn step(&mut self, level: usize, down: bool) {
        let on = self.enabled(level);
        let (Some(&first), Some(&last)) = (on.first(), on.last()) else { return };
        let next = match self.highlighted(level) {
            None => {
                if down {
                    first
                } else {
                    last
                }
            }
            Some(cur) if down => on.iter().copied().find(|&i| i > cur).unwrap_or(first),
            Some(cur) => on.iter().rev().copied().find(|&i| i < cur).unwrap_or(last),
        };
        self.set_hi(level, Some(next));
        self.hi.truncate(level + 1);
        self.level = level;
        self.reveal = true;
        // The keys move the highlight now, not the pointer heading for a submenu.
        if let Some(aim) = self.aims.get_mut(level) {
            *aim = None;
        }
    }

    /// Light the first enabled row of `level` once it has been drawn (a menu opened from the
    /// keyboard, as on Windows).
    pub fn highlight_first(&mut self, level: usize) {
        self.first_at = Some(level);
    }

    /// The enabled row of the level the arrows are in whose mnemonic is `c`.
    pub fn row_with_mnemonic(&self, c: char) -> Option<Row> {
        let level = self.level.min(self.rows.len().saturating_sub(1));
        self.rows.get(level)?.iter().find(|r| r.enabled && r.mnemonic == Some(c)).cloned()
    }

    /// Open the submenu of row `widget` of the level the arrows are in.
    pub fn open_sub_row(&mut self, ctx: &egui::Context, widget: Id) {
        let level = self.level.min(self.rows.len().saturating_sub(1));
        self.open_sub(ctx, level, widget);
    }

    /// Open the submenu of row `widget` of `level` and move the keyboard into it, on its first
    /// enabled row.
    pub fn open_sub(&mut self, ctx: &egui::Context, level: usize, widget: Id) {
        let Some(&root) = self.roots.get(level) else { return };
        let sub = SubMenu::id_from_widget_id(widget);
        // egui drops an open item it has never drawn: mark it shown so it survives to the next frame.
        MenuState::mark_shown(ctx, sub);
        MenuState::from_id(ctx, root, |s| s.open_item = Some(sub));
        self.level = level + 1;
        self.hi.truncate(level + 1);
        self.first_at = Some(level + 1);
    }

    /// Remember the row the submenu of `level` (1 = a top-level menu's submenu) hangs from.
    pub fn set_anchor(&mut self, level: usize, row: Rect) {
        if self.anchors.len() <= level {
            self.anchors.resize(level + 1, None);
        }
        if let Some(a) = self.anchors.get_mut(level) {
            *a = Some(row);
        }
    }

    /// Handle this frame's navigation keys (after the menus were drawn). `tops` are the bar's
    /// popup ids, in screen order; an activated command lands in `clicked`.
    pub fn keys(&mut self, ctx: &egui::Context, tops: &[Id], clicked: &mut Option<CommandRef>) {
        self.tops = tops.to_vec();
        let Some(top) = self.top.filter(|_| !tops.is_empty()) else { return };
        let level = self.level.min(self.rows.len().saturating_sub(1));
        self.level = level;
        let press = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
        let current = self.highlighted(level).and_then(|i| self.rows.get(level)?.get(i)).cloned();
        let switch = |by: usize| {
            let n = tops.len();
            if let Some(&id) = tops.get((top + by) % n) {
                egui::Popup::open_id(ctx, id);
            }
        };
        if press(Key::Escape) {
            egui::Popup::close_all(ctx);
        } else if press(Key::ArrowDown) {
            self.step(level, true);
        } else if press(Key::ArrowUp) {
            self.step(level, false);
        } else if press(Key::ArrowRight) {
            match current {
                Some(Row { widget, enabled: true, command: None, .. }) => self.open_sub(ctx, level, widget),
                _ => switch(1),
            }
        } else if press(Key::ArrowLeft) {
            if level > 0 {
                if let Some(&root) = self.roots.get(level - 1) {
                    MenuState::from_id(ctx, root, |s| s.open_item = None);
                }
                self.hi.truncate(level);
                self.level = level - 1;
            } else {
                switch(tops.len() - 1);
            }
        } else if press(Key::Enter) || press(Key::Space) {
            match current {
                Some(Row { enabled: true, command: Some(c), .. }) => {
                    *clicked = Some(c);
                    egui::Popup::close_all(ctx);
                }
                Some(Row { widget, enabled: true, command: None, .. }) => self.open_sub(ctx, level, widget),
                _ => {}
            }
        } else {
            return;
        }
        ctx.request_repaint();
    }

    pub fn store(self, ctx: &egui::Context, id: Id) {
        let pass = ctx.cumulative_pass_nr();
        ctx.data_mut(|d| {
            if self.top.is_some() {
                d.insert_temp(open_id(), pass);
            } else {
                d.remove::<u64>(open_id());
            }
            d.insert_temp(id, self);
        });
    }
}

/// Is the pointer at `p`, having left a submenu's row at `from`, heading for the submenu at
/// `target`? It is while it stays inside the triangle from `from` to the submenu's near edge
/// (with [`AIM_SLACK`] above and below), as macOS menus judge it.
pub(crate) fn heading(from: Pos2, p: Pos2, target: Rect) -> bool {
    if !target.is_positive() {
        return false;
    }
    let x = if target.center().x >= from.x { target.left() } else { target.right() };
    let (a, b) = (pos2(x, target.top() - AIM_SLACK), pos2(x, target.bottom() + AIM_SLACK));
    let side = |u: Pos2, v: Pos2| (v.x - u.x) * (p.y - u.y) - (v.y - u.y) * (p.x - u.x);
    let (d1, d2, d3) = (side(from, a), side(a, b), side(b, from));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

/// Is the pointer over `rect` on this ui's layer (not under a submenu drawn on top)?
fn pointer_on(ui: &Ui, rect: Rect) -> bool {
    ui.ctx().pointer_hover_pos().is_some_and(|p| rect.contains(p) && ui.ctx().layer_id_at(p) == Some(ui.layer_id()))
}

/// The height a menu level's rows may take. Every level stays below the bar: a top-level menu
/// hangs from it, and a submenu that is too tall for the window between its row and the bottom,
/// or (opening upward, as egui does when that fits) between the bar and its row, scrolls instead
/// of egui sliding it up over the bar, where it would hide the menu titles. `anchor` is the
/// submenu's row (unknown on its first frame), `rows` the level's rows' height when known (the
/// popup shrinks to it), `frame` the popup's margins.
pub(crate) fn level_room(screen: Rect, bar_bottom: Option<f32>, depth: usize, anchor: Option<Rect>, rows: Option<f32>, frame: f32) -> f32 {
    let top = bar_bottom.unwrap_or(screen.top()).max(screen.top()) + EDGE;
    let bottom = screen.bottom() - EDGE;
    let below_bar = bottom - top;
    let room = match anchor.filter(|_| depth > 1) {
        Some(row) => {
            // Downward from the row, or upward from it, whichever leaves more room. egui opens a
            // submenu upward only when the downward one doesn't fit the window (it hangs from the
            // row's top less half the frame margin).
            let (down, up) = (bottom - row.top(), row.bottom() - top);
            let opens_up = |h: f32| row.top() - (frame - 2.0) / 2.0 + h + frame > screen.bottom();
            let want = rows.unwrap_or(f32::INFINITY);
            if want > down && up > down && opens_up(want.min(up)) { up } else { down }.min(below_bar)
        }
        None => below_bar,
    } - frame;
    if room.is_finite() { room.max(4.0 * ARROW) } else { 4.0 * ARROW }
}

/// Draw one level of a menu, bounded by the window: when its rows don't fit they scroll, with
/// scroll arrows at the top and bottom. `depth` is 1 for a top-level menu. `label` translates the
/// arrows' accessible names.
pub(crate) fn level(ui: &mut Ui, depth: usize, nav: &mut Nav, label: &dyn Fn(&'static str) -> String, rows: impl FnOnce(&mut Ui, &mut Nav)) {
    let ctx = ui.ctx().clone();
    let level = depth.saturating_sub(1);
    nav.begin_level(level, find_menu_root(ui).scope_id);
    let screen = ctx.content_rect();
    // The menu frame's margin and stroke around the rows.
    let frame = ui.spacing().menu_margin.sum().y + 2.0;
    let anchor = nav.anchors.get(level).copied().flatten();
    let key = ui.scope_id().with(("craft-menu-level", depth));
    // Rows' height from the last frame: does this level overflow?
    let content: Option<f32> = ctx.data(|d| d.get_temp(key));
    let room = level_room(screen, nav.bar_bottom, depth, anchor, content, frame);
    let over = content.is_some_and(|h| h > room + 0.5);
    let up = over.then(|| ui.allocate_exact_size(vec2(0.0, ARROW), Sense::hover()).0);
    let height = if over { room - 2.0 * ARROW } else { room };
    // The popup's `Ui` is only as tall as the popup was last frame (egui's `default_area_size`,
    // 400 pt, on the first): a scroll area never grows past that, so every menu would stop at that
    // height and scroll even on a tall window. Ask for the room the window has; auto-shrink then
    // fits the area to its rows, so a menu scrolls only when they don't fit.
    let mut area = egui::ScrollArea::vertical()
        .id_salt(("craft-menu-level", depth))
        .max_height(height)
        .min_scrolled_height(height)
        .scroll_source(egui::containers::scroll_area::ScrollSource::ALL);
    // Where the scroll arrows moved the rows last frame; egui's scroll state has no setter.
    if let Some(y) = ctx.data_mut(|d| d.remove_temp::<f32>(key.with("scroll-to"))) {
        area = area.vertical_scroll_offset(y);
    }
    let out = area.show(ui, |ui| rows(ui, nav));
    let down = over.then(|| ui.allocate_exact_size(vec2(0.0, ARROW), Sense::hover()).0);
    ctx.data_mut(|d| d.insert_temp(key, out.content_size.y));
    if let Some(v) = nav.views.get_mut(level) {
        // `inner_rect` is the size asked for, before auto-shrink fits it to the rows.
        *v = Rect::from_min_size(out.inner_rect.min, vec2(out.inner_rect.width(), out.inner_rect.height().min(out.content_size.y)));
    }
    if let Some(c) = nav.contents.get_mut(level) {
        *c = out.content_size.y;
    }
    if nav.first_at == Some(level) {
        nav.first_at = None;
        nav.step(level, true);
        ctx.request_repaint();
    }
    let (Some(up), Some(down)) = (up, down) else { return };
    let (live_color, dead_color) = (ui.visuals().text_color(), ui.visuals().weak_text_color().gamma_multiply(0.5));
    let x = ui.min_rect().x_range();
    let max = (out.content_size.y - out.inner_rect.height()).max(0.0);
    let offset = out.state.clamped_offset().y;
    let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 0.1);
    let mut target = offset;
    for (strip, dir, live) in [(up, -1.0, offset > 0.5), (down, 1.0, offset < max - 0.5)] {
        let strip = Rect::from_x_y_ranges(x, strip.y_range());
        let c = strip.center();
        let (h, w) = (4.5, 6.0);
        let tri = if dir < 0.0 {
            vec![pos2(c.x - w, c.y + h * 0.5), pos2(c.x + w, c.y + h * 0.5), pos2(c.x, c.y - h * 0.5)]
        } else {
            vec![pos2(c.x - w, c.y - h * 0.5), pos2(c.x + w, c.y - h * 0.5), pos2(c.x, c.y + h * 0.5)]
        };
        ui.painter().add(egui::Shape::convex_polygon(tri, if live { live_color } else { dead_color }, egui::Stroke::NONE));
        let id = key.with(dir < 0.0);
        let r = ui.interact(strip, id, Sense::hover());
        let name = label(if dir < 0.0 { "Scroll menu up" } else { "Scroll menu down" });
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, live, &name));
        if live && pointer_on(ui, strip) {
            target = (offset + dir * SPEED * dt.max(1.0 / 120.0)).clamp(0.0, max);
            ctx.request_repaint();
        }
        // The wheel scrolls over the arrows too.
        if pointer_on(ui, strip) {
            let wheel = ctx.input(|i| i.smooth_scroll_delta.y);
            if wheel != 0.0 {
                target = (target - wheel).clamp(0.0, max);
            }
        }
    }
    if target != offset {
        ctx.data_mut(|d| d.insert_temp(key.with("scroll-to"), target));
    }
}
