use super::{ByteOrder, LayerMask, MAX_DEPTH, PsdError, Reader, Result, TaggedBlock, Version, mask_parameter_bytes, uses_long_length};

/// Converts the tagged blocks after the 37724 header string from `from` to `to`.
pub fn transcode_blocks(data: &[u8], version: Version, from: ByteOrder, to: ByteOrder) -> Result<(Vec<u8>, Vec<String>)> {
    let mut t = Tx::new(data, version, from, to);
    t.blocks()?;
    Ok(t.finish())
}

/// [`transcode_blocks`] written after `prefix`, in one buffer sized for both.
pub fn transcode_blocks_after(prefix: &[u8], data: &[u8], version: Version, from: ByteOrder, to: ByteOrder) -> Result<(Vec<u8>, Vec<String>)> {
    let mut out = Vec::new();
    out.try_reserve_exact(prefix.len().saturating_add(data.len())).map_err(|_| PsdError::OutOfMemory("not enough memory for the layer data"))?;
    out.extend_from_slice(prefix);
    let mut t = Tx::appending(out, data, version, from, to);
    t.blocks()?;
    Ok(t.finish())
}

/// Converts a layer-info body (the data of a `Layr` / `Lr16` / `Lr32` block) between orders.
pub fn transcode_layer_info(data: &[u8], version: Version, from: ByteOrder, to: ByteOrder) -> Result<(Vec<u8>, Vec<String>)> {
    let mut t = Tx::new(data, version, from, to);
    t.layer_info()?;
    Ok(t.finish())
}

/// Converts image-resource blocks (a 34377 tag) between orders.
pub fn transcode_resources(data: &[u8], from: ByteOrder, to: ByteOrder) -> Result<(Vec<u8>, Vec<String>)> {
    let mut t = Tx::new(data, Version::Psd, from, to);
    t.resources()?;
    Ok(t.finish())
}

/// A block signature as stored in `little` or big-endian data. Big-endian data (the model)
/// also carries the `MIB8` / `46B8` markers of blocks kept from a little-endian file.
fn is_sig(b: &[u8], little: bool) -> bool {
    if little {
        b.starts_with(b"MIB8") || b.starts_with(b"46B8")
    } else {
        b.starts_with(b"8BIM") || b.starts_with(b"8B64") || TaggedBlock::is_foreign_order_signature(b)
    }
}

fn height(top: i32, bottom: i32) -> usize {
    usize::try_from(bottom.saturating_sub(top)).unwrap_or(0)
}

/// Per layer record: the channel (id, length) list and the heights of the layer, mask and
/// real-mask rects.
type RecordShape = (Vec<(i16, u64)>, [usize; 3]);

struct Tx<'a> {
    r: Reader<'a>,
    out: Vec<u8>,
    from: ByteOrder,
    to: ByteOrder,
    version: Version,
    warnings: Vec<String>,
    depth: usize,
}

macro_rules! copy_int {
    ($name:ident, $ty:ty) => {
        fn $name(&mut self) -> Result<$ty> {
            let v = self.r.$name()?;
            self.out.extend_from_slice(&if self.to.is_little() { v.to_le_bytes() } else { v.to_be_bytes() });
            Ok(v)
        }
    };
}

impl<'a> Tx<'a> {
    fn new(data: &'a [u8], version: Version, from: ByteOrder, to: ByteOrder) -> Self {
        Self::appending(Vec::with_capacity(data.len()), data, version, from, to)
    }

    /// A walker that appends to `out`.
    fn appending(out: Vec<u8>, data: &'a [u8], version: Version, from: ByteOrder, to: ByteOrder) -> Self {
        let r = if from.is_little() { Reader::new_little(data) } else { Reader::new(data) };
        Tx { r, out, from, to, version, warnings: Vec::new(), depth: 0 }
    }

    fn finish(self) -> (Vec<u8>, Vec<String>) {
        (self.out, self.warnings)
    }

