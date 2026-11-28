//! The ribbon: the fit rule, keyed group levels, a collapsed group's popover, the tab-change slide, the run model, the
//! line group titles share and the ribbon's own context menu.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use craft_behavior::{IconId, MotionMode};
use craft_keymap::CommandRef;
use craft_menu::{GroupLevel, GroupWidths, Ribbon, RibbonGroup, RibbonMenuRequest, RibbonRow, RibbonTab, plan};
use craft_ui::buttons::{Arrangement, Button, SplitButton};
use craft_ui::contract::{Bind, BodyCx, Entry, Props as _, Role, Show as _, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::menu::MenuItem;
use craft_ui::runtime::env::{Direction, Env};
use craft_ui::runtime::inspect::{Element, Recorded};
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Fixture, is_measuring, last_pass_measuring_runs, trial};
use egui::{Event, Id, Key, Modifiers, PointerButton, Pos2, Ui, vec2};

/// How collapsed a level is: 0 full, 1 compact, 2 collapsed.
fn rank(level: GroupLevel) -> u8 {
    match level {
        GroupLevel::Full => 0,
        GroupLevel::Compact => 1,
        _ => 2,
    }
}

/// The width of `group` at `level`.
fn width_at(group: &GroupWidths, level: GroupLevel) -> f32 {
    match rank(level) {
        0 => group.full,
        1 => group.compact,
        _ => group.collapsed,
    }
}

/// The width of the row with each group at its level.
fn total(widths: &[GroupWidths], levels: &[GroupLevel]) -> f32 {
    widths.iter().zip(levels).map(|(group, level)| width_at(group, *level)).sum()
}

/// The width of `group` one level wider than `level`.
fn wider(group: &GroupWidths, level: GroupLevel) -> f32 {
    if rank(level) == 2 { group.compact } else { group.full }
}

/// The fit shrinks the groups from the end, first to compact and then to collapsed, stopping as soon as the row
/// fits, and then gives a level back from the start while the row still fits.
#[trial]
fn the_plan_compacts_then_collapses_from_the_end_and_gives_back_from_the_start() {
    let even = [GroupWidths::new(100.0, 80.0, 40.0); 4];
    let mut last_collapsed = 0;
    for available in (0..=40).rev().map(|step| step as f32 * 10.0) {
        let levels = plan(&even, available);
        let collapsed: u8 = levels.iter().map(|level| rank(*level)).sum();
        assert!(collapsed >= last_collapsed, "a narrower row never shows more of a group ({available})");
        last_collapsed = collapsed;
        if available >= 160.0 {
            assert!(total(&even, &levels) <= available, "the row fits whenever it can ({available})");
        } else {
            assert!(levels.iter().all(|level| rank(*level) == 2), "a row nothing fits in shows every group collapsed ({available})");
        }
        assert!(levels.windows(2).all(|pair| rank(pair[0]) <= rank(pair[1])), "the groups at the end give up first ({available}): {levels:?}");
        for (group, level) in even.iter().zip(&levels) {
            if rank(*level) > 0 {
                let given_back = total(&even, &levels) - width_at(group, *level) + wider(group, *level);
                assert!(given_back > available, "nothing more can be given back ({available})");
            }
        }
    }
    let roomy = plan(&even, 400.0);
    assert!(roomy.iter().all(|level| rank(*level) == 0), "a row that fits shows every group in full");

    // A wide group that shrinks frees room: the groups the shrinking passed over give back what they gave up.
    let uneven = [
        GroupWidths::new(300.0, 250.0, 60.0),
        GroupWidths::new(500.0, 500.0, 60.0),
        GroupWidths::new(60.0, 60.0, 60.0),
        GroupWidths::new(60.0, 60.0, 60.0),
    ];
    let levels = plan(&uneven, 800.0);
    assert!(total(&uneven, &levels) <= 800.0, "the row fits");
    assert_eq!(levels.iter().map(|level| rank(*level)).collect::<Vec<_>>(), vec![0, 2, 0, 0], "only the wide group stays collapsed");
}

