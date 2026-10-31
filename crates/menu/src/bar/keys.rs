//! The bar from the keyboard, as a Windows menu bar: a tap on Alt (or F10) puts the keyboard on
//! the bar (the first title lit; ← / → move along it, ↓ / ↩ / Space open the lit menu, Esc or a
//! click leaves), Alt+letter opens the menu with that mnemonic straight away, and in an open menu
//! a letter runs the row with that mnemonic (or opens its submenu).
//!
//! Mnemonics are underlined while Alt is held, the bar is focused or a menu was opened from the
//! keyboard; a menu opened with the mouse shows none until Alt is pressed, as Windows does.
//! They are assigned per menu level from the labels as shown (so translated), unique within
//! the level: the app's preferred letter where the label has it, else the first unused letter,
//! word starts first. Only ASCII letters and digits are keys the keyboard reports as such, so a
//! label without one (Japanese, Chinese…) shows its English label's letter after it, as Windows
//! menus do in those languages: "文件(F)" ([`localized`]).
//!
//! On a Mac target none of this runs: Option is a typing key there.

use egui::{Color32, Event, Id, Key, Modifiers, Stroke, TextFormat};

use craft_keymap::CommandRef;

use super::nav::{Nav, is_open};
use crate::Gate;

/// Keyboard state of the bar between frames.
#[derive(Clone, Debug, Default)]
pub(crate) struct Keys {
    /// The title the keyboard is on with no menu open, in screen order (Alt tap, F10).
    pub focus: Option<usize>,
    /// Underline the mnemonics: the bar is focused or a menu was opened from the keyboard.
    pub underline: bool,
    /// Alt is down (from its key events, whatever the platform reports as modifiers).
    alt_down: bool,
    /// Alt went down and nothing else was pressed since: releasing it is a tap.
    alt_tap: bool,
    /// The mouse buttons that are down, one bit each, from the button events in order (the
    /// frame's pointer state is its end state, which a whole click within one frame doesn't show).
    buttons_held: u8,
    /// A menu was just opened from the keyboard: light its first row once it is drawn.
    first_row: bool,
}

fn focus_id() -> Id {
    Id::unique("craft-menu-bar-focus")
}

fn leave_id() -> Id {
    Id::unique("craft-menu-bar-leave")
}

/// Whether the keyboard is on the bar (Alt tap or F10, no menu open yet). The app's shortcuts
/// leave the keys to it meanwhile, as they do for an open menu ([`super::is_open`]): ← / → / ↓ /
/// ↩ / Esc and the letters drive the bar. Call [`leave_for_shortcut`] first so that a command
/// shortcut still runs.
pub fn is_focused(ctx: &egui::Context) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<u64>(focus_id())).is_some_and(|seen| seen + 2 >= pass)
}

/// A command shortcut pressed while the keyboard is on the bar with no menu open (Ctrl+S, F7…)
/// takes it off the bar, so the shortcut runs in the same press, as in a native menu bar. Keys
/// the bar uses (arrows, ↩, Esc, letters, Alt+letter, F10) keep driving it, and an open menu
/// still holds the keys. True when the bar was left: [`is_focused`] is false from then on.
pub fn leave_for_shortcut(ctx: &egui::Context) -> bool {
    if !is_focused(ctx) || is_open(ctx) {
        return false;
    }
    let shortcut =
        ctx.input(|i| i.events.iter().any(|e| matches!(e, Event::Key { key, pressed: true, modifiers, .. } if is_command_key(*key, *modifiers))));
    if !shortcut {
        return false;
    }
    ctx.data_mut(|d| {
        d.remove::<u64>(focus_id());
        d.insert_temp(leave_id(), true);
    });
    ctx.request_repaint();
    true
}

/// A key press that is a command shortcut rather than a key of the bar: Cmd / Ctrl with a key
/// (not the modifier keys themselves), or a function key other than F10 (which toggles the bar).
pub(crate) fn is_command_key(key: Key, mods: Modifiers) -> bool {
    let modifier_key = matches!(
        key,
        Key::ShiftLeft | Key::ShiftRight | Key::ControlLeft | Key::ControlRight | Key::AltLeft | Key::AltRight | Key::SuperLeft | Key::SuperRight
    );
    if modifier_key {
        return false;
    }
    let function_key = key != Key::F10 && key.name().strip_prefix('F').is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    mods.command || mods.ctrl || mods.mac_cmd || function_key
}

/// The mnemonic a key press stands for: the letter or digit of a plain key.
pub(crate) fn key_char(key: Key) -> Option<char> {
    let name = key.name();
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphanumeric() => Some(c.to_ascii_uppercase()),
        _ => None,
    }
}

