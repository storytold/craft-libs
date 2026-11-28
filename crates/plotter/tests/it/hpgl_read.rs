//! HP-GL import, ported from VectorCraft's `crates/cad/src/hpgl/tests.rs` (MIT OR Apache-2.0) to the
//! reader's shapes: plotter units, y up, no document. Pens, runs, curves, filled and edged areas,
//! line types, scaling and rotation, encoded polylines, escapes, warnings, detection and hostile
//! files; and the pen and RO each shape keeps.

use craft_plotter::hpgl::plot::{FillRule, LineCap, LineJoin};
use craft_plotter::hpgl::{Drawing, ReadError, is_hpgl, read::read};
use kurbo::{BezPath, PathEl, Point, Shape as _};

const MM: f64 = 40.0;

fn plot(s: &str) -> Drawing {
    plot_bytes(s.as_bytes())
}

fn plot_bytes(b: &[u8]) -> Drawing {
    read(b).unwrap_or_else(|e| panic!("{e}: {:?}", String::from_utf8_lossy(b)))
}

fn ends(bp: &BezPath) -> Vec<Point> {
    bp.elements().iter().filter_map(|e| e.end_point()).collect()
}

fn near(a: Point, b: Point) -> bool {
    (a - b).hypot() < 1e-6
}

fn assert_points(got: &[Point], want: &[(f64, f64)]) {
    assert_eq!(got.len(), want.len(), "{got:?}");
    for (g, w) in got.iter().zip(want) {
        assert!(near(*g, Point::new(w.0, w.1)), "{got:?} != {want:?}");
    }
}

/// A PE number as the reference guide's encoding writes it (base 64, or base 32 for 7-bit data).
fn pe_number(v: i64, seven_bit: bool) -> Vec<u8> {
    let mut n = if v >= 0 { 2 * v as u64 } else { 2 * v.unsigned_abs() + 1 };
    let (base, high) = if seven_bit { (32, 95) } else { (64, 191) };
    let mut out = vec![];
    while n >= base {
        out.push(63 + (n % base) as u8);
        n /= base;
    }
    out.push(high + n as u8);
    out
}

#[test]
fn moves_are_plotter_units_with_y_up() {
    let d = plot("IN;SP1;PW0;PU0,0;PD1016,0,1016,1016;PU;");
    assert_eq!(d.shapes.len(), 1);
    assert_points(&ends(&d.shapes[0].path), &[(0.0, 0.0), (1016.0, 0.0), (1016.0, 1016.0)]);
    let s = d.shapes[0].stroke.as_ref().expect("stroked");
    assert_eq!((s.color, s.cap, s.join, s.miter), ([0, 0, 0], LineCap::Butt, LineJoin::Miter, 5.0));
    assert!(d.shapes[0].fill.is_none() && d.warnings.is_empty(), "{:?}", d.warnings);
}

#[test]
fn pens_pick_colors_widths_and_are_kept() {
    // Before any SP the pen is pen 1; SP0 draws nothing; pens past the palette wrap round it.
    let d = plot("IN;PD0,0,100,0;PU;SP2;PD0,100,100,100;PU;SP0;PD0,200,100,200;PU;SP9;PD0,300,100,300;PU;SP3;PD0,400,100,400;PU;");
    let colors: Vec<[u8; 3]> = d.shapes.iter().map(|s| s.stroke.as_ref().expect("stroked").color).collect();
    assert_eq!(colors, [[0, 0, 0], [255, 0, 0], [255, 0, 0], [0, 255, 0]]);
    let pens: Vec<i64> = d.shapes.iter().map(|s| s.pen).collect();
    assert_eq!(pens, [1, 2, 9, 3], "each shape keeps the pen that drew it");

    // PC gives a pen a color in the color range; PC with the pen alone gives back its default.
    let d = plot("IN;CR0,1,0,1,0,1;PC3,0,0,1;SP3;PD0,0,10,0;PU;PC3;PD0,10,10,10;PU;");
    let colors: Vec<[u8; 3]> = d.shapes.iter().map(|s| s.stroke.as_ref().expect("stroked").color).collect();
    assert_eq!(colors, [[0, 0, 255], [0, 255, 0]]);

    // PW in millimeters for every pen, or for one; IN gives back 0.35 mm.
    let d = plot("IN;SP1;PW1;PD0,0,10,0;PU;PW2,2;SP2;PD0,10,10,10;PU;SP1;PD0,20,10,20;PU;IN;SP1;PD0,30,10,30;");
    let widths: Vec<f64> = d.shapes.iter().map(|s| s.stroke.as_ref().expect("stroked").width).collect();
    let want = [MM, 2.0 * MM, MM, 0.35 * MM];
    assert!(widths.len() == want.len() && widths.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-9), "{widths:?}");
}

