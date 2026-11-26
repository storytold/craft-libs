//! A command picked from a menu acts where focus was before the menu opened: the pick closes the menu, focus returns
//! to the stop that held it, and only then does the command run. Undo picked from the Edit menu, or found through the
//! Help search, undoes the typing in the text field that held focus. Any other activation that closes a menu returns
//! focus the same way; Escape is no activation and leaves focus on the bar title.

use craft_host::target::Os;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::menu::MenuItem;
use craft_ui::menu::bar::{MenuBar, MenuHelp, MenuTitle};
use craft_ui::menu::search::HelpEntry;
use craft_ui::runtime::commands::EditCommands;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::TextField;
use egui::Id;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The fixture with the counter and the focused id the scoped trial reads back.
type ScopedRig = (Fixture, Rc<Cell<u32>>, Rc<Cell<Option<Id>>>);

/// A menu bar with Edit (Undo, Redo) and Help (the search over Undo), then a text field holding "hello" that has
/// focus and the typed run "abc".
struct Rig {
    fixture: Fixture,
    text: Rc<RefCell<String>>,
    field: Rc<Cell<Option<Id>>>,
}

fn rig() -> Rig {
    let text = Rc::new(RefCell::new("hello".to_owned()));
    let field = Rc::new(Cell::new(None));
    let shown = (text.clone(), field.clone());
    let mut fixture = Fixture::new(move |ui| {
        let (text, field) = &shown;
        let titles = vec![
            MenuTitle::new("&Edit", |ui| {
                MenuItem::command(EditCommands::Undo).show(ui);
                MenuItem::command(EditCommands::Redo).show(ui);
            }),
            MenuTitle::new("&Help", |ui| {
                MenuItem::new("&About").show(ui);
            }),
        ];
        let help = || vec![HelpEntry::new(EditCommands::Undo, "Undo").with_path(["Edit"])];
        MenuBar::new("Main menu", titles).help(MenuHelp::new(1, &help)).show(ui);
        let mut held = text.borrow_mut();
        field.set(Some(TextField::new("Name", Bind::bound(&mut *held)).show(ui).response.id));
    });
    // The in-window menu bar and its Alt, F10 and letter keys are PC behaviour: a Mac app's menu bar is native.
    fixture.os(Os::Linux);
    fixture.settle();
    let name = fixture.by_label("Name").expect("the text field");
    fixture.click(&name);
    fixture.type_text("abc");
    // The typing run ends, so one undo takes back all of it.
    fixture.advance(2.0);
    fixture.settle();
    assert_eq!(*text.borrow(), "helloabc", "the typing reaches the field");
    Rig { fixture, text, field }
}

impl Rig {
    fn click(&mut self, label: &str) {
        let target = self.fixture.by_label(label).unwrap_or_else(|| panic!("{label:?} shows"));
        self.fixture.click(&target);
        self.fixture.settle();
    }

    fn assert_undone_in_the_field(&self, path: &str) {
        assert_eq!(*self.text.borrow(), "hello", "{path}: Undo takes back the typing in the field");
        assert_eq!(self.fixture.focused_stop().map(|(id, _)| id), self.field.get(), "{path}: focus is back in the field");
        assert!(self.fixture.by_label("Redo").is_none() && self.fixture.by_label("About").is_none(), "{path}: the pick closes the menu");
    }
}

#[trial]
fn undo_picked_from_the_edit_menu_undoes_in_the_field_that_held_focus() {
    let mut rig = rig();
    rig.click("Edit");
    rig.click("Undo");
    rig.assert_undone_in_the_field("Edit › Undo");
}

/// The command of the scope around the target button in the bar-keys trial.
#[derive(Clone, Copy, Debug, PartialEq, Eq, craft_ui::contract::CommandSet)]
#[command(prefix = "barKeys", scope = "trial.barKeys")]
#[non_exhaustive]
pub enum BarKeysCommand {
    #[command(label = "Toggle", default = "Cmd+Alt+P")]
    Toggle,
    #[command(label = "Tool", default = "B")]
    Tool,
}