/// Mnemonics for `labels`, unique within the group: the preferred letter when the label has it,
/// else the first unused ASCII letter or digit, trying the first character of each word before
/// the others. Compared case-insensitively; the returned letter is upper case.
pub(crate) fn assign(labels: &[&str], preferred: &[Option<char>]) -> Vec<Option<char>> {
    let mut used: Vec<char> = Vec::new();
    let mut out = Vec::with_capacity(labels.len());
    for (i, label) in labels.iter().enumerate() {
        let upper: Vec<char> = label.chars().map(|c| c.to_ascii_uppercase()).collect();
        let has = |c: char| upper.contains(&c.to_ascii_uppercase());
        let mut pick = preferred.get(i).copied().flatten().map(|c| c.to_ascii_uppercase()).filter(|&c| has(c) && !used.contains(&c));
        if pick.is_none() {
            let starts = label.split(|c: char| !c.is_alphanumeric()).filter_map(|w| w.chars().next());
            let rest = label.chars();
            pick = starts.chain(rest).filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_uppercase()).find(|c| !used.contains(c));
        }
        if let Some(c) = pick {
            used.push(c);
        }
        out.push(pick);
    }
    out
}

/// The labels to show and their mnemonics for `labels` as the app shows them, whose English
/// originals are `english` (same order; `None` when the app gave none). A label with an ASCII
/// letter or digit keeps its text and is assigned as [`assign`] does. One without (Chinese,
/// Japanese, Korean…) can't be typed on its own, so it shows its English label's letter after it
/// in parentheses and takes that letter, as Windows menus do in those languages ("文件(F)").
/// Without an English label it keeps its text and gets no mnemonic. Mnemonics stay unique within
/// the group, and a shown "(F)" is always the row's key.
pub(crate) fn localized(labels: &[&str], english: &[Option<String>], preferred: &[Option<char>]) -> (Vec<String>, Vec<Option<char>>) {
    let typed = |l: &str| l.chars().any(|c| c.is_ascii_alphanumeric());
    // The key goes before a trailing ellipsis, as Windows writes it: "新建(N)…".
    let with_key = |l: &str, k: char| match l.strip_suffix('…').map(|b| (b, "…")).or_else(|| l.strip_suffix("...").map(|b| (b, "..."))) {
        Some((base, tail)) => format!("{base}({k}){tail}"),
        None => format!("{l}({k})"),
    };
    // Labels without an English one take part with a letterless stand-in, so the others keep
    // their places.
    let english: Vec<&str> = english.iter().map(|e| e.as_deref().unwrap_or("")).collect();
    let english_keys = assign(&english, preferred);
    let mut shown = Vec::with_capacity(labels.len());
    let mut pref = Vec::with_capacity(labels.len());
    for (i, label) in labels.iter().enumerate() {
        match english_keys.get(i).copied().flatten().filter(|_| !typed(label)) {
            Some(k) => {
                shown.push(with_key(label, k));
                pref.push(Some(k));
            }
            None => {
                shown.push((*label).to_owned());
                pref.push(preferred.get(i).copied().flatten());
            }
        }
    }
    // The suffixed labels take their keys first: a label with letters of its own must not take
    // the key a suffix shows ("Camera Raw 滤镜…" before "转换为智能滤镜(C)" leaves C to the latter).
    let suffixed = |i: &usize| shown.get(*i).map(String::as_str) != labels.get(*i).copied();
    let order: Vec<usize> = (0..shown.len()).filter(suffixed).chain((0..shown.len()).filter(|i| !suffixed(i))).collect();
    let ordered: Vec<&str> = order.iter().map(|&i| shown.get(i).map_or("", String::as_str)).collect();
    let ordered_pref: Vec<Option<char>> = order.iter().map(|&i| pref.get(i).copied().flatten()).collect();
    let mut keys = vec![None; shown.len()];
    for (&i, key) in order.iter().zip(assign(&ordered, &ordered_pref)) {
        if let Some(slot) = keys.get_mut(i) {
            *slot = key;
        }
    }
    (shown, keys)
}

