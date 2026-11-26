//! The menu bar: what the help search finds.

use craft_host::target::Os;
use craft_i18n::Direction;
use craft_ui::contract::Show as _;
use craft_ui::menu::MenuItem;
use craft_ui::menu::bar::{MenuBar, MenuHelp, MenuTitle};
use craft_ui::menu::search::HelpEntry;
use craft_ui::runtime::commands::EditCommands;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};

/// A menu bar with File, Edit and Help.
struct Rig {
    fixture: Fixture,
}

fn entries() -> Vec<HelpEntry> {
    // The help rows show their command's label, so each entry has its own command.
    vec![
        HelpEntry::new(EditCommands::Paste, "Café").with_path(["Edit"]),
        HelpEntry::new(EditCommands::Copy, "Copy").with_english("Duplicate").with_path(["Edit"]),
        HelpEntry::new(EditCommands::Cut, "Cut").with_path(["Edit", "Clipboard"]),
    ]
}

fn rig(direction: Direction) -> Rig {
    let mut fixture = Fixture::new(move |ui| {
        let titles = vec![
            MenuTitle::new("&File", |ui| {
                MenuItem::new("&New").show(ui);
                MenuItem::new("&Save").show(ui);
            }),
            MenuTitle::new("&Edit", |ui| {
                MenuItem::new("Cu&t").show(ui);
            }),
            MenuTitle::new("&Help", |ui| {
                MenuItem::new("&About").show(ui);
            }),
        ];
        MenuBar::new("Main menu", titles).help(MenuHelp::new(2, &entries)).show(ui);
    });
    // The in-window menu bar and its Alt, F10 and letter keys are PC behaviour: a Mac app's menu bar is native.
    fixture.os(Os::Linux);
    fixture.direction(direction).settle();
    Rig { fixture }
}

impl Rig {
    fn open(&self, label: &str) -> bool {
        self.fixture.by_label(label).is_some()
    }
}

#[trial]
fn help_search_ignores_accents_and_matches_the_english_label_and_the_path() {
    let mut rig = rig(Direction::Ltr);
    let help = rig.fixture.by_label("Help").expect("the Help title");
    rig.fixture.click(&help);
    rig.fixture.settle();
    let search = rig.fixture.by_label("Search menus").expect("the search field");
    rig.fixture.click(&search);
    for (typed, found, absent) in [("cafe", "Paste", "Copy"), ("duplic", "Copy", "Paste"), ("clipboard", "Cut", "Copy")] {
        rig.fixture.key(Key::A, Modifiers::COMMAND);
        rig.fixture.type_text(typed);
        rig.fixture.settle();
        assert!(rig.open(found), "{typed:?} finds the entry whose command row reads {found:?}");
        assert!(!rig.open(absent), "{typed:?} leaves {absent:?} out");
    }
}

/// A menu a title click opened takes the letter a row marks: the row runs and the run closes the menu.
#[trial]
fn a_letter_runs_the_row_it_marks_in_a_menu_a_title_click_opened() {
    use std::cell::Cell;
    use std::rc::Rc;
    let saved = Rc::new(Cell::new(0));
    let seen = saved.clone();
    let mut fixture = Fixture::new(move |ui| {
        let titles = vec![MenuTitle::new("&File", |ui| {
            MenuItem::new("&New").show(ui);
            if MenuItem::new("&Save").show(ui).ran {
                seen.set(seen.get() + 1);
            }
        })];
        MenuBar::new("Main menu", titles).show(ui);
    });
    fixture.os(Os::Linux);
    fixture.settle();
    let file = fixture.by_label("File").expect("the File title");
    fixture.click(&file);
    fixture.settle();
    assert!(fixture.by_label("Save").is_some(), "File is open");
    fixture.type_key(Key::S, "s");
    fixture.settle();
    assert_eq!(saved.get(), 1, "the letter runs the row it marks");
    assert!(fixture.by_label("Save").is_none(), "and the run closes the menu");
}

