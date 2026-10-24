//! The on-screen keyboard pane (§2.9 of the dialog plan, `Options::keyboard`): a US ANSI layout
//! colored by who holds each key with the chosen modifiers, modifier toggles that also follow the
//! held modifiers, the shortcuts of the clicked key, and Assign to Selected.

use std::collections::HashMap;
use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    Align, Button, Color32, CornerRadius, FontId, Layout, Rect, RichText, Role, ScrollArea, Sense, Stroke, StrokeKind, Ui, WidgetInfo, pos2, vec2,
};

use super::{Draw, fill};
use crate::chord::{Chord, Key, Mods, Trigger};
use crate::command::CommandRef;
use crate::keymap::{Binding, Holder, Keymap, Overrides};
use crate::scope::Scope;

/// The keyboard's width and height in key units: 15 for the main block, a gap, 3 for the
/// navigation cluster; six rows and the gap under the function row.
const UNITS_W: f32 = 18.5;
const UNITS_H: f32 = 6.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mod {
    Cmd,
    Ctrl,
    Alt,
    Shift,
}

impl Mod {
    fn id(self) -> &'static str {
        match self {
            Mod::Cmd => "cmd",
            Mod::Ctrl => "ctrl",
            Mod::Alt => "alt",
            Mod::Shift => "shift",
        }
    }

    fn get(self, m: Mods) -> bool {
        match self {
            Mod::Cmd => m.cmd,
            Mod::Ctrl => m.ctrl,
            Mod::Alt => m.alt,
            Mod::Shift => m.shift,
        }
    }

    fn flip(self, m: &mut Mods) {
        match self {
            Mod::Cmd => m.cmd = !m.cmd,
            Mod::Ctrl => m.ctrl = !m.ctrl,
            Mod::Alt => m.alt = !m.alt,
            Mod::Shift => m.shift = !m.shift,
        }
    }
}

/// A keycap: a key, a modifier, a key the dialog does not bind (Caps Lock, Win), or a gap.
#[derive(Clone, Copy)]
enum Cap {
    Key(Key),
    Mod(Mod),
    CapsLock,
    /// Win on Windows, Super on Linux; a Mac has none.
    System,
    Gap,
}

type Row = &'static [(Cap, f32)];

const fn k(key: Key) -> (Cap, f32) {
    (Cap::Key(key), 1.0)
}

const MAIN_ROWS: [Row; 5] = [
    &[
        (Cap::Key(Key::Backtick), 1.0),
        k(Key::Num1),
        k(Key::Num2),
        k(Key::Num3),
        k(Key::Num4),
        k(Key::Num5),
        k(Key::Num6),
        k(Key::Num7),
        k(Key::Num8),
        k(Key::Num9),
        k(Key::Num0),
        k(Key::Minus),
        k(Key::Equals),
        (Cap::Key(Key::Backspace), 2.0),
    ],
    &[
        (Cap::Key(Key::Tab), 1.5),
        k(Key::Q),
        k(Key::W),
        k(Key::E),
        k(Key::R),
        k(Key::T),
        k(Key::Y),
        k(Key::U),
        k(Key::I),
        k(Key::O),
        k(Key::P),
        k(Key::OpenBracket),
        k(Key::CloseBracket),
        (Cap::Key(Key::Backslash), 1.5),
    ],
    &[
        (Cap::CapsLock, 1.75),
        k(Key::A),
        k(Key::S),
        k(Key::D),
        k(Key::F),
        k(Key::G),
        k(Key::H),
        k(Key::J),
        k(Key::K),
        k(Key::L),
        k(Key::Semicolon),
        k(Key::Quote),
        (Cap::Key(Key::Enter), 2.25),
    ],
    &[
        (Cap::Mod(Mod::Shift), 2.25),
        k(Key::Z),
        k(Key::X),
        k(Key::C),
        k(Key::V),
        k(Key::B),
        k(Key::N),
        k(Key::M),
        k(Key::Comma),
        k(Key::Period),
        k(Key::Slash),
        (Cap::Mod(Mod::Shift), 2.75),
    ],
    &[],
];

const FUNCTION_ROW: Row = &[
    k(Key::Escape),
    (Cap::Gap, 1.0),
    k(Key::F1),
    k(Key::F2),
    k(Key::F3),
    k(Key::F4),
    (Cap::Gap, 0.5),
    k(Key::F5),
    k(Key::F6),
    k(Key::F7),
    k(Key::F8),
    (Cap::Gap, 0.5),
    k(Key::F9),
    k(Key::F10),
    k(Key::F11),
    k(Key::F12),
];

