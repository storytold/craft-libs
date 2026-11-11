//! Command languages of vinyl cutters and pen plotters: HP-GL ([`hpgl`]), DMPL ([`dmpl`]) and
//! Graphtec GP-GL ([`gpgl`], plain and the Silhouette dialect in [`gpgl::silhouette`]).
//!
//! A [`Job`] is ordered [`Operation`]s: polylines in millimetres (`x` right, `y` down, origin at
//! the media's top-left corner) plus the tool settings they are traced with. Each language's
//! `encode` turns a job into a [`Program`]: the commands, how they are framed on the wire, and
//! where a sender must wait for the device. The Silhouette module also has the bytes of the
//! device queries and the parsers of their replies.
//!
//! The crate does no I/O: bytes and plain types in, bytes and plain types out. Opening a port,
//! writing a program to it and polling for replies are the caller's business, and so is what a
//! particular machine can do (cutting area, force range, which tools it holds).
//!
//! Input is not trusted. A job is bounded by [`MAX_OPERATIONS`], [`MAX_POINTS`], [`MAX_PASSES`]
//! and [`MAX_OVERCUT_MM`], and must be finite; a reply is read only up to
//! [`gpgl::silhouette::MAX_REPLY_LEN`] bytes. Nothing here panics.
//!
//! Depends on no other Craft crate. Where the protocol facts come from is in the README.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod dmpl;
pub mod gpgl;
pub mod hpgl;

use gpgl::silhouette::{Mat, su};

/// Most points a job may hold, over all operations, before passes and overcut multiply them.
pub const MAX_POINTS: usize = 2_000_000;
/// Most times an operation may trace each path.
pub const MAX_PASSES: u8 = 10;
/// Most operations in a job.
pub const MAX_OPERATIONS: usize = 64;
/// Largest line-segment overcut at sharp corners, mm.
pub const MAX_OVERCUT_MM: f64 = 2.0;

/// Why a job was refused. Nothing is encoded from a refused job.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("{0} operations exceeds the {MAX_OPERATIONS} cap")]
    TooManyOperations(usize),
    #[error("{0} points exceeds the {MAX_POINTS} cap")]
    TooManyPoints(usize),
    #[error("passes {0} outside 1..={MAX_PASSES}")]
    Passes(u8),
    #[error("overcut {0} mm outside 0..={MAX_OVERCUT_MM}")]
    Overcut(f64),
    #[error("a coordinate, offset or setting is not finite")]
    NotFinite,
}

/// A polyline in mm, `(x right, y down)`.
pub type Path = Vec<(f64, f64)>;

/// Bounding box in mm `(down-min, right-min, down-max, right-max)`.
pub type BBoxMm = (f64, f64, f64, f64);

/// Tool settings for one operation. `None` leaves the value to the machine's panel.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSettings {
    /// Force in the device's own steps.
    pub force: Option<u32>,
    /// Speed in the device's own steps.
    pub speed: Option<u32>,
    /// Trace each path this many times, back to back (0 counts as 1; at most [`MAX_PASSES`]).
    pub passes: u8,
    /// Blade depth 0..10 for a blade the machine adjusts itself (Silhouette `TF`, holder 1 only).
    pub depth: Option<u8>,
    /// Line-segment overcut at sharp corners, mm (0 = off; at most [`MAX_OVERCUT_MM`]).
    pub overcut_mm: f64,
    /// Blade swivel offset, mm (0.9 for the stock Silhouette blade).
    pub blade_offset_mm: f64,
    /// Lift the head at sharp corners.
    pub sharpen_corners: bool,
    pub sharpen_start_mm: f64,
    pub sharpen_end_mm: f64,
}

impl Default for ToolSettings {
    fn default() -> Self {
        ToolSettings {
            force: None,
            speed: None,
            passes: 1,
            depth: None,
            overcut_mm: 0.0,
            blade_offset_mm: 0.9,
            sharpen_corners: false,
            sharpen_start_mm: 0.1,
            sharpen_end_mm: 0.1,
        }
    }
}

/// One ordered step of a job: geometry plus how to trace it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Operation {
    /// A pen, not a blade: no blade offset and no corner sharpening.
    pub pen: bool,
    /// Tool holder, 1-based.
    pub holder: u8,
    /// Polylines in mm. A path of fewer than two points is skipped.
    pub paths: Vec<Path>,
    pub settings: ToolSettings,
}

