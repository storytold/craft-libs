//! Printer's marks: layout from `Marks::new`, and `Marks::draw` rendered back through `plates`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_pdf::write::backend::SerializeSettings;
use craft_pdf::write::backend::geom::Size;
use craft_pdf::write::backend::page::PageSettings;
use craft_pdf::write::{Metadata, Writer};
use craft_prepress::marks::{BAR_PATCHES, Rect};
use craft_prepress::{InkId, MarkOptions, Marks, MarksError, Options, Process, plates};

/// A 200 × 100 pt trim at (100, 100).
const TRIM: Rect = Rect { x0: 100.0, y0: 100.0, x1: 300.0, y1: 200.0 };

fn only(f: impl FnOnce(&mut MarkOptions)) -> MarkOptions {
    let mut o = MarkOptions { crop: false, bleed: false, registration: false, color_bars: false, ..MarkOptions::default() };
    f(&mut o);
    o
}

fn inside(r: Rect, (x, y): (f64, f64)) -> bool {
    (r.x0..=r.x1).contains(&x) && (r.y0..=r.y1).contains(&y)
}

#[test]
fn the_sheet_holds_the_marks_and_clears_the_bleed() {
    let m = Marks::new(TRIM, TRIM, &MarkOptions::default()).unwrap();
    assert_eq!((m.gap, m.sheet), (6.0, Rect::new(76.0, 76.0, 324.0, 224.0)));

    let bleed = Rect::new(91.0, 100.0, 300.0, 200.0);
    let m = Marks::new(TRIM, bleed, &MarkOptions::default()).unwrap();
    assert_eq!((m.gap, m.sheet), (9.0, Rect::new(73.0, 73.0, 327.0, 227.0)), "the gap widens to clear a 9 pt bleed");
    for p in m.lines.iter().flatten() {
        assert!((inside(m.sheet, *p) && !inside(m.trim, *p)) || on_cross(&m, *p), "{p:?} sits in the slug");
    }
    for p in &m.patches {
        assert!(inside(m.sheet, (p.rect.x0, p.rect.y0)) && inside(m.sheet, (p.rect.x1, p.rect.y1)));
        assert!(p.rect.y1 <= m.bleed.y0 - m.gap + 1e-9, "patches stay clear of the bleed");
    }
}

/// Registration crosses lie in the slug too; this only lets their arms touch the sheet edge.
fn on_cross(m: &Marks, p: (f64, f64)) -> bool {
    m.circles.iter().any(|((x, y), r)| (p.0 - x).abs() <= r * 1.4 + 1e-9 && (p.1 - y).abs() <= r * 1.4 + 1e-9)
}

#[test]
fn crop_marks_run_outward_from_the_gap_on_the_trim_lines() {
    let m = Marks::new(TRIM, TRIM, &only(|o| o.crop = true)).unwrap();
    assert_eq!(m.lines.len(), 8);
    assert!(m.lines.contains(&[(94.0, 100.0), (76.0, 100.0)]), "top left, horizontal");
    assert!(m.lines.contains(&[(100.0, 94.0), (100.0, 76.0)]), "top left, vertical");
    assert!(m.lines.contains(&[(306.0, 200.0), (324.0, 200.0)]), "bottom right, horizontal");
    assert!(m.lines.contains(&[(300.0, 206.0), (300.0, 224.0)]), "bottom right, vertical");
    assert!(m.circles.is_empty() && m.patches.is_empty());
}

#[test]
fn bleed_marks_appear_only_when_the_bleed_passes_the_trim() {
    let opts = only(|o| o.bleed = true);
    assert!(Marks::new(TRIM, TRIM, &opts).unwrap().lines.is_empty());
    let m = Marks::new(TRIM, grown(TRIM, 9.0), &opts).unwrap();
    assert_eq!(m.lines.len(), 8);
    assert!(m.lines.contains(&[(91.0, 91.0), (82.0, 91.0)]), "half-length, on the bleed line: {:?}", m.lines);
    assert!(m.lines.contains(&[(309.0, 209.0), (309.0, 218.0)]));
}

fn grown(r: Rect, by: f64) -> Rect {
    Rect::new(r.x0 - by, r.y0 - by, r.x1 + by, r.y1 + by)
}

#[test]
fn a_registration_target_sits_mid_band_on_each_side() {
    let m = Marks::new(TRIM, TRIM, &only(|o| o.registration = true)).unwrap();
    let centres: Vec<_> = m.circles.iter().map(|(c, r)| (*c, *r)).collect();
    assert_eq!(centres, [((200.0, 85.0), 6.0), ((200.0, 215.0), 6.0), ((85.0, 150.0), 6.0), ((315.0, 150.0), 6.0)]);
    assert_eq!(m.lines.len(), 8, "a cross through each");
}

#[test]
fn colour_bars_fit_between_the_crop_marks_and_the_middle() {
    // Half the 200 pt width, less a mark length: 82 pt, so 7 patches of 10.8.
    assert_eq!(Marks::new(TRIM, TRIM, &only(|o| o.color_bars = true)).unwrap().patches.len(), 7);
    // Less the top target's arm too: 73.2 pt, so 6.
    let m = Marks::new(TRIM, TRIM, &only(|o| (o.color_bars, o.registration) = (true, true))).unwrap();
    assert_eq!(m.patches.len(), 6);
    let first = m.patches.first().unwrap();
    assert_eq!(first.cmyk, BAR_PATCHES[0]);
    assert!((first.rect.x0 - 118.0).abs() < 1e-9 && (first.rect.width() - 10.8).abs() < 1e-9);
    assert!(m.patches.iter().all(|p| p.rect.y0 >= m.sheet.y0 && p.rect.y1 <= TRIM.y0 - m.gap));
    assert!(m.patches.last().unwrap().rect.x1 < 200.0 - 6.0 * 1.4, "short of the top target");

    let narrow = Rect::new(0.0, 0.0, 40.0, 40.0);
    assert!(Marks::new(narrow, narrow, &only(|o| o.color_bars = true)).unwrap().patches.is_empty());
}

