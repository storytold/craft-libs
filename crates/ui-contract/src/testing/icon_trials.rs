//! Trials of the app icon registry: the catalog wins a clashing name, the first set keeps a name two sets share.

use crate::primitives::{AppIcons, IconSet, Mirror, SvgIcon, icon_names, register_app_icons};
use crate::testing::{style, trial};
use craft_behavior::IconId;
use craft_i18n::Direction;
use craft_theme::{Contrast, Mode};
use egui::ColorImage;

// Built from parts so the workspace scan of icon-id literals does not read them.
const FIRST_NAME: &str = concat!("zz_t10_", "first");
const SECOND_NAME: &str = concat!("zz_t10_", "second");
const CATALOG_NAME: &str = "close";
const DISC: &[u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><circle cx=\"12\" cy=\"12\" r=\"8\" fill=\"black\"/></svg>";
const SQUARE: &[u8] =
    b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><rect x=\"4\" y=\"4\" width=\"16\" height=\"16\" fill=\"black\"/></svg>";

static FIRST_SET: AppIcons = AppIcons { name: "t10-first", stroke_width: None, icons: &[(FIRST_NAME, DISC), (CATALOG_NAME, DISC)] };
static SECOND_SET: AppIcons = AppIcons { name: "t10-second", stroke_width: None, icons: &[(FIRST_NAME, SQUARE), (SECOND_NAME, SQUARE)] };

fn icon(name: &'static str) -> Option<SvgIcon> {
    let tokens = style::tokens(Mode::Light, Contrast::Standard);
    SvgIcon::of(&tokens, Direction::Ltr, 24.0, &IconId::from(std::borrow::Cow::Borrowed(name)), IconSet::default(), Mirror::Auto)
}

fn mask(name: &'static str) -> Option<ColorImage> {
    icon(name).map(|icon| icon.mask(24.0, 1.0))
}

#[trial]
fn an_app_icon_set_resolves_after_the_catalog_and_the_first_set_keeps_a_shared_name() {
    assert_eq!(icon(FIRST_NAME), None);
    let catalog = icon_names().len();
    let close = mask(CATALOG_NAME);
    let catalog_icon = icon(CATALOG_NAME);

    register_app_icons(&FIRST_SET);
    assert!(icon(FIRST_NAME).is_some(), "the set's new name resolves");
    assert_eq!(icon(CATALOG_NAME), catalog_icon, "the catalog keeps its name");
    assert_eq!(mask(CATALOG_NAME), close, "the refused icon does not change the catalog icon");
    let first = mask(FIRST_NAME);
    assert_ne!(first, close);
    assert_eq!(icon_names().len(), catalog, "icon_names lists the built-in catalog only");
    assert!(!icon_names().contains(&FIRST_NAME));

    // A later set repeating a registered name is refused for that name only; its other names join.
    assert_eq!(icon(SECOND_NAME), None);
    register_app_icons(&SECOND_SET);
    assert!(icon(SECOND_NAME).is_some());
    assert_eq!(mask(FIRST_NAME), first, "the first set keeps the shared name");
    assert_ne!(mask(SECOND_NAME), first);
    let second = mask(SECOND_NAME);

    // Registering a set again changes nothing.
    register_app_icons(&FIRST_SET);
    register_app_icons(&SECOND_SET);
    assert_eq!((mask(FIRST_NAME), mask(SECOND_NAME), mask(CATALOG_NAME)), (first, second, close));
    assert_eq!(icon_names().len(), catalog);
}
