//! A docked panel group.

use craft_ui::buttons::Button;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::docking::{DockGroup, DragUnit, GroupGrip, PanelKey, PanelTitle, Titles};
use craft_ui::testing::{Fixture, Frames, pointer, trial};
use egui::{Pos2, Rect, Vec2};
use std::cell::Cell;
use std::rc::Rc;

// A minimized group shows its header alone: its body does not run, so it leaves no node and nothing a press could
// reach under the groups below it. Expanded, the same body shows.
#[trial]
fn a_minimized_group_runs_no_body() {
    let body = |minimized: bool| {
        let runs = Rc::new(Cell::new(0));
        let counted = Rc::clone(&runs);
        let panels = vec![PanelTitle::new(PanelKey::new(1), "Color")];
        let mut fixture = Fixture::new(move |ui| {
            let group = DockGroup::new(1usize, Titles::from_content(panels.clone()), panels[0].key);
            let group = if minimized { group.minimized() } else { group };
            let counted = Rc::clone(&counted);
            group.show_at(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(240.0, 200.0)), move |ui, _| {
                counted.set(counted.get() + 1);
                Button::new("Body button").show(ui);
            });
        });
        fixture.idle(4);
        let found = fixture.by_label("Body button").is_some();
        (runs.get(), found)
    };
    let (runs, found) = body(false);
    assert!(runs > 0 && found, "an expanded group shows its body: {runs} runs, node {found}");
    assert_eq!(body(true), (0, false), "a minimized group runs no body and leaves no node");
}

// A panel's count follows its name on its tab and in the tab's name; the shown panel's caption sits in the header
// under the tabs and above the body, and goes when another panel of the group is shown.
#[trial]
fn the_shown_panels_caption_sits_under_the_tabs_and_its_count_names_its_tab() {
    let shown = Rc::new(Cell::new(0usize));
    let picked = Rc::clone(&shown);
    let panels = vec![
        PanelTitle::new(PanelKey::new(1), "Comments").count(Some(3)).caption(Some("2 replies".into())),
        PanelTitle::new(PanelKey::new(2), "Pages").count(Some(9)),
    ];
    let mut fixture = Fixture::new(move |ui| {
        let group = DockGroup::new(1usize, Titles::from_content(panels.clone()), panels[picked.get()].key);
        let out = group.show_at(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(300.0, 240.0)), |ui, _| {
            Button::new("Body button").show(ui);
        });
        if let Some(key) = out.output.picked {
            picked.set(panels.iter().position(|panel| panel.key == key).unwrap_or(0));
        }
    });
    fixture.settle();
    let Some(tab) = fixture.by_label("Comments, 3") else { panic!("the tab is named with its count") };
    let Some(caption) = fixture.by_label("2 replies") else { panic!("the shown panel's caption shows") };
    let Some(body) = fixture.by_label("Body button") else { panic!("the body shows") };
    assert!(caption.rect.top() >= tab.rect.bottom() - 0.5, "the caption {:?} is under the tab {:?}", caption.rect, tab.rect);
    assert!(body.rect.top() >= caption.rect.bottom() - 0.5, "the body {:?} is under the caption {:?}", body.rect, caption.rect);
    let Some(pages) = fixture.by_label("Pages, 9") else { panic!("the other tab is named with its count") };
    fixture.click(&pages).settle();
    assert_eq!(shown.get(), 1, "the click shows Pages");
    assert!(fixture.by_label("2 replies").is_none(), "Comments' caption goes with it");
}

