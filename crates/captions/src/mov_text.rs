//! MP4 Timed Text (`mov_text` / `tx3g`, 3GPP TS 26.245).
//!
//! A clean-room reader and writer for 3GPP Timed Text subtitle samples and sample entry
//! configurations (`tx3g`), as carried in ISO BMFF (`.mp4`) and QuickTime (`.mov`) files.
//!
//! ## Sample Structure
//!
//! A timed text sample consists of:
//! - A 16-bit big-endian text length.
//! - The text string bytes (UTF-8, or UTF-16 with BOM).
//! - Zero or more modifier boxes:
//!   - `styl`: Character style runs (bold, italic, underline, font ID, font size, text color).
//!   - `hlit`: Highlighted character range.
//!   - `hclr`: Highlight color (RGBA).
//!   - `krok`: Karaoke highlight timing and character spans.
//!   - `dlay`: Scroll delay in track-timescale ticks.
//!   - `href`: Hypertext link URL and alt text.
//!   - `tbox`: Text box override coordinates (`top, left, bottom, right`).
//!   - `blnk`: Blinking text character range.
//!   - `twrp`: Text wrap flag (0 = no wrap, 1 = automatic soft wrap).
//!
//! ## Style Flags
//!
//! Style flags in `styl` records:
//! - `0x01`: Bold (`<b>...</b>`)
//! - `0x02`: Italic (`<i>...</i>`)
//! - `0x04`: Underline (`<u>...</u>`)

use crate::time::Tick;
use crate::{Cue, Document, TICKS_PER_SECOND};

/// What went wrong reading or writing timed text.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The input slice is shorter than the declared field or box size.
    #[error("truncated timed text data")]
    Truncated,
    /// The text payload is not valid UTF-8 and contains no valid UTF-16 BOM.
    #[error("invalid text encoding in timed text sample")]
    InvalidText,
    /// The text exceeds the 16-bit length prefix (65,535 bytes).
    #[error("timed text string exceeds 65535 bytes: {0}")]
    TextTooLong(usize),
    /// A modifier box header has an invalid size (< 8).
    #[error("invalid modifier box size: {0}")]
    InvalidBox(&'static str),
}

/// `Result` alias for timed text operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Face style flag: Bold (`0x01`).
pub const STYLE_FLAG_BOLD: u8 = 0x01;
/// Face style flag: Italic (`0x02`).
pub const STYLE_FLAG_ITALIC: u8 = 0x02;
/// Face style flag: Underline (`0x04`).
pub const STYLE_FLAG_UNDERLINE: u8 = 0x04;

/// A character styling record within a `styl` box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StyleRecord {
    /// 0-indexed start character index (inclusive, measured in Unicode scalar values).
    pub start_char: u16,
    /// 0-indexed end character index (exclusive, measured in Unicode scalar values).
    pub end_char: u16,
    /// 16-bit font ID matching an entry in the sample description font table (`ftab`).
    pub font_id: u16,
    /// Face style flags: bit 0 = bold, bit 1 = italic, bit 2 = underline.
    pub face_style_flags: u8,
    /// Font size in points.
    pub font_size: u8,
    /// Text RGBA color (`[r, g, b, a]`).
    pub text_color_rgba: [u8; 4],
}

impl Default for StyleRecord {
    fn default() -> Self {
        Self { start_char: 0, end_char: 0, font_id: 1, face_style_flags: 0, font_size: 16, text_color_rgba: [255, 255, 255, 255] }
    }
}

impl StyleRecord {
    /// Returns `true` if bold is set.
    pub fn is_bold(&self) -> bool {
        self.face_style_flags & STYLE_FLAG_BOLD != 0
    }

    /// Returns `true` if italic is set.
    pub fn is_italic(&self) -> bool {
        self.face_style_flags & STYLE_FLAG_ITALIC != 0
    }

    /// Returns `true` if underline is set.
    pub fn is_underline(&self) -> bool {
        self.face_style_flags & STYLE_FLAG_UNDERLINE != 0
    }
}

/// Coordinates of a text display box (`tbox`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextBox {
    pub top: i16,
    pub left: i16,
    pub bottom: i16,
    pub right: i16,
}