#[trial]
fn a_shortcut_pressed_on_the_focused_bar_runs_once_where_focus_was_and_gives_focus_back() {
    use craft_ui::contract::ShowContainer as _;
    use craft_ui::runtime::commands::Commands as _;
    let count = Rc::new(Cell::new(0));
    let target = Rc::new(Cell::new(None));
    let (seen, stop) = (count.clone(), target.clone());
    let mut fixture = Fixture::new(move |ui| {
        let titles = vec![MenuTitle::new("&Edit", |ui| {
            MenuItem::command(EditCommands::Undo).show(ui);
        })];
        MenuBar::new("Main menu", titles).show(ui);
        craft_ui::layout_components::Stack::new().command_scope(<BarKeysCommand as craft_ui::contract::CommandSet>::SCOPE).show(ui, |ui, cx| {
            while cx.take_command::<BarKeysCommand>().is_some() {
                seen.set(seen.get() + 1);
            }
            stop.set(Some(craft_ui::buttons::Button::new("Target").show(ui).id));
        });
    });
    fixture.os(Os::Linux);
    fixture.settle();
    craft_ui::runtime::focus::request(fixture.ctx(), craft_ui::runtime::focus::FocusTarget::Id(target.get().expect("the button ran")));
    fixture.settle();
    fixture.key(egui::Key::F10, egui::Modifiers::NONE);
    fixture.settle();
    assert_ne!(fixture.focused_stop().map(|(id, _)| id), target.get(), "F10 takes the focus to the bar");
    fixture.key(egui::Key::P, egui::Modifiers::COMMAND | egui::Modifiers::ALT);
    fixture.settle();
    assert_eq!(count.get(), 1, "the shortcut runs once, in the scope of the stop the bar took the focus from");
    assert_eq!(fixture.focused_stop().map(|(id, _)| id), target.get(), "and the focus is back on that stop");
}

/// The commands of the access-key trial: one on a letter a title marks, one on a letter none marks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, craft_ui::contract::CommandSet)]
#[command(prefix = "barAccess", scope = "trial.barAccess")]
#[non_exhaustive]
pub enum BarAccessCommand {
    #[command(label = "Brush", default = "B")]
    Brush,
    #[command(label = "Quick", default = "Q")]
    Quick,
}

/// On the bar that a lone Alt or F10 focused, a plain letter is an access key, as in production's menu access mode:
/// the letter a title marks opens that title, not the shortcut on the same letter, and a letter no title marks is
/// swallowed, its shortcut unrun and the bar still focused.
#[trial]
fn a_plain_letter_on_the_focused_bar_opens_its_title_and_runs_no_shortcut() {
    use craft_ui::contract::ShowContainer as _;
    use craft_ui::runtime::commands::Commands as _;
    let ran = Rc::new(RefCell::new(Vec::new()));
    let target = Rc::new(Cell::new(None));
    let (seen, stop) = (ran.clone(), target.clone());
    let mut fixture = Fixture::new(move |ui| {
        let titles = vec![
            MenuTitle::new("&Edit", |ui| {
                MenuItem::command(EditCommands::Undo).show(ui);
            }),
            MenuTitle::new("&Brushes", |ui| {
                MenuItem::new("&Round").show(ui);
            }),
        ];
        MenuBar::new("Main menu", titles).show(ui);
        craft_ui::layout_components::Stack::new().command_scope(<BarAccessCommand as craft_ui::contract::CommandSet>::SCOPE).show(ui, |ui, cx| {
            while let Some((command, _)) = cx.take_command::<BarAccessCommand>() {
                seen.borrow_mut().push(command);
            }
            stop.set(Some(craft_ui::buttons::Button::new("Target").show(ui).id));
        });
    });
    fixture.os(Os::Linux);
    fixture.settle();
    let button = target.get().expect("the button ran");
    for alt in [true, false] {
        for (key, text) in [(egui::Key::B, "b"), (egui::Key::Q, "q")] {
            craft_ui::runtime::focus::request(fixture.ctx(), craft_ui::runtime::focus::FocusTarget::Id(button));
            fixture.settle();
            if alt {
                fixture.hold(egui::Modifiers::ALT).settle();
                fixture.hold(egui::Modifiers::NONE).settle();
            } else {
                fixture.key(egui::Key::F10, egui::Modifiers::NONE);
                fixture.settle();
            }
            let bar = fixture.focused_stop().map(|(id, _)| id);
            assert!(bar.is_some() && bar != Some(button), "the bar takes the keyboard (alt {alt})");
            fixture.type_key(key, text);
            fixture.settle();
            assert!(ran.borrow().is_empty(), "{text:?} on the bar runs no shortcut (alt {alt}): {:?}", ran.borrow());
            if key == egui::Key::B {
                assert!(fixture.by_label("Round").is_some(), "b opens Brushes, whose access key it is (alt {alt})");
                fixture.key(egui::Key::Escape, egui::Modifiers::NONE);
                fixture.key(egui::Key::Escape, egui::Modifiers::NONE);
            } else {
                assert!(fixture.by_label("Round").is_none() && fixture.by_label("Undo").is_none(), "q opens no menu (alt {alt})");
                assert_eq!(fixture.focused_stop().map(|(id, _)| id), bar, "and the bar keeps the keyboard (alt {alt})");
                fixture.key(egui::Key::Escape, egui::Modifiers::NONE);
            }
            fixture.settle();
        }
    }
    // Off the bar the letters are the shortcuts they are bound to.
    craft_ui::runtime::focus::request(fixture.ctx(), craft_ui::runtime::focus::FocusTarget::Id(button));
    fixture.settle();
    fixture.type_key(egui::Key::Q, "q");
    fixture.settle();
    assert_eq!(*ran.borrow(), [BarAccessCommand::Quick], "q off the bar runs its shortcut");
}