// Beside pill and block tabs (VectorCraft's) the panel menu button stands in the strip's middle, level with the shown
// tab's middle; beside plain tabs it keeps to the strip's top.
#[trial]
fn the_panel_menu_button_stands_level_with_pill_and_block_tabs() {
    use craft_ui::menu::PanelRow;
    use craft_ui::tabs::PanelTabLook;
    let middles = |look: PanelTabLook| {
        let panels = vec![PanelTitle::new(PanelKey::new(1), "Layers"), PanelTitle::new(PanelKey::new(2), "Pages")];
        let mut fixture = Fixture::new(move |ui| {
            let group = DockGroup::new(1usize, Titles::from_content(panels.clone()), panels[0].key).menu(Vec::<PanelRow>::new).tab_look(look);
            let _ = group.show_at(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(300.0, 240.0)), |_, _| ());
        });
        fixture.settle();
        let Some(tab) = fixture.by_label("Layers") else { panic!("the shown tab shows") };
        let Some(menu) = fixture.by_label("Panel Menu") else { panic!("the panel menu button shows") };
        (tab.rect.center().y, menu.rect)
    };
    let (_, plain) = middles(PanelTabLook::Plain);
    for look in [PanelTabLook::Pill, PanelTabLook::Block] {
        let (tab, menu) = middles(look);
        assert!((menu.center().y - tab).abs() <= 0.5, "beside {look:?} tabs the menu button {menu:?} is level with the tab's middle {tab}");
        assert!(menu.top() > plain.top() + 0.5, "beside plain tabs the button {plain:?} keeps to the strip's top, above {menu:?}");
    }
}

// A titled group shows the shown panel's name as a heading with its count and a close button, not a tab strip: the
// panel's name is not cut to a tab's width, and the close button closes the shown panel.
#[trial]
fn a_titled_group_shows_the_panel_title_and_a_close_button_instead_of_tabs() {
    let closed = Rc::new(Cell::new(false));
    let seen = Rc::clone(&closed);
    let panels = vec![PanelTitle::new(PanelKey::new(1), "Accessibility Checker").closable(true), PanelTitle::new(PanelKey::new(2), "Pages")];
    let mut fixture = Fixture::new(move |ui| {
        let out = DockGroup::new(1usize, Titles::from_content(panels.clone()), panels[0].key).titled().closable().show_at(
            ui,
            Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(300.0, 240.0)),
            |ui, _| {
                Button::new("Body button").show(ui);
            },
        );
        if out.output.closed.contains(&panels[0].key) {
            seen.set(true);
        }
    });
    fixture.settle();
    assert!(fixture.by_label("Accessibility Checker").is_some(), "the title shows whole");
    assert!(fixture.by_label("Pages").is_none(), "no tab for the other panel");
    let Some(close) = fixture.by_label("Close Accessibility Checker") else { panic!("a close button shows") };
    fixture.click(&close).settle();
    assert!(closed.get(), "it closes the shown panel");
}

// A floating window of a quick-action strip is a bare grip on top: no menu, float or close action, and no column
// arrow, as production's floating toolbar is a rounded strip with grip dots.
#[trial]
fn a_grip_window_shows_a_grip_and_no_title_actions() {
    use craft_ui::docking::{FloatingGroup, WindowColumn};
    let grip = Rc::new(Cell::new(false));
    let seen = Rc::clone(&grip);
    let mut fixture = Fixture::new(move |ui| {
        let group = FloatingGroup::new("Quick tools", Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(60.0, 200.0)))
            .column(WindowColumn::expanded(craft_behavior::dock::DockEdge::Start))
            .grip(seen.get());
        let _ = group.show_group(ui, |ui, _| {
            Button::new("Select").show(ui);
        });
    });
    fixture.settle();
    let actions = |fixture: &Fixture| ["Close", "Panel Menu", "Float"].iter().filter(|name| fixture.by_label(name).is_some()).count();
    assert!(actions(&fixture) > 0, "an ordinary floating window shows its title actions");
    grip.set(true);
    fixture.settle();
    assert_eq!(actions(&fixture), 0, "a grip window shows none");
    assert!(fixture.by_label("Select").is_some(), "the body stays");
}

