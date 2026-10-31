# craft-menu

A Craft app's menus, laid out for macOS, Windows, Linux and the web. The app supplies its menu
tree and runs its commands; this crate lays the menus out the way each platform's apps do, takes
shortcuts from craft-keymap, drives a platform `Backend`, and turns the menu's key
equivalents back into the input egui would have had without it. It depends on egui,
[`craft-keymap`](../keymap/src/lib.rs) (commands and chords) and [`craft-quit`](../quit/src/lib.rs),
and on no app. Everything but the macOS backend
(`craft_menu::mac`, built on muda) is platform-free and also builds for `wasm32`. The layout and driver come from PhotoCraft's `native_menu.rs`, the egui bar from its `menu_nav.rs` and in-window menu bar.

## The menus

Build a `MenuBar` directly, or from the flat rows an in-window bar draws (`from_items`, with
`Row { path, command, label, shortcut, enabled, kind }`, `---` for a separator). Give each menu
its role (`MenuRole::{App, File, Edit, View, Window, Help, Other}`), then tag what macOS puts in
fixed places:

```rust
let mut bar = craft_menu::from_items(&["File", "Edit", "View", "Window", "Help"], &rows);
bar.tag_menu("File", MenuRole::File);
bar.tag_menu("Edit", MenuRole::Edit);
bar.tag_menu("View", MenuRole::View);
bar.tag_menu("Window", MenuRole::Window);
bar.tag_menu("Help", MenuRole::Help);
bar.tag_item(&"help.about".into(), ItemRole::About);
bar.tag_item(&"file.exit".into(), ItemRole::Quit); // the app's quit, with its unsaved-changes prompt
bar.tag_submenu(MenuRole::Edit, "Preferences", ItemRole::Settings);
bar.tag_submenu(MenuRole::View, "Theme", ItemRole::Appearance);
```

Each item runs a `craft_keymap::CommandRef`, and a click reports it. A command that takes a
parameter carries it in the `CommandRef`:
`CommandRef::with_params("file.open_recent", json!({"index": 3}))`. Items built directly use
`Item::new(command, label)` with `.shortcut(chord)`, `.enabled(..)`, `.check(..)` or
`.radio(..)` (the item's `ItemKind`), `.role(..)` and `.platforms(..)`; submenus use
`Node::submenu(label, children)`, with `.role(..)` for a tagged one. A shortcut is a
`craft_keymap::Chord`: `cmd` is ⌘ and `ctrl` the Control key. macOS shows radio items as check
items. The app's open documents go in `MenuBar::documents` (`Document::new(command, name)`,
`.active(..)` for the one in front, `hidden` for one kept open but not listed), and the layouts
list them in the Window menu. The items the
layout adds itself run a `Builtin` command (Hide, Hide Others, Minimize, Quit), which the crate
handles.

## Layouts

`layout(&bar, target, &LayoutOptions::new("PhotoCraft"))` lays the menus out for a `Target`
(`Mac`, `Windows`, `Linux`, `Web`; `Target::current()` for this build) and returns the bar and
the clashes it resolved. Every layout first drops the items and submenus whose `.platforms(..)`
leave the target out, and lists the open documents (but the hidden ones) last in the Window menu
as radio items, the one in front selected. A submenu left empty goes; for an empty dynamic list
(no recent files) the app supplies a disabled placeholder item, which keeps the submenu.

| Target | About | Settings | Quit | Also |
|---|---|---|---|---|
| `Mac` | app menu | app menu, Settings… (⌘,) | app menu, ⌘Q | Services, Hide, Minimize, Zoom, Close Window |
| `Windows` | last in Help | last in Edit, Preferences… | last in File, Exit | |
| `Linux` | last in Help | last in Edit, Preferences… | last in File, Quit | |
| `Web` | last in Help | last in Edit, Preferences… | none | browser shortcuts removed |

The layouts find these items by role, never by title, and add a File, Edit, Window or Help menu
when the app has none to put them in. Language and Appearance stay where the app put them off
macOS, and the Windows, Linux and web layouts keep the app's shortcuts. The browser keeps Ctrl or
⌘ with W, N, T and Q, and with Shift+N, Shift+T and Shift+W, for its tabs and windows, so the
web layout takes those shortcuts off the items and reports each as a `Clash`.

`mac_layout(&bar, &LayoutOptions::new("PhotoCraft"))` returns the Mac menu bar and the clashes
it resolved:

