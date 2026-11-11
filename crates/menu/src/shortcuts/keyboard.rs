//! The on-screen keyboard pane (`Options::keyboard`): a US ANSI layout coloured by who holds each key with the
//! chosen modifiers, modifier toggles, the shortcuts of the clicked key and Assign to Selected.

use std::collections::HashMap;
use std::sync::Arc;

use craft_behavior::nav::NavShape;
use craft_behavior::{Metrics, PartKey, Roving, Sense};
use craft_keymap::{Binding, Chord, CommandRef, Holder, Key, Keymap, Mods, Overrides, Platform, Scope, Trigger};
use craft_theme::variant::Tone;
use craft_ui::buttons::{Button, ToggleButton};
use craft_ui::contract::{
    Access, Bind, BodyCx, Component, Cx, Input, MeasureCx, Name, NoValue, Outcome, PaintUi, PartsCx, Pointer as _, Props as _, Role, Show as _,
    ShowContainer as _, clicked,
};
use craft_ui::display::Text;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::primitives::{PaintShape, StrokeWidth, TextRun, Wrap};
use craft_ui::style::{ComponentStyle, FocusLook, StyleColor, StyleCx, TextLook, TypeRole};
use egui::{CornerRadius, Rect, Response, Ui, Vec2, vec2};

use super::state::{DialogState, Env};
use super::words::Say;

/// The keyboard's width and height in key units: 15 for the main block, a gap, 3 for the navigation cluster; six
/// rows and the gap under the function row.
const UNITS_W: f32 = 18.5;
const UNITS_H: f32 = 6.25;
/// The gap under the function row, in key units.
const ROW_GAP: f32 = 0.25;
/// Where the navigation cluster starts, in key units.
const NAV_AT: f32 = 15.5;

const KEY: PartKey = PartKey::new("key");
const MODIFIER: PartKey = PartKey::new("modifier");

/// A modifier the pane toggles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mod {
    Cmd,
    Ctrl,
    Alt,
    Shift,
}

impl Mod {
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

