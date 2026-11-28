//! `MenuItem` (11.4, 12.3): one row of a menu, and the level it shares with the rows around it.

use super::popup::MenuStep;
use crate::contract::{
    Access, Common, Component, Cx, Focus as _, MeasureCx, Name, NoValue, Outcome, PaintUi, PartsCx, Pointer as _, Props, Remember as _, Role,
    Styled as _,
};
use crate::display::Kbd;
use crate::list::RowStyleCx as _;
use crate::primitives::{IconSpec, StrokeWidth, TextRun, glyph};
use crate::runtime::access::PopupKind;
use crate::runtime::commands::{CommandRef, FireSource, ResolvedCommand};
use crate::runtime::env::Label;
use crate::runtime::frame;
use crate::style::{ComponentStyle, Flags, IconLook, StyleCx, SurfaceLook, TextLook, TypeRole};
use craft_behavior::{ColumnKey, Flow, IconId, Insets, Metrics, NavMove, PartKey, Sense};
use craft_keymap::mnemonic;
use craft_theme::{Contrast, MenuTint, Rgba, States};
use egui::{Context, Id, Rect, Response, Vec2, pos2};
use std::borrow::Cow;

use super::level::{MenuLevel, Row};

/// The part a row's shortcut text shows in, as the face of a `Kbd`.
const SHORTCUT: PartKey = PartKey::new("shortcut");

/// The column of a level's check and radio marks, and the gap after it, in the menu body's column group.
const MARKS: ColumnKey = ColumnKey::new("menu_marks");

/// The column of a level's row icons, and the gap after it, after the mark column.
const ICONS: ColumnKey = ColumnKey::new("menu_icons");

/// The column of a level's samples, and the gap before it, between the label and the shortcut.
const SAMPLES: ColumnKey = ColumnKey::new("menu_samples");

/// The seconds the pointer rests on a row it crossed on its way to another row's open submenu before the row takes
/// over: opens its own submenu, or closes the open one. A row the pointer does not cross that way takes over at once.
const AIM_REST: f64 = 0.3;

/// Closes the menus out from the one a picked row runs in. Focus leaves them first, so a command the row fires after
/// this reaches the stop focus returns to. Every row that runs on a pick (`MenuItem`, `MenuIconRow`) closes through
/// this one path.
pub(crate) fn close_on_pick(ctx: &Context) {
    if let Some(menu) = crate::runtime::focus::running_menu(ctx) {
        crate::runtime::focus::return_from_pick(ctx, crate::runtime::focus::PickedIn::Menus(menu));
    }
}

/// What the pointer and the keyboard did to a menu row in this pass. Every row that can be the level's active row
/// (`MenuItem`, `MenuIconRow`) reads it through [`RowInput::of`] and makes itself active by [`RowInput::activate`], so
/// the rules live once: which row is active, and what the first key in a menu the pointer opened does.
pub(crate) struct RowInput {
    /// The pointer moved in this pass.
    pub moving: bool,
    /// The pointer is over the row.
    pub hovered: bool,
    /// The row holds the focus.
    pub focused: bool,
    /// The row took Enter or Space as the first key in a menu the pointer opened, holding the focus the menu gave it
    /// as it opened, while the pointer had not lit it: the row does not run on it and is not made active by it.
    /// [`RowInput::of`] answers the key instead.
    pub withheld: bool,
    keyboard: bool,
    gained: bool,
    started: bool,
}

