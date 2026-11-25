//! Alt+numpad code entry, folded into the input events in the raw-input step.
//!
//! Digits typed with Alt as the only modifier held collect into a decimal code; letting go of Alt types the character
//! the code names (`craft_codepage::alt_code_char`: `0233` is é in Windows-1252, `233` is Θ in OEM 437). The digits'
//! presses and their own text (Linux sends it with Alt held) reach nothing. The step runs before the IME and key
//! stages, so every field and the canvas read the result as typed text, and only the first pass of a frame gets the
//! frame's events, so each one is folded once.

use crate::runtime::env::AltCodePages;
use craft_host::target::Os;
use egui::{Event, Key, Modifiers};

/// A code past seven significant digits names nothing (U+10FFFF is 1114111): the entry stops growing there.
const MAX_DIGITS: usize = 8;

/// The Alt+numpad entry in progress in one viewport.
#[derive(Default)]
pub(crate) struct AltCodeEntry {
    /// The digits typed while Alt is held.
    code: String,
    /// A digit was typed in this Alt hold, also when the code was then dropped (a focus loss): the Alt release is no
    /// lone Alt, since the key stage never saw the digits.
    digits_typed: bool,
    /// The last digit press's own text, which comes after it, is not typed.
    swallow_text: bool,
    /// What the last code typed, for the character a platform composes itself and sends after it.
    echo: Option<Echo>,
}

/// The character an Alt code typed, kept until the next key or pointer press.
struct Echo {
    typed: String,
    /// The code names a byte of the code pages (1–255): the platform's own character for it wins over ours, since it
    /// knows the system's code pages (double-byte ones too) better than the app's locale guess.
    byte_code: bool,
}

/// Folds Alt+numpad entry out of `events` and into text events, decoding with `pages`. `os` is the platform the events
/// come from: on Windows (a browser on Windows too) the platform composes the code itself and sends its character
/// after the Alt release. Returns whether an Alt release in `events` ended a hold in which digits were typed (a code,
/// or one a focus loss dropped), so the key stage does not take it for a lone Alt. The caller does not call it on macOS, where Option+digit types its own characters.
///
/// `claimed` tells whether the key stage takes a press as a chord the key target claims (the focused component, or with
/// nothing focused the window's document canvas) or a capture holds: such an Alt+digit is its owner's, as an app's
/// shortcut goes before Alt-code entry, and it ends the entry like any other key.
pub(crate) fn fold(entry: &mut AltCodeEntry, events: &mut Vec<Event>, pages: AltCodePages, os: Os, claimed: &dyn Fn(&Event) -> bool) -> bool {
    let composes = os == Os::Windows;
    let mut ended = false;
    let mut out = Vec::with_capacity(events.len() + 1);
    for event in events.drain(..) {
        match &event {
            Event::Key { key, pressed: true, repeat, modifiers, .. } => {
                entry.swallow_text = false;
                entry.echo = None;
                match digit(*key).filter(|_| alt_only(modifiers) && !claimed(&event)) {
                    Some(d) => {
                        // A held digit's auto-repeat adds nothing, as on Windows.
                        if !*repeat && entry.code.len() < MAX_DIGITS {
                            entry.code.push(d);
                        }
                        entry.digits_typed = true;
                        entry.swallow_text = true;
                        continue;
                    }
                    // Another key (Alt itself, a letter, a digit with Ctrl or Shift) ends the entry unfinished.
                    None => {
                        entry.code.clear();
                        // A new Alt hold starts clean: a release that went to another window ended the last one.
                        if matches!(key, Key::AltLeft | Key::AltRight) && !*repeat {
                            entry.digits_typed = false;
                        }
                    }
                }
            }
            Event::Text(_) if std::mem::take(&mut entry.swallow_text) => continue,
            Event::Text(text) => {
                if let Some(echo) = entry.echo.take() {
                    if echo.typed == *text {
                        // The platform composed the same character: it is typed once.
                        continue;
                    }
                    if composes {
                        // On Windows the text after a code is the platform's character for it. For a code up to 255
                        // it replaces ours, which was typed just before with no key or click since; above 255 ours
                        // stays (Windows reduces the code to a byte, Word types the Unicode character).
                        if echo.byte_code && text.chars().count() == 1 {
                            // A whole Backspace tap: its release ends it before the text, so the key stage reads the
                            // text as typed on its own and not as the text of a press the keymap binds.
                            for pressed in [true, false] {
                                out.push(Event::Key { key: Key::Backspace, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
                            }
                            out.push(event);
                        }
                        continue;
                    }
                    // Linux and the BSDs compose nothing: a different text is what was typed next.
                }
            }
            Event::PointerButton { pressed: true, .. } => entry.echo = None,
            // Alt going down with no code under way starts a new hold, whose digits are its own.
            Event::ModifiersChanged(modifiers) if modifiers.alt && entry.code.is_empty() => entry.digits_typed = false,
            // Focus in another window ends a code unfinished: Alt's release goes there.
            Event::WindowFocused(false) => {
                entry.code.clear();
                entry.swallow_text = false;
            }
            _ => {}
        }
        let released = alt_released(&event);
        out.push(event);
        if released {
            ended |= std::mem::take(&mut entry.digits_typed);
        }
        if released && !entry.code.is_empty() {
            let code = std::mem::take(&mut entry.code);
            // A digit's own text comes with its press, before Alt is let go: text after this is the platform's
            // composed character or what is typed next. A digit that sent no text must not cost the next one.
            entry.swallow_text = false;
            if let Some(c) = craft_codepage::alt_code_char(&code, pages.ansi, pages.oem) {
                let byte_code = code.trim_start_matches('0').parse::<u32>().is_ok_and(|value| value <= 255);
                entry.echo = Some(Echo { typed: c.to_string(), byte_code });
                out.push(Event::Text(c.to_string()));
            }
        }
    }
    *events = out;
    ended
}

/// The digit `key` adds to a code. egui reports keypad and top-row digits alike (`Key::Num0`…`Num9`) and carries no
/// key location, so the top-row digits compose codes too; with Alt alone held they type nothing else.
fn digit(key: Key) -> Option<char> {
    let n = match key {
        Key::Num0 => 0,
        Key::Num1 => 1,
        Key::Num2 => 2,
        Key::Num3 => 3,
        Key::Num4 => 4,
        Key::Num5 => 5,
        Key::Num6 => 6,
        Key::Num7 => 7,
        Key::Num8 => 8,
        Key::Num9 => 9,
        _ => return None,
    };
    char::from_digit(n, 10)
}

/// Alt is the only modifier held: the one a code's digits are typed with. AltGr reads as Ctrl+Alt on Windows, so its
/// digits are no code.
fn alt_only(modifiers: &Modifiers) -> bool {
    modifiers.alt && !modifiers.ctrl && !modifiers.shift && !modifiers.command && !modifiers.mac_cmd
}

/// Whether `event` lets go of Alt: its key's release, or held modifiers without it.
fn alt_released(event: &Event) -> bool {
    match event {
        Event::Key { key: Key::AltLeft | Key::AltRight, pressed: false, .. } => true,
        Event::ModifiersChanged(modifiers) => !modifiers.alt,
        _ => false,
    }
}
