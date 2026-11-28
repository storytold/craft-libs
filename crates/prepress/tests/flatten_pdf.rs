//! The PDF-in adapter on plate-test scenes: opaque items read and written back separate as the
//! source does.
#![cfg(feature = "flatten")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_pdf::write::backend::blend::Overprint as KOverprint;
use craft_pdf::write::backend::geom::PathBuilder;
use craft_pdf::write::backend::page::PageSettings;
use craft_pdf::write::backend::paint::{Fill, FillRule as KFillRule};
use craft_pdf::write::{Metadata, Writer};
use craft_plate_test::reference::{self, Context};
use craft_plate_test::scene;
use craft_prepress::flatten::kurbo::{BezPath, PathEl};
use craft_prepress::flatten::pdf::{ReadOptions, ReadReport, read_page};
use craft_prepress::flatten::{DisplayList, FillRule, GroupItem, Item, Paint};
use craft_prepress::{CmsConversion, Options, plates, to_pdf_color};

fn scene_pdf(id: &str) -> Vec<u8> {
    let scenes = scene::load_dir(&scene::scenes_dir()).unwrap();
    let s = scenes.iter().find(|s| s.id == id).unwrap();
    reference::write(s, &Context { scenes: &scenes, font: None }).unwrap().pdf
}

fn read(pdf: &[u8]) -> (DisplayList, ReadReport) {
    read_page(pdf, 0, &ReadOptions::default()).unwrap()
}

/// Every non-group item with the groups it is in, outermost first.
fn leaves(items: &[Item]) -> Vec<(Vec<&GroupItem>, &Item)> {
    fn walk<'a>(items: &'a [Item], path: &mut Vec<&'a GroupItem>, out: &mut Vec<(Vec<&'a GroupItem>, &'a Item)>) {
        for item in items {
            match item {
                Item::Group(g) => {
                    path.push(g);
                    walk(&g.children, path, out);
                    path.pop();
                }
                other => out.push((path.clone(), other)),
            }
        }
    }
    let mut out = Vec::new();
    walk(items, &mut Vec::new(), &mut out);
    out
}

fn write_fills(list: &DisplayList) -> Vec<u8> {
    let cms = CmsConversion::fogra39();
    let mut writer = Writer::new(Default::default(), Metadata::default());
    let size = list.page.size();
    let mut page = writer.document_mut().start_page_with(PageSettings::from_wh(size.width as f32, size.height as f32).unwrap());
    let mut surface = page.surface();
    for (groups, item) in leaves(&list.items) {
        assert!(groups.iter().all(|g| g.opacity == 1.0 && g.mask.is_none() && g.clip.is_none()), "opaque scenes only");
        let Item::Fill(f) = item else { panic!("{item:?}") };
        let Paint::Solid(color) = &f.paint else { panic!() };
        let o = f.overprint;
        surface.push_overprint(KOverprint { fill: o.fill, stroke: o.stroke, nonzero: o.nonzero });
        let rule = if f.rule == FillRule::EvenOdd { KFillRule::EvenOdd } else { KFillRule::NonZero };
        surface.set_fill(Some(Fill { paint: to_pdf_color(color, &list.inks, &cms).unwrap().into(), rule, ..Default::default() }));
        surface.draw_path(&krilla_path(&f.path));
        surface.pop();
    }
    surface.finish();
    page.finish();
    writer.finish().unwrap()
}

fn krilla_path(path: &BezPath) -> craft_pdf::write::backend::geom::Path {
    let mut b = PathBuilder::new();
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => b.move_to(p.x as f32, p.y as f32),
            PathEl::LineTo(p) => b.line_to(p.x as f32, p.y as f32),
            PathEl::QuadTo(p1, p) => b.quad_to(p1.x as f32, p1.y as f32, p.x as f32, p.y as f32),
            PathEl::CurveTo(p1, p2, p) => b.cubic_to(p1.x as f32, p1.y as f32, p2.x as f32, p2.y as f32, p.x as f32, p.y as f32),
            PathEl::ClosePath => b.close(),
        }
    }
    b.finish().unwrap()
}

#[test]
fn opaque_items_separate_as_the_source() {
    for id in ["S01", "S04", "D06", "S13"] {
        let source = scene_pdf(id);
        let (list, _) = read(&source);
        let copy = write_fills(&list);
        let (a, b) = (plates(&source, 0, 36.0, &Options::default()).unwrap(), plates(&copy, 0, 36.0, &Options::default()).unwrap());
        assert_eq!(a.planes.len(), b.planes.len(), "{id}: plates");
        for (k, (pa, pb)) in a.planes.iter().zip(&b.planes).enumerate() {
            let differ = pa.iter().zip(pb).filter(|(x, y)| x.abs_diff(**y) > 8).count();
            assert!(differ * 100 < pa.len(), "{id}: plate {k} differs in {differ} of {} pixels", pa.len());
        }
    }
}
