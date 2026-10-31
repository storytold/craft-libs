use craft_keymap::chord::foreign::{Flavor, ForeignError, ReadNote, read_foreign};
use craft_keymap::{Chord, Key, MAX_CHORD_TEXT, Mods, ParseError, Platform, Trigger};

fn chord(text: &str) -> Chord {
    Chord::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

fn mods(cmd: bool, ctrl: bool, alt: bool, shift: bool) -> Mods {
    Mods { cmd, ctrl, alt, shift }
}

#[test]
fn canonical_text_round_trips_for_every_key_and_modifier_set() {
    for &key in Key::ALL {
        for bits in 0..16u8 {
            let c = Chord::new(mods(bits & 1 != 0, bits & 2 != 0, bits & 4 != 0, bits & 8 != 0), key);
            let text = c.to_string();
            assert_eq!(Chord::parse(&text), Ok(c), "{text}");
            assert_eq!(Trigger::parse(&format!("{text} {text}")), Ok(Trigger::double(c)), "{text} double");
            let json = serde_json::to_string(&Trigger::single(c)).unwrap();
            assert_eq!(serde_json::from_str::<Trigger>(&json).unwrap(), Trigger::single(c));
            // macOS display text reads back as the same chord.
            assert_eq!(Chord::parse(&c.display(Platform::Mac)), Ok(c), "{}", c.display(Platform::Mac));
        }
    }
    assert_eq!(chord("Cmd+Ctrl+Alt+Shift+K").to_string(), "Cmd+Ctrl+Alt+Shift+K");
    assert_eq!(serde_json::to_string(&chord("shift+cmd+k")).unwrap(), "\"Cmd+Shift+K\"");
}

#[test]
fn plus_and_minus_keys_parse_from_their_glyphs() {
    assert_eq!(Key::parse("+"), Some(Key::Plus));
    assert_eq!(Key::parse("-"), Some(Key::Minus));
    assert_eq!(Key::parse("−"), Some(Key::Minus), "U+2212");
}

#[test]
fn arrow_glyphs_parse_as_arrow_keys() {
    for (glyph, key) in [("←", Key::Left), ("→", Key::Right), ("↑", Key::Up), ("↓", Key::Down)] {
        assert_eq!(Key::parse(glyph), Some(key), "{glyph}");
    }
    assert_eq!(Chord::parse("⌘←").unwrap().to_string(), "Cmd+Left");
    assert_eq!(Chord::parse("Alt+Shift+→").unwrap().to_string(), "Alt+Shift+Right");
}

#[test]
fn aliases_parse_to_canonical_chords() {
    let cases = [
        ("cmd+shift+z", "Cmd+Shift+Z"),
        ("Shift+Command+Z", "Cmd+Shift+Z"),
        ("meta+z", "Cmd+Z"),
        ("Super+Z", "Cmd+Z"),
        ("mod+z", "Cmd+Z"),
        ("⌘⇧Z", "Cmd+Shift+Z"),
        ("⌃⌥⇧⌘K", "Cmd+Ctrl+Alt+Shift+K"),
        ("⌘+K", "Cmd+K"),
        ("⌘Shift+K", "Cmd+Shift+K"),
        ("control+opt+left", "Ctrl+Alt+Left"),
        ("ctl+option+ArrowUp", "Ctrl+Alt+Up"),
        // craft-menu writes Ctrl before Cmd.
        ("Ctrl+Cmd+Alt+Shift+M", "Cmd+Ctrl+Alt+Shift+M"),
        ("Ctrl+Cmd+F", "Cmd+Ctrl+F"),
        ("Cmd++", "Cmd++"),
        ("⌘+", "Cmd++"),
        ("+", "+"),
        ("Cmd+Plus", "Cmd++"),
        ("cmd+Equals", "Cmd+="),
        ("Cmd+-", "Cmd+-"),
        ("Cmd+−", "Cmd+-"),
        ("⌘−", "Cmd+-"),
        ("Cmd + K", "Cmd+K"),
        ("esc", "Escape"),
        ("return", "Enter"),
        ("del", "Delete"),
        ("pgup", "PageUp"),
        ("pgdn", "PageDown"),
        ("arrowdown", "Down"),
        ("spacebar", "Space"),
        ("ins", "Insert"),
        ("f12", "F12"),
        ("Num5", "5"),
        ("Semicolon", ";"),
        ("Questionmark", "?"),
        ("OpenCurlyBracket", "{"),
        ("Backtick", "`"),
        ("⌘⌫", "Cmd+Backspace"),
    ];
    for (text, canonical) in cases {
        assert_eq!(chord(text).to_string(), canonical, "{text}");
    }
}

#[test]
fn hostile_text_is_refused() {
    let long = "Cmd+".repeat(MAX_CHORD_TEXT / 4) + "K";
    assert!(long.len() > MAX_CHORD_TEXT);
    let cases: &[(&str, ParseError)] = &[
        ("", ParseError::Empty),
        ("   ", ParseError::Empty),
        (&long, ParseError::TooLong),
        ("+++", ParseError::UnknownModifier("+".into())),
        ("++", ParseError::MissingKey),
        ("Cmd+", ParseError::MissingKey),
        ("Cmd", ParseError::MissingKey),
        ("⌘", ParseError::MissingKey),
        ("Shift+Alt", ParseError::MissingKey),
        ("Cmd+Command+K", ParseError::DuplicateModifier("Command".into())),
        ("⌘⌘K", ParseError::DuplicateModifier("⌘".into())),
        ("Cmd+é", ParseError::UnknownKey("é".into())),
        ("Ctrl+ж+K", ParseError::UnknownModifier("ж".into())),
        ("Hyper+K", ParseError::UnknownModifier("Hyper".into())),
        ("A B", ParseError::Sequence),
        ("Cmd+K Cmd+J", ParseError::Sequence),
        ("A A A", ParseError::UnknownKey("A A A".into())),
    ];
    for (text, err) in cases {
        assert_eq!(Trigger::parse(text).as_ref(), Err(err), "{text:?}");
    }
    let wide = "⌘".repeat(30);
    assert_eq!(Chord::parse(&wide), Err(ParseError::TooLong));
    assert!(serde_json::from_str::<Chord>("\"Cmd+Nope\"").is_err());
}

#[test]
fn display_follows_the_platform() {
    let c = chord("Cmd+Ctrl+Alt+Shift+K");
    assert_eq!(c.display(Platform::Mac), "⌃⌥⇧⌘K");
    assert_eq!(c.display(Platform::Windows), "Ctrl+Alt+Shift+K");
    assert_eq!(chord("Ctrl+Up").display(Platform::Mac), "⌃↑");
    assert_eq!(chord("Ctrl+Up").display(Platform::Linux), "Ctrl+Up");
    assert_eq!(chord("Cmd+Enter").display(Platform::Mac), "⌘⏎");
    assert_eq!(chord("Shift+Tab").display(Platform::Mac), "⇧⇥");
    assert_eq!(chord("Space").display(Platform::Mac), "Space");
    assert_eq!(chord("Alt+PageDown").display(Platform::Windows), "Alt+Page Down");
    assert_eq!(Trigger::parse("Cmd+K Cmd+K").unwrap().display(Platform::Mac), "⌘K ⌘K");
    assert_eq!(Trigger::parse("A A").unwrap().to_string(), "A A");
}

#[test]
fn ctrl_folds_into_cmd_off_macos() {
    let ctrl = chord("Ctrl+S");
    let cmd = chord("Cmd+S");
    assert_ne!(ctrl.effective(Platform::Mac), cmd.effective(Platform::Mac));
    for p in [Platform::Windows, Platform::Linux] {
        assert_eq!(ctrl.effective(p), cmd);
        assert_eq!(chord("Cmd+Ctrl+S").effective(p), cmd);
    }
    assert!(ctrl.has_command() && cmd.has_command());
    assert!(chord("Shift+B").is_plain() && !chord("Alt+B").is_plain());
    assert!(chord("Shift+F35").is_function_key() && !chord("F").is_function_key());
}

#[test]
fn foreign_windows_text() {
    let read = |t: &str| read_foreign(t, Flavor::Windows);
    let ok = |t: &str| read(t).unwrap_or_else(|e| panic!("{t}: {e}"));
    assert_eq!(ok("Shift+Ctrl+S"), (Trigger::single(chord("Cmd+Shift+S")), vec![]));
    assert_eq!(ok("Ctrl+,").0, Trigger::single(chord("Cmd+,")));
    assert_eq!(ok(",").0, Trigger::single(chord(",")));
    assert_eq!(ok("Ctrl++").0, Trigger::single(chord("Cmd++")));
    assert_eq!(ok("Ctrl+-").0, Trigger::single(chord("Cmd+-")));
    assert_eq!(ok("Alt-F4").0, Trigger::single(chord("Alt+F4")));
    assert_eq!(ok("Ctrl + S").0, Trigger::single(chord("Cmd+S")));
    assert_eq!(ok("Shift + Ctrl + S").0, Trigger::single(chord("Cmd+Shift+S")));
    assert_eq!(ok("Ctrl + +").0, Trigger::single(chord("Cmd++")));
    assert_eq!(ok("Alt - F4").0, Trigger::single(chord("Alt+F4")));
    assert_eq!(ok("Ctrl Page Down").0, Trigger::single(chord("Cmd+PageDown")));
    assert_eq!(ok("Shift+Up Arrow").0, Trigger::single(chord("Shift+Up")));
    assert_eq!(ok("Num +"), (Trigger::single(chord("+")), vec![ReadNote::Numpad]));
    assert_eq!(ok("Ctrl+Num /"), (Trigger::single(chord("Cmd+/")), vec![ReadNote::Numpad]));
    assert_eq!(ok("Numpad 5"), (Trigger::single(chord("5")), vec![ReadNote::Numpad]));
    assert_eq!(ok("Num Enter"), (Trigger::single(chord("Enter")), vec![ReadNote::Numpad]));
    assert_eq!(read("Win+E"), Err(ForeignError::UnsupportedModifier("Win".into())));
    assert_eq!(read("Meta+E"), Err(ForeignError::UnsupportedModifier("Meta".into())));
    assert_eq!(read(""), Err(ForeignError::Empty));
    assert_eq!(read(&"x".repeat(MAX_CHORD_TEXT + 1)), Err(ForeignError::TooLong));
    assert_eq!(read("Ctrl+Num Lock"), Err(ForeignError::UnknownKey("Num Lock".into())));
    assert_eq!(read("Alt"), Err(ForeignError::UnknownKey("Alt".into())));
}

#[test]
fn foreign_mac_text() {
    let texts = ["Shift+Cmd+S", "Ctrl+Opt+Left"];
    assert_eq!(Flavor::detect(texts), Flavor::Mac);
    assert_eq!(Flavor::detect(["Shift+Ctrl+S", "Alt+F4"]), Flavor::Windows);
    assert_eq!(Flavor::detect(["⌥⌘K"]), Flavor::Mac);
    let ok = |t: &str| read_foreign(t, Flavor::Mac).unwrap_or_else(|e| panic!("{t}: {e}")).0;
    assert_eq!(ok("Shift+Cmd+S"), Trigger::single(chord("Cmd+Shift+S")));
    assert_eq!(ok("Ctrl+Opt+Left"), Trigger::single(chord("Ctrl+Alt+Left")));
    assert_eq!(ok("Control-Option-Space"), Trigger::single(chord("Ctrl+Alt+Space")));
    assert_eq!(ok("⇧⌘S"), Trigger::single(chord("Cmd+Shift+S")));
    assert_eq!(ok("⌃⌥⇞"), Trigger::single(chord("Ctrl+Alt+PageUp")));
    assert_eq!(ok("Cmd+,"), Trigger::single(chord("Cmd+,")));
    // In a Windows set the same ⌃ is the command modifier.
    assert_eq!(read_foreign("⌃S", Flavor::Windows).map(|r| r.0), Ok(Trigger::single(chord("Cmd+S"))));
}