impl RowInput {
    /// The row's input in this pass; the row's memory keeps the focus and keyboard facts of the pass before, to see
    /// when focus lands on the row or the keyboard starts driving the menu.
    ///
    /// Enter or Space as the first key in a menu the pointer opened reaches the focused row, which the pointer may never
    /// have lit: the row withholds it, and the key runs the row the pointer lit, or with none lit only closes the menu,
    /// as a native menu does. A submenu row is no exception: the first key opens no submenu the pointer did not light.
    /// Only the focus the menu gave as it opened is withheld from: a row or part that a request focused (an AccessKit
    /// `Focus`, the app) was aimed at, and the key runs it.
    pub(crate) fn of<C: Component>(cx: &mut Cx<'_, C>, level: &MenuLevel) -> RowInput {
        // The rest restarts at each move: a tracked read.
        let moving = crate::runtime::pointer::frame(&cx.core.ctx).delta != Vec2::ZERO;
        let hovered = cx.response.hovered();
        let focused = cx.is_focused();
        let keyboard = level.keyboard();
        let by_key = cx.activation().is_some_and(|activation| activation.by_key);
        let id = cx.id;
        let rest = cx.memory::<Rest>();
        let gained = focused && !rest.focused;
        rest.focused = focused;
        let started = keyboard && !rest.keyboard;
        rest.keyboard = keyboard;
        let opened = focused && crate::runtime::focus::focus_came_with_open(&cx.core.ctx);
        let lit = level.active_enabled();
        let withheld = started && by_key && opened && lit != Some(id);
        if withheld {
            match lit {
                Some(row) => level.run(row),
                None => {
                    let ctx = &cx.core.ctx;
                    if let Some(menu) = crate::runtime::focus::running_menu(ctx) {
                        crate::runtime::focus::return_from_pick(ctx, crate::runtime::focus::PickedIn::Menus(menu));
                        crate::runtime::overlay::request_close(ctx, menu, crate::runtime::overlay::CloseReason::Requested);
                    }
                }
            }
        }
        RowInput { moving, hovered, focused, withheld, keyboard, gained, started }
    }

