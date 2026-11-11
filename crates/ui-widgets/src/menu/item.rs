//! `MenuItem` (11.4, 12.3): one row of a menu, and the level it shares with the rows around it.

use super::popup::MenuStep;
use crate::contract::{
    Access, Common, Component, Cx, Focus as _, MeasureCx, Name, NoValue, Outcome, PaintUi, PartsCx, Pointer as _, Props, Remember as _, Role,
    Styled as _,
};
use crate::display::Kbd;
use crate::list::RowStyleCx as _;
use crate::primitives::{IconSpec, PaintShape, StrokeWidth, TextLayout, TextRun, glyph};
use crate::runtime::access::PopupKind;
use crate::runtime::commands::{CommandRef, FireSource, ResolvedCommand};
use crate::runtime::env::Label;
use crate::runtime::frame;
use crate::style::{ComponentStyle, Flags, IconLook, StyleCx, SurfaceLook, TextLook, TypeRole};
use craft_behavior::{ColumnKey, Flow, IconId, Insets, Metrics, NavMove, PartKey, Sense};
use craft_keymap::mnemonic;
use craft_theme::{Rgba, States};
use egui::{Context, Id, Rect, Response, Vec2, pos2};
use std::borrow::Cow;

use super::level::{MenuLevel, Row};

/// The part a row's shortcut text shows in, as the face of a `Kbd`.
const SHORTCUT: PartKey = PartKey::new("shortcut");

/// The column of a level's check and radio marks, and the gap after it, in the menu body's column group.
const MARKS: ColumnKey = ColumnKey::new("menu_marks");

/// The column of a level's row icons, and the gap after it, after the mark column.
const ICONS: ColumnKey = ColumnKey::new("menu_icons");

/// The seconds the pointer rests on a row before the row opens its submenu, or closes the open submenu.
const DWELL: f64 = 0.3;

/// `label` with a `&` marker before the first character that is `letter`, so the level's assignment honours a row's
/// own mnemonic; a label without that letter is left as it is.
fn with_marker(label: &str, letter: char) -> String {
    match label.char_indices().find(|(_, c)| c.eq_ignore_ascii_case(&letter)) {
        Some((at, _)) => format!("{}&{}", &label[..at], &label[at..]),
        None => label.to_owned(),
    }
}

/// The mark a row shows in its mark column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mark {
    /// A check mark, shown while the value is true.
    Check(bool),
    /// A radio mark, shown while the value is true.
    Radio(bool),
}

/// The style of a menu row: the hover fill, the label, the row's icon and its detail.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct MenuItemStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
    /// The label and the marks.
    #[style(text)]
    text: TextLook,
    /// The row's icon, the check and radio marks and the submenu arrow.
    #[style(icon)]
    icon: IconLook,
    /// The detail after the label, in `text.muted`.
    #[style(text)]
    detail: TextLook,
    /// The least space between the label and the shortcut: `space.gap.section`.
    shortcut_gap: f32,
    /// The gap between a submenu and its parent menu: `space.gap.tight`.
    submenu_gap: f32,
}

/// What a row reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct MenuItemOutput {
    /// The row's response.
    pub response: Response,
    /// The row ran this pass: a click, Enter or Space on it, or its mnemonic letter. A submenu row never runs; it
    /// asks for its submenu.
    pub ran: bool,
    /// A submenu row asks for its submenu to open: the pointer rested on it, Forward (Right, mirrored under
    /// right-to-left) was pressed on it, or it was activated.
    pub opens_submenu: bool,
    /// A submenu row's submenu is the open one of its level: show the `Menu` after the row.
    pub submenu_open: bool,
    /// The row's rect grown by the menu's padding: where a submenu hangs from, so its first row is level with this one
    /// and it starts at the popup's edge.
    pub(crate) frame: Rect,
    level: MenuLevel,
    id: Id,
}

impl MenuItemOutput {
    /// A submenu row's menu was shown at `rect` (`open`), or closed itself (`!open`).
    pub(crate) fn note_submenu(&self, open: bool, rect: Rect) {
        self.level.note_submenu(self.id, open, rect);
    }
}

impl Outcome for MenuItemOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        crate::testing::StoryOutput { id: Some(self.response.id), clicked: self.ran, ..crate::testing::StoryOutput::default() }
    }
}