    /// A walker over `data` (a nested structure) that writes into this walker's output: it
    /// takes the buffer, so a nested structure is written once, not copied up per level. Hand
    /// the buffer back with [`Self::adopt`] before returning, error or not.
    fn nested<'b>(&mut self, data: &'b [u8], depth: usize) -> Tx<'b> {
        let r = if self.from.is_little() { Reader::new_little(data) } else { Reader::new(data) };
        Tx { r, out: std::mem::take(&mut self.out), from: self.from, to: self.to, version: self.version, warnings: Vec::new(), depth }
    }

    /// Takes the output buffer back from a [`Self::nested`] walker.
    fn adopt(&mut self, child: &mut Tx<'_>) {
        self.out = std::mem::take(&mut child.out);
    }

    fn same_order(&self) -> bool {
        self.from == self.to
    }

    fn remaining(&self) -> usize {
        self.r.remaining()
    }

    copy_int!(u16, u16);
    copy_int!(i16, i16);
    copy_int!(u32, u32);
    copy_int!(i32, i32);
    copy_int!(u64, u64);
    copy_int!(i64, i64);
    copy_int!(f64, f64);

    fn u8(&mut self) -> Result<u8> {
        let v = self.r.u8()?;
        self.out.push(v);
        Ok(v)
    }

    fn f32(&mut self) -> Result<()> {
        let b = self.r.array::<4>()?;
        let v = if self.from.is_little() { f32::from_le_bytes(b) } else { f32::from_be_bytes(b) };
        self.out.extend_from_slice(&if self.to.is_little() { v.to_le_bytes() } else { v.to_be_bytes() });
        Ok(())
    }

    /// Bytes copied as they are.
    fn raw(&mut self, n: usize) -> Result<()> {
        let b = self.r.bytes(n)?;
        self.out.extend_from_slice(b);
        Ok(())
    }

    fn rest(&mut self) -> Result<()> {
        let n = self.remaining();
        self.raw(n)
    }

    /// A four-character signature or key: stored reversed in little-endian data. Returns the
    /// canonical (big-endian) spelling.
    fn key(&mut self) -> Result<[u8; 4]> {
        let mut k = self.r.array::<4>()?;
        if self.from.is_little() {
            k.reverse();
        }
        let mut w = k;
        if self.to.is_little() {
            w.reverse();
        }
        self.out.extend_from_slice(&w);
        Ok(k)
    }

    fn len(&mut self, long: bool) -> Result<u64> {
        if long { self.u64() } else { Ok(u64::from(self.u32()?)) }
    }

    fn u16s(&mut self, n: usize) -> Result<()> {
        for _ in 0..n {
            self.u16()?;
        }
        Ok(())
    }

    /// Up to `n` `u16`s: a block shorter than its full layout keeps what it has.
    fn u16s_upto(&mut self, n: usize) -> Result<()> {
        for _ in 0..n {
            if self.remaining() < 2 {
                break;
            }
            self.u16()?;
        }
        Ok(())
    }

    fn i32s(&mut self, n: usize) -> Result<()> {
        for _ in 0..n {
            self.i32()?;
        }
        Ok(())
    }

    fn f64s(&mut self, n: usize) -> Result<()> {
        for _ in 0..n {
            self.f64()?;
        }
        Ok(())
    }

    /// `u16`s to the end (a lone odd byte is copied).
    fn u16s_to_end(&mut self) -> Result<()> {
        while self.remaining() >= 2 {
            self.u16()?;
        }
        self.rest()
    }

    fn u32s_to_end(&mut self) -> Result<()> {
        while self.remaining() >= 4 {
            self.u32()?;
        }
        self.rest()
    }

    /// A Photoshop Unicode string: u32 count + UTF-16 units.
    fn unicode(&mut self) -> Result<()> {
        let n = self.u32()?;
        self.r.check_count(u64::from(n), 2)?;
        self.u16s(n as usize)
    }

    /// A Pascal string padded to `align`.
    fn pascal(&mut self, align: usize) -> Result<()> {
        let n = usize::from(self.u8()?);
        self.raw(n)?;
        let total = 1 + n;
        self.raw((align - total % align) % align)
    }

    /// Transcodes the next `len` bytes with `f`; whatever `f` leaves is copied verbatim.
    fn region(&mut self, len: u64, f: impl FnOnce(&mut Tx<'_>) -> Result<()>) -> Result<()> {
        let data = self.r.bytes_u64(len)?;
        let start = self.out.len();
        let mut child = self.nested(data, self.depth);
        let res = f(&mut child).and_then(|()| child.rest());
        self.adopt(&mut child);
        res?;
        if self.out.len() - start != data.len() {
            return Err(PsdError::invalid("byte-order conversion changed a structure's length"));
        }
        self.warnings.append(&mut child.warnings);
        Ok(())
    }

    /// Reads a length field (4 bytes, or 8 when `long`), transcodes that many bytes with `f`
    /// and writes the length of the result: blocks dropped inside shrink the field.
    fn sized_region(&mut self, long: bool, f: impl FnOnce(&mut Tx<'_>) -> Result<()>) -> Result<()> {
        self.sized_region_counting(long, false, f)
    }

    /// [`Self::sized_region`] for a u32 length that counts its own 4 bytes.
    fn sized_region_inclusive(&mut self, f: impl FnOnce(&mut Tx<'_>) -> Result<()>) -> Result<()> {
        self.sized_region_counting(false, true, f)
    }

    fn sized_region_counting(&mut self, long: bool, inclusive: bool, f: impl FnOnce(&mut Tx<'_>) -> Result<()>) -> Result<()> {
        let mut len = self.r.len_field(long)?;
        let at = self.out.len();
        self.out.extend_from_slice(if long { &[0u8; 8] } else { &[0u8; 4] });
        if inclusive {
            len = len.checked_sub(4).ok_or_else(|| PsdError::invalid("length shorter than its own field"))?;
        }
        let data = self.r.bytes_u64(len)?;
        let start = self.out.len();
        let mut child = self.nested(data, self.depth);
        let res = f(&mut child).and_then(|()| child.rest());
        self.adopt(&mut child);
        res?;
        self.warnings.append(&mut child.warnings);
        self.patch_len(at, long, (self.out.len() - start) as u64 + if inclusive { 4 } else { 0 })?;
        Ok(())
    }

    /// Tries `f` on the rest of the data; it counts as a match only when at most three
    /// trailing zero bytes are left. On a miss nothing is consumed or written.
    fn attempt(&mut self, f: impl FnOnce(&mut Tx<'_>) -> Result<()>) -> bool {
        let (reader, written) = (self.r.clone(), self.out.len());
        let ok = f(self).is_ok() && self.remaining() <= 3 && self.r.peek_rest().iter().all(|&b| b == 0);
        if !ok {
            self.r = reader;
            self.out.truncate(written);
        }
        ok
    }

    /// Blocks whose layout is not known by key are very often a versioned descriptor,
    /// optionally behind a u32 version (every Photoshop block introduced since CS6 is):
    /// recognized by parsing to the end.
    fn descriptor_shaped(&mut self) -> bool {
        self.attempt(|t| t.versioned_descriptor())
            || self.attempt(|t| {
                t.u32()?;
                t.versioned_descriptor()
            })
    }

    /// Writes `n` into the length placeholder at `at`.
    fn patch_len(&mut self, at: usize, long: bool, n: u64) -> Result<()> {
        if long {
            let b = if self.to.is_little() { n.to_le_bytes() } else { n.to_be_bytes() };
            if let Some(slot) = self.out.get_mut(at..at + 8) {
                slot.copy_from_slice(&b);
            }
        } else {
            let n = u32::try_from(n).map_err(|_| PsdError::LimitExceeded("structure too large for a 32-bit length"))?;
            let b = if self.to.is_little() { n.to_le_bytes() } else { n.to_be_bytes() };
            if let Some(slot) = self.out.get_mut(at..at + 4) {
                slot.copy_from_slice(&b);
            }
        }
        Ok(())
    }

    /// An RLE row-count table (`rows` entries of 2 bytes, 4 in PSB data).
    fn rle_counts(&mut self, rows: usize) -> Result<()> {
        for _ in 0..rows {
            if self.version.is_psb() {
                if self.remaining() < 4 {
                    break;
                }
                self.u32()?;
            } else {
                if self.remaining() < 2 {
                    break;
                }
                self.u16()?;
            }
        }
        Ok(())
    }

    /// Encoded channel data for a plane of `rows` rows: the compression field, then the RLE
    /// row counts (container order) and the samples (always big-endian: copied).
    fn channel_data(&mut self, rows: usize, long_counts: bool) -> Result<()> {
        let saved = self.version;
        if long_counts {
            self.version = Version::Psb;
        }
        let compression = self.u16()?;
        let res = if compression == 1 { self.rle_counts(rows) } else { Ok(()) };
        self.version = saved;
        res?;
        self.rest()
    }

    // ---- image resources (34377) ----

    fn resources(&mut self) -> Result<()> {
        while self.remaining() >= 12 && is_sig(self.r.peek_rest(), self.from.is_little()) {
            self.key()?;
            let id = self.u16()?;
            self.pascal(2)?;
            let odd = self.r.clone().u32()? % 2 == 1;
            self.sized_region(false, |t| t.resource_data(id))?;
            if odd && self.remaining() > 0 {
                self.raw(1)?;
            }
        }
        self.rest()
    }

    /// Typed numeric resources; every other payload is a byte blob (ICC, XMP, EXIF, …) or
    /// unknown and is copied as stored.
    fn resource_data(&mut self, id: u16) -> Result<()> {
        match id {
            // Resolution info: fixed 16.16 h res, u16 unit, u16 width unit, then the same for v.
            1005 => {
                self.u32()?;
                self.u16()?;
                self.u16()?;
                self.u32()?;
                self.u16()?;
                self.u16()?;
            }
            // Layer state (u16 target), layer group info (u16 per layer).
            1024 | 1026 => self.u16s_to_end()?,
            // Global angle / altitude.
            1037 | 1049 => {
                self.u32()?;
            }
            // Layer selection ids: u16 count + u32 ids.
            1069 => {
                self.u16()?;
                self.u32s_to_end()?;
            }
            // Version info: u32 version, u8 has real merged data, two Unicode strings, u32.
            1057 => {
                self.u32()?;
                self.u8()?;
                self.unicode()?;
                self.unicode()?;
                self.u32()?;
            }
            // Grid and guides: u32 version, 2 × u32 grid, u32 count, (u32 location, u8 direction)…
            1032 => {
                self.u32()?;
                self.u32()?;
                self.u32()?;
                let n = self.u32()?;
                for _ in 0..n {
                    if self.remaining() < 5 {
                        break;
                    }
                    self.u32()?;
                    self.u8()?;
                }
            }
            // Path resources: 26-byte records.
            2000..=2997 | 2999 => self.path_records()?,
            _ => {}
        }
        Ok(())
    }

    // ---- tagged blocks ----

    /// Tagged blocks to the end; bytes that do not form a block are copied.
    fn blocks(&mut self) -> Result<()> {
        loop {
            let rest = self.r.peek_rest();
            if rest.len() < 12 || !is_sig(rest, self.from.is_little()) {
                return self.rest();
            }
            self.block()?;
        }
    }

    fn block(&mut self) -> Result<()> {
        let before = self.out.len();
        // A `MIB8` / `46B8` marker in big-endian data: a block kept verbatim from a
        // little-endian file. Written back as it was into little-endian output (its `8BIM`
        // signature reversed, as every signature there), copied as is otherwise.
        if !self.from.is_little() && TaggedBlock::is_foreign_order_signature(self.r.peek_rest()) {
            let sig = self.r.array::<4>()?;
            let mut key = self.r.array::<4>()?;
            // Only reached with `from == Big`: the key is canonical here whatever the
            // target order. `TaggedBlock::write` sizes the length field from that same key.
            // Reversing it for the target order before the check
            // asked about a key `PSB_LONG_KEYS` never contains, so a PSB long key's 8-byte
            // length was read as 4 and every later block was copied unconverted.
            let long = uses_long_length(self.version, &key);
            let len = self.r.len_field(long)?;
            let data = self.r.bytes_u64(len)?;
            if self.to.is_little() {
                key.reverse();
            }
            self.out.extend_from_slice(&sig);
            self.out.extend_from_slice(&key);
            let n = self.out.len();
            self.out.extend_from_slice(if long { &[0u8; 8] } else { &[0u8; 4] });
            self.patch_len(n, long, len)?;
            self.out.extend_from_slice(data);
            let rest = self.r.peek_rest();
            let pad = (0..=3usize).find(|&k| k == rest.len() || (k < rest.len() && is_sig(&rest[k..], self.from.is_little()))).unwrap_or(0);
            let pad_bytes = self.r.bytes(pad)?;
            self.out.extend_from_slice(pad_bytes);
            return Ok(());
        }
        let sig_at = self.out.len();
        let sig = self.key()?;
        let key = self.key()?;
        let long = uses_long_length(self.version, &key);
        let len = self.r.len_field(long)?;
        let len_at = self.out.len();
        self.out.extend_from_slice(if long { &[0u8; 8] } else { &[0u8; 4] });
        let data = self.r.bytes_u64(len)?;
        // The body is transcoded straight into this output, after the length placeholder.
        let data_at = self.out.len();
        let mut child = self.nested(data, self.depth + 1);
        let handled = if child.depth > MAX_DEPTH {
            Err(PsdError::LimitExceeded("tagged block nesting too deep"))
        } else if data.is_empty() {
            Ok(true)
        } else {
            child.block_body(&key)
        };
        let rest = if matches!(handled, Ok(true)) { child.rest() } else { Ok(()) };
        self.adopt(&mut child);
        rest?;
        if !matches!(handled, Ok(true)) {
            // A body that was not transcoded leaves nothing: it is copied as stored or dropped.
            self.out.truncate(data_at);
        }
        // Padding: the smallest k in 0..=3 after which the next block (or the end) starts.
        let rest = self.r.peek_rest();
        let pad = (0..=3usize).find(|&k| k == rest.len() || (k < rest.len() && is_sig(&rest[k..], self.from.is_little()))).unwrap_or(0);
        let pad_bytes = self.r.bytes(pad)?;
        match handled {
            Ok(true) => {
                self.warnings.append(&mut child.warnings);
                self.patch_len(len_at, long, (self.out.len() - data_at) as u64)?;
                self.out.extend_from_slice(pad_bytes);
            }
            Ok(false) if self.same_order() => {
                self.patch_len(len_at, long, data.len() as u64)?;
                self.out.extend_from_slice(data);
                self.out.extend_from_slice(pad_bytes);
            }
            // Little-endian in, big-endian model: keep the bytes as they are, marked.
            Ok(false) if self.from.is_little() => {
                let marker: &[u8; 4] = if &sig == b"8B64" { b"46B8" } else { b"MIB8" };
                self.out[sig_at..sig_at + 4].copy_from_slice(marker);
                self.patch_len(len_at, long, data.len() as u64)?;
                self.out.extend_from_slice(data);
                self.out.extend_from_slice(pad_bytes);
            }
            Ok(false) => {
                self.out.truncate(before);
                self.warnings.push(format!(
                    "{} block ({} bytes) dropped: its layout is unknown and cannot be converted to the file's byte order",
                    String::from_utf8_lossy(&key),
                    data.len()
                ));
            }
            Err(e) if self.same_order() => return Err(PsdError::invalid(format!("{} block: {e}", String::from_utf8_lossy(&key)))),
            Err(e) if self.from.is_little() => {
                let marker: &[u8; 4] = if &sig == b"8B64" { b"46B8" } else { b"MIB8" };
                self.out[sig_at..sig_at + 4].copy_from_slice(marker);
                self.patch_len(len_at, long, data.len() as u64)?;
                self.out.extend_from_slice(data);
                self.out.extend_from_slice(pad_bytes);
                self.warnings.push(format!(
                    "{} block ({} bytes) kept as stored, in little-endian layout: {e}",
                    String::from_utf8_lossy(&key),
                    data.len()
                ));
            }
            Err(e) => {
                self.out.truncate(before);
                self.warnings.push(format!("{} block ({} bytes) dropped: {e}", String::from_utf8_lossy(&key), data.len()));
            }
        }
        Ok(())
    }

    /// Transcodes a block's data by key. `Ok(false)` for keys without a known layout.
    fn block_body(&mut self, key: &[u8; 4]) -> Result<bool> {
        match key {
            b"Layr" | b"Lr16" | b"Lr32" => self.layer_info()?,
            b"luni" => self.unicode()?,
            b"lsct" | b"lsdk" => {
                self.u32()?;
                if self.remaining() >= 8 {
                    self.key()?;
                    self.key()?;
                }
                if self.remaining() >= 4 {
                    self.u32()?;
                }
            }
            // lyvr layer version, sn2P and vowv (shape and vector flags).
            b"lyid" | b"lspf" | b"lyvr" | b"sn2P" | b"vowv" => {
                self.u32()?;
            }
            // Text-engine global data: a textual structure, no byte order.
            b"Txt2" => self.rest()?,
            b"lnsr" => {
                self.key()?;
            }
            // Advanced Blending flags and fill opacity: one byte, then padding (Adobe spec,
            // "Additional Layer Information"; `lmgm`/`vmgm` = layer/vector mask hides effects).
            b"clbl" | b"infx" | b"knko" | b"tsly" | b"lmgm" | b"vmgm" | b"iOpa" => {
                self.u8()?;
            }
            b"lclr" => self.u16s_upto(4)?,
            b"fxrp" => {
                while self.remaining() >= 8 {
                    self.f64()?;
                }
            }
            b"brst" => self.u32s_to_end()?,
            b"LMsk" => self.u16s_upto(6)?,
            b"FMsk" => self.u16s_to_end()?,
            b"shmd" => self.metadata_setting()?,
            // Effects (lfxs: the effects as applied to a group), content credentials (CAI).
            b"lfx2" | b"lmfx" | b"lfxs" | b"CAI " => {
                self.u32()?;
                self.versioned_descriptor()?;
            }
            b"SoCo" | b"GdFl" | b"PtFl" | b"vstk" | b"blwh" | b"vibA" | b"CgEd" | b"artb" | b"artd" | b"cinf" | b"cust" | b"PxSD" | b"GenI"
            | b"OCIO" | b"extn" | b"pths" => {
                self.versioned_descriptor()?;
            }
            // Color Lookup: u16 version, then the descriptor.
            b"clrL" => {
                self.u16()?;
                self.versioned_descriptor()?;
            }
            b"lnk2" | b"lnk3" | b"lnkD" | b"lnkE" => self.linked_layers()?,
            b"Anno" => self.annotations()?,
            b"vogk" => {
                self.u32()?;
                self.versioned_descriptor()?;
            }
            b"vscg" => {
                self.key()?;
                self.versioned_descriptor()?;
            }
            b"SoLd" | b"SoLE" => {
                self.key()?;
                self.u32()?;
                self.versioned_descriptor()?;
            }
            b"PlLd" => {
                self.key()?;
                self.u32()?;
                self.pascal(1)?;
                for _ in 0..4 {
                    self.u32()?;
                }
                self.f64s(8)?;
                self.u32()?;
                self.versioned_descriptor()?;
            }
            // Type: version, transform, text version + descriptor, warp version + descriptor,
            // and the bounds as four 32-bit values (the specification says doubles; Photoshop
            // writes 16 bytes).
            b"TySh" => self.type_layer()?,
            b"vmsk" | b"vsms" => {
                self.u32()?;
                self.u32()?;
                self.path_records()?;
            }
            b"Patt" | b"Pat2" | b"Pat3" => {
                while self.remaining() >= 4 {
                    let len = self.r.clone().u32()?;
                    if len == 0 {
                        self.u32()?;
                        continue;
                    }
                    self.sized_region(false, Tx::pattern)?;
                    // Each pattern is padded to a multiple of 4.
                    let pad = ((4 - len % 4) % 4) as usize;
                    self.raw(pad.min(self.remaining()))?;
                }
            }
            b"FEid" | b"FXid" | b"FELS" => self.filter_effects()?,
            b"lrFX" => self.legacy_effects()?,
            b"levl" => {
                self.u16()?;
                for _ in 0..29 * 5 {
                    if self.remaining() < 2 {
                        break;
                    }
                    self.u16()?;
                }
                if self.remaining() >= 4 {
                    self.key()?;
                }
                self.u16s_to_end()?;
            }
            b"curv" => self.curves()?,
            b"hue2" => {
                self.u16()?;
                self.u8()?;
                self.u8()?;
                self.u16s_to_end()?;
            }
            b"brit" => self.u16s_upto(3)?,
            b"thrs" | b"post" => {
                self.u16()?;
            }
            b"nvrt" => {}
            b"expA" => {
                self.u16()?;
                for _ in 0..3 {
                    self.f32()?;
                }
            }
            b"blnc" => self.u16s_upto(9)?,
            b"mixr" | b"selc" => self.u16s_to_end()?,
            b"phfl" => match self.u16()? {
                2 => {
                    self.u16s(5)?;
                    self.u32()?;
                    self.u8()?;
                }
                3 => {
                    for _ in 0..4 {
                        self.u32()?;
                    }
                    self.u8()?;
                }
                v => return Err(PsdError::Unsupported(format!("photo filter version {v}"))),
            },
            b"grdm" => self.gradient_map()?,
            _ if !self.same_order() => return Ok(self.descriptor_shaped()),
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// `lnk2` / `lnk3` / `lnkD` / `lnkE` (linked and embedded smart-object files): items of
    /// u64 length + data, each padded to 4. Item: type (`liFD` embedded, `liFE` external,
    /// `liFA` alias), version 1–7, Pascal uuid, Unicode name, file type and creator keys, u64
    /// data length, open-descriptor flag (+ descriptor), per-type fields, the file bytes for
    /// `liFD`, then the fields later versions added.
    fn linked_layers(&mut self) -> Result<()> {
        while self.remaining() >= 8 {
            let len = self.r.clone().u64()?;
            self.sized_region(true, |t| {
                let kind = t.key()?;
                let version = t.u32()?;
                t.pascal(1)?;
                t.unicode()?;
                t.key()?;
                t.key()?;
                let data_len = t.u64()?;
                if t.u8()? != 0 {
                    t.versioned_descriptor()?;
                }
                // An external file's link descriptor.
                if &kind == b"liFE" {
                    t.versioned_descriptor()?;
                }
                if &kind == b"liFE" && version > 3 {
                    t.i32()?;
                    for _ in 0..4 {
                        t.u8()?;
                    }
                    t.f64()?;
                }
                if &kind == b"liFE" {
                    t.u64()?;
                }
                if &kind == b"liFA" {
                    t.raw(8)?;
                }
                let data_len = usize::try_from(data_len).map_err(|_| PsdError::LimitExceeded("linked file length"))?;
                if &kind == b"liFD" {
                    t.raw(data_len)?;
                }
                // From version 3 an external file's bytes are kept too.
                if &kind == b"liFE" && version > 2 {
                    t.raw(data_len)?;
                }
                if version >= 5 {
                    t.unicode()?;
                }
                if version >= 6 {
                    t.f64()?;
                }
                if version >= 7 {
                    t.u8()?;
                }
                if &kind == b"liFE" && version == 2 {
                    t.raw(data_len)?;
                }
                Ok(())
            })?;
            let pad = ((4 - len % 4) % 4) as usize;
            self.raw(pad.min(self.remaining()))?;
        }
        Ok(())
    }

    /// `Anno`: u16 major, u16 minor, u32 count, then per annotation a self-counting u32
    /// length, type key, open and flags bytes, u16 optional blocks, icon and popup rects, a
    /// colour, three even-padded Pascal strings, and a self-counting data block (key, u32
    /// size, text with its own byte-order mark).
    fn annotations(&mut self) -> Result<()> {
        self.u16()?;
        self.u16()?;
        let n = self.u32()?;
        for _ in 0..n {
            if self.remaining() < 4 {
                break;
            }
            self.sized_region_inclusive(|t| {
                t.key()?;
                t.u8()?;
                t.u8()?;
                t.u16()?;
                t.i32s(8)?;
                t.u16s(5)?;
                t.pascal(2)?;
                t.pascal(2)?;
                t.pascal(2)?;
                t.sized_region_inclusive(|d| {
                    d.key()?;
                    d.u32()?;
                    Ok(())
                })
            })?;
        }
        Ok(())
    }

    // ---- layer info ----

    fn layer_info(&mut self) -> Result<()> {
        let count = self.i16()?;
        let n = usize::from(count.unsigned_abs());
        self.r.check_count(n as u64, 34)?;
        // Per layer: the channel ids and lengths, and the heights of the layer, mask and
        // real-mask rects (what each channel's RLE count table spans).
        let mut records: Vec<RecordShape> = Vec::with_capacity(n);
        for _ in 0..n {
            let top = self.i32()?;
            self.i32()?;
            let bottom = self.i32()?;
            self.i32()?;
            let nch = self.u16()?;
            if nch > crate::layer::MAX_LAYER_CHANNELS {
                return Err(PsdError::LimitExceeded("too many channels in layer"));
            }
            let mut chans = Vec::with_capacity(usize::from(nch));
            let psb = self.version.is_psb();
            for _ in 0..nch {
                let id = self.i16()?;
                let len = self.len(psb)?;
                chans.push((id, len));
            }
            let sig = self.key()?;
            if &sig != b"8BIM" {
                return Err(PsdError::InvalidSignature { expected: "8BIM (layer blend mode)", found: sig });
            }
            self.key()?;
            for _ in 0..4 {
                self.u8()?;
            }
            let mut heights = [height(top, bottom), 0, 0];
            let mut mask_heights = (0usize, 0usize);
            self.sized_region(false, |t| {
                let mut mh = (0, 0);
                t.sized_region(false, |m| {
                    mh = m.mask()?;
                    Ok(())
                })?;
                mask_heights = mh;
                // Blending ranges are bytes (black low/high, white low/high per channel), so
                // Photoshop stores them in the same order in either byte order: an Intel-order
                // layered TIFF holds the default ranges as `00 00 ff ff`. Swapping them as u32s
                // turned every range into `ff ff 00 00`, which hides every pixel of the layer.
                t.sized_region(false, |x| x.rest())?;
                t.pascal(4)?;
                t.blocks()
            })?;
            heights[1] = mask_heights.0;
            heights[2] = mask_heights.1;
            records.push((chans, heights));
        }
        for (chans, heights) in &records {
            for &(id, len) in chans {
                if len == 0 {
                    continue;
                }
                if len == 1 {
                    return Err(PsdError::invalid("channel data length 1"));
                }
                let rows = match id {
                    -2 => heights[1],
                    -3 => heights[2],
                    _ => heights[0],
                };
                self.region(len, |t| t.channel_data(rows, false))?;
            }
        }
        self.rest()
    }

    /// A layer mask record. Returns the heights of the mask and real-mask rects.
    fn mask(&mut self) -> Result<(usize, usize)> {
        let whole = self.r.peek_rest();
        if whole.is_empty() {
            return Ok((0, 0));
        }
        let top = self.i32()?;
        self.i32()?;
        let bottom = self.i32()?;
        self.i32()?;
        self.u8()?;
        let flags = self.u8()?;
        let mut real_h = 0;
        if self.remaining() <= 2 {
            return Ok((height(top, bottom), 0));
        }
        let has_parameters = flags & LayerMask::FLAG_PARAMETERS != 0;
        // The helper only looks at lengths and the parameter-flags byte, so it reads the
        // same in either order.
        let real_first = has_parameters && LayerMask::real_before_parameters(whole);
        let mut read_real = |t: &mut Tx<'_>| -> Result<()> {
            if t.remaining() >= 18 {
                t.u8()?;
                t.u8()?;
                let rt = t.i32()?;
                t.i32()?;
                let rb = t.i32()?;
                t.i32()?;
                real_h = height(rt, rb);
            }
            Ok(())
        };
        if real_first {
            read_real(self)?;
        }
        if has_parameters {
            let pf = self.u8()?;
            let need = mask_parameter_bytes(pf);
            if need > self.remaining() {
                return Err(PsdError::UnexpectedEof { offset: self.r.pos(), needed: need - self.remaining() });
            }
            if pf & 1 != 0 {
                self.u8()?;
            }
            if pf & 2 != 0 {
                self.f64()?;
            }
            if pf & 4 != 0 {
                self.u8()?;
            }
            if pf & 8 != 0 {
                self.f64()?;
            }
        }
        if !real_first {
            read_real(self)?;
        }
        Ok((height(top, bottom), real_h))
    }

    // ---- descriptors ----

    /// Photoshop's Intel-order TIFF type blocks can use PSD byte order for the
    /// text descriptor, then container order again for warp and bounds (photocraft#4758).
    fn type_layer(&mut self) -> Result<()> {
        self.u16()?;
        self.f64s(6)?;
        self.type_descriptor(50, ByteOrder::Big)?;
        self.type_descriptor(1, self.to)?;
        self.i32s(4)
    }

    fn type_descriptor(&mut self, version: u16, to: ByteOrder) -> Result<()> {
        let [hi, lo] = version.to_be_bytes();
        let from = if self.from.is_little() && self.r.peek_rest().starts_with(&[hi, lo, 0, 0, 0, 16]) { ByteOrder::Big } else { self.from };
        let data = self.r.peek_rest();
        let mut descriptor = Tx::new(data, self.version, from, to);
        descriptor.depth = self.depth;
        descriptor.u16()?;
        descriptor.versioned_descriptor()?;
        let used = data.len() - descriptor.remaining();
        self.r.bytes(used)?;
        self.warnings.append(&mut descriptor.warnings);
        self.out.extend_from_slice(&descriptor.out);
        Ok(())
    }

    fn versioned_descriptor(&mut self) -> Result<()> {
        self.u32()?;
        self.descriptor()
    }

    fn descriptor(&mut self) -> Result<()> {
        if self.depth > MAX_DEPTH {
            return Err(PsdError::LimitExceeded("descriptor nesting too deep"));
        }
        self.depth += 1;
        self.unicode()?;
        self.id()?;
        let n = self.u32()?;
        self.r.check_count(u64::from(n), 10)?;
        for _ in 0..n {
            self.id()?;
            self.value()?;
        }
        self.depth -= 1;
        Ok(())
    }

    fn id(&mut self) -> Result<()> {
        let n = self.u32()?;
        if n == 0 {
            self.key()?;
        } else {
            self.raw(usize::try_from(n).map_err(|_| PsdError::LimitExceeded("id length"))?)?;
        }
        Ok(())
    }

    fn class(&mut self) -> Result<()> {
        self.unicode()?;
        self.id()
    }

    fn value(&mut self) -> Result<()> {
        if self.depth > MAX_DEPTH {
            return Err(PsdError::LimitExceeded("descriptor nesting too deep"));
        }
        let ty = self.key()?;
        match &ty {
            b"obj " => {
                let n = self.u32()?;
                self.r.check_count(u64::from(n), 8)?;
                for _ in 0..n {
                    self.reference_item()?;
                }
            }
            b"Objc" | b"GlbO" => self.descriptor()?,
            b"VlLs" => {
                let n = self.u32()?;
                self.r.check_count(u64::from(n), 4)?;
                self.depth += 1;
                for _ in 0..n {
                    self.value()?;
                }
                self.depth -= 1;
            }
            b"doub" => {
                self.f64()?;
            }
            b"UntF" => {
                self.key()?;
                self.f64()?;
            }
            b"UnFl" => {
                self.key()?;
                let n = self.u32()?;
                self.r.check_count(u64::from(n), 8)?;
                self.f64s(n as usize)?;
            }
            b"TEXT" => self.unicode()?,
            b"enum" => {
                self.id()?;
                self.id()?;
            }
            b"long" => {
                self.i32()?;
            }
            b"comp" => {
                self.i64()?;
            }
            b"bool" => {
                self.u8()?;
            }
            b"type" | b"GlbC" => self.class()?,
            b"alis" | b"Pth " | b"tdta" => {
                let n = self.u32()?;
                self.raw(usize::try_from(n).map_err(|_| PsdError::LimitExceeded("descriptor data length"))?)?;
            }
            b"ObAr" => {
                self.u32()?;
                self.descriptor()?;
            }
            other => return Err(PsdError::Unsupported(format!("descriptor OSType {:?}", String::from_utf8_lossy(other)))),
        }
        Ok(())
    }

    fn reference_item(&mut self) -> Result<()> {
        let ty = self.key()?;
        match &ty {
            b"prop" => {
                self.class()?;
                self.id()?;
            }
            b"Clss" => self.class()?,
            b"Enmr" => {
                self.class()?;
                self.id()?;
                self.id()?;
            }
            b"rele" => {
                self.class()?;
                self.i32()?;
            }
            b"Idnt" | b"indx" => {
                self.class()?;
                self.u32()?;
            }
            b"name" => {
                self.class()?;
                self.unicode()?;
            }
            other => return Err(PsdError::Unsupported(format!("reference item type {:?}", String::from_utf8_lossy(other)))),
        }
        Ok(())
    }

    // ---- other block layouts ----

    /// `shmd`: u32 count, then per item signature, key, copy flag, 3 bytes, u32 length, data.
    /// An item whose layout is unknown is dropped on its own (and the count patched) rather
    /// than taking the whole block with it.
    fn metadata_setting(&mut self) -> Result<()> {
        let n = self.r.u32()?;
        self.r.check_count(u64::from(n), 16)?;
        let count_at = self.out.len();
        self.out.extend_from_slice(&[0; 4]);
        let mut kept = 0u32;
        for _ in 0..n {
            let before = self.out.len();
            self.key()?;
            let key = self.key()?;
            self.u8()?;
            self.raw(3)?;
            let same = self.same_order();
            let res = self.sized_region(false, |t| match &key {
                b"cmls" | b"mlst" | b"cust" | b"tmln" | b"sgrp" | b"cinf" => t.versioned_descriptor(),
                // mdyn (dynamic metadata) is one u32.
                b"mdyn" => t.u32s_to_end(),
                _ if same => Ok(()),
                _ if t.descriptor_shaped() => Ok(()),
                _ => Err(PsdError::Unsupported(format!("metadata item {:?}", String::from_utf8_lossy(&key)))),
            });
            match res {
                Ok(()) => kept += 1,
                Err(e) if same => return Err(e),
                Err(e) => {
                    self.out.truncate(before);
                    self.warnings.push(format!("shmd item {} dropped: {e}", String::from_utf8_lossy(&key)));
                }
            }
        }
        self.patch_len(count_at, false, u64::from(kept))
    }

    /// 26-byte path records: a u16 selector then, for knots, six fixed-point coordinates.
    fn path_records(&mut self) -> Result<()> {
        while self.remaining() >= 26 {
            let selector = self.u16()?;
            match selector {
                1 | 2 | 4 | 5 => self.i32s(6)?,
                // Length records: knot count, path operation, then fields Photoshop writes as
                // 16-bit values (zero in practice).
                0 | 3 => self.u16s(12)?,
                // Clipboard: four fixed-point coordinates and a resolution.
                7 => {
                    self.i32s(5)?;
                    self.raw(4)?;
                }
                8 => {
                    self.u16()?;
                    self.raw(22)?;
                }
                _ => self.raw(24)?,
            }
        }
        Ok(())
    }

    /// One pattern of a `Patt` block (the data after its u32 length).
    fn pattern(t: &mut Tx<'_>) -> Result<()> {
        t.u32()?;
        let mode = t.u32()?;
        t.u16()?;
        t.u16()?;
        t.unicode()?;
        t.pascal(1)?;
        if mode == 2 {
            t.raw(768)?;
            // Some writers follow the palette with 4 extra bytes before the array list version.
            let next_is_version = t.r.clone().u32().is_ok_and(|v| v == 3);
            if !next_is_version && t.remaining() >= 4 {
                t.raw(4)?;
            }
        }
        t.u32()?;
        t.sized_region(false, |v| {
            v.i32s(4)?;
            let count = v.u32()?;
            for _ in 0..count.saturating_add(2) {
                if v.remaining() < 4 {
                    break;
                }
                if v.u32()? == 0 {
                    continue;
                }
                if v.r.clone().u32()? == 0 {
                    v.u32()?;
                    continue;
                }
                v.sized_region(false, |c| {
                    c.u32()?;
                    let ct = c.i32()?;
                    c.i32()?;
                    let cb = c.i32()?;
                    c.i32()?;
                    c.u16()?;
                    let comp = c.u8()?;
                    if comp == 1 {
                        let saved = c.version;
                        c.version = Version::Psd;
                        let res = c.rle_counts(height(ct, cb));
                        c.version = saved;
                        res?;
                    }
                    Ok(())
                })?;
            }
            Ok(())
        })
    }

    /// `FEid` / `FXid`: version, then one section per smart object (u64 length, items with
    /// per-channel planes and 4-byte RLE counts), each padded to 4 bytes.
    fn filter_effects(&mut self) -> Result<()> {
        self.u32()?;
        loop {
            let len = self.r.clone().len_field(true)?;
            self.filter_effects_section()?;
            // Mirrors `FilterEffects::parse`: the last section's padding may be cut short.
            let pad = usize::try_from((4 - len % 4) % 4).unwrap_or(0).min(self.remaining());
            self.raw(pad)?;
            let rest = self.r.peek_rest();
            if rest.len() <= 3 && rest.iter().all(|&b| b == 0) {
                return Ok(());
            }
            if rest.len() < 8 {
                return Err(PsdError::invalid(format!("filter effects: {} unexpected bytes after the data", rest.len())));
            }
        }
    }

    fn filter_effects_section(&mut self) -> Result<()> {
        self.sized_region(true, |t| {
            while !t.r.is_empty() {
                t.pascal(1)?;
                t.u32()?;
                t.sized_region(true, |it| {
                    let top = it.i32()?;
                    it.i32()?;
                    let bottom = it.i32()?;
                    it.i32()?;
                    let rect_h = height(top, bottom);
                    it.u32()?;
                    let max = it.u32()?;
                    if max > crate::filter_effects::MAX_SLOTS {
                        return Err(PsdError::LimitExceeded("filter effects: too many channels"));
                    }
                    for _ in 0..max + 2 {
                        if it.u32()? != 0 {
                            it.effects_plane(rect_h)?;
                        }
                    }
                    Ok(())
                })?;
                if !t.r.is_empty() && t.u8()? != 0 {
                    let top = t.i32()?;
                    t.i32()?;
                    let bottom = t.i32()?;
                    t.i32()?;
                    t.effects_plane(height(top, bottom))?;
                }
            }
            Ok(())
        })
    }

    fn effects_plane(&mut self, rows: usize) -> Result<()> {
        self.sized_region(true, |p| p.channel_data(rows, true))
    }

    /// `lrFX`: version, count, then effects (`cmnS`, `dsdw`, `isdw`, `oglw`, `iglw`, `bevl`,
    /// `sofi`), each a signature, key, u32 size and fixed fields.
    fn legacy_effects(&mut self) -> Result<()> {
        self.u16()?;
        let n = self.u16()?;
        for _ in 0..n {
            if self.remaining() < 12 {
                break;
            }
            self.key()?;
            let key = self.key()?;
            self.sized_region(false, |t| {
                // A colour: u16 space + 4 × u16 components.
                let colour = |t: &mut Tx<'_>| t.u16s(5);
                let blend = |t: &mut Tx<'_>| -> Result<()> {
                    t.key()?;
                    t.key()?;
                    Ok(())
                };
                match &key {
                    b"cmnS" => {
                        t.u32()?;
                        t.u8()?;
                        t.u16()?;
                    }
                    b"dsdw" | b"isdw" => {
                        t.u32()?;
                        t.i32s(4)?;
                        colour(t)?;
                        blend(t)?;
                        t.u8()?;
                        t.u8()?;
                        t.u8()?;
                        if t.remaining() >= 10 {
                            colour(t)?;
                        }
                    }
                    b"oglw" => {
                        t.u32()?;
                        t.i32s(2)?;
                        colour(t)?;
                        blend(t)?;
                        t.u8()?;
                        t.u8()?;
                        if t.remaining() >= 10 {
                            colour(t)?;
                        }
                    }
                    b"iglw" => {
                        t.u32()?;
                        t.i32s(2)?;
                        colour(t)?;
                        blend(t)?;
                        t.u8()?;
                        t.u8()?;
                        if t.remaining() >= 11 {
                            t.u8()?;
                            colour(t)?;
                        }
                    }
                    b"bevl" => {
                        t.u32()?;
                        t.i32s(3)?;
                        blend(t)?;
                        blend(t)?;
                        colour(t)?;
                        colour(t)?;
                        for _ in 0..6 {
                            t.u8()?;
                        }
                        if t.remaining() >= 20 {
                            colour(t)?;
                            colour(t)?;
                        }
                    }
                    b"sofi" => {
                        t.u32()?;
                        blend(t)?;
                        colour(t)?;
                        t.u8()?;
                        t.u8()?;
                        if t.remaining() >= 10 {
                            colour(t)?;
                        }
                    }
                    _ => {}
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    /// `curv`: pad byte, version, channel bitmap, per set bit a point list; then an optional
    /// `Crv ` extension with per-channel point lists.
    fn curves(&mut self) -> Result<()> {
        self.u8()?;
        self.u16()?;
        let bits = self.u32()?;
        for bit in 0..32 {
            if bits & (1 << bit) == 0 {
                continue;
            }
            if self.remaining() < 2 {
                break;
            }
            let n = self.u16()?;
            self.u16s(usize::from(n) * 2)?;
        }
        if self.remaining() >= 10 {
            self.key()?;
            self.u16()?;
            let n = self.u32()?;
            for _ in 0..n {
                if self.remaining() < 4 {
                    break;
                }
                self.u16()?;
                let pts = self.u16()?;
                self.u16s(usize::from(pts) * 2)?;
            }
        }
        Ok(())
    }

    /// `grdm`: version, reverse, dither, [method key], name, colour stops, transparency
    /// stops, then the noise-gradient fields (a u32 seed among u16s).
    fn gradient_map(&mut self) -> Result<()> {
        let version = self.u16()?;
        self.u8()?;
        self.u8()?;
        if version == 3 {
            self.key()?;
        }
        self.unicode()?;
        let n = self.u16()?;
        for _ in 0..n {
            if self.remaining() < 20 {
                break;
            }
            self.u32()?;
            self.u32()?;
            self.u16s(6)?;
        }
        let nt = self.u16()?;
        for _ in 0..nt {
            if self.remaining() < 10 {
                break;
            }
            self.u32()?;
            self.u32()?;
            self.u16()?;
        }
        // expansion, interpolation, length, mode, u32 seed, then u16 fields.
        for _ in 0..4 {
            if self.remaining() < 2 {
                return Ok(());
            }
            self.u16()?;
        }
        if self.remaining() >= 4 {
            self.u32()?;
        }
        self.u16s_to_end()
    }
}