/// A group named `title` whose content is `labels` as stacked buttons.
fn group(row: &mut RibbonRow<'_>, key: &'static str, title: &'static str, labels: &[&'static str]) {
    row.group(RibbonGroup::new(key, title, IconId::new("list").into()), |ui| {
        for label in labels {
            Button::new(*label).arrangement(Arrangement::Stacked).show(ui);
        }
    });
}

const WIDE: [&str; 12] = ["Alpha1", "Alpha2", "Alpha3", "Alpha4", "Alpha5", "Alpha6", "Alpha7", "Alpha8", "Alpha9", "Alpha10", "Alpha11", "Alpha12"];

/// Two groups with one title, shown in either order, each keep their own level: the wide one collapses, the narrow
/// one stays whole, and re-ordering does not swap them.
#[trial]
fn groups_with_the_same_title_keep_their_own_level() {
    let swapped = Rc::new(Cell::new(false));
    let flag = swapped.clone();
    let mut fixture = Fixture::new(move |ui| {
        let tabs = [RibbonTab::new(0_usize, "Home")];
        let _ = Ribbon::new("Ribbon", &tabs, Bind::given(0_usize)).show(ui, |row, _| {
            if flag.get() {
                group(row, "narrow", "Group", &["Only"]);
                group(row, "wide", "Group", &WIDE);
            } else {
                group(row, "wide", "Group", &WIDE);
                group(row, "narrow", "Group", &["Only"]);
            }
        });
    });
    fixture.room(vec2(420.0, 300.0));
    fixture.settle();
    assert!(fixture.by_label("Alpha1").is_none(), "the wide group is collapsed");
    assert!(fixture.by_label("Only").is_some(), "the narrow group of the same title stays whole");
    swapped.set(true);
    fixture.settle();
    assert!(fixture.by_label("Alpha1").is_none(), "the wide group stays collapsed after the groups swap places");
    assert!(fixture.by_label("Only").is_some(), "the narrow group stays whole after the groups swap places");
}

/// A collapsed group opens a popover; a menu opened from a split button inside it and a pick from that menu leave
/// the popover open, and a press outside closes it.
#[trial]
fn a_collapsed_groups_popover_stays_open_while_a_child_popup_is_used() {
    let picked: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new(Vec::new()));
    let seen = picked.clone();
    let mut fixture = Fixture::new(move |ui| {
        let tabs = [RibbonTab::new(0_usize, "Home")];
        let entries = [Entry::new(1_usize, "Keep text only"), Entry::new(2, "Paste values")];
        let _ = Ribbon::new("Ribbon", &tabs, Bind::given(0_usize)).show(ui, |row, _| {
            // Every control is stacked, so the clipboard group is as wide at the compact level as at the full one.
            row.group(RibbonGroup::new("clip", "Clipboard", IconId::new("list").into()), |ui| {
                if let Some(entry) = SplitButton::new("Paste", &entries).arrangement(Arrangement::Stacked).show(ui).chosen {
                    seen.borrow_mut().push(entry);
                }
                for label in &WIDE[..4] {
                    Button::new(*label).arrangement(Arrangement::Stacked).show(ui);
                }
            });
            group(row, "wide", "Wide", &WIDE);
        });
    });
    // The room is the clipboard group's own width and a little more, measured where both groups show whole: its content
    // alone fits, so its popover shows whole, but not beside the wide group's least, so the fit collapses it. Measured,
    // not a fixed room, so the trial does not hang on the buttons' padding.
    fixture.room(vec2(2000.0, 300.0));
    fixture.settle();
    let whole = fixture.by_label("Clipboard").expect("the clipboard group shows whole in a wide room").rect.width();
    fixture.room(vec2(whole + 8.0, 300.0));
    fixture.settle();
    assert!(fixture.by_label("Paste").is_none(), "the group is collapsed");
    let button = fixture.by_label("Clipboard").expect("the collapsed group's button");
    fixture.click(&button).idle(3);
    assert!(fixture.by_label("Paste").is_some(), "the button opens the group's popover");
    fixture.key(Key::ArrowDown, Modifiers::ALT).idle(3);
    fixture.key(Key::Enter, Modifiers::NONE).idle(3);
    assert_eq!(picked.borrow().clone(), vec![1], "the split button's menu picked its first row");
    assert!(fixture.by_label("Paste").is_some(), "the group's popover is still open after a pick from a menu opened inside it");
    fixture.click_at(Pos2::new(whole + 3.0, 295.0)).idle(3);
    assert!(fixture.by_label("Paste").is_none(), "a press outside closes the popover");
}