- **App menu:** About, Settings (⌘,), Language, Appearance, Services, Hide (⌘H), Hide Others
  (⌥⌘H), Show All, Quit (⌘Q). The tagged items leave their old menus. Without a tagged Quit item
  the layout adds one with `craft_quit::QUIT_MENU_ID`. A menu the app tagged `MenuRole::App` is
  merged in: its other items follow Appearance.
- **Window:** Minimize, Zoom, the app's own Window items, Bring All to Front, then the open
  documents. A Window menu is added if the app has none. **File** gets Close Window (⌘W).
- **Clashes:** when the app binds ⌘H, ⌥⌘H, ⌘M, ⌘W, ⌘, or ⌘Q to a command, the command keeps
  it. Hide and Minimize move to ⌃⌘H and ⌃⌘M, the others go without a shortcut, and each case is
  reported as a `Clash`. Shortcuts bound outside the menus (tool and canvas keys) count when the
  app lists them: `LayoutOptions::new("PhotoCraft").extra_shortcuts(&[(chord, command), ..])`.

The labels the layouts write (`About {app}`, `Preferences…`, `Exit`, `Window`, `Minimize`, …)
are English. To translate them, pass `LayoutOptions { label, ..LayoutOptions::new(app_name) }`,
where `label` maps an English label such as `"Quit {app}"` to the app's translation; the crate
substitutes `{app}`.

## Keymap

`apply_keymap(&mut bar, &keymap)` gives each item the shortcut its command has in the app's
`craft_keymap::Keymap`, the user's overrides included: the first single-press binding, one in
the command's home scope first. A command the keymap knows but binds to nothing loses its
shortcut; an item whose command the keymap doesn't know keeps its own. Items of toggle commands
(`Bindable::toggle`) become check items, keeping their checked state; radio items stay radio
items.

Each frame, `apply_state(&mut bar, |command| ItemState { enabled, checked })` sets enabled and
checked from the app's state. `checked: Some(..)` makes a plain item a check item and selects or
clears a radio item; `None` leaves the kind as it is. The built-in items and the open documents
are left alone. Both run on the app's bar, before the layout.

## Windows, Linux and web: the egui bar

`MenuBarUi` draws a laid-out bar with egui, in a top panel, and returns the commands chosen that
frame:

```rust
let laid_out = craft_menu::layout(&menu_bar, Target::current(), &LayoutOptions::new("PhotoCraft")).bar;
egui::Panel::top("menu-bar").show(ui, |ui| {
    let gate = Gate { modal: dialog_open, recording_shortcut, text_editing: ui.ctx().text_edit_focused() };
    for command in MenuBarUi::new(&laid_out).gate(gate).label(&tr).show(ui) {
        app.run(&command);
    }
});
```

- **Mouse:** a press on a title opens its menu and releasing over an item runs it; a click on
  the title again, Esc or a click outside closes it. While a menu is open, moving onto another
  title opens that one. Submenus open on hover and stay open while the pointer heads for them
  across other rows. A menu taller than the window scrolls, with the wheel or by resting on the
  ▲ and ▼ arrows.
- **Keys, while a menu is open:** ↑ and ↓ move the highlight, → opens a submenu or the next
  menu, ← closes the submenu or opens the previous menu, ↩ and Space run the item, Esc closes.
  `craft_menu::bar::is_open(ctx)` tells the app's shortcut handling to wait meanwhile.
- **Keyboard access (Windows, Linux, web):** a tap on Alt alone, or F10, puts the keyboard on
  the bar: the first title is lit, ← and → move along the titles, ↓, ↩ and Space open the lit
  menu, Esc, a second tap or a click leaves. Alt+letter opens the menu with that mnemonic, and
  in an open menu a letter runs the row with that mnemonic or opens its submenu. Mnemonics are
  underlined while Alt is held, while the bar has the keyboard, and in a menu opened from the
  keyboard. Each menu level assigns them from the labels as shown, so translated: unique within
  the level, the first unused letter, word starts first; `.mnemonics(&[("View", 'W'),
  ("file.open", 'P')])` names preferred letters by title as shown or by command id. A label
  without an ASCII letter or digit shows its English label's letter after it, "文件(F)", before a
  trailing ellipsis ("開く(O)…"), when `.english(&hook)` returns that English label for the
  `Labeled::Command` or `Labeled::Title`; without one it has no mnemonic. While the bar has the keyboard the app's shortcuts wait, as
  for an open menu (`craft_menu::bar::is_focused(ctx)`); call
  `craft_menu::bar::leave_for_shortcut(ctx)` before them so that a command shortcut (Ctrl+S,
  F7) leaves the bar and runs. While a dialog is open or a shortcut is being recorded (`Gate`),
  Alt, F10 and the letters leave the bar alone. While a text field has the keys, a tap on Alt
  still focuses the bar, as on Windows, but a letter without Alt goes on to the field (and takes
  the keyboard off the bar) unless a menu is open. On `Target::Mac`, and on `Target::Web` in a
  browser on a Mac (`ctx.os()`), the bar has no mnemonics and Alt does nothing: Option is a typing
  key there.