#[test]
fn each_run_with_the_pen_down_is_one_path() {
    // Relative moves; a run that comes back to its start is closed; a pen-up move elsewhere starts
    // another path, and one back to where the line ended continues it.
    let d = plot("IN;SP1;PW0;PA0,0;PR;PD1016,0,0,1016,-1016,0,0,-1016;PU2032,0;PD0,1016;PU0,1016;PU0,-1016;PD508,0;PU0,500;PD0,500;");
    assert_eq!(d.shapes.len(), 3, "{:?}", d.shapes);
    assert!(matches!(d.shapes[0].path.elements().last(), Some(PathEl::ClosePath)));
    assert_eq!(ends(&d.shapes[1].path).len(), 3, "the run went on from where it ended");
    assert_eq!(ends(&d.shapes[2].path).len(), 2);

    // A pen-down move of no length draws a dot one plotter unit long.
    let d = plot("IN;SP1;PA500,500;PD;PA500,500;PU;");
    assert_eq!(d.shapes.len(), 1);
    assert!((d.shapes[0].path.bounding_box().width() - 1.0).abs() < 1e-9);
}

#[test]
fn circles_arcs_and_curves_are_drawn_as_curves() {
    // A circle of an inch round the pen, which stays at its center (the next line starts there).
    let d = plot("IN;SP1;PW0;PA2032,2032;CI1016;PD3048,2032;");
    assert_eq!(d.shapes.len(), 2);
    let c = &d.shapes[0].path;
    assert!(c.elements().iter().any(|e| matches!(e, PathEl::CurveTo(..))) && matches!(c.elements().last(), Some(PathEl::ClosePath)));
    let b = c.bounding_box();
    assert!((b.width() - 2032.0).abs() < 1e-2 && (b.height() - 2032.0).abs() < 1e-2, "{b:?}");
    assert!((ends(&d.shapes[1].path)[0] - b.center()).hypot() < 1e-2);

    // AA: a quarter turn counterclockwise round (0, 0) from (1016, 0) ends at (0, 1016), and the
    // pen goes on from there. AR takes its center relative to the pen.
    for s in ["IN;SP1;PW0;PA1016,0;PD;AA0,0,90;PA0,2032;", "IN;SP1;PW0;PA1016,0;PD;AR-1016,0,90;PA0,2032;"] {
        let d = plot(s);
        assert_eq!(d.shapes.len(), 1, "{s}");
        let pts = ends(&d.shapes[0].path);
        let (start, end) = (pts[0], pts[pts.len() - 2]);
        assert!(near(start, Point::new(1016.0, 0.0)) && near(end, Point::new(0.0, 1016.0)), "{s}: {pts:?}");
        assert!(near(*pts.last().expect("a point"), Point::new(0.0, 2032.0)), "{s}: {pts:?}");
    }

    // AT: the arc from the pen through a point to an end; points in a line give a line.
    let d = plot("IN;SP1;PW0;PA1016,0;PD;AT0,1016,-1016,0;");
    let b = d.shapes[0].path.bounding_box();
    assert!((b.width() - 2032.0).abs() < 1e-6 && (b.height() - 1016.0).abs() < 1e-6, "a half circle: {b:?}");
    assert_eq!(ends(&plot("IN;SP1;PW0;PA0,0;PD;AT508,0,1016,0;").shapes[0].path).len(), 2);

    // BZ and BR: cubic Béziers, BR's points relative to each curve's start.
    for s in [
        "IN;SP1;PW0;PA0,0;PD;BZ0,1016,1016,1016,1016,0,1016,-1016,2032,-1016,2032,0;",
        "IN;SP1;PW0;PA0,0;PD;BR0,1016,1016,1016,1016,0,0,-1016,1016,-1016,1016,0;",
    ] {
        let els = plot(s).shapes[0].path.elements().to_vec();
        assert_eq!(els.iter().filter(|e| matches!(e, PathEl::CurveTo(..))).count(), 2, "{s}");
        assert!(near(els.last().and_then(PathEl::end_point).expect("an end"), Point::new(2032.0, 0.0)), "{s}: {els:?}");
    }
}