/// The pass a pointer move over a menu's rows runs paints the row under the pointer as the active row and no other,
/// moving down or up, at 1x and at a fractional scale. The two frames egui runs after any frame with input change
/// nothing, and then the menu asks for none.
#[trial]
fn a_pointer_move_over_menu_rows_lights_the_row_under_it_in_that_frame() {
    use craft_behavior::MotionMode;
    use craft_ui::runtime::env::Scale;
    use craft_ui::testing::pointer;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    const ROWS: usize = 8;
    let names: Vec<String> = (1..=ROWS).map(|at| format!("Item {at:02}")).collect();
    for scale in [1.0, 1.5] {
        let lit: Rc<RefCell<HashMap<String, bool>>> = Rc::default();
        let seen = lit.clone();
        let rows = names.clone();
        let mut fixture = Fixture::new(move |ui| {
            let seen = seen.clone();
            let rows = rows.clone();
            let titles = vec![MenuTitle::new("&File", move |ui| {
                for name in &rows {
                    let active = MenuItem::new(name.clone()).show(ui).active;
                    // The last run of the frame is the one that paints.
                    seen.borrow_mut().insert(name.clone(), active);
                }
            })];
            MenuBar::new("Main menu", titles).show(ui);
        });
        fixture.os(Os::Linux);
        fixture.room(egui::vec2(480.0, 320.0)).scale(Scale::new(scale)).motion(MotionMode::Off).settle();
        let file = fixture.by_label("File").expect("the File title");
        fixture.click(&file);
        fixture.settle();
        let shown: Vec<(String, egui::Pos2)> =
            names.iter().map(|name| (name.clone(), fixture.by_label(name).expect("the File menu shows every row").center())).collect();
        let downward = shown.iter();
        let upward = shown.iter().rev().skip(1);
        for (name, center) in downward.chain(upward) {
            lit.borrow_mut().clear();
            fixture.frame(vec![pointer::moved(*center)]);
            let active: Vec<String> = lit.borrow().iter().filter(|(_, on)| **on).map(|(row, _)| row.clone()).collect();
            assert_eq!(active, vec![name.clone()], "at {scale}x the move frame onto {name} lights it and no other row");
            // egui repaints twice for a frame with input; the menu asks for nothing beyond that.
            for _ in 0..2 {
                lit.borrow_mut().clear();
                fixture.frame(Vec::new());
                assert_eq!(lit.borrow().get(name), Some(&true), "at {scale}x {name} stays lit with the pointer at rest");
            }
            assert!(!fixture.repaint_requested(), "at {scale}x a pointer at rest on {name} asks for no frame");
        }
    }
}

/// A help search over built entries builds them only in a pass that shows the Help menu: a bar whose menus stay closed
/// never runs the builder, an open File menu doesn't either, and the open Help menu searches what it built.
#[trial]
fn built_help_entries_are_collected_only_while_the_help_menu_shows() {
    use std::cell::Cell;
    use std::rc::Rc;
    let builds = Rc::new(Cell::new(0u32));
    let counted = builds.clone();
    let mut fixture = Fixture::new(move |ui| {
        let titles = vec![
            MenuTitle::new("&File", |ui| {
                MenuItem::new("&New").show(ui);
            }),
            MenuTitle::new("&Help", |ui| {
                MenuItem::new("&About").show(ui);
            }),
        ];
        let counted = counted.clone();
        let build = move || {
            counted.set(counted.get() + 1);
            entries()
        };
        MenuBar::new("Main menu", titles).help(MenuHelp::new(1, &build)).show(ui);
    });
    fixture.os(Os::Linux);
    fixture.settle();
    fixture.settle();
    assert_eq!(builds.get(), 0, "closed menus build nothing");

    let file = fixture.by_label("File").expect("the File title");
    fixture.click(&file);
    fixture.settle();
    assert!(fixture.by_label("New").is_some(), "File is open");
    assert_eq!(builds.get(), 0, "another open menu builds nothing");
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.settle();

    let help = fixture.by_label("Help").expect("the Help title");
    fixture.click(&help);
    fixture.settle();
    assert!(builds.get() > 0, "the open Help menu builds its entries");
    let search = fixture.by_label("Search menus").expect("the search field");
    fixture.click(&search);
    fixture.type_text("duplic");
    fixture.settle();
    assert!(fixture.by_label("Copy").is_some(), "the search finds a built entry by its English label");
}

