//! Keytips: a lone Alt tap or F10 shows them, a letter a badge takes is claimed and never types, a second letter
//! narrows the badges, the badges sit on the controls of the pass's own record and stay off in a text field or under
//! a modal, badges never start one another, and a host sequence waits, ends on Escape and fires once.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use craft_host::target::Os;
use craft_i18n::Folder;
use craft_keymap::mnemonic::assign_keytips;
use craft_menu::{KeytipControl, Keytips, KeytipsOutput};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, CommandSet, Overlay, Props as _, Show as _, ShowContainer as _};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::popover::Popover;
use craft_ui::runtime::commands::{CommandRef, FireSource};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::frame::take_output;
use craft_ui::runtime::inspect::{self, Recorded};
use craft_ui::testing::input::key_reached;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::TextField;
use egui::{Event, Id, Key, Modifiers, Pos2, Rect};

/// The commands of the trial's controls and sequences.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "keytipsTrial")]
#[non_exhaustive]
enum Trial {
    #[command(label = "Bold", modal = "always")]
    Bold,
    #[command(label = "Italic", modal = "always")]
    Italic,
    #[command(label = "Find", modal = "always")]
    Find,
    #[command(label = "Zoom", modal = "always")]
    Zoom,
    #[command(label = "Word Count", modal = "always")]
    WordCount,
}

/// The controls in the order they show, with their fixed badges: `FB` and `FI` share a first letter, so `Find` takes
/// `I`, the first free letter of its label.
const CONTROLS: [(Trial, &str, Option<&str>); 3] = [(Trial::Bold, "Bold", Some("FB")), (Trial::Italic, "Italic", Some("FI")), (Trial::Find, "Find", None)];

/// The letters whose presses the trials follow into egui.
const FOLLOWED: [Key; 7] = [Key::F, Key::H, Key::I, Key::Q, Key::T, Key::V, Key::Z];

/// The id of the tab button named `name`.
fn tab_id(name: &str) -> Id {
    Id::NULL.with(("trial.keytips.tab", name))
}

/// What one pass saw.
#[derive(Clone)]
struct Pass {
    output: KeytipsOutput,
    recorded: Option<Arc<Recorded>>,
    /// The followed letters whose press reached egui in this pass: no one claimed them.
    reached: Vec<Key>,
    /// The frame the pass ran in.
    frame: u64,
}

/// A window with a recorded region (an optional text field, the tabs Home and Insert, the controls) and keytips over
/// it, with two host sequences.
struct Rig {
    fixture: Fixture,
    passes: Rc<RefCell<Vec<Pass>>>,
    modal: Rc<Cell<bool>>,
    rerun: Rc<Cell<bool>>,
}

impl Rig {
    fn new(field: bool) -> Rig {
        let passes = Rc::new(RefCell::new(Vec::<Pass>::new()));
        let (modal, rerun) = (Rc::new(Cell::new(false)), Rc::new(Cell::new(false)));
        let (seen, shown_modal, shown_rerun) = (Rc::clone(&passes), Rc::clone(&modal), Rc::clone(&rerun));
        let mut notes = String::new();
        let fixture = Fixture::new(move |ui| {
            let (_, recorded) = inspect::record(ui, Id::NULL.with("trial.keytips"), |ui| {
                Stack::new().show(ui, |ui, _| {
                    if field {
                        TextField::new("Notes", Bind::bound(&mut notes)).show(ui);
                    }
                    Row::new().show(ui, |ui, _| {
                        Button::new("Home").id(tab_id("Home")).show(ui);
                        Button::new("Insert").id(tab_id("Insert")).show(ui);
                    });
                    Row::new().show(ui, |ui, _| {
                        for (command, _, _) in CONTROLS {
                            Button::command(command).show(ui);
                        }
                    });
                })
            });
            if shown_modal.get() {
                Popover::new("Dialog", Bind::given(true)).overlay(|_| Overlay::modal()).show(ui, |_, _| {});
            }
            let places: Vec<Arc<Recorded>> = recorded.iter().cloned().collect();
            let tabs = [(Label::from("Home"), "H", tab_id("Home")), (Label::from("Insert"), "N", tab_id("Insert"))];
            let controls: Vec<KeytipControl<'_>> =
                CONTROLS.iter().map(|(command, label, letters)| KeytipControl { command: CommandRef::from(*command), label: *label, letters: *letters }).collect();
            let sequences = [("VZ", CommandRef::from(Trial::Zoom)), ("TW", CommandRef::from(Trial::WordCount))];
            let output = Keytips::new(true).tabs(&tabs).controls(&controls).sequences(&sequences).places(&places).show(ui);
            if shown_rerun.take() {
                ui.ctx().request_discard("trial: a second pass in the frame");
            }
            let reached = FOLLOWED.into_iter().filter(|key| key_reached(ui.ctx(), *key)).collect();
            let frame = ui.ctx().cumulative_frame_nr();
            seen.borrow_mut().push(Pass { output, recorded, reached, frame });
        });
        Rig { fixture, passes, modal, rerun }
    }