/// The ribbon's inspected record of the latest real run.
type Controls = Rc<RefCell<Option<Arc<Recorded>>>>;

/// The x at which the latest frame painted the group titled `title`: its rect in the ribbon's record, which maps each
/// element through the layer it shows on (the slide's), unlike its access bounds, which stay in that layer's points.
fn group_x(controls: &Controls, title: &str) -> f32 {
    let recorded = controls.borrow();
    let named = |element: &&Element| element.part.is_none() && element.facts.as_ref().and_then(|facts| facts.name.as_deref()) == Some(title);
    let elements: &[Element] = recorded.as_deref().map_or(&[], |recorded| recorded.elements.as_slice());
    elements.iter().filter(named).min_by_key(|element| element.depth).map_or(f32::NAN, |element| element.rect.left())
}

/// A ribbon of three tabs with one group each, showing the tab `shown` holds; `controls` takes its record.
fn slide_rig(shown: &Rc<Cell<usize>>, controls: &Controls, direction: Direction, motion: MotionMode) -> Fixture {
    let state = shown.clone();
    let record = controls.clone();
    let mut fixture = Fixture::new(move |ui| {
        let tabs = [RibbonTab::new(0_usize, "One"), RibbonTab::new(1, "Two"), RibbonTab::new(2, "Three")];
        let out = Ribbon::new("Ribbon", &tabs, Bind::given(state.get())).show(ui, |row, tab| match tab {
            0 => group(row, "a", "Alpha", &["First"]),
            1 => group(row, "b", "Beta", &["Second"]),
            _ => group(row, "c", "Gamma", &["Third"]),
        });
        if let Some(next) = out.shown {
            state.set(next);
        }
        if let Some(controls) = out.controls {
            *record.borrow_mut() = Some(controls);
        }
    });
    fixture.direction(direction).motion(motion).room(vec2(600.0, 300.0));
    fixture.settle();
    fixture
}

/// The x of the group of tab `to` on the first frame it shows after a click on its tab while tab `from` showed, and
/// at rest. The click's release reports the tab through `RibbonOutput::shown`; the rig applies it, so the frame after
/// the release (the click's idle frame) is the first that shows the new row.
fn slide(direction: Direction, motion: MotionMode, from: usize, to: usize) -> (f32, f32) {
    let shown = Rc::new(Cell::new(from));
    let controls: Controls = Rc::new(RefCell::new(None));
    let mut fixture = slide_rig(&shown, &controls, direction, motion);
    let tab = fixture.by_label(["One", "Two", "Three"][to]).expect("the tab to change to");
    fixture.click(&tab);
    assert_eq!(shown.get(), to, "the click on the tab showed it");
    let title = ["Alpha", "Beta", "Gamma"][to];
    let first = group_x(&controls, title);
    // The slide is a tween on the frame clock: two seconds of clock bring it to rest in the frames a settle runs.
    fixture.advance(2.0).settle();
    let rest = group_x(&controls, title);
    (first, rest)
}

