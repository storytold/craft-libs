use egui_kittest::{Harness, kittest::Queryable as _};

use super::ComboBox;

struct Menu {
    filter: String,
    button: Option<egui::Id>,
}

fn popup(h: &Harness<'_, Menu>) -> egui::Id {
    super::popup_id(h.state().button.unwrap_or(egui::Id::NULL))
}

/// The menu's height as egui keeps it between passes.
fn height(h: &Harness<'_, Menu>) -> f32 {
    egui::AreaState::load(&h.ctx, popup(h)).and_then(|s| s.size).map_or(0.0, |s| s.y)
}

#[test]
fn a_filtered_menu_grows_back_when_the_filter_is_cleared() {
    let mut h = Harness::builder().with_size(egui::vec2(600.0, 800.0)).build_ui_state(
        |ui, m: &mut Menu| {
            let inner = ComboBox::new("language", "Language").selected_text("Any language").width(200.0).height(380.0).show_ui(ui, |ui| {
                let name = ui.label("Find a language").id;
                ui.text_edit_singleline(&mut m.filter).labelled_by(name);
                let rows = if m.filter.is_empty() { 60 } else { 0 };
                let row = ui.spacing().interact_size.y;
                egui::ScrollArea::vertical().max_height(300.0).auto_shrink([false, true]).show_rows(ui, row, rows, |ui, range| {
                    for n in range {
                        let _ = ui.selectable_label(false, format!("Language {n}"));
                    }
                });
            });
            m.button = Some(inner.response.id);
        },
        Menu { filter: String::new(), button: None },
    );
    h.run();
    egui::Popup::open_id(&h.ctx, popup(&h));
    h.run();
    let full = height(&h);
    assert!(full > 300.0, "the open menu shows the list: {full}");

    h.state_mut().filter = "xxxx".into();
    h.run();
    let none = height(&h);
    assert!(none < 100.0, "no matches shrink the menu: {none}");

    h.state_mut().filter.clear();
    h.run();
    let again = height(&h);
    assert!((again - full).abs() < 1.0, "cleared, the menu is {again} tall, not {full} as when it opened");
}

struct Search {
    filter: String,
    scroll: bool,
    field: egui::Rect,
    first_row: usize,
    button: Option<egui::Id>,
}

fn search_menu() -> Harness<'static, Search> {
    Harness::builder().with_size(egui::vec2(600.0, 800.0)).build_ui_state(
        |ui, m: &mut Search| {
            let row = ui.spacing().interact_size.y;
            let header = |ui: &mut egui::Ui| {
                let name = ui.label("Find a language").id;
                m.field = ui.text_edit_singleline(&mut m.filter).labelled_by(name).rect;
                if m.filter.is_empty() { 60 } else { 0 }
            };
            let rows = |ui: &mut egui::Ui, range: std::ops::Range<usize>, _: usize| {
                if std::mem::take(&mut m.scroll) {
                    ui.scroll_with_delta(egui::vec2(0.0, -200.0));
                }
                m.first_row = range.start;
                for n in range {
                    let _ = ui.selectable_label(false, format!("Language {n}"));
                }
            };
            let inner = ComboBox::new("language", "Language").selected_text("Any language").width(200.0).height(380.0).show_rows_with_header(
                ui,
                header,
                row,
                |&n| n,
                rows,
            );
            m.button = Some(inner.response.id);
        },
        Search { filter: String::new(), scroll: false, field: egui::Rect::NOTHING, first_row: 0, button: None },
    )
}

fn search_height(h: &Harness<'_, Search>) -> f32 {
    let popup = super::popup_id(h.state().button.unwrap_or(egui::Id::NULL));
    egui::AreaState::load(&h.ctx, popup).and_then(|s| s.size).map_or(0.0, |s| s.y)
}