    /// The passes since the last call.
    fn take(&self) -> Vec<Pass> {
        std::mem::take(&mut *self.passes.borrow_mut())
    }

    /// The last pass.
    fn last(&self) -> Pass {
        let passes = self.passes.borrow();
        passes.last().cloned().unwrap_or_else(|| panic!("the rig ran a pass"))
    }

    /// A key press and release, then a settled window; the passes it ran. They stay the rig's passes until the next
    /// call, so `last` reads the last of them.
    fn key(&mut self, key: Key) -> Vec<Pass> {
        self.take();
        self.fixture.key(key, Modifiers::NONE).settle();
        self.passes.borrow().clone()
    }

    /// Alt pressed and released with nothing between, then a settled window.
    fn alt_tap(&mut self) {
        self.fixture.frame(vec![Event::ModifiersChanged(Modifiers::ALT)]);
        self.fixture.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
        self.fixture.settle();
    }

    /// The badges painted in the latest frame, with where each one's letters lie.
    fn badges(&self) -> Vec<(String, Rect)> {
        const BADGES: [&str; 6] = ["H", "N", "FB", "FI", "I", "F"];
        self.fixture.texts().iter().filter(|text| BADGES.contains(&text.text.as_str())).map(|text| (text.text.clone(), text.rect)).collect()
    }
}

/// The rect of the element of `recorded` named `name`, or bound to `command`.
fn rect_of(recorded: &Recorded, name: Option<&str>, command: Option<Trial>) -> Rect {
    let command = command.map(CommandRef::from);
    let element = recorded.elements.iter().position(|element| {
        element.part.is_none()
            && (name.is_some_and(|name| element.facts.as_ref().and_then(|facts| facts.name.as_deref()) == Some(name))
                || (command.is_some() && element.command == command))
    });
    element.and_then(|index| recorded.stop_rect(index)).unwrap_or_else(|| panic!("{name:?} {command:?} is recorded"))
}

/// Whether the badge whose letters lie at `letters` sits on `control`: its middle is on the control or within half a
/// badge of it.
fn sits_on(letters: Rect, control: Rect) -> bool {
    control.expand2(letters.size()).contains(letters.center()) && letters.center().distance(control.center()) < control.size().length()
}