/// A command chord pressed while a menu is open closes the menu and runs where focus was, as with the menu closed
/// (#9065); the open menu's own keys, the letters its rows' mnemonics name among them, still drive it, and a plain
/// key no row names reaches the app's shortcuts.
#[trial]
fn a_command_shortcut_closes_an_open_menu_and_runs() {
    use craft_ui::contract::ShowContainer as _;
    use craft_ui::runtime::commands::Commands as _;
    let count = Rc::new(Cell::new(0));
    let tools = Rc::new(Cell::new(0));
    let target = Rc::new(Cell::new(None));
    let (seen, seen_tools, stop) = (count.clone(), tools.clone(), target.clone());
    let mut fixture = Fixture::new(move |ui| {
        let titles = vec![MenuTitle::new("&Edit", |ui| {
            MenuItem::command(EditCommands::Undo).show(ui);
            MenuItem::new("&Zoom").show(ui);
        })];
        MenuBar::new("Main menu", titles).show(ui);
        craft_ui::layout_components::Stack::new().command_scope(<BarKeysCommand as craft_ui::contract::CommandSet>::SCOPE).show(ui, |ui, cx| {
            while let Some((command, _)) = cx.take_command::<BarKeysCommand>() {
                let counter = if command == BarKeysCommand::Tool { &seen_tools } else { &seen };
                counter.set(counter.get() + 1);
            }
            stop.set(Some(craft_ui::buttons::Button::new("Target").show(ui).id));
        });
    });
    fixture.os(Os::Linux);
    fixture.settle();
    craft_ui::runtime::focus::request(fixture.ctx(), craft_ui::runtime::focus::FocusTarget::Id(target.get().expect("the button ran")));
    fixture.settle();
    let open_edit = |fixture: &mut Fixture| {
        let edit = fixture.by_label("Edit").expect("the Edit title");
        fixture.click(&edit);
        fixture.settle();
        assert!(fixture.by_label("Zoom").is_some(), "Edit is open");
    };

    open_edit(&mut fixture);
    fixture.key(egui::Key::P, egui::Modifiers::COMMAND | egui::Modifiers::ALT);
    fixture.settle();
    assert_eq!(count.get(), 1, "the shortcut runs once with the menu open, in the scope focus came from");
    assert!(fixture.by_label("Zoom").is_none(), "and closes the menu");
    assert_eq!(fixture.focused_stop().map(|(id, _)| id), target.get(), "focus is back on the stop that held it");

    open_edit(&mut fixture);
    fixture.type_key(egui::Key::Z, "z");
    fixture.settle();
    assert!(fixture.by_label("Zoom").is_none(), "a plain letter still picks the row it marks, which closes the menu");
    assert_eq!((count.get(), tools.get()), (1, 0), "and runs no shortcut");
    assert_eq!(fixture.focused_stop().map(|(id, _)| id), target.get());

    open_edit(&mut fixture);
    fixture.type_key(egui::Key::B, "b");
    fixture.settle();
    assert_eq!(tools.get(), 1, "a letter no row names runs its shortcut under the open menu");
}

