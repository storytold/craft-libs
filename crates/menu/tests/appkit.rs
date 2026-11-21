//! The macOS backend through AppKit's own menus, read back from `NSApp`'s main menu. On the main
//! thread (`harness = false`): AppKit's menus need it, and one process installs one menu bar.

#[cfg(target_os = "macos")]
fn main() {
    // Without a harness nothing answers `--list`: list the one test (and nothing for `--ignored`),
    // as libtest's terse format would.
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--list") {
        if !args.iter().any(|a| a == "--ignored") {
            println!("appkit: test");
        }
        return;
    }
    // One process installs one menu bar, so each case runs in a process of its own.
    if args.iter().any(|a| a == "--without-window-menu") {
        mac::a_bar_without_a_window_menu_keeps_its_main_menu();
        return;
    }
    mac::disabled_items_lose_their_key_equivalents_in_place();
    let exe = std::env::current_exe().expect("the test binary");
    let status = std::process::Command::new(exe).arg("--without-window-menu").status().expect("the second case runs");
    assert!(status.success(), "a_bar_without_a_window_menu_keeps_its_main_menu failed");
    println!("test appkit ... ok");
}

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
mod mac {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use craft_menu::mac::{MacOptions, install_with};
    use craft_menu::{Chord, Item, Layout, Menu, MenuBar, MenuRole, Node};
    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSApplication, NSMenu, NSMenuItem};

    fn item(command: &str, label: &str, chord: &str, enabled: bool) -> Item {
        Item::new(command, label).shortcut(Chord::parse(chord).unwrap()).enabled(enabled)
    }

    /// Save, Bold (a check item in a submenu) and the Window menu's own item are enabled or not;
    /// Close always is.
    fn bar(enabled: bool) -> MenuBar {
        MenuBar {
            menus: vec![
                Menu::new("Test", vec![Item::new("app.about", "About").into()]).role(MenuRole::App),
                Menu::new("File", vec![item("file.save", "Save", "Cmd+S", enabled).into(), item("file.close", "Close", "Cmd+W", true).into()])
                    .role(MenuRole::File),
                Menu::new("Format", vec![Node::submenu("Font", vec![item("format.bold", "Bold", "Cmd+B", enabled).check(true).into()])]),
                Menu::new("Window", vec![item("window.next", "Next", "Cmd+K", enabled).into()]).role(MenuRole::Window),
            ],
            documents: Vec::new(),
        }
    }

    fn find(menu: &NSMenu, title: &str) -> Retained<NSMenuItem> {
        (0..menu.numberOfItems())
            .filter_map(|i| menu.itemAtIndex(i))
            .find(|it| it.title().to_string() == title)
            .unwrap_or_else(|| panic!("no {title} item"))
    }

    fn titles(menu: &NSMenu) -> Vec<String> {
        (0..menu.numberOfItems()).filter_map(|i| menu.itemAtIndex(i)).map(|it| it.title().to_string()).collect()
    }

    /// The menu at `path` in the main menu.
    fn menu(app: &NSApplication, path: &[&str]) -> Retained<NSMenu> {
        let mut m = app.mainMenu().expect("a main menu");
        for title in path {
            m = find(&m, title).submenu().unwrap_or_else(|| panic!("{title} has no submenu"));
        }
        m
    }

    /// (key equivalent, enabled, checked) of each item: Save, Close, Bold and Next.
    fn read(app: &NSApplication) -> Vec<(String, bool, bool)> {
        [(&["File"][..], "Save"), (&["File"][..], "Close"), (&["Format", "Font"][..], "Bold"), (&["Window"][..], "Next")]
            .iter()
            .map(|(path, title)| {
                let it = find(&menu(app, path), title);
                (it.keyEquivalent().to_string(), it.isEnabled(), it.state() == objc2_app_kit::NSControlStateValueOn)
            })
            .collect()
    }

    fn row(key: &str, enabled: bool, checked: bool) -> (String, bool, bool) {
        (key.to_owned(), enabled, checked)
    }

    /// An app without a Window menu: enabling and disabling an item never replaces the main menu.
    pub fn a_bar_without_a_window_menu_keeps_its_main_menu() {
        let mtm = MainThreadMarker::new().expect("the main thread");
        let app = NSApplication::sharedApplication(mtm);
        let ctx = egui::Context::default();
        let bar = |enabled: bool| MenuBar {
            menus: vec![
                Menu::new("Test", vec![Item::new("app.about", "About").into()]).role(MenuRole::App),
                Menu::new("File", vec![item("file.save", "Save", "Cmd+S", enabled).into()]).role(MenuRole::File),
            ],
            documents: Vec::new(),
        };
        let mut native = install_with(&ctx, &bar(true), MacOptions::default()).expect("the menu bar installs");
        let main = app.mainMenu().expect("a main menu");
        for enabled in [false, true, false] {
            native.sync(Layout { bar: bar(enabled), clashes: Vec::new() });
            assert_eq!(find(&menu(&app, &["File"]), "Save").isEnabled(), enabled);
        }
        assert!(std::ptr::eq(Retained::as_ptr(&main), Retained::as_ptr(&app.mainMenu().expect("a main menu"))), "the menu bar was rebuilt");
    }

    pub fn disabled_items_lose_their_key_equivalents_in_place() {
        let mtm = MainThreadMarker::new().expect("the main thread");
        let app = NSApplication::sharedApplication(mtm);
        let ctx = egui::Context::default();
        let options = MacOptions::default().disabled_key_equivalents(false);
        let mut native = install_with(&ctx, &bar(false), options).expect("the menu bar installs");

        let disabled = vec![row("", false, false), row("w", true, false), row("", false, true), row("", false, false)];
        let enabled = vec![row("s", true, false), row("w", true, false), row("b", true, true), row("k", true, false)];
        assert_eq!(read(&app), disabled, "built disabled: no key equivalents");
        let main = app.mainMenu().expect("a main menu");

        native.sync(Layout { bar: bar(true), clashes: Vec::new() });
        assert_eq!(read(&app), enabled, "enabled: each item has its key equivalent");

        native.sync(Layout { bar: bar(false), clashes: Vec::new() });
        assert_eq!(read(&app), disabled, "disabled again: no key equivalents, Bold still checked");
        assert_eq!(titles(&menu(&app, &["File"])), ["Save", "Close"], "Save is back in its place");

        native.sync(Layout { bar: bar(true), clashes: Vec::new() });
        assert_eq!(read(&app), enabled);
        assert!(std::ptr::eq(Retained::as_ptr(&main), Retained::as_ptr(&app.mainMenu().expect("a main menu"))), "the menu bar was never rebuilt");
    }
}
