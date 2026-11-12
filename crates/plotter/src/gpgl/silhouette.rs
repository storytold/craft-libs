//! The GP-GL dialect of Silhouette cutters (and the Graphtec Craft Robo they descend from), in
//! three generations: [`Generation::Legacy`], [`Generation::Cameo3`] and [`Generation::Cameo4`].
//!
//! [`encode`] gives the whole program: per operation a setup preamble (mat, cutting region, tool
//! holder, force, speed, blade offset, corner sharpening), the registration-mark commands after
//! the first preamble, the move/draw body, and a park below the cut. Coordinates are feed axis
//! first in units of 0.05 mm.
//!
//! The device is asked about itself with [`INIT`], [`STATUS_QUERY`], [`VERSION_QUERY`] and
//! [`TOOL_QUERY`]; [`parse_status`], [`parse_version`] and [`parse_tool_setup`] read the replies.

use crate::{BBoxMm, Command, Error, Framing, Job, Operation, Path, Program, Registration};

/// Device units ("Silhouette Units") per mm: the native resolution is 0.05 mm.
pub const SU_PER_MM: f64 = 20.0;
/// Device units per inch.
pub const SU_PER_INCH: f64 = 508.0;

/// Escape `\x1b\x04` (EOT): initializes the device.
pub const INIT: &str = "\u{1b}\u{4}";
/// Escape `\x1b\x05` (ENQ): device status query; [`parse_status`] reads the reply.
pub const STATUS_QUERY: &str = "\u{1b}\u{5}";
/// Escape `\x1b\x15` (NAK): tool-setup query (Cameo 4 line); [`parse_tool_setup`] reads the reply.
pub const TOOL_QUERY: &str = "\u{1b}\u{15}";
/// The firmware-version command, `FG`. It is an ordinary command: [`VERSION_QUERY`] is the bytes
/// to send. The reply is e.g. `CAMEO 4 PRO V1.02`; [`parse_version`] reads it.
pub const VERSION_COMMAND: &str = "FG";
/// [`VERSION_COMMAND`] with its ETX terminator.
pub const VERSION_QUERY: &[u8] = b"FG\x03";

/// Most bytes of a reply the parsers look at. Real replies are a few dozen bytes.
pub const MAX_REPLY_LEN: usize = 256;
/// Most holders [`parse_tool_setup`] reports. Real machines have one or two.
pub const MAX_TOOL_HOLDERS: usize = 8;

/// Millimetres to device units, rounded.
pub fn su(mm: f64) -> i64 {
    (mm * SU_PER_MM).round() as i64
}

/// Which generation of the dialect a machine speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Generation {
    /// Before the Cameo 3 (Cameo 1/2, Portrait 1/2, Craft Robo, SD): `FW` media select, and
    /// force, speed and blade offset without a tool holder.
    #[default]
    Legacy,
    /// Cameo 3: commands addressed to a tool holder, mat select, blade depth.
    Cameo3,
    /// Cameo 4 line (Cameo 4/5, Portrait 3/4): force and blade offset are sent twice.
    Cameo4,
}

/// A cutting mat: the device's `TG` code and the mat's usable extent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mat {
    #[default]
    None,
    Cameo12x12,
    Cameo12x24,
    Portrait8x12,
    CameoPlus15x15,
    CameoPro24x24,
    Portrait8_5x12,
}

impl Mat {
    pub const ALL: &'static [Mat] =
        &[Mat::None, Mat::Cameo12x12, Mat::Cameo12x24, Mat::Portrait8x12, Mat::CameoPlus15x15, Mat::CameoPro24x24, Mat::Portrait8_5x12];

    /// The device's `TG` mat code.
    pub fn code(self) -> u8 {
        match self {
            Mat::None => 0,
            Mat::Cameo12x12 => 1,
            Mat::Cameo12x24 => 2,
            Mat::Portrait8x12 => 3,
            Mat::CameoPlus15x15 => 8,
            Mat::CameoPro24x24 => 9,
            Mat::Portrait8_5x12 => 11,
        }
    }