/// A tab change slides the new group row in from the side of the new tab (mirrored in right-to-left) and reaches
/// rest; with reduced motion the row is at rest on the first frame.
#[trial]
fn a_tab_change_slides_the_row_in_from_the_new_tabs_side_and_shows_at_rest_under_reduced_motion() {
    let (first, rest) = slide(Direction::Ltr, MotionMode::Full, 0, 2);
    assert!(first > rest + 1.0, "a later tab's row starts toward the end side: {first} vs {rest}");
    let (first, rest) = slide(Direction::Ltr, MotionMode::Full, 2, 0);
    assert!(first < rest - 1.0, "an earlier tab's row starts toward the start side: {first} vs {rest}");
    let (first, rest) = slide(Direction::Rtl, MotionMode::Full, 0, 2);
    assert!(first < rest - 1.0, "under right-to-left the sides mirror: {first} vs {rest}");
    let (first, rest) = slide(Direction::Rtl, MotionMode::Full, 2, 0);
    assert!(first > rest + 1.0, "under right-to-left the sides mirror: {first} vs {rest}");
    let (first, rest) = slide(Direction::Ltr, MotionMode::Reduced, 0, 2);
    assert!((first - rest).abs() < 0.5, "with reduced motion the row shows at rest at once: {first} vs {rest}");
}

/// What each group's content did, by the group's key: how many times it ran in a measuring run since the counts were
/// cleared, and the level its last real run showed it at.
type Runs = Rc<RefCell<HashMap<&'static str, (u32, GroupLevel)>>>;

