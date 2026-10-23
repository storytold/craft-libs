use super::*;
use egui::{Context, Event, IMEPurpose, ImeEvent, Key, Modifiers, PointerButton, RawInput, Rect, output::IMEOutput, pos2, vec2};

fn preedit(text: &str, active: Option<std::ops::Range<usize>>) -> Event {
    Event::Ime(ImeEvent::Preedit { text: text.into(), active_range_chars: active })
}

fn commit(text: &str) -> Event {
    Event::Ime(ImeEvent::Commit(text.into()))
}

fn key_with(key: Key, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
}

fn key(key: Key) -> Event {
    key_with(key, Modifiers::NONE)
}

fn text(t: &str) -> Event {
    Event::Text(t.into())
}

fn click() -> Event {
    Event::PointerButton { pos: pos2(5.0, 5.0), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }
}

/// Feeds passes of `(time, events)` and returns the text the editor ends with and every key it
/// was given. Backspace deletes the last char; other keys are only recorded.
fn type_passes(composer: &mut Composer, passes: &[(f64, Vec<Event>)]) -> (String, Vec<Key>) {
    let mut doc = String::new();
    let mut keys = Vec::new();
    for (time, events) in passes {
        for edit in composer.process(events, *time).edits {
            match edit {
                Edit::Insert(t) => doc.push_str(&t),
                Edit::Key { key: Key::Backspace, .. } => {
                    doc.pop();
                    keys.push(Key::Backspace);
                }
                Edit::Key { key, .. } => keys.push(key),
                Edit::LineBreak { .. } => keys.push(Key::Enter),
                _ => {}
            }
        }
    }
    (doc, keys)
}

fn inserts(edits: &[Edit]) -> Vec<&str> {
    edits.iter().filter_map(|e| if let Edit::Insert(t) = e { Some(t.as_str()) } else { None }).collect()
}

#[test]
fn pinyin_on_wayland_commits_once_and_ignores_trailing_empty_preedits() {
    let mut c = Composer::new();
    let (doc, keys) = type_passes(
        &mut c,
        &[
            (0.0, vec![preedit("c", Some(1..1))]),
            (0.1, vec![preedit("ce", Some(2..2))]),
            (0.2, vec![preedit("ce shi", Some(6..6))]),
            // Space picks the first candidate (fcitx5 on GNOME Wayland).
            (0.3, vec![preedit("", None), commit("测试"), preedit("", Some(0..0)), preedit("", None)]),
            (0.4, vec![preedit("", None)]),
            (1.0, vec![preedit("", None), key(Key::Enter)]),
        ],
    );
    assert_eq!(doc, "测试");
    assert_eq!(keys, [Key::Enter], "an Enter long after the commit is the user's");
    assert!(!c.is_composing());
}

#[test]
fn pinyin_on_windows_keeps_candidate_keys_for_the_ime() {
    let mut c = Composer::new();
    let (doc, keys) = type_passes(
        &mut c,
        &[
            (0.0, vec![preedit("ni", Some(2..2))]),
            // Picking candidate 2: the key and its text are the IME's.
            (0.1, vec![key(Key::Num2), text("2"), preedit("你", Some(1..1))]),
            #[expect(deprecated)]
            (0.2, vec![preedit("你", Some(1..1)), preedit("", None), commit("你"), Event::Ime(ImeEvent::Disabled)]),
        ],
    );
    assert_eq!(doc, "你");
    assert!(keys.is_empty());
}

#[test]
fn korean_two_set_splits_syllables_and_types_one_space() {
    let mut c = Composer::new();
    let (doc, keys) = type_passes(
        &mut c,
        &[
            // g k s: ㅎ 하 한, the first key composing like the rest.
            (0.0, vec![preedit("ㅎ", None)]),
            (0.1, vec![preedit("하", None)]),
            (0.2, vec![preedit("한", None)]),
            // k moves ㄴ to the next syllable: 하 is committed, 나 composes.
            (0.3, vec![commit("하"), preedit("나", None)]),
            // Space after a syllable: commit, then the space itself.
            (0.4, vec![commit("나"), key(Key::Space), text(" ")]),
            // Apple Korean's Space with no syllable: committed and also sent as the key's text.
            (0.8, vec![preedit(" ", None), commit(" "), preedit("", None), key(Key::Space), text(" ")]),
            // 한 then 5: the digit after the syllable is kept.
            (1.2, vec![preedit("한", None)]),
            (1.3, vec![commit("한"), key(Key::Num5), text("5")]),
        ],
    );
    assert_eq!(doc, "하나  한5");
    assert_eq!(keys, [Key::Num5]);
}