#[test]
fn a_menu_header_stays_put_while_the_list_scrolls_and_the_menu_grows_back() {
    let mut h = search_menu();
    h.run();
    h.get_by_role_and_label(egui::Role::ComboBox, "Language").click();
    h.run();
    let (field, full) = (h.state().field, search_height(&h));
    assert!(full > 300.0, "the open menu shows the list: {full}");
    assert!(full < 400.0, "the menu's contents keep to its height of 380, inside its frame: {full}");

    h.state_mut().scroll = true;
    h.run();
    assert!(h.state().first_row > 0, "the list scrolled");
    assert_eq!(h.state().field, field, "the search field stays put");

    h.state_mut().filter = "xxxx".into();
    h.run();
    let none = search_height(&h);
    assert!(none < 100.0, "no matches shrink the menu: {none}");

    h.state_mut().filter.clear();
    h.run();
    let again = search_height(&h);
    assert!((again - full).abs() < 1.0, "cleared, the menu is {again} tall, not {full} as when it opened");
}

const SIZES: [&str; 3] = ["Small", "Medium", "Large"];

#[test]
fn show_index_picks_a_choice_and_marks_the_response_changed() {
    let mut h = Harness::new_ui_state(
        |ui, s: &mut (usize, bool)| {
            let response = ComboBox::from_label("Size").show_index(ui, &mut s.0, SIZES.len(), |i| SIZES[i]);
            s.1 |= response.changed();
        },
        (0, false),
    );
    h.run();
    h.get_by_role_and_label(egui::Role::ComboBox, "Size").click();
    h.run();
    h.get_by_label("Large").click();
    h.run();
    assert_eq!(*h.state(), (2, true));
    assert!(h.query_by_label("Medium").is_none(), "picking closes the menu");
}

fn sizes(ui: &mut egui::Ui, rows: usize) {
    for n in 0..rows {
        let size = SIZES.get(n).map_or_else(|| format!("Row {n}"), |s| (*s).to_owned());
        let _ = ui.selectable_label(n == 1, size);
    }
}

/// Combo boxes in the states a user sees them in, drawn with egui's combo box or ours.
#[expect(clippy::disallowed_types, reason = "the reference our combo box is compared with")]
fn scene(ui: &mut egui::Ui, ours: bool) {
    macro_rules! both {
        ($ui:ident, $rows:expr; $ctor:ident ( $($arg:expr),* ) $(. $m:ident ( $($a:expr),* ))*) => {
            if ours {
                ComboBox::$ctor($($arg),*)$(.$m($($a),*))*.show_ui($ui, |ui| sizes(ui, $rows)).response
            } else {
                egui::ComboBox::$ctor($($arg),*)$(.$m($($a),*))*.show_ui($ui, |ui| sizes(ui, $rows)).response
            }
        };
    }
    let _ = both!(ui, 3; from_label("Size").selected_text("Medium"));
    let _ = both!(ui, 3; from_id_salt("narrow").selected_text("A long selected value that is cut short").width(120.0).truncate());
    let _ = both!(ui, 3; new("wrapped", "Wrapped").selected_text("A long selected value that wraps onto a second line").width(140.0).wrap());
    let _ = ui.add_enabled_ui(false, |ui| both!(ui, 3; new("disabled", "Disabled").selected_text("Medium")));
    let _ = both!(ui, 3; new("open", "Open").selected_text("Medium").width(160.0));
    let _ = both!(ui, 40; new("long", "Long").selected_text("Row 1").height(300.0));
}

fn render(ours: bool, hover: Option<&str>, open: Option<&str>) -> image::RgbaImage {
    // Two of the combo boxes have no label: the scene compares looks, not names.
    let mut h = Harness::builder().with_size(egui::vec2(480.0, 700.0)).with_accessibility_check(false).build_ui(move |ui| scene(ui, ours));
    h.run();
    if let Some(label) = open {
        h.get_by_role_and_label(egui::Role::ComboBox, label).click();
        h.run();
    }
    if let Some(label) = hover {
        h.get_by_role_and_label(egui::Role::ComboBox, label).hover();
    }
    h.run();
    h.render().unwrap_or_else(|e| panic!("render: {e}"))
}

#[test]
fn the_button_and_menu_look_like_eguis() {
    for (hover, open) in [(None, None), (Some("Size"), None), (None, Some("Open")), (None, Some("Long"))] {
        let (egui, ours) = (render(false, hover, open), render(true, hover, open));
        let differ = egui.pixels().zip(ours.pixels()).filter(|(a, b)| a != b).count();
        assert_eq!(differ, 0, "hover {hover:?}, open {open:?}: {differ} pixels differ");
    }
}

