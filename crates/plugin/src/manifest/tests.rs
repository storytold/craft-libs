use serde_json::json;

use super::{ButtonSize, Host, Manifest};

const GRID: Host<'static> = Host { id: "gridcraft", name: "GridCraft", menus: &["cell", "row", "column", "sheetTab"] };
const WORD: Host<'static> = Host { id: "wordcraft", name: "WordCraft", menus: &["text", "table", "picture"] };

fn manifest(functions: serde_json::Value) -> String {
    json!({"id": "t.plugin", "name": "T", "run": {"command": "true"}, "functions": functions}).to_string()
}

#[test]
fn manifests_are_validated() {
    let m = Manifest::parse(&manifest(json!([{"name": "t.one", "maxArgs": 2}])), &GRID).unwrap();
    assert_eq!(m.functions[0].name, "T.ONE");
    assert_eq!((m.functions[0].min_args, m.functions[0].max_args), (0, 2));
    for bad in [
        json!([{"name": "1BAD"}]),
        json!([{"name": "BAD NAME"}]),
        json!([{"name": "T.X"}, {"name": "t.x"}]),
        json!([{"name": "T.X", "minArgs": 3, "maxArgs": 2}]),
        json!([{"name": "T.X", "maxArgs": 300}]),
    ] {
        assert!(Manifest::parse(&manifest(bad.clone()), &GRID).is_err(), "{bad}");
    }
    assert!(Manifest::parse(r#"{"id": "bad id", "name": "x", "run": {"command": "x"}}"#, &GRID).is_err());
    assert!(Manifest::parse(r#"{"id": "x", "name": "x", "run": {"command": ""}}"#, &GRID).is_err());
    assert!(Manifest::parse("not json", &GRID).is_err());
}

#[test]
fn an_add_in_the_app_runs_itself_needs_no_program() {
    let text = r#"{"id": "x", "name": "x"}"#;
    assert!(Manifest::parse(text, &GRID).is_err());
    assert!(Manifest::parse_without_program(text, &GRID).is_ok());
    assert!(Manifest::parse_without_program(r#"{"id": "bad id", "name": "x"}"#, &GRID).is_err(), "the other checks still apply");
}

#[test]
fn one_manifest_serves_several_apps() {
    let both = json!({
        "id": "t.both", "name": "Both", "run": {"command": "true"}, "apps": ["wordcraft", "GridCraft"],
        "functions": [{"name": "T.TWICE", "minArgs": 1, "maxArgs": 1}],
        "commands": [{"id": "tidy", "label": "Tidy"}],
        "ui": {"menus": [{"menu": "text", "command": "tidy"}]},
        "uiFor": {"gridcraft": {"menus": [{"menu": "row", "command": "tidy"}]}, "wordcraft": {"menus": [{"menu": "picture", "command": "tidy"}]}}
    })
    .to_string();
    // Each app reads its own `uiFor` entry in place of `ui`.
    let grid = Manifest::parse(&both, &GRID).unwrap();
    assert_eq!(grid.ui.menus.iter().map(|i| i.menu.as_str()).collect::<Vec<_>>(), ["row"]);
    let word = Manifest::parse(&both, &WORD).unwrap();
    assert_eq!(word.ui.menus.iter().map(|i| i.menu.as_str()).collect::<Vec<_>>(), ["picture"]);
    assert!(grid.is_for("gridcraft") && grid.is_for("WORDCRAFT") && !grid.is_for("deckcraft"));

    let only_word = json!({"id": "t.word", "name": "Word", "run": {"command": "true"}, "apps": ["wordcraft"]}).to_string();
    assert!(Manifest::parse(&only_word, &GRID).unwrap_err().contains("not GridCraft"));
    assert!(Manifest::parse(&only_word, &WORD).is_ok());

    // Without a `uiFor` entry, `ui` is the app's: another app's menu names are an error.
    let plain = json!({"id": "t.p", "name": "P", "run": {"command": "true"}, "commands": [{"id": "go", "label": "Go"}], "ui": {"menus": [{"menu": "text", "command": "go"}]}})
        .to_string();
    assert!(Manifest::parse(&plain, &GRID).is_err());
    assert!(Manifest::parse(&plain, &WORD).is_ok());
}

#[test]
fn ui_declarations_must_name_the_plugins_commands() {
    let with_ui = |ui: serde_json::Value| {
        json!({"id": "t.ui", "name": "T", "run": {"command": "true"},
            "commands": [{"id": "go", "label": "Go"}], "ui": ui})
        .to_string()
    };
    let m = Manifest::parse(
        &with_ui(json!({
            "ribbon": [{"tab": "Data", "group": "Mine", "buttons": [{"command": "go", "size": "small", "params": {"n": 1}}]}],
            "menus": [{"command": "go"}, {"menu": "sheetTab", "command": "go", "label": "Go here"}]
        })),
        &GRID,
    )
    .unwrap();
    assert_eq!(m.ui.ribbon[0].buttons[0].size, ButtonSize::Small);
    assert_eq!(m.ui.menus[0].menu, "cell", "an item that names no menu goes in the app's default one");
    assert_eq!(m.ui.menus[1].menu, "sheetTab");
    assert_eq!(m.command("go").map(|c| c.label.as_str()), Some("Go"));
    for bad in [
        json!({"ribbon": [{"tab": "Data", "group": "G", "buttons": [{"command": "nope"}]}]}),
        json!({"ribbon": [{"tab": "Data", "group": "G", "buttons": []}]}),
        json!({"ribbon": [{"tab": " ", "group": "G", "buttons": [{"command": "go"}]}]}),
        json!({"ribbon": [{"tab": "Data", "group": "G", "buttons": [{"command": "go", "size": "huge"}]}]}),
        json!({"menus": [{"command": "nope"}]}),
        json!({"menus": [{"menu": "statusBar", "command": "go"}]}),
        json!({"menus": [{"command": "go", "label": "x".repeat(65)}]}),
    ] {
        assert!(Manifest::parse(&with_ui(bad.clone()), &GRID).is_err(), "{bad}");
    }
    // An app with no right-click menus for add-ins takes no menu items.
    let none = Host { id: "x", name: "X", menus: &[] };
    assert!(Manifest::parse(&with_ui(json!({"menus": [{"command": "go"}]})), &none).is_err());
}

#[test]
fn homepages_must_be_web_links() {
    let with = |url: &str| json!({"id": "t.home", "name": "T", "run": {"command": "true"}, "homepage": url}).to_string();
    assert_eq!(Manifest::parse(&with("https://example.org/doc"), &GRID).unwrap().homepage, "https://example.org/doc");
    assert!(Manifest::parse(&with(""), &GRID).is_ok());
    for bad in ["javascript:alert(1)", "file:///etc/passwd", "example.org", "https://a b"] {
        assert!(Manifest::parse(&with(bad), &GRID).is_err(), "{bad}");
    }
}