/// A group like [`group`] whose content counts its runs in `runs`.
fn counted(row: &mut RibbonRow<'_>, runs: &Runs, key: &'static str, labels: [&'static str; 2]) {
    let runs = runs.clone();
    row.group(RibbonGroup::new(key, key, IconId::new("list").into()), move |ui| {
        let level = Env::of(ui).get::<GroupLevel>();
        let measuring = is_measuring(ui.ctx());
        let mut runs = runs.borrow_mut();
        let entry = runs.entry(key).or_insert((0, level));
        if measuring {
            entry.0 += 1;
        } else {
            entry.1 = level;
        }
        for label in labels {
            Button::new(label).arrangement(Arrangement::Stacked).show(ui);
        }
    });
}

/// The measuring runs of `key`'s content since the counts were cleared.
fn measured(runs: &Runs, key: &str) -> u32 {
    runs.borrow().get(key).map_or(0, |(count, _)| *count)
}

/// The level `key`'s group last showed at.
fn level(runs: &Runs, key: &str) -> GroupLevel {
    runs.borrow().get(key).map_or(GroupLevel::Full, |(_, level)| *level)
}

/// Clears the measuring counts, keeping the levels.
fn clear(runs: &Runs) {
    for (count, _) in runs.borrow_mut().values_mut() {
        *count = 0;
    }
}

const KEYS: [&str; 4] = ["one", "two", "three", "four"];

/// Count trial of the run model: a settled ribbon runs no group's content in a measuring run; each shows once, at its
/// kept level.
#[trial]
fn a_settled_ribbon_runs_each_group_body_once() {
    let runs: Runs = Rc::new(RefCell::new(HashMap::new()));
    let seen = runs.clone();
    let mut fixture = Fixture::new(move |ui| {
        let tabs = [RibbonTab::new(0_usize, "Home")];
        let _ = Ribbon::new("Ribbon", &tabs, Bind::given(0_usize)).show(ui, |row, _| {
            counted(row, &seen, "one", ["Alpha", "Beta"]);
            counted(row, &seen, "two", ["Gamma", "Delta"]);
            counted(row, &seen, "three", ["Epsilon", "Zeta"]);
            counted(row, &seen, "four", ["Eta", "Theta"]);
        });
    });
    fixture.room(vec2(900.0, 300.0));
    fixture.settle();
    assert!(KEYS.iter().all(|key| level(&runs, key) == GroupLevel::Full), "every group shows in full in a wide room");
    clear(&runs);
    fixture.idle(1);
    assert!(KEYS.iter().all(|key| measured(&runs, key) == 0), "a pass with unchanged inputs measures no group: {:?}", runs.borrow());
    assert!(last_pass_measuring_runs(fixture.ctx()).is_empty(), "a pass with unchanged inputs runs no measuring run");
}

/// Every open group's title sits on one line at the foot of the row whatever its content's height: a group of one
/// button, a group of three stacked rows and a group of one stacked button.
#[trial]
fn every_open_groups_title_sits_on_one_line() {
    let mut fixture = Fixture::new(|ui| {
        let tabs = [RibbonTab::new(0_usize, "Home")];
        let _ = Ribbon::new("Ribbon", &tabs, Bind::given(0_usize)).show(ui, |row, _| {
            row.group(RibbonGroup::new("short", "Short", IconId::new("list").into()), |ui| {
                Button::new("One").show(ui);
            });
            row.group(RibbonGroup::new("rows", "Rows", IconId::new("list").into()), |ui| {
                Stack::new().show(ui, |ui, _| {
                    for label in ["First", "Second", "Third"] {
                        Button::new(label).show(ui);
                    }
                });
            });
            group(row, "big", "Big", &["Large"]);
        });
    });
    fixture.room(vec2(900.0, 400.0));
    fixture.settle();
    let baseline = |title: &str| fixture.texts().iter().find(|run| run.text == title).map_or(f32::NAN, |run| run.baseline);
    let (short, rows, big) = (baseline("Short"), baseline("Rows"), baseline("Big"));
    assert!((short - rows).abs() < 0.5, "a one-button group's title is on the line of a three-row group's: {short} against {rows}");
    assert!((big - rows).abs() < 0.5, "a stacked button group's title is on the same line: {big} against {rows}");
}

/// A group's title sits directly under its body on one line shared by groups with and without a dialog launcher, the
/// launcher sits on that line at the group's end, and each group, its separator with it, is no taller than its body
/// and title: in a room much taller than the ribbon, no group takes more height than that.
#[trial]
fn a_groups_title_sits_under_its_body_and_the_group_ends_with_it() {
    let mut fixture = Fixture::new(|ui| {
        let tabs = [RibbonTab::new(0_usize, "Home")];
        let _ = Ribbon::new("Ribbon", &tabs, Bind::given(0_usize)).show(ui, |row, _| {
            row.group(RibbonGroup::new("plain", "Plain", IconId::new("list").into()), |ui| {
                Stack::new().show(ui, |ui, _| {
                    for label in ["First", "Second", "Third"] {
                        Button::new(label).show(ui);
                    }
                });
            });
            row.group(RibbonGroup::new("launched", "Launched", IconId::new("list").into()).launcher(CommandRef::new("group.dialog")), |ui| {
                Button::new("Large").arrangement(Arrangement::Stacked).show(ui);
            });
        });
    });
    fixture.room(vec2(900.0, 700.0));
    fixture.settle();
    let run = |title: &str| fixture.texts().iter().find(|run| run.text == title).cloned().expect("the title shows");
    let (plain, launched) = (run("Plain"), run("Launched"));
    assert!(
        (plain.baseline - launched.baseline).abs() < 0.5,
        "a launcher does not move its group's title: {} against {}",
        plain.baseline,
        launched.baseline
    );
    let third = fixture.by_label("Third").expect("the third row").rect;
    assert!(plain.rect.top() >= third.bottom() - 0.5, "the title {:?} is under the body {third:?}", plain.rect);
    assert!(plain.rect.top() - third.bottom() < plain.rect.height(), "the title {:?} sits directly under the body {third:?}", plain.rect);
    let launcher = fixture.by_label("Launched settings").expect("the launcher").rect;
    assert!((launcher.center().y - launched.rect.center().y).abs() <= 1.0, "the launcher {launcher:?} is on the title's line {:?}", launched.rect);
    assert!(launcher.left() >= launched.rect.right() - 0.5, "the launcher {launcher:?} is after the title {:?}", launched.rect);
    let foot = plain.rect.bottom().max(launcher.bottom());
    for group in
        fixture.by_role(Role::GROUP).iter().filter(|group| group.rect.contains(plain.rect.center()) || group.rect.contains(launched.rect.center()))
    {
        assert!(group.rect.bottom() <= foot + 2.0, "the group {:?} ends with its title (at {foot}), not further down the room", group.rect);
    }
}

/// Presses and releases the secondary button at `at`, a frame each.
fn right_click(fixture: &mut Fixture, at: Pos2) {
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };
    fixture.frame(vec![Event::PointerMoved(at), button(true)]);
    fixture.frame(vec![button(false)]);
    fixture.idle(1);
}