    /// Makes the row `id` the level's active row when it `runs` and the keyboard landed on it. The pointer's row is
    /// made active by its `Menu` before the rows run (`MenuLevel::follow_pointer`); the focused row of a menu the
    /// pointer opened is not active until the keyboard lands on it, and a withheld first key does not make it so.
    pub(crate) fn activate(&self, ctx: &Context, id: Id, level: &MenuLevel, runs: bool) {
        let landed = self.focused && self.keyboard && (self.gained || self.started);
        if runs && !self.withheld && landed && level.activate_row(id) {
            frame::request_repaint(ctx);
        }
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
    /// The detail line under the label, in `text.muted`.
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
    /// The row is its level's active row: the pointer moved onto it, or the keyboard moved to it.
    pub active: bool,
    /// The row's rect grown by the menu's padding: where a submenu hangs from, so its first row is level with this one
    /// and it starts at the popup's edge.
    pub(crate) frame: Rect,
    level: MenuLevel,
    id: Id,
}

impl MenuItemOutput {
    pub(super) fn sibling_submenu_open(&self) -> bool {
        self.level.sibling_submenu_open(self.id)
    }

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
/// and icon columns include the gap after them, the sample column the gap before it.
struct Columns {
    mark: f32,
    icon: f32,
    sample: f32,
    trail: f32,
}

/// Where the parts of a row's label line lie.
struct Placed {
    mark: Rect,
    icon: Rect,
    label: Rect,
    sample: Rect,
    trail: Rect,
}

/// The label line of a row's content `area`: all of it but the detail line of height `detail` under it.
fn label_line(area: Rect, detail: f32) -> Rect {
    Rect::from_min_max(area.min, pos2(area.max.x, (area.max.y - detail).max(area.min.y)))
}

/// What a menu row shows as its shortcut.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum MenuShortcut {
    /// The bound command's chord; none for a row with no command.
    #[default]
    Command,
    /// The caller's text, which wins over the command's chord.
    Text(Label),
    /// No shortcut.
    Hidden,
}

/// A row of a menu: a label, an optional icon, a check or radio mark, shortcut text and a detail line under the label.
/// A row bound to a command takes its label, shortcut, enabled and checked state from the command and fires it as a
/// menu row.
///
/// - Running a row (click, Enter or Space, its mnemonic letter) closes the whole chain of menus it sits in and
///   returns focus to where it was before the menus opened (before a menu bar took it, or the Help search field),
///   and only then fires the command: Undo picked from a menu undoes in the text field that held focus.
/// - A submenu row opens a submenu instead: the output's `opens_submenu` asks the caller to open the `Menu` it shows
///   after the row.
/// - [`keep_open`](MenuItem::keep_open) keeps the menus open when the row runs (a check row that toggles).
/// - The mnemonic is the `&` letter of the label, else the first letter no earlier row of the level uses. The
///   underline sits under that letter while Alt is held or the keyboard drives the menu, never on macOS.
/// - The level's active row paints with the shared row fill: the row the pointer moved onto (its `Menu` sets it
///   before the rows run), or the one the keyboard moved to.
/// - A [`tint`](MenuItem::tint) (a user label colour) fills the row behind its content at a strength the theme keeps
///   the text readable on (`MenuTint`); the active, open and disabled looks win over it. The row keeps its text, so the
///   colour is never the only cue.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(content_painter)]
pub struct MenuItem<'a> {
    #[common]
    common: Common,
    command_ref: Option<CommandRef>,
    submenu: bool,
    heading: bool,
    /// An icon before the label.
    #[prop(into)]
    icon: Option<IconSpec<'a>>,
    /// The row shows no icon, not even the one its command names: a text-only menu's row, which is also a line shorter.
    #[prop]
    no_icon: bool,
    /// The row is its label's line plus the menu's padding, not a control's height: a dense classic menu.
    #[style_key]
    #[prop]
    compact: bool,
    /// A check mark, shown while the value is true; the row reserves the mark column either way.
    #[prop]
    check: Option<bool>,
    /// A radio mark, shown while the value is true; the row reserves the mark column either way.
    #[prop]
    radio_mark: Option<bool>,
    /// The letter that runs the row, in place of the level's own pick. A letter the label lacks still runs the row,
    /// with nothing underlined.
    #[prop]
    mnemonic: Option<char>,
    /// A muted second line under the label, in the caption role: the row is a line taller and as wide as the longer
    /// line. It is the row's accessible description unless the row has a tooltip.
    #[prop(into)]
    detail: Option<Label>,
    /// A user label colour behind the row.
    #[style_key]
    #[prop(into)]
    tint: Option<Rgba>,
    /// What the row shows as its shortcut: the bound command's chord, the caller's text, or nothing.
    shortcut: MenuShortcut,
    /// The sample (`sample`) is painted in place of the label, across the label's room and the sample column; the
    /// label stays the row's accessible name and its mnemonic source.
    #[prop(flag)]
    sample_only: bool,
    /// Running the row leaves the menus open.
    #[prop]
    keep_open: bool,
    /// The checked state of the entry the row shows: a radio mark in a value menu, a check mark elsewhere while true.
    #[prop(name = "entry_checked")]
    entry: Option<bool>,
    sample: Option<(crate::list::ThumbAspect, crate::primitives::FacePaint)>,
    ran: bool,
    /// The row took a key activation it does not run: the first key in a menu the pointer opened (`update`).
    withheld: bool,
    opens: bool,
    open: bool,
    active: bool,
    submenu_engaged: bool,
}

impl<'a> MenuItem<'a> {
    /// A row showing `label`, which is also its accessible name.
    pub fn new(label: impl Into<Label>) -> Self {
        MenuItem {
            common: Common::new(),
            command_ref: None,
            submenu: false,
            heading: false,
            icon: None,
            no_icon: false,
            compact: false,
            check: None,
            radio_mark: None,
            mnemonic: None,
            detail: None,
            tint: None,
            shortcut: MenuShortcut::default(),
            sample_only: false,
            keep_open: false,
            entry: None,
            sample: None,
            ran: false,
            withheld: false,
            opens: false,
            open: false,
            active: false,
            submenu_engaged: false,
        }
        .label(label)
    }