#[test]
fn korean_backspace_that_empties_the_syllable_deletes_nothing_else() {
    let mut c = Composer::new();
    let (doc, keys) = type_passes(
        &mut c,
        &[
            (0.0, vec![text("a")]),
            (0.1, vec![preedit("ㅎ", None)]),
            (0.2, vec![preedit("", None), key(Key::Backspace)]),
            (0.6, vec![key(Key::Backspace)]),
        ],
    );
    assert_eq!(doc, "", "only the second Backspace reaches the text");
    assert_eq!(keys, [Key::Backspace]);
}

#[test]
fn a_raw_first_jamo_is_typed_not_lost() {
    // Unpatched macOS sends the first key in a fresh app as a raw jamo (fixed in the shared
    // winit); it is still typed.
    let mut c = Composer::new();
    let (doc, _) = type_passes(&mut c, &[(0.0, vec![key(Key::G), text("ㅎ")]), (0.1, vec![preedit("ㅏ", None)])]);
    assert_eq!(doc, "ㅎ");
    assert_eq!(c.preedit().map(|p| p.text), Some("ㅏ"));
}

#[test]
fn japanese_conversion_tracks_the_segment_and_guards_the_confirming_enter() {
    let mut c = Composer::new();
    c.process(&[preedit("にほんご", Some(4..4))], 0.0);
    // Space converts: the key is the IME's.
    let p = c.process(&[key(Key::Space), preedit("日本語", Some(0..3))], 0.1);
    assert!(p.events.is_empty(), "the converting Space is removed from the app's input");
    assert_eq!(c.preedit().and_then(|p| p.active.cloned()), Some(0..3));
    // Moving to the next segment.
    c.process(&[key(Key::ArrowRight), preedit("日本語", Some(2..3))], 0.2);
    assert_eq!(c.preedit().and_then(|p| p.active.cloned()), Some(2..3));
    // An out-of-range segment is dropped rather than trusted.
    c.process(&[preedit("日本語", Some(2..9))], 0.25);
    assert_eq!(c.preedit().map(|p| p.active), Some(None));
    let p = c.process(&[commit("日本語"), key(Key::Enter)], 0.3);
    assert_eq!(inserts(&p.edits), ["日本語"]);
    assert!(!p.edits.iter().any(|e| matches!(e, Edit::Key { .. })), "the confirming Enter doesn't submit");
    // The next Enter is the user's.
    let p = c.process(&[key(Key::Enter)], 0.9);
    assert_eq!(p.edits, [Edit::Key { key: Key::Enter, modifiers: Modifiers::NONE, repeat: false }]);
    // An Enter forwarded in the pass after the commit is still the IME's.
    c.process(&[preedit("か", None)], 1.0);
    c.process(&[commit("課")], 1.1);
    let p = c.process(&[key(Key::Enter)], 1.11);
    assert!(p.edits.is_empty());
}

#[test]
fn a_bare_line_break_commit_keeps_the_marked_text() {
    let mut c = Composer::new();
    let (doc, keys) = type_passes(&mut c, &[(0.0, vec![preedit("かな", None)]), (0.1, vec![commit("\n"), key(Key::Enter)])]);
    assert_eq!(doc, "かな");
    assert!(keys.is_empty());
    assert!(c.interrupted(), "the platform's input context may still hold the marked text");
    // Interrupting the IME can commit the same text again.
    let p = c.process(&[commit("かな")], 0.15);
    assert!(inserts(&p.edits).is_empty());
    assert!(!c.interrupted());
}