/// ⌘Z and ⌘C are the Help search field's own editing chords: with the field focused they stay with it, Help stays open,
/// and no app command runs (#9314). The text-field rule of `holds_press` meets the command-chord rule of an open menu
/// here.
#[trial]
fn undo_and_copy_stay_with_the_help_search_field() {
    let mut rig = rig();
    rig.click("Help");
    rig.fixture.type_text("und");
    rig.fixture.settle();
    let search = rig.fixture.focused_stop().map(|(id, _)| id);
    assert!(search.is_some() && search != rig.field.get(), "Help opens with its search field focused");
    for key in [egui::Key::Z, egui::Key::C] {
        rig.fixture.key(key, egui::Modifiers::COMMAND);
        rig.fixture.settle();
        assert!(rig.fixture.by_label("About").is_some(), "⌘{key:?} keeps Help open");
        assert_eq!(rig.fixture.focused_stop().map(|(id, _)| id), search, "⌘{key:?} keeps the focus in the search field");
    }
    assert_eq!(*rig.text.borrow(), "helloabc", "no app Undo ran in the field that held focus before Help opened");
}

/// A bar whose Help searches the target scope's Toggle, then the Target button inside that scope; returns the count
/// of Toggle runs and the button's id.
fn scoped_rig() -> ScopedRig {
    use craft_ui::contract::ShowContainer as _;
    use craft_ui::runtime::commands::Commands as _;
    let count = Rc::new(Cell::new(0));
    let target = Rc::new(Cell::new(None));
    let (seen, stop) = (count.clone(), target.clone());
    let mut fixture = Fixture::new(move |ui| {
        let titles = vec![MenuTitle::new("&Help", |ui| {
            MenuItem::new("&About").show(ui);
        })];
        let help = || vec![HelpEntry::new(BarKeysCommand::Toggle, "Toggle")];
        MenuBar::new("Main menu", titles).help(MenuHelp::new(0, &help)).show(ui);
        craft_ui::layout_components::Stack::new().command_scope(<BarKeysCommand as craft_ui::contract::CommandSet>::SCOPE).show(ui, |ui, cx| {
            while cx.take_command::<BarKeysCommand>().is_some() {
                seen.set(seen.get() + 1);
            }
            stop.set(Some(craft_ui::buttons::Button::new("Target").show(ui).id));
        });
    });
    fixture.os(Os::Linux);
    fixture.settle();
    craft_ui::runtime::focus::request(fixture.ctx(), craft_ui::runtime::focus::FocusTarget::Id(target.get().expect("the button ran")));
    fixture.settle();
    (fixture, count, target)
}

#[trial]
fn opening_help_focuses_its_search_and_down_then_enter_runs_the_first_match() {
    let (mut fixture, count, target) = scoped_rig();
    let help = fixture.by_label("Help").expect("the Help title");
    fixture.click(&help);
    fixture.settle();
    fixture.type_text("tog");
    fixture.settle();
    assert!(fixture.by_label("Toggle").is_some(), "the typed text lands in the search field and finds Toggle");
    fixture.key(egui::Key::ArrowDown, egui::Modifiers::NONE);
    fixture.settle();
    fixture.key(egui::Key::Enter, egui::Modifiers::NONE);
    fixture.settle();
    assert_eq!(count.get(), 1, "Down then Enter runs the first match once");
    assert!(fixture.by_label("About").is_none(), "and the menu closes");
    assert_eq!(fixture.focused_stop().map(|(id, _)| id), target.get(), "with the focus back on the stop that held it");
}

