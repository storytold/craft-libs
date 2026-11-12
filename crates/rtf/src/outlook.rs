//! Rich text as Outlook keeps it in `PidTagRtfCompressed` (0x1009), in .pst and .msg files and
//! TNEF attachments: the compression of
//! [MS-OXRTFCP] ([`decompress_rtf`]) and the text of the RTF that comes out ([`rtf_text`]),
//! written from those two specifications and the public Rich Text Format specification.
//!
//! Both read hostile bytes: output is capped by the caller, a header's size is never trusted
//! for an allocation past that cap, nothing recurses (groups are counted, not nested calls),
//! and every loop ends with its input.

use std::collections::HashMap;

use craft_mime::charset::{code_page_label, decode};

use crate::codepage;

/// Why rich text was not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The bytes are not what the format says; the text names what was wrong.
    Corrupt(&'static str),
    /// The compressed bytes do not have the CRC their header gives.
    BadChecksum,
    /// More would come out than the caller's cap allows.
    TooLarge,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Corrupt(what) => f.write_str(what),
            Error::BadChecksum => f.write_str("compressed rich text with a wrong checksum"),
            Error::TooLarge => f.write_str("rich text larger than allowed"),
        }
    }
}

impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

/// The CRC of [MS-OXRTFCP] 2.1.3.1 and 3.2: the reflected CRC-32 polynomial, seeded with zero and
/// with no final inversion.
pub(crate) fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { 0xEDB8_8320 ^ (crc >> 1) } else { crc >> 1 };
        }
    }
    crc
}

/// The dictionary a stream starts with ([MS-OXRTFCP] 2.1.2.1), 207 bytes.
const DICT_INIT: &[u8] = b"{\\rtf1\\ansi\\mac\\deff0\\deftab720{\\fonttbl;}{\\f0\\fnil \\froman \\fswiss \\fmodern \\fscript \\fdecor MS Sans SerifSymbolArialTimes New RomanCourier{\\colortbl\\red0\\green0\\blue0\r\n\\par \\pard\\plain\\f0\\fs20\\b\\i\\u\\tab\\tx";
const _: () = assert!(DICT_INIT.len() == 207);
const DICT_SIZE: usize = 4096;
const HEADER: usize = 16;
const LZFU: u32 = 0x7546_5A4C;
const MELA: u32 = 0x414C_454D;

fn le32(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes(<[u8; 4]>::try_from(s).ok()?))
}

/// The RTF inside `PidTagRtfCompressed` bytes: the 16-byte header, then either the text as it
/// is (`MELA`) or the dictionary-compressed runs (`LZFu`) whose CRC is checked. At most
/// `max_out` bytes come out; past that is [`Error::TooLarge`], never a bigger allocation.
pub fn decompress_rtf(data: &[u8], max_out: usize) -> Result<Vec<u8>> {
    let (Some(comp_size), Some(raw_size), Some(kind), Some(crc)) = (le32(data, 0), le32(data, 4), le32(data, 8), le32(data, 12)) else {
        return Err(Error::Corrupt("rich text shorter than its header"));
    };
    let raw_size = usize::try_from(raw_size).unwrap_or(usize::MAX);
    let body = data.get(HEADER..).unwrap_or_default();
    match kind {
        MELA => {
            // The reader may take the sizes the header gives; never more than is there.
            let take = raw_size.min(body.len());
            if take > max_out {
                return Err(Error::TooLarge);
            }
            Ok(body.get(..take).unwrap_or_default().to_vec())
        }
        LZFU => {
            if raw_size > max_out {
                return Err(Error::TooLarge);
            }
            // COMPSIZE counts the 12 bytes of the header after itself.
            let content_len = usize::try_from(comp_size)
                .unwrap_or(usize::MAX)
                .checked_sub(12)
                .ok_or(Error::Corrupt("rich text with a size smaller than its header"))?;
            let content = body.get(..content_len).ok_or(Error::Corrupt("rich text cut short"))?;
            let out = lzfu(content, max_out, raw_size)?;
            if crc32(content) != crc {
                return Err(Error::BadChecksum);
            }
            Ok(out)
        }
        _ => Err(Error::Corrupt("rich text of a kind not known")),
    }
}