#[trial]
fn a_lone_alt_tap_toggles_the_keytips_but_a_chord_a_pointer_press_or_altgr_does_not() {
    let mut rig = Rig::new(false);
    // The framework arms the menu key from a lone Alt only off macOS; the trial does not depend on its runner.
    rig.fixture.os(Os::Windows);
    rig.fixture.settle();
    assert!(!rig.last().output.active, "keytips start hidden");

    rig.alt_tap();
    assert!(rig.last().output.active, "a lone Alt tap shows the keytips");
    assert!(rig.badges().iter().any(|(letters, _)| letters == "H"), "the tab badges show: {:?}", rig.badges());
    rig.alt_tap();
    assert!(!rig.last().output.active, "a second tap hides them");

    // Alt held over another key is a chord.
    rig.fixture.key(Key::ArrowDown, Modifiers::ALT).settle();
    assert!(!rig.last().output.active, "Alt with another key is a chord, not a tap");

    // Alt held over a press is part of the press (Alt+drag).
    let at = Pos2::new(1.0, 1.0);
    rig.fixture.frame(vec![Event::ModifiersChanged(Modifiers::ALT), moved(at)]);
    rig.fixture.frame(vec![press(at)]);
    rig.fixture.frame(vec![release(at)]);
    rig.fixture.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
    rig.fixture.settle();
    assert!(!rig.last().output.active, "Alt released after a pointer press is no tap");

    // AltGr is the right Alt key, with Ctrl on Windows: it types characters.
    let altgr = |pressed: bool| Event::Key {
        key: Key::AltRight,
        physical_key: Some(Key::AltRight),
        pressed,
        repeat: false,
        modifiers: if pressed { Modifiers::ALT | Modifiers::CTRL } else { Modifiers::NONE },
    };
    rig.fixture.frame(vec![altgr(true), Event::ModifiersChanged(Modifiers::ALT | Modifiers::CTRL)]);
    rig.fixture.frame(vec![altgr(false), Event::ModifiersChanged(Modifiers::NONE)]);
    rig.fixture.settle();
    assert!(!rig.last().output.active, "AltGr does not show the keytips");

    rig.key(Key::F10);
    assert!(rig.last().output.active, "F10 shows them");
    rig.key(Key::F10);
    assert!(!rig.last().output.active, "and hides them");

    // On macOS Option types characters: only F10 shows the keytips.
    rig.fixture.os(Os::Mac);
    rig.fixture.settle();
    rig.alt_tap();
    assert!(!rig.last().output.active, "a lone Option tap shows nothing on macOS");
    rig.key(Key::F10);
    assert!(rig.last().output.active, "F10 shows them on macOS");
}

#[trial]
fn a_letter_a_badge_takes_never_types_and_a_second_letter_narrows_the_badges() {
    let mut rig = Rig::new(false);
    rig.fixture.settle();
    take_output(rig.fixture.ctx());
    rig.key(Key::F10);
    assert!(rig.last().output.active);

    let passes = rig.key(Key::H);
    assert!(passes.iter().all(|pass| !pass.reached.contains(&Key::H)), "the tab badge's letter never reaches egui");
    assert_eq!(passes.iter().filter_map(|pass| pass.output.tab.clone()).collect::<Vec<_>>(), ["H"], "H picks Home, reported once");
    let mut shown: Vec<String> = rig.badges().into_iter().map(|(letters, _)| letters).collect();
    shown.sort();
    assert_eq!(shown, ["FB", "FI", "I"], "the controls' badges show, and the tabs' are gone");

    // A letter no badge takes is not claimed: it reaches egui, and the keytips wait.
    let passes = rig.key(Key::Q);
    assert!(passes.iter().any(|pass| pass.reached.contains(&Key::Q)), "a letter no badge takes is left to egui");
    assert!(rig.last().output.active && rig.last().output.pending.is_none(), "it changes nothing");

    let passes = rig.key(Key::F);
    assert!(passes.iter().all(|pass| !pass.reached.contains(&Key::F)), "the first letter of a two-letter badge never types");
    assert_eq!(rig.last().output.pending.as_deref(), Some("F"), "it waits for the second letter");
    let mut shown: Vec<String> = rig.badges().into_iter().map(|(letters, _)| letters).collect();
    shown.sort();
    assert_eq!(shown, ["FB", "FI"], "only the badges it can complete show");

    let passes = rig.key(Key::I);
    assert!(passes.iter().all(|pass| !pass.reached.contains(&Key::I)), "the second letter never types");
    assert_eq!(passes.iter().filter_map(|pass| pass.output.fired.clone()).collect::<Vec<_>>(), [CommandRef::from(Trial::Italic)]);
    assert!(!rig.last().output.active, "a complete badge ends the keytips");
    let fired: Vec<(CommandRef, FireSource)> = take_output(rig.fixture.ctx()).fired.into_iter().map(|fired| (fired.command, fired.source)).collect();
    assert_eq!(fired, [(CommandRef::from(Trial::Italic), FireSource::Keytip)], "FI fired Italic, not Find, once");
}