- **Items:** disabled items, and the items the `Gate` forbids (`Gate::allows`), are greyed and
  never run. Check items show ✓, the selected radio item a dot, and the shortcut column shows
  every shortcut, the display-only ones too, in the target's names (⌘ glyphs on `Target::Mac`,
  Ctrl names on Windows and Linux, the browser's system's names on the web; `.target(..)` to
  choose).
- **Narrow windows:** the menus that don't fit move into a trailing `⋯` menu; when not even the
  first fits, one `Menu` title holds them all. `.label(&tr)` translates `⋯`, `Menu` and the
  scroll arrows' names, with the same hook as `LayoutOptions::label`. The bar fills its row;
  `.shrink_to_fit(max_width)` makes it take only the width its titles need, up to `max_width`,
  so other controls can share the row, and the menus past that overflow.
- **Quit:** without an app Quit item, the Windows and Linux layouts add `Builtin::Quit`
  (`craft_quit::QUIT_MENU_ID`). The bar runs it itself through `craft_quit::on_menu_event`,
  which asks the window to close so the unsaved-changes check runs, and doesn't return it; the
  same goes for `Builtin::Minimize`.
- **Accessibility:** AccessKit sees a menu bar of menu items, the open menus, check and radio
  items with their state, disabled items, and each item's shortcut and mnemonic (`access_key`).
- **Style:** `BarStyle::from_style(ui.style())` by default; `.style(..)` sets title padding and
  color, the menu width, separator room and the check and radio marks.

## macOS

Install the menu bar once, on the main thread, in eframe's creator closure (so winit's default
menu doesn't stay up). `None` means AppKit refused the menus: keep the in-window ones.

```rust
#[cfg(target_os = "macos")]
let native_menu = craft_menu::mac::install(&cc.egui_ctx, &craft_menu::mac_layout(&menu_bar, &LayoutOptions::new("PhotoCraft")).bar);
```

An app without a `MenuBar` yet installs the minimal app menu instead: About, Services, Hide,
Hide Others, Show All and Quit, where Quit asks the app first.

```rust
#[cfg(target_os = "macos")]
craft_menu::mac::install_app_menu(&cc.egui_ctx, "WordCraft")?;
```

