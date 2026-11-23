//! Legacy form fields ([MS-DOC] §2.9.78, `FFData`): the definition of a `FORMTEXT`,
//! `FORMCHECKBOX` or `FORMDROPDOWN` field, which a `.doc` keeps in its Data stream. The field's
//! code holds, after its instruction and before its separator, a U+0001 character whose CHPX has
//! `sprmCFData` set and whose `sprmCPicLocation` is the offset of the definition (Word writes it
//! behind a picture header, see [`definition`]). It is returned as the `w:ffData` markup the `.docx` reader and writer use
//! (ECMA-376 Part 1 §17.16.17), so the field survives a conversion.
//!
//! Layout, all little-endian:
//!
//! | Bytes | Field |
//! |---|---|
//! | 4 | `version`, 0xFFFFFFFF |
//! | 2 | bits: `iType` (2: 0 text, 1 check box, 2 drop-down), `iRes` (5, the check box state or the drop-down choice), `fOwnHelp`, `fOwnStat`, `fProt`, `iSize`, `iTypeTxt` (3), `fRecalc`, `fHasListBox` |
//! | 2 | `cch`: the text field's maximum length, 0 for none |
//! | 2 | `hps`: the check box size in half-points when `iSize` is set |
//! | | `xstzName`: a count of UTF-16 units, the units, a 0 terminator |
//! | | `xstzTextDef` (text) or `wDef`, 2 bytes (check box, drop-down) |
//! | | `xstzTextFormat`, `xstzHelpText`, `xstzStatText`, `xstzEntryMcr`, `xstzExitMcr` |
//! | | `hsttbDropList` (drop-down): 0xFFFF, a count, 0, then each entry as a count and its units |
//!
//! The Data stream is hostile input: every byte read is charged to the document's work budget,
//! reads stay inside the stream (and inside [`MAX_FFDATA`] bytes), and anything truncated,
//! oversized or inconsistent gives no definition at all.

use crate::{Cost, Work};

/// Most bytes of one definition read. Real ones are a few hundred bytes.
const MAX_FFDATA: usize = 1 << 16;
/// Most drop-down entries kept (Word allows 25).
const MAX_ENTRIES: usize = 64;
const VERSION: u32 = 0xFFFF_FFFF;
/// `iRes` of a check box that is checked.
const CHECKED: u8 = 1;
/// `iRes` of a field with no state or choice.
const NO_CHOICE: u8 = 25;
/// The size of a picture header (`PICF`) Word writes in front of a definition.
const PICF_HEADER: usize = 0x44;
/// The longest `w:ffData` markup the `.docx` reader keeps (`MAX_FORM` in `craft-docx`).
const MAX_FFDATA_XML: usize = 64 * 1024;

/// A bounded reader over one definition that charges each byte it takes.
struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
    work: &'a Work,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let bytes = self.buf.get(self.pos..end)?;
        if !self.work.charge(n as u64, Cost::FormData) {
            return None;
        }
        self.pos = end;
        Some(bytes)
    }

    fn u16(&mut self) -> Option<u16> {
        match self.take(2)? {
            [a, b] => Some(u16::from_le_bytes([*a, *b])),
            _ => None,
        }
    }

    /// An `xstz`: a UTF-16 string with its count in front and a 0 unit after it.
    fn xstz(&mut self) -> Option<String> {
        let units = usize::from(self.u16()?);
        let bytes = self.take(units.checked_mul(2)?)?;
        if self.u16()? != 0 {
            return None;
        }
        Some(utf16(bytes))
    }

    /// The drop-down's entries (an extended `STTB`): up to [`MAX_ENTRIES`] are kept.
    fn entries(&mut self) -> Option<Vec<String>> {
        if self.u16()? != 0xFFFF {
            return None;
        }
        let count = usize::from(self.u16()?);
        let extra = usize::from(self.u16()?);
        let mut out = Vec::new();
        for _ in 0..count {
            let units = usize::from(self.u16()?);
            let bytes = self.take(units.checked_mul(2)?)?;
            self.take(extra)?;
            if out.len() < MAX_ENTRIES {
                out.push(utf16(bytes));
            }
        }
        Some(out)
    }
}