#[trial]
fn badges_sit_on_the_controls_of_this_passes_record_and_stay_off_in_a_text_field_or_under_a_modal() {
    let mut rig = Rig::new(true);
    rig.fixture.settle();
    rig.key(Key::F10);
    let recorded = rig.last().recorded.unwrap_or_else(|| panic!("the region ran"));
    let badges = rig.badges();
    for (letters, name) in [("H", "Home"), ("N", "Insert")] {
        let control = rect_of(&recorded, Some(name), None);
        let at = badges.iter().find(|(shown, _)| shown == letters).map(|(_, rect)| *rect);
        assert!(at.is_some_and(|at| sits_on(at, control)), "{letters} sits on {name} at {control:?}: {badges:?}");
    }

    rig.key(Key::H);
    let recorded = rig.last().recorded.unwrap_or_else(|| panic!("the region ran"));
    let badges = rig.badges();
    let controls: Vec<Rect> = [Trial::Bold, Trial::Italic, Trial::Find].into_iter().map(|command| rect_of(&recorded, None, Some(command))).collect();
    for (letters, own) in [("FB", 0), ("FI", 1), ("I", 2)] {
        let at = badges.iter().find(|(shown, _)| shown == letters).map(|(_, rect)| *rect).unwrap_or_else(|| panic!("{letters} shows: {badges:?}"));
        assert!(sits_on(at, controls[own]), "{letters} sits on its control {:?}: {at:?}", controls[own]);
        let nearest = controls.iter().enumerate().min_by(|a, b| a.1.center().distance(at.center()).total_cmp(&b.1.center().distance(at.center())));
        assert_eq!(nearest.map(|(index, _)| index), Some(own), "{letters} is nearest its own control");
    }

    // Tab moves focus into the text field: its letters are typed there, so the keytips end.
    rig.key(Key::Tab);
    assert!(!rig.last().output.active, "focus in a text field ends the keytips");
    assert!(rig.badges().is_empty(), "and no badge shows");
    rig.key(Key::F10);
    assert!(!rig.last().output.active, "F10 in a text field shows none");

    let mut rig = Rig::new(false);
    rig.fixture.settle();
    rig.key(Key::F10);
    assert!(rig.last().output.active);
    rig.modal.set(true);
    rig.fixture.settle();
    assert!(!rig.last().output.active, "a modal layer ends the keytips");
    assert!(rig.badges().is_empty(), "no badge shows under it");
    rig.key(Key::F10);
    assert!(!rig.last().output.active, "F10 under a modal shows none");
    rig.modal.set(false);
    rig.fixture.settle();
    rig.key(Key::F10);
    assert!(rig.last().output.active, "F10 shows them again once the modal is gone");
}