    /// A content-painted cell after the label and before the shortcut, in a column shared by the level's rows: as high
    /// as the row's text line and as wide as that height times `aspect`.
    #[must_use]
    pub fn sample(mut self, aspect: crate::list::ThumbAspect, face: crate::primitives::FacePaint) -> Self {
        self.sample = Some((aspect, face));
        self
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

    /// A caption over the rows after it, in the detail's look, its text starting where the row labels start. It is no
    /// row: it senses nothing, takes no focus and no mnemonic, and its AccessKit node is a heading.
    pub fn heading(label: impl Into<Label>) -> Self {
        MenuItem { heading: true, ..MenuItem::new(label) }
    }

    /// The look of the label: a heading's is the detail's.
    fn label_look<'s>(&self, style: &'s MenuItemStyle) -> &'s TextLook {
        if self.heading { &style.detail } else { &style.text }
    }

    /// The icon a bound command names, when it names one with a static name.
    fn command_icon(command: Option<&ResolvedCommand>) -> Option<IconSpec<'static>> {
        match command?.icon.as_ref()? {
            Cow::Borrowed(name) => Some(IconSpec::svg(IconId::new(name))),
            Cow::Owned(_) => None,
        }
    }

    fn lead_icon(&self, command: Option<&ResolvedCommand>) -> Option<IconSpec<'_>> {
        if self.no_icon {
            return None;
        }
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

    /// The shortcut as the `Kbd` that shows it: the caller's text, even on a row whose command has a chord; nothing
    /// when hidden; else the bound command's chord, and nothing for a row with no command or no chord.
    fn keys(&self, command: Option<&ResolvedCommand>) -> Option<Kbd> {
        match &self.shortcut {
            MenuShortcut::Text(text) => Some(Kbd::new(text.clone())),
            MenuShortcut::Hidden => None,
            MenuShortcut::Command => match (&self.command_ref, command) {
                (Some(reference), Some(resolved)) if resolved.chord.is_some() => Some(Kbd::command(reference.clone())),
                _ => None,
            },
        }
    }

    /// The shortcut text the caller set.
    fn shortcut_text(&self) -> Option<&Label> {
        match &self.shortcut {
            MenuShortcut::Text(text) => Some(text),
            MenuShortcut::Command | MenuShortcut::Hidden => None,
        }
    }

    /// Shortcut text at the end of the row, written by the caller; it wins over the bound command's chord.
    #[must_use]
    pub fn shortcut(mut self, text: impl Into<Label>) -> Self {
        self.shortcut = MenuShortcut::Text(text.into());
        self
    }

    /// Shows no shortcut on the row, whatever the bound command's chord is.
    #[must_use]
    pub fn hide_shortcut(mut self) -> Self {
        self.shortcut = MenuShortcut::Hidden;
        self
    }

    /// Whether the sample takes the label's place: a `sample_only` row that has a sample.
    fn sample_replaces_label(&self) -> bool {
        self.sample_only && self.sample.is_some()
    }

    /// The parts of the label line `line`.
    fn place(&self, columns: &Columns, line: Rect, rtl: bool, trail_gap: f32) -> Placed {
        let step = |width: f32, gap: f32| if width > 0.0 { width + gap } else { 0.0 };
        let (mark, flow) = Flow::new(line, rtl).take_start(columns.mark);
        let (icon, flow) = flow.take_start(columns.icon);
        let (trail, flow) = flow.take_end(step(columns.trail, trail_gap));
        let (sample, flow) = flow.take_end(columns.sample);
        Placed { mark, icon, label: flow.rect(), sample, trail }
    }

    fn register(&self, ctx: &Context, level: &MenuLevel, id: Id, label: &str, enabled: bool) {
        level.register(ctx.cumulative_pass_nr(), Row { id, text: label.to_owned(), own: self.mnemonic, enabled });
    }

    /// The row's mark and icon columns with the gap after each: as wide as the column group of the menu body gives
    /// them (the widest of its rows, from this pass's measuring run), or the row's own outside a group.
    fn lead_columns(&self, ctx: &Context, side: f32, gap: f32, command: Option<&ResolvedCommand>, level: &MenuLevel) -> (f32, f32) {
        let own = |on: bool| if on { side + gap } else { 0.0 };
        let shared = |column: &ColumnKey, width: f32| crate::layout::ambient_width(ctx, column, width).unwrap_or(width);
        (shared(&MARKS, own(self.mark(command, level).is_some())), shared(&ICONS, own(self.lead_icon(command).is_some())))
    }

    /// The row's sample column with the gap before it: as wide as the column group of the menu body gives it (the
    /// widest sample of its rows), or the row's own outside a group. A sample is as high as the label line, `line`
    /// high; a level with no sample has no column.
    fn sample_column(&self, ctx: &Context, line: f32, gap: f32) -> f32 {
        let own = self.sample.as_ref().map_or(0.0, |(aspect, _)| gap + aspect.of(line));
        crate::layout::ambient_width(ctx, &SAMPLES, own).unwrap_or(own)
    }

    /// The row's columns, for a label line `line` high.
    fn columns(&self, cx: &MeasureCx<'_, Self>, level: &MenuLevel, line: f32) -> Columns {
        let side = cx.icon();
        let command = cx.command();
        let (mark, icon) = self.lead_columns(cx.ui.ctx(), side, cx.gap(), command, level);
        let sample = self.sample_column(cx.ui.ctx(), line, cx.gap());
        let trail = if self.submenu { side } else { self.keys(command).map_or(0.0, |keys| cx.face(keys).x) };
        Columns { mark, icon, sample, trail }
    }

    /// The size of the detail line; zero without one.
    fn detail_size(&self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        self.detail.as_ref().map_or(Vec2::ZERO, |detail| {
            let detail = cx.env.resolve(detail);
            cx.layout_at(&TextRun::new(detail.as_str()).role(cx.style.detail.role), f32::INFINITY).size()
        })
    }
}