/// The Help search edits as any text field does inside the open menu: Backspace removes a letter, and Left moves the
/// caret, so the next letter lands before the last one.
#[trial]
fn the_help_search_takes_its_editing_keys() {
    let (mut fixture, _, _) = scoped_rig();
    let help = fixture.by_label("Help").expect("the Help title");
    fixture.click(&help);
    fixture.settle();
    fixture.type_text("tog");
    fixture.settle();
    let field_reads = |fixture: &Fixture, text: &str| fixture.texts().iter().any(|run| run.text == text);
    fixture.key(egui::Key::Backspace, egui::Modifiers::NONE);
    fixture.settle();
    assert!(field_reads(&fixture, "to"), "Backspace removes the last letter");
    fixture.key(egui::Key::ArrowLeft, egui::Modifiers::NONE);
    fixture.settle();
    fixture.type_text("x");
    fixture.settle();
    assert!(field_reads(&fixture, "txo"), "Left moves the caret before the last letter");
}

/// A command chord pressed after the focus moved between the rows of an open menu still runs in the scope focus came
/// from and gives the focus back there, for a menu a title click opened and one F10 and Down opened.
#[trial]
fn a_command_shortcut_after_moving_in_an_open_menu_runs_where_focus_was() {
    use craft_ui::contract::ShowContainer as _;
    use craft_ui::runtime::commands::Commands as _;
    let count = Rc::new(Cell::new(0));
    let target = Rc::new(Cell::new(None));
    let (seen, stop) = (count.clone(), target.clone());
    let mut fixture = Fixture::new(move |ui| {
        let titles = vec![MenuTitle::new("&View", |ui| {
            MenuItem::new("&One").show(ui);
            MenuItem::new("&Two").show(ui);
            MenuItem::new("T&hree").show(ui);
        })];
        MenuBar::new("Main menu", titles).show(ui);
        craft_ui::layout_components::Stack::new().command_scope(<BarKeysCommand as craft_ui::contract::CommandSet>::SCOPE).show(ui, |ui, cx| {
            while cx.take_command::<BarKeysCommand>().is_some() {
                seen.set(seen.get() + 1);
            }
            stop.set(Some(craft_ui::buttons::Button::new("Target").show(ui).id));
        });
    });
    fixture.os(Os::Linux);
    fixture.settle();
    craft_ui::runtime::focus::request(fixture.ctx(), craft_ui::runtime::focus::FocusTarget::Id(target.get().expect("the button ran")));
    fixture.settle();
    let press = |fixture: &mut Fixture, key: egui::Key, modifiers: egui::Modifiers| {
        fixture.key(key, modifiers);
        fixture.settle();
    };
    let down_twice_then_chord = |fixture: &mut Fixture, how: &str| {
        assert!(fixture.by_label("Two").is_some(), "{how}: View is open");
        press(fixture, egui::Key::ArrowDown, egui::Modifiers::NONE);
        press(fixture, egui::Key::ArrowDown, egui::Modifiers::NONE);
        assert_ne!(fixture.focused_stop().map(|(id, _)| id), target.get(), "{how}: the focus is on a row of the menu");
        press(fixture, egui::Key::P, egui::Modifiers::COMMAND | egui::Modifiers::ALT);
        assert!(fixture.by_label("Two").is_none(), "{how}: the chord closes the menu");
        assert_eq!(fixture.focused_stop().map(|(id, _)| id), target.get(), "{how}: the focus is back on the stop that held it");
    };

    let view = fixture.by_label("View").expect("the View title");
    fixture.click(&view);
    fixture.settle();
    down_twice_then_chord(&mut fixture, "opened by a click");
    assert_eq!(count.get(), 1, "opened by a click: the chord runs once, in the scope focus came from");

    press(&mut fixture, egui::Key::F10, egui::Modifiers::NONE);
    press(&mut fixture, egui::Key::ArrowDown, egui::Modifiers::NONE);
    down_twice_then_chord(&mut fixture, "opened by F10 and Down");
    assert_eq!(count.get(), 2, "opened by F10 and Down: the chord runs once, in the scope focus came from");
}

