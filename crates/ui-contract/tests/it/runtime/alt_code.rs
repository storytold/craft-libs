//! SG-32: Alt+numpad codes, folded into typed text by the raw-input step of every viewport, so a focused text field
//! takes the character as typing.

use craft_behavior::{EditorInput, PartKey, Sense, TextPos};
use craft_host::target::Os;
use craft_ui::contract::{Common, Component, Cx, Input, PaintUi, PartsCx, Props as _, Role, Show as _};
use craft_ui::primitives::{EditorLine, EditorPurpose, EditorRun, EditorState};
use craft_ui::runtime::env::{AltCodePages, RootEnv};
use craft_ui::runtime::focus::{FocusTarget, KeyUse, host_canvas, request};
use craft_ui::runtime::frame::take_output;
use craft_ui::style::StyleCx;
use craft_ui::testing::focus::{named_id, widget};
use craft_ui::testing::input::{key_press, key_repeat, text};
use craft_ui::testing::pointer::click;
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Event, Key, Modifiers, Pos2, Rect, Response};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const AT: Rect = Rect { min: Pos2::new(10.0, 10.0), max: Pos2::new(210.0, 90.0) };
const SHOWN: &str = "abc";
const EDITOR: PartKey = PartKey::new("editor");

/// A text field that keeps what it is typed: text joins it and a Backspace takes its last character. It holds drafts,
/// so a lone Alt in it alerts (`FrameOutput::refused`) instead of focusing a menu bar: the trial reads that alert to
/// tell whether the key stage took an Alt release for a lone Alt.
#[derive(craft_ui::contract::Component)]
struct Probe<'a> {
    #[common]
    common: Common,
    typed: &'a RefCell<String>,
}

impl<'a> Probe<'a> {
    fn new(typed: &'a RefCell<String>) -> Self {
        Probe { common: Common::new(), typed }
    }
}

