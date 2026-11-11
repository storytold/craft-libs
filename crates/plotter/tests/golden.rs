//! Golden programs: the exact commands each language emits for fixed jobs. A change here changes
//! what reaches a machine, so review any diff by hand.

use craft_plotter::gpgl::silhouette::{self, Generation, Mat, Setup};
use craft_plotter::{Command, Error, Framing, Job, Operation, Path, Program, Registration, ToolSettings, dmpl, gpgl, hpgl};

/// A closed square and an open triangle.
fn design() -> Vec<Path> {
    vec![vec![(5.0, 5.0), (15.0, 5.0), (15.0, 15.0), (5.0, 15.0), (5.0, 5.0)], vec![(20.0, 5.0), (30.0, 15.0), (20.0, 15.0)]]
}

fn square() -> Vec<Path> {
    vec![vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (0.0, 0.0)]]
}

fn job(paths: Vec<Path>, force: Option<u32>, speed: Option<u32>) -> Job {
    let settings = ToolSettings { force, speed, ..ToolSettings::default() };
    Job { operations: vec![Operation { holder: 1, paths, settings, ..Operation::default() }], ..Job::default() }
}

/// A 12 in wide machine with the 12x24 mat, vinyl preset 102.
fn machine(generation: Generation, area_right_mm: f64) -> Setup {
    Setup {
        generation,
        two_holders: generation != Generation::Legacy,
        area_down_mm: 3000.0,
        area_right_mm,
        mat: Mat::Cameo12x24,
        media_id: Some(102),
        ..Setup::default()
    }
}

fn lines(p: Result<Program, Error>) -> String {
    p.unwrap().transcript().join(" ")
}

const BODY: &str = "M100,100 D100,300 D300,300 D300,100 D100,100 M100,400 D300,600 D300,400 M300,0 SO0";

#[test]
fn silhouette_cameo4_program() {
    let p = silhouette::encode(&machine(Generation::Cameo4, 304.8), &job(design(), Some(10), Some(5)));
    assert_eq!(lines(p), format!("TG2 FN0 TB50,0 \\0,0 Z60000,6096 J1 FX10,1 TJ0 !5,1 FC0,1,1 FE0,1 FF1,0,1 FF1,1,1 FX10,1 TJ3 FC18,1,1 {BODY}"));
}

#[test]
fn silhouette_cameo3_program() {
    let p = silhouette::encode(&machine(Generation::Cameo3, 304.8), &job(design(), Some(10), Some(5)));
    assert_eq!(lines(p), format!("TG2 FN0 TB50,0 \\0,0 Z60000,6096 J1 !5,1 FX10,1 FE0,1 FF1,0,1 FF1,1,1 FC0,1,1 FC18,1,1 {BODY}"));
}

#[test]
fn silhouette_legacy_program() {
    let p = silhouette::encode(&machine(Generation::Legacy, 304.0), &job(design(), Some(10), Some(5)));
    assert_eq!(lines(p), format!("FW102 !5 FX10 FC18 FY1 FN0 TB50,0 \\0,0 Z60000,6080 L0 FE0,0 FF0,0,0 {BODY}"));
}

#[test]
fn silhouette_stream_is_etx_framed() {
    let p = silhouette::encode(&machine(Generation::Legacy, 304.0), &job(vec![vec![(0.0, 0.0), (1.0, 0.0)]], None, None)).unwrap();
    assert_eq!(p.framing, Framing::Etx);
    assert_eq!(p.bbox_mm, Some((0.0, 0.0, 0.0, 1.0)));
    assert_eq!(
        p.to_stream(),
        b"FW102\x03FC18\x03FY1\x03FN0\x03TB50,0\x03\\0,0\x03Z60000,6080\x03L0\x03FE0,0\x03FF0,0,0\x03M0,0\x03D0,20\x03M0,0\x03SO0\x03".to_vec()
    );
}