/// A field inside an open menu keeps its own edit menu: Cut picked from the Help search field's edit menu cuts the
/// search text, not the text of the field that held focus before Help opened, and closes only the edit menu.
#[trial]
fn cut_from_the_help_search_fields_edit_menu_cuts_the_search_text() {
    use craft_ui::testing::pointer;
    let mut rig = rig();
    rig.click("Help");
    rig.fixture.type_text("und");
    rig.fixture.key(egui::Key::A, egui::Modifiers::COMMAND);
    rig.fixture.settle();
    let held = rig.fixture.focused_stop().map(|(id, _)| id);
    // Well after the title click: egui counts presses of any button into one click run.
    rig.fixture.advance(1.0);
    let search = rig.fixture.by_label("Search menus").expect("the search field").center();
    rig.fixture.frame(vec![pointer::secondary(search, true)]);
    rig.fixture.frame(vec![pointer::secondary(search, false)]);
    rig.fixture.settle();
    let cut = rig.fixture.by_label("Cut").expect("the search field's edit menu");
    assert!(!rig.fixture.tree().is_disabled_key(cut.node), "Cut follows the search field's selection");
    let at = cut.center();
    let mut copied = Vec::new();
    let click = [vec![pointer::moved(at)], vec![pointer::press(at)], vec![pointer::release(at)]];
    for events in click.into_iter().chain(std::iter::repeat_n(Vec::new(), 8)) {
        rig.fixture.frame(events);
        copied.extend_from_slice(rig.fixture.copied());
    }
    assert_eq!(copied, ["und"], "Cut takes the search text");
    assert_eq!(*rig.text.borrow(), "helloabc", "and leaves the field under the menus alone");
    assert!(rig.fixture.by_label("About").is_some(), "the pick closes only the edit menu: Help stays open");
    assert_eq!(rig.fixture.focused_stop().map(|(id, _)| id), held, "focus is back in the search field");
}

/// A menu opened from the bar while a text field (the zoom field) holds focus takes the letters and arrows: Down moves
/// in the menu and a letter picks the row it marks, while the field's text stays as it was; the pick gives focus back
/// to the field.
#[trial]
fn an_open_menu_takes_letters_and_arrows_from_a_focused_text_field() {
    let text = Rc::new(RefCell::new("100%".to_owned()));
    let (field, zoomed) = (Rc::new(Cell::new(None)), Rc::new(Cell::new(0_u32)));
    let shown = (text.clone(), field.clone(), zoomed.clone());
    let mut fixture = Fixture::new(move |ui| {
        let (text, field, zoomed) = &shown;
        let titles = vec![MenuTitle::new("&Edit", |ui| {
            if MenuItem::new("&Zoom").show(ui).ran {
                zoomed.set(zoomed.get() + 1);
            }
            MenuItem::new("&Undo").show(ui);
        })];
        MenuBar::new("Main menu", titles).show(ui);
        let mut held = text.borrow_mut();
        field.set(Some(TextField::new("Zoom level", Bind::bound(&mut *held)).show(ui).response.id));
    });
    fixture.os(Os::Linux);
    fixture.settle();
    let name = fixture.by_label("Zoom level").expect("the text field");
    fixture.click(&name);
    fixture.settle();
    assert_eq!(fixture.focused_stop().map(|(id, _)| id), field.get(), "the field holds focus");

    let edit = fixture.by_label("Edit").expect("the Edit title");
    fixture.click(&edit);
    fixture.settle();
    assert!(fixture.by_label("Undo").is_some(), "Edit is open");
    fixture.key(egui::Key::ArrowDown, egui::Modifiers::NONE);
    fixture.settle();
    assert!(fixture.by_label("Undo").is_some(), "Down keeps the menu open");
    assert_ne!(fixture.focused_stop().map(|(id, _)| id), field.get(), "Down moves in the menu, not in the field");
    assert_eq!(*text.borrow(), "100%", "the field's text stays");

    fixture.type_key(egui::Key::Z, "z");
    fixture.settle();
    assert_eq!(zoomed.get(), 1, "the letter picks the row it marks");
    assert!(fixture.by_label("Undo").is_none(), "and closes the menu");
    assert_eq!(*text.borrow(), "100%", "the letter does not reach the field");
    assert_eq!(fixture.focused_stop().map(|(id, _)| id), field.get(), "focus is back in the field");
}