#[trial]
fn assign_keytips_keeps_given_letters_and_no_badge_is_a_prefix_of_another() {
    let folder = Folder::new("en");
    let distinct = |badges: &[String]| {
        badges.iter().enumerate().all(|(at, badge)| badges.iter().enumerate().all(|(other, them)| at == other || !them.starts_with(badge.as_str())))
    };

    // Given letters stay; the rest come from the labels, never a reserved letter.
    let labels = [("Paste", None), ("Bold", Some("B")), ("Italic", Some("I")), ("Underline", None)];
    let badges = assign_keytips(&labels, &['U'], &folder);
    assert_eq!(&badges[1..3], ["B", "I"], "the app's letters are kept");
    assert_eq!(badges[0], "P", "a label takes its first letter");
    assert!(!badges[3].is_empty() && !badges[3].starts_with('U'), "a reserved letter is never assigned: {badges:?}");
    assert!(distinct(&badges), "{badges:?}");

    // A given badge that would start, or be the start of, an earlier one is not kept as given.
    let labels = [("Format Painter", Some("F")), ("Find", Some("FD")), ("Fill", None)];
    let badges = assign_keytips(&labels, &[], &folder);
    assert_eq!(badges[0], "F");
    assert_ne!(badges[1], "FD", "FD would never be reachable after F");
    assert!(distinct(&badges), "{badges:?}");

    // More controls than letters: the overflow takes two letters, whose first letter no one-letter badge has.
    let names: Vec<String> = (0..40).map(|at| format!("Control {at}")).collect();
    let labels: Vec<(&str, Option<&str>)> = names.iter().map(|name| (name.as_str(), None)).collect();
    let badges = assign_keytips(&labels, &['Q'], &folder);
    assert!(badges.iter().all(|badge| !badge.is_empty()), "every control gets a badge: {badges:?}");
    assert!(badges.iter().any(|badge| badge.len() == 2), "some take two letters: {badges:?}");
    assert!(distinct(&badges), "no badge is the start of another: {badges:?}");
    assert!(badges.iter().filter(|badge| badge.len() == 1).all(|badge| badge != "Q"), "Q stays reserved: {badges:?}");

    // In Turkish the dotless ı takes the I key.
    let badges = assign_keytips(&[("ılık", None)], &[], &Folder::new("tr"));
    assert_eq!(badges, ["I"]);
}

#[trial]
fn a_host_sequence_ignores_a_letter_that_leads_nowhere_ends_on_escape_and_fires_a_complete_sequence_once() {
    let mut rig = Rig::new(false);
    rig.fixture.settle();
    take_output(rig.fixture.ctx());
    rig.key(Key::F10);

    let passes = rig.key(Key::V);
    assert!(passes.iter().all(|pass| !pass.reached.contains(&Key::V)), "a sequence's first letter is claimed");
    assert_eq!(rig.last().output.pending.as_deref(), Some("V"), "the sequence waits");
    assert!(rig.badges().is_empty(), "a sequence shows no badges");

    let passes = rig.key(Key::Q);
    assert!(passes.iter().any(|pass| pass.reached.contains(&Key::Q)), "a letter that leads nowhere is not claimed");
    assert_eq!(rig.last().output.pending.as_deref(), Some("V"), "and the sequence still waits");

    rig.key(Key::Escape);
    assert!(!rig.last().output.active && rig.last().output.pending.is_none(), "Escape ends the sequence");
    let passes = rig.key(Key::Z);
    assert!(passes.iter().any(|pass| pass.reached.contains(&Key::Z)), "after Escape the letter is the window's again");
    assert!(take_output(rig.fixture.ctx()).fired.is_empty(), "nothing fired");

    rig.key(Key::F10);
    rig.key(Key::V);
    // The frame with the last letter runs a second pass: only the pass that received the key applies it.
    rig.rerun.set(true);
    let passes = rig.key(Key::Z);
    let pressed: Vec<&Pass> = passes.iter().filter(|pass| pass.frame == passes[0].frame).collect();
    assert!(pressed.len() >= 2, "the frame of the press ran a second pass");
    assert!(pressed[0].output.fired.is_some(), "the pass that received the key fires the sequence");
    assert_eq!(passes.iter().filter(|pass| pass.output.fired.is_some()).count(), 1, "no other pass reports it again");
    assert!(!rig.last().output.active, "a complete sequence ends the keytips");
    let fired: Vec<(CommandRef, FireSource)> = take_output(rig.fixture.ctx()).fired.into_iter().map(|fired| (fired.command, fired.source)).collect();
    assert_eq!(fired, [(CommandRef::from(Trial::Zoom), FireSource::Keytip)], "VZ fired Zoom once");
}
