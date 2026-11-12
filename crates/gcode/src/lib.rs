//! A Grbl-style G-code writer for cutting lasers and plotters.
//!
//! Modality: `G21` millimetres, `G90` absolute, feed in mm/min. Each contour is one cut: rapid
//! (`G0`) to the start with the beam off, power on, `G1` along the path, power off. Laser
//! machines take `M4` (dynamic power) plus an `S` power value in the conventional 0–1000 range,
//! or `M3` for controllers that only model a spindle ([`LaserMode`]). A contour can be traced in
//! several passes, with an optional power ramp. Coordinates are absolute millimetres, exactly as
//! given.
//!
//! [`Writer::new`] writes a file to keep: commented, fixed four-decimal coordinates.
//! [`Writer::bare`] writes the same moves for a sender that streams a job line by line and waits
//! for each `ok`: no comments, shortest coordinates. [`grbl`] has what such a sender needs from
//! the controller's side: the real-time command bytes and the reply parsers.
//!
//! Flattening curves, choosing what to cut and at what power, and machine or material presets
//! are the caller's business. Depends on no other Craft crate.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod grbl;

use std::fmt::Write as _;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum Error {
    /// A setting or power no machine should be sent. Names the knob and the value.
    #[error("{0}")]
    Settings(String),
    #[error("a vertex is not finite")]
    NotFinite,
}

/// Most repeated passes a job may ask for: past this the settings are a mistake, not a thick
/// material.
pub const MAX_PASSES: u32 = 10;

/// Highest power Grbl's `S` word conventionally accepts.
pub const MAX_POWER: u32 = 1000;

/// How the controller is told to fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LaserMode {
    /// `M4`: power tracks motion (Grbl laser mode). Rapids go dark on their own, but the writer
    /// still switches off explicitly so the output is safe on any sender.
    #[default]
    Dynamic,
    /// `M3`: constant power while moving, for controllers that only understand a spindle on/off.
    Constant,
}

impl LaserMode {
    /// The G-code word that turns the tool on.
    pub fn on_command(self) -> &'static str {
        match self {
            Self::Dynamic => "M4",
            Self::Constant => "M3",
        }
    }

    /// Parse the CLI/protocol spelling.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "dynamic" | "m4" => Some(Self::Dynamic),
            "constant" | "m3" => Some(Self::Constant),
            _ => None,
        }
    }
}

/// How every contour of a program is traced. Power is given per contour.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Feed rate while cutting, millimetres per minute.
    pub feed_mm_per_min: f64,
    /// How many times each contour is traced, 1–[`MAX_PASSES`].
    pub passes: u32,
    /// Spread the power across passes so the last one runs at full power. Useful on material
    /// that scorches at full power in one hit.
    pub ramp_power: bool,
    /// Which on/off command the controller understands.
    pub mode: LaserMode,
}

impl Default for Settings {
    fn default() -> Self {
        Self { feed_mm_per_min: 1000.0, passes: 1, ramp_power: false, mode: LaserMode::Dynamic }
    }
}

impl Settings {
    /// Checks every knob is inside its range.
    pub fn validate(&self) -> Result<(), Error> {
        check_feed(self.feed_mm_per_min)?;
        if self.passes == 0 || self.passes > MAX_PASSES {
            return Err(Error::Settings(format!("passes {} is outside the 1–{MAX_PASSES} range", self.passes)));
        }
        Ok(())
    }

    /// Power for one pass of a contour running at `base` power. Without a ramp every pass is at
    /// `base`; with one, pass `n` of `N` gets `base × n / N`, so the last pass is the full-power
    /// pass.
    pub fn power_for_pass(&self, base: u32, pass: u32) -> u32 {
        if !self.ramp_power || self.passes <= 1 || pass >= self.passes {
            return base;
        }
        let scaled = u64::from(base) * u64::from(pass) / u64::from(self.passes);
        let scaled = u32::try_from(scaled).unwrap_or(base);
        // A pass at S0 would trace the path with the beam off.
        scaled.max(1).min(base)
    }
}

fn check_feed(feed_mm_per_min: f64) -> Result<(), Error> {
    // Written to three decimals: a smaller rate would come out as `F0`, which Grbl rejects.
    if !feed_mm_per_min.is_finite() || feed_mm_per_min < 0.001 {
        return Err(Error::Settings(format!("feed rate {feed_mm_per_min} mm/min is not a finite rate of at least 0.001")));
    }
    Ok(())
}

/// A coordinate at up to three decimals, without trailing zeros.
fn short(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".to_string() } else { s.to_string() }
}