/// UTF-16LE text with unpaired surrogates replaced and the characters XML cannot hold dropped.
fn utf16(bytes: &[u8]) -> String {
    let units = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c));
    char::decode_utf16(units).map(|r| r.unwrap_or('\u{FFFD}')).filter(|c| xml_char(*c)).collect()
}

/// The characters kept in a form value: a subset of what XML 1.0 allows in an attribute value
/// (the `.docx` reader accepts every one of them), so other control characters a file might hold
/// are dropped. Tab, line feed and carriage return are kept; [`attr`] writes them as numeric
/// references so attribute normalisation does not turn them into spaces.
fn xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r') || !matches!(c, '\0'..='\u{1F}' | '\u{7F}'..='\u{9F}' | '\u{FFFE}' | '\u{FFFF}')
}

fn attr(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // The same escapes as the .docx writer, so both sides measure the same markup.
            '\'' => out.push_str("&apos;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
}

/// `<tag w:val="value"/>`, and nothing when the value is empty.
fn leaf(out: &mut String, tag: &str, value: &str) {
    if !value.is_empty() {
        out.push_str(&format!("<{tag} w:val=\""));
        attr(out, value);
        out.push_str("\"/>");
    }
}

/// Help or status text: the field's own text, or the name of an AutoText entry.
fn text_leaf(out: &mut String, tag: &str, own: bool, value: &str) {
    if !value.is_empty() {
        out.push_str(&format!("<{tag} w:type=\"{}\" w:val=\"", if own { "text" } else { "autoText" }));
        attr(out, value);
        out.push_str("\"/>");
    }
}

/// The bytes of the `FFData` at `at`. Word writes it behind a picture header (`PICF`: its total
/// size, then the header's size at offset 4), so the definition starts at the header's size and
/// ends at the total; a definition at `at` itself (it starts with its version) is taken whole.
fn definition(data: &[u8], at: usize) -> Option<&[u8]> {
    let rest = data.get(at..)?;
    if rest.get(..4)? == VERSION.to_le_bytes() {
        return Some(rest);
    }
    let total = match rest.get(..4)? {
        [a, b, c, d] => usize::try_from(u32::from_le_bytes([*a, *b, *c, *d])).ok()?,
        _ => return None,
    };
    let header = match rest.get(4..6)? {
        [a, b] => usize::from(u16::from_le_bytes([*a, *b])),
        _ => return None,
    };
    // The record must be the size it says and fit in the stream: a clipped record is not read.
    if header != PICF_HEADER || total <= header {
        return None;
    }
    rest.get(header..total)
}

/// The `w:ffData` for the `FFData` at `at` in the Data stream, or `None` when there is none or it
/// is truncated, inconsistent or past the work budget.
pub(crate) fn read(data: &[u8], at: u32, work: &Work) -> Option<String> {
    let buf = definition(data, usize::try_from(at).ok()?)?;
    let mut cur = Cursor { buf: buf.get(..buf.len().min(MAX_FFDATA))?, pos: 0, work };
    let version = match cur.take(4)? {
        [a, b, c, d] => u32::from_le_bytes([*a, *b, *c, *d]),
        _ => return None,
    };
    if version != VERSION {
        return None;
    }
    let bits = cur.u16()?;
    let kind = (bits & 0b11) as u8;
    let res = ((bits >> 2) & 0x1F) as u8;
    let own_help = bits & (1 << 7) != 0;
    let own_stat = bits & (1 << 8) != 0;
    let protected = bits & (1 << 9) != 0;
    let size_set = bits & (1 << 10) != 0;
    let recalc = bits & (1 << 14) != 0;
    // iTypeTxt: the kind of a text field (0 is regular, which WordprocessingML assumes).
    let text_type = match (bits >> 11) & 0b111 {
        1 => Some("number"),
        2 => Some("date"),
        3 => Some("currentDate"),
        4 => Some("currentTime"),
        5 => Some("calculated"),
        0 => None,
        // Reserved values: a definition that does not say what it is gives no field definition.
        _ if bits & 0b11 == 0 => return None,
        _ => None,
    };
    let max_len = cur.u16()?;
    let hps = cur.u16()?;
    let name = cur.xstz()?;
    let (text_default, default) = if kind == 0 { (cur.xstz()?, 0) } else { (String::new(), cur.u16()?) };
    let format = cur.xstz()?;
    let help = cur.xstz()?;
    let status = cur.xstz()?;
    let entry_macro = cur.xstz()?;
    let exit_macro = cur.xstz()?;

    let mut out = String::from("<w:ffData>");
    leaf(&mut out, "w:name", &name);
    out.push_str(if protected { "<w:enabled w:val=\"0\"/>" } else { "<w:enabled/>" });
    out.push_str(&format!("<w:calcOnExit w:val=\"{}\"/>", u8::from(recalc)));
    leaf(&mut out, "w:entryMacro", &entry_macro);
    leaf(&mut out, "w:exitMacro", &exit_macro);
    text_leaf(&mut out, "w:helpText", own_help, &help);
    text_leaf(&mut out, "w:statusText", own_stat, &status);
    match kind {
        0 => {
            out.push_str("<w:textInput>");
            if let Some(t) = text_type {
                out.push_str(&format!("<w:type w:val=\"{t}\"/>"));
            }
            leaf(&mut out, "w:default", &text_default);
            if max_len > 0 {
                out.push_str(&format!("<w:maxLength w:val=\"{max_len}\"/>"));
            }
            leaf(&mut out, "w:format", &format);
            out.push_str("</w:textInput>");
        }
        1 => {
            out.push_str("<w:checkBox>");
            if size_set {
                out.push_str(&format!("<w:size w:val=\"{hps}\"/>"));
            } else {
                out.push_str("<w:sizeAuto/>");
            }
            // The default and the state are 0 or 1 (and 25 for no state); anything else is no definition.
            if default > 1 || !matches!(res, 0 | CHECKED | NO_CHOICE) {
                return None;
            }
            out.push_str(&format!("<w:default w:val=\"{default}\"/>"));
            if res == CHECKED {
                out.push_str("<w:checked/>");
            } else if default != 0 {
                // Without w:checked a reader falls back to w:default, so say the box is not ticked.
                out.push_str("<w:checked w:val=\"0\"/>");
            }
            out.push_str("</w:checkBox>");
        }
        2 => {
            let entries = cur.entries()?;
            out.push_str("<w:ddList>");
            // Only a choice that names an entry is kept (25 means none). An absent w:result selects
            // the w:default entry, so Word leaves it out when the first entry is chosen and the
            // default is the first; with another default a choice of 0 is written explicitly.
            let valid = |n: usize| n != 0 && n < entries.len();
            let chosen = res != NO_CHOICE && valid(usize::from(res));
            if chosen || (res == 0 && valid(usize::from(default))) {
                out.push_str(&format!("<w:result w:val=\"{res}\"/>"));
            }
            if valid(usize::from(default)) {
                out.push_str(&format!("<w:default w:val=\"{default}\"/>"));
            }
            for e in &entries {
                out.push_str("<w:listEntry w:val=\"");
                attr(&mut out, e);
                out.push_str("\"/>");
            }
            out.push_str("</w:ddList>");
        }
        _ => return None,
    }
    out.push_str("</w:ffData>");
    // The .docx reader drops a w:ffData longer than this, so none that long is produced.
    if out.len() > MAX_FFDATA_XML {
        return None;
    }
    // The markup is copied into the field, so its size is charged too.
    work.charge(out.len() as u64, Cost::FormData).then_some(out)
}
