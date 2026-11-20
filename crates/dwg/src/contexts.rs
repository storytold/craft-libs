//! Annotation scale contexts in the bridge's DXF, between AutoCAD's layout and the one acadrust
//! reads and writes.
//!
//! - Block reference and text contexts (`ACDB_BLKREFOBJECTCONTEXTDATA_CLASS`,
//!   `ACDB_TEXTOBJECTCONTEXTDATA_CLASS`): AutoCAD's DXF has their fields straight after the
//!   context's scale (340), with the rotation (50) in degrees. acadrust 0.6.6 reads them only after
//!   an `AcDbBlkRefObjectContextData` or `AcDbTextObjectContextData` marker, with the rotation in
//!   radians (as DWG holds it), and without the marker reads zeros: an annotative block reference
//!   at scale 0 draws nothing in AutoCAD.
//! - Dimension contexts (`ACDB_*DIMOBJECTCONTEXTDATA_CLASS`): AutoCAD names the geometry block in
//!   group 2, where acadrust reads a BLOCK_RECORD handle.
//! - Multileader contexts (`ACDB_MLEADEROBJECTCONTEXTDATA_CLASS`): AutoCAD has the multileader's
//!   context groups straight after the scale; acadrust reads them only after a `300 CONTEXT_DATA{`
//!   group, and writes them between that and `301 }`, after an `AcDbMLeaderObjectContextData`
//!   marker.
//!
//! The text is read once as (code, value) line pairs, without a line index: the output is the only
//! allocation the size of the input. Line endings (`\n` or `\r\n`) are kept.

use std::collections::HashMap;

/// Block reference and text contexts, and the marker acadrust reads their fields after.
const MARKED: [(&str, &str); 2] =
    [("ACDB_BLKREFOBJECTCONTEXTDATA_CLASS", "AcDbBlkRefObjectContextData"), ("ACDB_TEXTOBJECTCONTEXTDATA_CLASS", "AcDbTextObjectContextData")];

/// Multileader contexts.
const MLEADER: &str = "ACDB_MLEADEROBJECTCONTEXTDATA_CLASS";

/// What every dimension context's record name ends with.
const DIM: &str = "DIMOBJECTCONTEXTDATA_CLASS";

/// Most block records whose names and handles are kept for dimension contexts.
const MAX_RECORDS: usize = 1_000_000;

/// The converted text would be longer than the limit it was given.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct TooLarge;

/// The DXF with its annotation scale contexts in acadrust's layout (`to_acadrust`), or back in
/// AutoCAD's, at most `limit` bytes long. `Ok(None)` when nothing changes: text that isn't UTF-8,
/// or has no such context. The conversion stops at the limit: a dimension context's block name
/// can be much longer than the handle it replaces.
pub(super) fn contexts(dxf: &[u8], to_acadrust: bool, limit: usize) -> Result<Option<Vec<u8>>, TooLarge> {
    let Ok(text) = std::str::from_utf8(dxf) else { return Ok(None) };
    let marked = MARKED.iter().any(|(kind, _)| text.contains(kind));
    let dims = text.contains(DIM);
    if !(marked || dims || text.contains(MLEADER)) {
        return Ok(None);
    }
    let records = if dims { Records::read(text) } else { Records::default() };
    let mut pass = Pass::new(to_acadrust, &records, dxf.len().min(limit), limit);
    let mut lines = text.split('\n');
    while let Some(code_line) = lines.next() {
        if pass.out.exceeded {
            break;
        }
        let Some(value_line) = lines.next() else {
            pass.out.line(code_line);
            break;
        };
        pass.pair(code_line, value_line);
    }
    if pass.out.exceeded {
        return Err(TooLarge);
    }
    Ok(Some(pass.out.buf))
}

/// The output text: lines joined by `\n`, as they were split, at most `limit` bytes; past it,
/// nothing more is written and `exceeded` is set.
struct Out {
    buf: Vec<u8>,
    started: bool,
    limit: usize,
    exceeded: bool,
}

impl Out {
    fn line(&mut self, line: &str) {
        self.push(line, false);
    }

    /// A line of ours, ending in `\r` when `like` (the input line it stands by) does.
    fn line_like(&mut self, line: &str, like: &str) {
        self.push(line, like.ends_with('\r'));
    }