    /// A short name by size (`12x24`, `none`, …); [`Mat::parse`] reads it back.
    pub fn id(self) -> &'static str {
        match self {
            Mat::None => "none",
            Mat::Cameo12x12 => "12x12",
            Mat::Cameo12x24 => "12x24",
            Mat::Portrait8x12 => "8x12",
            Mat::CameoPlus15x15 => "15x15",
            Mat::CameoPro24x24 => "24x24",
            Mat::Portrait8_5x12 => "8.5x12",
        }
    }

    pub fn parse(s: &str) -> Option<Mat> {
        let s = s.trim().to_ascii_lowercase();
        if s == "portrait8x12" {
            return Some(Mat::Portrait8x12);
        }
        Mat::ALL.iter().copied().find(|m| m.id() == s)
    }

    /// Usable mat extent (down, right) in inches. The Pro mat is 24 in × 24 in even though the
    /// tray takes 609.6 mm-wide material.
    pub fn extent_inch(self) -> (f64, f64) {
        match self {
            Mat::None => (0.0, 0.0),
            Mat::Cameo12x12 => (12.0, 12.0),
            Mat::Cameo12x24 => (24.0, 12.0),
            Mat::Portrait8x12 => (12.0, 8.0),
            Mat::CameoPlus15x15 => (15.0, 15.0),
            Mat::CameoPro24x24 => (24.0, 24.0),
            Mat::Portrait8_5x12 => (12.0, 8.5),
        }
    }

    /// Usable mat extent (down, right) in mm.
    pub fn extent_mm(self) -> (f64, f64) {
        let (d, r) = self.extent_inch();
        (d * 25.4, r * 25.4)
    }
}

/// A mat is serialized as its short name ([`Mat::id`]); an unknown name is an error.
#[cfg(feature = "serde")]
impl serde::Serialize for Mat {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.id())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Mat {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Mat, D::Error> {
        let name = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        Mat::parse(&name).ok_or_else(|| serde::de::Error::custom(format!("unknown mat `{name}`")))
    }
}

/// The machine and media a program is written for: what the preamble announces besides the
/// per-operation tool settings.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Setup {
    pub generation: Generation,
    /// The machine has two tool holders. A Cameo 4 line machine with one holder gets no `J`.
    pub two_holders: bool,
    /// The cutting region (down, right) in mm the boundary commands announce.
    pub area_down_mm: f64,
    pub area_right_mm: f64,
    pub mat: Mat,
    /// Landscape media feed (`TB50,1`).
    pub landscape: bool,
    /// Roller track enhancing (`FY0`).
    pub track_enhancing: bool,
    /// Media preset id for the legacy `FW` command; ids outside 100..=300 are sent as 300 (custom).
    pub media_id: Option<u16>,
}

/// `FF` corner-sharpening pair: tenths of a millimetre (not device units), truncated, start-only
/// first then start+end; pens send zeros.
fn sharpen_pair(op: &Operation, t: u8) -> [Command; 2] {
    let s = &op.settings;
    let (start, end) = if op.pen { (0, 0) } else { (((s.sharpen_start_mm + 0.05) * 10.0) as i64, ((s.sharpen_end_mm + 0.05) * 10.0) as i64) };
    [
        Command::SharpenCorners { start_tenths: start, end_tenths: 0, toolholder: t },
        Command::SharpenCorners { start_tenths: start, end_tenths: end, toolholder: t },
    ]
}

/// Mat select, reset, feed direction and cutting region: what the Cameo 3 and newer receive
/// before any tool command.
fn mat_block(setup: &Setup) -> Vec<Command> {
    vec![
        Command::SelectMat(setup.mat),
        Command::Reset,
        Command::PortraitMode(setup.landscape),
        Command::BoundaryTopLeft { down_mm: 0.0, right_mm: 0.0 },
        Command::BoundaryBottomRight { down_mm: setup.area_down_mm, right_mm: setup.area_right_mm },
    ]
}

