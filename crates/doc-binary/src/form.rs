//! Legacy form fields (`FORMCHECKBOX`, `FORMTEXT`, `FORMDROPDOWN`): their settings, an FFData
//! ([MS-DOC] §2.9.75) in the Data stream, as the `w:ffData` XML the shared model keeps for them.
//!
//! Word marks the location on the 0x01 character that ends the field's instruction, with
//! `sprmCFData` on and `sprmCPicLocation` pointing at a NilPICFAndBinData: a 68-byte header
//! (`lcb`, `cbHeader`) followed by the FFData. Layout and the DOCX writer read the XML the same
//! way as one that came from a `.docx`, so the check boxes are drawn and survive a save.
//!
//! [MS-DOC]: https://learn.microsoft.com/en-us/openspecs/office_file_formats/ms-doc/

/// `NilPICFAndBinData.cbHeader`: the header before the FFData, which MUST be 0x44.
const CB_HEADER: usize = 0x44;
/// `FFData.version`.
const VERSION: u32 = 0xFFFF_FFFF;
/// The longest Xstz kept ([MS-DOC] caps every FFData Xstz at 255 characters or fewer); a
/// longer one makes the whole FFData unreadable, and the field is kept without settings. The
/// drop-down entries have no such cap in [MS-DOC]: each is read in full, bounded by the record.
const MAX_XSTZ: usize = 255;
/// Most drop-down entries read ([MS-DOC]: at most 25).
const MAX_ENTRIES: usize = 25;
/// The most bytes [`ff_data_xml`] decodes after the header: version, bits, cch and hps, `wDef`,
/// seven Xstz at [`MAX_XSTZ`], the STTB header, and [`MAX_ENTRIES`] entries of up to
/// `u16::MAX` units (an entry's extra data is skipped, not decoded).
const MAX_DECODED: usize = 10 + 2 + 7 * (2 + 2 * MAX_XSTZ + 2) + 6 + MAX_ENTRIES * (2 + 2 * u16::MAX as usize);
/// `iRes` for "no state": a check box that follows its default, a drop-down with no selection.
const RES_UNDEFINED: u16 = 25;

/// The `w:ffData` of the form field whose FFData is at `at` in the Data stream; `None` when
/// the bytes there aren't one.
pub(crate) fn ff_data_xml(data: &[u8], at: u32) -> Option<String> {
    let at = usize::try_from(at).ok()?;
    let lcb = u32::from_le_bytes(data.get(at..at.checked_add(4)?)?.try_into().ok()?) as usize;
    let cb_header = u16::from_le_bytes(data.get(at.checked_add(4)?..at.checked_add(6)?)?.try_into().ok()?) as usize;
    if cb_header != CB_HEADER || lcb < CB_HEADER {
        return None;
    }
    let end = at.checked_add(lcb)?.min(data.len());
    let ff = FfData::parse(data.get(at.checked_add(CB_HEADER)?..end)?)?;
    Some(ff.to_xml())
}

/// The bytes [`ff_data_xml`] may read for the record at `at`: the record (its `lcb`, when there
/// is one to read) up to the Data stream's end, at most the header and [`MAX_DECODED`]. To be
/// charged before decoding, so a hostile `lcb` costs no more than the decoding it allows.
pub(crate) fn decode_cost(data: &[u8], at: u32) -> u64 {
    let Ok(at) = usize::try_from(at) else { return 0 };
    let left = data.len().saturating_sub(at);
    let lcb = at.checked_add(4).and_then(|end| data.get(at..end)).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
    let span = lcb.and_then(|l| usize::try_from(l).ok()).map_or(left, |l| l.min(left));
    span.min(CB_HEADER + MAX_DECODED) as u64
}

/// What an FFData says, decoded.
struct FfData {
    bits: u16,
    cch: u16,
    hps: u16,
    name: String,
    text_def: String,
    w_def: u16,
    text_format: String,
    help: String,
    status: String,
    entry_macro: String,
    exit_macro: String,
    entries: Vec<String>,
}

impl FfData {
    fn parse(b: &[u8]) -> Option<FfData> {
        let mut r = Reader { b, at: 0 };
        if r.u32()? != VERSION {
            return None;
        }
        let bits = r.u16()?;
        let cch = r.u16()?;
        let hps = r.u16()?;
        let kind = bits & 0b11;
        if kind > 2 {
            return None;
        }
        let name = r.xstz()?;
        let text_def = if kind == 0 { r.xstz()? } else { String::new() };
        let w_def = if kind == 0 { 0 } else { r.u16()? };
        let text_format = r.xstz()?;
        let help = r.xstz()?;
        let status = r.xstz()?;
        let entry_macro = r.xstz()?;
        let exit_macro = r.xstz()?;
        let entries = if kind == 2 { r.sttb()? } else { Vec::new() };
        Some(FfData { bits, cch, hps, name, text_def, w_def, text_format, help, status, entry_macro, exit_macro, entries })
    }

    fn kind(&self) -> u16 {
        self.bits & 0b11
    }
    fn res(&self) -> u16 {
        (self.bits >> 2) & 0x1F
    }
    fn own_help(&self) -> bool {
        self.bits & (1 << 7) != 0
    }
    fn own_status(&self) -> bool {
        self.bits & (1 << 8) != 0
    }
    fn protected(&self) -> bool {
        self.bits & (1 << 9) != 0
    }
    /// `iSize`: 1 is an exact size (`hps`), 0 follows the text, as Word writes it.
    fn exact_size(&self) -> bool {
        self.bits & (1 << 10) != 0
    }
    fn text_type(&self) -> u16 {
        (self.bits >> 11) & 0b111
    }
    fn recalc(&self) -> bool {
        self.bits & (1 << 14) != 0
    }