#[test]
fn rectangles_wedges_and_polygons_fill_and_edge() {
    // RA fills, EA edges; neither moves the pen.
    let d = plot("IN;SP2;PW0;PA0,0;RA1016,1016;EA1016,1016;PD508,0;");
    assert_eq!(d.shapes.len(), 3);
    assert_eq!(d.shapes[0].fill, Some(([255, 0, 0], FillRule::EvenOdd)));
    assert!(d.shapes[0].stroke.is_none() && d.shapes[1].fill.is_none());
    assert!(near(ends(&d.shapes[2].path)[0], Point::new(0.0, 0.0)), "the line starts where the pen was");

    // WG fills a wedge, EW edges one; a whole turn is a circle.
    let d = plot("IN;SP1;PA0,0;WG1016,0,90;EW1016,0,360;");
    assert!(d.shapes[0].fill.is_some() && ends(&d.shapes[0].path).len() >= 3);
    assert!((d.shapes[1].path.bounding_box().width() - 2032.0).abs() < 1e-6);

    // Polygon mode: two squares, one inside the other; FP fills even-odd by default, FP1
    // non-zero; EP edges only what was drawn with the pen down.
    let poly = "IN;SP1;PW0;PA0,0;PM0;PD3048,0,3048,3048,0,3048,0,0;PM1;PU1016,1016;PD2032,1016;PU2032,2032;PD1016,2032,1016,1016;PM2;";
    let d = plot(&format!("{poly}FP;FP1;EP;"));
    assert_eq!(d.shapes.len(), 3);
    let rule = |i: usize| d.shapes[i].fill.map(|(_, r)| r);
    assert_eq!((rule(0), rule(1)), (Some(FillRule::EvenOdd), Some(FillRule::NonZero)));
    assert_eq!(d.shapes[0].path.elements().iter().filter(|e| matches!(e, PathEl::ClosePath)).count(), 2);
    let moves = d.shapes[2].path.elements().iter().filter(|e| matches!(e, PathEl::MoveTo(..))).count();
    assert_eq!(moves, 3, "the pen-up side isn't edged");
    assert!(plot(poly).shapes.is_empty(), "nothing is drawn in polygon mode itself");

    // Fill types: shading mixes the pen with white; hatching fills solid with a warning.
    let d = plot("IN;SP5;PA0,0;FT10,50;RA100,100;FT3,10,45;RA200,200;");
    assert_eq!((d.shapes[0].fill.map(|f| f.0), d.shapes[1].fill.map(|f| f.0)), (Some([128, 128, 255]), Some([0, 0, 255])));
    assert_eq!(d.warnings, ["hatched and patterned fills are filled solid"]);
}

