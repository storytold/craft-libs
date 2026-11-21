//! The bar under a right-to-left interface: the titles read from the right, a menu opens under
//! its title's reading-start edge, a row's atoms mirror (the shortcut at the physical left, the
//! checkmark column at the right), a submenu opens to the left of its row, the arrow keys follow
//! the reading direction, and a letterless label keeps its text and takes its English key as a
//! transient mark while the mnemonics show.

use egui::Key;

use super::*;
use crate::bar::keys::localized;

fn rtl() -> Bar {
    let mut b = wide();
    b.dir = craft_ui::direction::Direction::Rtl;
    b.run(3);
    b
}

impl Bar {
    fn painted(&self, text: &str) -> &Painted {
        self.texts
            .iter()
            .find(|p| p.text == text)
            .unwrap_or_else(|| panic!("no {text:?} painted: {:?}", self.texts.iter().map(|p| &p.text).collect::<Vec<_>>()))
    }
}

#[test]
fn the_bar_reads_right_to_left() {
    let ltr = wide();
    let rtl = rtl();
    assert!(ltr.rect("File").left() < ltr.rect("Help").left(), "LTR: File left of Help");
    assert!(rtl.rect("File").right() > rtl.rect("Help").right(), "RTL: File right of Help");
    // Same order, mirrored: File is the rightmost title, Edit just left of it.
    assert!(rtl.rect("File").left() >= rtl.rect("Edit").right() - 1.0, "RTL: {:?} left of {:?}", rtl.rect("File"), rtl.rect("Edit"));
    assert!(ltr.rect("File").right() <= ltr.rect("Edit").left() + 1.0, "LTR: {:?} left of {:?}", ltr.rect("File"), ltr.rect("Edit"));
}

#[test]
fn a_menu_opens_under_its_titles_reading_start_edge() {
    for (dir, at_right) in [(craft_ui::direction::Direction::Ltr, false), (craft_ui::direction::Direction::Rtl, true)] {
        let mut b = wide();
        b.dir = dir;
        b.run(2);
        b.click("File");
        assert!(b.shown("New"));
        let state = egui::AreaState::load(&b.ctx, b.nav().tops[0]).expect("the open menu's area state");
        let popup = state.rect();
        let title = b.rect("File");
        if at_right {
            assert!((popup.right() - title.right()).abs() < 2.0, "RTL: popup {popup:?} under the title's right edge {title:?}");
        } else {
            assert!((popup.left() - title.left()).abs() < 2.0, "LTR: popup {popup:?} under the title's left edge {title:?}");
        }
        assert!(popup.top() >= title.bottom() - 1.0, "below the bar: {popup:?} {title:?}");
    }
}

#[test]
fn a_rows_shortcut_sits_at_the_physical_left_in_rtl() {
    let shortcut = chord("Cmd+N").display(craft_keymap::Platform::Linux);
    for (dir, shortcut_on_left) in [(craft_ui::direction::Direction::Ltr, false), (craft_ui::direction::Direction::Rtl, true)] {
        let mut b = wide();
        b.dir = dir;
        b.run(2);
        b.click("File");
        b.run(2);
        let label = b.painted("New").rect();
        let sc = b.painted(&shortcut).rect();
        assert_eq!(sc.left() < label.left(), shortcut_on_left, "{dir:?}: shortcut {sc:?} vs label {label:?}");
    }
}

#[test]
fn the_checkmark_column_sits_at_the_reading_start() {
    for (dir, mark_on_right) in [(craft_ui::direction::Direction::Ltr, false), (craft_ui::direction::Direction::Rtl, true)] {
        let mut b = wide();
        b.dir = dir;
        b.run(2);
        b.click("View");
        b.run(2);
        // Grid is checked: its galley is "✓Grid"; Rulers is not, but keeps the column's width.
        let grid = b.painted("✓Grid");
        let rulers = b.painted("Rulers");
        let mark = grid.first_char_rect();
        let at_right = mark.center().x > grid.rect().center().x;
        assert_eq!(at_right, mark_on_right, "{dir:?}: the ✓ of {grid:?} at x {mark:?}");
        // The column keeps the labels aligned: checked and unchecked rows' labels start together.
        if mark_on_right {
            assert!((grid.rect().right() - rulers.rect().right()).abs() < 1.0, "{dir:?}: {grid:?} {rulers:?}");
        } else {
            assert!((grid.rect().left() - rulers.rect().left()).abs() < 1.0, "{dir:?}: {grid:?} {rulers:?}");
        }
    }
}