/// An entry in a karaoke box (`krok`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KaraokeEntry {
    /// Highlight end time in timescale ticks.
    pub highlight_end_time: u32,
    /// Start character index (inclusive).
    pub start_char: u16,
    /// End character index (exclusive).
    pub end_char: u16,
}

/// A hypertext link entry (`href`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HyperText {
    /// Start character index (inclusive).
    pub start_char: u16,
    /// End character index (exclusive).
    pub end_char: u16,
    /// Destination URL.
    pub url: String,
    /// Accessible alt text string.
    pub alt_string: String,
}

/// An unpacked 3GPP Timed Text (`tx3g` / `mov_text`) sample.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextSample {
    /// Decoded UTF-8 text content.
    pub text: String,
    /// Character styling records from the `styl` box.
    pub styles: Vec<StyleRecord>,
    /// Highlighted character range `(start_char, end_char)` from `hlit`.
    pub highlight: Option<(u16, u16)>,
    /// Highlight color `[r, g, b, a]` from `hclr`.
    pub highlight_color: Option<[u8; 4]>,
    /// Karaoke timing and spans `(highlight_start_time, entries)` from `krok`.
    pub karaoke: Option<(u32, Vec<KaraokeEntry>)>,
    /// Scroll delay in milliseconds from `dlay`.
    pub scroll_delay: Option<u32>,
    /// Hypertext links from `href` boxes.
    pub hypertext: Vec<HyperText>,
    /// Text box override from `tbox`.
    pub text_box: Option<TextBox>,
    /// Blinking character ranges `(start_char, end_char)` from `blnk` boxes.
    pub blinking: Vec<(u16, u16)>,
    /// Wrap flag from `wrap` box: `0` = automatic wrap, `1` = no wrap.
    pub wrap_flag: Option<u8>,
}

/// Sample description table configuration for a `tx3g` track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tx3gConfig {
    /// 32-bit display flags (e.g. scroll in, scroll out, continuous scroll).
    pub display_flags: u32,
    /// Horizontal justification: 0 = left, 1 = center, -1 = right.
    pub horizontal_justification: i8,
    /// Vertical justification: 0 = top, 1 = center, -1 = bottom.
    pub vertical_justification: i8,
    /// Background RGBA color.
    pub background_color: [u8; 4],
    /// Default text box coordinates.
    pub default_text_box: TextBox,
    /// Default style record.
    pub default_style: StyleRecord,
    /// Font table entries `(font_id, font_name)`.
    pub fonts: Vec<(u16, String)>,
}

impl Default for Tx3gConfig {
    fn default() -> Self {
        Self {
            display_flags: 0,
            horizontal_justification: 1, // center
            vertical_justification: -1,  // bottom
            background_color: [0, 0, 0, 255],
            default_text_box: TextBox::default(),
            default_style: StyleRecord::default(),
            fonts: vec![(1, "Arial".to_string())],
        }
    }
}