/// The level keeps its letters between passes and assigns them again when the rows' labels change: a row that read
/// Save when the menu last opened and reads Open now takes O, so typing O runs it.
#[trial]
fn a_row_whose_label_changed_since_the_menu_last_opened_takes_its_new_letter() {
    use std::cell::Cell;
    use std::rc::Rc;
    let renamed = Rc::new(Cell::new(false));
    let runs = Rc::new(Cell::new(0));
    let (label, ran) = (renamed.clone(), runs.clone());
    let mut fixture = Fixture::new(move |ui| {
        let (label, ran) = (label.clone(), ran.clone());
        let titles = vec![MenuTitle::new("&File", move |ui| {
            MenuItem::new("Alpha").show(ui);
            MenuItem::new("Beta").show(ui);
            if MenuItem::new(if label.get() { "Open" } else { "Save" }).show(ui).ran {
                ran.set(ran.get() + 1);
            }
        })];
        MenuBar::new("Main menu", titles).show(ui);
    });
    fixture.os(Os::Linux);
    fixture.settle();
    let file = fixture.by_label("File").expect("the File title");
    fixture.click(&file);
    fixture.settle();
    assert!(fixture.by_label("Save").is_some(), "File is open");
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.settle();
    assert!(fixture.by_label("Save").is_none(), "File is closed");

    renamed.set(true);
    let file = fixture.by_label("File").expect("the File title");
    fixture.click(&file);
    fixture.settle();
    assert!(fixture.by_label("Open").is_some(), "File is open again, the row reads Open");
    fixture.type_key(Key::O, "o");
    fixture.settle();
    assert_eq!(runs.get(), 1, "O runs the row that reads Open");
}

/// A view of a menu bar with a File menu.
#[derive(craft_ui::contract::MemoProps)]
struct MainMenu {}

fn main_menu(_: &mut MainMenu, ui: &mut egui::Ui) {
    let titles = vec![MenuTitle::new("&File", |ui| {
        MenuItem::new("&New").show(ui);
    })];
    MenuBar::new("Main menu", titles).show(ui);
}

/// A menu bar in a memoized view still takes its keys while the view replays its last run: F10 takes the focus to the
/// bar, and Alt with a title's marked letter opens that menu.
#[trial]
fn a_settled_menu_bar_takes_its_mnemonics_and_f10() {
    use craft_ui::contract::MemoView;
    use craft_ui::testing::{MemoRun, last_frame_memo_runs};
    let mut fixture = Fixture::new(|ui| {
        MemoView::new("title", MainMenu {}, main_menu).show(ui);
    });
    fixture.os(Os::Linux);
    fixture.settle().idle(2);
    assert!(fixture.focused_stop().is_none(), "nothing holds the focus at the start");
    let runs = last_frame_memo_runs(fixture.ctx());
    assert!(runs.iter().all(|(_, _, run)| *run == MemoRun::Replayed) && !runs.is_empty(), "the bar's view replays: {runs:?}");
    fixture.key(Key::F10, Modifiers::NONE);
    fixture.settle();
    assert!(fixture.focused_stop().is_some(), "F10 takes the focus to the bar");
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.settle().idle(2);
    fixture.key(Key::F, Modifiers::ALT);
    fixture.settle();
    assert!(fixture.by_label("New").is_some(), "Alt+F opens File");
}

