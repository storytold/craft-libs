//! Bounded, borrowed views and serialization of PSD global `Anno` records.
//!
//! Framing follows PhotoCraft's annotation reader (Brandon Thomas, aa87012a), with
//! David Mann's minimum-length guard and the per-entry span fix from PhotoCraft
//! #11138. Coordinates, colour conversion, text decoding and popup defaults are
//! deliberately caller policy. Unknown records can be carried through byte-exact.

use crate::{PsdError, Result};

/// Explicit limits shared by readers and writers.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Maximum complete block size.
    pub bytes: usize,
    /// Maximum annotation count.
    pub records: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self { bytes: 16 * 1024 * 1024, records: 4096 }
    }
}

struct Reader<'a> {
    d: &'a [u8],
    p: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.p.checked_add(n).ok_or(PsdError::LimitExceeded("annotation offset"))?;
        let s = self.d.get(self.p..end).ok_or(PsdError::UnexpectedEof { offset: self.p, needed: n })?;
        self.p = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?.first().copied().unwrap_or_default())
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().map_err(|_| PsdError::invalid("annotation u16"))?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().map_err(|_| PsdError::invalid("annotation u32"))?))
    }
    fn key(&mut self) -> Result<[u8; 4]> {
        self.take(4)?.try_into().map_err(|_| PsdError::invalid("annotation key"))
    }
    fn rect(&mut self) -> Result<[i32; 4]> {
        Ok([self.u32()? as i32, self.u32()? as i32, self.u32()? as i32, self.u32()? as i32])
    }
    fn pascal(&mut self) -> Result<&'a [u8]> {
        let n = usize::from(self.u8()?);
        let bytes = self.take(n)?;
        if (n + 1) % 2 == 1 {
            self.take(1)?;
        }
        Ok(bytes)
    }
}

/// A borrowed record, including its original length, padding and unknown fields.
#[derive(Clone, Copy, Debug)]
pub struct Record<'a> {
    raw: &'a [u8],
}
impl<'a> Record<'a> {
    /// Complete self-counting record, suitable for lossless passthrough.
    pub fn raw(&self) -> &'a [u8] {
        self.raw
    }
    /// Parse text records; other record kinds remain opaque.
    pub fn text(&self) -> Result<Option<Text<'a>>> {
        let mut r = Reader { d: self.raw, p: 4 };
        if r.key()? != *b"txtA" {
            return Ok(None);
        }
        // Never read beyond this record, even if a text length points into its neighbour.
        let open = r.u8()?;
        let flags = r.u8()?;
        let optional_blocks = r.u16()?;
        let icon = r.rect()?;
        let popup = r.rect()?;
        let color_space = r.u16()?;
        let color = [r.u16()?, r.u16()?, r.u16()?, r.u16()?];
        let author = r.pascal()?;
        let name = r.pascal()?;
        let modified = r.pascal()?;
        let data_length = r.u32()?;
        let data_key = r.key()?;
        let size = usize::try_from(r.u32()?).map_err(|_| PsdError::LimitExceeded("annotation text"))?;
        let data = r.take(size)?;
        Ok(Some(Text { open, flags, optional_blocks, icon, popup, color_space, color, author, name, modified, data_length, data_key, data }))
    }
}

/// Wire-level text annotation. Text payload and Pascal strings remain bytes.
/// No text normalisation or application colour/coordinate policy is applied.
#[derive(Clone, Copy, Debug)]
pub struct Text<'a> {
    /// Open state byte.
    pub open: u8,
    /// Flags byte.
    pub flags: u8,
    /// Optional block flags.
    pub optional_blocks: u16,
    /// Icon rectangle, top/left/bottom/right.
    pub icon: [i32; 4],
    /// Popup rectangle, top/left/bottom/right.
    pub popup: [i32; 4],
    /// PSD colour space.
    pub color_space: u16,
    /// Raw colour components.
    pub color: [u16; 4],
    /// Pascal author bytes, maximum 255.
    pub author: &'a [u8],
    /// Pascal name bytes, maximum 255.
    pub name: &'a [u8],
    /// Pascal modification-date bytes, maximum 255.
    pub modified: &'a [u8],
    /// Original self-counting data length, exposed for diagnostics.
    /// The writer recomputes this field.
    pub data_length: u32,
    /// Usually `txtC`; retained without imposing application policy.
    pub data_key: [u8; 4],
    /// Payload, including any text byte-order mark.
    pub data: &'a [u8],
}