    fn glyph(self) -> &'static str {
        match self {
            Mod::Cmd => "⌘",
            Mod::Ctrl => "⌃",
            Mod::Alt => "⌥",
            Mod::Shift => "⇧",
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

type CapRow = &'static [(Cap, f32)];

const fn k(key: Key) -> (Cap, f32) {
    (Cap::Key(key), 1.0)
}

const MAIN_ROWS: [CapRow; 4] = [
    &[
        k(Key::Backtick),
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
];

const FUNCTION_ROW: CapRow = &[
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

const MAC_BOTTOM: CapRow = &[
    (Cap::Mod(Mod::Ctrl), 1.5),
    (Cap::Mod(Mod::Alt), 1.25),
    (Cap::Mod(Mod::Cmd), 1.5),
    (Cap::Key(Key::Space), 6.5),
    (Cap::Mod(Mod::Cmd), 1.5),
    (Cap::Mod(Mod::Alt), 1.25),
    (Cap::Mod(Mod::Ctrl), 1.5),
];

/// Off macOS the Ctrl keys are the command modifier.
const PC_BOTTOM: CapRow = &[
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

impl Class {
    fn of(holders: &[KeyHolder]) -> Class {
        match (holders.iter().any(|h| h.scope.is_app()), holders.iter().any(|h| !h.scope.is_app())) {
            (false, false) => Class::Free,
            (true, false) => Class::App,
            (false, true) => Class::Scoped,
            (true, true) => Class::Both,
        }
    }

    /// The theme tone a class is drawn in.
    const fn tone(self) -> Tone {
        match self {
            Class::Free => Tone::Neutral,
            Class::App => Tone::Accent,
            Class::Scoped => Tone::Warn,
            Class::Both => Tone::Bad,
        }
    }
}

type Index = HashMap<Chord, Vec<KeyHolder>>;

/// The pane's state between passes.
#[derive(Clone, Debug, Default)]
pub(crate) struct KeyboardState {
    /// Hidden with the header's Keyboard toggle.
    pub hidden: bool,
    /// The modifiers toggled on.
    pub mods: Mods,
    /// The key whose shortcuts are listed.
    pub key: Option<Key>,
    /// Every binding by effective chord, rebuilt when the overrides change.
    index: Option<Arc<(Overrides, Index)>>,
}

impl KeyboardState {
    fn index(&mut self, keymap: &Keymap) -> Arc<(Overrides, Index)> {
        if let Some(i) = self.index.as_ref().filter(|i| i.0 == *keymap.overrides()) {
            return i.clone();
        }
        let platform = keymap.platform();
        let mut map: Index = HashMap::new();
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

/// One cap as the pane shows it this pass.
struct CapView {
    cap: Cap,
    at: Vec2,
    width: f32,
    class: Class,
    /// A modifier cap that is on, or the selected key.
    marked: bool,
    legend: String,
    title: Option<String>,
    name: String,
    description: String,
}

/// What the pane reports.
#[non_exhaustive]
pub(crate) struct PaneOutput {
    pub response: Response,
    /// The key clicked this pass.
    pub key: Option<Key>,
    /// The modifier cap clicked this pass.
    pub toggled: Option<Mod>,
}

impl Outcome for PaneOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The style of the pane.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub(crate) struct PaneStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    #[style(text)]
    text: TextLook,
    /// The step one key unit is: a control's height.
    step: f32,
    radius: CornerRadius,
    /// Cap fills by class, and the ink that reads on each: free, application, panel or context, both.
    fills: [StyleColor; 4],
    inks: [StyleColor; 4],
    /// A modifier cap that is on.
    on_fill: StyleColor,
    on_ink: StyleColor,
}

/// The keycaps, one part each, over the layout table.
#[derive(craft_ui::contract::Component)]
#[component(sample = KeyCaps::sample)]
struct KeyCaps {
    #[common]
    common: craft_ui::contract::Common,
    caps: Vec<CapView>,
    key: Option<Key>,
    toggled: Option<Mod>,
}

impl KeyCaps {
    /// The keycaps of an empty layout, for the registry's style measurements.
    #[cfg(feature = "screen-testing")]
    fn sample() -> Self {
        KeyCaps { common: craft_ui::contract::Common::new(), caps: Vec::new(), key: None, toggled: None }
    }

    fn unit(rect: Rect) -> f32 {
        (rect.width() / UNITS_W).min(rect.height() / UNITS_H)
    }

    fn cell(rect: Rect, unit: f32, view: &CapView) -> Rect {
        Rect::from_min_size(rect.min + view.at * unit, vec2(view.width * unit, unit))
    }

    fn part(index: usize, view: &CapView) -> craft_behavior::PartRef {
        match view.cap {
            Cap::Key(_) => KEY.at(index as u64),
            _ => MODIFIER.at(index as u64),
        }
    }

    fn interactive(view: &CapView) -> bool {
        matches!(view.cap, Cap::Key(_) | Cap::Mod(_))
    }
}

impl Component for KeyCaps {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: craft_ui::runtime::focus::KeyUse = craft_ui::runtime::focus::KeyUse::of(Input::Pointer).with_moves();
    type Style = PaneStyle;
    type Output = PaneOutput;

    fn style(cx: &StyleCx<'_>, _: Self::StyleKey) -> PaneStyle {
        let classes = [Class::Free, Class::App, Class::Scoped, Class::Both];
        PaneStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            text: TextLook::new(TypeRole::Caption, craft_theme::Stateful::new(cx.ink(craft_ui::style::Ink::OnPanel))),
            step: cx.control().height,
            radius: cx.surface(craft_ui::style::SurfaceKind::Raised).radius,
            fills: classes.map(|c| cx.tone_fill(c.tone())),
            inks: classes.map(|c| if c == Class::Free { cx.ink(craft_ui::style::Ink::OnPanel) } else { cx.tone(c.tone()) }),
            on_fill: cx.tone_fill(Tone::Info),
            on_ink: cx.tone(Tone::Info),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let step = cx.style.step;
        vec2(UNITS_W * step, UNITS_H * step)
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let unit = Self::unit(rect);
        for (at, view) in self.caps.iter().enumerate().filter(|(_, view)| Self::interactive(view)) {
            let part = cx.part(Self::part(at, view), Self::cell(rect, unit, view)).sense(Sense::CLICK | Sense::FOCUS).focusable();
            if view.marked {
                part.selected();
            }
        }
        cx.roving(Roving::new(NavShape::Spatial));
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        for (at, view) in self.caps.iter().enumerate().filter(|(_, view)| Self::interactive(view)) {
            if cx.part_activated(Self::part(at, view)).is_some() {
                match view.cap {
                    Cap::Key(key) => self.key = Some(key),
                    Cap::Mod(m) => self.toggled = Some(m),
                    _ => {}
                }
            }
        }
    }

    fn access(&self, access: &mut Access, _cx: &Cx<'_, Self>) {
        for (at, view) in self.caps.iter().enumerate().filter(|(_, view)| Self::interactive(view)) {
            let node = access.node(Self::part(at, view), Role::BUTTON);
            node.name = Name::Text(view.name.clone());
            node.description = Some(view.description.clone());
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let unit = Self::unit(cx.rect);
        let style = cx.style;
        let draw = ui.draw();
        for (at, view) in self.caps.iter().enumerate() {
            if matches!(view.cap, Cap::Gap) {
                continue;
            }
            let cell = Self::cell(cx.rect, unit, view);
            let (fill, ink) = if view.marked && matches!(view.cap, Cap::Mod(_)) {
                (style.on_fill, style.on_ink)
            } else {
                (style.fills[view.class as usize], style.inks[view.class as usize])
            };
            let shape = PaintShape::Rect(cell, style.radius);
            draw.fill(&shape, fill);
            if Self::interactive(view) && cx.part(Self::part(at, view)).hovered {
                draw.stroke(&shape, StrokeWidth::Medium, ink);
            }
            if view.marked && matches!(view.cap, Cap::Key(_)) {
                draw.stroke(&shape, StrokeWidth::Thick, ink);
            }
            let (top, bottom) = cell.split_top_bottom_at_fraction(0.5);
            if let Some(title) = &view.title {
                draw.text(top, &TextRun::new(title.as_str()).role(style.text.role).color(ink).wrap(Wrap::Elide));
            }
            draw.text(bottom, &TextRun::new(view.legend.as_str()).role(style.text.role).color(ink).wrap(Wrap::Elide));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> PaneOutput {
        PaneOutput { response: cx.response.clone(), key: self.key, toggled: self.toggled }
    }
}

fn mods_shown(platform: Platform) -> &'static [Mod] {
    if platform.is_mac() { &[Mod::Cmd, Mod::Ctrl, Mod::Alt, Mod::Shift] } else { &[Mod::Cmd, Mod::Alt, Mod::Shift] }
}

/// A modifier cap's word, without the glyph (which goes above it on macOS).
fn mod_word(say: &Say, m: Mod, mac: bool) -> String {
    let w = say.words();
    match (m, mac) {
        (Mod::Cmd, true) => w.command.as_str(),
        (Mod::Ctrl, true) => w.control.as_str(),
        (Mod::Alt, true) => w.option.as_str(),
        (Mod::Cmd | Mod::Ctrl, false) => w.ctrl.as_str(),
        (Mod::Alt, false) => w.alt.as_str(),
        (Mod::Shift, _) => w.shift.as_str(),
    }
    .to_owned()
}

/// A modifier's name, with its glyph on macOS.
fn mod_name(say: &Say, m: Mod, mac: bool) -> String {
    let word = mod_word(say, m, mac);
    if mac { format!("{} {word}", m.glyph()) } else { word }
}

/// The English legend printed on a key: Apple's words on macOS, PC words elsewhere. A key with a word in the
/// scope's language shows that word.
fn cap_legend(say: &Say, key: Key, platform: Platform) -> String {
    let mac = platform.is_mac();
    let printed = match (key, mac) {
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
        (k, _) => {
            return say.trigger(
                &Trigger::single(Chord::plain(k)),
                if matches!(k, Key::Up | Key::Down | Key::Left | Key::Right) { Platform::Mac } else { platform },
            );
        }
    };
    printed.to_owned()
}

/// The holder's label, with its scope when that is not Application.
fn holder_text(say: &Say, keymap: &Keymap, h: &KeyHolder) -> String {
    let label = say.label_of(keymap, &h.holder.command);
    if h.scope.is_app() { label } else { say.words().label_scope.fill(&[("label", label.as_str()), ("scope", say.scope_name(&h.scope).as_str())]) }
}

/// Every cap of the layout with its colour, legend and name for `mods`.
fn cap_views(env: &Env<'_>, state: &KeyboardState, index: &Index, mods: Mods) -> Vec<CapView> {
    let say = env.text;
    let platform = env.keymap.platform();
    let mac = platform.is_mac();
    let words = say.words();
    let bottom = if mac { MAC_BOTTOM } else { PC_BOTTOM };
    let mut placed: Vec<(Cap, Vec2, f32)> = Vec::new();
    let rows = std::iter::once(FUNCTION_ROW).chain(MAIN_ROWS.iter().copied()).chain(std::iter::once(bottom));
    for (r, row) in rows.enumerate() {
        let y = r as f32 + if r > 0 { ROW_GAP } else { 0.0 };
        let mut x = 0.0;
        for &(cap, w) in row {
            placed.push((cap, vec2(x, y), w));
            x += w;
        }
    }
    for &(r, col, key) in NAV {
        placed.push((Cap::Key(key), vec2(NAV_AT + col, r as f32 + ROW_GAP), 1.0));
    }
    placed
        .into_iter()
        .map(|(cap, at, width)| {
            let mut view = CapView {
                cap,
                at,
                width,
                class: Class::Free,
                marked: false,
                legend: String::new(),
                title: None,
                name: String::new(),
                description: String::new(),
            };
            match cap {
                Cap::Key(key) => {
                    let holders = index.get(&Chord { mods, key }.effective(platform)).map_or(&[][..], Vec::as_slice);
                    view.class = Class::of(holders);
                    let first = holders.iter().find(|h| h.scope.is_app()).or(holders.first());
                    view.title = first.map(|h| say.label_of(env.keymap, &h.holder.command));
                    view.legend = cap_legend(say, key, platform);
                    let spoken =
                        if mods.is_none() { say.key_word(key, platform) } else { say.trigger(&Trigger::single(Chord { mods, key }), platform) };
                    let mut names: Vec<String> = Vec::new();
                    for h in holders {
                        let t = holder_text(say, env.keymap, h);
                        if !names.contains(&t) {
                            names.push(t);
                        }
                    }
                    view.name = if names.is_empty() {
                        spoken
                    } else {
                        words.chord_label.fill(&[("chord", spoken.as_str()), ("label", names.join(", ").as_str())])
                    };
                    view.description = match view.class {
                        Class::Free => words.not_assigned.as_str(),
                        Class::App => words.application.as_str(),
                        Class::Scoped => words.panel_or_context.as_str(),
                        Class::Both => words.both.as_str(),
                    }
                    .to_owned();
                    view.marked = state.key == Some(key);
                }
                Cap::Mod(m) => {
                    view.legend = mod_word(say, m, mac);
                    view.title = mac.then(|| m.glyph().to_owned());
                    view.marked = m.get(mods);
                    view.name = mod_name(say, m, mac);
                }
                Cap::CapsLock => view.legend = words.caps_lock.as_str().to_owned(),
                Cap::System => {
                    view.legend = if platform == Platform::Windows { words.win.as_str().to_owned() } else { words.super_key.as_str().to_owned() }
                }
                Cap::Gap => {}
            }
            view
        })
        .collect()
}

/// Shows the pane: the modifier toggles, the keys, the clicked key's shortcuts and Assign to Selected.
pub(crate) fn show(ui: &mut Ui, _cx: &mut BodyCx<'_>, env: &mut Env<'_>, state: &mut DialogState) {
    let say = env.text;
    let platform = env.keymap.platform();
    let mac = platform.is_mac();
    let words = say.words();
    let mut toggled = None;
    let mut picked_key = None;
    let mut go = None;
    let mut assign = false;
    let mods = state.keyboard.mods;
    let index = state.keyboard.index(env.keymap);
    Stack::new().show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            let layout = if mac { &words.keyboard_us_macos } else { &words.keyboard_us };
            Text::new(layout.clone()).show(ui);
            for &m in mods_shown(platform) {
                let mut on = m.get(mods);
                ToggleButton::new(mod_name(say, m, mac), Bind::bound(&mut on)).id_salt(("shortcuts.mod", m.glyph())).show(ui);
                if on != m.get(mods) {
                    toggled = Some(m);
                }
            }
            for (tone, word) in [
                (Tone::Accent, &words.application),
                (Tone::Warn, &words.panel_or_context),
                (Tone::Bad, &words.both),
                (Tone::Info, &words.modifier_on),
            ] {
                Text::new(word.clone()).tone(tone).show(ui);
            }
        });
        Row::new().show(ui, |ui, _| {
            let caps = KeyCaps {
                common: craft_ui::contract::Common::new(),
                caps: cap_views(env, &state.keyboard, &index.1, mods),
                key: None,
                toggled: None,
            };
            let pane = caps.id_salt("shortcuts.keyboardCaps").show(ui);
            picked_key = pane.key;
            toggled = toggled.or(pane.toggled);
            Stack::new().show(ui, |ui, _| key_list(ui, env, state, &index.1, mods, &mut go, &mut assign));
        });
    });
    if let Some(m) = toggled {
        m.flip(&mut state.keyboard.mods);
    }
    if let Some(key) = picked_key {
        state.keyboard.key = Some(key);
    }
    if let Some(c) = go {
        state.go_to(&c);
    }
    if assign && let (Some(command), Some(key)) = (state.selected.clone(), state.keyboard.key) {
        let trigger = Trigger::single(Chord { mods, key });
        let binding = match &state.context {
            Some(scope) => Binding::in_scope(trigger, scope.clone()),
            None => Binding::new(trigger),
        };
        state.stop_recording();
        state.commit(env, &command, None, binding);
    }
}

/// The clicked key's shortcuts with any modifiers, and Assign to Selected.
fn key_list(ui: &mut Ui, env: &Env<'_>, state: &DialogState, index: &Index, mods: Mods, go: &mut Option<CommandRef>, assign: &mut bool) {
    let say = env.text;
    let words = say.words();
    let platform = env.keymap.platform();
    let Some(key) = state.keyboard.key else {
        Text::new(words.click_a_key_to_see_its.clone()).shade(craft_ui::display::Shade::Muted).show(ui);
        return;
    };
    Text::new(words.key_key.fill(&[("key", say.key_word(key, platform).as_str())])).show(ui);
    if env.options.editable {
        let chord = say.trigger(&Trigger::single(Chord { mods, key }), platform);
        let selected = state.selected.clone().filter(|c| env.keymap.bindable(c).is_some());
        let tip = match &selected {
            Some(c) => super::words::around_chord(
                say.rtl(),
                &words.assign_chord_to_label.fill(&[("chord", chord.as_str()), ("label", say.label_of(env.keymap, c).as_str())]),
                &chord,
            ),
            None => words.select_a_command_first.as_str().to_owned(),
        };
        let button = Button::new(words.assign_to_selected.clone()).id_salt("shortcuts.assignKey").enabled(selected.is_some()).tooltip(tip).show(ui);
        if clicked(&button) {
            *assign = true;
        }
    }
    let mut entries: Vec<(Chord, &KeyHolder)> =
        index.iter().filter(|(c, _)| c.key == key).flat_map(|(c, hs)| hs.iter().map(move |h| (*c, h))).collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.holder.command.id.cmp(&b.1.holder.command.id)));
    if entries.is_empty() {
        Text::new(words.nothing_uses_this_key.clone()).shade(craft_ui::display::Shade::Muted).show(ui);
        return;
    }
    let current = Chord { mods, key }.effective(platform);
    for (at, (chord, holder)) in entries.iter().enumerate() {
        Row::new().show(ui, |ui, _| {
            let text = say.trigger(&holder.holder.binding.trigger, platform);
            let shade = if *chord == current { craft_ui::display::Shade::Normal } else { craft_ui::display::Shade::Muted };
            Text::new(text).shade(shade).id_salt(("shortcuts.keyChord", at)).show(ui);
            if clicked(&craft_ui::buttons::Link::new(holder_text(say, env.keymap, holder)).id_salt(("shortcuts.keyHolder", at)).show(ui)) {
                *go = Some(holder.holder.command.clone());
            }
        });
    }
}