fn lzfu(content: &[u8], max_out: usize, raw_size: usize) -> Result<Vec<u8>> {
    let mut dict = vec![0u8; DICT_SIZE];
    if let Some(head) = dict.get_mut(..DICT_INIT.len()) {
        head.copy_from_slice(DICT_INIT);
    }
    let mut write = DICT_INIT.len();
    let mut out: Vec<u8> = Vec::with_capacity(raw_size.min(max_out).min(1 << 20));
    let mut at = 0usize;
    let cut = Error::Corrupt("compressed rich text ends before its end mark");
    loop {
        let control = *content.get(at).ok_or(cut)?;
        at = at.saturating_add(1);
        for bit in 0..8u32 {
            if control >> bit & 1 == 0 {
                let b = *content.get(at).ok_or(cut)?;
                at = at.saturating_add(1);
                if out.len() >= max_out {
                    return Err(Error::TooLarge);
                }
                out.push(b);
                if let Some(d) = dict.get_mut(write) {
                    *d = b;
                }
                write = write.saturating_add(1) % DICT_SIZE;
            } else {
                let (Some(&hi), Some(&lo)) = (content.get(at), content.get(at.saturating_add(1))) else { return Err(cut) };
                at = at.saturating_add(2);
                let word = usize::from(hi) << 8 | usize::from(lo);
                let mut read = word >> 4;
                if read == write {
                    return Ok(out);
                }
                for _ in 0..(word & 0xF).saturating_add(2) {
                    let b = dict.get(read).copied().unwrap_or(0);
                    if out.len() >= max_out {
                        return Err(Error::TooLarge);
                    }
                    out.push(b);
                    read = read.saturating_add(1) % DICT_SIZE;
                    if let Some(d) = dict.get_mut(write) {
                        *d = b;
                    }
                    write = write.saturating_add(1) % DICT_SIZE;
                }
            }
        }
    }
}

/// Deepest nesting of groups read.
pub const MAX_RTF_DEPTH: usize = 256;

/// Destinations whose content is not the document's text.
const SKIPPED: &[&str] = &[
    "fonttbl",
    "colortbl",
    "stylesheet",
    "info",
    "pict",
    "object",
    "header",
    "headerl",
    "headerr",
    "headerf",
    "footer",
    "footerl",
    "footerr",
    "footerf",
    "themedata",
    "colorschememapping",
    "latentstyles",
    "datastore",
    "listtable",
    "listoverridetable",
    "revtbl",
    "rsidtbl",
    "generator",
    "filetbl",
    "fldinst",
    "xmlnstbl",
    "mmathpr",
    "pgdsctbl",
    "template",
    "private",
    "bkmkstart",
    "bkmkend",
    "wgrffmtfilter",
    "shpinst",
    "nonshppict",
    "pnseclvl",
    "listtext",
    "blipuid",
    "falt",
    "panose",
    "fchars",
    "lchars",
    "xe",
    "tc",
    "tcn",
    "ud",
    "upr",
    "title",
    "subject",
    "author",
    "operator",
    "company",
    "keywords",
    "doccomm",
    "comment",
    "category",
    "manager",
    "hlinkbase",
    "buptim",
    "creatim",
    "revtim",
    "printim",
    "txe",
    "rxe",
    "pnseclvl",
    "protusertbl",
    "defchp",
    "defpap",
    "fontemb",
    "fontfile",
    "background",
    "docvar",
    "userprops",
    "mhtmltag",
    "htmltag",
    "factoidname",
    "oleclsid",
    "objdata",
];

#[derive(Clone, Copy)]
struct Group {
    /// Skip characters after `\u` (`\ucN`).
    uc: u32,
    skip: bool,
    /// In the font table: `\fN` names the font that `\fcharsetN` is for.
    fonttbl: bool,
    /// The font being defined (in the font table).
    defining: i64,
    /// The font text is in (`\fN`), if one was chosen.
    font: Option<i64>,
    /// Encapsulated HTML: under `\htmlrtf`, what isn't part of the HTML.
    suppressed: bool,
    /// Encapsulated HTML: inside an `\htmltag` group.
    in_tag: bool,
}