/// The widths of a row's columns and its trailing items, in points; zero for a column the row does not show. The mark
/// and icon columns include the gap after them.
struct Columns {
    mark: f32,
    icon: f32,
    detail: f32,
    trail: f32,
}

/// Where the parts of a row lie.
struct Placed {
    mark: Rect,
    icon: Rect,
    label: Rect,
    detail: Rect,
    trail: Rect,
}

/// A row of a menu: a label, an optional icon, a check or radio mark, shortcut text and a detail. A row bound to a
/// command takes its label, shortcut, enabled and checked state from the command and fires it as a menu row.
///
/// - Running a row (click, Enter or Space, its mnemonic letter) closes the whole chain of menus it sits in and
///   returns focus to where it was before the menus opened (before a menu bar took it, or the Help search field),
///   and only then fires the command: Undo picked from a menu undoes in the text field that held focus.
/// - A submenu row opens a submenu instead: the output's `opens_submenu` asks the caller to open the `Menu` it shows
///   after the row.
/// - [`keep_open`](MenuItem::keep_open) keeps the menus open when the row runs (a check row that toggles).
/// - The mnemonic is the `&` letter of the label, else the first letter no earlier row of the level uses. The
///   underline sits under that letter while Alt is held or the keyboard drives the menu, never on macOS.
/// - The pointer moves the level's active row; the row under it paints with the shared row fill.
#[derive(crate::contract::Component, crate::contract::Props)]
pub struct MenuItem<'a> {
    #[common]
    common: Common,
    command_ref: Option<CommandRef>,
    submenu: bool,
    /// An icon before the label.
    #[prop(into)]
    icon: Option<IconSpec<'a>>,
    /// A check mark, shown while the value is true; the row reserves the mark column either way.
    #[prop]
    check: Option<bool>,
    /// A radio mark, shown while the value is true; the row reserves the mark column either way.
    #[prop]
    radio_mark: Option<bool>,
    /// The letter that runs the row, in place of the level's own pick.
    #[prop]
    mnemonic: Option<char>,
    /// A muted text after the label.
    #[prop(into)]
    detail: Option<Label>,
    /// Shortcut text at the end of the row, written by the caller; a command row shows its chord.
    #[prop(into)]
    shortcut: Option<Label>,
    /// Running the row leaves the menus open.
    #[prop]
    keep_open: bool,
    /// The checked state of the entry the row shows: a radio mark in a value menu, a check mark elsewhere while true.
    #[prop(name = "entry_checked")]
    entry: Option<bool>,
    ran: bool,
    opens: bool,
    open: bool,
    active: bool,
}

impl<'a> MenuItem<'a> {
    /// A row showing `label`, which is also its accessible name.
    pub fn new(label: impl Into<Label>) -> Self {
        MenuItem {
            common: Common::new(),
            command_ref: None,
            submenu: false,
            icon: None,
            check: None,
            radio_mark: None,
            mnemonic: None,
            detail: None,
            shortcut: None,
            keep_open: false,
            entry: None,
            ran: false,
            opens: false,
            open: false,
            active: false,
        }
        .label(label)
    }

    /// A row bound to `command`: its label, chord, enabled state and checked state follow the command, and running it
    /// fires the command.
    pub fn command(command: impl Into<CommandRef>) -> Self {
        let command = command.into();
        let mut item = MenuItem::new(Label::Empty);
        item.command_ref = Some(command.clone());
        <MenuItem as Props>::command(item, command)
    }

    /// A row that opens a submenu, with an arrow at its end.
    pub fn submenu(label: impl Into<Label>) -> Self {
        MenuItem { submenu: true, ..MenuItem::new(label) }
    }