#[test]
fn silhouette_setup_variants() {
    let op = &job(vec![], Some(10), Some(5)).operations[0];
    let ascii = |s: &Setup, op: &Operation| silhouette::setup(s, op, true).iter().map(Command::to_ascii).collect::<Vec<_>>().join(" ");
    // The 24 in Pro: its own mat, 600 mm across.
    let pro = Setup {
        generation: Generation::Cameo4,
        two_holders: true,
        area_down_mm: 3000.0,
        area_right_mm: 600.0,
        mat: Mat::CameoPro24x24,
        ..Setup::default()
    };
    assert_eq!(ascii(&pro, op), "TG9 FN0 TB50,0 \\0,0 Z60000,12000 J1 FX10,1 TJ0 !5,1 FC0,1,1 FE0,1 FF1,0,1 FF1,1,1 FX10,1 TJ3 FC18,1,1");
    // A one-holder machine of the Cameo 4 line gets no tool select.
    let portrait = Setup { two_holders: false, mat: Mat::Portrait8_5x12, ..pro.clone() };
    assert!(ascii(&portrait, op).starts_with("TG11 FN0 TB50,0 \\0,0 Z60000,12000 FX10,1 "));
    // Blade depth and track enhancing close the preamble; depth is for holder 1 only.
    let mut deep = op.clone();
    deep.settings.depth = Some(4);
    let enhanced = Setup { track_enhancing: true, landscape: true, ..pro.clone() };
    assert!(ascii(&enhanced, &deep).ends_with(" FC18,1,1 TF4,1 FY0"));
    assert!(ascii(&enhanced, &deep).contains(" TB50,1 "));
    deep.holder = 2;
    assert!(!ascii(&enhanced, &deep).contains("TF"));
    // A pen: no blade offset, no corner sharpening.
    let pen = Operation { pen: true, holder: 2, ..op.clone() };
    assert!(ascii(&pro, &pen).ends_with("J2 FX10,2 TJ0 !5,2 FC0,1,2 FE0,2 FF0,0,2 FF0,0,2 FX10,2 TJ3 FC0,1,2"));
    let cameo3 = Setup { generation: Generation::Cameo3, ..pro.clone() };
    assert!(ascii(&cameo3, &pen).ends_with("J2 !5,2 FX10,2 FC0,1,2 FE0,2 FF0,0,2 FF0,0,2"));
    // Legacy: a media id outside the device table is sent as custom; track enhancing is FY0.
    let legacy = Setup { generation: Generation::Legacy, media_id: Some(999), track_enhancing: true, ..Setup::default() };
    assert_eq!(ascii(&legacy, &pen), "FW300 !5 FX10 FC0 FY0 FN0 TB50,0 \\0,0 Z0,0 L0 FE0,0 FF0,0,0");
}

#[test]
fn silhouette_retools_between_operations() {
    let mut j = job(square(), Some(10), Some(10));
    j.operations[0].pen = true;
    j.operations[0].holder = 2;
    let settings = ToolSettings { force: Some(20), speed: Some(5), depth: Some(3), passes: 2, ..ToolSettings::default() };
    j.operations.push(Operation { holder: 1, paths: square(), settings, ..Operation::default() });
    let setup = Setup { mat: Mat::Cameo12x12, ..machine(Generation::Cameo4, 304.8) };
    let square = "M0,0 D0,200 D200,200 D200,0 D0,0";
    assert_eq!(
        lines(silhouette::encode(&setup, &j)),
        format!(
            "TG1 FN0 TB50,0 \\0,0 Z60000,6096 J2 FX10,2 TJ0 !10,2 FC0,1,2 FE0,2 FF0,0,2 FF0,0,2 FX10,2 TJ3 FC0,1,2 {square} \
             J1 FX20,1 TJ0 !5,1 FC0,1,1 FE0,1 FF1,0,1 FF1,1,1 FX20,1 TJ3 FC18,1,1 TF3,1 {square} {square} M200,0 SO0"
        )
    );
}

#[test]
fn silhouette_registration_marks() {
    let pro = Setup {
        generation: Generation::Cameo4,
        two_holders: true,
        area_down_mm: 3000.0,
        area_right_mm: 600.0,
        mat: Mat::CameoPro24x24,
        ..Setup::default()
    };
    let mut j = job(vec![vec![(20.0, 25.0), (30.0, 25.0)]], Some(10), Some(5));
    let mut reg = Registration { width_mm: 180.0, length_mm: 230.0, origin_right_mm: 15.0, origin_down_mm: 20.0, search: true, quad: false };
    j.registration = Some(reg);
    let p = silhouette::encode(&pro, &j).unwrap();
    let (head, rest) = p.split_at_registration();
    let text = |c: &[Command]| c.iter().map(Command::to_ascii).collect::<Vec<_>>().join(" ");
    assert!(text(head).ends_with(" FC18,1,1 TB50,0 TB99 TB52,2 TB51,400 TB53,10 TB55,1 TB123,4600,3600,200,100"));
    // The design is shifted so the first mark is the origin.
    assert_eq!(text(rest), "M100,100 D100,300 M100,0 SO0");
    assert_eq!(p.registration_wait, Some(head.len()));

    let ascii = |r: &Registration| text(&silhouette::registration(r));
    reg.quad = true;
    assert!(ascii(&reg).ends_with(" TB124,4600,3600,200,100"));
    // The search margin never goes negative.
    reg.origin_down_mm = 4.0;
    reg.origin_right_mm = 0.0;
    assert!(ascii(&reg).ends_with(" TB124,4600,3600,0,0"));
    reg.search = false;
    assert_eq!(ascii(&reg), "TB50,0 TB99 TB52,2 TB51,400 TB53,10 TB55,1 TB23,4600,3600");
}