/// Blade depth (`TF`, holder 1 only).
fn depth_cmd(op: &Operation) -> Option<Command> {
    (op.holder == 1).then_some(op.settings.depth).flatten().map(|d| Command::Depth(d, 1))
}

fn step(v: Option<u32>) -> Option<u8> {
    v.and_then(|v| u8::try_from(v).ok())
}

fn cameo4_setup(setup: &Setup, op: &Operation, first: bool, c: &mut Vec<Command>) {
    let t = op.holder;
    let (force, speed) = (step(op.settings.force), step(op.settings.speed));
    if first {
        c.extend(mat_block(setup));
    }
    if setup.two_holders {
        c.push(Command::SelectTool(t));
    }
    if let Some(p) = force {
        c.push(Command::Pressure(p, t));
        // On first connect acceleration is set to 0, then bumped later.
        c.push(Command::Acceleration(0));
    }
    if let Some(s) = speed {
        c.push(Command::Speed(s, t));
    }
    // The first cutter-offset command always reads 0 mm × 0.05 mm.
    c.push(Command::CutterOffset { x_mm: 0.0, y_mm: 0.05, toolholder: t });
    c.push(Command::Lift { lift: op.settings.sharpen_corners, toolholder: t });
    c.extend(sharpen_pair(op, t));
    // Force and acceleration go out a second time (observed behaviour).
    if let Some(p) = force {
        c.push(Command::Pressure(p, t));
        c.push(Command::Acceleration(3));
    }
    c.push(Command::CutterOffset { x_mm: blade_offset(op), y_mm: 0.05, toolholder: t });
    c.extend(depth_cmd(op));
    if first && setup.track_enhancing {
        c.push(Command::TrackEnhancing(true));
    }
}

fn cameo3_setup(setup: &Setup, op: &Operation, first: bool, c: &mut Vec<Command>) {
    let t = op.holder;
    if first {
        c.extend(mat_block(setup));
    }
    c.push(Command::SelectTool(t));
    if let Some(s) = step(op.settings.speed) {
        c.push(Command::Speed(s, t));
    }
    if let Some(p) = step(op.settings.force) {
        c.push(Command::Pressure(p, t));
    }
    let zero = Command::CutterOffset { x_mm: 0.0, y_mm: 0.05, toolholder: t };
    if op.pen {
        c.push(zero.clone());
    }
    c.push(Command::Lift { lift: op.settings.sharpen_corners, toolholder: t });
    c.extend(sharpen_pair(op, t));
    if !op.pen {
        c.push(zero);
        c.push(Command::CutterOffset { x_mm: blade_offset(op), y_mm: 0.05, toolholder: t });
    }
    c.extend(depth_cmd(op));
    if first && setup.track_enhancing {
        c.push(Command::TrackEnhancing(true));
    }
}

fn legacy_setup(setup: &Setup, op: &Operation, first: bool, c: &mut Vec<Command>) {
    if first && let Some(id) = setup.media_id {
        c.push(Command::MediaSelect(if (100..=300).contains(&id) { id } else { 300 }));
    }
    if let Some(s) = step(op.settings.speed) {
        c.push(Command::SpeedLegacy(s));
    }
    if let Some(p) = step(op.settings.force) {
        c.push(Command::PressureLegacy(p));
    }
    c.push(Command::CutterOffsetLegacy(blade_offset(op)));
    if first {
        c.push(Command::TrackEnhancing(setup.track_enhancing));
        c.push(Command::Reset);
        c.push(Command::PortraitMode(setup.landscape));
        c.push(Command::BoundaryTopLeft { down_mm: 0.0, right_mm: 0.0 });
        c.push(Command::BoundaryBottomRight { down_mm: setup.area_down_mm, right_mm: setup.area_right_mm });
        c.push(Command::PenUp);
        c.push(Command::Raw("FE0,0".to_string()));
        c.push(Command::Raw("FF0,0,0".to_string()));
    }
}