impl Group {
    fn hidden(&self) -> bool {
        self.skip || self.suppressed
    }
}

/// Most fonts whose character set is kept.
const MAX_FONTS: usize = 1024;
/// Most bytes of `\'hh` kept waiting for a character boundary.
const PENDING: usize = 4096;

struct Out {
    text: String,
    max: usize,
    /// Text was dropped for the cap.
    cut: bool,
    pending: Vec<u8>,
    /// The code page of `pending`.
    pending_cp: u32,
    high: Option<u16>,
    ansicpg: Option<u32>,
    mac: bool,
    deff: Option<i64>,
    /// `\fcharset` of each font in the font table.
    fonts: HashMap<i64, i64>,
    /// Reading encapsulated HTML rather than the text a document shows.
    html: bool,
}

/// The code page a font character set stands for ([MSFT-RTF] `\fcharsetN`): `None` for the
/// document's own (0 is Windows ANSI, 1252, which is not the document's when it says
/// `\ansicpg` otherwise: [MS-OXRTFEX] 3.1 marks one font `\fcharset0` in a 1251 document to
/// have 0xA8 read as U+00A8). Symbol (2), Johab (130) and the OEM sets (254, 255) are read in the document's.
fn charset_code_page(charset: i64) -> Option<u32> {
    match charset {
        0 => Some(1252),
        2 | 254 | 255 => None,
        other => i32::try_from(other).ok().and_then(codepage::charset_code_page).map(u32::from),
    }
}

/// Whether `b` begins a two-byte character in code page `cp`.
fn is_lead(cp: u32, b: u8) -> bool {
    match cp {
        932 => matches!(b, 0x81..=0x9F | 0xE0..=0xFC),
        936 | 949 | 950 => matches!(b, 0x81..=0xFE),
        _ => false,
    }
}

impl Out {
    fn new(max: usize) -> Out {
        Out {
            text: String::new(),
            max,
            cut: false,
            pending: Vec::new(),
            pending_cp: 1252,
            high: None,
            ansicpg: None,
            mac: false,
            deff: None,
            fonts: HashMap::new(),
            html: false,
        }
    }

    /// The code page of the document: `\ansicpg`, else the Macintosh's for `\mac`, else 1252.
    fn document_cp(&self) -> u32 {
        self.ansicpg.unwrap_or(if self.mac { 10000 } else { 1252 })
    }

    /// The code page of bytes in font `font` (else the default font).
    fn code_page(&self, font: Option<i64>) -> u32 {
        font.or(self.deff).and_then(|f| self.fonts.get(&f)).and_then(|cs| charset_code_page(*cs)).unwrap_or_else(|| self.document_cp())
    }

    fn full(&self) -> bool {
        self.text.len() >= self.max
    }

    fn flush(&mut self) {
        let bytes = std::mem::take(&mut self.pending);
        self.decode(&bytes);
    }

    fn decode(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() {
            let decoded = decode(bytes, code_page_label(i64::from(self.pending_cp)).unwrap_or("windows-1252"));
            self.push_str(&decoded);
        }
    }

    /// Decodes the whole characters waiting, and keeps a lead byte whose second is not here yet.
    fn flush_whole(&mut self) {
        let mut cut = 0usize;
        let mut i = 0usize;
        while i < self.pending.len() {
            let step = if self.pending.get(i).is_some_and(|b| is_lead(self.pending_cp, *b)) { 2 } else { 1 };
            if i.saturating_add(step) > self.pending.len() {
                break;
            }
            i = i.saturating_add(step);
            cut = i;
        }
        let rest = self.pending.split_off(cut);
        self.flush();
        self.pending = rest;
    }

    fn push_str(&mut self, s: &str) {
        for c in s.chars() {
            // Controls other than the line end and the tab are not text.
            if c.is_control() && c != '\n' && c != '\t' {
                continue;
            }
            if self.full() {
                self.cut = true;
                return;
            }
            self.text.push(c);
        }
    }

    fn char(&mut self, c: char) {
        self.flush_high();
        self.flush();
        self.push_str(c.encode_utf8(&mut [0; 4]));
    }