// A grip title row draws no rule under it, so the window keeps no band for one: its body starts the rule's width
// (a point or more) higher than an ordinary window's, whose rule takes a band of its own.
#[trial]
fn a_grip_window_keeps_no_band_for_a_title_rule() {
    use craft_ui::docking::{FloatingGroup, WindowColumn};
    let body_top = |grip: bool| {
        let mut fixture = Fixture::new(move |ui| {
            let group = FloatingGroup::new("Quick tools", Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(120.0, 200.0)))
                .column(WindowColumn::expanded(craft_behavior::dock::DockEdge::Start))
                .grip(grip);
            let _ = group.show_group(ui, |ui, _| {
                Button::new("Select").show(ui);
            });
        });
        fixture.settle();
        fixture.by_label("Select").map_or(f32::NAN, |button| button.rect.top())
    };
    let (ruled, grip) = (body_top(false), body_top(true));
    assert!(ruled - grip >= 0.5, "the grip window's body ({grip}) starts above the ruled window's ({ruled})");
}

// A floating window's resize grips are named, so assistive technology and automation address them: each edge and
// corner the window declares is one node.
#[trial]
fn a_floating_window_names_each_resize_grip() {
    use craft_ui::contract::Role;
    use craft_ui::docking::FloatingGroup;
    let mut fixture = Fixture::new(move |ui| {
        let group = FloatingGroup::new("Quick tools", Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(120.0, 200.0)));
        let _ = group.show_group(ui, |ui, _| {
            Button::new("Select").show(ui);
        });
    });
    fixture.settle();
    let grips = fixture.by_role(Role::SPLITTER);
    assert!(grips.len() >= 4, "a resizable window declares a grip on each edge");
    for grip in &grips {
        assert_eq!(craft_ui::testing::access::name(fixture.tree(), grip.node), Some("Resize panel window"), "every grip is named");
    }
}

// A toolbar's marked grip is a handle (photocraft#16467): the grab cursor over it and grabbing while it is held, as
// the grips of a collapsed column show. An unmarked band and a grip that does not move keep the plain arrow.
#[trial]
fn a_marked_grip_shows_the_grab_cursor() {
    let strip = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(60.0, 16.0));
    for (ticks, movable) in [(true, true), (false, true), (true, false)] {
        let mut frames = Frames::new();
        let mut show = |ui: &mut egui::Ui| {
            GroupGrip::new(DragUnit::Group(1usize), PanelKey::new(1)).ticks(ticks).movable(movable).show_at(ui, strip);
        };
        frames.frame(vec![], &mut show);
        frames.frame(vec![pointer::moved(strip.center())], &mut show);
        frames.frame(vec![], &mut show);
        let (over, held) = if ticks && movable {
            (egui::CursorIcon::Grab, egui::CursorIcon::Grabbing)
        } else {
            (egui::CursorIcon::Default, egui::CursorIcon::Default)
        };
        assert_eq!(frames.cursor_icon(), over, "over the grip (ticks {ticks}, movable {movable})");
        frames.frame(vec![pointer::press(strip.center())], &mut show);
        frames.frame(vec![], &mut show);
        assert_eq!(frames.cursor_icon(), held, "the grip held (ticks {ticks}, movable {movable})");
    }
}

// A panel's toggle command marks its tab (the marker dot) as it marks the panel's rail button. The marks resolve
// outside the memoized tab row and join its key, so a mark that comes or goes repaints the settled row.
#[trial]
fn a_tab_mark_change_repaints_the_tab_row() {
    use craft_ui::runtime::commands::{CommandRef, CommandState, EditCommands};
    let command: CommandRef = EditCommands::Undo.into();
    let panels = vec![PanelTitle::new(PanelKey::new(1), "Brushes").command(Some(command.clone())), PanelTitle::new(PanelKey::new(2), "Swatches")];
    let mut fixture = Fixture::new(move |ui| {
        let group = DockGroup::new(1usize, Titles::from_content(panels.clone()), panels[1].key);
        group.show_at(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(400.0, 200.0)), |_, _| {});
    });
    let width = |fixture: &mut Fixture| fixture.by_label("Brushes").map_or(0.0, |tab| tab.rect.width());
    fixture.settle();
    let plain = width(&mut fixture);
    fixture.command_state(command.clone(), CommandState::new().checked(false).marked(true)).settle();
    let marked = width(&mut fixture);
    assert!(marked > plain, "the mark's dot widens the settled tab: {plain} then {marked}");
    fixture.command_state(command, CommandState::new().checked(false)).settle();
    let cleared = width(&mut fixture);
    assert!((cleared - plain).abs() < 0.5, "the tab loses its dot again: {plain} then {cleared}");
}

