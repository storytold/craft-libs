//! The table's model through the dialog: which rows are listed and which count as conflicts.

use std::sync::Arc;

use craft_host::{AppInfo, Host};
use craft_keymap::{Binding, CommandDef, CommandRef, Keymap, Platform, Policy, Scope, Scopes};
use craft_menu::shortcuts::{Area, HoldSetting, Outcome};
use craft_ui::contract::Role;
use craft_ui::testing::trial;
use egui::{Key, Modifiers};

use super::fixture::{self, Rig};

#[trial]
fn hidden_entries_are_not_listed_but_still_conflict() {
    let mut options = fixture::options();
    options.visible = Some(|b| b.command.id != "file.export");
    let mut rig = Rig::new(fixture::keymap(Platform::Mac), options, None);
    assert!(!rig.has("Export…: ⌘E"), "a hidden command is not listed");
    rig.shared.borrow_mut().km.assign(&CommandRef::from("file.save"), Binding::parse("Cmd+E").expect("a chord"), Policy::Keep).expect("assigns");
    rig.pass();
    rig.choose("Filter:", "Conflicts");
    assert!(rig.has("Save: ⌘S") && rig.has("Save: ⌘E"), "the hidden command still clashes with Save");
}

/// Every row that can take another shortcut shows its add button after its chips, in a narrow dialog too: the chips
/// give way to it.
#[trial]
fn every_row_that_can_take_a_shortcut_has_its_add_button() {
    let mut rig = Rig::new(fixture::keymap(Platform::Mac), fixture::options(), None);
    rig.fixture.room(egui::vec2(640.0, 600.0));
    rig.fixture.settle();
    for label in ["Save", "Save As…", "Export…", "Bring to Front", "Loop"] {
        let add = format!("Add a shortcut to {label}");
        assert!(rig.has(&add), "{label:?} has no add button; buttons {:?}", rig.names_with(craft_ui::contract::Role::BUTTON, "Add a shortcut"));
    }
}

/// A query typed in capitals finds the row: the search folds the dotted capital İ to `i`, where a lowercase fold
/// gives `i` plus a combining dot and finds nothing. The English alias finds it too.
#[trial]
fn the_search_folds_dotted_capitals() {
    let (i18n, _) = craft_i18n::I18n::from_owned([("az".to_owned(), String::new(), String::new())], &["az"]);
    let lang = i18n.from_code("az").expect("az is offered");
    let host = Arc::new(Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).i18n(Arc::new(i18n)).lang(lang).build());
    let path: Vec<String> = ["Image", "Image Size…"].iter().map(|s| (*s).to_owned()).collect();
    let size = CommandDef::new("image.size", "Şəkil ölçüsü…", Scope::app()).with_path(path).with_alias("Image Size");
    let other = CommandDef::new("file.save", "Saxla", Scope::app());
    let listed = |query: &str| {
        let keymap = Keymap::new(vec![size.clone(), other.clone()], Scopes::new(), Platform::Mac);
        let mut rig = Rig::new(keymap, fixture::options(), None);
        rig.fixture.host(host.clone());
        rig.pass();
        let field = rig.find("Search:");
        rig.fixture.click(&field);
        rig.fixture.type_text(query);
        rig.fixture.idle(2);
        (!rig.names_with(Role::ROW, "Şəkil ölçüsü").is_empty(), !rig.names_with(Role::ROW, "Saxla").is_empty())
    };
    assert_eq!(listed("ŞƏKİL ÖLÇÜSÜ"), (true, false), "capitals with the dotted İ");
    assert_eq!(listed("IMAGE SIZE"), (true, false), "the English alias");
}

/// With holds on, Summarize… writes a hold binding in its command's line as its trigger text; with holds off the
/// summary leaves it out.
#[trial]
fn the_summary_lists_hold_bindings() {
    let mut km = fixture::keymap(Platform::Mac);
    km.set_holds(true);
    km.assign(&CommandRef::from("file.save"), Binding::parse("Hold K").expect("a hold"), Policy::Keep).expect("assigns");
    let mut rig = Rig::with_holds(km, fixture::options(), HoldSetting::new(true, 0.5, 0.2..=2.0));
    let summarize = |rig: &mut Rig| {
        rig.click("Summarize…");
        rig.outcomes()
            .into_iter()
            .rev()
            .find_map(|o| match o {
                Outcome::Summary { text, html, .. } => Some((text, html)),
                _ => None,
            })
            .expect("a summary")
    };

    let (text, html) = summarize(&mut rig);
    assert!(text.lines().any(|line| line.contains("Save") && line.contains("Hold K")), "Save's line has its hold: {text}");
    assert!(html.contains("Hold K"), "the HTML has the hold: {html}");

    rig.click("Hold Shortcuts");
    let (text, html) = summarize(&mut rig);
    assert!(!text.contains("Hold K") && !html.contains("Hold K"), "with holds off the summary has no hold: {text}");
}

