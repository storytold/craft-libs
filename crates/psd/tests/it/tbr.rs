//! Toolbar presets (`.tbr`): the measured layout, round trips and hostile input
//! (storytold/photocraft#14624).

use craft_psd::PsdError;
use craft_psd::descriptor::{Descriptor, Id, UnicodeString, Value};
use craft_psd::tbr::{self, MAX_BYTES, Shortcut, Tool, ToolbarPreset};

fn tool(key: &[u8; 4], shortcut: Shortcut) -> Tool {
    Tool { key: *key, shortcut }
}

fn text(s: &str) -> Shortcut {
    Shortcut::Text(s.into())
}

/// A preset with two toolbar groups and one extras group, shortcuts present, empty and absent.
fn sample() -> ToolbarPreset {
    ToolbarPreset {
        toolbar: vec![vec![tool(b"arwT", text("V")), tool(b"ArtT", text("V"))], vec![tool(b"blur", Shortcut::Absent)]],
        extras: vec![vec![tool(b"zoom", text(""))]],
        ..ToolbarPreset::default()
    }
}

/// A descriptor as presets hold them: the name is a lone NUL, the class `null`.
fn object() -> Descriptor {
    Descriptor { name: UnicodeString::new_nul(""), class_id: Id::new("null"), items: Vec::new() }
}

#[test]
fn a_written_preset_has_the_measured_layout() {
    let bytes = tbr::write(&sample()).unwrap();
    // Name: one NUL; class `null`.
    assert_eq!(bytes[..14], [0, 0, 0, 1, 0, 0, 0, 0, 0, 0, b'n', b'u', b'l', b'l']);
    let d = Descriptor::from_bytes(&bytes).unwrap();
    let keys: Vec<String> = d.items.iter().map(|(k, _)| String::from_utf8_lossy(k.as_bytes()).into_owned()).collect();
    assert_eq!(keys, ["tver", "tpad", "fgbg", "qkmm", "sImI", "scmd", "shov", "nana", "ddkb", "tlst", "oflt"]);
    let Some(Value::List(groups)) = d.get("tlst") else { panic!("tlst is a list") };
    let Value::List(first) = &groups[0] else { panic!("a group is a list") };
    let Value::Descriptor(mv) = &first[0] else { panic!("a tool is an object") };
    assert_eq!(mv.get("toolKey"), Some(&Value::Integer(i32::from_be_bytes(*b"arwT"))));
    assert_eq!(mv.get_text("keyboardShortcut").as_deref(), Some("V"));
    let Value::List(second) = &groups[1] else { panic!("a group is a list") };
    let Value::Descriptor(blur) = &second[0] else { panic!("a tool is an object") };
    assert!(blur.get("keyboardShortcut").is_none(), "a tool without a shortcut has no keyboardShortcut");
}

#[test]
fn a_preset_round_trips_every_value() {
    let def = ToolbarPreset::default();
    let changed = ToolbarPreset {
        version: def.version + 1,
        pad: def.pad + 7,
        show_colors: !def.show_colors,
        show_quick_mask: !def.show_quick_mask,
        show_add_images: !def.show_add_images,
        show_screen_mode: !def.show_screen_mode,
        show_extras_slot: !def.show_extras_slot,
        nana: !def.nana,
        disable_extras_shortcuts: !def.disable_extras_shortcuts,
        ..sample()
    };
    for preset in [ToolbarPreset::default(), sample(), changed] {
        assert_eq!(tbr::read(&tbr::write(&preset).unwrap()).unwrap(), preset);
    }
}

/// Entries that aren't tools are skipped, a shortcut that isn't text says so, and a preset that
/// leaves out its flags reads them as the defaults.
#[test]
fn a_hand_made_preset_keeps_its_tools_and_skips_the_rest() {
    let mv = object().with("toolKey", Value::Integer(i32::from_be_bytes(*b"arwT"))).with("keyboardShortcut", Value::Integer(5));
    let group = Value::List(vec![Value::Integer(3), Value::Descriptor(object().with("other", Value::Boolean(true))), Value::Descriptor(mv)]);
    let bytes = object().with("tlst", Value::List(vec![group])).with("oflt", Value::List(vec![])).to_bytes();
    let preset = tbr::read(&bytes).unwrap();
    assert_eq!(preset.toolbar, [vec![tool(b"arwT", Shortcut::NotText)]]);
    assert_eq!(ToolbarPreset { toolbar: Vec::new(), ..preset }, ToolbarPreset::default());
}

#[test]
fn a_damaged_preset_is_refused() {
    let good = tbr::write(&sample()).unwrap();
    let mut bad: Vec<Vec<u8>> = vec![Vec::new(), vec![b'x'; 100], [good.as_slice(), &[0]].concat(), vec![0; MAX_BYTES + 1]];
    bad.extend((1..good.len()).map(|n| good[..n].to_vec()));
    bad.push(object().with("tver", Value::Integer(1)).to_bytes());
    bad.push(object().with("tlst", Value::List(vec![Value::Integer(3)])).with("oflt", Value::List(vec![])).to_bytes());
    // A group longer than any toolbar.
    let crowded = Value::List((0..300).map(|_| Value::Descriptor(object().with("toolKey", Value::Integer(1)))).collect());
    bad.push(object().with("tlst", Value::List(vec![crowded])).with("oflt", Value::List(vec![])).to_bytes());
    for bytes in &bad {
        assert!(tbr::read(bytes).is_err(), "{} bytes were read as a preset", bytes.len());
    }
}

/// The writer stops where the reader does: a preset at each limit round trips, and one past it is
/// a limit error rather than bytes the reader would refuse.
#[test]
fn a_preset_at_the_limits_round_trips_and_one_past_them_is_refused() {
    let tools = |n: usize| (0..n).map(|_| tool(b"blur", Shortcut::Absent)).collect::<Vec<_>>();
    let at_limit = [
        ToolbarPreset { toolbar: vec![tools(256)], ..ToolbarPreset::default() },
        ToolbarPreset { extras: vec![Vec::new(); 256], ..ToolbarPreset::default() },
    ];
    let past_limit = [
        ToolbarPreset { toolbar: vec![tools(257)], ..ToolbarPreset::default() },
        ToolbarPreset { extras: vec![Vec::new(); 257], ..ToolbarPreset::default() },
    ];
    for preset in at_limit {
        assert_eq!(tbr::read(&tbr::write(&preset).unwrap()).unwrap(), preset);
    }
    for preset in past_limit {
        assert!(matches!(tbr::write(&preset), Err(PsdError::LimitExceeded(_))));
    }

    // A shortcut long enough to bring the file within one UTF-16 unit of MAX_BYTES, then one
    // unit longer.
    let long = |n: usize| ToolbarPreset { toolbar: vec![vec![tool(b"blur", text(&"x".repeat(n)))]], ..ToolbarPreset::default() };
    let base = tbr::write(&long(0)).unwrap().len();
    let fits = long((MAX_BYTES - base) / 2);
    let bytes = tbr::write(&fits).unwrap();
    assert!(bytes.len() <= MAX_BYTES && bytes.len() + 2 > MAX_BYTES, "{} bytes is not at the limit", bytes.len());
    assert_eq!(tbr::read(&bytes).unwrap(), fits);
    assert!(matches!(tbr::write(&long((MAX_BYTES - base) / 2 + 1)), Err(PsdError::LimitExceeded(_))));
}