/// The row the ribbon's context menu body shows.
const MENU_ROW: &str = "Add to Quick Access Toolbar";

/// A right click on a ribbon control opens the ribbon's own menu, whose body gets the control's bound command; the
/// menu stays open on a pass with no new input, and Escape closes it with focus back on the control. A right click on
/// a tab opens it with no command.
#[trial]
fn a_right_click_on_a_control_opens_the_ribbons_menu_with_its_command() {
    let menus: Rc<RefCell<Vec<Option<CommandRef>>>> = Rc::new(RefCell::new(Vec::new()));
    let paste_id = Rc::new(Cell::new(Id::NULL));
    let (seen, at) = (menus.clone(), paste_id.clone());
    let mut fixture = Fixture::new(move |ui| {
        let tabs = [RibbonTab::new(0_usize, "Home"), RibbonTab::new(1, "Insert")];
        let mut body = |ui: &mut Ui, request: &RibbonMenuRequest, _: &mut BodyCx<'_>| {
            seen.borrow_mut().push(request.command.clone());
            MenuItem::new(MENU_ROW).show(ui);
        };
        let _ = Ribbon::new("Ribbon", &tabs, Bind::given(0_usize)).context_menu(&mut body).show(ui, |row, _| {
            row.group(RibbonGroup::new("clip", "Clipboard", IconId::new("list").into()), |ui| {
                at.set(Button::new("Paste").command(CommandRef::new("edit.paste")).arrangement(Arrangement::Stacked).show(ui).response.id);
            });
        });
    });
    fixture.room(vec2(600.0, 300.0));
    fixture.settle();
    let paste = fixture.by_label("Paste").expect("the control");
    // The control holds focus, so the menu has a stop to give it back to. A press on a button acts and moves no focus
    // to it (press focus kind 3, `classify_press`), so a Focus request puts it there.
    fixture.access(AccessActionKind::Focus, &paste).settle();
    assert_eq!(fixture.focused(), Some(paste_id.get()), "the Focus request focused the control");
    right_click(&mut fixture, paste.rect.center());
    assert!(fixture.by_label(MENU_ROW).is_some(), "the right click opens the ribbon's menu");
    assert!(
        menus.borrow().iter().all(|command| *command == Some(CommandRef::new("edit.paste"))) && !menus.borrow().is_empty(),
        "the body gets the control's command: {:?}",
        menus.borrow()
    );

    menus.borrow_mut().clear();
    fixture.idle(1);
    assert!(fixture.by_label(MENU_ROW).is_some(), "the menu stays open on a pass with no new input");
    assert!(!menus.borrow().is_empty(), "its body runs on that pass");

    fixture.key(Key::Escape, Modifiers::NONE).settle();
    assert!(fixture.by_label(MENU_ROW).is_none(), "Escape closes the menu");
    assert_eq!(fixture.focused(), Some(paste_id.get()), "focus is back on the control");

    menus.borrow_mut().clear();
    let tab = fixture.by_label("Insert").expect("a tab");
    right_click(&mut fixture, tab.rect.center());
    assert!(fixture.by_label(MENU_ROW).is_some(), "a right click on a tab opens the menu");
    assert!(menus.borrow().iter().all(Option::is_none) && !menus.borrow().is_empty(), "a tab's request carries no command: {:?}", menus.borrow());
}