impl Tx3gConfig {
    /// Parse a `tx3g` sample entry payload (after the standard 8-byte sample entry header).
    pub fn parse(payload: &[u8]) -> Result<Self> {
        if payload.len() < 30 {
            return Err(Error::Truncated);
        }
        let display_flags = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
        let horizontal_justification = payload[4] as i8;
        let vertical_justification = payload[5] as i8;
        let background_color = [payload[6], payload[7], payload[8], payload[9]];

        let default_text_box = TextBox {
            top: i16::from_be_bytes([payload[10], payload[11]]),
            left: i16::from_be_bytes([payload[12], payload[13]]),
            bottom: i16::from_be_bytes([payload[14], payload[15]]),
            right: i16::from_be_bytes([payload[16], payload[17]]),
        };

        let default_style = StyleRecord {
            start_char: u16::from_be_bytes([payload[18], payload[19]]),
            end_char: u16::from_be_bytes([payload[20], payload[21]]),
            font_id: u16::from_be_bytes([payload[22], payload[23]]),
            face_style_flags: payload[24],
            font_size: payload[25],
            text_color_rgba: [payload[26], payload[27], payload[28], payload[29]],
        };

        let mut fonts = Vec::new();
        let mut pos = 30;
        while pos + 8 <= payload.len() {
            let box_size = u32::from_be_bytes([payload[pos], payload[pos + 1], payload[pos + 2], payload[pos + 3]]) as usize;
            if box_size < 8 || pos.checked_add(box_size).is_none_or(|end| end > payload.len()) {
                break;
            }
            let box_type = &payload[pos + 4..pos + 8];
            if box_type == b"ftab" && box_size >= 10 {
                let count = u16::from_be_bytes([payload[pos + 8], payload[pos + 9]]) as usize;
                let mut fpos = pos + 10;
                for _ in 0..count {
                    if fpos + 3 > pos + box_size {
                        break;
                    }
                    let font_id = u16::from_be_bytes([payload[fpos], payload[fpos + 1]]);
                    let name_len = payload[fpos + 2] as usize;
                    fpos += 3;
                    if fpos + name_len > pos + box_size {
                        break;
                    }
                    if let Ok(name) = std::str::from_utf8(&payload[fpos..fpos + name_len]) {
                        fonts.push((font_id, name.to_string()));
                    }
                    fpos += name_len;
                }
            }
            pos += box_size;
        }

        Ok(Self { display_flags, horizontal_justification, vertical_justification, background_color, default_text_box, default_style, fonts })
    }

    /// Write the `tx3g` sample entry payload (after the 8-byte sample entry header).
    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(30 + 32);
        out.extend_from_slice(&self.display_flags.to_be_bytes());
        out.push(self.horizontal_justification as u8);
        out.push(self.vertical_justification as u8);
        out.extend_from_slice(&self.background_color);

        out.extend_from_slice(&self.default_text_box.top.to_be_bytes());
        out.extend_from_slice(&self.default_text_box.left.to_be_bytes());
        out.extend_from_slice(&self.default_text_box.bottom.to_be_bytes());
        out.extend_from_slice(&self.default_text_box.right.to_be_bytes());

        out.extend_from_slice(&self.default_style.start_char.to_be_bytes());
        out.extend_from_slice(&self.default_style.end_char.to_be_bytes());
        out.extend_from_slice(&self.default_style.font_id.to_be_bytes());
        out.push(self.default_style.face_style_flags);
        out.push(self.default_style.font_size);
        out.extend_from_slice(&self.default_style.text_color_rgba);

        // ftab box
        let mut ftab_payload = Vec::new();
        let fonts = &self.fonts[..self.fonts.len().min(u16::MAX as usize)];
        ftab_payload.extend_from_slice(&(fonts.len() as u16).to_be_bytes());
        for (id, name) in fonts {
            ftab_payload.extend_from_slice(&id.to_be_bytes());
            let name_bytes = name.as_bytes();
            let limit = name.floor_char_boundary(name_bytes.len().min(255));
            ftab_payload.push(limit as u8);
            ftab_payload.extend_from_slice(&name_bytes[..limit]);
        }
        let ftab_size = 8u32.saturating_add(ftab_payload.len() as u32);
        out.extend_from_slice(&ftab_size.to_be_bytes());
        out.extend_from_slice(b"ftab");
        out.extend_from_slice(&ftab_payload);

        out
    }
}

