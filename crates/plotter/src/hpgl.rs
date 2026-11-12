//! HP-GL, the subset every cutter that speaks it understands (`IN`, `SP`, `PU`, `PD`, which is
//! also valid HP-GL/2): Roland, USCutter and many unbranded sign cutters, and Graphtec machines in
//! HP-GL mode.
//!
//! The stream is `IN;`, then per operation `SP<holder>;` and per path `PUx,y;` to its start and
//! one `PDx,y,…;` through the rest, then `PU;` and `SP0;`. One command per line. The origin is
//! bottom-left and y points up, so the design is flipped about its bottom edge. Force and speed
//! are set on the machine's panel.

use crate::{Command, Error, Framing, Job, Program, y_up};

/// Plotter units per mm (1 plu = 0.025 mm).
pub const UNITS_PER_MM: f64 = 40.0;

/// Encodes `job` as HP-GL. A job with no geometry gives an empty program.
pub fn encode(job: &Job) -> Result<Program, Error> {
    let Some(prepared) = job.prepare()? else {
        return Ok(Program::default());
    };
    let p = y_up(job, prepared.bbox_mm.2, UNITS_PER_MM);
    let mut c = vec![Command::Raw("IN;".to_string())];
    for (op, paths) in job.operations.iter().zip(&prepared.operations) {
        c.push(Command::Raw(format!("SP{};", op.holder)));
        for path in paths {
            let Some((first, rest)) = path.split_first() else {
                continue;
            };
            let (x, y) = p(first);
            c.push(Command::Raw(format!("PU{x},{y};")));
            let pts: Vec<String> = rest
                .iter()
                .map(|q| {
                    let (x, y) = p(q);
                    format!("{x},{y}")
                })
                .collect();
            c.push(Command::Raw(format!("PD{};", pts.join(","))));
        }
    }
    c.push(Command::Raw("PU;".to_string()));
    c.push(Command::Raw("SP0;".to_string()));
    Ok(Program { commands: c, bbox_mm: Some(prepared.bbox_mm), framing: Framing::Newline, registration_wait: None })
}