#[test]
fn the_info_line_sits_in_the_bottom_band() {
    let m = Marks::new(TRIM, TRIM, &MarkOptions::default()).unwrap();
    let (x, y) = m.info.origin;
    assert_eq!((x, m.info.size), (118.0, 7.0));
    assert!(y > TRIM.y1 + m.gap && y < m.sheet.y1);
    let ((cx, _), r) = m.circles[1];
    assert!(x + m.info.max_width <= cx - r * 1.4 + 1e-9, "the line stops short of the bottom target");
    let crop_only = Marks::new(TRIM, TRIM, &only(|o| o.crop = true)).unwrap();
    assert_eq!(crop_only.info.max_width, 82.0, "with no target, the line may reach the middle");
}

#[test]
fn bad_options_and_boxes_are_errors() {
    let e = |o: MarkOptions| Marks::new(TRIM, TRIM, &o).unwrap_err();
    assert_eq!(e(only(|_| {})), MarksError::NoMarks);
    for (o, what) in [
        (MarkOptions { offset: -1.0, ..MarkOptions::default() }, "offset"),
        (MarkOptions { length: 0.0, ..MarkOptions::default() }, "length"),
        (MarkOptions { length: f64::NAN, ..MarkOptions::default() }, "length"),
        (MarkOptions { line_width: 145.0, ..MarkOptions::default() }, "line width"),
        (MarkOptions { line_width: 6.5, ..MarkOptions::default() }, "line width"),
    ] {
        assert!(matches!(e(o), MarksError::Size { what: w, .. } if w == what), "{what}");
    }
    let opts = MarkOptions::default();
    assert_eq!(Marks::new(Rect::new(10.0, 0.0, 0.0, 10.0), TRIM, &opts).unwrap_err(), MarksError::Box("trim"));
    assert_eq!(Marks::new(TRIM, Rect::new(0.0, f64::INFINITY, 1.0, 1.0), &opts).unwrap_err(), MarksError::Box("bleed"));
    let huge = Rect::new(0.0, 0.0, 1e30, 10.0);
    assert_eq!(Marks::new(huge, huge, &opts).unwrap_err(), MarksError::Box("sheet"));
}

/// `m` drawn on a sheet-sized page, rendered to plates at 72 dpi (one pixel per point).
fn render(m: &Marks) -> craft_prepress::Plates {
    let mut writer = Writer::new(SerializeSettings::default(), Metadata::default());
    let mut page = writer.document_mut().start_page_with(PageSettings::new(Size::from_wh(m.sheet.width() as f32, m.sheet.height() as f32).unwrap()));
    let mut surface = page.surface();
    m.draw(&mut surface);
    surface.finish();
    page.finish();
    plates(&writer.finish().unwrap(), 0, 72.0, &Options::default()).unwrap()
}

#[test]
fn drawn_marks_print_on_every_plate_and_patches_on_their_own() {
    let m = Marks::new(TRIM, TRIM, &MarkOptions { line_width: 3.0, ..MarkOptions::default() }).unwrap();
    let p = render(&m);
    let px = |(x, y): (f64, f64)| ((x - m.sheet.x0) as u32, (y - m.sheet.y0) as u32);
    let ink = |id: Process, at: (f64, f64)| {
        let (x, y) = px(at);
        p.tint(InkId::process(id), x, y).unwrap()
    };
    let all = [Process::Cyan, Process::Magenta, Process::Yellow, Process::Black];

    let crop = (85.0, 100.0);
    assert!(all.iter().all(|&c| ink(c, crop) > 200), "a crop mark inks every plate");
    let target = (200.0, 85.0);
    assert!(all.iter().all(|&c| ink(c, target) > 200), "the registration cross inks every plate");
    let cyan = m.patches.first().unwrap().rect;
    let mid = ((cyan.x0 + cyan.x1) / 2.0, (cyan.y0 + cyan.y1) / 2.0);
    assert!(ink(Process::Cyan, mid) > 250, "the cyan patch inks cyan");
    assert!([Process::Magenta, Process::Yellow, Process::Black].iter().all(|&c| ink(c, mid) == 0), "and nothing else");
    assert!(all.iter().all(|&c| ink(c, (200.0, 150.0)) == 0), "the trim area is left alone");
}

/// A trim at the origin puts the sheet origin at negative coordinates; a bleed moves the marks out.
#[test]
fn drawn_marks_follow_the_sheet_origin_and_the_bleed() {
    let trim = Rect::new(0.0, 0.0, 200.0, 100.0);
    let m = Marks::new(trim, grown(trim, 9.0), &MarkOptions { line_width: 3.0, ..MarkOptions::default() }).unwrap();
    assert_eq!((m.sheet.x0, m.sheet.y0), (-27.0, -27.0));
    let p = render(&m);
    let tint = |id: Process, (x, y): (f64, f64)| p.tint(InkId::process(id), (x - m.sheet.x0) as u32, (y - m.sheet.y0) as u32).unwrap();
    assert!(tint(Process::Black, (-18.0, 0.0)) > 200, "the top-left crop mark, past the bleed");
    assert!(tint(Process::Black, (-13.0, -9.0)) > 200, "the top-left bleed mark");
    assert_eq!(tint(Process::Black, (-5.0, 0.0)), 0, "nothing inside the bleed");
}