impl Component for MenuItem<'_> {
    const ROLE: Role = Role::MENU_ITEM;
    const ALT_ROLES: &'static [Role] = &[Role::MENU_ITEM_CHECK_BOX, Role::MENU_ITEM_RADIO, Role::HEADING];
    const OVERLAYS: bool = false;
    const KEYS: crate::runtime::focus::KeyUse = crate::runtime::focus::KeyUse::of(crate::contract::Input::Pointer).with_moves();
    type Style = MenuItemStyle;
    type Output = MenuItemOutput;

    fn style(cx: &StyleCx<'_>, key: MenuItemStyleKey) -> MenuItemStyle {
        let tokens = cx.tokens;
        let text = &tokens.color.text;
        let c = |rgba: Rgba| cx.color(rgba);
        // A label colour over the popup card, with the text inks moved until they read on it.
        let tint = key.tint.map(|color| MenuTint::derive(color, tokens.color.surface.panel, &tokens.color, cx.contrast() == Contrast::High));
        let (normal, muted) = tint.map_or((text.normal, text.muted), |tint| (tint.text, tint.shortcut));
        // A checked row shows its mark, not a selection fill; a disabled row drops the tint.
        let clear = c(Rgba::TRANSPARENT);
        let rest = tint.map_or(clear, |tint| c(tint.fill));
        let mut surface = cx.command_row_look();
        surface.fill = surface.fill.off(States::new(rest, rest, rest, clear)).on(States::new(rest, rest, rest, clear));
        // A checked row keeps its base ink: its mark shows the check.
        let ink = |base: Rgba| cx.command_row_ink(base).on(States::new(c(base), c(base), c(base), c(text.disabled)));
        let control = cx.control();
        // A row's content sits `space.pad.regular` in from the popup's own padding, so a label starts clear of the edge.
        let mut metrics = control.metrics();
        metrics.padding = Insets::new(tokens.space.pad.regular, tokens.space.pad.regular, metrics.padding.top, metrics.padding.bottom);
        // The text line sets a compact row's height, as in a classic menu; any other row stands a menu row high.
        metrics.min_size.y = if key.compact { 0.0 } else { cx.menu_row_height() };
        MenuItemStyle {
            surface,
            metrics,
            text: TextLook::new(TypeRole::Body, ink(normal)),
            icon: IconLook::new(control.icon, ink(normal)),
            detail: TextLook::new(TypeRole::Caption, ink(muted)),
            shortcut_gap: tokens.space.gap.section,
            submenu_gap: tokens.space.gap.tight,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let level = cx.env.get::<MenuLevel>();
        // The measuring run of the menu body registers the rows, so the real run's letters come from this pass's rows.
        if !self.heading {
            self.register(cx.ui.ctx(), &level, cx.id, cx.resolved.label, cx.resolved.enabled);
        }
        let detail = self.detail_size(cx);
        let text = mnemonic::marked(cx.resolved.label, &cx.env.folder()).0;
        let run = TextRun::new(text.as_str()).role(self.label_look(cx.style).role);
        let label = if text.is_empty() { Vec2::ZERO } else { cx.layout_at(&run, f32::INFINITY).size() };
        let metrics = &cx.style.metrics;
        let padding = metrics.padding;
        // The label line paint finds (`label_line`): the row is at least its least height, and the sample is as high
        // as that line.
        let line = label.y.max(cx.icon()).max(metrics.min_size.y - padding.top - padding.bottom - detail.y);
        let columns = self.columns(cx, &level, line);
        let gap = cx.gap();
        let step = |width: f32, gap: f32| if width > 0.0 { width + gap } else { 0.0 };
        // A sample-only row's sample stands in the label's place: the label takes no width.
        let shown = if self.sample_replaces_label() { 0.0 } else { label.x };
        let rest = shown.max(detail.x) + step(columns.trail, cx.style.shortcut_gap.max(gap));
        // A row measured before the level's widest mark, icon or sample read narrower columns: the body's group adds
        // the widest columns to what the rows reach past them.
        crate::layout::report_beyond(cx.ui.ctx(), &[MARKS, ICONS, SAMPLES], padding.start + padding.end + rest);
        let width = columns.mark + columns.icon + columns.sample + rest;
        Vec2::new(cx.room.map_or(width, |room| width.min(room)), line + detail.y)
    }

    fn fills(&self) -> Option<craft_behavior::Axis> {
        Some(craft_behavior::Axis::X)
    }

    fn sense(&self) -> Sense {
        if self.heading { Sense::empty() } else { Sense::CLICK | Sense::FOCUS }
    }

    /// The shortcut's slot, which paints as the face of a `Kbd`.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let level = cx.env.get::<MenuLevel>();
        let line = label_line(cx.content_rect, self.detail_size(cx).y);
        let columns = self.columns(cx, &level, line.height());
        if columns.trail == 0.0 || self.submenu || self.heading {
            return;
        }
        let rtl = cx.flow.is_rtl();
        let placed = self.place(&columns, line, rtl, cx.style.shortcut_gap.max(cx.gap()));
        let (rect, _) = Flow::new(placed.trail, rtl).take_end(columns.trail);
        cx.part(SHORTCUT, rect);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if self.heading {
            return;
        }
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
        let input = RowInput::of(cx, &level);
        let (moving, hovered, focused) = (input.moving, input.hovered, input.focused);
        // The pointer's last move headed for the open submenu of another row: this row is one it crosses on the way.
        let heading = level.aimed(cx.id);
        let rested = {
            let rest = cx.memory::<Rest>();
            // The rest starts at the pointer's last move over the row.
            if hovered && moving {
                rest.since = Some(now);
                0.0
            } else if hovered {
                now - *rest.since.get_or_insert(now)
            } else {
                rest.since = None;
                0.0
            }
        };
        let takes_over = hovered && (!heading || rested >= AIM_REST);
        // The first key in a menu the pointer opened, on a row the pointer did not light, is answered by `RowInput::of`.
        let first_key = input.withheld;
        // The focused row of a menu that the pointer opened is not active: the keyboard makes it so when focus lands on
        // it or the keyboard starts driving the menu.
        input.activate(&ctx, cx.id, &level, enabled);
        // A disabled row is never active.
        self.active = enabled && level.is_active(cx.id);
        // A crossed row waits for the rest to end: one pass when it does.
        if hovered && heading && rested < AIM_REST {
            frame::request_repaint_after(&ctx, AIM_REST - rested);
        }
        let typed = enabled && level.take_run(cx.id);
        self.withheld = first_key;
        let asked = enabled && (typed || (cx.activation().is_some() && !first_key));
        if self.submenu {
            self.opens = asked || (enabled && forward) || (enabled && takes_over);
            if self.opens {
                let replaces_sibling = level.sibling_submenu_open(cx.id);
                if level.open_submenu(cx.id) {
                    // An earlier row may already have drawn the old submenu in this pass.
                    if replaces_sibling && frame::discardable(&ctx) {
                        frame::discard_pass(&ctx, "menu: a sibling replaced the open submenu");
                    }
                    frame::request_repaint(&ctx);
                }
            }
            self.open = level.submenu_open(cx.id);
            if self.open && level.descended(cx.id, focused) {
                // The active row is in the submenu. Keep the accent on its parent so the active path stays visible.
                self.active = false;
                self.submenu_engaged = enabled;
            }
            return;
        }
        if takes_over && level.clear_submenu() {
            frame::request_repaint(&ctx);
        }
        if asked && !self.keep_open {
            close_on_pick(&ctx);
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

    /// A submenu row never runs: it opens its submenu, and the menus stay open. Nor does a row whose key activation it
    /// withheld: the row the pointer lit runs instead, or none.
    fn activated(&self, cx: &Cx<'_, Self>) -> bool {
        !self.submenu && !self.withheld && cx.activation().is_some()
    }

    fn flags(&self) -> Flags {
        let mut flags = Flags::empty();
        flags.set(Flags::CHECKED, self.check == Some(true) || self.radio_mark == Some(true) || self.entry == Some(true));
        let highlighted = self.active || self.submenu_engaged;
        flags.set(Flags::LEAD, highlighted);
        // If aim holds an open submenu while a sibling takes the highlight, keep the parent's quieter tint.
        flags.set(Flags::EMPHASIZED, self.open && !highlighted);
        flags
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (rtl, side, gap) = (cx.flow.is_rtl(), cx.style.icon.side, cx.style.metrics.gap);
        let level = cx.env.get::<MenuLevel>();
        let command = cx.command.as_deref();
        let (mark_column, icon_column) = self.lead_columns(&cx.core.ctx, side, gap, command, &level);
        let text = mnemonic::marked(cx.resolved.label, &cx.env.folder()).0;
        let mark = self.mark(command, &level);
        let draw = ui.draw();
        let detail = self.detail.as_ref().map(|detail| cx.env.resolve(detail));
        let detail_run =
            detail.as_ref().map(|detail| TextRun::new(detail.as_str()).role(cx.style.detail.role).color(cx.resolve(&cx.style.detail.color)));
        let detail_height = detail_run.as_ref().map_or(0.0, |run| draw.layout(run, f32::INFINITY).size().y);
        let trail = if self.submenu { side } else { cx.part_if_declared(SHORTCUT).map_or(0.0, |part| part.rect.width()) };
        let line = label_line(cx.content_rect, detail_height);
        let sample_column = self.sample_column(&cx.core.ctx, line.height(), gap);
        let columns = Columns { mark: mark_column, icon: icon_column, sample: sample_column, trail };
        let placed = self.place(&columns, line, rtl, cx.style.shortcut_gap.max(gap));
        let ink = cx.resolve(&cx.style.icon.color);
        let centred = |slot: Rect| Rect::from_center_size(slot.center(), Vec2::splat(side));
        if columns.mark > 0.0 {
            let square = Rect::from_center_size(Flow::new(placed.mark, rtl).take_start(side).0.center(), Vec2::splat(side));
            match mark {
                Some(Mark::Radio(true)) => draw.fill(&glyph::dot(square.center(), side), ink),
                Some(Mark::Radio(false)) => draw.stroke(&glyph::ring(square.center(), side), StrokeWidth::Thin, ink),
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
        let mnemonic = if level.keyboard() || cx.modifiers().alt { level.mnemonic_of(cx.id).and_then(|(_, at)| at) } else { None };
        let look = self.label_look(cx.style);
        let run = TextRun::mnemonic(text.as_str(), mnemonic).elide().role(look.role).color(cx.resolve(&look.color));
        // A sample-only row draws no label: the label stays its accessible name and its mnemonic source.
        if !self.sample_replaces_label() {
            let layout = draw.layout(&run, placed.label.width());
            let size = layout.size();
            let x = if rtl { placed.label.max.x - size.x } else { placed.label.min.x };
            let y = placed.label.center().y - size.y / 2.0;
            draw.galley(pos2(x, y), &layout);
        }
        // The detail starts where the label starts, under the label line.
        if let Some(run) = &detail_run {
            let layout = draw.layout(&run.clone().elide(), placed.label.width());
            let size = layout.size();
            let x = if rtl { placed.label.max.x - size.x } else { placed.label.min.x };
            draw.galley(pos2(x, line.max.y), &layout);
        }
        // Samples start at one edge of the shared column, after its gap, so a level's samples line up; a sample-only
        // row's sample takes the label's room and the column.
        if let Some((aspect, face)) = &self.sample {
            let rect =
                if self.sample_only { placed.label.union(placed.sample) } else { Flow::new(placed.sample, rtl).span(gap, aspect.of(line.height())) };
            face.paint(&ui.content_painter(cx), rect);
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
        access.name(Name::Text(mnemonic::marked(cx.resolved.label, &cx.env.folder()).0));
        if self.heading {
            access.role(Role::HEADING);
            return;
        }
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
        if let Some(text) = self.shortcut_text() {
            access.shortcut(cx.env.resolve(text).as_str().to_owned());
        } else if self.shortcut == MenuShortcut::Hidden {
            access.no_shortcut();
        }
        if cx.resolved.tooltip.is_none()
            && let Some(detail) = &self.detail
        {
            access.description(cx.env.resolve(detail).as_str().to_owned());
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> MenuItemOutput {
        let level = cx.env.get::<MenuLevel>();
        // The frame is where a submenu hangs: the row with the popup padding, widened by `gap.tight` on both sides so a
        // submenu that flips to the other side keeps the gap.
        let frame = cx.response.rect.expand(level.pad()).expand2(egui::vec2(cx.style.submenu_gap, 0.0));
        MenuItemOutput {
            response: cx.response.clone(),
            ran: self.ran,
            opens_submenu: self.opens,
            submenu_open: self.open,
            active: self.active,
            frame,
            level,
            id: cx.id,
        }
    }
}

/// Since when the pointer has rested on a row (its last move over the row), and whether the row had the focus in the
/// last pass.
#[derive(Clone, Default)]
struct Rest {
    since: Option<f64>,
    focused: bool,
    /// Whether the keyboard drove the menu in the last pass.
    keyboard: bool,
}