/// What a ribbon of a one-button group, a group of three rows and a group of one stacked button shows in its first
/// frame and once settled: the ribbon's rect, and each group's rect, whose foot its title sits at, from the first frame
/// that shows the groups (the slide's layer shows nothing in its first frame).
fn first_and_settled() -> [(egui::Rect, [egui::Rect; 3]); 2] {
    let rect = Rc::new(Cell::new(egui::Rect::NOTHING));
    let controls: Controls = Rc::new(RefCell::new(None));
    let (out, record) = (rect.clone(), controls.clone());
    let mut fixture = Fixture::new(move |ui| {
        let tabs = [RibbonTab::new(0_usize, "Home")];
        let shown = Stack::new().show(ui, |ui, _| {
            let shown = Ribbon::new("Ribbon", &tabs, Bind::given(0_usize)).show(ui, |row, _| {
                row.group(RibbonGroup::new("short", "Short", IconId::new("list").into()), |ui| {
                    Button::new("One").show(ui);
                });
                row.group(RibbonGroup::new("rows", "Rows", IconId::new("list").into()).launcher(CommandRef::new("group.dialog")), |ui| {
                    Stack::new().show(ui, |ui, _| {
                        for label in ["First", "Second", "Third"] {
                            Button::new(label).show(ui);
                        }
                    });
                });
                group(row, "big", "Big", &["Large"]);
            });
            if let Some(controls) = shown.controls {
                *record.borrow_mut() = Some(controls);
            }
        });
        if !ui.is_sizing_pass() && !is_measuring(ui.ctx()) {
            out.set(shown.output.rect);
        }
    });
    fixture.room(vec2(900.0, 400.0));
    // A group is the outermost element of the ribbon's record named by its title: the record maps it through the
    // slide's layer, which shows nothing in its first frame.
    let seen = |_: &Fixture| {
        let recorded = controls.borrow();
        let elements: &[Element] = recorded.as_deref().map_or(&[], |recorded| recorded.elements.as_slice());
        let group = |title: &str| {
            let named = |element: &&Element| element.part.is_none() && element.facts.as_ref().and_then(|facts| facts.name.as_deref()) == Some(title);
            elements.iter().filter(named).min_by_key(|element| element.depth).map_or(egui::Rect::NOTHING, |element| element.rect)
        };
        (rect.get(), [group("Short"), group("Rows"), group("Big")])
    };
    fixture.frame(Vec::new());
    let (first, mut groups) = seen(&fixture);
    for _ in 0..2 {
        if groups.iter().all(|group| group.is_finite()) {
            break;
        }
        fixture.frame(Vec::new());
        groups = seen(&fixture).1;
    }
    fixture.settle();
    [(first, groups), seen(&fixture)]
}

/// The ribbon measures right in its first frame: no group stands taller than its content and title then, so a host
/// that lays out under the ribbon (a document view scrolling its caret into view) never sees a taller ribbon first.
#[trial]
fn the_ribbon_is_its_settled_height_in_its_first_frame() {
    let [(first, _), (settled, _)] = first_and_settled();
    assert!(settled.height() > 0.0, "the ribbon shows: {settled:?}");
    assert!((first.height() - settled.height()).abs() < 0.5, "the first frame's ribbon {first:?} is as tall as the settled one {settled:?}");
}

/// Every group fills the row's line in the first frame that shows it, so every title, at its group's foot, sits on the
/// settled line from the start.
#[trial]
fn group_titles_sit_at_the_foot_in_the_first_frame() {
    let [(_, first), (_, settled)] = first_and_settled();
    let foot = settled[1].bottom();
    for (title, (first, settled)) in ["Short", "Rows", "Big"].into_iter().zip(first.into_iter().zip(settled)) {
        assert!(
            (first.bottom() - foot).abs() < 0.5,
            "in the first frame that shows it the group {title} {first:?} ends on the settled foot ({foot})"
        );
        assert!((settled.bottom() - foot).abs() < 0.5, "once settled the group {title} {settled:?} ends on the row's foot ({foot})");
    }
}