#[test]
fn a_submenu_opens_to_the_left_of_its_row_in_rtl() {
    for (dir, on_left) in [(craft_ui::direction::Direction::Ltr, false), (craft_ui::direction::Direction::Rtl, true)] {
        let mut b = wide();
        b.dir = dir;
        b.run(2);
        b.click("File");
        b.hover("Open Recent");
        b.run(3);
        assert!(b.shown("a.txt"), "{dir:?}: the submenu is open");
        let nav = b.nav();
        let row = nav.rows[0].iter().find(|r| r.rect.contains(b.rect("Open Recent").center())).expect("the Open Recent row").clone();
        let sub =
            egui::AreaState::load(&b.ctx, egui::containers::menu::SubMenu::id_from_widget_id(row.widget)).expect("the submenu's area state").rect();
        if on_left {
            assert!(sub.right() <= row.rect.left() + 1.0, "RTL: submenu {sub:?} left of its row {row:?}");
        } else {
            assert!(sub.left() >= row.rect.right() - 1.0, "LTR: submenu {sub:?} right of its row {row:?}");
        }
    }
}

#[test]
fn the_arrow_keys_follow_the_reading_direction() {
    let mut b = rtl();
    b.alt_tap();
    assert_eq!(b.focus(), Some(0));
    b.key(Key::ArrowLeft);
    assert_eq!(b.focus(), Some(1), "RTL: ← is forward, to the next title in reading order");
    b.key(Key::ArrowRight);
    assert_eq!(b.focus(), Some(0), "RTL: → is back");
    // With a menu open, ← opens the next menu (or a submenu), → the previous one.
    b.click("File");
    b.key(Key::ArrowLeft);
    assert!(b.shown("Undo") && !b.shown("New"), "RTL: ← from File opens Edit");
    b.key(Key::ArrowRight);
    assert!(b.shown("New"), "RTL: → back to File");
    // ← on a submenu row opens it (into the submenu); → there closes it again.
    b.key(Key::ArrowDown);
    b.key(Key::ArrowDown);
    b.key(Key::ArrowDown);
    assert_eq!(b.nav().highlighted(0), Some(2), "on Open Recent");
    b.key(Key::ArrowLeft);
    assert_eq!(b.nav().level, 1, "RTL: ← opened the submenu");
    assert!(b.shown("a.txt"));
    b.key(Key::ArrowRight);
    assert_eq!(b.nav().level, 0, "RTL: → closed it");

    // LTR keeps the stock meaning: → forward.
    let mut b = wide();
    b.alt_tap();
    b.key(Key::ArrowRight);
    assert_eq!(b.focus(), Some(1), "LTR: → is forward");
}

#[test]
fn right_to_left_labels_keep_their_text_and_take_the_english_key() {
    let en = |v: &[&str]| v.iter().map(|s| Some((*s).to_owned())).collect::<Vec<_>>();
    // Arabic: the label renders unchanged right to left; the English key stays the mnemonic.
    let (shown, keys) = localized(&["ملف", "تحرير"], &en(&["File", "Edit"]), &[], true);
    assert_eq!(shown, ["ملف", "تحرير"]);
    assert_eq!(keys, [Some('F'), Some('E')]);
    // Left to right the same labels get the "(F)" suffix, as before.
    let (shown, keys) = localized(&["ملف", "تحرير"], &en(&["File", "Edit"]), &[], false);
    assert_eq!(shown, ["ملف(F)", "تحرير(E)"]);
    assert_eq!(keys, [Some('F'), Some('E')]);
    // Chinese takes the same keys under both directions; only the suffix differs.
    let (shown_rtl, keys_rtl) = localized(&["文件", "编辑"], &en(&["File", "Edit"]), &[], true);
    let (shown_ltr, keys_ltr) = localized(&["文件", "编辑"], &en(&["File", "Edit"]), &[], false);
    assert_eq!(keys_rtl, keys_ltr);
    assert_eq!(shown_rtl, ["文件", "编辑"]);
    assert_eq!(shown_ltr, ["文件(F)", "编辑(E)"]);
}

#[test]
fn a_letterless_rtl_title_shows_its_key_as_a_transient_mark_while_alt_is_held() {
    let mut b = rtl();
    b.english = Some(|l: Labeled<'_>| match l {
        Labeled::Title("ملف") => Some("File".to_owned()),
        _ => None,
    });
    b.bar.menus[0].title = "ملف".to_owned();
    b.run(2);
    assert!(b.texts.iter().any(|p| p.text == "ملف"), "the title paints unchanged: {:?}", b.texts.iter().map(|p| &p.text).collect::<Vec<_>>());
    assert!(!b.texts.iter().any(|p| p.text.contains("(F")), "no mark without Alt");
    b.alt(true);
    assert!(
        b.texts.iter().any(|p| p.text == "ملف (F)"),
        "the English key shows while Alt is held: {:?}",
        b.texts.iter().map(|p| &p.text).collect::<Vec<_>>()
    );
    b.alt(false);
    // The tap focused the bar, so the mark stays; Esc leaves the bar and the mark goes.
    assert_eq!(b.focus(), Some(0));
    b.key(Key::Escape);
    b.run(2);
    assert!(b.texts.iter().any(|p| p.text == "ملف") && !b.texts.iter().any(|p| p.text.contains("(F")));
    // And the key still opens the menu.
    b.alt_key(Key::F);
    assert!(b.shown("New"), "Alt+F opens ملف");
}