/// The summary written without a `Ui` (a command's file export) is the one Summarize… writes under the same host.
#[trial]
fn the_summary_without_a_ui_is_the_one_summarize_writes() {
    let host = Arc::new(Host::builder(AppInfo { id: "trial", name: "Trial", version: "0" }).build());
    let km = fixture::keymap(Platform::Mac);
    let mut rig = Rig::new(km.clone(), fixture::options(), None);
    rig.fixture.host(host.clone());
    rig.pass();
    rig.click("Summarize…");
    let text = rig
        .outcomes()
        .into_iter()
        .rev()
        .find_map(|o| match o {
            Outcome::Summary { text, .. } => Some(text),
            _ => None,
        })
        .expect("a summary");
    let (title, _) = text.split_once('\n').expect("a title line");
    assert_eq!(craft_menu::shortcuts::summary::text_for(&km, &host, title), text);
}

fn is_tool(def: &CommandDef) -> bool {
    matches!(def.kind, craft_keymap::CommandKind::Tool { .. })
}

fn area(rig: &Rig) -> String {
    let shared = rig.shared.borrow();
    shared.state.snapshot(&shared.km).expect("the dialog has shown").area
}

/// With `own_areas`, Show lists only the app's own entries, the dialog opens on the first of them, and its table
/// lists that entry's rows. Without it, the built-in entries come first and the dialog opens on All Commands.
#[trial]
fn own_areas_show_only_the_apps_entries_and_open_on_the_first() {
    let tune = |own: bool| {
        move |options: &mut craft_menu::shortcuts::Options| {
            options.area_tabs = true;
            options.areas = vec![Area::new("tools", "Tools", is_tool), Area::new("menu", "Menu Commands", |def| !is_tool(def))];
            options.own_areas = own;
        }
    };
    let rig = Rig::tuned(Platform::Mac, tune(true));
    assert_eq!(rig.names_with(Role::TAB, ""), ["Tools", "Menu Commands"], "only the app's entries are tabs");
    assert_eq!(area(&rig), "tools", "the dialog opens on the first of them");
    let rows = rig.names_with(Role::ROW, "");
    assert!(rows.iter().all(|row| row.contains("Marquee")) && !rows.is_empty(), "the table lists the Tools rows only: {rows:?}");

    let rig = Rig::tuned(Platform::Mac, tune(false));
    let tabs = rig.names_with(Role::TAB, "");
    assert_eq!(tabs.first().map(String::as_str), Some("All Commands"), "the built-in entries come first: {tabs:?}");
    assert!(tabs.iter().any(|t| t == "Menu Commands"), "the app's entries follow them: {tabs:?}");
    assert_eq!(area(&rig), "all");
}

/// Show as tabs is a tab list named like the Show select it replaces.
#[trial]
fn area_tabs_are_named_show() {
    let rig = Rig::tuned(Platform::Mac, |options| options.area_tabs = true);
    assert_eq!(rig.names_with(Role::TAB_LIST, ""), ["Show:"]);
}

/// A dialog opened with a command selected, with Show listing only the app's entries, opens on the entry that lists it.
#[trial]
fn own_areas_open_on_the_selected_commands_entry() {
    let mut rig = Rig::tuned(Platform::Mac, |options| {
        options.areas = vec![Area::new("tools", "Tools", is_tool), Area::new("menu", "Menu Commands", |def| !is_tool(def))];
        options.own_areas = true;
    });
    {
        let mut shared = rig.shared.borrow_mut();
        let mut state = craft_menu::shortcuts::DialogState::new();
        state.select(&CommandRef::from("file.export"));
        shared.state = state;
    }
    rig.pass();
    assert_eq!(area(&rig), "menu", "Export… is listed under Menu Commands");
    assert!(rig.names_with(Role::ROW, "").iter().any(|row| row.starts_with("Export…")), "its row shows");
}

/// Accept and Go to Conflict, with Show listing only the app's entries, shows the holder under the entry that lists
/// it.
#[trial]
fn going_to_a_conflict_picks_an_area_that_lists_the_holder() {
    let mut rig = Rig::tuned(Platform::Mac, |options| {
        options.areas = vec![Area::new("tools", "Tools", is_tool), Area::new("menu", "Menu Commands", |def| !is_tool(def))];
        options.own_areas = true;
    });
    rig.click("Marquee Tool: M");
    rig.press(Modifiers::COMMAND, Key::E);
    rig.click("Accept and Go to Conflict");
    assert_eq!(area(&rig), "menu", "Export… is listed under Menu Commands");
    assert!(rig.has("Export…"), "the holder's row shows");
}