/// Print-and-cut registration: the optical sensor finds the printed marks before cutting. All mm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Registration {
    /// Distance between the marks across the media (right axis).
    pub width_mm: f64,
    /// Distance between the marks along the feed (down axis).
    pub length_mm: f64,
    /// Position of the top-left mark on the page; the design is shifted so that the mark becomes
    /// the origin.
    pub origin_right_mm: f64,
    pub origin_down_mm: f64,
    /// Automatic search; `false` = manual positioning.
    pub search: bool,
    /// Four-mark search (only machines that have it).
    pub quad: bool,
}

/// What to trace, in which order, and where on the media.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Job {
    pub operations: Vec<Operation>,
    /// Offset of the job origin from the media's top-left corner, mm.
    pub origin_down_mm: f64,
    pub origin_right_mm: f64,
    /// Extra feed after the last cut (GP-GL park position), mm.
    pub end_feed_mm: f64,
    /// Trace only the bounding rectangle, with the first operation's tool (placement preview).
    pub bbox_only: bool,
    /// Registration marks. Every language shifts the design by the mark origin; only the
    /// Silhouette dialect has the commands that make the device look for the marks.
    pub registration: Option<Registration>,
}

/// The paths a job traces, ready to encode: see [`Job::prepare`].
#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    /// One path list per operation, in job order (only the first, holding the bounding rectangle,
    /// for a `bbox_only` job).
    pub operations: Vec<Vec<Path>>,
    /// Bounding box of everything the job traces, before the `bbox_only` substitution.
    pub bbox_mm: BBoxMm,
}

impl Job {
    /// Checks the job against the crate's limits. Every `encode` runs this first.
    pub fn check(&self) -> Result<(), Error> {
        if self.operations.len() > MAX_OPERATIONS {
            return Err(Error::TooManyOperations(self.operations.len()));
        }
        let reg = self.registration.map_or([0.0; 4], |r| [r.width_mm, r.length_mm, r.origin_right_mm, r.origin_down_mm]);
        let mut finite = reg.iter().chain(&[self.origin_down_mm, self.origin_right_mm, self.end_feed_mm]).all(|v| v.is_finite());
        let mut points = 0usize;
        for op in &self.operations {
            let s = &op.settings;
            if s.passes > MAX_PASSES {
                return Err(Error::Passes(s.passes));
            }
            if s.overcut_mm.is_nan() || s.overcut_mm > MAX_OVERCUT_MM {
                return Err(Error::Overcut(s.overcut_mm));
            }
            finite &= [s.overcut_mm, s.blade_offset_mm, s.sharpen_start_mm, s.sharpen_end_mm].iter().all(|v| v.is_finite());
            for path in &op.paths {
                points = points.saturating_add(path.len());
                finite &= path.iter().all(|(x, y)| x.is_finite() && y.is_finite());
            }
        }
        if points > MAX_POINTS {
            return Err(Error::TooManyPoints(points));
        }
        if !finite {
            return Err(Error::NotFinite);
        }
        Ok(())
    }

    /// Checks the job, then gives each operation's paths as the tool traces them: overcut applied,
    /// each path repeated once per pass, shifted so the first registration mark is the origin.
    /// `None` when the job has no geometry.
    pub fn prepare(&self) -> Result<Option<Prepared>, Error> {
        self.check()?;
        let reg = self.registration.as_ref();
        let mut operations: Vec<Vec<Path>> = self.operations.iter().map(|op| prepared_paths(op, reg)).collect();
        let Some(bbox_mm) = operations.iter().fold(None, |bb, paths| grow_bbox(bb, paths)) else {
            return Ok(None);
        };
        if self.bbox_only {
            operations.truncate(1);
            if let Some(first) = operations.first_mut() {
                *first = vec![bbox_rect(bbox_mm)];
            }
        }
        Ok(Some(Prepared { operations, bbox_mm }))
    }
}

fn grow_bbox(mut bbox: Option<BBoxMm>, paths: &[Path]) -> Option<BBoxMm> {
    for &(x, y) in paths.iter().flatten() {
        bbox = Some(match bbox {
            None => (y, x, y, x),
            Some((d0, r0, d1, r1)) => (d0.min(y), r0.min(x), d1.max(y), r1.max(x)),
        });
    }
    bbox
}