/// Parse complete framing before exposing records. Borrowed payloads do not allocate.
/// Trailing block bytes are accepted as in the existing PhotoCraft reader.
pub fn read(data: &[u8], limits: Limits) -> Result<Vec<Record<'_>>> {
    if data.len() > limits.bytes {
        return Err(PsdError::LimitExceeded("annotation bytes"));
    }
    let mut r = Reader { d: data, p: 0 };
    r.u16()?;
    r.u16()?;
    let count = usize::try_from(r.u32()?).map_err(|_| PsdError::LimitExceeded("annotation count"))?;
    if count > limits.records || count > data.len().saturating_sub(8) / 8 {
        return Err(PsdError::LimitExceeded("annotation count"));
    }
    let mut out = Vec::new();
    out.try_reserve_exact(count).map_err(|_| PsdError::OutOfMemory("annotation records"))?;
    for _ in 0..count {
        let start = r.p;
        let len = usize::try_from(r.u32()?).map_err(|_| PsdError::LimitExceeded("annotation length"))?;
        // David Mann's zero-length guard: a self-counting record must advance.
        if len < 8 {
            return Err(PsdError::invalid("annotation record shorter than its header"));
        }
        r.p = start;
        let record = Record { raw: r.take(len)? };
        record.text()?; // validate text spans, including data borrowed by later consumers
        out.push(record);
    }
    Ok(out)
}

fn append(out: &mut Vec<u8>, bytes: &[u8], limit: usize) -> Result<()> {
    let len = out.len().checked_add(bytes.len()).ok_or(PsdError::LimitExceeded("annotation bytes"))?;
    if len > limit {
        return Err(PsdError::LimitExceeded("annotation bytes"));
    }
    out.try_reserve(bytes.len()).map_err(|_| PsdError::OutOfMemory("annotation bytes"))?;
    out.extend_from_slice(bytes);
    Ok(())
}

fn pascal(out: &mut Vec<u8>, bytes: &[u8], limit: usize) -> Result<()> {
    let n = u8::try_from(bytes.len()).map_err(|_| PsdError::LimitExceeded("annotation Pascal string"))?;
    append(out, &[n], limit)?;
    append(out, bytes, limit)?;
    if (bytes.len() + 1) % 2 == 1 {
        append(out, &[0], limit)?;
    }
    Ok(())
}

/// Serialize one text record, computing both length fields. Caller supplies raw
/// text bytes and already-mapped colour/coordinates; nothing is truncated.
pub fn write_text(text: &Text<'_>, limits: Limits) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    append(&mut out, &[0; 4], limits.bytes)?;
    append(&mut out, b"txtA", limits.bytes)?;
    append(&mut out, &[text.open, text.flags], limits.bytes)?;
    append(&mut out, &text.optional_blocks.to_be_bytes(), limits.bytes)?;
    for value in text.icon.into_iter().chain(text.popup) {
        append(&mut out, &value.to_be_bytes(), limits.bytes)?;
    }
    append(&mut out, &text.color_space.to_be_bytes(), limits.bytes)?;
    for value in text.color {
        append(&mut out, &value.to_be_bytes(), limits.bytes)?;
    }
    for value in [text.author, text.name, text.modified] {
        pascal(&mut out, value, limits.bytes)?;
    }
    let n = u32::try_from(text.data.len()).map_err(|_| PsdError::LimitExceeded("annotation text"))?;
    let data_length = n.checked_add(12).ok_or(PsdError::LimitExceeded("annotation data length"))?;
    append(&mut out, &data_length.to_be_bytes(), limits.bytes)?;
    append(&mut out, &text.data_key, limits.bytes)?;
    append(&mut out, &n.to_be_bytes(), limits.bytes)?;
    append(&mut out, text.data, limits.bytes)?;
    let len = u32::try_from(out.len()).map_err(|_| PsdError::LimitExceeded("annotation record length"))?;
    out.get_mut(..4).ok_or_else(|| PsdError::invalid("annotation header"))?.copy_from_slice(&len.to_be_bytes());
    Ok(out)
}

/// Combine already-framed records into a version 2.1 block. Unknown record types
/// are preserved byte-exact; malformed text records are refused.
pub fn write(records: &[&[u8]], limits: Limits) -> Result<Vec<u8>> {
    if records.len() > limits.records {
        return Err(PsdError::LimitExceeded("annotation count"));
    }
    let count = u32::try_from(records.len()).map_err(|_| PsdError::LimitExceeded("annotation count"))?;
    let mut out = Vec::new();
    append(&mut out, &[0, 2, 0, 1], limits.bytes)?;
    append(&mut out, &count.to_be_bytes(), limits.bytes)?;
    for raw in records {
        let mut r = Reader { d: raw, p: 0 };
        let declared = usize::try_from(r.u32()?).map_err(|_| PsdError::LimitExceeded("annotation record length"))?;
        if raw.len() < 8 || declared != raw.len() {
            return Err(PsdError::invalid("annotation record length mismatch"));
        }
        Record { raw }.text()?;
        append(&mut out, raw, limits.bytes)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