/// An Alt+letter the keymap binds to an enabled command runs that command and opens no menu, as a shortcut comes before
/// a mnemonic; without the binding the same chord opens the menu.
#[trial]
fn a_bound_alt_letter_runs_its_command_instead_of_the_mnemonic() {
    use craft_ui::runtime::commands::{AppCatalog, Binding, CommandDef, CommandInput, Scope};
    use craft_ui::runtime::frame::take_output;
    const RULERS: &str = "barTrial.rulers";
    let mut rig = rig(Direction::Ltr);
    let binding = Binding::parse("Alt+F").expect("the trial's chord parses");
    let catalog = AppCatalog::new(vec![CommandDef::new(RULERS, "Rulers", Scope::app()).with_default(binding)], 1).expect("the trial's catalog");
    rig.fixture.commands(CommandInput::new().catalog(catalog)).settle();
    take_output(rig.fixture.ctx());
    rig.fixture.key(Key::F, Modifiers::ALT);
    rig.fixture.settle();
    let fired: Vec<String> = take_output(rig.fixture.ctx()).fired.into_iter().map(|fired| fired.command.id).collect();
    assert_eq!(fired, [RULERS], "the bound Alt+F runs its command once");
    assert!(!rig.open("New"), "and does not also open File");

    rig.fixture.commands(CommandInput::new()).settle();
    rig.fixture.key(Key::F, Modifiers::ALT);
    rig.fixture.settle();
    assert!(rig.open("New"), "unbound, Alt+F opens File");
}

/// `MenuBarOutput::opened` is true in the one pass where a menu opened while none was open: a held menu, a switch from
/// one title to another and the passes a frame reruns report false.
#[trial]
fn a_bar_menu_opening_is_reported_once() {
    use std::cell::RefCell;
    use std::rc::Rc;
    let passes: Rc<RefCell<Vec<bool>>> = Rc::default();
    let seen = passes.clone();
    let mut fixture = Fixture::new(move |ui| {
        let titles = vec![
            MenuTitle::new("&File", |ui| {
                MenuItem::new("&New").show(ui);
            }),
            MenuTitle::new("&Edit", |ui| {
                MenuItem::new("&Undo").show(ui);
            }),
        ];
        let out = MenuBar::new("Main menu", titles).show(ui);
        seen.borrow_mut().push(out.opened);
    });
    fixture.os(Os::Linux);
    let reports = || {
        let count = passes.borrow().iter().filter(|opened| **opened).count();
        passes.borrow_mut().clear();
        count
    };
    fixture.settle();
    assert_eq!(reports(), 0, "a closed bar reports no opening");

    let file = fixture.by_label("File").expect("the File title");
    fixture.click(&file);
    fixture.settle();
    assert!(fixture.by_label("New").is_some(), "File is open");
    assert_eq!(reports(), 1, "the click that opened File reports once across every pass it ran");

    fixture.settle().idle(2);
    assert_eq!(reports(), 0, "a menu held open reports nothing");

    fixture.key(Key::E, Modifiers::ALT);
    fixture.settle();
    assert!(fixture.by_label("Undo").is_some() && fixture.by_label("New").is_none(), "Alt+E switches to Edit");
    assert_eq!(reports(), 0, "a switch from one open title to another reports nothing");

    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.settle();
    assert!(fixture.by_label("Undo").is_none(), "Edit is closed");
    assert_eq!(reports(), 0, "closing reports nothing");

    let file = fixture.by_label("File").expect("the File title");
    fixture.click(&file);
    fixture.settle();
    assert_eq!(reports(), 1, "opening again reports once more");
}

/// A view beside the menu bar that reads nothing the menu changes.
#[derive(craft_ui::contract::MemoProps)]
struct Status {}

fn status(_: &mut Status, ui: &mut egui::Ui) {
    craft_ui::display::Text::new("Ready").show(ui);
}