/// The mm bounding box of a path list, or `None` when it has no point.
pub fn find_bbox(paths: &[Path]) -> Option<BBoxMm> {
    grow_bbox(None, paths)
}

/// The closed rectangle path `(x, y)` around a bounding box.
pub fn bbox_rect(bb: BBoxMm) -> Path {
    vec![(bb.1, bb.0), (bb.3, bb.0), (bb.3, bb.2), (bb.1, bb.2), (bb.1, bb.0)]
}

/// Corners sharper than this turn (30°) get a line-segment overcut; gentler joints are flattened
/// curves and must not be overcut.
const OVERCUT_MIN_TURN_COS: f64 = 0.866;

fn prepared_paths(op: &Operation, reg: Option<&Registration>) -> Vec<Path> {
    let passes = op.settings.passes.max(1);
    let (sd, sr) = reg.map_or((0.0, 0.0), |r| (r.origin_down_mm, r.origin_right_mm));
    let mut out = Vec::with_capacity(op.paths.len().saturating_mul(usize::from(passes)));
    for path in op.paths.iter().filter(|p| p.len() >= 2) {
        let cut = if op.settings.overcut_mm > 0.0 { overcut_path(path, op.settings.overcut_mm) } else { path.clone() };
        let cut: Path = cut.into_iter().map(|(x, y)| (x - sr, y - sd)).collect();
        for _ in 0..passes {
            out.push(cut.clone());
        }
    }
    out
}

/// Line-segment overcut: at every corner sharper than 30° the blade runs `o` mm past the corner
/// and back, so a drag knife cuts the corner through. A closed path's closing corner counts.
pub fn overcut_path(path: &[(f64, f64)], o: f64) -> Path {
    let closed = path.len() > 2 && path.first() == path.last();
    let n = path.len();
    let mut out = Vec::with_capacity(n.saturating_mul(3));
    for (i, &b) in path.iter().enumerate() {
        out.push(b);
        // The segment arriving at b, and the one leaving it (wrapping on closed paths).
        let Some(&a) = i.checked_sub(1).and_then(|j| path.get(j)) else {
            continue;
        };
        let next = if i + 1 < n {
            path.get(i + 1)
        } else if closed {
            path.get(1)
        } else {
            None
        };
        let Some(&c) = next else { continue };
        let (ux, uy, ul) = (b.0 - a.0, b.1 - a.1, (b.0 - a.0).hypot(b.1 - a.1));
        let (vx, vy, vl) = (c.0 - b.0, c.1 - b.1, (c.0 - b.0).hypot(c.1 - b.1));
        if ul <= f64::EPSILON || vl <= f64::EPSILON {
            continue;
        }
        let cos = (ux * vx + uy * vy) / (ul * vl);
        if cos < OVERCUT_MIN_TURN_COS {
            out.push((b.0 + ux / ul * o, b.1 + uy / ul * o));
            out.push(b);
        }
    }
    out
}

/// Millimetres to a language's integer device units, rounded.
fn units(mm: f64, per_mm: f64) -> i64 {
    (mm * per_mm).round() as i64
}

/// HP-GL and DMPL are y-up: flip about the design's bottom edge so the cut reads the right way
/// round with the origin at the bottom-left.
fn y_up(job: &Job, bottom: f64, per: f64) -> impl Fn(&(f64, f64)) -> (i64, i64) {
    let (dm, rm) = (job.origin_down_mm, job.origin_right_mm);
    move |&(x, y)| (units(x + rm, per), units(bottom + dm - (y + dm), per))
}

