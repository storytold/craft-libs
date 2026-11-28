//! HP-GL import without a document model: VectorCraft's reader (`crates/cad/src/hpgl.rs`), which
//! plays a plot file into the [`Shape`]s it paints, in plotter units (40 to the millimetre, y up),
//! turned by RO. Mapping them into a document is the caller's.

use super::lex::{ETX, Lexer};
use super::plot::{Plot, Shape};
use super::{FIRST, IGNORED, PLAYED};

/// What a plot file paints, and what the reader left out.
#[derive(Debug, Default)]
pub struct Drawing {
    /// In paint order.
    pub shapes: Vec<Shape>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    #[error("not an HP-GL file")]
    NotHpgl,
}

/// Does `bytes` start like an HP-GL file? After any printer escapes and PJL lines (however long),
/// it must hold nothing but instructions the reader knows, the first one of those a plot file
/// starts with, up to the eighth instruction; none starting more than 4 KB past the first is
/// checked, though the text of one starting before may reach further. Their parameters must be
/// numbers, but for a comment's quoted string and the text, characters and encoded data of LB, BL,
/// WD, DT, SM and PE; IN and DF give back the default label terminator. An instruction the reader doesn't know counts when its numbers end with a
/// semicolon.
pub fn is_hpgl(bytes: &[u8]) -> bool {
    holds_instructions(bytes, true)
}

/// Read an HP-GL file. It may start with any instruction the reader knows, or one of another set
/// that a semicolon ends; the rest of [`is_hpgl`]'s check applies, and at least one instruction the
/// reader knows must be among those it checks.
pub fn read(bytes: &[u8]) -> Result<Drawing, ReadError> {
    if !holds_instructions(bytes, false) {
        return Err(ReadError::NotHpgl);
    }
    let mut lex = Lexer::new(bytes);
    let mut plot = Plot::new();
    while !plot.full
        && let Some(op) = lex.op(false)
    {
        plot.op(op, &mut lex);
    }
    let plot = plot.finish(lex.raster);
    Ok(Drawing { shapes: plot.shapes, warnings: plot.warnings })
}

/// Does `bytes` hold HP-GL instructions as [`is_hpgl`] checks? When `strict` is false, any of them
/// may come first: a file picked by its extension may start with one a plot file seldom starts
/// with (`SI`, `DI`). Either way, at least one of the instructions checked must be one the reader
/// knows.
fn holds_instructions(bytes: &[u8], strict: bool) -> bool {
    let mut lex = Lexer::new(bytes);
    let mut terminator = ETX;
    let mut first = 0;
    let mut any_known = false;
    for n in 0..8 {
        if n > 0 && lex.pos() > first + 4096 {
            return any_known;
        }
        let Some(op) = lex.op(true) else {
            return n > 0 && any_known && lex.done();
        };
        if n == 0 {
            first = lex.pos();
        }
        if strict && n == 0 && !FIRST.contains(&op) {
            return false;
        }
        let known = PLAYED.contains(&op) || IGNORED.contains(&op);
        any_known |= known;
        match &op {
            b"CO" => {
                if !lex.comment() {
                    return false;
                }
            }
            b"LB" | b"BL" | b"WD" => lex.label(terminator),
            b"DT" => {
                terminator = lex.char().unwrap_or(ETX);
                lex.params();
            }
            b"SM" => {
                // The symbol character is read so it isn't taken for an instruction.
                let _ = lex.char();
            }
            b"PE" => lex.skip_pe(),
            // IN and DF give back the default label terminator, as playback does.
            b"IN" | b"DF" => {
                terminator = ETX;
                lex.closed_params();
            }
            // Numbers only: a quoted string belongs to CO alone.
            _ if known => {
                lex.closed_params();
            }
            // An instruction the reader doesn't know (of an older or a device's own set) counts
            // when numbers and a semicolon follow it.
            _ => {
                if !lex.closed_params() {
                    return false;
                }
            }
        }
    }
    any_known
}