    fn flush_high(&mut self) {
        if self.high.take().is_some() {
            self.push_str("\u{FFFD}");
        }
    }

    /// A byte of 8-bit text in code page `cp`.
    fn byte(&mut self, b: u8, cp: u32) {
        self.flush_high();
        if cp != self.pending_cp {
            self.flush();
            self.pending_cp = cp;
        }
        if self.pending.len() >= PENDING {
            self.flush_whole();
        }
        // (A buffer that is full of one character's bytes is not a text any code page here
        // has; it is let grow by one rather than dropped.)
        self.pending.push(b);
    }

    fn unit(&mut self, u: u16) {
        self.flush();
        match (self.high.take(), u) {
            (Some(h), 0xDC00..=0xDFFF) => {
                let c = 0x10000u32.saturating_add((u32::from(h).saturating_sub(0xD800)) << 10).saturating_add(u32::from(u).saturating_sub(0xDC00));
                self.push_str(&char::from_u32(c).unwrap_or('\u{FFFD}').to_string());
            }
            (Some(_), _) => {
                self.push_str("\u{FFFD}");
                self.unit(u);
            }
            (None, 0xD800..=0xDBFF) => self.high = Some(u),
            (None, 0xDC00..=0xDFFF) => self.push_str("\u{FFFD}"),
            (None, u) => self.push_str(&char::from_u32(u32::from(u)).unwrap_or('\u{FFFD}').to_string()),
        }
    }
}

/// The text of an RTF document: paragraphs and lines as newlines, tabs, `\'hh` bytes in the
/// code page of the font they are set in (its `\fcharset`, else the document's `\ansicpg`,
/// Windows-1252 when it names none), `\uN` with its `\ucN` fallback skipped, and none of the
/// font table, colours, styles, pictures, objects, headers or footers, nor any `{\*…}`
/// destination (for encapsulated HTML, [MS-OXRTFEX], that leaves the text the document shows
/// and not the `\htmltag` HTML, and the text as it is for `\fromtext`). Control characters
/// other than the line end and the tab are dropped. At most `max` bytes of text; whether more
/// was cut is the second part of the answer. Groups nest at most [`MAX_RTF_DEPTH`] deep, else
/// an error; unbalanced braces are read as they come.
pub fn rtf_text(rtf: &[u8], max: usize) -> Result<(String, bool)> {
    read(rtf, max, false).map(|(text, cut)| (tidy(&text), cut))
}

/// The HTML a document keeps when it is encapsulated HTML ([MS-OXRTFEX] 2.2.3): what its
/// `\htmltag` groups hold and the text no `\htmlrtf` suppresses, under the same caps as
/// [`rtf_text`]. `None` for any other document. The HTML is as hostile as the file: whoever
/// shows it reads it as any other message's.
pub fn rtf_html(rtf: &[u8], max: usize) -> Result<Option<(String, bool)>> {
    if !encapsulates_html(rtf) {
        return Ok(None);
    }
    read(rtf, max, true).map(Some)
}

/// Whether one of the document's first ten tokens says it was HTML (2.2.3.1).
fn encapsulates_html(rtf: &[u8]) -> bool {
    let mut i = rtf.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(rtf.len());
    if rtf.get(i..i.saturating_add(5)) != Some(b"{\\rtf") {
        return false;
    }
    for _ in 0..10 {
        match rtf.get(i) {
            Some(b'{') => i = i.saturating_add(1),
            Some(b'\\') => {
                let from = i.saturating_add(1);
                i = from;
                while rtf.get(i).is_some_and(u8::is_ascii_alphabetic) {
                    i = i.saturating_add(1);
                }
                let word = rtf.get(from..i).unwrap_or_default();
                if word.is_empty() {
                    return false;
                }
                let digits = i;
                while rtf.get(i).is_some_and(u8::is_ascii_digit) {
                    i = i.saturating_add(1);
                }
                match word {
                    b"fromhtml" => return rtf.get(digits..i) != Some(b"0"),
                    b"fromtext" => return false,
                    _ => {}
                }
                if rtf.get(i) == Some(&b' ') {
                    i = i.saturating_add(1);
                }
            }
            _ => return false,
        }
    }
    false
}