#[trial]
fn switching_sibling_submenus_paints_only_the_new_submenu() {
    use craft_behavior::MotionMode;
    use craft_ui::contract::MemoView;
    use craft_ui::menu::Menu;
    use craft_ui::runtime::env::Scale;
    use craft_ui::testing::{MemoRun, last_frame_memo_runs, pointer};
    use egui::Shape;

    for motion in [MotionMode::Full, MotionMode::Off] {
        let mut fixture = Fixture::new(move |ui| {
            MemoView::new("status", Status {}, status).show(ui);
            let titles = vec![MenuTitle::new("Filter", |ui| {
                Menu::submenu_row(ui, "Blur", |ui, _| {
                    MenuItem::new("Gaussian Blur").show(ui);
                    MenuItem::new("Motion Blur").show(ui);
                });
                Menu::submenu_row(ui, "Sharpen", |ui, _| {
                    MenuItem::new("Unsharp Mask").show(ui);
                });
            })];
            MenuBar::new("Main menu", titles).show(ui);
        });
        fixture.os(Os::Windows).room(egui::vec2(480.0, 320.0)).scale(Scale::new(1.25)).motion(motion).settle();
        let filter = fixture.by_label("Filter").expect("the Filter title");
        fixture.click(&filter).settle();
        let blur = fixture.by_label("Blur").expect("the Blur row");
        let sharpen = fixture.by_label("Sharpen").expect("the Sharpen row");
        fixture.frame(vec![pointer::moved(blur.center())]).advance(0.25).idle(1);
        assert!(fixture.texts().iter().any(|run| run.text == "Gaussian Blur"), "the first submenu starts open");

        for (next, previous, shown, hidden) in [(sharpen, blur, "Unsharp Mask", "Gaussian Blur"), (blur, sharpen, "Gaussian Blur", "Unsharp Mask")] {
            let shapes = fixture.frame_shapes_with(vec![pointer::moved(next.center())]);
            let ran =
                last_frame_memo_runs(fixture.ctx()).into_iter().filter(|(name, _, run)| name.contains("Status") && *run == MemoRun::Ran).count();
            assert_eq!(ran, 0, "switching submenus leaves an unrelated view replaying");
            let painted = |label: &str| fixture.texts().iter().any(|run| run.text == label);
            assert!(painted(shown), "the switch frame paints {shown}");
            assert!(!painted(hidden), "the switch frame no longer paints {hidden}");
            let highlighted = |target| shapes.iter().any(|shape| matches!(shape, Shape::Rect(row) if row.rect == target && row.fill.a() > 0));
            assert!(highlighted(next.rect) && !highlighted(previous.rect), "only the new parent row is highlighted");
            let child_surfaces = shapes.iter().filter(|shape| {
                matches!(shape, Shape::Rect(surface)
                    if surface.rect.left() >= next.rect.right() && surface.corner_radius != egui::CornerRadius::ZERO && surface.fill.a() == 255
                )
            });
            assert_eq!(child_surfaces.count(), 1, "only the new submenu surface is painted");
            fixture.advance(0.25).idle(1);
        }

        // Moving diagonally across Sharpen toward the open Blur menu keeps Blur open.
        let toward_blur = egui::pos2(sharpen.rect.right() - 2.0, sharpen.center().y);
        fixture.frame(vec![pointer::moved(toward_blur)]);
        assert!(!fixture.texts().iter().any(|run| run.text == "Unsharp Mask"), "aiming into Blur does not switch menus");
        let blur_item = fixture.by_label("Gaussian Blur").expect("the Blur submenu is still open");
        fixture.frame(vec![pointer::moved(blur_item.center())]);

        fixture.key(Key::Escape, Modifiers::NONE);
        assert_eq!(
            fixture.texts().iter().any(|run| run.text == "Gaussian Blur"),
            motion == MotionMode::Full,
            "ordinary dismissal follows the menu's inherited motion mode",
        );
        assert!(!fixture.texts().iter().any(|run| run.text == "Unsharp Mask"), "the replaced submenu stays closed");
        fixture.advance(0.25).idle(1);
        assert!(!fixture.texts().iter().any(|run| run.text == "Gaussian Blur"), "Escape finishes closing the submenu");
        assert!(fixture.by_label("Blur").is_some(), "Escape leaves the parent menu open");
    }
}

/// A menu bar in a memoized view underlines its mnemonics while Alt is held: the modifiers it paints by are a tracked
/// input, so the frame Alt goes down runs the view, and so does the frame it comes up, though its props stay the same.
/// In between the view replays again.
#[trial]
fn holding_alt_runs_the_memoized_view_of_a_menu_bar() {
    use craft_ui::contract::MemoView;
    use craft_ui::testing::{MemoRun, last_frame_memo_runs};
    let mut fixture = Fixture::new(|ui| {
        MemoView::new("title", MainMenu {}, main_menu).show(ui);
    });
    fixture.os(Os::Linux);
    fixture.settle();
    let frame = |fixture: &mut Fixture| {
        fixture.frame(Vec::new());
        last_frame_memo_runs(fixture.ctx()).iter().filter(|(_, _, run)| *run == MemoRun::Ran).count()
    };
    assert_eq!(frame(&mut fixture), 0, "the view replays");
    fixture.hold(Modifiers::ALT);
    assert!(frame(&mut fixture) > 0, "Alt going down runs the view, which underlines the mnemonics");
    assert_eq!(frame(&mut fixture), 0, "with Alt held the view replays again");
    fixture.hold(Modifiers::NONE);
    assert!(frame(&mut fixture) > 0, "Alt coming up runs the view, which drops the underlines");
}