impl Component for Probe<'_> {
    const ROLE: Role = Role::MULTILINE_TEXT_INPUT;
    const INPUT: craft_ui::contract::Input = craft_ui::contract::Input::Editor;
    const KEYS: KeyUse = KeyUse::of(Input::Editor).with_holds_drafts();
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: ProbeStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS | Sense::TAKE_FOCUS
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.editor(EDITOR, rect, EditorPurpose::Text);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        for input in cx.editor_input() {
            match input {
                EditorInput::Text(added) => self.typed.borrow_mut().push_str(&added),
                EditorInput::Delete { forward: false, .. } => {
                    self.typed.borrow_mut().pop();
                }
                _ => {}
            }
        }
    }

    fn paint(&mut self, _: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let positions: Vec<f32> = (0..SHOWN.len()).map(|at| at as f32 * 8.0).collect();
        let widths = vec![8.0; SHOWN.len()];
        let runs = [EditorRun::new(0..SHOWN.len(), false, &positions, &widths)];
        let line = Rect::from_min_size(cx.content_rect.min, egui::vec2(cx.content_rect.width(), 16.0));
        let lines = [EditorLine::new(SHOWN, line, line.bottom(), &runs)];
        let caret = Rect::from_min_size(line.min + egui::vec2(SHOWN.len() as f32 * 8.0, 0.0), egui::vec2(0.0, 16.0));
        cx.editor_state(EditorState::new(caret, &lines, TextPos::new(0, SHOWN.len())));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// The field in a window on one OS with the app's code pages, focused by a click.
struct Session {
    frames: Frames,
    typed: Rc<RefCell<String>>,
    /// The next frames run a second pass, which egui gives none of the frame's events.
    rerun: Rc<Cell<bool>>,
}

/// What one input sequence did: the text it typed into the field and whether the key stage alerted for a lone Alt.
#[derive(Debug, PartialEq)]
struct Typed {
    text: String,
    lone_alt: bool,
}

impl Session {
    fn new(os: Os, pages: AltCodePages) -> Self {
        let mut frames = Frames::new();
        craft_ui::testing::windows::set_os(&frames.ctx, os);
        frames.root(RootEnv::default().alt_code_pages(pages));
        let mut session = Session { frames, typed: Rc::default(), rerun: Rc::default() };
        let mut show = session.show();
        session.frames.idle(3, &mut show);
        session.focus();
        session
    }

    /// The frame body: the field, and a discard of the first pass while `rerun` is set.
    fn show(&self) -> impl FnMut(&mut egui::Ui) + use<> {
        let (typed, rerun) = (Rc::clone(&self.typed), Rc::clone(&self.rerun));
        move |ui| {
            Probe::new(&typed).label("Text").id_salt("alt_code_field").show_at(ui, AT);
            if rerun.get() && ui.ctx().current_pass_index() == 0 {
                ui.ctx().request_discard("the trial reruns each input frame");
            }
        }
    }

    /// Clicks the field, which focuses it.
    fn focus(&mut self) {
        let mut show = self.show();
        click(&mut self.frames, AT.center(), &mut show);
        self.frames.idle(1, &mut show);
    }

    /// Plays `frames` of input, each frame running two passes, then a frame with none; returns what they typed.
    fn send(&mut self, frames: Vec<Vec<Event>>) -> Typed {
        let mut show = self.show();
        take_output(&self.frames.ctx);
        self.typed.borrow_mut().clear();
        self.rerun.set(true);
        for events in frames {
            let passes = self.frames.frame(events, &mut show);
            assert_eq!(passes, 2, "each input frame runs a second pass, which gets none of the frame's events");
        }
        self.rerun.set(false);
        self.frames.frame(Vec::new(), &mut show);
        Typed { text: self.typed.take(), lone_alt: take_output(&self.frames.ctx).refused }
    }
}

const ALT: Modifiers = Modifiers::ALT;

/// The keypad digit key of `digit`; egui names keypad and top-row digits alike.
fn digit_key(digit: char) -> Key {
    const DIGITS: [Key; 10] = [Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
    DIGITS[digit.to_digit(10).expect("a decimal digit") as usize]
}

fn key_release(key: Key, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: Some(key), pressed: false, repeat: false, modifiers }
}

/// Alt goes down or comes up, as the platform reports held modifiers.
fn alt(down: bool) -> Event {
    Event::ModifiersChanged(if down { ALT } else { Modifiers::NONE })
}

/// The frame of one digit typed with Alt held: its press with the digit's own text (Linux sends it), then its release.
fn digit(d: char) -> Vec<Event> {
    let key = digit_key(d);
    vec![key_press(key, ALT), text(&d.to_string()), key_release(key, ALT)]
}

/// The frames of an Alt code: Alt down, each digit in a frame of its own, then Alt up.
fn code(digits: &str) -> Vec<Vec<Event>> {
    std::iter::once(vec![alt(true)]).chain(digits.chars().map(digit)).chain([vec![alt(false)]]).collect()
}

/// `code(digits)`, then a frame in which the platform sends `composed`, the character it made of the code itself.
fn code_then(digits: &str, composed: &str) -> Vec<Vec<Event>> {
    let mut frames = code(digits);
    frames.push(vec![text(composed)]);
    frames
}

fn typed(text: &str) -> Typed {
    Typed { text: text.to_owned(), lone_alt: false }
}

/// SG-32 and sh.K6: Alt with keypad digits types the character the app's code pages give the code, once, as the
/// text of the Alt release, and that release is no lone Alt. Every input frame runs a second pass: only the first
/// gets the frame's events, so no digit and no character is folded twice.
///
/// Windows composes the code itself and sends its character after the release: the same character is typed once; a
/// different one for a code up to 255 replaces ours (the platform knows the system's code pages better than the
/// app's locale guess); any other text then is dropped. The platform signal is the shell's OS, on every runtime:
/// unlike production's web build, a browser on Windows replaces the echo too, since it gets the composed character
/// as well. Linux composes nothing, so a different text after a code is what was typed next. macOS has no Alt
/// codes: Option with a digit types the platform's own character.
#[trial]
fn an_alt_numpad_code_types_its_character_once() {
    let western = AltCodePages::new(1252, 437);
    for os in [Os::Windows, Os::Linux] {
        let mut session = Session::new(os, western);
        let lone = session.send(vec![vec![alt(true)], vec![alt(false)]]);
        assert_eq!(lone, Typed { text: String::new(), lone_alt: true }, "{os:?}: a lone Alt in the field alerts");

        assert_eq!(session.send(code("0233")), typed("é"), "{os:?}: a leading 0 reads the ANSI page, the digits type nothing");
        assert_eq!(session.send(code("233")), typed("Θ"), "{os:?}: no leading 0 reads the OEM page");
        assert_eq!(session.send(code("8364")), typed("€"), "{os:?}: above 255 the code is the Unicode character");
        // With the repeat counted the code would be 02333, U+091D.
        let held = vec![key_press(Key::Num3, ALT), text("3"), key_repeat(Key::Num3, ALT), text("3"), key_release(Key::Num3, ALT)];
        let held_digit = vec![vec![alt(true)], digit('0'), digit('2'), held, digit('3'), vec![alt(false)]];
        assert_eq!(session.send(held_digit), typed("é"), "{os:?}: a digit's auto-repeat adds nothing and types nothing");
        // Eight digits are kept: 00000233 is é, where all nine (2334) would be U+091E.
        assert_eq!(session.send(code("000002334")), typed("é"), "{os:?}: a ninth digit is ignored");

        // The platform composes the same character: it is typed once on every OS.
        assert_eq!(session.send(code_then("0233", "é")), typed("é"), "{os:?}: the composed echo of our character is dropped");
        let replaced = session.send(code_then("0233", "й"));
        let other_echo = session.send(code_then("8364", "¬"));
        match os {
            Os::Windows => {
                assert_eq!(replaced, typed("й"), "Windows: its own character for a code up to 255 replaces ours");
                assert_eq!(other_echo, typed("€"), "Windows: above 255 ours stays and the composed byte is dropped");
            }
            _ => {
                assert_eq!(replaced, typed("éй"), "{os:?}: a different text after a code is typed as normal");
                assert_eq!(other_echo, typed("€¬"), "{os:?}: a different text after a code is typed as normal");
            }
        }

        // Focus going to another window ends the code: its Alt release types nothing and is no lone Alt.
        let away = vec![vec![alt(true)], digit('0'), digit('2'), vec![Event::WindowFocused(false)], vec![Event::WindowFocused(true), alt(false)]];
        assert_eq!(session.send(away), typed(""), "{os:?}: a focus loss drops the code");
        session.focus();
        assert_eq!(session.send(code("0233")), typed("é"), "{os:?}: the next code types again");

        // The Alt release of a code that lost focus went to another window: this window's next lone Alt still alerts.
        let stuck = vec![vec![alt(true)], digit('0'), vec![Event::WindowFocused(false)], vec![Event::WindowFocused(true)]];
        session.send(stuck);
        let lone = session.send(vec![vec![alt(true)], vec![alt(false)]]);
        assert_eq!(lone, Typed { text: String::new(), lone_alt: true }, "{os:?}: a code whose release went elsewhere leaves no mark");

        let mut cyrillic = Session::new(os, AltCodePages::new(1251, 866));
        assert_eq!(cyrillic.send(code("0233")), typed("й"), "{os:?}: the app's ANSI page decodes the code");
        assert_eq!(cyrillic.send(code("233")), typed("щ"), "{os:?}: the app's OEM page decodes the code");
    }

    let mut mac = Session::new(Os::Mac, western);
    let option = mac.send(code("0233"));
    assert!(!option.text.contains('é'), "macOS has no Alt codes: {option:?}");
    assert!(!option.lone_alt, "macOS has no lone Alt either");
}

/// Alt with the digit 3, which the host canvas of the next trial claims.
const ALT_3: craft_keymap::Chord = craft_keymap::Chord::new(craft_keymap::Mods::ALT, craft_keymap::Key::Num3);

/// An app's Alt+digit shortcut on Windows and Linux: the app's own egui canvas (`host_canvas`) claims Alt+3. With
/// nothing focused the canvas is the window's key target, so Alt-code entry leaves the claimed press and egui's input
/// keeps it for the app, while an unclaimed Alt+5 still folds into a code. With the canvas focused, the claimed press
/// still reaches egui. A focused text field types Alt codes again, with the claimed digit among them.
#[trial]
fn an_alt_digit_a_host_canvas_claims_reaches_the_app_and_a_field_still_types_codes() {
    for os in [Os::Windows, Os::Linux] {
        let canvas = named_id("alt digit canvas");
        let typed = Rc::new(RefCell::new(String::new()));
        let reached = Rc::new(RefCell::new(Vec::<Key>::new()));
        let mut show = {
            let (typed, reached) = (Rc::clone(&typed), Rc::clone(&reached));
            move |ui: &mut egui::Ui| {
                let alt_digits: Vec<Key> = ui.input(|input| {
                    input
                        .events
                        .iter()
                        .filter_map(|event| match event {
                            Event::Key { key, pressed: true, modifiers, .. } if modifiers.alt => Some(*key),
                            _ => None,
                        })
                        .collect()
                });
                reached.borrow_mut().extend(alt_digits);
                widget(ui, canvas);
                host_canvas(ui.ctx(), canvas, KeyUse::CANVAS.with_claims(&[ALT_3]));
                Probe::new(&typed).label("Text").id_salt("alt_code_field").show_at(ui, AT);
            }
        };
        let mut frames = Frames::new();
        craft_ui::testing::windows::set_os(&frames.ctx, os);
        frames.root(RootEnv::default().alt_code_pages(AltCodePages::new(1252, 437)));
        frames.idle(3, &mut show);
        // Plays `sequence`, then a frame with none; returns the Alt presses that reached egui and the field's text.
        let send = |frames: &mut Frames, show: &mut dyn FnMut(&mut egui::Ui), sequence: Vec<Vec<Event>>| {
            reached.borrow_mut().clear();
            typed.borrow_mut().clear();
            for events in sequence {
                frames.frame(events, &mut *show);
            }
            frames.frame(Vec::new(), &mut *show);
            (reached.take(), typed.take())
        };

        let (keys, _) = send(&mut frames, &mut show, code("3"));
        assert!(keys.contains(&Key::Num3), "{os:?}: with nothing focused, the canvas's claimed Alt+3 reaches egui: {keys:?}");
        let (keys, _) = send(&mut frames, &mut show, code("5"));
        assert!(!keys.contains(&Key::Num5), "{os:?}: an unclaimed Alt+5 folds into an Alt code: {keys:?}");

        request(&frames.ctx, FocusTarget::Id(canvas));
        let (keys, _) = send(&mut frames, &mut show, code("3"));
        assert!(keys.contains(&Key::Num3), "{os:?}: the focused canvas's claimed Alt+3 reaches egui: {keys:?}");

        click(&mut frames, AT.center(), &mut show);
        frames.idle(1, &mut show);
        let (keys, text) = send(&mut frames, &mut show, code("0233"));
        assert_eq!(text, "é", "{os:?}: the focused field types the code, its 3 included");
        assert!(keys.is_empty(), "{os:?}: no digit of the code reaches egui: {keys:?}");
    }
}
