//! HP-GL, the subset every cutter that speaks it understands (`IN`, `SP`, `PU`, `PD`, which is
//! also valid HP-GL/2): Roland, USCutter and many unbranded sign cutters, and Graphtec machines in
//! HP-GL mode.
//!
//! The stream is `IN;`, then per operation `SP<holder>;` and per path `PUx,y;` to its start and
//! one `PDx,y,…;` through the rest, then `PU;` and `SP0;`. One command per line. The origin is
//! bottom-left and y points up, so the design is flipped about its bottom edge. Force and speed
//! are set on the machine's panel.

/// Byte-oriented HP-GL/2 lexer, including PCL/PJL framing and encoded polylines.
pub mod lex;
/// HP-GL import: VectorCraft's reader, without its document types.
pub mod read;

use crate::{Command, Error, Framing, Job, Program, y_up};

pub mod plot;
pub mod write;

pub use read::{Drawing, ReadError, is_hpgl};

/// Instructions the reader knows and draws nothing for (their parameters are read and dropped).
const IGNORED: &[[u8; 2]] = &[
    *b"AC", *b"AD", *b"AF", *b"AH", *b"AP", *b"AS", *b"CA", *b"CF", *b"CM", *b"CP", *b"CS", *b"CT", *b"CV", *b"DC", *b"DI", *b"DL", *b"DP", *b"DR",
    *b"DS", *b"DV", *b"EC", *b"ES", *b"FI", *b"FN", *b"FS", *b"GM", *b"IM", *b"IV", *b"KY", *b"LM", *b"LO", *b"MC", *b"MG", *b"MT", *b"NR", *b"OA",
    *b"OC", *b"OD", *b"OE", *b"OF", *b"OG", *b"OH", *b"OI", *b"OL", *b"OP", *b"OS", *b"OT", *b"OW", *b"PB", *b"PP", *b"PS", *b"PT", *b"QL", *b"RF",
    *b"RP", *b"SA", *b"SB", *b"SD", *b"SG", *b"SI", *b"SL", *b"SR", *b"SS", *b"ST", *b"SV", *b"TD", *b"TL", *b"TR", *b"UC", *b"VA", *b"VN", *b"VS",
    *b"XT", *b"YT",
];

/// Instructions the reader plays.
const PLAYED: &[[u8; 2]] = &[
    *b"AA", *b"AR", *b"AT", *b"BL", *b"BP", *b"BR", *b"BZ", *b"CI", *b"CO", *b"CR", *b"DF", *b"DT", *b"EA", *b"EP", *b"ER", *b"EW", *b"FP", *b"FR",
    *b"FT", *b"IN", *b"IP", *b"IR", *b"IW", *b"LA", *b"LB", *b"LT", *b"NP", *b"PA", *b"PC", *b"PD", *b"PE", *b"PG", *b"PM", *b"PR", *b"PU", *b"PW",
    *b"RA", *b"RO", *b"RR", *b"RT", *b"SC", *b"SM", *b"SP", *b"UL", *b"WD", *b"WG", *b"WU",
];

/// Instructions a plot file starts with.
const FIRST: &[[u8; 2]] = &[
    *b"IN", *b"DF", *b"BP", *b"CO", *b"PS", *b"SP", *b"PU", *b"PD", *b"PA", *b"PR", *b"IP", *b"IR", *b"SC", *b"NP", *b"PC", *b"PW", *b"WU", *b"RO",
    *b"QL", *b"MC", *b"PP", *b"TR", *b"CR", *b"LT", *b"LA", *b"FT", *b"VS", *b"FS", *b"AC", *b"PG", *b"PE", *b"DT",
];

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