/// Read and decode a single 3GPP Timed Text sample (`tx3g` / `mov_text`).
pub fn read_sample(data: &[u8]) -> Result<TextSample> {
    if data.len() < 2 {
        return Err(Error::Truncated);
    }

    let text_len = u16::from_be_bytes([data[0], data[1]]) as usize;
    if data.len() < 2 + text_len {
        return Err(Error::Truncated);
    }

    let text_bytes = &data[2..2 + text_len];
    let text = decode_text(text_bytes)?;

    let mut sample = TextSample { text, ..Default::default() };

    let mut pos = 2 + text_len;
    while pos + 8 <= data.len() {
        let size = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        if size < 8 {
            return Err(Error::InvalidBox("size smaller than 8"));
        }
        let box_end = pos.checked_add(size).ok_or(Error::Truncated)?;
        if box_end > data.len() {
            return Err(Error::Truncated);
        }

        let box_type = &data[pos + 4..pos + 8];
        let payload = &data[pos + 8..box_end];

        match box_type {
            b"styl" => {
                if payload.len() < 2 {
                    return Err(Error::Truncated);
                }
                let count = u16::from_be_bytes([payload[0], payload[1]]) as usize;
                let mut spos = 2;
                for _ in 0..count {
                    if spos + 12 > payload.len() {
                        return Err(Error::Truncated);
                    }
                    sample.styles.push(StyleRecord {
                        start_char: u16::from_be_bytes([payload[spos], payload[spos + 1]]),
                        end_char: u16::from_be_bytes([payload[spos + 2], payload[spos + 3]]),
                        font_id: u16::from_be_bytes([payload[spos + 4], payload[spos + 5]]),
                        face_style_flags: payload[spos + 6],
                        font_size: payload[spos + 7],
                        text_color_rgba: [payload[spos + 8], payload[spos + 9], payload[spos + 10], payload[spos + 11]],
                    });
                    spos += 12;
                }
            }
            b"hlit" => {
                if payload.len() < 4 {
                    return Err(Error::Truncated);
                }
                let start = u16::from_be_bytes([payload[0], payload[1]]);
                let end = u16::from_be_bytes([payload[2], payload[3]]);
                sample.highlight = Some((start, end));
            }
            b"hclr" => {
                if payload.len() < 4 {
                    return Err(Error::Truncated);
                }
                sample.highlight_color = Some([payload[0], payload[1], payload[2], payload[3]]);
            }
            b"krok" => {
                if payload.len() < 6 {
                    return Err(Error::Truncated);
                }
                let start_time = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let count = u16::from_be_bytes([payload[4], payload[5]]) as usize;
                let mut entries = Vec::new();
                let mut kpos = 6;
                for _ in 0..count {
                    if kpos + 8 > payload.len() {
                        return Err(Error::Truncated);
                    }
                    entries.push(KaraokeEntry {
                        highlight_end_time: u32::from_be_bytes([payload[kpos], payload[kpos + 1], payload[kpos + 2], payload[kpos + 3]]),
                        start_char: u16::from_be_bytes([payload[kpos + 4], payload[kpos + 5]]),
                        end_char: u16::from_be_bytes([payload[kpos + 6], payload[kpos + 7]]),
                    });
                    kpos += 8;
                }
                sample.karaoke = Some((start_time, entries));
            }
            b"dlay" => {
                if payload.len() < 4 {
                    return Err(Error::Truncated);
                }
                sample.scroll_delay = Some(u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]));
            }
            b"href" => {
                if payload.len() < 6 {
                    return Err(Error::Truncated);
                }
                let start = u16::from_be_bytes([payload[0], payload[1]]);
                let end = u16::from_be_bytes([payload[2], payload[3]]);
                let url_len = payload[4] as usize;
                if 5 + url_len > payload.len() {
                    return Err(Error::Truncated);
                }
                let url = std::str::from_utf8(&payload[5..5 + url_len]).map_err(|_| Error::InvalidText)?.to_string();
                let alt_pos = 5 + url_len;
                let alt_len = payload.get(alt_pos).copied().unwrap_or(0) as usize;
                let alt_string = if alt_len > 0 && alt_pos + 1 + alt_len <= payload.len() {
                    std::str::from_utf8(&payload[alt_pos + 1..alt_pos + 1 + alt_len]).map_err(|_| Error::InvalidText)?.to_string()
                } else {
                    String::new()
                };
                sample.hypertext.push(HyperText { start_char: start, end_char: end, url, alt_string });
            }
            b"tbox" => {
                if payload.len() < 8 {
                    return Err(Error::Truncated);
                }
                sample.text_box = Some(TextBox {
                    top: i16::from_be_bytes([payload[0], payload[1]]),
                    left: i16::from_be_bytes([payload[2], payload[3]]),
                    bottom: i16::from_be_bytes([payload[4], payload[5]]),
                    right: i16::from_be_bytes([payload[6], payload[7]]),
                });
            }
            b"blnk" => {
                if payload.len() < 4 {
                    return Err(Error::Truncated);
                }
                let start = u16::from_be_bytes([payload[0], payload[1]]);
                let end = u16::from_be_bytes([payload[2], payload[3]]);
                sample.blinking.push((start, end));
            }
            b"twrp" | b"wrap" => {
                if payload.is_empty() {
                    return Err(Error::Truncated);
                }
                sample.wrap_flag = Some(payload[0]);
            }
            _ => {
                // Unknown modifier box: safely ignored.
            }
        }

        pos = box_end;
    }

    Ok(sample)
}