#[test]
fn vietnamese_telex_keeps_backspace_between_its_inserts() {
    // Unikey without preedit: "toaf" replaces the a with à by Backspace and text.
    let mut c = Composer::new();
    let p = c.process(&[text("t"), text("o"), text("a"), key(Key::Backspace), text("à")], 0.0);
    assert_eq!(
        p.edits,
        [
            Edit::Insert("t".into()),
            Edit::Insert("o".into()),
            Edit::Insert("a".into()),
            Edit::Key { key: Key::Backspace, modifiers: Modifiers::NONE, repeat: false },
            Edit::Insert("à".into()),
        ]
    );
    // IBus-Unikey with preedit: the word composes and Space commits it, then types the space.
    let (doc, _) = type_passes(
        &mut c,
        &[
            (1.0, vec![preedit("t", None)]),
            (1.1, vec![preedit("to", None)]),
            (1.2, vec![preedit("toa", None)]),
            (1.3, vec![preedit("toà", None)]),
            (1.4, vec![preedit("", None), commit("toà"), key(Key::Space), text(" ")]),
        ],
    );
    assert_eq!(doc, "toà ");
}

#[test]
fn commits_without_a_composition_leave_the_next_keys_to_the_user() {
    // IBus and fcitx send plain letters as commits.
    let mut c = Composer::new();
    let (doc, keys) = type_passes(&mut c, &[(0.0, vec![commit("a"), commit("b"), key(Key::Backspace)])]);
    assert_eq!(doc, "a");
    assert_eq!(keys, [Key::Backspace]);
    for k in [Key::Enter, Key::Escape, Key::Delete, Key::Space] {
        let (_, keys) = type_passes(&mut c, &[(1.0, vec![commit("x"), key(k)])]);
        assert_eq!(keys, [k], "{k:?} after a bare commit is the user's");
    }
    // Telex through IBus without preedit: Backspace replaces the a.
    let (doc, _) = type_passes(&mut c, &[(2.0, vec![commit("t"), commit("o"), commit("a"), key(Key::Backspace), commit("à")])]);
    assert_eq!(doc, "toà");
    // A bare commit of a space guards only the Space key that typed it.
    let (doc, keys) = type_passes(&mut c, &[(3.0, vec![commit(" "), key(Key::Space), text(" ")]), (3.01, vec![commit(" "), key(Key::Backspace)])]);
    assert_eq!(doc, " ");
    assert_eq!(keys, [Key::Backspace]);
}

#[test]
fn only_text_right_after_a_commit_is_its_echo() {
    let typed = |passes: &[(f64, Vec<Event>)]| type_passes(&mut Composer::new(), passes).0;
    // A key the editor gets after a commit types its own text.
    assert_eq!(typed(&[(0.0, vec![commit("a"), key(Key::A), text("a")])]), "aa");
    assert_eq!(typed(&[(0.0, vec![commit("a"), text("a")])]), "a");
    assert_eq!(typed(&[(0.0, vec![commit("あ"), text("あ")])]), "あ");
    // The echo comes in the commit's pass; text in a later pass is typed.
    assert_eq!(typed(&[(0.0, vec![commit("a")]), (0.01, vec![text("a")])]), "aa");
    assert_eq!(typed(&[(0.0, vec![key(Key::A), text("a")])]), "a");
}

#[test]
fn a_line_break_with_nothing_composing_is_enter_once() {
    let line_break = |modifiers| Edit::LineBreak { modifiers };
    let enter = Edit::Key { key: Key::Enter, modifiers: Modifiers::NONE, repeat: false };
    let mut c = Composer::new();
    assert_eq!(c.process(&[commit("\n")], 0.0).edits, [line_break(Modifiers::NONE)]);
    assert_eq!(c.process(&[text("\n")], 1.0).edits, [line_break(Modifiers::NONE)]);
    // The key and its text, in either order, are one Enter: the key when it came first.
    let p = c.process(&[key(Key::Enter), text("\r")], 2.0);
    assert_eq!(p.edits, [enter]);
    assert_eq!(p.events, [key(Key::Enter), text("\r")], "the pressed Enter stays in the input");
    assert_eq!(c.process(&[commit("\n"), key(Key::Enter), text("\n")], 3.0).edits, [line_break(Modifiers::NONE)]);
    // Shift+Enter stays Shift+Enter.
    let p = c.process(&[Event::ModifiersChanged(Modifiers::SHIFT), commit("\r")], 4.0);
    assert_eq!(p.edits, [line_break(Modifiers::SHIFT)]);
}