    /// The icon a bound command names, when it names one with a static name.
    fn command_icon(command: Option<&ResolvedCommand>) -> Option<IconSpec<'static>> {
        match command?.icon.as_ref()? {
            Cow::Borrowed(name) => Some(IconSpec::svg(IconId::new(name))),
            Cow::Owned(_) => None,
        }
    }

    fn lead_icon(&self, command: Option<&ResolvedCommand>) -> Option<IconSpec<'_>> {
        self.icon.clone().or_else(|| MenuItem::command_icon(command))
    }

    /// The mark in the mark column: the caller's own, the bound command's, or the entry's.
    fn mark(&self, command: Option<&ResolvedCommand>, level: &MenuLevel) -> Option<Mark> {
        if let Some(on) = self.radio_mark {
            return Some(Mark::Radio(on));
        }
        if let Some(on) = self.check {
            return Some(Mark::Check(on));
        }
        if let Some(on) = command.and_then(|command| command.checked) {
            return Some(Mark::Check(on));
        }
        match self.entry {
            Some(on) if level.values() => Some(Mark::Radio(on)),
            Some(true) => Some(Mark::Check(true)),
            _ => None,
        }
    }

    /// The shortcut as the `Kbd` that shows it.
    fn keys(&self, command: Option<&ResolvedCommand>) -> Option<Kbd> {
        match (&self.command_ref, command) {
            (Some(reference), Some(resolved)) if resolved.chord.is_some() => Some(Kbd::command(reference.clone())),
            _ => self.shortcut.clone().map(Kbd::new),
        }
    }

    fn place(&self, columns: &Columns, area: Rect, rtl: bool, gap: f32, trail_gap: f32) -> Placed {
        let step = |width: f32, gap: f32| if width > 0.0 { width + gap } else { 0.0 };
        let (mark, flow) = Flow::new(area, rtl).take_start(columns.mark);
        let (icon, flow) = flow.take_start(columns.icon);
        let (trail, flow) = flow.take_end(step(columns.trail, trail_gap));
        let (detail, flow) = flow.take_end(step(columns.detail, gap));
        Placed { mark, icon, label: flow.rect(), detail, trail }
    }

    fn register(&self, ctx: &Context, level: &MenuLevel, id: Id, label: &str, enabled: bool) {
        let text = match self.mnemonic {
            Some(letter) => with_marker(label, letter),
            None => label.to_owned(),
        };
        level.register(ctx.cumulative_pass_nr(), Row { id, text, enabled });
    }

    /// The row's mark and icon columns with the gap after each: as wide as the column group of the menu body gives
    /// them (the widest of its rows, from this pass's measuring run), or the row's own outside a group.
    fn lead_columns(&self, ctx: &Context, side: f32, gap: f32, command: Option<&ResolvedCommand>, level: &MenuLevel) -> (f32, f32) {
        let own = |on: bool| if on { side + gap } else { 0.0 };
        let shared = |column: &ColumnKey, width: f32| crate::layout::ambient_width(ctx, column, width).unwrap_or(width);
        (shared(&MARKS, own(self.mark(command, level).is_some())), shared(&ICONS, own(self.lead_icon(command).is_some())))
    }

    fn columns(&self, cx: &MeasureCx<'_, Self>, level: &MenuLevel) -> Columns {
        let side = cx.icon();
        let command = cx.command();
        let (mark, icon) = self.lead_columns(cx.ui.ctx(), side, cx.gap(), command, level);
        let detail = self.detail.as_ref().map_or(0.0, |detail| {
            let detail = cx.env.resolve(detail);
            cx.layout_at(&TextRun::new(detail.as_str()).role(cx.style.detail.role), f32::INFINITY).size().x
        });
        let trail = if self.submenu { side } else { self.keys(command).map_or(0.0, |keys| cx.face(keys).x) };
        Columns { mark, icon, detail, trail }
    }
}