fn blade_offset(op: &Operation) -> f64 {
    if op.pen { 0.0 } else { op.settings.blade_offset_mm }
}

/// The preamble for one operation. `first` adds the job-level block (mat, region, media, track
/// enhancing); later operations only re-tool. A force or speed that does not fit the command's
/// one-byte step is left to the machine.
pub fn setup(setup: &Setup, op: &Operation, first: bool) -> Vec<Command> {
    let mut c = Vec::new();
    match setup.generation {
        Generation::Cameo4 => cameo4_setup(setup, op, first, &mut c),
        Generation::Cameo3 => cameo3_setup(setup, op, first, &mut c),
        Generation::Legacy => legacy_setup(setup, op, first, &mut c),
    }
    c
}

/// Registration-mark commands: mark geometry, then the search (or manual positioning) request.
/// The device answers `    0\x03` (see [`parse_status`]) once it has the marks.
pub fn registration(reg: &Registration) -> Vec<Command> {
    let raw = |s: &str| Command::Raw(s.to_string());
    let mut c = vec![
        Command::PortraitMode(false),
        raw("TB99"),
        // Mark type 2 = Cameo/Portrait marks; arm length 400 SU (20 mm); stroke 10 SU.
        raw("TB52,2"),
        raw("TB51,400"),
        raw("TB53,10"),
        raw("TB55,1"),
    ];
    let (h, w) = (su(reg.length_mm), su(reg.width_mm));
    if reg.search {
        // Search 10 mm around the expected first-mark position.
        let top = su((reg.origin_down_mm - 10.0).max(0.0));
        let left = su((reg.origin_right_mm - 10.0).max(0.0));
        let code = if reg.quad { 124 } else { 123 };
        c.push(Command::Raw(format!("TB{code},{h},{w},{top},{left}")));
    } else {
        c.push(Command::Raw(format!("TB23,{h},{w}")));
    }
    c
}

/// Move/draw commands for paths, offset by the job origin. Paths of fewer than two points are
/// skipped.
pub fn plot_body(paths: &[Path], down: f64, right: f64) -> Vec<Command> {
    let mut commands = Vec::new();
    for path in paths {
        let Some((first, rest)) = path.split_first() else {
            continue;
        };
        if rest.is_empty() {
            continue;
        }
        commands.push(Command::Move { down_mm: first.1 + down, right_mm: first.0 + right });
        for &(x, y) in rest {
            commands.push(Command::Draw { down_mm: y + down, right_mm: x + right });
        }
    }
    commands
}

/// Park below the cut so the next job (or the cross-cutter) starts there.
pub fn park(bb: BBoxMm, end_feed_mm: f64) -> [Command; 2] {
    [Command::Move { down_mm: bb.2 + end_feed_mm, right_mm: 0.0 }, Command::Origin]
}

/// Encodes `job` for a Silhouette machine. A job with no geometry gives an empty program. With
/// registration, [`Program::registration_wait`] says where the sender waits for the marks.
pub fn encode(s: &Setup, job: &Job) -> Result<Program, Error> {
    let Some(prepared) = job.prepare()? else {
        return Ok(Program::default());
    };
    let mut commands = Vec::new();
    let mut registration_wait = None;
    for (i, (op, paths)) in job.operations.iter().zip(&prepared.operations).enumerate() {
        commands.extend(setup(s, op, i == 0));
        if i == 0
            && let Some(r) = &job.registration
        {
            commands.extend(registration(r));
            registration_wait = Some(commands.len());
        }
        commands.extend(plot_body(paths, job.origin_down_mm, job.origin_right_mm));
    }
    commands.extend(park(prepared.bbox_mm, job.end_feed_mm));
    Ok(Program { commands, bbox_mm: Some(prepared.bbox_mm), framing: Framing::Etx, registration_wait })
}

/// Device status from the status query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ready,
    Moving,
    /// No media loaded.
    Unloaded,
    /// A status byte this crate does not know (255 when the reply had none).
    Unknown(u8),
}