craft-menu owns muda's menu event handler. muda keeps the first one set per process, so the
crate sets it once and both `mac::install` and `mac::install_app_menu` use it; a later
`mac::install` replaces the minimal app menu, and a second `mac::install` returns `None`.
`craft_quit::install_app_menu` is deprecated in favor of `mac::install_app_menu`: it sets its own
handler, so an app calls it or craft-menu's installers, never both. Called first, it leaves
craft-menu's menus without events; called after, it replaces their menu bar. Quit is
never AppKit's predefined Quit: it is the app's item tagged `ItemRole::Quit`, or the item with
`craft_quit::QUIT_MENU_ID`, which runs `craft_quit::on_menu_event` (the window is asked to close,
so the app's unsaved-changes prompt runs). Each menu event wakes egui, so a chosen item runs at
once rather than with the next input event.

The backend calls AppKit's Window and Help menu hooks for the menus with `MenuRole::Window` and
`MenuRole::Help`; it never looks at titles. Services is AppKit's own submenu. AppKit's Window
menu lists the app's windows after the menu's own items. An app that keeps its documents in one
window and lists them in the Window menu itself (`MenuBar::documents`) would see each twice, so it
installs with
`mac::install_with(ctx, &bar, MacOptions::default().window_list(false))`.

Only shortcuts with ⌘ or ⌃ become key equivalents. The others (`B`, `⇧B`, `⌥B`, a bare `F5`) are
tool keys the app handles itself; as key equivalents AppKit would take them from text fields.
The native menu does not show them: muda 0.21 gives no access to the `NSMenuItem`, and its
`set_attributed_title` needs an `NSAttributedString` with a font and a right tab stop, which
objc2 builds only through `unsafe` calls, and the workspace forbids unsafe code. The egui bar
shows them in its shortcut column.

Check and radio items are AppKit check items. AppKit flips one when it is chosen; the next sync
sets it back to the app's state, so the app's state always wins.

While a menu is open, a click or Escape in the window closes it and does not reach egui (no
canvas gesture, no tool action); pointer moves pass through. Only one click is kept this way
until the menu reports closed, so a menu wrongly reported open costs one click. Menu updates wait
until the menu closes. A rebuild that fails keeps the menus as they were.

## Per frame

Key equivalents stay with egui: the backend reports them as `Event::Key`, and `raw_input` feeds
each one to egui as egui-winit would have, so the app's own key handling (text fields, dialogs,
tool-specific undo) runs as without a native menu. ⌘C, ⌘X and ⌘V become egui's Copy, Cut and
Paste (`ClipboardKeys::Egui`, the default), so text fields copy and paste; an app that runs its
clipboard commands from the keys itself sets `menu.clipboard_keys(ClipboardKeys::Raw)` to get
key presses. A shortcut the layout assigned (⌘Q on the app's Quit item, ⌘, on Settings) comes
back as a click instead, since the app's key handling doesn't bind it.

The app tells the menu what may run with a `Gate`, every frame (`set_gate`):

| Gate field | Effect on the native menu |
| --- | --- |
| `modal` | The app's items are greyed (updated in place) and their clicks dropped, also clicks that arrived before the dialog opened. Hide, Hide Others, Minimize and Quit stay, and so do the items whose shortcut a text field handles (Undo, Copy, Select All): AppKit doesn't pass a disabled item's key equivalent on, so ⌘C and ⌘Z would die in the dialog's fields. |
| `recording_shortcut` | As `modal`, and every key equivalent reaches egui as the plain key press (⌘Q, ⌘H, ⌘C included), so the recorder sees it. |
| `text_editing` | A key equivalent comes back as a click (⌘S saves while typing), except the keys the field handles itself: ⌘C, ⌘X and ⌘V (as `ClipboardKeys` says) and `craft_keymap::text_editing_chords` (⌘A, ⌘Z, ⇧⌘Z, ⌘←, …). A mouse click on one of those items (Edit › Copy, Undo) is a press and release of its shortcut, so it acts on the field. |

Each frame, in this order:

1. `raw_input`, before egui reads the input (`eframe::App::raw_input_hook`). It uses the gate
   from the last `set_gate`.
2. `set_gate`, with the app's state for this frame.
3. `take_clicks`, and run the commands it returns: the clicks since the last frame that the gate
   allows now. Minimize and Quit are already handled.
4. `sync_with`, which lays the menus out again only when the app's state changed or a menu event
   arrived, and re-syncs them in place when the gate changed.

```rust
// eframe::App::raw_input_hook, before egui reads the input:
if let Some(menu) = &mut self.native_menu {
    menu.raw_input(raw_input);
}

// In the frame:
if let Some(menu) = &mut self.native_menu {
    menu.set_gate(Gate {
        modal: self.dialog.is_some(),
        recording_shortcut: self.shortcut_editor.is_recording(),
        text_editing: ctx.wants_keyboard_input(),
    });
    for command in menu.take_clicks(ctx) {
        self.run_command(ctx, &command);
    }
    let state = self.menu_state_hash(craft_menu::had_input(ctx).then_some(ctx.cumulative_frame_nr()));
    for c in menu.sync_with(state, || craft_menu::mac_layout(&self.menu_bar(), &LayoutOptions::new("PhotoCraft"))) {
        log::info!("menu: {} is {} on macOS and {} in the app; {}", c.shortcut, c.system, c.command, c.resolution);
    }
}
```

`sync_with` builds the layout only when `state` changed or a menu event arrived, because
building the rows usually walks every command's enabled state. The backend then updates labels,
enabled and checked in place and rebuilds only when `structure_key` (or, for the open
documents, `window_key`) changes. Adding or removing an item in a dynamic list (recent files,
workspaces, plugins) changes `structure_key`; relabelling one doesn't. An item that was a check
item once stays one, unchecked, when the app's state stops giving it a checked state, so that
doesn't rebuild the menus.

`state` must cover everything the menus show. A change that comes without input, such as a load
or save finishing on a worker or a timer, leaves the menus stale until the next input or menu
event unless it is in the hash. An app that can't keep the hash complete calls
`menu.sync(layout)` instead, with the layout built every frame: it is compared with the last
one and synced only when it differs.

An app that needs a click while egui is idle (to wake its own loop, or hand the command to a
worker) sets `menu.on_command(Box::new(|command| ..))`. The hook runs from the menu event
handler as the click arrives, on the main thread, for the app commands the gate allows; key
equivalents reported as key presses and the built-in items don't reach it. It returns `true`
when it ran the command; `take_clicks` leaves those out and returns the others.