/// A combo box at `top` in a window `screen` tall, with `rows` rows (none while filtered).
struct Long {
    rows: usize,
    filtered: bool,
    to_end: bool,
    last_row: egui::Rect,
    button: egui::Rect,
    popup: egui::Id,
}

fn long_menu(top: f32, screen: f32, height: f32, rows: usize) -> Harness<'static, Long> {
    Harness::builder().with_size(egui::vec2(400.0, screen)).build_ui_state(
        move |ui, m: &mut Long| {
            ui.add_space(top);
            let inner = ComboBox::new("long", "Long").selected_text("Row 0").height(height).show_ui(ui, |ui| {
                let shown = if m.filtered { 2 } else { m.rows };
                for n in 0..shown {
                    m.last_row = ui.selectable_label(false, format!("Row {n}")).rect;
                }
                if std::mem::take(&mut m.to_end) {
                    ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                }
            });
            m.button = inner.response.rect;
            m.popup = super::popup_id(inner.response.id);
        },
        Long { rows, filtered: false, to_end: false, last_row: egui::Rect::NOTHING, button: egui::Rect::NOTHING, popup: egui::Id::NULL },
    )
}

fn menu_rect(h: &Harness<'_, Long>) -> egui::Rect {
    h.ctx.memory(|m| m.area_rect(h.state().popup)).unwrap_or(egui::Rect::NOTHING)
}

#[test]
fn an_unbounded_menu_fits_on_screen_and_scrolls_to_its_last_row() {
    let mut h = long_menu(0.0, 300.0, f32::INFINITY, 100);
    h.run();
    h.get_by_role_and_label(egui::Role::ComboBox, "Long").click();
    h.run();
    let menu = menu_rect(&h);
    assert!(menu.bottom() <= 300.0 + 0.5, "the menu ends on screen: {menu:?}");
    h.state_mut().to_end = true;
    h.run();
    let last = h.state().last_row;
    assert!(last.bottom() <= menu_rect(&h).bottom() && last.top() >= menu_rect(&h).top(), "the last row {last:?} scrolls into the menu {menu:?}");
}

#[test]
fn a_menu_that_shrinks_and_grows_while_open_stays_on_its_side() {
    // Too little room below for the full list: the menu opens above. Filtered, it would fit
    // below too.
    let mut h = long_menu(480.0, 600.0, 380.0, 60);
    h.run();
    h.get_by_role_and_label(egui::Role::ComboBox, "Long").click();
    h.run();
    let above = |h: &Harness<'_, Long>| menu_rect(h).bottom() <= h.state().button.top() + 0.5;
    assert!(above(&h), "opens above: {:?}", menu_rect(&h));
    h.state_mut().filtered = true;
    h.run();
    assert!(above(&h), "stays above when it shrinks: {:?}", menu_rect(&h));
    h.state_mut().filtered = false;
    h.run();
    assert!(above(&h), "and when it grows back: {:?}", menu_rect(&h));
    assert!(menu_rect(&h).top() >= -0.5, "keeping to the room there: {:?}", menu_rect(&h));
}

#[test]
fn a_menu_opened_by_the_app_settles_its_side_once_measured() {
    // Opened with `Popup::open_id`, as photocraft's press-and-drag menus are, so it is open on
    // the pass it is first shown.
    let mut h = long_menu(480.0, 600.0, 380.0, 60);
    h.run();
    egui::Popup::open_id(&h.ctx, h.state().popup);
    h.run();
    let menu = menu_rect(&h);
    assert!(menu.bottom() <= h.state().button.top() + 0.5, "opens above, where there is room: {menu:?}");
    assert!(menu.height() > 300.0, "with the whole list: {menu:?}");
}