    /// The `w:ffData` element, in the order Word writes its children (ECMA-376 Part 1 §17.16.17).
    fn to_xml(&self) -> String {
        let mut x = String::from("<w:ffData>");
        x.push_str(&format!(r#"<w:name w:val="{}"/>"#, escape(&self.name)));
        x.push_str(if self.protected() { r#"<w:enabled w:val="0"/>"# } else { "<w:enabled/>" });
        x.push_str(if self.recalc() { "<w:calcOnExit/>" } else { r#"<w:calcOnExit w:val="0"/>"# });
        if !self.entry_macro.is_empty() {
            x.push_str(&format!(r#"<w:entryMacro w:val="{}"/>"#, escape(&self.entry_macro)));
        }
        if !self.exit_macro.is_empty() {
            x.push_str(&format!(r#"<w:exitMacro w:val="{}"/>"#, escape(&self.exit_macro)));
        }
        if self.own_help() && !self.help.is_empty() {
            x.push_str(&format!(r#"<w:helpText w:type="text" w:val="{}"/>"#, escape(&self.help)));
        }
        if self.own_status() && !self.status.is_empty() {
            x.push_str(&format!(r#"<w:statusText w:type="text" w:val="{}"/>"#, escape(&self.status)));
        }
        match self.kind() {
            1 => {
                x.push_str("<w:checkBox>");
                if self.exact_size() {
                    x.push_str(&format!(r#"<w:size w:val="{}"/>"#, self.hps.clamp(2, 3168)));
                } else {
                    x.push_str("<w:sizeAuto/>");
                }
                let default = u16::from(self.w_def != 0);
                x.push_str(&format!(r#"<w:default w:val="{default}"/>"#));
                match self.res() {
                    1 => x.push_str("<w:checked/>"),
                    0 if default == 1 => x.push_str(r#"<w:checked w:val="0"/>"#),
                    _ => {}
                }
                x.push_str("</w:checkBox>");
            }
            2 => {
                x.push_str("<w:ddList>");
                let n = self.entries.len();
                let res = self.res();
                if res != RES_UNDEFINED && usize::from(res) < n {
                    x.push_str(&format!(r#"<w:result w:val="{res}"/>"#));
                }
                if self.w_def != 0 && usize::from(self.w_def) < n {
                    x.push_str(&format!(r#"<w:default w:val="{}"/>"#, self.w_def));
                }
                for e in &self.entries {
                    x.push_str(&format!(r#"<w:listEntry w:val="{}"/>"#, escape(e)));
                }
                x.push_str("</w:ddList>");
            }
            _ => {
                x.push_str("<w:textInput>");
                let kind = match self.text_type() {
                    1 => Some("number"),
                    2 => Some("date"),
                    3 => Some("currentDate"),
                    4 => Some("currentTime"),
                    5 => Some("calculated"),
                    _ => None,
                };
                if let Some(kind) = kind {
                    x.push_str(&format!(r#"<w:type w:val="{kind}"/>"#));
                }
                if !self.text_def.is_empty() {
                    x.push_str(&format!(r#"<w:default w:val="{}"/>"#, escape(&self.text_def)));
                }
                if self.cch > 0 {
                    x.push_str(&format!(r#"<w:maxLength w:val="{}"/>"#, self.cch.min(32767)));
                }
                if !self.text_format.is_empty() {
                    x.push_str(&format!(r#"<w:format w:val="{}"/>"#, escape(&self.text_format)));
                }
                x.push_str("</w:textInput>");
            }
        }
        x.push_str("</w:ffData>");
        x
    }
}

/// A bounds-checked little-endian cursor: every read past the end is `None`.
struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let end = self.at.checked_add(n)?;
        let s = self.b.get(self.at..end)?;
        self.at = end;
        Some(s)
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).and_then(|s| s.try_into().ok()).map(u16::from_le_bytes)
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4).and_then(|s| s.try_into().ok()).map(u32::from_le_bytes)
    }
    /// `cch` UTF-16 code units; `None` past `max` or past the end.
    fn chars(&mut self, cch: usize, max: usize) -> Option<String> {
        if cch > max {
            return None;
        }
        let units = self.take(cch.checked_mul(2)?)?.as_chunks::<2>().0.iter().map(|u| u16::from_le_bytes(*u));
        Some(char::decode_utf16(units).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)).collect())
    }
    /// An Xstz: a counted UTF-16 string and its 0 terminator.
    fn xstz(&mut self) -> Option<String> {
        let cch = usize::from(self.u16()?);
        let s = self.chars(cch, MAX_XSTZ)?;
        self.u16()?;
        Some(s)
    }
    /// The drop-down's STTB: extended (UTF-16) strings without extra data; past
    /// [`MAX_ENTRIES`] the rest are left out. An entry is read whatever its length, as long as
    /// the record holds it.
    fn sttb(&mut self) -> Option<Vec<String>> {
        if self.u16()? != 0xFFFF {
            return None;
        }
        let count = usize::from(self.u16()?);
        let extra = usize::from(self.u16()?);
        let mut entries = Vec::new();
        for _ in 0..count.min(MAX_ENTRIES) {
            let cch = usize::from(self.u16()?);
            entries.push(self.chars(cch, usize::from(u16::MAX))?);
            self.take(extra)?;
        }
        Some(entries)
    }
}

/// `s` as an XML attribute value. Tabs and line breaks become character references; other
/// characters XML can't hold are left out.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => {}
            c => out.push(c),
        }
    }
    out
}