/// Checks a power is one the `S` word accepts; `what` names it in the error ("power",
/// "engrave power").
pub fn check_power(what: &str, power: u32) -> Result<(), Error> {
    if power > MAX_POWER {
        return Err(Error::Settings(format!("{what} {power} is outside the 0–{MAX_POWER} range")));
    }
    Ok(())
}

/// Builds one program: [`Writer::new`] writes the header, [`Writer::finish`] the footer.
#[derive(Debug)]
pub struct Writer {
    out: String,
    settings: Settings,
    /// No comments, shortest coordinates: see [`Writer::bare`].
    bare: bool,
}

impl Writer {
    /// Starts a program with `banner` as its first comment line.
    pub fn new(settings: Settings, banner: &str) -> Result<Writer, Error> {
        settings.validate()?;
        let mut w = Writer { out: String::with_capacity(4096), settings, bare: false };
        w.comment(banner);
        let _ = writeln!(w.out, "G21 ; millimetres");
        let _ = writeln!(w.out, "G90 ; absolute coordinates");
        let _ = writeln!(w.out, "G17 ; XY plane");
        // Start dark: the first move is a rapid to the first contour.
        let _ = writeln!(w.out, "{} S0 ; laser off until the first cut", w.settings.mode.on_command());
        Ok(w)
    }

    /// Starts a program for a sender that streams it line by line and waits for the controller's
    /// `ok` after each: every line is a command. There are no comments ([`Writer::comment`] writes
    /// nothing), coordinates have at most three decimals and no trailing zeros, the program starts
    /// with the tool off (`M5`), and it ends with the return to the origin, each contour having
    /// switched the tool off itself.
    pub fn bare(settings: Settings) -> Result<Writer, Error> {
        settings.validate()?;
        Ok(Writer { out: "G21\nG90\nG17\nM5\n".to_string(), settings, bare: true })
    }

    /// Changes the feed rate for the contours that follow.
    pub fn set_feed(&mut self, feed_mm_per_min: f64) -> Result<(), Error> {
        check_feed(feed_mm_per_min)?;
        self.settings.feed_mm_per_min = feed_mm_per_min;
        Ok(())
    }

    /// A comment line. Control characters become spaces: a line break in `text` (a layer name
    /// from a file, say) would otherwise start a line the machine runs as a command. Grbl acts
    /// on its real-time bytes (`!` feed hold, `?`, `~` and everything past ASCII) wherever they
    /// arrive, comments included, so those become `_`.
    pub fn comment(&mut self, text: &str) {
        if self.bare {
            return;
        }
        let safe = |c: char| match c {
            c if c.is_control() => ' ',
            '!' | '?' | '~' => '_',
            c if !c.is_ascii() => '_',
            c => c,
        };
        let text: String = text.chars().map(safe).collect();
        let _ = writeln!(self.out, "; {text}");
    }

    /// One contour along `points` (x, y in millimetres) at `power`, traced once per pass. A
    /// closed contour ends where it started; a single point is a minimal in-and-out dot; an
    /// empty one writes nothing. A refused contour writes nothing either.
    pub fn contour(&mut self, points: &[(f64, f64)], closed: bool, power: u32) -> Result<(), Error> {
        check_power("power", power)?;
        let Some(&first) = points.first() else {
            return Ok(());
        };
        if points.iter().any(|p| !p.0.is_finite() || !p.1.is_finite()) {
            return Err(Error::NotFinite);
        }
        let mut path = points.to_vec();
        if points.len() < 2 {
            path.extend([(first.0 + 0.001, first.1), first]);
        } else if closed {
            path.push(first);
        }
        let on = self.settings.mode.on_command();
        let passes = self.settings.passes;
        let bare = self.bare;
        let xy = |p: (f64, f64)| if bare { format!("X{} Y{}", short(p.0), short(p.1)) } else { format!("X{:.4} Y{:.4}", p.0, p.1) };
        for pass in 1..=passes {
            let _ = writeln!(self.out, "G0 {}", xy(first));
            let power = self.settings.power_for_pass(power, pass);
            if passes > 1 && !bare {
                let _ = writeln!(self.out, "{on} S{power} ; pass {pass} of {passes}");
            } else {
                let _ = writeln!(self.out, "{on} S{power}");
            }
            for p in path.iter().skip(1) {
                let _ = writeln!(self.out, "G1 {} F{}", xy(*p), (self.settings.feed_mm_per_min * 1000.0).round() / 1000.0);
            }
            // Never rapid with the beam on, whatever the mode or controller.
            let _ = writeln!(self.out, "M5");
        }
        Ok(())
    }