    fn push(&mut self, line: &str, cr: bool) {
        let size = self.buf.len().saturating_add(usize::from(self.started)).saturating_add(line.len()).saturating_add(usize::from(cr));
        if self.exceeded || size > self.limit {
            self.exceeded = true;
            return;
        }
        if self.started {
            self.buf.push(b'\n');
        }
        self.started = true;
        self.buf.extend_from_slice(line.as_bytes());
        if cr {
            self.buf.push(b'\r');
        }
    }
}

/// The drawing's block records, by name (upper case) and by handle (upper case), for dimension
/// contexts. A name or handle given twice keeps its first record.
#[derive(Default)]
struct Records {
    by_name: HashMap<String, String>,
    by_handle: HashMap<String, String>,
}

impl Records {
    fn read(text: &str) -> Records {
        let mut records = Records::default();
        let (mut in_record, mut handle) = (false, None::<String>);
        let mut lines = text.split('\n');
        while let (Some(code), Some(value)) = (lines.next(), lines.next()) {
            let value = value.trim();
            match code.trim() {
                "0" => (in_record, handle) = (value == "BLOCK_RECORD", None),
                "5" if in_record => handle = Some(value.to_ascii_uppercase()),
                "2" if in_record => {
                    if let Some(h) = handle.take()
                        && records.by_name.len() < MAX_RECORDS
                        && records.by_handle.len() < MAX_RECORDS
                    {
                        records.by_name.entry(value.to_ascii_uppercase()).or_insert_with(|| h.clone());
                        records.by_handle.entry(h).or_insert_with(|| value.to_string());
                    }
                }
                _ => {}
            }
        }
        records
    }

    /// A dimension context's group 2 the other way: a block name as its record's handle (to
    /// acadrust), or a handle as its record's name (back).
    fn swap(&self, value: &str, to_acadrust: bool) -> Option<&String> {
        let key = value.trim().to_ascii_uppercase();
        if to_acadrust { self.by_name.get(&key) } else { self.by_handle.get(&key) }
    }
}

/// One pass over the text, and where it is in the record being read.
struct Pass<'a> {
    to_acadrust: bool,
    records: &'a Records,
    out: Out,
    /// In a block reference or text context: its marker, whether its scale (340) was passed,
    /// whether its marker was (read or written), and whether the input had it (the record is in
    /// acadrust's layout already, its rotation in radians).
    marker: Option<&'static str>,
    scaled: bool,
    marked: bool,
    input_marked: bool,
    /// In a dimension context.
    dim: bool,
    /// In a multileader context: whether its scale was passed and its body opened.
    mleader: bool,
    mleader_scaled: bool,
    mleader_open: bool,
}