#[test]
fn a_footer_stays_under_the_list() {
    let mut h = Harness::builder().with_size(egui::vec2(400.0, 600.0)).build_ui_state(
        |ui, s: &mut (bool, egui::Rect, egui::Rect)| {
            let _ = ComboBox::new("buses", "Send").selected_text("Bus 1").height(300.0).show_ui_with_header_footer(
                ui,
                |ui| ui.label("Buses"),
                |ui, _| {
                    for n in 0..50 {
                        let _ = ui.selectable_label(false, format!("Bus {n}"));
                    }
                    if std::mem::take(&mut s.0) {
                        ui.scroll_with_delta(egui::vec2(0.0, -200.0));
                    }
                    s.1 = ui.clip_rect();
                },
                |ui| s.2 = ui.button("New bus…").rect,
            );
        },
        (false, egui::Rect::NOTHING, egui::Rect::NOTHING),
    );
    h.run();
    h.get_by_role_and_label(egui::Role::ComboBox, "Send").click();
    h.run();
    let (list, footer) = (h.state().1, h.state().2);
    assert!(footer.top() >= list.bottom() - 0.5, "the footer {footer:?} is under the list {list:?}");
    h.state_mut().0 = true;
    h.run();
    assert_eq!(h.state().2, footer, "the footer stays put while the list scrolls");
    assert!(h.query_by_label("New bus…").is_some());
}

struct Typed {
    text: String,
    open: bool,
    picked: Option<usize>,
}

/// A text field that searches its menu: the menu shows while `open`, which the field sets.
fn font_field() -> Harness<'static, Typed> {
    Harness::new_ui_state(
        |ui, t: &mut Typed| {
            let name = ui.label("Font").id;
            let field = ui.text_edit_singleline(&mut t.text).labelled_by(name);
            t.open |= field.gained_focus() || field.changed();
            let text = t.text.clone();
            let menu = super::Menu::new(ui, &field).open(&mut t.open);
            if let Some(Some(n)) = menu.show_ui(|ui| (0..3).find(|n| ui.selectable_label(false, format!("{text} {n}")).clicked())) {
                t.picked = Some(n);
            }
        },
        Typed { text: String::new(), open: false, picked: None },
    )
}

#[test]
fn a_menu_under_a_text_field_leaves_it_the_focus_and_takes_a_pick() {
    let mut h = font_field();
    h.run();
    h.get_by_role_and_label(egui::Role::TextInput, "Font").click();
    h.run();
    assert!(h.state().open, "focusing the field opens the menu");
    h.get_by_role_and_label(egui::Role::TextInput, "Font").type_text("ar");
    h.run();
    assert_eq!(h.state().text, "ar");
    assert!(h.get_by_role_and_label(egui::Role::TextInput, "Font").is_focused(), "the field keeps the focus");
    assert!(h.query_by_label("ar 1").is_some(), "the menu follows the text");

    h.get_by_role_and_label(egui::Role::TextInput, "Font").click();
    h.run();
    assert!(h.state().open, "a click in the field leaves the menu open");

    h.get_by_label("ar 1").click();
    h.run();
    assert_eq!(h.state().picked, Some(1), "a click on a row picks it");
    assert!(!h.state().open && h.query_by_label("ar 1").is_none(), "and closes the menu");

    h.get_by_role_and_label(egui::Role::TextInput, "Font").click();
    h.run();
    assert!(h.state().open && h.query_by_label("ar 1").is_some(), "focusing the field opens it again");
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(!h.state().open && h.query_by_label("ar 1").is_none(), "Escape closes it");
}

#[test]
fn a_menu_under_a_text_field_closes_when_the_focus_leaves_it() {
    for mouse_over_menu in [false, true] {
        let mut h = Harness::new_ui_state(
            |ui, t: &mut Typed| {
                let name = ui.label("Font").id;
                let field = ui.text_edit_singleline(&mut t.text).labelled_by(name);
                t.open |= field.gained_focus() || field.changed();
                let _ = ui.button("Other");
                let menu = super::Menu::new(ui, &field).open(&mut t.open);
                let _ = menu.show_ui(|ui| ui.label("Fonts"));
            },
            Typed { text: String::new(), open: false, picked: None },
        );
        h.run();
        h.get_by_role_and_label(egui::Role::TextInput, "Font").click();
        h.run();
        assert!(h.state().open);
        if mouse_over_menu {
            h.get_by_label("Fonts").hover();
            h.run();
            assert!(h.state().open, "hovering the menu keeps it open");
        }
        h.key_press(egui::Key::Tab);
        h.run();
        h.run();
        assert!(!h.state().open, "Tab to another widget closes the menu (mouse over menu {mouse_over_menu})");
    }
}