/// Append `label` to `job` after `lead` points, with the first occurrence of `mnemonic`
/// (case-insensitively) underlined.
pub(crate) fn append_underlined(job: &mut egui::text::LayoutJob, label: &str, mut lead: f32, mnemonic: Option<char>, format: &TextFormat) {
    let at = mnemonic.and_then(|m| label.char_indices().find(|(_, c)| c.to_ascii_uppercase() == m).map(|(i, c)| (i, c.len_utf8())));
    let parts = match at {
        Some((i, len)) => [label.get(..i), label.get(i..i + len), label.get(i + len..)],
        None => [Some(label), None, None],
    };
    let mut any = false;
    for (k, part) in parts.into_iter().enumerate() {
        let Some(part) = part.filter(|p| !p.is_empty()) else { continue };
        let underline = if k == 1 { Stroke::new(1.0, format.color) } else { Stroke::NONE };
        job.append(part, lead, TextFormat { underline, ..format.clone() });
        lead = 0.0;
        any = true;
    }
    if !any {
        // An empty label still keeps its lead.
        job.append("", lead, format.clone());
    }
}

/// A title's text: `label` in `color`, its mnemonic underlined.
pub(crate) fn title_text(ui: &egui::Ui, label: &str, mnemonic: Option<char>, color: Color32) -> egui::WidgetText {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    append_underlined(&mut job, label, 0.0, mnemonic, &TextFormat { font_id: font, color, ..Default::default() });
    job.into()
}

/// Take `key` (with `mods`) out of this frame's events, and the text it typed: a letter the bar
/// used doesn't also land in a text field.
fn consume_letter(ctx: &egui::Context, mods: Modifiers, key: Key, c: char) {
    ctx.input_mut(|i| {
        i.consume_key(mods, key);
        i.events.retain(|e| !matches!(e, Event::Text(t) if t.chars().count() == 1 && t.chars().all(|t| t.to_ascii_uppercase() == c)));
    });
}

impl Keys {
    /// Last frame's state, without the focus when [`leave_for_shortcut`] took it.
    pub fn load(ctx: &egui::Context, id: Id) -> Keys {
        let mut keys: Keys = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
        if ctx.data_mut(|d| d.remove_temp::<bool>(leave_id())).unwrap_or(false) {
            keys.focus = None;
            keys.underline = false;
        }
        keys
    }

    /// The state as the last frame left it.
    #[cfg(test)]
    pub fn current(ctx: &egui::Context, id: Id) -> Keys {
        ctx.data(|d| d.get_temp(id)).unwrap_or_default()
    }

    pub fn store(self, ctx: &egui::Context, id: Id) {
        let pass = ctx.cumulative_pass_nr();
        ctx.data_mut(|d| {
            if self.focus.is_some() {
                d.insert_temp(focus_id(), pass);
            } else {
                d.remove::<u64>(focus_id());
            }
            d.insert_temp(id, self);
        });
    }

    /// Underline the mnemonics this frame?
    pub fn show_underlines(&self, ctx: &egui::Context) -> bool {
        self.underline || self.focus.is_some() || self.alt_down || ctx.input(|i| i.modifiers.alt)
    }

    /// Before drawing: a menu just opened from the keyboard lights its first row (once, when it
    /// is drawn); with no menu open and the keyboard off the bar, nothing is underlined any more.
    pub fn begin(&mut self, nav: &mut Nav) {
        if nav.top.is_some() {
            if self.first_row {
                nav.highlight_first(0);
                self.first_row = false;
            }
        } else if self.focus.is_none() && !self.first_row {
            self.underline = false;
        }
    }

    /// Off: a Mac target, or a dialog or the shortcut recorder has the keys. Forget the focus
    /// and any tap in progress.
    pub fn off(&mut self) {
        *self = Keys { buttons_held: self.buttons_held, ..Keys::default() };
    }