fn read(rtf: &[u8], max: usize, html: bool) -> Result<(String, bool)> {
    let start = rtf.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(rtf.len());
    if rtf.get(start..start.saturating_add(5)) != Some(b"{\\rtf") {
        return Err(Error::Corrupt("not rich text"));
    }
    let mut out = Out::new(max);
    out.html = html;
    let mut stack: Vec<Group> = Vec::new();
    let mut cur = Group { uc: 1, skip: false, fonttbl: false, defining: 0, font: None, suppressed: false, in_tag: false };
    let mut star = false;
    let mut first = false;
    let mut skip_chars = 0u32;
    let mut i = start;
    while let Some(&b) = rtf.get(i) {
        i = i.saturating_add(1);
        match b {
            b'{' => {
                if stack.len() >= MAX_RTF_DEPTH {
                    return Err(Error::Corrupt("rich text nested too deeply"));
                }
                stack.push(cur);
                first = true;
                skip_chars = 0;
            }
            b'}' => {
                out.flush();
                out.flush_high();
                if let Some(g) = stack.pop() {
                    cur = g;
                }
                first = false;
                skip_chars = 0;
            }
            b'\r' | b'\n' => {}
            b'\\' => {
                let Some(&n) = rtf.get(i) else { break };
                i = i.saturating_add(1);
                match n {
                    b'\\' | b'{' | b'}' => {
                        first = false;
                        if !cur.hidden() && !take(&mut skip_chars) {
                            out.char(char::from(n));
                        }
                    }
                    // A backslash before a line end is a paragraph end.
                    b'\r' | b'\n' => {
                        first = false;
                        if !cur.hidden() {
                            out.char('\n');
                        }
                    }
                    b'\'' => {
                        first = false;
                        let hex =
                            rtf.get(i..i.saturating_add(2)).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
                        if let Some(v) = hex {
                            i = i.saturating_add(2);
                            if !cur.hidden() && !take(&mut skip_chars) {
                                let cp = out.code_page(cur.font);
                                out.byte(v, cp);
                            }
                        }
                    }
                    b'*' => {
                        cur.skip = true;
                        first = false;
                        star = true;
                    }
                    b'~' => symbol(&mut out, &cur, '\u{A0}', &mut first),
                    b'_' => {
                        let hyphen = if out.html { '\u{AD}' } else { '\u{2011}' };
                        symbol(&mut out, &cur, hyphen, &mut first);
                    }
                    b'-' | b'|' | b':' => first = false,
                    n if n.is_ascii_alphabetic() => {
                        let from = i.saturating_sub(1);
                        while rtf.get(i).is_some_and(u8::is_ascii_alphabetic) {
                            i = i.saturating_add(1);
                        }
                        let word = rtf.get(from..i).and_then(|w| std::str::from_utf8(w).ok()).unwrap_or_default();
                        let word = if word.len() > 32 { word.get(..32).unwrap_or(word) } else { word };
                        // An optional parameter, then at most one space of delimiter.
                        let mut neg = false;
                        if rtf.get(i) == Some(&b'-') {
                            neg = true;
                            i = i.saturating_add(1);
                        }
                        let mut num: Option<i64> = None;
                        while let Some(d) = rtf.get(i).filter(|d| d.is_ascii_digit()) {
                            num = Some(num.unwrap_or(0).saturating_mul(10).saturating_add(i64::from(d.saturating_sub(b'0'))).min(1 << 40));
                            i = i.saturating_add(1);
                        }
                        let num = num.map(|v| if neg { v.saturating_neg() } else { v });
                        if rtf.get(i) == Some(&b' ') {
                            i = i.saturating_add(1);
                        }
                        let was_first = std::mem::replace(&mut first, false);
                        // The HTML of a tag is read whatever suppresses what is round it, by the
                        // document's own code page (2.2.3.2).
                        if html && std::mem::take(&mut star) && word == "htmltag" && !stack.last().is_some_and(|g| g.skip) {
                            cur.skip = false;
                            cur.in_tag = true;
                            cur.suppressed = false;
                            cur.font = None;
                        }
                        if was_first && SKIPPED.contains(&word.to_ascii_lowercase().as_str()) {
                            cur.skip = true;
                            cur.fonttbl = word.eq_ignore_ascii_case("fonttbl");
                        }
                        skip_chars = 0;
                        control(word, num, &mut cur, &mut out, &mut skip_chars, &mut i, rtf);
                    }
                    _ => first = false,
                }
            }
            b'\t' => {
                first = false;
                if !cur.hidden() && !take(&mut skip_chars) {
                    out.char('\t');
                }
            }
            0x00..=0x1F => {}
            _ => {
                first = false;
                if cur.hidden() || take(&mut skip_chars) {
                    continue;
                }
                if b.is_ascii() {
                    out.char(char::from(b));
                } else {
                    let cp = out.code_page(cur.font);
                    out.byte(b, cp);
                }
            }
        }
    }
    out.flush();
    out.flush_high();
    let cut = out.cut;
    Ok((out.text, cut))
}