// A click on an app's own egui widget in a group's body (a raw text field) is the widget's, not a press on the
// group's empty body: the group reads no surface press, so its stop does not take the focus, and the field keeps
// egui's focus and takes typing. A press on the body's empty room still focuses the group's panel.
#[trial]
fn a_click_on_an_app_field_in_the_body_keeps_its_focus() {
    use std::cell::RefCell;
    let text = Rc::new(RefCell::new(String::new()));
    let field = Rc::new(Cell::new(None::<(egui::Id, Rect)>));
    let surface = Rc::new(Cell::new(false));
    let (typed, shown, pressed) = (Rc::clone(&text), Rc::clone(&field), Rc::clone(&surface));
    let panels = vec![PanelTitle::new(PanelKey::new(1), "Color")];
    let mut fixture = Fixture::new(move |ui| {
        let group = DockGroup::new(1usize, Titles::from_content(panels.clone()), panels[0].key);
        let (typed, shown, pressed) = (Rc::clone(&typed), Rc::clone(&shown), Rc::clone(&pressed));
        group.show_at(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(300.0, 240.0)), move |ui, cx| {
            pressed.set(pressed.get() || cx.surface_pressed());
            let response = egui::TextEdit::singleline(&mut *typed.borrow_mut()).desired_width(120.0).show(ui).response;
            shown.set(Some((response.id, response.rect)));
        });
    });
    fixture.settle();
    let Some((field_id, field_rect)) = field.get() else { panic!("the body shows the app's field") };
    let empty = Pos2::new(field_rect.center().x, field_rect.bottom() + 80.0);

    // Typing a character at a time with a click before each. The clicks are a second apart, so egui sees no double
    // click (which would select the word).
    for (clicks, typed) in ["a", "1"].into_iter().enumerate() {
        fixture.advance(1.0).click_at(field_rect.center()).settle();
        assert!(!surface.get(), "click {clicks} on the field is no press on the group's empty body");
        assert_eq!(fixture.focused(), Some(field_id), "click {clicks} leaves egui's focus on the app's field");
        fixture.type_text(typed).settle();
    }
    let on_field = fixture.focused_stop();
    assert!(text.borrow().contains("a1"), "the field takes the typing after each click: {:?}", text.borrow());

    fixture.click_at(empty).settle();
    let group_stop = fixture.focused_stop();
    assert!(surface.get(), "a press on the empty body is the group's surface press");
    assert!(group_stop.is_some() && fixture.focused() != Some(field_id), "a press on the empty body focuses the group's panel");
    assert_ne!(on_field, group_stop, "the clicks on the field left the group's stop unfocused");
}

// A press on an app's own egui widget in a group's body that only drags (a canvas) is the widget's, not a press on the
// group's empty body; a press on the body's empty room still is.
#[trial]
fn a_press_on_an_app_drag_widget_in_the_body_is_no_surface_press() {
    let surface = Rc::new(Cell::new(false));
    let canvas = Rc::new(Cell::new(None::<Rect>));
    let (pressed, shown) = (Rc::clone(&surface), Rc::clone(&canvas));
    let panels = vec![PanelTitle::new(PanelKey::new(1), "Navigator")];
    let mut fixture = Fixture::new(move |ui| {
        let group = DockGroup::new(1usize, Titles::from_content(panels.clone()), panels[0].key);
        let (pressed, shown) = (Rc::clone(&pressed), Rc::clone(&shown));
        group.show_at(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(300.0, 240.0)), move |ui, cx| {
            pressed.set(pressed.get() || cx.surface_pressed());
            let (rect, _) = ui.allocate_exact_size(Vec2::new(120.0, 60.0), egui::Sense::drag());
            shown.set(Some(rect));
        });
    });
    fixture.settle();
    let Some(rect) = canvas.get() else { panic!("the body shows the app's canvas") };
    fixture.click_at(rect.center()).settle();
    assert!(!surface.get(), "a press on the drag-only widget is no press on the group's empty body");
    fixture.click_at(Pos2::new(rect.center().x, rect.bottom() + 80.0)).settle();
    assert!(surface.get(), "a press on the empty body is the group's surface press");
}

