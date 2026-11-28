//! `Menu` and `MenuItem`: the columns of an inline menu's rows.

pub(crate) mod bar;

pub(crate) mod closed;

pub(crate) mod context_capture;
pub(crate) mod context_capture_guards;
pub(crate) mod context_capture_nested;
pub(crate) mod context_capture_text_native;

pub(crate) mod companion;

pub(crate) mod icon_row;

pub(crate) mod native;

pub(crate) mod openers;

pub(crate) mod overflow;

pub(crate) mod pick_focus;

pub(crate) mod rows;

pub(crate) mod runs_per_row;
pub(crate) mod search;

pub(crate) mod submenus;

use craft_behavior::IconId;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::primitives::IconSpec;
use craft_ui::testing::{Fixture, trial};

/// Regression: an inline menu inside a content-sized container shows by the record its container's measuring run
/// left, so its own body runs only for real; the rows before the first iconed or checked row still take the columns,
/// in every frame (a tab strip's overflow menu checks only the shown tab).
#[trial]
fn rows_of_an_inline_menu_shown_from_its_container_record_take_the_columns_of_later_rows() {
    let mut fixture = Fixture::new(|ui| {
        Stack::new().show(ui, |ui, _| {
            Menu::inline("Tabs").show(ui, |ui, _| {
                MenuItem::new("Layers").show(ui);
                MenuItem::new("History").icon(IconSpec::svg(IconId::new("history"))).show(ui);
                MenuItem::new("Swatches").check(true).show(ui);
            });
        });
    });
    let at = |fixture: &Fixture, label: &str| fixture.texts().iter().find(|text| text.text.replace('&', "") == label).map(|text| text.rect.left());
    let mut frames = 0;
    while at(&fixture, "Swatches").is_none() && frames < 8 {
        fixture.frame(vec![]);
        frames += 1;
    }
    let x = |fixture: &Fixture, label: &str| at(fixture, label).expect("the row is shown");
    for frame in 0..3 {
        assert_eq!(x(&fixture, "Layers"), x(&fixture, "Swatches"), "frame {frame}: a plain row takes the mark and icon columns");
        assert_eq!(x(&fixture, "History"), x(&fixture, "Swatches"), "frame {frame}: an iconed row takes the mark column");
        fixture.frame(vec![]);
    }
}

/// A text-only row shows no icon and reserves no icon column, even with an icon set: its menu's labels start where a
/// menu with no icons at all starts them.
#[trial]
fn a_row_with_no_icon_takes_no_icon_column() {
    let redo_left = |with_icon: bool| {
        let mut fixture = Fixture::new(move |ui| {
            Stack::new().show(ui, |ui, _| {
                Menu::inline("Edit").show(ui, |ui, _| {
                    let undo = MenuItem::new("Undo");
                    if with_icon {
                        undo.icon(IconSpec::svg(IconId::new("history"))).no_icon(true).show(ui);
                    } else {
                        undo.show(ui);
                    }
                    MenuItem::new("Redo").show(ui);
                });
            });
        });
        fixture.settle();
        fixture.texts().iter().find(|text| text.text == "Redo").map(|text| text.rect.left() - fixture.shown().left())
    };
    assert_eq!(redo_left(true), redo_left(false), "the iconless row reserves no icon column");
}

/// A compact row is its label's line tall, not a control's height: the rows of a dense classic menu stand closer.
#[trial]
fn a_compact_row_is_shorter_than_a_control_row() {
    let pitch = |compact: bool| {
        let mut fixture = Fixture::new(move |ui| {
            Stack::new().show(ui, |ui, _| {
                Menu::inline("Edit").show(ui, |ui, _| {
                    MenuItem::new("Undo").compact(compact).show(ui);
                    MenuItem::new("Redo").compact(compact).show(ui);
                });
            });
        });
        fixture.settle();
        let top = |name: &str| fixture.texts().iter().find(|text| text.text == name).map(|text| text.rect.top());
        top("Redo").zip(top("Undo")).map(|(redo, undo)| redo - undo)
    };
    let (control, compact) = (pitch(false), pitch(true));
    assert!(compact < control, "compact rows stand closer: {compact:?} against {control:?}");
}