fn take(skip: &mut u32) -> bool {
    if *skip > 0 {
        *skip = skip.saturating_sub(1);
        true
    } else {
        false
    }
}

fn symbol(out: &mut Out, cur: &Group, c: char, first: &mut bool) {
    *first = false;
    if !cur.hidden() {
        out.char(c);
    }
}

fn control(word: &str, num: Option<i64>, cur: &mut Group, out: &mut Out, skip_chars: &mut u32, i: &mut usize, rtf: &[u8]) {
    match word {
        "ansicpg" => out.ansicpg = num.and_then(|n| u32::try_from(n).ok()),
        "mac" => out.mac = true,
        // The fonts: which character set each has, and which one the text is in.
        "f" if cur.fonttbl => cur.defining = num.unwrap_or(0),
        "fcharset" if cur.fonttbl => {
            if let Some(cs) = num
                && (out.fonts.len() < MAX_FONTS || out.fonts.contains_key(&cur.defining))
            {
                out.fonts.insert(cur.defining, cs);
            }
        }
        "f" if !cur.skip => cur.font = num,
        "deff" => out.deff = num,
        "plain" if !cur.skip => cur.font = None,
        "uc" => cur.uc = num.and_then(|n| u32::try_from(n).ok()).unwrap_or(1).min(16),
        "htmlrtf" if out.html && !cur.in_tag => cur.suppressed = num != Some(0),
        "u" if !cur.hidden() => {
            if let Some(n) = num {
                // A negative number is the unit minus 65536; the fallback after it is skipped.
                let unit = if n < 0 { n.saturating_add(65536) } else { n };
                out.unit(u16::try_from(unit & 0xFFFF).unwrap_or(0xFFFD));
                *skip_chars = cur.uc;
            }
        }
        "bin" => {
            let n = num.and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
            *i = i.saturating_add(n).min(rtf.len());
        }
        "par" | "line" if !cur.hidden() => out.char('\n'),
        "sect" | "page" | "row" if !cur.hidden() && !out.html => out.char('\n'),
        "tab" if !cur.hidden() => out.char('\t'),
        "cell" if !cur.hidden() && !out.html => out.char('\t'),
        "emdash" if !cur.hidden() => out.char('\u{2014}'),
        "endash" if !cur.hidden() => out.char('\u{2013}'),
        "lquote" if !cur.hidden() => out.char('\u{2018}'),
        "rquote" if !cur.hidden() => out.char('\u{2019}'),
        "ldblquote" if !cur.hidden() => out.char('\u{201C}'),
        "rdblquote" if !cur.hidden() => out.char('\u{201D}'),
        "bullet" if !cur.hidden() => out.char('\u{2022}'),
        "emspace" | "enspace" | "qmspace" if !cur.hidden() => out.char(' '),
        _ => {}
    }
}

/// At most one blank line in a row, and no blanks at the ends.
fn tidy(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut newlines = 0u32;
    for c in s.chars() {
        if c == '\n' {
            newlines = newlines.saturating_add(1);
            if newlines > 2 {
                continue;
            }
        } else {
            newlines = 0;
        }
        out.push(c);
    }
    out.trim().to_string()
}

#[cfg(test)]
#[path = "outlook_tests.rs"]
mod tests;