const MAC_BOTTOM: Row = &[
    (Cap::Mod(Mod::Ctrl), 1.5),
    (Cap::Mod(Mod::Alt), 1.25),
    (Cap::Mod(Mod::Cmd), 1.5),
    (Cap::Key(Key::Space), 6.5),
    (Cap::Mod(Mod::Cmd), 1.5),
    (Cap::Mod(Mod::Alt), 1.25),
    (Cap::Mod(Mod::Ctrl), 1.5),
];

/// Off macOS the Ctrl keys are the command modifier.
const PC_BOTTOM: Row = &[
    (Cap::Mod(Mod::Cmd), 1.5),
    (Cap::System, 1.25),
    (Cap::Mod(Mod::Alt), 1.5),
    (Cap::Key(Key::Space), 6.5),
    (Cap::Mod(Mod::Alt), 1.5),
    (Cap::System, 1.25),
    (Cap::Mod(Mod::Cmd), 1.5),
];

/// The navigation cluster: (row, column in units, key). Row 0 is the function row.
const NAV: &[(usize, f32, Key)] = &[
    (1, 0.0, Key::Insert),
    (1, 1.0, Key::Home),
    (1, 2.0, Key::PageUp),
    (2, 0.0, Key::Delete),
    (2, 1.0, Key::End),
    (2, 2.0, Key::PageDown),
    (4, 1.0, Key::Up),
    (5, 0.0, Key::Left),
    (5, 1.0, Key::Down),
    (5, 2.0, Key::Right),
];

/// A binding on a key and the scope it works in.
#[derive(Clone, Debug)]
pub(crate) struct KeyHolder {
    pub holder: Holder,
    pub scope: Scope,
}

/// Who holds a key with given modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Free,
    App,
    Scoped,
    Both,
}

fn class(holders: &[KeyHolder]) -> Class {
    match (holders.iter().any(|h| h.scope.is_app()), holders.iter().any(|h| !h.scope.is_app())) {
        (false, false) => Class::Free,
        (true, false) => Class::App,
        (false, true) => Class::Scoped,
        (true, true) => Class::Both,
    }
}

/// The pane's state between frames.
#[derive(Clone, Debug, Default)]
pub(crate) struct KeyboardState {
    /// Hidden with the header's Keyboard toggle.
    pub hidden: bool,
    /// The modifiers toggled on; the held ones are added each frame.
    pub mods: Mods,
    /// The key whose shortcuts are listed.
    pub key: Option<Key>,
    /// Every binding by effective chord, rebuilt when the overrides change.
    index: Option<Arc<(Overrides, Index)>>,
}

type Index = HashMap<Chord, Vec<KeyHolder>>;

impl KeyboardState {
    fn index(&mut self, keymap: &Keymap) -> Arc<(Overrides, Index)> {
        if let Some(i) = self.index.as_ref().filter(|i| i.0 == *keymap.overrides()) {
            return i.clone();
        }
        let platform = keymap.platform();
        let mut map: HashMap<Chord, Vec<KeyHolder>> = HashMap::new();
        for b in keymap.catalog() {
            for binding in keymap.bindings(&b.command) {
                let scope = binding.scope_or(&b.scope).clone();
                let holder = Holder { command: b.command.clone(), binding };
                map.entry(holder.binding.trigger.chord.effective(platform)).or_default().push(KeyHolder { holder, scope });
            }
        }
        let index = Arc::new((keymap.overrides().clone(), map));
        self.index = Some(index.clone());
        index
    }
}

fn mod_glyph(m: Mod) -> &'static str {
    match m {
        Mod::Cmd => "⌘",
        Mod::Ctrl => "⌃",
        Mod::Alt => "⌥",
        Mod::Shift => "⇧",
    }
}