fn capped(reply: &[u8]) -> &[u8] {
    reply.get(..MAX_REPLY_LEN).unwrap_or(reply)
}

/// Parses a status reply (e.g. `b"    0\x03"`: an ASCII digit before the ETX terminator, possibly
/// padded with spaces). The same reply reports that the registration marks were found.
pub fn parse_status(reply: &[u8]) -> Status {
    match capped(reply).iter().find(|b| **b != b' ' && **b != 0x03).copied() {
        Some(b'0') => Status::Ready,
        Some(b'1') => Status::Moving,
        Some(b'2') => Status::Unloaded,
        Some(other) => Status::Unknown(other),
        None => Status::Unknown(255),
    }
}

fn reply_text(reply: &[u8]) -> String {
    String::from_utf8_lossy(capped(reply)).trim_matches(|c: char| c == '\u{3}' || c.is_whitespace()).to_string()
}

/// The firmware version string of a version reply, trimmed of ETX and padding.
pub fn parse_version(reply: &[u8]) -> String {
    reply_text(reply)
}

/// A tool as the tool-setup query reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Empty,
    RatchetBlade,
    AutoBlade,
    DeepCutBlade,
    KraftBlade,
    RotaryBlade,
    Pen,
    /// The holder reports a fault.
    Error,
}

impl Tool {
    pub fn from_code(code: u8) -> Option<Tool> {
        Some(match code {
            0 => Tool::Empty,
            1 => Tool::RatchetBlade,
            2 => Tool::AutoBlade,
            3 => Tool::DeepCutBlade,
            4 => Tool::KraftBlade,
            5 => Tool::RotaryBlade,
            7 => Tool::Pen,
            255 => Tool::Error,
            _ => return None,
        })
    }
}