#[test]
fn silhouette_plot_body_and_park() {
    let text = |c: &[Command]| c.iter().map(Command::to_ascii).collect::<Vec<_>>().join(" ");
    // Axes are swapped: down first. One-point and empty paths are skipped.
    let mut paths = square();
    paths.extend([vec![(1.0, 1.0)], vec![]]);
    assert_eq!(text(&silhouette::plot_body(&paths, 0.0, 0.0)), "M0,0 D0,200 D200,200 D200,0 D0,0");
    assert_eq!(text(&silhouette::plot_body(&square(), 1.0, 2.0)), "M20,40 D20,240 D220,240 D220,40 D20,40");
    assert_eq!(text(&silhouette::park((0.0, 0.0, 10.0, 10.0), 5.0)), "M300,0 SO0");
    assert_eq!(text(&silhouette::park((0.0, 0.0, 10.0, 10.0), -5.0)), "M100,0 SO0");
}

#[test]
fn bbox_only_traces_the_rectangle_with_the_first_tool() {
    let mut j = job(vec![vec![(2.0, 1.0), (6.0, 5.0)]], None, None);
    j.operations.push(Operation { holder: 2, paths: square(), ..Operation::default() });
    j.bbox_only = true;
    let s = Setup::default();
    assert!(lines(silhouette::encode(&s, &j)).ends_with(" M0,0 D0,200 D200,200 D200,0 D0,0 M200,0 SO0"));
    assert_eq!(lines(hpgl::encode(&j)), "IN; SP1; PU0,400; PD400,400,400,0,0,0,0,400; PU; SP0;");
    assert_eq!(lines(dmpl::encode(&j)), ";: EC1 U0,394 D394,394,394,0,0,0,0,394 U0,0 @");
    assert_eq!(lines(gpgl::encode(&j)), "M0,0 D0,100 D100,100 D100,0 D0,0 M100,0");
}

#[test]
fn hpgl_program() {
    let p = hpgl::encode(&job(design(), None, None)).unwrap();
    assert_eq!(p.transcript().join(" "), "IN; SP1; PU200,400; PD600,400,600,0,200,0,200,400; PU800,400; PD1200,0,800,0; PU; SP0;");
    assert_eq!(p.framing, Framing::Newline);
    assert_eq!(p.to_stream(), b"IN;\nSP1;\nPU200,400;\nPD600,400,600,0,200,0,200,400;\nPU800,400;\nPD1200,0,800,0;\nPU;\nSP0;\n".to_vec());
    assert_eq!(p.bbox_mm, Some((5.0, 5.0, 15.0, 30.0)));
    assert_eq!(p.registration_wait, None);
    // A second operation selects its own pen; margins move the design.
    let mut j = job(square(), None, None);
    j.operations.push(Operation { holder: 2, paths: square(), ..Operation::default() });
    j.origin_down_mm = 1.0;
    j.origin_right_mm = 2.0;
    assert_eq!(lines(hpgl::encode(&j)), "IN; SP1; PU80,400; PD480,400,480,0,80,0,80,400; SP2; PU80,400; PD480,400,480,0,80,0,80,400; PU; SP0;");
}

#[test]
fn dmpl_program() {
    let p = dmpl::encode(&job(design(), None, None)).unwrap();
    assert_eq!(p.transcript().join(" "), ";: EC1 U197,394 D591,394,591,0,197,0,197,394 U787,394 D1181,0,787,0 U0,0 @");
    assert_eq!(p.framing, Framing::Newline);
    assert_eq!(p.to_stream(), b";:\nEC1\nU197,394\nD591,394,591,0,197,0,197,394\nU787,394\nD1181,0,787,0\nU0,0\n@\n".to_vec());
}

#[test]
fn gpgl_program() {
    let p = gpgl::encode(&job(design(), None, None)).unwrap();
    assert_eq!(p.transcript().join(" "), "M50,50 D50,150 D150,150 D150,50 D50,50 M50,200 D150,300 D150,200 M150,0");
    assert_eq!(p.framing, Framing::Etx);
    assert_eq!(p.to_stream().last(), Some(&gpgl::ETX));
    // Speed when set; the park never backs into the design.
    let mut j = job(square(), Some(9), Some(4));
    j.end_feed_mm = -50.0;
    assert_eq!(lines(gpgl::encode(&j)), "!4 M0,0 D0,100 D100,100 D100,0 D0,0 M100,0");
    j.end_feed_mm = 5.0;
    j.operations[0].settings.passes = 2;
    assert_eq!(lines(gpgl::encode(&j)), "!4 M0,0 D0,100 D100,100 D100,0 D0,0 M0,0 D0,100 D100,100 D100,0 D0,0 M150,0");
}

#[test]
fn a_job_without_geometry_is_an_empty_program() {
    let empty = job(vec![vec![(1.0, 1.0)]], Some(1), Some(1));
    for p in [hpgl::encode(&empty), dmpl::encode(&empty), gpgl::encode(&empty), silhouette::encode(&Setup::default(), &empty)] {
        assert_eq!(p, Ok(Program::default()));
    }
}

#[test]
fn a_refused_job_encodes_nothing() {
    let mut j = job(square(), None, None);
    j.operations[0].settings.passes = 11;
    for p in [hpgl::encode(&j), dmpl::encode(&j), gpgl::encode(&j), silhouette::encode(&Setup::default(), &j)] {
        assert_eq!(p, Err(Error::Passes(11)));
    }
}
