//! The flattener's writer: display lists written with krilla and separated again by `plates`.
//! Pages are 100 pt square and separated at 36 dpi; sample points are in points.
#![cfg(feature = "flatten")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::OnceLock;

use craft_pdf::write::backend::SerializeSettings;
use craft_pdf::write::print::{self, PrintColor};
use craft_prepress::flatten::kurbo::{Rect, Shape};
use craft_prepress::flatten::write::page_pdf;
use craft_prepress::flatten::{BlendMode, DisplayList, FillItem, FillRule, FlattenOptions, GroupItem, Item, Paint, flatten};
use craft_prepress::{CmsConversion, InkColor, InkId, InkSet, Options, Overprint, Plates, Process, plates};

const K: InkId = InkId::process(Process::Black);

fn fill(r: Rect, tints: &[(InkId, f32)], overprint: Overprint) -> Item {
    Item::Fill(FillItem { path: r.to_path(0.1), rule: FillRule::NonZero, paint: Paint::Solid(InkColor::Tints(tints.to_vec())), overprint })
}

fn solid(r: Rect, tints: &[(InkId, f32)]) -> Item {
    fill(r, tints, Overprint::default())
}

fn group(opacity: f32, children: Vec<Item>) -> Item {
    Item::Group(GroupItem {
        opacity,
        blend: BlendMode::Normal,
        isolated: false,
        knockout: false,
        clip: None,
        mask: None,
        actual_text: None,
        children,
    })
}

fn page(inks: InkSet, items: Vec<Item>) -> DisplayList {
    DisplayList { page: Rect::new(0.0, 0.0, 100.0, 100.0), inks, items }
}

fn pdf(list: &DisplayList) -> Vec<u8> {
    // Reading the profile takes a quarter second in a debug build.
    static SETTINGS: OnceLock<SerializeSettings> = OnceLock::new();
    let settings = SETTINGS.get_or_init(|| print::settings(PrintColor::fogra39(craft_cms::builtin::COATED_FOGRA39_ICC)).unwrap());
    page_pdf(list, settings.clone(), &CmsConversion::fogra39()).unwrap()
}

/// `list` flattened, with nothing left for the raster step, written.
fn flat_pdf(list: &DisplayList) -> Vec<u8> {
    let flat = flatten(list, &FlattenOptions::default()).unwrap();
    assert!(flat.complex.is_empty());
    pdf(&DisplayList { items: flat.items, ..list.clone() })
}

fn separate(pdf: &[u8]) -> Plates {
    plates(pdf, 0, 36.0, &Options::default()).unwrap()
}

/// Ink `name`'s tint at (`x`, `y`) in points, 0 when the page doesn't use the ink.
fn tint(p: &Plates, name: &str, x: u32, y: u32) -> u8 {
    p.inks.lookup(name).and_then(|id| p.tint(id, x / 2, y / 2)).unwrap_or(0)
}

/// Each ink's tint at (`x`, `y`) is within 3 of `expected`.
fn assert_inks(p: &Plates, x: u32, y: u32, expected: &[(&str, u8)]) {
    for (name, want) in expected {
        let got = tint(p, name, x, y);
        assert!(got.abs_diff(*want) <= 3, "{name} at ({x}, {y}): {got}, expected {want}");
    }
}

#[test]
fn group_opacity_applies_to_the_group_not_to_each_child() {
    // Two overlapping K100 fills in a 50% group: K50 where they overlap, not K75.
    let list = page(
        InkSet::new(),
        vec![group(0.5, vec![solid(Rect::new(0.0, 0.0, 60.0, 100.0), &[(K, 1.0)]), solid(Rect::new(40.0, 0.0, 100.0, 100.0), &[(K, 1.0)])])],
    );
    for written in [pdf(&list), flat_pdf(&list)] {
        let p = separate(&written);
        assert_inks(&p, 50, 50, &[("Black", 128)]);
        assert_inks(&p, 20, 50, &[("Black", 128)]);
    }
}