/// Parses a tool-setup reply (`"2,7\x03"`: the tool codes of holder 1, holder 2). An unknown code
/// is `None`. At most [`MAX_TOOL_HOLDERS`] holders are reported.
pub fn parse_tool_setup(reply: &[u8]) -> Vec<Option<Tool>> {
    reply_text(reply)
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .take(MAX_TOOL_HOLDERS)
        .map(|s| s.trim().parse::<u8>().ok().and_then(Tool::from_code))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_conversions() {
        assert_eq!(su(0.05), 1);
        assert_eq!(su(0.9), 18); // stock blade diameter
        assert_eq!(su(600.0), 12_000);
        assert_eq!(su(3000.0), 60_000);
        assert_eq!(18.0 / SU_PER_MM, 0.9);
        assert_eq!((24.0 * SU_PER_INCH) as i64, 12_192);
        // Out of range saturates; not-a-number is zero.
        assert_eq!(su(f64::MAX), i64::MAX);
        assert_eq!(su(f64::NAN), 0);
    }

    #[test]
    fn query_bytes() {
        assert_eq!(INIT.as_bytes(), b"\x1b\x04");
        assert_eq!(STATUS_QUERY.as_bytes(), b"\x1b\x05");
        assert_eq!(TOOL_QUERY.as_bytes(), b"\x1b\x15");
        assert_eq!(VERSION_QUERY, [VERSION_COMMAND.as_bytes(), &[crate::gpgl::ETX]].concat());
    }

    #[test]
    fn parse_status_decodes_known_replies() {
        assert_eq!(parse_status(b"0"), Status::Ready);
        assert_eq!(parse_status(b"1"), Status::Moving);
        assert_eq!(parse_status(b"2"), Status::Unloaded);
        assert_eq!(parse_status(b"    0\x03"), Status::Ready);
        assert_eq!(parse_status(b"9"), Status::Unknown(b'9'));
        assert_eq!(parse_status(b""), Status::Unknown(255));
        assert_eq!(parse_status(&[0xff, 0x00]), Status::Unknown(0xff));
    }

    #[test]
    fn version_and_tool_setup_replies_parse() {
        assert_eq!(parse_version(b"CAMEO 4 PRO V1.02\x03"), "CAMEO 4 PRO V1.02");
        assert_eq!(parse_version(b"  \x03"), "");
        // Bytes that are not UTF-8 are replaced, never a panic.
        assert_eq!(parse_version(b"CAMEO\xff\x03"), "CAMEO\u{fffd}");
        assert_eq!(parse_tool_setup(b"2,7\x03"), vec![Some(Tool::AutoBlade), Some(Tool::Pen)]);
        assert_eq!(parse_tool_setup(b"2,0\x03"), vec![Some(Tool::AutoBlade), Some(Tool::Empty)]);
        assert_eq!(parse_tool_setup(b"9\x03"), vec![None]);
        assert_eq!(parse_tool_setup(b"255,x,-1,300"), vec![Some(Tool::Error), None, None, None]);
        assert!(parse_tool_setup(b"").is_empty());
    }

    #[test]
    fn replies_are_read_only_up_to_the_cap() {
        // A status digit past the cap is not seen.
        let mut padded = vec![b' '; MAX_REPLY_LEN];
        padded.push(b'0');
        assert_eq!(parse_status(&padded), Status::Unknown(255));
        *padded.last_mut().unwrap() = b' ';
        padded[MAX_REPLY_LEN - 1] = b'0';
        assert_eq!(parse_status(&padded), Status::Ready);
        assert_eq!(parse_version(&vec![b'A'; MAX_REPLY_LEN * 4]).len(), MAX_REPLY_LEN);
        // A reply of nothing but separators and codes reports at most MAX_TOOL_HOLDERS holders.
        assert_eq!(parse_tool_setup(&b"1,".repeat(100)).len(), MAX_TOOL_HOLDERS);
        assert_eq!(parse_tool_setup(&b"1,".repeat(MAX_TOOL_HOLDERS)).len(), MAX_TOOL_HOLDERS);
    }

    #[test]
    fn mats_round_trip_and_have_codes() {
        for m in Mat::ALL {
            assert_eq!(Mat::parse(m.id()), Some(*m));
        }
        assert_eq!(Mat::parse(" Portrait8x12 "), Some(Mat::Portrait8x12));
        assert_eq!(Mat::parse("13x13"), None);
        assert_eq!(Mat::ALL.iter().map(|m| m.code()).collect::<Vec<_>>(), [0, 1, 2, 3, 8, 9, 11]);
        let (down, right) = Mat::Cameo12x24.extent_mm();
        assert!((down - 609.6).abs() < 1e-9 && (right - 304.8).abs() < 1e-9);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn mats_serialize_as_their_names() {
        use serde::de::value::{Error, StrDeserializer, StringDeserializer};
        use serde::{Deserialize, de::IntoDeserializer};
        for m in Mat::ALL {
            let borrowed: StrDeserializer<Error> = m.id().into_deserializer();
            assert_eq!(Mat::deserialize(borrowed), Ok(*m));
        }
        let owned: StringDeserializer<Error> = "8.5X12 ".to_string().into_deserializer();
        assert_eq!(Mat::deserialize(owned), Ok(Mat::Portrait8_5x12));
        let unknown: StrDeserializer<Error> = "13x13".into_deserializer();
        assert_eq!(Mat::deserialize(unknown).unwrap_err().to_string(), "unknown mat `13x13`");
    }

    #[test]
    fn a_force_or_speed_past_one_byte_is_left_to_the_machine() {
        let mut op = Operation { holder: 1, ..Operation::default() };
        op.settings.force = Some(256);
        op.settings.speed = Some(u32::MAX);
        for generation in [Generation::Legacy, Generation::Cameo3, Generation::Cameo4] {
            let lines: Vec<String> = setup(&Setup { generation, ..Setup::default() }, &op, false).iter().map(Command::to_ascii).collect();
            assert!(!lines.iter().any(|l| l.starts_with("FX") || l.starts_with('!')), "{lines:?}");
        }
    }
}