#[test]
fn line_types_become_dash_patterns() {
    let dash = |s: &str| plot(s).shapes[0].stroke.as_ref().and_then(|s| s.dash.clone());
    // A 10 mm pattern of line type 2 is 5 mm on, 5 mm off.
    let d = dash("IN;SP1;LT2,10,1;PD0,0,4000,0;").expect("dashed");
    assert!(d.pattern.iter().all(|v| (v - 5.0 * MM).abs() < 1e-9) && d.pattern.len() == 2 && !d.adaptive, "{d:?}");
    // The default length is 4% of the P1-P2 diagonal; IP sets P1 and P2.
    let d = dash("IN;IP0,0,3000,4000;SP1;LT3;PD0,0,4000,0;").expect("dashed");
    assert!((d.pattern[0] - 140.0).abs() < 1e-9 && (d.pattern[1] - 60.0).abs() < 1e-9, "{d:?}");
    // Adaptive types stretch to each line; a zero length is a dot one plotter unit long.
    let d = dash("IN;SP1;LT-4,10,1;PD0,0,4000,0;").expect("dashed");
    assert!(d.adaptive && (d.pattern[2] - 1.0).abs() < 1e-12, "{d:?}");
    // UL redefines a type; LT; draws solid and LT99 gives the type back.
    let d = plot("IN;SP1;UL2,1,3;LT2,10,1;PD0,0,4000,0;PU;LT;PD0,100,4000,100;PU;LT99;PD0,200,4000,200;");
    let dashes: Vec<bool> = d.shapes.iter().map(|s| s.stroke.as_ref().is_some_and(|s| s.dash.is_some())).collect();
    assert_eq!(dashes, [true, false, true]);
    let p = &d.shapes[0].stroke.as_ref().and_then(|s| s.dash.as_ref()).expect("dashed").pattern;
    assert!((p[0] - 2.5 * MM).abs() < 1e-9 && (p[1] - 7.5 * MM).abs() < 1e-9, "{p:?}");
    // Line type 0 draws a dot at each point.
    assert_eq!(plot("IN;SP1;LT0;PD0,0,1000,0,2000,0;").shapes.len(), 3);
    // LA sets the caps, joins and miter limit.
    let d = plot("IN;SP1;LA1,4,2,4,3,9;PD0,0,100,0;");
    let s = d.shapes[0].stroke.as_ref().expect("stroked");
    assert_eq!((s.cap, s.join, s.miter), (LineCap::Round, LineJoin::Round, 9.0));
    // Each LA value keeps its own kind.
    let d = plot("IN;SP1;LA1,3,2,6;PD0,0,100,0;");
    let s = d.shapes[0].stroke.as_ref().expect("stroked");
    assert_eq!((s.cap, s.join), (LineCap::Triangular, LineJoin::None));
    let d = plot("IN;SP1;LA2,2;PD0,0,100,0;PU;LA2,3;PD0,10,100,10;PU;LA2,5;PD0,20,100,20;");
    let joins: Vec<LineJoin> = d.shapes.iter().map(|s| s.stroke.as_ref().expect("stroked").join).collect();
    assert_eq!(joins, [LineJoin::MiterBevel, LineJoin::Triangular, LineJoin::Bevel]);
}

#[test]
fn scaling_and_rotation_map_user_units() {
    let size = |s: &str| plot(s).shapes[0].path.bounding_box().size();
    // Anisotropic: 100 user units across an inch.
    let s = size("IN;IP0,0,1016,2032;SC0,100,0,100;SP1;PD0,0,100,100;");
    assert!((s.width - 1016.0).abs() < 1e-9 && (s.height - 2032.0).abs() < 1e-9, "{s:?}");
    // Isotropic: the smaller of the two scales on both axes.
    let s = size("IN;IP0,0,1016,2032;SC0,100,0,100,1;SP1;PU0,0;PD100,100;");
    assert!((s.width - 1016.0).abs() < 1e-9 && (s.height - 1016.0).abs() < 1e-9, "{s:?}");
    // Point factor: 10 plotter units per user unit.
    let s = size("IN;SC0,10,0,10,2;SP1;PD0,0,101.6,50.8;");
    assert!((s.width - 1016.0).abs() < 1e-9 && (s.height - 508.0).abs() < 1e-9, "{s:?}");
    // Without IP, P2 lies 11 × 8.5 in from the origin.
    let s = size("IN;SC0,11,0,8.5;SP1;PD0,0,1,1;");
    assert!((s.width - 1016.0).abs() < 1e-9 && (s.height - 1016.0).abs() < 1e-9, "{s:?}");

    // RO turns the coordinate system: x runs up, and the shape keeps the angle.
    let d = plot("IN;RO90;SP1;PW0;PA0,0;PD1016,0;");
    let b = d.shapes[0].path.bounding_box();
    assert!(b.width() < 1e-9 && (b.height() - 1016.0).abs() < 1e-9, "{b:?}");
    assert_eq!(d.shapes[0].ro, 90);
    // The pen stays where it was on the page: a line after RO starts where the one before ended.
    let d = plot("IN;SP1;PW0;PD0,0,1016,0;PU;RO90;PD;PR1016,0;");
    let all = d.shapes.iter().map(|s| s.path.bounding_box()).reduce(|a, b| a.union(b)).expect("bounds");
    assert!((all.width() - 1016.0).abs() < 1e-9 && (all.height() - 1016.0).abs() < 1e-9, "{all:?}");
    assert_eq!(d.shapes.iter().map(|s| s.ro).collect::<Vec<_>>(), [0, 90]);
}