/// Helper to decode UTF-8 or UTF-16 with BOM.
fn decode_text(bytes: &[u8]) -> Result<String> {
    if bytes.len() >= 2 {
        if bytes[0] == 0xFE && bytes[1] == 0xFF {
            // UTF-16 BE
            let u16s: Vec<u16> = bytes[2..].as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            return String::from_utf16(&u16s).map_err(|_| Error::InvalidText);
        } else if bytes[0] == 0xFF && bytes[1] == 0xFE {
            // UTF-16 LE
            let u16s: Vec<u16> = bytes[2..].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            return String::from_utf16(&u16s).map_err(|_| Error::InvalidText);
        }
    }
    std::str::from_utf8(bytes).map(ToString::to_string).map_err(|_| Error::InvalidText)
}

/// Pack and encode a timed text sample into bytes.
pub fn write_sample(sample: &TextSample) -> Result<Vec<u8>> {
    let text_bytes = sample.text.as_bytes();
    if text_bytes.len() > u16::MAX as usize {
        return Err(Error::TextTooLong(text_bytes.len()));
    }

    let mut out = Vec::with_capacity(2 + text_bytes.len() + 64);
    out.extend_from_slice(&(text_bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(text_bytes);

    // 1. styl box
    if !sample.styles.is_empty() {
        if sample.styles.len() > u16::MAX as usize {
            return Err(Error::InvalidBox("too many styles in sample"));
        }
        let size = 8u32.checked_add(2).and_then(|s| s.checked_add(sample.styles.len() as u32 * 12)).ok_or(Error::InvalidBox("styl size overflow"))?;
        out.extend_from_slice(&size.to_be_bytes());
        out.extend_from_slice(b"styl");
        out.extend_from_slice(&(sample.styles.len() as u16).to_be_bytes());
        for st in &sample.styles {
            out.extend_from_slice(&st.start_char.to_be_bytes());
            out.extend_from_slice(&st.end_char.to_be_bytes());
            out.extend_from_slice(&st.font_id.to_be_bytes());
            out.push(st.face_style_flags);
            out.push(st.font_size);
            out.extend_from_slice(&st.text_color_rgba);
        }
    }

    // 2. hlit box
    if let Some((start, end)) = sample.highlight {
        out.extend_from_slice(&12u32.to_be_bytes());
        out.extend_from_slice(b"hlit");
        out.extend_from_slice(&start.to_be_bytes());
        out.extend_from_slice(&end.to_be_bytes());
    }

    // 3. hclr box
    if let Some(color) = sample.highlight_color {
        out.extend_from_slice(&12u32.to_be_bytes());
        out.extend_from_slice(b"hclr");
        out.extend_from_slice(&color);
    }

    // 4. krok box
    if let Some((start_time, ref entries)) = sample.karaoke {
        if entries.len() > u16::MAX as usize {
            return Err(Error::InvalidBox("too many karaoke entries"));
        }
        let size = 8u32.checked_add(6).and_then(|s| s.checked_add(entries.len() as u32 * 8)).ok_or(Error::InvalidBox("krok size overflow"))?;
        out.extend_from_slice(&size.to_be_bytes());
        out.extend_from_slice(b"krok");
        out.extend_from_slice(&start_time.to_be_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_be_bytes());
        for e in entries {
            out.extend_from_slice(&e.highlight_end_time.to_be_bytes());
            out.extend_from_slice(&e.start_char.to_be_bytes());
            out.extend_from_slice(&e.end_char.to_be_bytes());
        }
    }

    // 5. dlay box
    if let Some(delay) = sample.scroll_delay {
        out.extend_from_slice(&12u32.to_be_bytes());
        out.extend_from_slice(b"dlay");
        out.extend_from_slice(&delay.to_be_bytes());
    }

    // 6. href boxes
    for link in &sample.hypertext {
        let url_bytes = link.url.as_bytes();
        let alt_bytes = link.alt_string.as_bytes();
        let ulen = url_bytes.len().min(255) as u8;
        let alen = alt_bytes.len().min(255) as u8;
        let size = 8u32 + 4 + 1 + ulen as u32 + 1 + alen as u32;
        out.extend_from_slice(&size.to_be_bytes());
        out.extend_from_slice(b"href");
        out.extend_from_slice(&link.start_char.to_be_bytes());
        out.extend_from_slice(&link.end_char.to_be_bytes());
        out.push(ulen);
        out.extend_from_slice(&url_bytes[..ulen as usize]);
        out.push(alen);
        out.extend_from_slice(&alt_bytes[..alen as usize]);
    }

    // 7. tbox box
    if let Some(tb) = sample.text_box {
        out.extend_from_slice(&16u32.to_be_bytes());
        out.extend_from_slice(b"tbox");
        out.extend_from_slice(&tb.top.to_be_bytes());
        out.extend_from_slice(&tb.left.to_be_bytes());
        out.extend_from_slice(&tb.bottom.to_be_bytes());
        out.extend_from_slice(&tb.right.to_be_bytes());
    }

    // 8. blnk boxes
    for &(start, end) in &sample.blinking {
        out.extend_from_slice(&12u32.to_be_bytes());
        out.extend_from_slice(b"blnk");
        out.extend_from_slice(&start.to_be_bytes());
        out.extend_from_slice(&end.to_be_bytes());
    }

    // 9. twrp box
    if let Some(flag) = sample.wrap_flag {
        out.extend_from_slice(&9u32.to_be_bytes());
        out.extend_from_slice(b"twrp");
        out.push(flag);
    }

    Ok(out)
}

fn push_escaped_char(out: &mut String, ch: char) {
    match ch {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        _ => out.push(ch),
    }
}

/// Convert a [`TextSample`] into HTML-style formatted markup (`<b>`, `<i>`, `<u>`, `<font color="...">`).
pub fn sample_to_markup(sample: &TextSample) -> String {
    if sample.styles.is_empty() {
        let mut out = String::with_capacity(sample.text.len());
        for ch in sample.text.chars() {
            push_escaped_char(&mut out, ch);
        }
        return out;
    }

    let chars: Vec<char> = sample.text.chars().collect();
    let n = chars.len();
    if n == 0 {
        return String::new();
    }

    let mut starts: std::collections::HashMap<u16, Vec<&StyleRecord>> = std::collections::HashMap::new();
    let mut ends: std::collections::HashMap<u16, Vec<&StyleRecord>> = std::collections::HashMap::new();
    for st in &sample.styles {
        if st.end_char > st.start_char {
            starts.entry(st.start_char).or_default().push(st);
            ends.entry(st.end_char).or_default().push(st);
        }
    }

    let mut out = String::with_capacity(sample.text.len() + sample.styles.len() * 16);

    for (i, &ch) in chars.iter().enumerate() {
        let char_idx = i as u16;

        // Open tags for any style starting at char_idx
        if let Some(list) = starts.get(&char_idx) {
            for st in list {
                let has_color = st.text_color_rgba != [255, 255, 255, 255];
                if has_color {
                    out.push_str(&format!(
                        "<font color=\"#{:02x}{:02x}{:02x}\">",
                        st.text_color_rgba[0], st.text_color_rgba[1], st.text_color_rgba[2]
                    ));
                }
                if st.is_bold() {
                    out.push_str("<b>");
                }
                if st.is_italic() {
                    out.push_str("<i>");
                }
                if st.is_underline() {
                    out.push_str("<u>");
                }
            }
        }

        push_escaped_char(&mut out, ch);

        // Close tags for any style ending at char_idx + 1
        if let Some(list) = ends.get(&(char_idx + 1)) {
            for st in list.iter().rev() {
                if st.is_underline() {
                    out.push_str("</u>");
                }
                if st.is_italic() {
                    out.push_str("</i>");
                }
                if st.is_bold() {
                    out.push_str("</b>");
                }
                let has_color = st.text_color_rgba != [255, 255, 255, 255];
                if has_color {
                    out.push_str("</font>");
                }
            }
        }
    }

    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct CharFormatting {
    flags: u8,
    color: [u8; 4],
}

impl Default for CharFormatting {
    fn default() -> Self {
        Self { flags: 0, color: [255, 255, 255, 255] }
    }
}

/// Parse HTML-style formatted markup (`<b>`, `<i>`, `<u>`, `<font color="...">`) into a [`TextSample`].
pub fn markup_to_sample(markup: &str) -> TextSample {
    let mut bold_depth = 0usize;
    let mut italic_depth = 0usize;
    let mut underline_depth = 0usize;
    let mut color_stack: Vec<[u8; 4]> = Vec::new();

    let mut decoded_chars: Vec<char> = Vec::with_capacity(markup.len());
    let mut char_styles: Vec<CharFormatting> = Vec::with_capacity(markup.len());

    let mut chars = markup.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '<' {
            let mut tag = String::new();
            for next in chars.by_ref() {
                if next == '>' {
                    break;
                }
                tag.push(next);
            }
            let trimmed = tag.trim().to_ascii_lowercase();
            if trimmed == "b" {
                bold_depth += 1;
            } else if trimmed == "/b" {
                bold_depth = bold_depth.saturating_sub(1);
            } else if trimmed == "i" {
                italic_depth += 1;
            } else if trimmed == "/i" {
                italic_depth = italic_depth.saturating_sub(1);
            } else if trimmed == "u" {
                underline_depth += 1;
            } else if trimmed == "/u" {
                underline_depth = underline_depth.saturating_sub(1);
            } else if trimmed.starts_with("font") {
                if let Some(color) = parse_font_color(&tag) {
                    color_stack.push(color);
                }
            } else if trimmed == "/font" {
                color_stack.pop();
            }
        } else {
            let mut text_chunk = String::new();
            text_chunk.push(ch);
            while let Some(&next) = chars.peek() {
                if next == '<' {
                    break;
                }
                text_chunk.push(next);
                chars.next();
            }

            let decoded = crate::decode_entities(&text_chunk);
            let mut flags = 0u8;
            if bold_depth > 0 {
                flags |= STYLE_FLAG_BOLD;
            }
            if italic_depth > 0 {
                flags |= STYLE_FLAG_ITALIC;
            }
            if underline_depth > 0 {
                flags |= STYLE_FLAG_UNDERLINE;
            }
            let color = color_stack.last().copied().unwrap_or([255, 255, 255, 255]);
            let fmt = CharFormatting { flags, color };

            for c in decoded.chars() {
                decoded_chars.push(c);
                char_styles.push(fmt);
            }
        }
    }

    let text: String = decoded_chars.iter().collect();

    // Group adjacent characters with identical non-default formatting into sorted, non-overlapping style runs.
    let mut styles = Vec::new();
    let mut run_start: Option<(usize, CharFormatting)> = None;

    for (idx, &fmt) in char_styles.iter().enumerate() {
        let is_styled = fmt.flags != 0 || fmt.color != [255, 255, 255, 255];
        match run_start {
            Some((start, current_fmt)) => {
                if fmt != current_fmt {
                    if current_fmt.flags != 0 || current_fmt.color != [255, 255, 255, 255] {
                        styles.push(StyleRecord {
                            start_char: start as u16,
                            end_char: idx as u16,
                            font_id: 1,
                            face_style_flags: current_fmt.flags,
                            font_size: 16,
                            text_color_rgba: current_fmt.color,
                        });
                    }
                    run_start = if is_styled { Some((idx, fmt)) } else { None };
                }
            }
            None => {
                if is_styled {
                    run_start = Some((idx, fmt));
                }
            }
        }
    }

    if let Some((start, fmt)) = run_start {
        styles.push(StyleRecord {
            start_char: start as u16,
            end_char: char_styles.len() as u16,
            font_id: 1,
            face_style_flags: fmt.flags,
            font_size: 16,
            text_color_rgba: fmt.color,
        });
    }

    TextSample { text, styles, ..Default::default() }
}

/// Helper to parse color attribute from `<font color="...">` tag.
fn parse_font_color(tag: &str) -> Option<[u8; 4]> {
    let lower = tag.to_ascii_lowercase();
    let col_idx = lower.find("color=")?;
    let rest = tag[col_idx + 6..].trim_start();
    let val = if let Some(stripped) = rest.strip_prefix('"') {
        let end = stripped.find('"')?;
        &stripped[..end]
    } else if let Some(stripped) = rest.strip_prefix('\'') {
        let end = stripped.find('\'')?;
        &stripped[..end]
    } else {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        &rest[..end]
    };

    let hex = val.strip_prefix('#').unwrap_or(val);
    if hex.len() == 6 {
        let r = u8::from_str_radix(hex.get(0..2)?, 16).ok()?;
        let g = u8::from_str_radix(hex.get(2..4)?, 16).ok()?;
        let b = u8::from_str_radix(hex.get(4..6)?, 16).ok()?;
        Some([r, g, b, 255])
    } else if hex.len() == 8 {
        let r = u8::from_str_radix(hex.get(0..2)?, 16).ok()?;
        let g = u8::from_str_radix(hex.get(2..4)?, 16).ok()?;
        let b = u8::from_str_radix(hex.get(4..6)?, 16).ok()?;
        let a = u8::from_str_radix(hex.get(6..8)?, 16).ok()?;
        Some([r, g, b, a])
    } else {
        None
    }
}

/// Convert a caption [`Cue`] into a [`TextSample`].
pub fn cue_to_sample(cue: &Cue) -> TextSample {
    markup_to_sample(&cue.text)
}

/// Convert a [`TextSample`] into a caption [`Cue`] spanning `start` to `end`.
pub fn sample_to_cue(sample: &TextSample, start: Tick, end: Tick) -> Cue {
    Cue { start, end, text: sample_to_markup(sample), speaker: None, id: None, settings: String::new() }
}

/// Decode a stream of timed text samples into a [`Document`].
pub fn decode_timed_samples<'a, I>(doc: &mut Document, samples: I, timescale: u32) -> Result<()>
where
    I: IntoIterator<Item = (u64, u32, &'a [u8])>,
{
    decode_timed_samples_with_config(doc, samples, timescale, None)
}

/// Decode a stream of timed text samples into a [`Document`], applying an optional sample description configuration.
pub fn decode_timed_samples_with_config<'a, I>(doc: &mut Document, samples: I, timescale: u32, config: Option<&Tx3gConfig>) -> Result<()>
where
    I: IntoIterator<Item = (u64, u32, &'a [u8])>,
{
    if timescale == 0 {
        return Ok(());
    }
    let ts = timescale as i128;
    for (pts, duration, data) in samples {
        let mut sample = read_sample(data)?;
        if sample.text.trim().is_empty() {
            continue;
        }
        if sample.styles.is_empty()
            && let Some(cfg) = config
            && (cfg.default_style.face_style_flags != 0 || cfg.default_style.text_color_rgba != [255, 255, 255, 255])
        {
            sample.styles.push(StyleRecord {
                start_char: 0,
                end_char: sample.text.chars().count() as u16,
                font_id: cfg.default_style.font_id,
                face_style_flags: cfg.default_style.face_style_flags,
                font_size: cfg.default_style.font_size,
                text_color_rgba: cfg.default_style.text_color_rgba,
            });
        }
        let end_pts = pts.checked_add(duration as u64).ok_or(Error::InvalidBox("timestamp interval overflow"))?;
        let start_ticks = (pts as i128 * TICKS_PER_SECOND as i128 / ts) as i64;
        let end_ticks = (end_pts as i128 * TICKS_PER_SECOND as i128 / ts) as i64;
        doc.cues.push(sample_to_cue(&sample, Tick(start_ticks), Tick(end_ticks)));
    }
    Ok(())
}