/// The English legend printed on a key: Apple's words on macOS, PC words elsewhere.
fn cap_name(key: Key, mac: bool) -> &'static str {
    match (key, mac) {
        (Key::Escape, true) => "esc",
        (Key::Escape, false) => "Esc",
        (Key::Backspace, true) => "delete",
        (Key::Delete, true) => "fwd del",
        (Key::Insert, true) => "ins",
        (Key::Enter, true) => "return",
        (Key::Tab, true) => "tab",
        (Key::PageUp, true) => "page up",
        (Key::PageDown, true) => "page down",
        (Key::Home, true) => "home",
        (Key::End, true) => "end",
        (Key::Insert, false) => "Ins",
        (Key::PageUp, false) => "PgUp",
        (Key::PageDown, false) => "PgDn",
        (Key::Delete, false) => "Del",
        (Key::Space, true) => "",
        (Key::Up | Key::Down | Key::Left | Key::Right, _) => key.mac_symbol(),
        (k, _) => k.text_name(),
    }
}

fn union(a: Mods, b: Mods) -> Mods {
    Mods { cmd: a.cmd || b.cmd, ctrl: a.ctrl || b.ctrl, alt: a.alt || b.alt, shift: a.shift || b.shift }
}

/// Black or white, whichever reads better on `fill`.
fn ink(fill: Color32) -> Color32 {
    let l = 0.299 * f32::from(fill.r()) + 0.587 * f32::from(fill.g()) + 0.114 * f32::from(fill.b());
    if l > 150.0 { Color32::from_gray(24) } else { Color32::from_gray(240) }
}

/// What a frame of the pane asked for.
#[derive(Default)]
struct Clicks {
    key: Option<Key>,
    toggle: Option<Mod>,
    go: Option<CommandRef>,
    assign: bool,
}

impl Draw<'_, '_> {
    /// The pane's height for a middle area of `height`, or `None` when it is off or does not fit.
    pub(super) fn keyboard_height(&self, height: f32) -> Option<f32> {
        if !self.env.options.keyboard || self.state.keyboard.hidden {
            return None;
        }
        let h = (height * 0.45).min(320.0).min(height - 160.0);
        (h >= 120.0).then_some(h)
    }

    /// The header's toggle that shows and hides the pane.
    pub(super) fn keyboard_toggle(&mut self, ui: &mut Ui) {
        if !self.env.options.keyboard {
            return;
        }
        let label = self.ui("Keyboard");
        let r = ui.add(Button::selectable(!self.state.keyboard.hidden, self.vis(&label)));
        self.tag(|| "shortcuts.keyboard".into(), r.rect, &label);
        if r.clicked() {
            self.state.keyboard.hidden = !self.state.keyboard.hidden;
        }
    }