/// What a window's own Space does: the canvas's momentary Hand, say.
#[derive(Clone, Copy, Debug, PartialEq, Eq, craft_ui::contract::CommandSet)]
#[command(prefix = "trialHand", scope = "trial.hand")]
#[non_exhaustive]
pub enum HandCommand {
    #[command(label = "Hand", default = "Space")]
    Hand,
}

/// Space on the menu bar that F10 or a lone Alt focused opens no menu: the press goes on to the window's own Space
/// command (a canvas's Hand), as in the reference application, where Space is always the Hand, and the shortcut gives
/// the keyboard back to the stop the bar took it from. Enter and Down still open the focused title's menu.
#[trial]
fn space_on_the_focused_bar_opens_no_menu_and_reaches_the_window() {
    use craft_ui::buttons::Button;
    use craft_ui::contract::ShowContainer as _;
    use craft_ui::layout_components::Stack;
    use craft_ui::runtime::commands::Commands as _;
    use craft_ui::runtime::focus::{FocusTarget, request};
    use std::cell::Cell;
    use std::rc::Rc;
    let (hands, clicks, button) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)), Rc::new(Cell::new(None)));
    let (seen_hands, seen_clicks, seen_button) = (Rc::clone(&hands), Rc::clone(&clicks), Rc::clone(&button));
    let mut fixture = Fixture::new(move |ui| {
        let titles = vec![MenuTitle::new("&File", |ui| {
            MenuItem::new("&New").show(ui);
        })];
        MenuBar::new("Main menu", titles).show(ui);
        Stack::new().command_scope(<HandCommand as craft_ui::contract::CommandSet>::SCOPE).show(ui, |ui, cx| {
            while cx.take_command::<HandCommand>().is_some() {
                seen_hands.set(seen_hands.get() + 1);
            }
            let shown = Button::new("Layer").show(ui);
            if shown.clicked() {
                seen_clicks.set(seen_clicks.get() + 1);
            }
            seen_button.set(Some(shown.response.id));
        });
    });
    fixture.os(Os::Linux);
    fixture.settle();
    let layer = button.get().expect("the button ran");
    for (enter, alt) in [(None, false), (None, true), (Some(Key::Enter), false), (Some(Key::ArrowDown), false)] {
        request(fixture.ctx(), FocusTarget::Id(layer));
        fixture.settle();
        let before = (hands.get(), clicks.get());
        if alt {
            fixture.hold(Modifiers::ALT).settle();
            fixture.hold(Modifiers::NONE).settle();
        } else {
            fixture.key(Key::F10, Modifiers::NONE);
            fixture.settle();
        }
        assert!(fixture.focused_stop().is_some_and(|(id, _)| id != layer), "the bar takes the keyboard (alt {alt})");
        if let Some(key) = enter {
            fixture.key(key, Modifiers::NONE);
            fixture.settle();
            assert!(fixture.by_label("New").is_some(), "{key:?} opens the focused title's menu");
            assert_eq!((hands.get(), clicks.get()), before, "{key:?} reaches neither the window nor the button");
            fixture.key(Key::Escape, Modifiers::NONE);
            fixture.key(Key::Escape, Modifiers::NONE);
            fixture.settle();
            continue;
        }
        fixture.key(Key::Space, Modifiers::NONE);
        fixture.settle();
        assert!(fixture.by_label("New").is_none(), "Space opens no menu (alt {alt})");
        assert_eq!(fixture.focused_stop().map(|(id, _)| id), Some(layer), "Space takes the keyboard off the bar, back to the button (alt {alt})");
        assert_eq!(hands.get(), before.0 + 1, "the press reaches the window's Space command (alt {alt})");
        assert_eq!(clicks.get(), before.1, "and activates nothing else (alt {alt})");
    }
}

