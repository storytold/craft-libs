//! Graphtec GP-GL. This module is the plain language as Graphtec's own cutters take it; the
//! dialect of the Silhouette machines, with its setup preamble, registration marks and device
//! queries, is [`silhouette`].
//!
//! The plain stream is `!<speed>` when an operation sets one, then per path `M<down>,<right>` to
//! its start and `D<down>,<right>` to each further point, then `M<below>,0` to park below the cut.
//! Every command ends with ETX. Coordinates are feed axis first, in steps of 0.1 mm, which must be
//! the GP-GL step size set on the machine's panel. Force is set on the panel.

pub mod silhouette;

use crate::{Command, Error, Framing, Job, Program, units};

/// Command terminator: every GP-GL command is followed by ETX (0x03).
pub const ETX: u8 = 0x03;

/// Plain GP-GL steps per mm (0.1 mm steps).
pub const UNITS_PER_MM: f64 = 10.0;

/// Encodes `job` as plain GP-GL. A job with no geometry gives an empty program.
pub fn encode(job: &Job) -> Result<Program, Error> {
    let Some(prepared) = job.prepare()? else {
        return Ok(Program::default());
    };
    let (dm, rm) = (job.origin_down_mm, job.origin_right_mm);
    let g = |&(x, y): &(f64, f64)| (units(y + dm, UNITS_PER_MM), units(x + rm, UNITS_PER_MM));
    let mut c = Vec::new();
    for (op, paths) in job.operations.iter().zip(&prepared.operations) {
        if let Some(s) = op.settings.speed {
            c.push(Command::Raw(format!("!{s}")));
        }
        for path in paths {
            let Some((first, rest)) = path.split_first() else {
                continue;
            };
            let (d, r) = g(first);
            c.push(Command::Raw(format!("M{d},{r}")));
            for q in rest {
                let (d, r) = g(q);
                c.push(Command::Raw(format!("D{d},{r}")));
            }
        }
    }
    // Park below the cut, head up.
    c.push(Command::Raw(format!("M{},0", units(prepared.bbox_mm.2 + job.end_feed_mm.max(0.0), UNITS_PER_MM))));
    Ok(Program { commands: c, bbox_mm: Some(prepared.bbox_mm), framing: Framing::Etx, registration_wait: None })
}