/// One device command. [`Command::to_ascii`] gives the exact text, without the terminator the
/// program's [`Framing`] adds. The typed variants are the Silhouette GP-GL command set, with
/// lengths in millimetres; every other language builds its commands as [`Command::Raw`] text.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// `J<toolholder>`: select the active tool holder.
    SelectTool(u8),
    /// `FX<p>,<toolholder>`: cutting force.
    Pressure(u8, u8),
    /// `!<speed>,<toolholder>`: cut speed.
    Speed(u8, u8),
    /// `TF<depth>,<toolholder>`: blade depth.
    Depth(u8, u8),
    /// `FC<x>,<y>,<toolholder>`: blade swivel circle (in device units).
    CutterOffset { x_mm: f64, y_mm: f64, toolholder: u8 },
    /// `FE<n>,<toolholder>`: lift head between paths when `n == 1`.
    Lift { lift: bool, toolholder: u8 },
    /// `FF<start>,<end>,<toolholder>`: corner sharpening range, tenths of mm.
    SharpenCorners { start_tenths: i64, end_tenths: i64, toolholder: u8 },
    /// `TJ<acceleration>`: acceleration ramp.
    Acceleration(i64),
    /// `M<down>,<right>`: head-up move, mm coordinates.
    Move { down_mm: f64, right_mm: f64 },
    /// `D<down>,<right>`: head-down draw/cut to point, mm coordinates.
    Draw { down_mm: f64, right_mm: f64 },
    /// `\<down>,<right>`: cutting-region upper-left bound.
    BoundaryTopLeft { down_mm: f64, right_mm: f64 },
    /// `Z<down>,<right>`: cutting-region lower-right bound.
    BoundaryBottomRight { down_mm: f64, right_mm: f64 },
    /// `FY<0|1>`: roller track enhancing (`FY0` = on).
    TrackEnhancing(bool),
    /// `TG<code>`: select cutting mat.
    SelectMat(Mat),
    /// `FN0`: device soft reset.
    Reset,
    /// `TB50,0` / `TB50,1`: portrait/landscape feed direction.
    PortraitMode(bool),
    /// `L0`: pen-up marker between plot sections.
    PenUp,
    /// `SO0`: mark origin/service position.
    Origin,
    /// `H`: full home (Cameo 3 and newer).
    Home,
    /// `FW<id>`: media select (legacy generation only).
    MediaSelect(u16),
    /// `FX<p>`: force without a holder (legacy generation).
    PressureLegacy(u8),
    /// `!<s>`: speed without a holder (legacy generation).
    SpeedLegacy(u8),
    /// `FC<d>`: blade offset without a holder (legacy generation).
    CutterOffsetLegacy(f64),
    /// A command of another language (HP-GL, DMPL, plain GP-GL, G-code) or a fixed-text Silhouette
    /// command (`TB99`, …), already as text.
    Raw(String),
}

impl Command {
    pub fn to_ascii(&self) -> String {
        match self {
            Command::SelectTool(t) => format!("J{t}"),
            Command::Pressure(p, t) => format!("FX{p},{t}"),
            Command::Speed(s, t) => format!("!{s},{t}"),
            Command::Depth(d, t) => format!("TF{d},{t}"),
            Command::CutterOffset { x_mm, y_mm, toolholder } => format!("FC{},{},{}", su(*x_mm), su(*y_mm), toolholder),
            Command::Lift { lift, toolholder } => format!("FE{},{}", u8::from(*lift), toolholder),
            Command::SharpenCorners { start_tenths, end_tenths, toolholder } => format!("FF{start_tenths},{end_tenths},{toolholder}"),
            Command::Acceleration(a) => format!("TJ{a}"),
            Command::Move { down_mm, right_mm } => format!("M{},{}", su(*down_mm), su(*right_mm)),
            Command::Draw { down_mm, right_mm } => format!("D{},{}", su(*down_mm), su(*right_mm)),
            Command::BoundaryTopLeft { down_mm, right_mm } => format!("\\{},{}", su(*down_mm), su(*right_mm)),
            Command::BoundaryBottomRight { down_mm, right_mm } => format!("Z{},{}", su(*down_mm), su(*right_mm)),
            Command::TrackEnhancing(on) => format!("FY{}", if *on { 0 } else { 1 }),
            Command::SelectMat(mat) => format!("TG{}", mat.code()),
            Command::Reset => "FN0".to_string(),
            Command::PortraitMode(landscape) => format!("TB50,{}", u8::from(*landscape)),
            Command::PenUp => "L0".to_string(),
            Command::Origin => "SO0".to_string(),
            Command::Home => "H".to_string(),
            Command::MediaSelect(id) => format!("FW{id}"),
            Command::PressureLegacy(p) => format!("FX{p}"),
            Command::SpeedLegacy(s) => format!("!{s}"),
            Command::CutterOffsetLegacy(d) => format!("FC{}", su(*d)),
            Command::Raw(s) => s.clone(),
        }
    }
}