impl Component for MenuItem<'_> {
    const ROLE: Role = Role::MENU_ITEM;
    const ALT_ROLES: &'static [Role] = &[Role::MENU_ITEM_CHECK_BOX, Role::MENU_ITEM_RADIO];
    const OVERLAYS: bool = false;
    const KEYS: crate::runtime::focus::KeyUse = crate::runtime::focus::KeyUse::of(crate::contract::Input::Pointer).with_moves();
    type Style = MenuItemStyle;
    type Output = MenuItemOutput;

    fn style(cx: &StyleCx<'_>, _: MenuItemStyleKey) -> MenuItemStyle {
        let tokens = cx.tokens;
        let text = &tokens.color.text;
        let c = |rgba: Rgba| cx.color(rgba);
        // A checked row shows its mark, not a selection fill.
        let clear = c(Rgba::TRANSPARENT);
        let mut surface = cx.command_row_look();
        surface.fill = surface.fill.on(States::new(clear, clear, clear, clear));
        let ink = |base: Rgba| {
            let disabled = c(text.disabled);
            cx.command_row_ink(base).on(States::new(c(base), c(base), c(base), disabled)).off(States::new(c(base), c(base), c(base), disabled))
        };
        let control = cx.control();
        // A row's content sits `space.pad.regular` in from the popup's own padding, so a label starts clear of the edge.
        let mut metrics = control.metrics();
        metrics.padding = Insets::new(tokens.space.pad.regular, tokens.space.pad.regular, metrics.padding.top, metrics.padding.bottom);
        MenuItemStyle {
            surface,
            metrics,
            text: TextLook::new(TypeRole::Body, ink(text.normal)),
            icon: IconLook::new(control.icon, ink(text.normal)),
            detail: TextLook::new(TypeRole::Caption, ink(text.muted)),
            shortcut_gap: tokens.space.gap.section,
            submenu_gap: tokens.space.gap.tight,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let level = cx.env.get::<MenuLevel>();
        // The measuring run of the menu body registers the rows, so the real run's letters come from this pass's rows.
        self.register(cx.ui.ctx(), &level, cx.id, cx.resolved.label, cx.resolved.enabled);
        let columns = self.columns(cx, &level);
        let text = mnemonic::marked(cx.resolved.label).0;
        let run = TextRun::new(text.as_str()).role(cx.style.text.role);
        let label = if text.is_empty() { Vec2::ZERO } else { cx.layout_at(&run, f32::INFINITY).size() };
        let gap = cx.gap();
        let step = |width: f32, gap: f32| if width > 0.0 { width + gap } else { 0.0 };
        let rest = label.x + step(columns.detail, gap) + step(columns.trail, cx.style.shortcut_gap.max(gap));
        let padding = cx.style.metrics.padding;
        // A row measured before the level's widest mark or icon read narrower columns: the body's group adds the
        // widest columns to what the rows reach past them.
        crate::layout::report_beyond(cx.ui.ctx(), &[MARKS, ICONS], padding.start + padding.end + rest);
        let width = columns.mark + columns.icon + rest;
        Vec2::new(cx.room.map_or(width, |room| width.min(room)), label.y.max(cx.icon()))
    }

    fn fills(&self) -> Option<craft_behavior::Axis> {
        Some(craft_behavior::Axis::X)
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    /// The shortcut's slot, which paints as the face of a `Kbd`.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let level = cx.env.get::<MenuLevel>();
        let columns = self.columns(cx, &level);
        if columns.trail == 0.0 || self.submenu {
            return;
        }
        let rtl = cx.flow.is_rtl();
        let placed = self.place(&columns, cx.content_rect, rtl, cx.gap(), cx.style.shortcut_gap.max(cx.gap()));
        let (rect, _) = Flow::new(placed.trail, rtl).take_end(columns.trail);
        cx.part(SHORTCUT, rect);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        cx.join_roving();
        let ctx = cx.core.ctx.clone();
        let level = cx.env.get::<MenuLevel>();
        let enabled = cx.resolved.enabled;
        self.register(&ctx, &level, cx.id, cx.resolved.label, enabled);
        let (mut forward, mut back) = (false, false);
        while let Some((step, _)) = cx.intents().take_move() {
            match step {
                NavMove::Forward => forward = true,
                NavMove::Back => back = true,
                _ => {}
            }
        }
        if back && level.is_submenu() {
            level.request_close();
        } else if !level.is_submenu() && (back || (forward && !self.submenu)) {
            let rtl = cx.flow.is_rtl();
            level.request_step(if back != rtl { MenuStep::Prev } else { MenuStep::Next });
            frame::request_repaint(&ctx);
        }
        let now = frame::clock(&ctx);
        let moving = ctx.input(|input| input.pointer.delta() != Vec2::ZERO);
        let hovered = cx.response.hovered();
        let focused = cx.is_focused();
        let aimed = moving && level.aimed(cx.id);
        let keyboard = level.keyboard();
        let (waited, focus_gained, key_started) = {
            let rest = cx.memory::<Rest>();
            let gained = focused && !rest.focused;
            rest.focused = focused;
            let started = keyboard && !rest.keyboard;
            rest.keyboard = keyboard;
            // The pointer heading for the open submenu does not count as resting on this row.
            if hovered && !aimed {
                (now - *rest.since.get_or_insert(now), gained, started)
            } else {
                rest.since = None;
                (0.0, gained, started)
            }
        };
        // The focused row of a menu that the pointer opened is not active: the keyboard makes it so when focus lands on
        // it or the keyboard starts driving the menu.
        if enabled && ((hovered && moving) || (focused && keyboard && (focus_gained || key_started))) && level.activate_row(cx.id) {
            frame::request_repaint(&ctx);
        }
        // A disabled row is never active.
        self.active = enabled && level.is_active(cx.id);
        if hovered && waited < DWELL {
            frame::request_repaint(&ctx);
        }
        let typed = enabled && level.take_run(cx.id);
        let asked = enabled && (typed || cx.activation().is_some());
        if self.submenu {
            let rested = enabled && hovered && waited >= DWELL;
            self.opens = asked || (enabled && forward) || rested;
            if self.opens && level.open_submenu(cx.id) {
                frame::request_repaint(&ctx);
            }
            self.open = level.submenu_open(cx.id);
            // The active row is in the open submenu: this row keeps only the open-submenu tint.
            if self.open && level.descended(cx.id, focused) {
                self.active = false;
            }
            return;
        }
        if hovered && waited >= DWELL && level.clear_submenu() {
            frame::request_repaint(&ctx);
        }
        // The run closes the menus: focus leaves them first, so the command reaches the stop focus returns to.
        if asked && !self.keep_open {
            crate::runtime::focus::return_from_menus(&ctx);
        }
        // Every activation of a menu row reaches its command as `FireSource::Menu`, whether it was typed, clicked or keyed.
        // A typed letter also runs a row that has no command: the injected activation closes the menu.
        if typed || (asked && cx.command.is_some()) {
            cx.activate(FireSource::Menu);
        }
        if asked && self.keep_open {
            cx.keep_open();
        }
        self.ran = asked;
    }

    /// A submenu row never runs: it opens its submenu, and the menus stay open.
    fn activated(&self, cx: &Cx<'_, Self>) -> bool {
        !self.submenu && cx.activation().is_some()
    }

    fn flags(&self) -> Flags {
        let mut flags = Flags::empty();
        flags.set(Flags::CHECKED, self.check == Some(true) || self.radio_mark == Some(true) || self.entry == Some(true));
        flags.set(Flags::LEAD, self.active);
        // The row whose submenu is open keeps the hover tint while another row is active.
        flags.set(Flags::EMPHASIZED, self.open && !self.active);
        flags
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (rtl, side, gap) = (cx.flow.is_rtl(), cx.style.icon.side, cx.style.metrics.gap);
        let level = cx.env.get::<MenuLevel>();
        let command = cx.command.as_deref();
        let (mark_column, icon_column) = self.lead_columns(&cx.core.ctx, side, gap, command, &level);
        let text = mnemonic::marked(cx.resolved.label).0;
        let mark = self.mark(command, &level);
        let draw = ui.draw();
        let detail = self.detail.as_ref().map(|detail| cx.env.resolve(detail));
        let detail_run =
            detail.as_ref().map(|detail| TextRun::new(detail.as_str()).role(cx.style.detail.role).color(cx.resolve(&cx.style.detail.color)));
        let detail_width = detail_run.as_ref().map_or(0.0, |run| draw.layout(run, f32::INFINITY).size().x);
        let trail = if self.submenu { side } else { cx.part_if_declared(SHORTCUT).map_or(0.0, |part| part.rect.width()) };
        let columns = Columns { mark: mark_column, icon: icon_column, detail: detail_width, trail };
        let placed = self.place(&columns, cx.content_rect, rtl, gap, cx.style.shortcut_gap.max(gap));
        let ink = cx.resolve(&cx.style.icon.color);
        let centred = |slot: Rect| Rect::from_center_size(slot.center(), Vec2::splat(side));
        if columns.mark > 0.0 {
            let square = Rect::from_center_size(Flow::new(placed.mark, rtl).take_start(side).0.center(), Vec2::splat(side));
            match mark {
                Some(Mark::Radio(true)) => draw.fill(&glyph::dot(square.center(), side), ink),
                Some(Mark::Check(true)) => draw.stroke(&glyph::check(square), StrokeWidth::Medium, ink),
                _ => {}
            }
        }
        if columns.icon > 0.0
            && let Some(icon) = self.lead_icon(command)
        {
            let slot = Flow::new(placed.icon, rtl).take_start(side).0;
            let icon = if icon.color.is_some() { icon } else { icon.color(ink) };
            draw.icon(centred(slot), &icon);
        }
        let run = TextRun::new(text.as_str()).elide().role(cx.style.text.role).color(cx.resolve(&cx.style.text.color));
        let layout = draw.layout(&run, placed.label.width());
        let size = layout.size();
        let x = if rtl { placed.label.max.x - size.x } else { placed.label.min.x };
        let y = placed.label.center().y - size.y / 2.0;
        draw.galley(pos2(x, y), &layout);
        let underlined = level.keyboard() || cx.modifiers().alt;
        if underlined
            && let Some((_, at)) = level.mnemonic_of(cx.id)
            && let Some((from, width)) = glyph_span(&layout, at)
        {
            let line = y + size.y;
            let (from, to) = (x + from, x + from + width);
            draw.stroke(&PaintShape::Path(vec![pos2(from, line), pos2(to, line)]), StrokeWidth::Thin, cx.resolve(&cx.style.text.color));
        }
        if let Some(run) = &detail_run {
            let layout = draw.layout(run, f32::INFINITY);
            let size = layout.size();
            let x = if rtl { placed.detail.min.x } else { placed.detail.max.x - size.x };
            draw.galley(pos2(x, placed.detail.center().y - size.y / 2.0), &layout);
        }
        if self.submenu {
            let slot = Flow::new(placed.trail, rtl).take_end(side).0;
            draw.icon(centred(slot), &crate::controls::disclosure::DisclosureGlyph::submenu().color(ink));
        } else if cx.part_if_declared(SHORTCUT).is_some()
            && let Some(keys) = self.keys(command)
        {
            ui.face(SHORTCUT, keys);
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let level = cx.env.get::<MenuLevel>();
        access.name(Name::Text(mnemonic::marked(cx.resolved.label).0));
        match self.mark(cx.command.as_deref(), &level) {
            Some(Mark::Radio(_)) => {
                access.role(Role::MENU_ITEM_RADIO);
            }
            Some(Mark::Check(_)) => {
                access.role(Role::MENU_ITEM_CHECK_BOX);
            }
            None => {}
        }
        if self.submenu {
            access.has_popup(PopupKind::Menu);
        }
        if let Some((letter, _)) = level.mnemonic_of(cx.id) {
            access.access_key(letter);
        }
        if let Some(text) = &self.shortcut {
            access.shortcut(cx.env.resolve(text).as_str().to_owned());
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> MenuItemOutput {
        let level = cx.env.get::<MenuLevel>();
        // The frame is where a submenu hangs: the row with the popup padding, widened by `gap.tight` on both sides so a
        // submenu that flips to the other side keeps the gap.
        let frame = cx.response.rect.expand(level.pad()).expand2(egui::vec2(cx.style.submenu_gap, 0.0));
        MenuItemOutput { response: cx.response.clone(), ran: self.ran, opens_submenu: self.opens, submenu_open: self.open, frame, level, id: cx.id }
    }
}

/// The span of the glyph for character `at` of a single-row layout: its start from the layout's left edge and its
/// advance. The glyph rects are in display order, so an underline taken from them is right in right-to-left text
/// too. `None` when the layout cut the character off.
fn glyph_span(layout: &TextLayout, at: usize) -> Option<(f32, f32)> {
    let row = layout.galley.rows.first()?;
    let glyph = row.row.glyphs.get(at)?;
    Some((row.pos.x + glyph.pos.x, glyph.advance_width))
}

/// How long the pointer has rested on a row, and whether the row had the focus in the last pass.
#[derive(Clone, Default)]
struct Rest {
    since: Option<f64>,
    focused: bool,
    /// Whether the keyboard drove the menu in the last pass.
    keyboard: bool,
}