/// With nothing focused the window's document canvas is the key target: a bar that F10 or a lone Alt focused then
/// gives the keyboard back to the canvas, so Space, the canvas's Hand, matches there and leaves the bar in one press.
#[trial]
fn space_on_a_bar_focused_from_nothing_goes_to_the_document_canvas() {
    use craft_ui::contract::ShowContainer as _;
    use craft_ui::layout_components::Stack;
    use craft_ui::runtime::commands::Commands as _;
    use craft_ui::runtime::focus::{KeyUse, host_canvas};
    use std::cell::Cell;
    use std::rc::Rc;
    let canvas = egui::Id::unique("document canvas");
    let hands = Rc::new(Cell::new(0));
    let seen_hands = Rc::clone(&hands);
    let mut fixture = Fixture::new(move |ui| {
        let titles = vec![MenuTitle::new("&File", |ui| {
            MenuItem::new("&New").show(ui);
        })];
        MenuBar::new("Main menu", titles).show(ui);
        Stack::new().command_scope(<HandCommand as craft_ui::contract::CommandSet>::SCOPE).show(ui, |ui, cx| {
            while cx.take_command::<HandCommand>().is_some() {
                seen_hands.set(seen_hands.get() + 1);
            }
            let rect = ui.available_rect_before_wrap();
            ui.interact(rect, canvas, egui::Sense::click());
            host_canvas(ui.ctx(), canvas, KeyUse::CANVAS);
        });
    });
    fixture.os(Os::Linux);
    fixture.settle();
    for alt in [false, true] {
        assert!(fixture.focused_stop().is_none_or(|(id, _)| id == canvas), "nothing but the canvas holds the keyboard (alt {alt})");
        if alt {
            fixture.hold(Modifiers::ALT).settle();
            fixture.hold(Modifiers::NONE).settle();
        } else {
            fixture.key(Key::F10, Modifiers::NONE);
            fixture.settle();
        }
        assert!(fixture.focused_stop().is_some_and(|(id, _)| id != canvas), "the bar takes the keyboard (alt {alt})");
        let before = hands.get();
        fixture.key(Key::Space, Modifiers::NONE);
        fixture.settle();
        assert!(fixture.by_label("New").is_none(), "Space opens no menu (alt {alt})");
        assert_eq!(hands.get(), before + 1, "Space reaches the canvas's command (alt {alt})");
        assert_eq!(fixture.focused_stop().map(|(id, _)| id), Some(canvas), "and the keyboard goes to the canvas (alt {alt})");
    }
}

/// Tc-9 (3): the selection of a focused field holds while a menu the window's menu bar opened has the keyboard, which
/// goes back to the field: the bar's Edit menu shows Copy enabled for it. The field kept its selection only for its own
/// edit menu, so Copy was disabled there.
#[trial]
fn a_fields_selection_holds_while_the_menu_bar_has_the_keyboard() {
    use craft_ui::contract::{Bind, Props as _};
    use craft_ui::text::TextField;
    use std::cell::RefCell;
    let text = RefCell::new("hello".to_owned());
    let mut fixture = Fixture::new(move |ui| {
        let titles = vec![MenuTitle::new("&Edit", |ui| {
            MenuItem::command(EditCommands::Copy).show(ui);
        })];
        MenuBar::new("Main menu", titles).show(ui);
        TextField::new("Name", Bind::bound(&mut *text.borrow_mut())).id_salt("name").show(ui);
    });
    fixture.os(Os::Linux);
    fixture.settle();
    let field = fixture.by_label("Name").expect("the field");
    fixture.click(&field).settle();
    fixture.key(Key::A, Modifiers::CTRL).settle();
    let edit = fixture.by_label("Edit").expect("the Edit title");
    fixture.click(&edit).settle();
    let copy = fixture.by_label("Copy").expect("the Edit menu is open");
    assert!(!fixture.tree().is_disabled_key(copy.node), "Copy applies to the field's selection");
}