/// How a program's commands are framed on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Framing {
    /// GP-GL: each command followed by ETX.
    #[default]
    Etx,
    /// HP-GL / DMPL / G-code: commands carry their own terminators; newline between.
    Newline,
}

/// An encoded program: the commands, their framing, and where a sender must pause for the
/// registration-mark search.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Program {
    pub commands: Vec<Command>,
    pub bbox_mm: Option<BBoxMm>,
    pub framing: Framing,
    /// Registration: after this many commands the sender must wait for the device to report the
    /// marks found before streaming the rest.
    pub registration_wait: Option<usize>,
}

impl Program {
    /// The exact byte stream to write to the device.
    pub fn to_stream(&self) -> Vec<u8> {
        stream(&self.commands, self.framing)
    }

    /// One command per line, for dry-run transcripts.
    pub fn transcript(&self) -> Vec<String> {
        self.commands.iter().map(Command::to_ascii).collect()
    }

    /// The commands to send before waiting for the registration marks, and the commands to send
    /// once the device has found them. Without registration everything is in the second part.
    pub fn split_at_registration(&self) -> (&[Command], &[Command]) {
        let at = self.registration_wait.unwrap_or(0).min(self.commands.len());
        self.commands.split_at_checked(at).unwrap_or((&[], &self.commands))
    }
}

/// The byte stream of `commands` under `framing`.
pub fn stream(commands: &[Command], framing: Framing) -> Vec<u8> {
    let end = match framing {
        Framing::Etx => gpgl::ETX,
        Framing::Newline => b'\n',
    };
    let mut out = Vec::with_capacity(commands.len().saturating_mul(8));
    for cmd in commands {
        out.extend_from_slice(cmd.to_ascii().as_bytes());
        out.push(end);
    }
    out
}