// As a group narrows, at both densities, the drag band narrows to its least before the shown tab goes below the width a
// shrunk tab keeps (its first letters), alone and beside a tab that goes to the overflow menu. The shown tab stays
// inside the group, and the band never covers it, down to a 40 pt column.
#[trial]
fn a_narrow_groups_drag_band_narrows_before_the_shown_tab() {
    use craft_theme::Size;
    let tab = |width: f32, density: Size, names: &'static [&'static str]| {
        let panels: Vec<PanelTitle> = names.iter().enumerate().map(|(at, name)| PanelTitle::new(PanelKey::new(at as u64 + 1), *name)).collect();
        let group_rect = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(width, 200.0));
        let grip = Rc::new(Cell::new(None::<Rect>));
        let seen = Rc::clone(&grip);
        let mut fixture = Fixture::new(move |ui| {
            let out = DockGroup::new(1usize, Titles::from_content(panels.clone()), panels[0].key).show_at(ui, group_rect, |_, _| ());
            seen.set(out.output.grip.map(|(rect, _)| rect));
        });
        fixture.density(density).settle();
        (fixture.by_label("Tools").map(|node| node.rect), grip.get())
    };
    for density in [Size::Compact, Size::Comfortable] {
        // The least a tab shows: the narrowest "Tools" gets beside a shown tab as the group narrows by 1 pt steps, before
        // it overflows; so within 1 pt of the strip's least.
        let least = (0..260)
            .filter_map(|step| tab(300.0 - step as f32, density, &["Tools Options", "Tools"]).0)
            .map(|rect| rect.width())
            .fold(f32::INFINITY, f32::min);
        assert!(least.is_finite(), "the other tab shows shrunk in some group ({density:?})");
        for names in [&["Tools"][..], &["Tools", "Swatches"][..]] {
            let (Some(natural), Some(whole)) = tab(300.0, density, names) else { panic!("a wide group shows the tab and the band ({density:?})") };
            let least = least.min(natural.width());
            // The band at its narrowest, in the narrowest group.
            let (_, Some(narrowest)) = tab(40.0, density, names) else { panic!("a 40 pt group shows the band ({density:?}, {names:?})") };
            assert!(narrowest.width() < whole.width() - 0.5, "the band narrows in a 40 pt group ({density:?}, {names:?})");
            for width in (0..13).map(|step| 40.0 + 8.0 * step as f32) {
                let (shown, grip) = tab(width, density, names);
                let Some(shown) = shown else { panic!("the shown tab shows in a {width} pt group ({density:?}, {names:?})") };
                let Some(grip) = grip else { panic!("the band shows in a {width} pt group ({density:?}, {names:?})") };
                let group = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(width, 200.0));
                assert!(group.contains_rect(shown), "the shown tab {shown:?} lies inside the {width} pt group ({density:?}, {names:?})");
                assert!(!grip.intersects(shown.shrink(0.5)), "the band {grip:?} does not cover the shown tab {shown:?} in {width} pt ({density:?})");
                assert!(
                    shown.width() >= least - 1.0 || grip.width() <= narrowest.width() + 0.5,
                    "the band {grip:?} narrows to {} pt before the shown tab {shown:?} goes below {least} pt in {width} pt ({density:?}, {names:?})",
                    narrowest.width()
                );
            }
        }
    }
}