    fn mods_shown(&self) -> &'static [Mod] {
        if self.env.keymap.platform().is_mac() { &[Mod::Cmd, Mod::Ctrl, Mod::Alt, Mod::Shift] } else { &[Mod::Cmd, Mod::Alt, Mod::Shift] }
    }

    /// A modifier's name, with its glyph on macOS.
    fn mod_name(&self, m: Mod) -> String {
        let word = self.mod_word(m);
        if self.env.keymap.platform().is_mac() { format!("{} {word}", mod_glyph(m)) } else { word }
    }

    /// The key's name on its cap: the app's localized name, else the glyph on macOS.
    fn cap_legend(&self, key: Key) -> String {
        let p = self.env.keymap.platform();
        self.env.text.key(key, p).unwrap_or_else(|| cap_name(key, p.is_mac()).to_string())
    }

    /// A modifier cap's word, without the glyph (which goes above it on macOS).
    fn mod_word(&self, m: Mod) -> String {
        match (m, self.env.keymap.platform().is_mac()) {
            (Mod::Cmd, true) => self.ui("Command"),
            (Mod::Ctrl, true) => self.ui("Control"),
            (Mod::Alt, true) => self.ui("Option"),
            (Mod::Cmd | Mod::Ctrl, false) => self.ui("Ctrl"),
            (Mod::Alt, false) => self.ui("Alt"),
            (Mod::Shift, _) => self.ui("Shift"),
        }
    }

    /// The key's spoken name: the app's localized name, else the name Windows and Linux show.
    fn key_name(&self, key: Key) -> String {
        self.env.text.key(key, self.env.keymap.platform()).unwrap_or_else(|| key.text_name().to_string())
    }

    /// The holder's label, with its scope when that is not Application.
    fn holder_text(&self, h: &KeyHolder) -> String {
        let label = self.label_of(&h.holder.command);
        if h.scope.is_app() { label } else { fill(self.ui("{label} ({scope})"), &[("label", &label), ("scope", &self.scope_name(&h.scope))]) }
    }

    pub(super) fn keyboard_pane(&mut self, ui: &mut Ui) {
        let platform = self.env.keymap.platform();
        let held = if self.state.recording.is_none() { ui.input(|i| Mods::from_egui(&i.modifiers, platform)) } else { Mods::NONE };
        let mut clicks = Clicks::default();
        let rect = ui.max_rect();
        ui.painter().hline(rect.x_range(), rect.min.y, ui.visuals().widgets.noninteractive.bg_stroke);
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            let layout = if platform.is_mac() { self.ui("Keyboard: US (macOS)") } else { self.ui("Keyboard: US") };
            ui.label(RichText::new(self.vis(&layout)).color(self.style.dim));
            ui.add_space(8.0);
            for &m in self.mods_shown() {
                let name = self.mod_name(m);
                let on = m.get(union(self.state.keyboard.mods, held));
                let r = ui.add(Button::selectable(on, self.vis(&name)));
                self.tag(|| format!("shortcuts.mod.{}", m.id()), r.rect, &name);
                if r.clicked() {
                    clicks.toggle = Some(m);
                }
            }
            ui.add_space(12.0);
            self.keyboard_legend(ui);
        });
        let mods = union(self.state.keyboard.mods, held);
        let body = ui.available_rect_before_wrap();
        let gap = 12.0;
        let list_w = (body.width() * 0.3).clamp(160.0, 320.0).min(body.width() / 2.0);
        let rtl = self.env.text.rtl();
        let (kb, list) = if rtl {
            (
                Rect::from_min_max(pos2(body.min.x + list_w + gap, body.min.y), body.max),
                Rect::from_min_max(body.min, pos2(body.min.x + list_w, body.max.y)),
            )
        } else {
            (
                Rect::from_min_max(body.min, pos2(body.max.x - list_w - gap, body.max.y)),
                Rect::from_min_max(pos2(body.max.x - list_w, body.min.y), body.max),
            )
        };
        self.keys_drawn(ui, kb, mods, &mut clicks);
        self.child(ui, list, |d, ui| d.key_list(ui, mods, &mut clicks));
        ui.advance_cursor_after_rect(body);

        let kb_state = &mut self.state.keyboard;
        if let Some(m) = clicks.toggle {
            // A held modifier is on anyway; clicking it turns the toggle off.
            if m.get(held) {
                let mut off = kb_state.mods;
                if m.get(off) {
                    m.flip(&mut off);
                }
                kb_state.mods = off;
            } else {
                m.flip(&mut kb_state.mods);
            }
        }
        if let Some(key) = clicks.key {
            kb_state.key = Some(key);
        }
        if let Some(c) = clicks.go {
            self.state.go_to(&c);
        }
        if clicks.assign
            && let (Some(command), Some(key)) = (self.state.selected.clone(), self.state.keyboard.key)
        {
            let trigger = Trigger::single(Chord { mods, key });
            let binding = match &self.state.context {
                Some(scope) => Binding::in_scope(trigger, scope.clone()),
                None => Binding::new(trigger),
            };
            self.state.stop_recording();
            self.state.commit(self.env, &command, None, binding);
        }
    }

    fn keyboard_legend(&self, ui: &mut Ui) {
        let s = &self.style;
        for (color, text) in [
            (s.app_key, self.ui("Application")),
            (s.scoped_key, self.ui("Panel or context")),
            (s.both_key, self.ui("Both")),
            (s.modifier_key, self.ui("Modifier on")),
        ] {
            let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
            ui.painter().rect_filled(r, 2.0, color);
            ui.label(RichText::new(self.vis(&text)).color(self.style.dim));
        }
    }

    /// The keycaps in `area`, scaled to fit.
    fn keys_drawn(&mut self, ui: &mut Ui, area: Rect, mods: Mods, clicks: &mut Clicks) {
        let mac = self.env.keymap.platform().is_mac();
        let u = (area.width() / UNITS_W).min(area.height() / UNITS_H).max(1.0);
        let origin = area.min;
        let row_y = |row: usize| origin.y + row as f32 * u + if row > 0 { 0.25 * u } else { 0.0 };
        let index = self.state.keyboard.index(self.env.keymap);
        let index = &index.1;
        let bottom = if mac { MAC_BOTTOM } else { PC_BOTTOM };
        let rows = std::iter::once(FUNCTION_ROW).chain(MAIN_ROWS[..4].iter().copied()).chain(std::iter::once(bottom));
        for (r, row) in rows.enumerate() {
            let mut x = origin.x;
            for &(cap, w) in row {
                let rect = Rect::from_min_size(pos2(x, row_y(r)), vec2(w * u, u));
                x += w * u;
                self.cap(ui, rect, cap, u, mods, index, clicks);
            }
        }
        let nav_x = origin.x + 15.5 * u;
        for &(r, col, key) in NAV {
            let rect = Rect::from_min_size(pos2(nav_x + col * u, row_y(r)), vec2(u, u));
            self.cap(ui, rect, Cap::Key(key), u, mods, index, clicks);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn cap(&mut self, ui: &mut Ui, rect: Rect, cap: Cap, u: f32, mods: Mods, index: &Index, clicks: &mut Clicks) {
        if matches!(cap, Cap::Gap) {
            return;
        }
        let r = rect.shrink((u * 0.05).clamp(1.0, 2.0));
        let font = FontId::proportional((u * 0.26).clamp(7.0, 12.0));
        let s = self.style;
        let (bg, legend, title) = match cap {
            Cap::Key(key) => {
                let holders = index.get(&Chord { mods, key }.effective(self.env.keymap.platform())).map_or(&[][..], Vec::as_slice);
                let bg = match class(holders) {
                    Class::Free => s.free_key,
                    Class::App => s.app_key,
                    Class::Scoped => s.scoped_key,
                    Class::Both => s.both_key,
                };
                // The application-wide holder names the key; a scoped one when there is none.
                let first = holders.iter().find(|h| h.scope.is_app()).or(holders.first());
                (bg, self.cap_legend(key), first.map(|h| self.label_of(&h.holder.command)))
            }
            Cap::Mod(m) => {
                let glyph = self.env.keymap.platform().is_mac().then(|| mod_glyph(m).to_string());
                (if m.get(mods) { s.modifier_key } else { s.free_key }, self.mod_word(m), glyph)
            }
            Cap::CapsLock => (s.free_key.gamma_multiply(0.7), self.ui("Caps Lock"), None),
            Cap::System => {
                let name = if self.env.keymap.platform() == crate::chord::Platform::Windows { self.ui("Win") } else { self.ui("Super") };
                (s.free_key.gamma_multiply(0.7), name, None)
            }
            Cap::Gap => return,
        };
        let ink = ink(bg);
        ui.painter().rect_filled(r, CornerRadius::same(3), bg);
        let pad = (u * 0.1).clamp(2.0, 4.0);
        let text_w = (r.width() - 2.0 * pad).max(1.0);
        let lines = |text: &str, rows: usize, color: Color32| {
            let mut job = LayoutJob::single_section(text.to_string(), TextFormat::simple(font.clone(), color));
            job.wrap = TextWrapping { max_width: text_w, max_rows: rows, break_anywhere: false, overflow_character: Some('…') };
            ui.painter().layout_job(job)
        };
        if let Some(t) = &title {
            let rows = if r.height() > 4.0 * font.size { 2 } else { 1 };
            let g = lines(&self.vis(t), rows, ink);
            ui.painter().galley(r.min + vec2(pad, pad * 0.5), g, ink);
        }
        // A cap without a command name has room for a two-line legend ("page up").
        let legend_rows = if title.is_none() && r.height() > 3.0 * font.size { 2 } else { 1 };
        let g = lines(&self.vis(&legend), legend_rows, ink.gamma_multiply(0.85));
        ui.painter().galley(pos2(r.min.x + pad, r.max.y - pad * 0.5 - g.size().y), g, ink);

        let Cap::Key(key) = cap else {
            if let Cap::Mod(m) = cap {
                // Modifier caps toggle like the buttons above; the buttons are their accessible form.
                let resp = ui.interact(r, self.base.with(("kbmod", m.id(), r.min.x as i32)), Sense::CLICK);
                if resp.clicked() {
                    clicks.toggle = Some(m);
                }
                if resp.hovered() {
                    ui.painter().rect_stroke(r, CornerRadius::same(3), Stroke::new(1.5, ink), StrokeKind::Inside);
                }
            }
            return;
        };
        let holders = index.get(&Chord { mods, key }.effective(self.env.keymap.platform())).map_or(&[][..], Vec::as_slice);
        let chord = self.chord(&Trigger::single(Chord { mods, key }));
        let spoken = if mods.is_none() { self.key_name(key) } else { chord };
        let mut names: Vec<String> = Vec::new();
        for h in holders {
            let t = self.holder_text(h);
            if !names.contains(&t) {
                names.push(t);
            }
        }
        let a11y = if names.is_empty() { spoken } else { fill(self.ui("{chord}: {label}"), &[("chord", &spoken), ("label", &names.join(", "))]) };
        let selected = self.state.keyboard.key == Some(key);
        let resp = ui.interact(r, self.base.with(("key", key.name())), Sense::click());
        resp.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, &a11y));
        let description = match class(holders) {
            Class::Free => self.ui("Not assigned"),
            Class::App => self.ui("Application"),
            Class::Scoped => self.ui("Panel or context"),
            Class::Both => self.ui("Both"),
        };
        ui.ctx().accesskit_node_builder(resp.id, |node| node.set_description(description));
        self.tag(|| format!("shortcuts.key.{}", key.name()), r, &a11y);
        let v = ui.visuals();
        if selected {
            ui.painter().rect_stroke(r, CornerRadius::same(3), Stroke::new(2.0, v.strong_text_color()), StrokeKind::Outside);
        } else if resp.has_focus() {
            ui.painter().rect_stroke(r, CornerRadius::same(3), v.selection.stroke, StrokeKind::Outside);
        } else if resp.hovered() {
            ui.painter().rect_stroke(r, CornerRadius::same(3), Stroke::new(1.5, ink), StrokeKind::Inside);
        }
        if resp.clicked() {
            clicks.key = Some(key);
        }
        if !names.is_empty() {
            resp.on_hover_text(self.vis(&names.join("\n")));
        }
    }

    /// The clicked key's shortcuts with any modifiers, and Assign to Selected.
    fn key_list(&mut self, ui: &mut Ui, mods: Mods, clicks: &mut Clicks) {
        let Some(key) = self.state.keyboard.key else {
            ui.label(RichText::new(self.vis(&self.ui("Click a key to see its shortcuts"))).color(self.style.dim));
            return;
        };
        let platform = self.env.keymap.platform();
        let title = fill(self.ui("Key: {key}"), &[("key", &self.key_name(key))]);
        ui.label(RichText::new(self.vis(&title)).strong());
        if self.env.options.editable {
            let chord = self.chord(&Trigger::single(Chord { mods, key }));
            let selected = self.state.selected.clone().filter(|c| self.env.keymap.bindable(c).is_some());
            let label = self.ui("Assign to Selected");
            let r = ui.add_enabled(selected.is_some(), Button::new(self.vis(&label)));
            self.tag(|| "shortcuts.assignKey".into(), r.rect, &label);
            let r = match &selected {
                Some(c) => r.on_hover_text(
                    self.vis_chord(&fill(self.ui("Assign {chord} to {label}"), &[("chord", &chord), ("label", &self.label_of(c))]), &chord),
                ),
                None => r.on_disabled_hover_text(self.vis(&self.ui("Select a command first"))),
            };
            if r.clicked() {
                clicks.assign = true;
            }
        }
        let index = self.state.keyboard.index(self.env.keymap);
        let mut entries: Vec<(Chord, KeyHolder)> =
            index.1.iter().filter(|(c, _)| c.key == key).flat_map(|(c, hs)| hs.iter().map(|h| (*c, h.clone()))).collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.holder.command.id.cmp(&b.1.holder.command.id)));
        if entries.is_empty() {
            ui.label(RichText::new(self.vis(&self.ui("Nothing uses this key"))).color(self.style.dim));
            return;
        }
        let current = Chord { mods, key }.effective(platform);
        let chord_w = 72.0;
        let chord_layout = if self.env.text.rtl() { Layout::right_to_left(Align::Center) } else { Layout::left_to_right(Align::Center) };
        ScrollArea::vertical().id_salt(self.base.with("keylist")).auto_shrink([false, false]).show(ui, |ui| {
            for (c, h) in &entries {
                ui.horizontal(|ui| {
                    let mut text = RichText::new(self.chord(&h.holder.binding.trigger));
                    if *c == current {
                        text = text.strong();
                    }
                    ui.allocate_ui_with_layout(vec2(chord_w, ui.spacing().interact_size.y), chord_layout, |ui| ui.label(text));
                    if ui.link(self.vis(&self.holder_text(h))).clicked() {
                        clicks.go = Some(h.holder.command.clone());
                    }
                });
            }
        });
    }
}