/// A GP-GL byte stream: each command's ASCII followed by an ETX byte.
pub fn delimit(commands: &[Command]) -> Vec<u8> {
    stream(commands, Framing::Etx)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Path {
        vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (0.0, 0.0)]
    }

    fn job(paths: Vec<Path>) -> Job {
        Job { operations: vec![Operation { holder: 1, paths, ..Operation::default() }], ..Job::default() }
    }

    #[test]
    fn overcut_extends_sharp_corners_only() {
        // Every square corner gets out-and-back, including the closing corner.
        assert_eq!(
            overcut_path(&square(), 1.0),
            vec![
                (0.0, 0.0),
                (10.0, 0.0),
                (11.0, 0.0),
                (10.0, 0.0),
                (10.0, 10.0),
                (10.0, 11.0),
                (10.0, 10.0),
                (0.0, 10.0),
                (-1.0, 10.0),
                (0.0, 10.0),
                (0.0, 0.0),
                (0.0, -1.0),
                (0.0, 0.0),
            ]
        );
        // A gentle polyline (10° turns) is left alone.
        let gentle = vec![(0.0, 0.0), (10.0, 0.0), (19.85, 1.74)];
        assert_eq!(overcut_path(&gentle, 1.0), gentle);
    }

    #[test]
    fn prepare_repeats_passes_shifts_by_the_mark_origin_and_boxes() {
        let mut j = job(vec![vec![(20.0, 25.0), (30.0, 35.0)], vec![(1.0, 1.0)]]);
        j.operations[0].settings.passes = 3;
        j.registration =
            Some(Registration { width_mm: 180.0, length_mm: 230.0, origin_right_mm: 15.0, origin_down_mm: 20.0, search: true, quad: false });
        let p = j.prepare().unwrap().unwrap();
        // The one-point path is dropped; the other is traced three times, shifted.
        assert_eq!(p.operations, vec![vec![vec![(5.0, 5.0), (15.0, 15.0)]; 3]]);
        assert_eq!(p.bbox_mm, (5.0, 5.0, 15.0, 15.0));
        // Zero passes count as one.
        j.operations[0].settings.passes = 0;
        assert_eq!(j.prepare().unwrap().unwrap().operations[0].len(), 1);
        j.bbox_only = true;
        j.operations.push(j.operations[0].clone());
        assert_eq!(j.prepare().unwrap().unwrap().operations, vec![vec![bbox_rect((5.0, 5.0, 15.0, 15.0))]]);
        assert_eq!(job(vec![]).prepare(), Ok(None));
        assert_eq!(find_bbox(&[]), None);
    }

    #[test]
    fn every_job_limit_is_enforced() {
        let op = Operation { holder: 1, paths: vec![square()], ..Operation::default() };
        let mut j = Job { operations: vec![op.clone(); MAX_OPERATIONS], ..Job::default() };
        assert_eq!(j.check(), Ok(()));
        j.operations.push(op);
        assert_eq!(j.check(), Err(Error::TooManyOperations(MAX_OPERATIONS + 1)));

        let mut j = job(vec![vec![(0.0, 0.0); MAX_POINTS / 2]; 2]);
        assert_eq!(j.check(), Ok(()));
        j.operations[0].paths.push(vec![(0.0, 0.0)]);
        assert_eq!(j.check(), Err(Error::TooManyPoints(MAX_POINTS + 1)));

        let mut j = job(vec![square()]);
        j.operations[0].settings.passes = MAX_PASSES;
        assert_eq!(j.check(), Ok(()));
        j.operations[0].settings.passes = MAX_PASSES + 1;
        assert_eq!(j.check(), Err(Error::Passes(MAX_PASSES + 1)));

        let mut j = job(vec![square()]);
        j.operations[0].settings.overcut_mm = MAX_OVERCUT_MM;
        assert_eq!(j.check(), Ok(()));
        j.operations[0].settings.overcut_mm = MAX_OVERCUT_MM + 0.01;
        assert_eq!(j.check(), Err(Error::Overcut(MAX_OVERCUT_MM + 0.01)));
        j.operations[0].settings.overcut_mm = f64::NAN;
        assert!(matches!(j.check(), Err(Error::Overcut(_))));
    }

    #[test]
    fn a_non_finite_job_is_refused_by_every_language() {
        let bad: [fn(&mut Job); 6] = [
            |j| j.operations[0].paths[0][1].0 = f64::NAN,
            |j| j.origin_down_mm = f64::INFINITY,
            |j| j.origin_right_mm = f64::NAN,
            |j| j.end_feed_mm = f64::NEG_INFINITY,
            |j| j.operations[0].settings.blade_offset_mm = f64::NAN,
            |j| {
                j.registration =
                    Some(Registration { width_mm: f64::NAN, length_mm: 1.0, origin_right_mm: 0.0, origin_down_mm: 0.0, search: false, quad: false })
            },
        ];
        for spoil in bad {
            let mut j = job(vec![square()]);
            spoil(&mut j);
            assert_eq!(hpgl::encode(&j), Err(Error::NotFinite));
            assert_eq!(dmpl::encode(&j), Err(Error::NotFinite));
            assert_eq!(gpgl::encode(&j), Err(Error::NotFinite));
            assert_eq!(gpgl::silhouette::encode(&gpgl::silhouette::Setup::default(), &j), Err(Error::NotFinite));
        }
    }

    #[test]
    fn huge_coordinates_saturate_instead_of_overflowing() {
        let j = job(vec![vec![(0.0, 0.0), (f64::MAX, f64::MAX)]]);
        for p in [hpgl::encode(&j), dmpl::encode(&j), gpgl::encode(&j)] {
            assert!(p.unwrap().transcript().iter().any(|l| l.contains(&i64::MAX.to_string())));
        }
    }

    #[test]
    fn framing_and_the_registration_split() {
        assert_eq!(delimit(&[Command::Home, Command::Origin]), b"H\x03SO0\x03".to_vec());
        let mut p = Program { commands: vec![Command::Home, Command::Origin], ..Program::default() };
        assert_eq!(p.to_stream(), b"H\x03SO0\x03".to_vec());
        assert_eq!(p.split_at_registration(), (&[][..], &p.commands[..]));
        p.registration_wait = Some(1);
        assert_eq!(p.split_at_registration(), (&[Command::Home][..], &[Command::Origin][..]));
        // A wait index past the end sends everything before the wait, never panics.
        p.registration_wait = Some(usize::MAX);
        assert_eq!(p.split_at_registration(), (&p.commands[..], &[][..]));
        p.framing = Framing::Newline;
        assert_eq!(p.to_stream(), b"H\nSO0\n".to_vec());
        assert_eq!(p.transcript(), ["H", "SO0"]);
    }
}