// The host's items before the tab strip count in the room the header keeps: as a group with one or two of them narrows,
// at both densities and in both directions, the drag band narrows to its least before the shown tab goes below the
// width a shrunk tab keeps, and the host's items stay inside the group, clear of the band and the shown tab.
#[trial]
fn a_narrow_groups_drag_band_leaves_room_for_the_hosts_header_items() {
    use craft_behavior::IconId;
    use craft_theme::Size;
    use craft_ui::buttons::IconButton;
    use craft_ui::contract::Props as _;
    use craft_ui::runtime::env::Direction;
    const HOSTS: [&str; 2] = ["Host one", "Host two"];
    let tab = |width: f32, (density, direction): (Size, Direction), names: &'static [&'static str], hosts: usize| {
        let panels: Vec<PanelTitle> = names.iter().enumerate().map(|(at, name)| PanelTitle::new(PanelKey::new(at as u64 + 1), *name)).collect();
        let group_rect = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(width, 200.0));
        let grip = Rc::new(Cell::new(None::<Rect>));
        let seen = Rc::clone(&grip);
        let mut fixture = Fixture::new(move |ui| {
            let group = DockGroup::new(1usize, Titles::from_content(panels.clone()), panels[0].key).extra(move |ui, _| {
                for name in &HOSTS[..hosts] {
                    IconButton::new(IconId::new("settings")).label(*name).show(ui);
                }
            });
            let out = group.show_at(ui, group_rect, |_, _| ());
            seen.set(out.output.grip.map(|(rect, _)| rect));
        });
        fixture.density(density).direction(direction).settle();
        let host = HOSTS[..hosts].iter().map(|name| fixture.by_label(name).map(|node| node.rect)).collect::<Vec<_>>();
        (fixture.by_label("Tools").map(|node| node.rect), grip.get(), host)
    };
    for density in [Size::Compact, Size::Comfortable] {
        // The least a tab shows, found as the trial above finds it, with no host items.
        let least = (0..260)
            .filter_map(|step| tab(300.0 - step as f32, (density, Direction::Ltr), &["Tools Options", "Tools"], 0).0)
            .map(|rect| rect.width())
            .fold(f32::INFINITY, f32::min);
        assert!(least.is_finite(), "the other tab shows shrunk in some group ({density:?})");
        let looks = [Direction::Ltr, Direction::Rtl].map(|direction| (density, direction));
        for (look, names) in looks.into_iter().flat_map(|look| [(look, &["Tools"][..]), (look, &["Tools", "Swatches"][..])]) {
            for hosts in 1..=HOSTS.len() {
                let (Some(natural), _, _) = tab(300.0, look, names, hosts) else { panic!("a wide group shows the tab ({look:?})") };
                let least = least.min(natural.width());
                let (_, Some(narrowest), _) = tab(40.0, look, names, 0) else { panic!("a 40 pt group shows the band ({look:?})") };
                for width in (0..17).map(|step| 40.0 + 8.0 * step as f32) {
                    let (shown, grip, host) = tab(width, look, names, hosts);
                    let case = format!("{width} pt, {look:?}, {names:?}, {hosts} host items");
                    let Some(grip) = grip else { panic!("the band shows ({case})") };
                    let group = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(width, 200.0));
                    // With no tab in the strip, the group's own node carries the shown panel's name.
                    let shown = shown.filter(|shown| shown.width() < group.width() - 0.5);
                    let shown_width = shown.map_or(0.0, |shown| shown.width());
                    // A band above its least leaves the line room for the host's items.
                    if grip.width() > narrowest.width() + 0.5 {
                        for item in &host {
                            let Some(item) = item else { panic!("each host item shows ({case})") };
                            assert!(group.contains_rect(*item), "the host item {item:?} lies inside the group ({case})");
                            assert!(!grip.intersects(item.shrink(0.5)), "the band {grip:?} does not cover the host item {item:?} ({case})");
                        }
                        if let Some(shown) = shown {
                            assert!(!grip.intersects(shown.shrink(0.5)), "the band {grip:?} does not cover the shown tab {shown:?} ({case})");
                        }
                    }
                    assert!(
                        shown_width >= least - 1.0 || grip.width() <= narrowest.width() + 0.5,
                        "the band {grip:?} narrows to {} pt before the shown tab ({shown_width} pt) goes below {least} pt ({case})",
                        narrowest.width()
                    );
                }
            }
        }
    }
}