    /// Turns the tool off, returns to the origin and gives the program.
    pub fn finish(mut self) -> String {
        if self.bare {
            self.out.push_str("G0 X0 Y0\n");
            return self.out;
        }
        let _ = writeln!(self.out, "M5 ; tool off");
        let _ = writeln!(self.out, "G0 X0 Y0 ; return to origin");
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(settings: Settings, points: &[(f64, f64)], closed: bool) -> String {
        let mut w = Writer::new(settings, "test").unwrap();
        w.contour(points, closed, 500).unwrap();
        w.finish()
    }

    #[test]
    fn a_line_is_a_whole_program() {
        assert_eq!(
            program(Settings::default(), &[(0.0, 0.0), (10.0, 0.0)], false),
            "; test\nG21 ; millimetres\nG90 ; absolute coordinates\nG17 ; XY plane\nM4 S0 ; laser off until the first cut\n\
             G0 X0.0000 Y0.0000\nM4 S500\nG1 X10.0000 Y0.0000 F1000\nM5\nM5 ; tool off\nG0 X0 Y0 ; return to origin\n"
        );
    }

    #[test]
    fn closed_loops_return_to_start_and_a_point_is_a_dot() {
        let g = program(Settings::default(), &[(1.0, 2.0), (10.0, 2.0), (10.0, 10.0)], true);
        assert!(g.contains("G1 X10.0000 Y10.0000 F1000\nG1 X1.0000 Y2.0000 F1000\nM5\n"), "{g}");
        // Closed or not, a dot is the same in-and-out move.
        for closed in [false, true] {
            let dot = program(Settings::default(), &[(5.0, 5.0)], closed);
            assert!(dot.contains("G0 X5.0000 Y5.0000\nM4 S500\nG1 X5.0010 Y5.0000 F1000\nG1 X5.0000 Y5.0000 F1000\nM5\n"), "{dot}");
        }
        assert!(!program(Settings::default(), &[], true).contains("G1 "));
    }

    #[test]
    fn rapids_never_run_with_the_beam_on() {
        let settings = Settings { passes: 3, mode: LaserMode::Constant, ..Settings::default() };
        let mut w = Writer::new(settings, "test").unwrap();
        w.contour(&[(0.0, 0.0), (10.0, 0.0)], false, 500).unwrap();
        w.contour(&[(20.0, 0.0), (30.0, 0.0), (30.0, 5.0)], true, 200).unwrap();
        let g = w.finish();
        let mut lit = false;
        for line in g.lines().filter(|l| !l.starts_with(';')) {
            if line.starts_with("M3 S0") || line.starts_with("M5") {
                lit = false;
            } else if line.starts_with("M3 ") {
                lit = true;
            } else if line.starts_with("G0") {
                assert!(!lit, "rapid with the beam on:\n{g}");
            }
        }
        assert!(!g.contains("M4"), "{g}");
        assert_eq!(g.matches("G1 X10.0000 Y0.0000 F1000").count(), 3, "{g}");
        assert_eq!(g.matches("M3 S500 ; pass ").count(), 3, "{g}");
    }

    #[test]
    fn ramp_power_climbs_to_the_set_power() {
        let ramp = Settings { passes: 4, ramp_power: true, ..Settings::default() };
        assert_eq!([1, 2, 3, 4].map(|pass| ramp.power_for_pass(800, pass)), [200, 400, 600, 800]);
        let flat = Settings { passes: 4, ..Settings::default() };
        assert_eq!(flat.power_for_pass(800, 1), 800);
        // A ramped pass never drops to S0, which would trace with the beam off.
        let fine = Settings { passes: 10, ramp_power: true, ..Settings::default() };
        assert_eq!(fine.power_for_pass(3, 1), 1);
        let g = program(ramp, &[(0.0, 0.0), (1.0, 0.0)], false);
        assert!(g.contains("M4 S125 ; pass 1 of 4") && g.contains("M4 S500 ; pass 4 of 4"), "{g}");
    }

    #[test]
    fn bad_settings_powers_and_vertices_are_errors() {
        for feed in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(matches!(Writer::new(Settings { feed_mm_per_min: feed, ..Settings::default() }, ""), Err(Error::Settings(_))));
        }
        for passes in [0, MAX_PASSES + 1] {
            assert!(matches!(Settings { passes, ..Settings::default() }.validate(), Err(Error::Settings(_))));
        }
        assert_eq!(check_power("engrave power", 1001), Err(Error::Settings("engrave power 1001 is outside the 0–1000 range".into())));
        let mut w = Writer::new(Settings::default(), "").unwrap();
        assert!(matches!(w.contour(&[(0.0, 0.0), (1.0, 1.0)], false, MAX_POWER + 1), Err(Error::Settings(_))));
        assert_eq!(w.contour(&[(0.0, 0.0), (f64::NAN, 1.0)], false, 500), Err(Error::NotFinite));
        // Nothing of a refused contour is written, not even its travel move.
        assert!(!w.finish().contains("G0 X0.0000"));
    }