#[test]
fn a_click_on_the_canvas_closes_a_menu_under_a_text_field() {
    let mut h = font_field();
    h.run();
    h.get_by_role_and_label(egui::Role::TextInput, "Font").click();
    h.run();
    assert!(h.state().open);
    h.hover_at(egui::pos2(380.0, 280.0));
    h.run();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: egui::pos2(380.0, 280.0),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.run();
    }
    h.run();
    h.run();
    assert!(!h.state().open, "a click elsewhere closes the menu");
}

#[test]
fn a_click_opened_menu_under_a_text_field_leaves_it_the_focus_while_typing() {
    let mut h = Harness::new_ui_state(
        |ui, shown: &mut (String, bool)| {
            let name = ui.label("Font").id;
            let field = ui.text_edit_singleline(&mut shown.0).labelled_by(name);
            let text = shown.0.clone();
            let menu = super::Menu::new(ui, &field).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
            shown.1 = menu.show_ui(|ui| ui.label(format!("Fonts matching {text:?}"))).is_some();
        },
        (String::new(), false),
    );
    h.run();
    h.get_by_role_and_label(egui::Role::TextInput, "Font").click();
    h.run();
    h.get_by_role_and_label(egui::Role::TextInput, "Font").type_text("ar");
    h.run();
    assert_eq!(h.state().0, "ar");
    assert!(h.get_by_role_and_label(egui::Role::TextInput, "Font").is_focused(), "the field keeps the focus");
    assert!(h.state().1, "the menu stays open");
    assert!(h.query_by_label("Fonts matching \"ar\"").is_some(), "the menu follows the text");
}

#[test]
fn a_menu_with_a_footer_shrinks_when_its_list_empties_and_grows_back() {
    let mut h = Harness::builder().with_size(egui::vec2(400.0, 800.0)).build_ui_state(
        |ui, (filter, seen): &mut (String, Vec<f32>)| {
            ui.add_space(700.0);
            let row = ui.spacing().interact_size.y;
            let field = ui.button("Send");
            let _ = super::Menu::new(ui, &field)
                .height(260.0)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show_rows_with_header_footer(
                    |ui| {
                        ui.label("Title");
                        let name = ui.label("Search").id;
                        // A search field that takes the focus when the menu opens, as apps do.
                        let (id, pass) = (ui.scope_id().with("search"), ui.ctx().cumulative_pass_nr());
                        let opened = ui.data(|d| d.get_temp::<u64>(id)).is_none_or(|last| last + 1 < pass);
                        ui.data_mut(|d| d.insert_temp(id, pass));
                        let field = ui.text_edit_singleline(filter).labelled_by(name);
                        if opened {
                            field.request_focus();
                        }
                        if filter.is_empty() { 30 } else { 0 }
                    },
                    row,
                    |&n| n,
                    |ui, range, _| {
                        for n in range {
                            let _ = ui.selectable_label(false, format!("Bus {n}"));
                        }
                    },
                    |ui| {
                        let _ = ui.button("new bus...");
                        let remove = ui.button("Remove send");
                        // Where the footer is in each frame the user sees in full.
                        if ui.opacity() >= 1.0 {
                            seen.push(remove.rect.bottom());
                        }
                    },
                );
        },
        (String::new(), Vec::new()),
    );
    h.run();
    h.get_by_label("Send").click();
    h.run();
    // The menu opens above its field, so it shrinks from the top: the search field moves down.
    let footer = |h: &Harness<'_, (String, Vec<f32>)>| h.get_by_role(egui::Role::TextInput).rect();
    let full = footer(&h);
    // Opening shows the menu where it stays: no visible frame at a side or height it then leaves.
    let seen = &h.state().1;
    assert!(!seen.is_empty() && seen.iter().all(|b| (b - seen[0]).abs() < 0.5), "visible frames moved: {seen:?}");
    h.get_by_role(egui::Role::TextInput).type_text("zzzz");
    h.run();
    let none = footer(&h);
    assert_eq!(h.state().0, "zzzz");
    assert!(none.top() > full.top() + 100.0, "no matches shrink the menu: {none:?} vs {full:?}");
    for _ in 0..4 {
        h.key_press(egui::Key::Backspace);
        h.run();
    }
    assert!((footer(&h).top() - full.top()).abs() < 1.0, "cleared, the footer is back: {:?} vs {full:?}", footer(&h));
}