#[test]
fn polyline_encoded_data_is_read() {
    // Pen 2, a pen-up move to the absolute (1016, 0), then a relative line an inch up.
    let mut f = b"IN;SP1;PW0;PE:".to_vec();
    f.extend(pe_number(2, false));
    f.extend(b"<=");
    for v in [1016, 0, 0, 1016] {
        f.extend(pe_number(v, false));
    }
    f.push(b';');
    let d = plot_bytes(&f);
    assert_eq!(d.shapes.len(), 1);
    assert_eq!((d.shapes[0].stroke.as_ref().map(|s| s.color), d.shapes[0].pen), (Some([255, 0, 0]), 2));
    assert_points(&ends(&d.shapes[0].path), &[(1016.0, 0.0), (1016.0, 1016.0)]);

    // 7-bit data with two fractional bits: 4064 / 4 = 1016.
    let mut f = b"IN;SP1;PW0;PE7>".to_vec();
    f.extend(pe_number(2, true));
    f.extend(pe_number(-4064, true));
    f.extend(pe_number(0, true));
    f.push(b';');
    assert!((plot_bytes(&f).shapes[0].path.bounding_box().width() - 1016.0).abs() < 1e-9);

    // Data that can't be decoded ends the instruction at its semicolon.
    let d = plot("IN;SP1;PE#PD5000,5000;PD1016,0;");
    assert_eq!(d.shapes.len(), 1);
    assert!((d.shapes[0].path.bounding_box().width() - 1016.0).abs() < 1e-9);
}

#[test]
fn printer_escapes_job_lines_and_labels_are_skipped_with_warnings() {
    let mut f = b"\x1b%-12345X@PJL JOB\r\n@PJL ENTER LANGUAGE=PCL\r\n\x1bE\x1b%0BIN;SP1;PW0;PD0,0,1016,0;PU;\x1b%0A".to_vec();
    // An HP RTL raster image whose data spells out instructions.
    f.extend(b"\x1b*r1A\x1b*b11WPD9999,9999\x1b*rC");
    f.extend(b"\x1b%1BPD1016,1016;\x1b%-12345X@PJL EOJ\r\n");
    assert!(is_hpgl(&f));
    let d = plot_bytes(&f);
    assert_eq!(d.shapes.len(), 1, "the pen came down where the line ended, so it goes on");
    assert_eq!(ends(&d.shapes[0].path).len(), 3);
    assert_eq!(d.warnings, ["raster images in the file were left out"]);

    // A label's text, up to its terminator, isn't read as instructions; unknown ones are counted.
    let d = plot("IN;SP1;LBPD9999,9999\x03DT*,1;LBPU0,0;PD5000,5000*CO\"PD8000,8000\";PD1016,0;PU;ZZ1,2;QQ;");
    assert_eq!(d.shapes.len(), 1);
    assert_eq!(d.warnings, ["labels were left out", "2 instructions of kinds the HP-GL reader doesn't support were skipped"]);
    // Symbol marks, later pages and the input window are warned about.
    let d = plot("IN;SP1;SM*;PD100,0;SM;");
    assert_eq!((d.warnings, d.shapes.len()), (vec!["symbols drawn at plotted points (symbol mode) were left out".to_string()], 1));
    let d = plot("IN;IW0,0,10,10;SP1;PD100,0;PU;PG;PD0,100;PG;");
    assert_eq!(d.warnings, ["the input window doesn't clip the art", "the art of later pages is drawn over the first page"]);
}