impl<'a> Pass<'a> {
    fn new(to_acadrust: bool, records: &'a Records, size: usize, limit: usize) -> Pass<'a> {
        Pass {
            to_acadrust,
            records,
            out: Out { buf: Vec::with_capacity(size.saturating_add(size / 64).min(limit)), started: false, limit, exceeded: false },
            marker: None,
            scaled: false,
            marked: false,
            input_marked: false,
            dim: false,
            mleader: false,
            mleader_scaled: false,
            mleader_open: false,
        }
    }

    fn pair(&mut self, code_line: &str, value_line: &str) {
        let code = code_line.trim();
        let value = value_line.strip_suffix('\r').unwrap_or(value_line);
        if code == "0" {
            self.record(code_line, value_line, value.trim());
        }
        if self.mleader && self.mleader_group(code_line, value_line, code, value.trim()) {
            return;
        }
        let mut value = value.to_string();
        if let Some(marker) = self.marker {
            if self.marked_group(code_line, value_line, code, &value, marker) {
                return;
            }
            if let Some(v) = self.rotation(code, &value) {
                value = v;
            }
        }
        if self.dim
            && code == "2"
            && let Some(v) = self.records.swap(&value, self.to_acadrust)
        {
            value.clone_from(v);
        }
        self.out.line(code_line);
        self.out.line_like(&value, value_line);
    }

    /// A new record starts (`0` and its type): a multileader context body left open closes first,
    /// as acadrust writes it.
    fn record(&mut self, code_line: &str, value_line: &str, kind: &str) {
        if self.mleader_open {
            self.out.line_like("301", code_line);
            self.out.line_like("}", value_line);
        }
        self.marker = MARKED.iter().find(|(k, _)| *k == kind).map(|(_, m)| *m);
        (self.scaled, self.marked, self.input_marked) = (false, false, false);
        self.dim = kind.starts_with("ACDB_") && kind.ends_with(DIM);
        (self.mleader, self.mleader_scaled, self.mleader_open) = (kind == MLEADER, false, false);
    }

    /// A group of a multileader context. Back to AutoCAD's layout, acadrust's marker and braces
    /// go (true: the group is left out). To acadrust's, the body opens after the scale.
    fn mleader_group(&mut self, code_line: &str, value_line: &str, code: &str, value: &str) -> bool {
        let theirs = (code == "100" && value == "AcDbMLeaderObjectContextData")
            || (code == "300" && value.starts_with("CONTEXT_DATA"))
            || (code == "301" && value == "}");
        if theirs && !self.to_acadrust {
            return true;
        }
        if self.to_acadrust && self.mleader_scaled && !self.mleader_open {
            self.out.line_like("300", code_line);
            self.out.line_like("CONTEXT_DATA{", value_line);
            self.mleader_open = true;
        }
        // The first 340 is the scale (a later one is the text style).
        if code == "340" {
            self.mleader_scaled = true;
        }
        false
    }

    /// A group of a block reference or text context. Its marker is kept to acadrust and left out
    /// back to AutoCAD's layout (true: the group is handled); to acadrust, the marker goes before
    /// the first field after the scale.
    fn marked_group(&mut self, code_line: &str, value_line: &str, code: &str, value: &str, marker: &str) -> bool {
        if code == "100" && value.trim() == marker {
            (self.marked, self.input_marked) = (true, true);
            if self.to_acadrust {
                self.out.line(code_line);
                self.out.line(value_line);
            }
            return true;
        }
        if self.to_acadrust && self.scaled && !self.marked && code != "100" {
            self.out.line_like("100", code_line);
            self.out.line_like(marker, value_line);
            self.marked = true;
        }
        if code == "340" {
            self.scaled = true;
        }
        false
    }

    /// A context's rotation (50, after its scale) in radians (to acadrust) or degrees (back). A
    /// record that came in acadrust's layout (with its marker) has radians already.
    fn rotation(&self, code: &str, value: &str) -> Option<String> {
        if code != "50" || !self.scaled || (self.to_acadrust && self.input_marked) {
            return None;
        }
        let a = value.trim().parse::<f64>().ok().filter(|a| a.is_finite())?;
        Some(if self.to_acadrust { a.to_radians() } else { a.to_degrees() }.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to(text: &str, to_acadrust: bool) -> String {
        String::from_utf8(contexts(text.as_bytes(), to_acadrust, usize::MAX).unwrap().unwrap()).unwrap()
    }

    /// A block reference context: the marker goes in after the scale and the rotation becomes
    /// radians; back, the marker goes and the rotation is degrees again, with the line endings kept.
    #[test]
    fn a_block_reference_context_gets_its_marker_and_radians() {
        for eol in ["\n", "\r\n"] {
            let autocad = ["  0", "ACDB_BLKREFOBJECTCONTEXTDATA_CLASS", "  5", "90", "340", "51", " 50", "90", " 10", "1", "  0", "ENDSEC"].join(eol);
            let ours = to(&autocad, true);
            let lines: Vec<&str> = ours.lines().map(str::trim).collect();
            let scale = lines.iter().position(|l| *l == "340").unwrap();
            assert_eq!(lines.get(scale + 2..scale + 4), Some(&["100", "AcDbBlkRefObjectContextData"][..]), "{ours}");
            let rotation: f64 = lines.get(scale + 5).unwrap().parse().unwrap();
            assert!((rotation - std::f64::consts::FRAC_PI_2).abs() < 1e-12, "{ours}");
            // Every inserted line ends as the input's lines do.
            if eol == "\r\n" {
                assert_eq!(ours.matches('\n').count(), ours.matches("\r\n").count(), "{ours:?}");
            } else {
                assert!(!ours.contains('\r'), "{ours:?}");
            }
            assert_eq!(to(&ours, false), autocad);
        }
    }

    /// A rotation before the scale isn't the context's. A record already in acadrust's layout
    /// (its marker there) keeps its marker once and its rotation, which is radians already.
    #[test]
    fn only_a_rotation_after_the_scale_changes_and_a_marked_record_is_left_alone() {
        let text = "  0\nACDB_TEXTOBJECTCONTEXTDATA_CLASS\n 50\n90\n340\n51\n100\nAcDbTextObjectContextData\n 50\n1.5707963267948966\n  0\nEOF";
        assert_eq!(to(text, true), text);
    }

    /// The conversion stops at its limit: here a dimension context's block name is far longer
    /// than the handle it replaces.
    #[test]
    fn the_output_stops_at_its_limit() {
        let name = "N".repeat(1000);
        let back = format!("  0\nBLOCK_RECORD\n  5\n1F\n  2\n{name}\n  0\nACDB_ALDIMOBJECTCONTEXTDATA_CLASS\n  2\n1F\n  0\nEOF");
        let fits = contexts(back.as_bytes(), false, back.len() + 1000).unwrap().unwrap();
        assert!(String::from_utf8(fits).unwrap().ends_with(&format!("  2\n{name}\n  0\nEOF")));
        assert_eq!(contexts(back.as_bytes(), false, back.len() + 10), Err(TooLarge));
        assert_eq!(contexts(back.as_bytes(), false, 0), Err(TooLarge));
    }

    /// A dimension context names its geometry block (AutoCAD) or that block's record handle
    /// (acadrust).
    #[test]
    fn a_dimension_context_swaps_its_block_name_and_handle() {
        let autocad = "  0\nBLOCK_RECORD\n  5\n1F\n  2\n*D3\n  0\nACDB_ALDIMOBJECTCONTEXTDATA_CLASS\n  2\n*d3\n  0\nEOF";
        let ours = to(autocad, true);
        assert!(ours.contains("ACDB_ALDIMOBJECTCONTEXTDATA_CLASS\n  2\n1F\n"), "{ours}");
        let back = to(&ours, false);
        assert!(back.contains("ACDB_ALDIMOBJECTCONTEXTDATA_CLASS\n  2\n*D3\n"), "{back}");
        // The record's own group 2 stays its name.
        assert!(back.starts_with("  0\nBLOCK_RECORD\n  5\n1F\n  2\n*D3\n"), "{back}");
    }

    /// A multileader context read by acadrust has its body between `300 CONTEXT_DATA{` and
    /// `301 }`, closed before the next record; back as AutoCAD writes it, both go.
    #[test]
    fn a_multileader_context_body_is_closed() {
        let autocad = "  0\nACDB_MLEADEROBJECTCONTEXTDATA_CLASS\n  5\n90\n340\n51\n 40\n1\n  0\nENDSEC\n";
        let ours = to(autocad, true);
        let lines: Vec<&str> = ours.lines().map(str::trim).collect();
        let open = lines.iter().position(|l| *l == "CONTEXT_DATA{").unwrap();
        let close = lines.iter().position(|l| *l == "}").unwrap();
        let next = lines.iter().position(|l| *l == "ENDSEC").unwrap();
        assert!(open < close && close < next && lines.get(close - 1) == Some(&"301"), "{ours}");
        assert_eq!(to(&ours, false), autocad);
    }

    /// Text without a context, or that isn't UTF-8, is left as it is.
    #[test]
    fn other_text_is_left_alone() {
        assert_eq!(contexts(b"  0\nSECTION\n  2\nENTITIES\n  0\nENDSEC\n  0\nEOF\n", true, usize::MAX), Ok(None));
        assert_eq!(contexts(b"  0\nACDB_BLKREFOBJECTCONTEXTDATA_CLASS\n  1\n\xff\n", true, usize::MAX), Ok(None));
    }

    /// Cut anywhere, a context is converted without a panic, and an odd last line is kept.
    #[test]
    fn hostile_text_does_not_panic() {
        let text = "  0\nBLOCK_RECORD\n  5\n1F\n  2\n*D3\n  0\nACDB_BLKREFOBJECTCONTEXTDATA_CLASS\n340\n51\n 50\nnan\n 50\n1e400\n  0\nACDB_DIMOBJECTCONTEXTDATA_CLASS\n  2\n*D3\n  0\nACDB_MLEADEROBJECTCONTEXTDATA_CLASS\n340\n1\n301\n}\n";
        for n in 0..=text.len() {
            if let Some(cut) = text.get(..n) {
                for to_acadrust in [true, false] {
                    for limit in [0, n / 2, usize::MAX] {
                        let _ = contexts(cut.as_bytes(), to_acadrust, limit);
                    }
                }
            }
        }
        assert!(to("  0\nACDB_BLKREFOBJECTCONTEXTDATA_CLASS\n340", true).ends_with("\n340"));
    }
}