    /// Handle this frame's keys, after the menus were drawn and [`Nav::keys`] took the arrows.
    /// `tops` are the titles' popup ids and `mnemonics` their letters, both in screen order;
    /// `nav` is the open menu's state. A command a mnemonic ran lands in `clicked`.
    pub fn keys(
        &mut self,
        ctx: &egui::Context,
        gate: Gate,
        tops: &[Id],
        mnemonics: &[Option<char>],
        nav: &mut Nav,
        clicked: &mut Option<CommandRef>,
    ) {
        if tops.is_empty() {
            return;
        }
        let events = ctx.input(|i| i.events.clone());
        let mut tap = false;
        // The tap is judged on every event of the frame, also those another handler consumed
        // before the bar's turn: a shortcut such as Ctrl+Alt+I that ran first leaves no trace in
        // `events`, and releasing Alt must not then count as a tap.
        let raw = ctx.input(|i| i.raw.events.clone());
        for e in &raw {
            match e {
                Event::Key { key: Key::AltLeft | Key::AltRight, pressed: true, .. } => {
                    self.alt_down = true;
                    // Alt pressed while a mouse button is down is a modifier for that drag, never
                    // a tap.
                    self.alt_tap = self.buttons_held == 0;
                }
                Event::Key { key: Key::AltLeft | Key::AltRight, pressed: false, .. } => {
                    self.alt_down = false;
                    tap |= std::mem::take(&mut self.alt_tap);
                }
                // A click or the end of a drag while Alt is held is no tap. It also cancels a tap
                // taken earlier in this frame: a whole drag can arrive in one frame (press, Alt
                // down, Alt up, release).
                Event::PointerButton { button, pressed, .. } => {
                    let bit = 1u8 << (*button as u8);
                    if *pressed {
                        self.buttons_held |= bit;
                    } else {
                        self.buttons_held &= !bit;
                    }
                    self.alt_tap = false;
                    tap = false;
                }
                // Alt going to the desktop is no tap, and a button let go out there is up.
                Event::WindowFocused(false) => {
                    self.alt_tap = false;
                    self.buttons_held = 0;
                }
                // Alt+wheel (zoom, say) is no tap either.
                Event::Key { pressed: true, .. } | Event::Text(_) | Event::MouseWheel { .. } => self.alt_tap = false,
                _ => {}
            }
        }
        if events.iter().any(|e| matches!(e, Event::PointerButton { pressed: true, .. })) {
            self.focus = None;
            self.underline = false;
        }
        let open = nav.top.is_some();
        let n = tops.len();
        let open_top = |i: usize, keys: &mut Keys| {
            if let Some(&id) = tops.get(i) {
                egui::Popup::open_id(ctx, id);
                keys.focus = None;
                keys.underline = true;
                keys.first_row = true;
                ctx.request_repaint();
            }
        };
        // Alt+letter opens that menu, open or not (an open one switches), as on Windows.
        let alt_letter = events.iter().find_map(|e| match e {
            Event::Key { key, pressed: true, modifiers, .. } if modifiers.alt && !modifiers.command && !modifiers.ctrl => {
                key_char(*key).map(|c| (*key, c))
            }
            _ => None,
        });
        if let Some((key, c)) = alt_letter
            && let Some(i) = mnemonics.iter().position(|m| *m == Some(c))
        {
            consume_letter(ctx, Modifiers::ALT, key, c);
            self.alt_tap = false;
            open_top(i, self);
            return;
        }
        let letter = || {
            events.iter().find_map(|e| match e {
                Event::Key { key, pressed: true, modifiers, .. } if !modifiers.command && !modifiers.ctrl && !modifiers.alt && !modifiers.mac_cmd => {
                    key_char(*key).map(|c| (*key, c))
                }
                _ => None,
            })
        };
        if open {
            // A letter runs the row with that mnemonic in the level the arrows are in, a text
            // field having the keys or not: the open menu has them.
            if let Some((key, c)) = letter()
                && let Some(row) = nav.row_with_mnemonic(c)
            {
                consume_letter(ctx, Modifiers::NONE, key, c);
                self.underline = true;
                if let Some(cmd) = row.command.clone() {
                    *clicked = Some(cmd);
                    egui::Popup::close_all(ctx);
                } else {
                    nav.open_sub_row(ctx, row.widget);
                }
                ctx.request_repaint();
            }
            if tap {
                egui::Popup::close_all(ctx);
                self.underline = false;
                ctx.request_repaint();
            }
            return;
        }
        // No menu open: Alt (tapped) or F10 puts the keyboard on the bar or takes it off again.
        let f10 = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F10));
        if tap || f10 {
            self.focus = if self.focus.is_some() { None } else { Some(0) };
            self.underline = false;
            ctx.request_repaint();
            return;
        }
        let Some(focus) = self.focus.map(|f| f.min(n - 1)) else { return };
        let press = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
        if press(Key::Escape) {
            self.focus = None;
        } else if press(Key::ArrowRight) {
            self.focus = Some((focus + 1) % n);
        } else if press(Key::ArrowLeft) {
            self.focus = Some((focus + n - 1) % n);
        } else if press(Key::ArrowDown) || press(Key::Enter) || press(Key::Space) {
            open_top(focus, self);
        } else if let Some((key, c)) = letter() {
            if gate.text_editing {
                // Typing goes on in the text field, and the keyboard leaves the bar for it.
                self.focus = None;
            } else if let Some(i) = mnemonics.iter().position(|m| *m == Some(c)) {
                consume_letter(ctx, Modifiers::NONE, key, c);
                open_top(i, self);
            }
        } else {
            return;
        }
        ctx.request_repaint();
    }
}
