//! Houston Instrument DMPL, as sign cutters take it.
//!
//! The stream is `;:` and `EC1` (0.001 in resolution), then per path `Ux,y` to its start and one
//! `Dx,y,…` through the rest, then `U0,0` and `@`. One command per line. The origin is
//! bottom-left and y points up, so the design is flipped about its bottom edge. Tool holders,
//! force and speed are not addressed.
//!
//! This framing was not checked against a published manual: treat it as experimental.

use crate::{Command, Error, Framing, Job, Program, y_up};

/// Device units per mm under `EC1`: 0.001 in.
pub const UNITS_PER_MM: f64 = 1.0 / 0.0254;

/// Encodes `job` as DMPL. A job with no geometry gives an empty program.
pub fn encode(job: &Job) -> Result<Program, Error> {
    let Some(prepared) = job.prepare()? else {
        return Ok(Program::default());
    };
    let p = y_up(job, prepared.bbox_mm.2, UNITS_PER_MM);
    let mut c = vec![Command::Raw(";:".to_string()), Command::Raw("EC1".to_string())];
    for path in prepared.operations.iter().flatten() {
        let Some((first, rest)) = path.split_first() else {
            continue;
        };
        let (x, y) = p(first);
        c.push(Command::Raw(format!("U{x},{y}")));
        let pts: Vec<String> = rest
            .iter()
            .map(|q| {
                let (x, y) = p(q);
                format!("{x},{y}")
            })
            .collect();
        c.push(Command::Raw(format!("D{}", pts.join(","))));
    }
    c.push(Command::Raw("U0,0".to_string()));
    c.push(Command::Raw("@".to_string()));
    Ok(Program { commands: c, bbox_mm: Some(prepared.bbox_mm), framing: Framing::Newline, registration_wait: None })
}
