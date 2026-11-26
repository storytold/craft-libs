//! The macOS menu bar end to end, on the main thread (`harness = false`): install the sample bar through
//! `craft_menu::mac::install`, read AppKit's menu bar back, update it in place, then choose items the ways AppKit
//! can: a key equivalent (a real ⌘R key-down through `-[NSMenu performKeyEquivalent:]`) must reach egui as the key
//! press it was, a click (`-[NSMenu performActionForItemAtIndex:]`) must come back from `take_clicks`, also when the
//! click is ↩ inside the open menu, and a click made with ⌥ held on a row with an alternate runs the alternate.

use super::sample;

pub(super) fn run() {
    use craft_keymap::Mods;
    use craft_menu::{Alternate, CommandRef, Document, Gate, Item, LayoutOptions, MenuRole, Node, mac, mac_layout};
    use craft_ui::testing::Frames;
    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2_app_kit::{NSApplication, NSEvent, NSEventMask, NSEventModifierFlags, NSEventSubtype, NSEventType, NSMenu};
    use objc2_foundation::{NSPoint, NSString};

    let Some(mtm) = MainThreadMarker::new() else {
        println!("mac_appkit: harness = false runs on the main thread");
        std::process::exit(1);
    };
    let nsapp = NSApplication::sharedApplication(mtm);
    nsapp.finishLaunching();

    let titles = |menu: &NSMenu| -> Vec<String> {
        (0..menu.numberOfItems()).filter_map(|i| menu.itemAtIndex(i)).filter(|it| !it.isSeparatorItem()).map(|it| it.title().to_string()).collect()
    };
    let submenu = |menu: &NSMenu, title: &str| -> Retained<NSMenu> {
        (0..menu.numberOfItems())
            .filter_map(|i| menu.itemAtIndex(i))
            .find(|it| it.title().to_string() == title)
            .and_then(|it| it.submenu())
            .unwrap_or_else(|| panic!("no {title} submenu in {:?}", titles(menu)))
    };
    let index = |menu: &NSMenu, title: &str| -> isize {
        (0..menu.numberOfItems())
            .find(|&i| menu.itemAtIndex(i).is_some_and(|it| it.title().to_string() == title))
            .unwrap_or_else(|| panic!("no {title} in {:?}", titles(menu)))
    };
    let main_menu = || nsapp.mainMenu().expect("a main menu");
    // Make `event` AppKit's current event, as the run loop does before dispatching it.
    let dequeue = |event: &NSEvent| {
        nsapp.postEvent_atStart(event, true);
        let mode = NSString::from_str("kCFRunLoopDefaultMode");
        let got = nsapp.nextEventMatchingMask_untilDate_inMode_dequeue(NSEventMask::Any, None, &mode, true).expect("the posted event");
        assert_eq!(got.r#type(), event.r#type());
        got
    };
    let key_down = |chars: &str, code: u16, flags: NSEventModifierFlags| {
        let s = NSString::from_str(chars);
        NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            NSEventType::KeyDown,
            NSPoint::new(0.0, 0.0),
            flags,
            0.0,
            0,
            None,
            &s,
            &s,
            false,
            code,
        )
        .expect("a key event")
    };
    // The current event of a mouse click on a menu row: not a key-down, with `flags` held.
    let other = |flags: NSEventModifierFlags| {
        NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
            NSEventType::ApplicationDefined,
            NSPoint::new(0.0, 0.0),
            flags,
            0.0,
            0,
            None,
            NSEventSubtype::ApplicationActivated.0,
            0,
            0,
        )
        .expect("an application-defined event")
    };

    // The sample bar with View › Rulers on ⌘R, and Merge Visible with Stamp Visible as its ⌥ alternate.
    let bar = |documents: usize| {
        let mut bar = sample::sample(false, true);
        if let Some(view) = bar.menus.iter_mut().find(|m| m.role == MenuRole::View) {
            view.children.push(Node::Item(Item::new("view.rulers", "Rulers").shortcut(sample::chord("Cmd+R"))));
            view.children
                .push(Node::Item(Item::new("layer.mergeVisible", "Merge Visible").alternate(Alternate::new(Mods::ALT, "layer.stampVisible"))));
        }
        for n in 1..documents {
            bar.documents.push(Document::new(format!("doc.{n}").as_str(), format!("Untitled-{}", n + 1)));
        }
        bar
    };
    let options = LayoutOptions::new("Sample");
    let layout = |documents: usize| mac_layout(&bar(documents), &options);

    let mut frames = Frames::new();
    frames.ctx.set_os(egui::os::OperatingSystem::Mac);
    let Some(mut menu) = mac::install(&frames.ctx, &layout(1).bar) else {
        println!("mac_appkit: the menu bar did not install");
        std::process::exit(1);
    };
    // One frame, as an app runs it: poll before and after, run the clicks. Returns what egui saw of the input and
    // the commands the menu returned.
    let pass = |menu: &mut craft_menu::NativeMenu, frames: &mut Frames| -> (Vec<egui::Event>, Vec<CommandRef>) {
        menu.poll(&frames.ctx);
        let mut seen = Vec::new();
        let mut clicks = Vec::new();
        frames.frame(vec![], |ui| {
            seen = ui.ctx().input(|input| input.events.clone());
            clicks = menu.take_clicks(ui.ctx());
        });
        menu.poll(&frames.ctx);
        (seen, clicks)
    };

    // The Mac layout, as AppKit has it.
    let main = main_menu();
    let tops: Vec<String> = (0..main.numberOfItems()).filter_map(|i| main.itemAtIndex(i)?.submenu()).map(|m| m.title().to_string()).collect();
    assert_eq!(tops, ["Sample", "File", "Edit", "View", "Window", "Help"]);
    let app_menu = submenu(&main, "Sample");
    let quit = app_menu.itemAtIndex(index(&app_menu, "Quit Sample")).expect("Quit");
    assert_eq!(quit.keyEquivalent().to_string(), "q");
    assert_eq!(quit.keyEquivalentModifierMask(), NSEventModifierFlags::Command);
    assert!(!titles(&submenu(&main, "File")).iter().any(|n| n == "Exit"), "Exit moved to the app menu as Quit");
    assert_eq!(nsapp.windowsMenu().map(|m| m.title().to_string()).as_deref(), Some("Window"), "AppKit knows the Window menu");
    assert_eq!(nsapp.helpMenu().map(|m| m.title().to_string()).as_deref(), Some("Help"), "AppKit knows the Help menu");

    // A gate change greys File › New in place: same NSMenu, no rebuild.
    let new_enabled = |bar: &NSMenu| {
        let file = submenu(bar, "File");
        file.itemAtIndex(index(&file, "New")).expect("New").isEnabled()
    };
    assert!(new_enabled(&main), "no dialog: New is enabled");
    menu.set_gate(Gate::default().modal(true));
    menu.sync_with(&1_u64, || layout(1));
    assert!(Retained::as_ptr(&main) == Retained::as_ptr(&main_menu()), "the gate updated the menu, not rebuilt it");
    assert!(!new_enabled(&main_menu()), "a dialog greys New");
    menu.set_gate(Gate::default());
    menu.sync_with(&1_u64, || layout(1));
    assert!(new_enabled(&main_menu()), "New is enabled again");

    // A key equivalent: ⌘R reaches egui as the key press ⌘R, once, and is no click.
    let press = dequeue(&key_down("r", 15, NSEventModifierFlags::Command));
    assert!(main_menu().performKeyEquivalent(&press), "⌘R is View › Rulers' key equivalent");
    let (seen, clicks) = pass(&mut menu, &mut frames);
    let keys: Vec<_> = seen
        .iter()
        .filter_map(|e| if let egui::Event::Key { key, pressed, modifiers, .. } = e { Some((*key, *pressed, modifiers.command)) } else { None })
        .collect();
    // AppKit took only the key-down; winit delivers the key-up itself.
    assert_eq!(keys, [(egui::Key::R, true, true)], "the key press");
    assert!(clicks.is_empty(), "a key equivalent is no click: {clicks:?}");

    // A click: View › Rulers chosen with the mouse comes back as a click, and no key press reaches egui.
    let view = submenu(&main_menu(), "View");
    let rulers = index(&view, "Rulers");
    dequeue(&other(NSEventModifierFlags::empty()));
    view.performActionForItemAtIndex(rulers);
    let (seen, clicks) = pass(&mut menu, &mut frames);
    assert!(!seen.iter().any(|e| matches!(e, egui::Event::Key { .. })), "a click is no key press: {seen:?}");
    assert_eq!(clicks, [CommandRef::new("view.rulers")], "the click");

    // ↩ inside the open menu is a key-down without ⌘: still a click.
    dequeue(&key_down("\r", 36, NSEventModifierFlags::empty()));
    view.performActionForItemAtIndex(rulers);
    let (seen, clicks) = pass(&mut menu, &mut frames);
    assert!(!seen.iter().any(|e| matches!(e, egui::Event::Key { .. })), "↩ in the menu is a click: {seen:?}");
    assert_eq!(clicks, [CommandRef::new("view.rulers")], "↩ on Rulers");

    // A row with an alternate: a plain click runs the row's command, a click with ⌥ held runs the alternate.
    let merge = index(&view, "Merge Visible");
    dequeue(&other(NSEventModifierFlags::empty()));
    view.performActionForItemAtIndex(merge);
    assert_eq!(pass(&mut menu, &mut frames).1, [CommandRef::new("layer.mergeVisible")], "no ⌥: the row's own command");
    dequeue(&other(NSEventModifierFlags::Option));
    view.performActionForItemAtIndex(merge);
    assert_eq!(pass(&mut menu, &mut frames).1, [CommandRef::new("layer.stampVisible")], "⌥-click: the alternate");

    // Window ends with the open documents. Opening one rebuilds only the Window menu, which AppKit still knows as
    // its Window menu; choosing a row brings that document forward.
    menu.sync_with(&2_u64, || layout(2));
    assert!(Retained::as_ptr(&main) == Retained::as_ptr(&main_menu()), "a new document rebuilt only the Window menu");
    let window = submenu(&main_menu(), "Window");
    assert!(nsapp.windowsMenu().is_some_and(|w| Retained::as_ptr(&w) == Retained::as_ptr(&window)), "AppKit knows the new Window menu");
    let rows = [window.numberOfItems() - 2, window.numberOfItems() - 1];
    let shown: Vec<String> = rows.iter().filter_map(|&i| window.itemAtIndex(i)).map(|it| it.title().to_string()).collect();
    assert_eq!(shown, ["Untitled-1", "Untitled-2"], "the documents, after Bring All to Front: {:?}", titles(&window));
    dequeue(&other(NSEventModifierFlags::empty()));
    window.performActionForItemAtIndex(rows[1]);
    assert_eq!(pass(&mut menu, &mut frames).1, [CommandRef::new("doc.1")], "choosing the second row brings that document forward");

    println!("mac_appkit: ok");
}