#[test]
fn only_plot_files_are_taken_for_hpgl() {
    for f in [
        &b"IN;SP1;PU0,0;PD100,100;"[..],
        b"in;sp1;pu0,0;pd100,100;",
        b";IN;\r\nSP1;\r\nPA100,100;",
        b"SP1PD0,0,10,10PU",
        b"CO\"made by hand\";IN;",
        b"\x1b%-12345X@PJL ENTER LANGUAGE=HPGL2\nIN;SP1;",
        // IN gives back ETX as the label terminator: the label doesn't end at the comment's `*`.
        b"DT*;IN;SP1;LBx\x03PD10,0;CO\"*\";",
    ] {
        assert!(is_hpgl(f), "{:?}", String::from_utf8_lossy(f));
    }
    for f in [
        &b""[..],
        b"IN THE BEGINNING",
        b"set terminal png\nplot sin(x)\n",
        b"SPDX-License-Identifier: MIT",
        b"\x89PNG\r\n\x1a\n",
        b"IN;SP1;PD0,0;hello world",
        b"\x1b%-12345X@PJL ENTER LANGUAGE=PCL\n\x1bE\x1b*r1A",
        // Only CO takes a quoted string.
        b"IN\"not a plot\";",
    ] {
        assert!(!is_hpgl(f), "{:?}", String::from_utf8_lossy(f));
        assert_eq!(read(f).err(), Some(ReadError::NotHpgl), "{:?}", String::from_utf8_lossy(f));
    }
    // A file picked by its extension may start with any instruction the reader knows.
    for f in [&b"SI0.2,0.3;SP1;PA0,0;PD1016,0;"[..], b"DI1,0;SP1;PA0,0;PD1016,0;", b"GC5;SP1;PA0,0;PD1016,0;"] {
        assert!(!is_hpgl(f), "{:?}", String::from_utf8_lossy(f));
        assert_eq!(plot_bytes(f).shapes.len(), 1, "{:?}", String::from_utf8_lossy(f));
    }
}

#[test]
fn hostile_files_read_without_panics() {
    let mut cases: Vec<Vec<u8>> = vec![
        b"IN;SP1;PD99999999999999999999999999999,0;PU0,0;PD1016,1016;".to_vec(),
        b"IN;SP1;PD0.000000000000000000000000001,1e999,--,++,..;".to_vec(),
        b"IN;SP1;LBno end".to_vec(),
        b"IN;CO\"no end".to_vec(),
        b"IN;SP1;PE<=\xff\xff\xff\xfe\xfe".to_vec(),
        b"IN;SP1;PD;AA0,0,32767;AR0,0,90;AT0,0,0,0;RT1,1,2,2;CI0;CI-5;WG0,0,0;EW5,99999,-99999;WG-5,0,360;CI99999999999999;".to_vec(),
        b"IN;IP5,5,5,5;SC0,1,0,1,1;SP1;PD1,1;SC0,0,0,0,2;RO45;NP99999999;SP99999999999;PC99999999,1,2,3;PW99999,99999;PD2,2;".to_vec(),
        b"IN;SP1;FP;EP;PM1;PM2;PM0;PD10,10;PM1;PM1;PM1;CI5;PE<\xc0\xc0;".to_vec(),
        b"\x1b%-12345X@PJL".to_vec(),
        b"IN;SP1;PD0,0;\x1b*b99999999999W".to_vec(),
        b"IN;\x1b".to_vec(),
    ];
    cases.push([&b"IN;SP1;PD"[..], &(0..=255u8).collect::<Vec<_>>()].concat());
    for f in &cases {
        assert!(std::panic::catch_unwind(|| read(f)).is_ok(), "{:?}", String::from_utf8_lossy(f));
    }
    let far = plot_bytes(&cases[0]);
    assert!(far.warnings.iter().any(|w| w == "objects far outside the drawing were left out"), "{:?}", far.warnings);
    assert_eq!(far.shapes.len(), 1, "the line in range is kept");
}