#[test]
fn dead_keys_compose_or_arrive_as_text() {
    let mut c = Composer::new();
    // macOS: ⌥E marks the accent, the next key commits the letter.
    let (doc, _) = type_passes(&mut c, &[(0.0, vec![preedit("´", None)]), (0.1, vec![preedit("", None), commit("í")])]);
    assert_eq!(doc, "í");
    // Windows, X11 and Wayland compose in the keyboard layout: only the result arrives.
    let (doc, _) = type_passes(&mut c, &[(1.0, vec![text("é")])]);
    assert_eq!(doc, "é");
    // Escape cancels a marked accent without reaching the editor.
    let (doc, keys) = type_passes(&mut c, &[(2.0, vec![preedit("¨", None)]), (2.1, vec![preedit("", None), key(Key::Escape)])]);
    assert_eq!(doc, "");
    assert!(keys.is_empty());
}

#[test]
fn a_click_while_composing_keeps_the_text_and_drops_the_late_commit() {
    let mut c = Composer::new();
    c.process(&[preedit("ce", None)], 0.0);
    let p = c.process(&[click()], 0.1);
    assert_eq!(p.edits, [Edit::Preedit { text: String::new(), active: None }, Edit::Insert("ce".into())]);
    assert_eq!(p.events, [click()], "the app still handles the click");
    // Windows commits the dropped composition when the IME is interrupted.
    let p = c.process(&[preedit("", Some(0..0)), commit("ce")], 0.15);
    assert!(inserts(&p.edits).is_empty());
    // The same text typed later is typed.
    let p = c.process(&[commit("ce")], 2.0);
    assert_eq!(inserts(&p.edits), ["ce"]);
}

#[test]
fn shortcut_chords_dont_type_but_altgr_does() {
    let mut c = Composer::new();
    let p = c.process(&[key_with(Key::S, Modifiers::COMMAND), text("s")], 0.0);
    assert!(inserts(&p.edits).is_empty());
    assert!(matches!(p.edits[..], [Edit::Key { key: Key::S, .. }]));
    let altgr = Modifiers { alt: true, ctrl: true, ..Modifiers::NONE };
    let p = c.process(&[key_with(Key::Q, altgr), text("@")], 0.1);
    assert_eq!(inserts(&p.edits), ["@"]);
}

#[test]
fn clipboard_and_surrounding_deletes_are_the_apps_to_apply() {
    let delete = |before_chars, after_chars| Event::Ime(ImeEvent::DeleteSurrounding { before_chars, after_chars });
    let mut c = Composer::new();
    // While composing, they are the IME's: no edits, and gone from the input.
    let p = c.process(&[preedit("ni", None), Event::Paste("x".into()), Event::Copy, Event::Cut, delete(1, 0)], 0.0);
    assert_eq!(p.edits, [Edit::Preedit { text: "ni".into(), active: None }]);
    assert!(p.events.is_empty());
    // After it, in order with the text. The clipboard events stay in the input too.
    let events = [preedit("", None), Event::Copy, text("a"), Event::Cut, Event::Paste("x".into()), delete(2, 1)];
    let p = c.process(&events, 0.1);
    assert_eq!(
        p.edits,
        [
            Edit::Preedit { text: String::new(), active: None },
            Edit::Copy,
            Edit::Insert("a".into()),
            Edit::Cut,
            Edit::Paste("x".into()),
            Edit::DeleteSurrounding { before_chars: 2, after_chars: 1 },
        ]
    );
    assert_eq!(p.events, [Event::Copy, text("a"), Event::Cut, Event::Paste("x".into())]);
}