// Closable panel tabs stay panel tabs: in a group too narrow for their whole names every tab keeps its place in the
// strip, the shown one its whole name, and each shows a close button. A middle click closes an inactive tab without
// showing it first.
#[trial]
fn closable_panel_tabs_shrink_and_close_on_a_middle_click() {
    let (picked, closed) = (Rc::new(Cell::new(None::<PanelKey>)), Rc::new(Cell::new(None::<PanelKey>)));
    let (picks, closes) = (Rc::clone(&picked), Rc::clone(&closed));
    let names = ["Character", "Paragraph", "Glyphs"];
    let panels: Vec<PanelTitle> = (0..3).map(|at| PanelTitle::new(PanelKey::new(at as u64 + 1), names[at]).closable(true)).collect();
    let keys: Vec<PanelKey> = panels.iter().map(|panel| panel.key).collect();
    let group_rect = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(300.0, 200.0));
    let mut fixture = Fixture::new(move |ui| {
        let out = DockGroup::new(1usize, Titles::from_content(panels.clone()), panels[0].key).closable().show_at(ui, group_rect, |_, _| ());
        if let Some(key) = out.output.picked {
            picks.set(Some(key));
        }
        if let Some(key) = out.output.closed.first() {
            closes.set(Some(*key));
        }
    });
    fixture.settle();
    let tabs: Vec<_> = names.iter().map(|name| fixture.by_label(name).unwrap_or_else(|| panic!("the {name} tab stays in the strip"))).collect();
    for (name, tab) in names.iter().zip(&tabs) {
        assert!(group_rect.contains_rect(tab.rect), "the {name} tab {:?} lies inside the group", tab.rect);
        assert!(fixture.by_label(&format!("Close {name}")).is_some(), "the {name} tab shows a close button");
    }
    // Paragraph's name is as long as Character's: shrunk beside the shown tab, which keeps its whole name.
    assert!(tabs[1].rect.width() < tabs[0].rect.width() - 0.5, "the other tabs shrink and the shown tab keeps its whole name");
    assert!(fixture.by_label("More tabs").is_none(), "no tab goes to the overflow menu while its first letters fit");
    let at = tabs[2].center();
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::middle(at, true)]);
    fixture.frame(vec![pointer::middle(at, false)]).settle();
    assert_eq!(closed.get(), Some(keys[2]), "the middle click closes the tab");
    assert_eq!(picked.get(), None, "the middle click does not show the tab first");
}

// An AccessKit click on a closable tab's close button closes the tab, as a click on it does, though the tab row has
// settled and replays its last run.
#[trial]
fn an_access_click_on_a_settled_tabs_close_button_closes_it() {
    use craft_ui::testing::focus::AccessActionKind;
    let closed = Rc::new(Cell::new(None::<PanelKey>));
    let closes = Rc::clone(&closed);
    let panels = vec![PanelTitle::new(PanelKey::new(1), "Color").closable(true), PanelTitle::new(PanelKey::new(2), "Swatches").closable(true)];
    let key = panels[1].key;
    let mut fixture = Fixture::new(move |ui| {
        let group = DockGroup::new(1usize, Titles::from_content(panels.clone()), panels[0].key).closable();
        let out = group.show_at(ui, Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(300.0, 240.0)), |_, _| ());
        if let Some(key) = out.output.closed.first() {
            closes.set(Some(*key));
        }
    });
    fixture.idle(4);
    let Some(close) = fixture.by_label("Close Swatches") else { panic!("the Swatches tab shows a close button") };
    fixture.access(AccessActionKind::Click, &close).settle();
    assert_eq!(closed.get(), Some(key), "the access click closes the tab");
}