#[test]
fn a_footer_that_changes_height_every_pass_settles() {
    let mut h = Harness::builder().with_size(egui::vec2(400.0, 600.0)).build_ui_state(
        |ui, passes: &mut u32| {
            let field = ui.button("Send");
            let _ = super::Menu::new(ui, &field).show_ui_with_header_footer(
                |ui| ui.label("Title"),
                |ui, _| {
                    let _ = ui.label("Row");
                },
                |ui| {
                    *passes += 1;
                    ui.add_space(if (*passes).is_multiple_of(2) { 10.0 } else { 30.0 });
                },
            );
        },
        0,
    );
    h.run();
    h.get_by_label("Send").click();
    h.run();
    let before = *h.state();
    h.run();
    h.run();
    assert!(*h.state() - before <= 4, "the menu keeps remeasuring: {} passes in two frames", *h.state() - before);
}

#[test]
fn a_footer_that_changes_height_after_settling_still_lands() {
    // (footer height, bottom of the footer)
    let mut h = Harness::builder().with_size(egui::vec2(400.0, 600.0)).build_ui_state(
        |ui, s: &mut (f32, f32)| {
            let field = ui.button("Send");
            let _ = super::Menu::new(ui, &field).height(300.0).show_ui_with_header_footer(
                |ui| ui.label("Title"),
                |ui, _| {
                    for n in 0..50 {
                        let _ = ui.label(format!("Row {n}"));
                    }
                },
                |ui| {
                    ui.add_space(s.0);
                    s.1 = ui.min_rect().bottom();
                },
            );
        },
        (20.0, 0.0),
    );
    h.run();
    h.get_by_label("Send").click();
    h.run();
    let bottom = h.state().1;
    // The list fills the menu's height, so the footer's bottom stays put when the footer's height
    // changes and the list gives or takes the room. Changes are spaced over several passes: the
    // remeasure budget resets while the footer is stable.
    for (n, height) in [60.0, 20.0, 80.0, 30.0].into_iter().enumerate() {
        h.state_mut().0 = height;
        h.run();
        let now = h.state().1;
        assert!((now - bottom).abs() < 1.0, "change {n} to {height}: the footer ends at {now}, was {bottom}");
        h.run();
        h.run();
    }
}

#[test]
fn a_footer_that_hits_the_remeasure_cap_recovers_once_it_holds_still() {
    // (held footer height, footer passes so far, bottom of the footer)
    let mut h = Harness::builder().with_size(egui::vec2(400.0, 600.0)).build_ui_state(
        |ui, s: &mut (f32, u32, f32)| {
            let field = ui.button("Send");
            let _ = super::Menu::new(ui, &field).height(300.0).show_ui_with_header_footer(
                |ui| ui.label("Title"),
                |ui, _| {
                    for n in 0..50 {
                        let _ = ui.label(format!("Row {n}"));
                    }
                },
                |ui| {
                    s.1 += 1;
                    // Grows on each of the first passes, past the cap of two remeasures.
                    ui.add_space(if s.1 <= 5 { 20.0 + 15.0 * s.1 as f32 } else { s.0 });
                    s.2 = ui.min_rect().bottom();
                },
            );
        },
        (40.0, 0, 0.0),
    );
    h.run();
    h.get_by_label("Send").click();
    // Passes with a stable footer follow the growth: each one resets the remeasure budget.
    for _ in 0..3 {
        h.step();
    }
    let bottom = h.state().2;
    h.state_mut().0 = 90.0;
    h.run();
    let now = h.state().2;
    assert!((now - bottom).abs() < 1.0, "the footer ends at {now}, was {bottom}");
}