/// The suite's egui fork's `ImeEvent::DeleteSurroundingBytes`, read from JSON; `None` on
/// crates.io egui, which has no such event, so the test builds against both.
fn delete_bytes(before_bytes: usize, after_bytes: usize) -> Option<Event> {
    let known: Result<ImeEvent, _> = serde_json::from_str(r#"{"DeleteSurrounding":{"before_chars":1,"after_chars":0}}"#);
    assert!(known.is_ok(), "egui's events read from JSON: {known:?}");
    let json = format!(r#"{{"DeleteSurroundingBytes":{{"before_bytes":{before_bytes},"after_bytes":{after_bytes}}}}}"#);
    serde_json::from_str(&json).ok().map(Event::Ime)
}

#[test]
fn byte_deletes_come_before_the_commit_that_replaces_them() {
    let (Some(delete_e), Some(delete_jamo)) = (delete_bytes(1, 0), delete_bytes(3, 2)) else {
        return;
    };
    let mut c = Composer::new();
    // macOS press-and-hold: "e" typed, then "é" from the accent menu replaces it.
    c.process(&[text("e")], 0.0);
    let p = c.process(&[delete_e, commit("é")], 1.0);
    assert_eq!(p.edits, [Edit::DeleteSurroundingBytes { before_bytes: 1, after_bytes: 0 }, Edit::Insert("é".into())]);
    assert!(p.events.is_empty());
    // While composing too: the counts start at the caret, where the marked text is.
    c.process(&[preedit("ㅎ", None)], 2.0);
    let p = c.process(&[delete_jamo, commit("하")], 2.1);
    assert_eq!(
        p.edits,
        [
            Edit::DeleteSurroundingBytes { before_bytes: 3, after_bytes: 2 },
            Edit::Preedit { text: String::new(), active: None },
            Edit::Insert("하".into()),
        ]
    );
}

#[test]
fn byte_counts_delete_only_whole_chars() {
    // A combining acute is two bytes; "é" is two bytes, "🇧🇪" two four-byte chars.
    assert_eq!(surrounding_chars("cafe\u{301}", "", 2, 0), (1, 0));
    assert_eq!(surrounding_chars("aé", "", 1, 0), (0, 0));
    assert_eq!(surrounding_chars("aé", "", 3, 0), (2, 0));
    assert_eq!(surrounding_chars("x🇧🇪", "", 4, 0), (1, 0));
    assert_eq!(surrounding_chars("", "éb", 0, 3), (0, 2));
    assert_eq!(surrounding_chars("", "éb", 0, 1), (0, 0));
    assert_eq!(surrounding_chars("ab", "cd", 9, 9), (2, 2));
}

#[test]
fn the_app_ends_a_composition_keeping_or_discarding_the_marked_text() {
    let ctx = Context::default();
    let mut c = Composer::new();
    run(&ctx, 0.0, vec![preedit("かな", None)], |ctx| {
        c.update(ctx, true);
    });
    let mut kept = None;
    let out = run(&ctx, 0.1, vec![], |ctx| {
        // A command run before the composer's update ends the editing.
        kept = c.end(ctx);
        c.update(ctx, true);
    });
    assert_eq!(kept.as_deref(), Some("かな"));
    assert!(c.interrupted());
    assert!(out.platform_output.ime.is_some_and(|i| i.should_interrupt_composition));

    // An editor that already keeps the marked text discards the composer's.
    run(&ctx, 0.2, vec![preedit("ね", None)], |ctx| {
        c.update(ctx, true);
    });
    assert!(!c.interrupted());
    let out = run(&ctx, 0.3, vec![], |ctx| {
        c.update(ctx, true);
        c.discard(ctx);
    });
    assert!(!c.is_composing() && c.interrupted());
    assert!(out.platform_output.ime.is_some_and(|i| i.should_interrupt_composition));
    let mut edits = Vec::new();
    run(&ctx, 0.35, vec![commit("ね")], |ctx| edits = c.update(ctx, true));
    assert!(inserts(&edits).is_empty(), "the IME's late commit is dropped");
}

fn run(ctx: &Context, time: f64, events: Vec<Event>, f: impl FnMut(&Context)) -> egui::FullOutput {
    let raw = RawInput { time: Some(time), events, screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))), ..Default::default() };
    let mut f = f;
    let mut out = ctx.run_ui(raw, |ui| f(ui.ctx()));
    out.textures_delta.clear();
    out
}

