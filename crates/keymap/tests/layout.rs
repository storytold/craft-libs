use craft_keymap::{Chord, Key, KeyInput, Layout, Mods, Platform};

fn chord(text: &str) -> Chord {
    Chord::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

/// A press as egui reports it at the US position `physical`: the modifiers held and the logical
/// key, which is the physical key when the character typed has no key (`ф`, `é`, `ข`).
fn at(reported: &str, physical: Key) -> KeyInput {
    let c = chord(reported);
    KeyInput::new(c.mods).with_typed(c.key).with_physical(physical)
}

fn texts(chords: &[Chord]) -> Vec<String> {
    chords.iter().map(Chord::to_string).collect()
}

#[test]
fn each_layout_resolves_presses_by_the_rule() {
    let latin = Layout::new();
    let other = Layout::new().with_latin(false);
    let rows: &[(&str, &Layout, KeyInput, &str)] = &[
        // Russian: letters type Cyrillic, which egui reports as the physical key.
        ("Russian ⌘С", &other, at("Cmd+C", Key::C), "Cmd+C"),
        ("Russian ⌘Х", &other, at("Cmd+[", Key::OpenBracket), "Cmd+["),
        ("Russian / key types .", &other, at("Cmd+.", Key::Slash), "Cmd+."),
        ("Russian ⇧7 types ?", &other, at("Shift+?", Key::Num7), "?"),
        // Arabic 101: ⇧D types ].
        ("Arabic ⌃⇧D", &other, at("Cmd+Shift+]", Key::D), "Cmd+Shift+D"),
        ("Arabic ⌃V", &other, at("Cmd+V", Key::V), "Cmd+V"),
        // Hebrew: Q types /, W types ', the / position types ..
        ("Hebrew ⌃Q", &other, at("Cmd+/", Key::Q), "Cmd+Q"),
        ("Hebrew ⌃W", &other, at("Cmd+'", Key::W), "Cmd+W"),
        ("Hebrew / key types .", &other, at("Cmd+.", Key::Slash), "Cmd+."),
        // Greek: Q types ;.
        ("Greek ⌃Q", &other, at("Cmd+;", Key::Q), "Cmd+Q"),
        ("Greek ⌃Z", &other, at("Cmd+Z", Key::Z), "Cmd+Z"),
        // Dvorak + Russian, as the suite's egui-winit reports them: the Latin layout's character
        // for each key. Dvorak types ' on the Q position, which egui reports as Q or as '.
        ("Dvorak + Russian ⌃С", &other, at("Cmd+J", Key::C).with_latin('j'), "Cmd+J"),
        ("Dvorak + Russian ⌃Й", &other, at("Cmd+Q", Key::Q).with_latin('\''), "Cmd+'"),
        ("Dvorak + Russian ⌃Й as '", &other, at("Cmd+'", Key::Q).with_latin('\''), "Cmd+'"),
        ("Dvorak + Russian / key types .", &other, at("Cmd+.", Key::Slash).with_latin('z'), "Cmd+Z"),
        // Dvorak + Hebrew: the Q position types / and has no Dvorak letter.
        ("Dvorak + Hebrew ⌃Q", &other, at("Cmd+/", Key::Q).with_latin('\''), "Cmd+/"),
        // Thai Kedmanee: 2 types /, - types ข, D types ก.
        ("Thai 2 key types /", &other, at("Cmd+/", Key::Num2), "Cmd+/"),
        ("Thai - key types ข", &other, at("Cmd+-", Key::Minus), "Cmd+-"),
        ("Thai ⌃D", &other, at("Cmd+D", Key::D), "Cmd+D"),
        // AZERTY.
        ("AZERTY A", &latin, at("Cmd+A", Key::Q), "Cmd+A"),
        ("AZERTY M", &latin, at("Cmd+M", Key::Semicolon), "Cmd+M"),
        ("AZERTY M key types ,", &latin, at("Cmd+,", Key::M), "Cmd+,"),
        ("AZERTY & key", &latin, at("Cmd+1", Key::Num1), "Cmd+1"),
        ("AZERTY ⇧& types 1", &latin, at("Cmd+Shift+1", Key::Num1), "Cmd+Shift+1"),
        ("AZERTY 6 key types -", &latin, at("Cmd+-", Key::Num6), "Cmd+-"),
        ("AZERTY ⇧; types .", &latin, at("Shift+.", Key::Comma), "."),
        // Dvorak.
        ("Dvorak O", &latin, at("Cmd+O", Key::S), "Cmd+O"),
        ("Dvorak ⇧O keeps Shift", &latin, at("Cmd+Shift+O", Key::S), "Cmd+Shift+O"),
        ("Dvorak Q key types '", &latin, at("Cmd+'", Key::Q), "Cmd+'"),
        ("Dvorak W key types ,", &latin, at("Cmd+,", Key::W), "Cmd+,"),
        // QWERTZ (German).
        ("QWERTZ ⇧⌘7 types /", &latin, at("Cmd+Shift+/", Key::Num7), "Cmd+/"),
        ("QWERTZ Z", &latin, at("Cmd+Z", Key::Y), "Cmd+Z"),
        ("QWERTZ ⇧ß types ?", &latin, at("Cmd+Shift+?", Key::Minus), "Cmd+?"),
        ("QWERTZ + key", &latin, at("Cmd++", Key::CloseBracket), "Cmd++"),
        ("QWERTZ ⇧, types ;", &latin, at("Cmd+Shift+;", Key::Comma), "Cmd+;"),
        // US.
        ("US ⇧;", &latin, at("Shift+:", Key::Semicolon), ":"),
        ("US ⇧⌘=", &latin, at("Cmd+Shift++", Key::Equals), "Cmd++"),
        ("US ⇧- types _, which has no key", &latin, at("Shift+-", Key::Minus), "Shift+-"),
        // macOS "Dvorak – QWERTY ⌘": with ⌘ held the layout's ⌘ table is QWERTY, while the
        // logical key egui reports is Dvorak's K.
        ("Dvorak – QWERTY ⌘ ⌘V", &latin, at("Cmd+K", Key::V).with_latin('v'), "Cmd+V"),
        // A Shift level the platform reports wins over the inference.
        ("reported Shift level", &latin, at("Cmd+Shift+/", Key::Num7).with_shift_needed(false), "Cmd+Shift+/"),
    ];
    for (name, layout, input, want) in rows {
        assert_eq!(layout.resolve(input).map(|c| c.to_string()).as_deref(), Some(*want), "{name}");
    }
}

#[test]
fn reported_cases_match_only_the_chord_the_press_means() {
    let p = Platform::Windows;
    let arabic = Layout::new().with_latin(false);
    let ctrl_shift_d = at("Cmd+Shift+]", Key::D);
    assert!(arabic.matches(&chord("Ctrl+Shift+D"), &ctrl_shift_d, p));
    assert!(!arabic.matches(&chord("Ctrl+Shift+]"), &ctrl_shift_d, p), "Arabic Ctrl+Shift+D must not run Ctrl+Shift+]");

    let dvorak_russian_q = at("Cmd+Q", Key::Q).with_latin('\'');
    assert!(!Layout::new().with_latin(false).matches(&chord("Ctrl+Q"), &dvorak_russian_q, p), "Dvorak + Russian й must not quit");

    let qwerty_cmd = at("Cmd+K", Key::V).with_latin('v');
    assert!(Layout::new().matches(&chord("Cmd+V"), &qwerty_cmd, Platform::Mac));
    assert!(!Layout::new().matches(&chord("Cmd+K"), &qwerty_cmd, Platform::Mac), "⌘V must not open the command palette");

    let german = Layout::new();
    let slash = at("Cmd+Shift+/", Key::Num7);
    assert_eq!(texts(&german.candidates(&slash)), ["Cmd+/", "Cmd+Shift+7"]);
    assert!(german.matches(&chord("⌘/"), &slash, Platform::Mac));
    // A US-written ⌘? binding is reached with German's ⇧ß, not with ⇧7.
    assert!(!german.matches(&chord("Cmd+Shift+/"), &slash, p));
    assert_eq!(texts(&german.candidates(&at("Cmd+Shift+?", Key::Minus))), ["Cmd+?", "Cmd+Shift+/"]);
    // German Ctrl+; is Ctrl+Shift+, and must not run a Ctrl+Shift+, (font size) binding.
    let semicolon = at("Cmd+Shift+;", Key::Comma);
    assert!(german.matches(&chord("Ctrl+;"), &semicolon, p));
    assert!(!german.matches(&chord("Ctrl+Shift+,"), &semicolon, p));

    // AZERTY's 6 key types -: Ctrl+- first, then Ctrl+6.
    assert_eq!(texts(&Layout::new().candidates(&at("Cmd+-", Key::Num6))), ["Cmd+-", "Cmd+6"]);
    // Zoom keys: + also runs =, and Shift+= runs +.
    assert_eq!(texts(&Layout::new().candidates(&at("Cmd+Shift++", Key::Equals))), ["Cmd++", "Cmd+Shift+=", "Cmd+="]);
    assert_eq!(texts(&Layout::new().candidates(&KeyInput::new(Mods::SHIFT).with_typed(Key::Equals))), ["Shift+=", "+"]);
}

#[test]
fn typing_decides_whether_the_layout_is_latin() {
    let mut layout = Layout::new();
    assert!(layout.is_latin(), "nothing known counts as Latin");
    let ctrl_shift_d = at("Cmd+Shift+]", Key::D);
    // Read as punctuation until the layout is known: ] needs Shift on the D key.
    assert_eq!(layout.resolve(&ctrl_shift_d), Some(chord("Cmd+]")));

    layout.observe(Key::D, Mods::NONE, "ي");
    assert!(!layout.is_latin());
    assert_eq!(layout.resolve(&ctrl_shift_d), Some(chord("Cmd+Shift+D")));

    // Punctuation on a letter key says nothing: Hebrew Q and Dvorak Q both type a symbol.
    layout.observe(Key::Q, Mods::NONE, "/");
    assert!(!layout.is_latin());
    layout.observe(Key::A, Mods::SHIFT, "A");
    assert!(layout.is_latin());
    layout.observe(Key::W, Mods::NONE, "ς");
    assert!(!layout.is_latin());
    layout.observe(Key::S, Mods::CMD, "s");
    assert!(!layout.is_latin(), "a command chord teaches nothing");
    layout.observe(Key::S, Mods::NONE, "š");
    assert!(layout.is_latin(), "Latin letters beyond ASCII count as Latin");
    layout.observe(Key::M, Mods::NONE, "µ");
    assert!(layout.is_latin(), "Latin-1 symbols that count as letters say nothing");

    // What the platform reports wins.
    let mut reported = Layout::new().with_latin(false);
    reported.observe(Key::A, Mods::NONE, "a");
    assert!(!reported.is_latin());
    reported.set_latin(None);
    assert!(reported.is_latin());
}

#[test]
fn dvorak_shift_punctuation_never_runs_its_position() {
    // Dvorak's , key sits on W and types < with Shift, which egui has no key for, so egui
    // reports the press as W.
    let ctrl_shift_lt = at("Cmd+Shift+W", Key::W);
    let mut dvorak = Layout::new();
    assert_eq!(dvorak.resolve(&ctrl_shift_lt), Some(chord("Cmd+Shift+W")), "nothing known yet");

    dvorak.observe(Key::S, Mods::NONE, "o");
    assert_eq!(dvorak.resolve(&ctrl_shift_lt), Some(chord("Cmd+Shift+W")), "the W position is unseen");
    dvorak.observe(Key::W, Mods::NONE, ",");
    assert_eq!(dvorak.resolve(&ctrl_shift_lt), None);
    assert!(!dvorak.matches(&chord("Ctrl+Shift+W"), &ctrl_shift_lt, Platform::Windows));
    // Without Shift, and for letters typed where they sit (Dvorak's A) or elsewhere, presses
    // resolve as before.
    assert_eq!(dvorak.resolve(&at("Cmd+W", Key::W)), Some(chord("Cmd+W")));
    dvorak.observe(Key::A, Mods::NONE, "a");
    assert_eq!(dvorak.resolve(&at("Cmd+Shift+A", Key::A)), Some(chord("Cmd+Shift+A")));
    assert_eq!(dvorak.resolve(&at("Cmd+Shift+O", Key::S)), Some(chord("Cmd+Shift+O")));

    // AZERTY: A is moved, but the unseen E position still runs Ctrl+Shift+E.
    let mut azerty = Layout::new();
    azerty.observe(Key::Q, Mods::NONE, "a");
    assert_eq!(azerty.resolve(&at("Cmd+Shift+E", Key::E)), Some(chord("Cmd+Shift+E")));

    // QWERTY: a position typing its own letter keeps the position fallback.
    let mut qwerty = Layout::new();
    qwerty.observe(Key::S, Mods::NONE, "s");
    qwerty.observe(Key::W, Mods::SHIFT, "W");
    assert_eq!(qwerty.resolve(&ctrl_shift_lt), Some(chord("Cmd+Shift+W")));
    assert_eq!(qwerty.resolve(&at("Cmd+Shift+E", Key::E)), Some(chord("Cmd+Shift+E")));
}

#[test]
fn a_layout_switch_forgets_what_letter_positions_typed() {
    // On Dvorak the W position types `,`; after a switch to QWERTY, Ctrl+Shift+W is W again.
    let ctrl_shift_w = at("Cmd+Shift+W", Key::W);
    let mut layout = Layout::new().with_latin(true);
    layout.observe(Key::W, Mods::NONE, ",");
    assert_eq!(layout.resolve(&ctrl_shift_w), None);
    layout.set_latin(Some(true));
    assert_eq!(layout.resolve(&ctrl_shift_w), None, "the same report is the same layout");

    // Dvorak, then Russian, then QWERTY, as the platform reports them.
    layout.set_latin(Some(false));
    layout.set_latin(Some(true));
    assert_eq!(layout.resolve(&ctrl_shift_w), Some(chord("Cmd+Shift+W")));

    // An app that learns of a switch itself.
    let mut unreported = Layout::new();
    unreported.observe(Key::W, Mods::NONE, ",");
    assert_eq!(unreported.resolve(&ctrl_shift_w), None);
    unreported.forget_letters();
    assert_eq!(unreported.resolve(&ctrl_shift_w), Some(chord("Cmd+Shift+W")));
}

#[test]
fn labels_show_the_layouts_own_punctuation() {
    let mut german = Layout::new().with_char(Key::OpenBracket, 'ü').with_char(Key::Slash, '-').with_char(Key::CloseBracket, '+');
    german.observe(Key::Minus, Mods::NONE, "ß");
    let label = |layout: &Layout, text: &str, p| chord(text).display_for(p, layout);
    assert_eq!(label(&german, "Cmd+[", Platform::Windows), "Ctrl+Ü");
    assert_eq!(label(&german, "Cmd+[", Platform::Mac), "⌘Ü");
    // German types - at the / position, so Ctrl+- shows -; its + key types a key of its own.
    assert_eq!(label(&german, "Cmd+-", Platform::Windows), "Ctrl+-");
    assert_eq!(label(&german, "Cmd+]", Platform::Windows), "Ctrl+]");
    assert_eq!(label(&german, "Cmd+Z", Platform::Windows), "Ctrl+Z");
    assert_eq!(label(&german, "Cmd+1", Platform::Windows), "Ctrl+1");

    let russian = Layout::new().with_latin(false).with_char(Key::OpenBracket, 'х');
    assert_eq!(label(&russian, "Cmd+[", Platform::Linux), "Ctrl+Х");
    assert_eq!(label(&russian, "Cmd+C", Platform::Linux), "Ctrl+C");
    assert_eq!(craft_keymap::Trigger::double(chord("[")).display_for(Platform::Linux, &russian), "Х Х");
    assert_eq!(label(&Layout::new(), "Cmd+[", Platform::Linux), "Ctrl+[");
}