    #[test]
    fn grbl_real_time_bytes_never_reach_the_program() {
        let mut w = Writer::new(Settings::default(), "stop! now? ~ café").unwrap();
        w.comment("\u{18}reset");
        let g = w.finish();
        assert!(g.starts_with("; stop_ now_ _ caf_\n"), "{g}");
        assert!(g.is_ascii() && !g.contains(['!', '?', '~', '\u{18}']), "{g}");
    }

    #[test]
    fn a_fractional_feed_is_written_not_rounded_to_zero() {
        let slow = Settings { feed_mm_per_min: 0.25, ..Settings::default() };
        assert!(program(slow, &[(0.0, 0.0), (1.0, 0.0)], false).contains(" F0.25\n"));
        assert!(program(Settings::default(), &[(0.0, 0.0), (1.0, 0.0)], false).contains(" F1000\n"));
        assert!(matches!(Writer::new(Settings { feed_mm_per_min: 0.0004, ..Settings::default() }, ""), Err(Error::Settings(_))));
    }

    #[test]
    fn a_line_break_in_a_comment_cannot_start_a_command() {
        let mut w = Writer::new(Settings::default(), "banner").unwrap();
        w.comment("layer x\nM3 S1000\r\nG1 X9");
        let g = w.finish();
        assert!(g.contains("; layer x M3 S1000  G1 X9\n"), "{g}");
        assert!(g.lines().all(|l| !l.starts_with("M3")));
    }

    #[test]
    fn a_bare_program_is_commands_only() {
        let mut w = Writer::bare(Settings { feed_mm_per_min: 1200.0, ..Settings::default() }).unwrap();
        w.comment("never written");
        w.contour(&[(0.0, 5.0), (10.0, 5.0), (10.0, 0.0)], false, 800).unwrap();
        w.set_feed(0.25).unwrap();
        w.contour(&[(-0.0001, 1.0 / 3.0), (2.5, 1e-9)], false, 0).unwrap();
        assert_eq!(
            w.finish(),
            "G21\nG90\nG17\nM5\nG0 X0 Y5\nM4 S800\nG1 X10 Y5 F1200\nG1 X10 Y0 F1200\nM5\n\
             G0 X0 Y0.333\nM4 S0\nG1 X2.5 Y0 F0.25\nM5\nG0 X0 Y0\n"
        );
        // Passes repeat the contour without a comment; the mode still picks the on command.
        let mut w = Writer::bare(Settings { passes: 2, mode: LaserMode::Constant, ..Settings::default() }).unwrap();
        w.contour(&[(0.0, 0.0), (1.0, 0.0)], false, 5).unwrap();
        assert_eq!(w.finish(), "G21\nG90\nG17\nM5\nG0 X0 Y0\nM3 S5\nG1 X1 Y0 F1000\nM5\nG0 X0 Y0\nM3 S5\nG1 X1 Y0 F1000\nM5\nG0 X0 Y0\n");
        assert!(matches!(Writer::bare(Settings { passes: 0, ..Settings::default() }), Err(Error::Settings(_))));
    }

    #[test]
    fn set_feed_refuses_a_bad_rate_and_keeps_the_old_one() {
        let mut w = Writer::new(Settings::default(), "").unwrap();
        for feed in [0.0, -1.0, f64::NAN, f64::INFINITY, 0.0004] {
            assert!(matches!(w.set_feed(feed), Err(Error::Settings(_))));
        }
        w.contour(&[(0.0, 0.0), (1.0, 0.0)], false, 1).unwrap();
        w.set_feed(300.0).unwrap();
        w.contour(&[(0.0, 0.0), (2.0, 0.0)], false, 1).unwrap();
        let g = w.finish();
        assert!(g.contains("G1 X1.0000 Y0.0000 F1000\n") && g.contains("G1 X2.0000 Y0.0000 F300\n"), "{g}");
    }

    #[test]
    fn laser_modes_parse_their_spellings() {
        assert_eq!(LaserMode::parse(" M3 "), Some(LaserMode::Constant));
        assert_eq!(LaserMode::parse("Dynamic"), Some(LaserMode::Dynamic));
        assert_eq!(LaserMode::parse("m5"), None);
    }
}