#[test]
fn the_ime_is_on_at_the_caret_only_while_editing() {
    let ctx = Context::default();
    let mut c = Composer::new();
    let caret = Rect::from_min_size(pos2(100.0, 50.0), vec2(0.0, 16.0));
    let out = run(&ctx, 0.0, vec![preedit("に", None), key(Key::Space)], |ctx| {
        c.update(ctx, true);
        assert!(ctx.input(|i| i.events.is_empty()), "the IME's events are removed for later handlers");
        c.set_caret(ctx, caret);
    });
    let ime = out.platform_output.ime.expect("IME on while a caret is on the canvas");
    assert_eq!(ime.cursor_rect, Rect::from_min_size(pos2(100.0, 50.0), vec2(1.0, 16.0)));
    assert!(!ime.should_interrupt_composition);

    // A click away keeps the text and interrupts the IME.
    let mut edits = Vec::new();
    let out = run(&ctx, 0.1, vec![click()], |ctx| {
        edits = c.update(ctx, true);
        c.set_caret(ctx, caret);
    });
    assert!(edits.contains(&Edit::Insert("に".into())));
    assert!(out.platform_output.ime.is_some_and(|i| i.should_interrupt_composition));

    // Leaving the canvas keeps marked text and turns the IME off.
    run(&ctx, 0.2, vec![preedit("か", None)], |ctx| {
        c.update(ctx, true);
    });
    let out = run(&ctx, 0.3, vec![], |ctx| edits = c.update(ctx, false));
    assert_eq!(edits, [Edit::Preedit { text: String::new(), active: None }, Edit::Insert("か".into())]);
    assert!(out.platform_output.ime.is_none());
    assert!(c.interrupted(), "the IME may still hold the marked text it dropped");
}

#[test]
fn editing_that_ends_mid_pass_turns_the_ime_off_in_that_pass() {
    let ctx = Context::default();
    let mut c = Composer::new();
    let caret = Rect::from_min_size(pos2(100.0, 50.0), vec2(0.0, 16.0));
    run(&ctx, 0.0, vec![preedit("に", None)], |ctx| {
        c.update(ctx, true);
        c.set_caret(ctx, caret);
    });
    let mut edits = Vec::new();
    let mut interrupted = false;
    let out = run(&ctx, 0.1, vec![click()], |ctx| {
        edits = c.update(ctx, true);
        // The click closed the editor.
        edits.extend(c.update(ctx, false));
        interrupted = c.interrupted();
        c.set_caret(ctx, caret);
    });
    assert_eq!(inserts(&edits), ["に"]);
    assert!(interrupted, "the second update keeps the first one's interrupt");
    assert!(out.platform_output.ime.is_none());
    // A pass in which the composer wrote nothing leaves another widget's IME output alone.
    let other = IMEOutput { purpose: IMEPurpose::Normal, rect: caret, cursor_rect: caret, should_interrupt_composition: false };
    let out = run(&ctx, 0.2, vec![], |ctx| {
        ctx.output_mut(|o| o.ime = Some(other));
        c.update(ctx, false);
    });
    assert_eq!(out.platform_output.ime, Some(other));
}

#[test]
fn a_bare_line_break_commit_interrupts_the_ime() {
    let ctx = Context::default();
    let mut c = Composer::new();
    run(&ctx, 0.0, vec![preedit("かな", None)], |ctx| {
        c.update(ctx, true);
    });
    let mut edits = Vec::new();
    let out = run(&ctx, 0.1, vec![commit("\n")], |ctx| edits = c.update(ctx, true));
    assert_eq!(inserts(&edits), ["かな"]);
    assert!(out.platform_output.ime.is_some_and(|i| i.should_interrupt_composition));
}

#[test]
fn update_starts_from_the_modifiers_held() {
    // Cmd went down while another view had the keyboard; its ModifiersChanged went there.
    let ctx = Context::default();
    let mut c = Composer::new();
    run(&ctx, 0.0, vec![Event::ModifiersChanged(Modifiers::COMMAND)], |_| {});
    let mut edits = Vec::new();
    run(&ctx, 0.1, vec![text("s")], |ctx| edits = c.update(ctx, true));
    assert!(edits.is_empty(), "Cmd+S doesn't type an s");
}

#[test]
fn a_caret_off_the_canvas_keeps_the_candidates_on_screen() {
    let ctx = Context::default();
    let mut c = Composer::new();
    let out = run(&ctx, 0.0, vec![], |ctx| {
        c.update(ctx, true);
        c.set_caret(ctx, Rect::from_min_size(pos2(-50.0, 900.0), vec2(2.0, 20.0)));
    });
    let r = out.platform_output.ime.map(|i| i.cursor_rect);
    assert_eq!(r, Some(Rect::from_min_size(pos2(0.0, 580.0), vec2(2.0, 20.0))));
}
