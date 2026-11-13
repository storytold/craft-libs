//! PNG via the `png` crate directly: 8/16-bit, Adam7 read (and write),
//! iCCP, eXIf, tEXt/zTXt/iTXt (XMP in `XML:com.adobe.xmp`), pHYs.

use std::io::{self, Cursor, Write};

use crate::Format;
use crate::error::CodecError;
use crate::fidelity::Plan;
use crate::image::{ChannelLayout, DecodeWarning, Image, Metadata, SampleType};
use crate::options::{EncodeOptions, Limits, PngCompression, PngStereo};

const F: Format = Format::Png;
pub(crate) const XMP_KEYWORD: &str = "XML:com.adobe.xmp";
const METERS_PER_INCH: f32 = 0.0254;

fn err(e: impl std::fmt::Display) -> CodecError {
    CodecError::malformed(F, e)
}

/// Pixels per inch for `pHYs` pixels per metre. A whole number of ppi is stored rounded to whole
/// pixels per metre (300 ppi = 11811), so reading it back gives 299.9994: the whole number that
/// rounds to the stored value is the resolution that was meant, as Photoshop reads it.
pub(crate) fn dpi_of_ppm(ppm: u32) -> f32 {
    let dpi = f64::from(ppm) * f64::from(METERS_PER_INCH);
    let whole = dpi.round();
    if (whole / f64::from(METERS_PER_INCH)).round() == f64::from(ppm) { whole as f32 } else { dpi as f32 }
}

/// Decodes a whole PNG, by driving a [`PngDecodeRows`] to its end.
pub(crate) fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    PngDecodeRows::new(bytes, limits)?.step(&mut || true)?.ok_or_else(|| err("the decoder stopped before the image ended"))
}

/// PNG bytes shared between the decoder and whoever else keeps them (a placed file's embedded
/// source): cloning shares the buffer.
#[derive(Debug, Clone)]
pub struct SharedBytes(pub std::sync::Arc<Vec<u8>>);

impl AsRef<[u8]> for SharedBytes {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

/// The input of a [`PngDecodeRows`] decoder: a cursor that also tells where the reader stopped, which
/// the text chunks after the image are read up to.
struct Tracked<B> {
    cursor: Cursor<B>,
    at: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl<B: AsRef<[u8]>> Tracked<B> {
    fn sync(&self) {
        self.at.store(self.cursor.position(), std::sync::atomic::Ordering::Relaxed);
    }
}

impl<B: AsRef<[u8]>> std::io::Read for Tracked<B> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.cursor.read(buf)?;
        self.sync();
        Ok(n)
    }
}

impl<B: AsRef<[u8]>> std::io::BufRead for Tracked<B> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        self.cursor.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.cursor.consume(amt);
        self.sync();
    }
}

impl<B: AsRef<[u8]>> std::io::Seek for Tracked<B> {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        let at = self.cursor.seek(pos)?;
        self.sync();
        Ok(at)
    }
}

/// The decoder's input, shared between the reader and the metadata pass: moving an owned buffer in
/// doesn't keep a second copy of it.
struct SharedInput<B>(std::sync::Arc<B>);

impl<B> Clone for SharedInput<B> {
    fn clone(&self) -> Self {
        Self(std::sync::Arc::clone(&self.0))
    }
}

impl<B: AsRef<[u8]>> AsRef<[u8]> for SharedInput<B> {
    fn as_ref(&self) -> &[u8] {
        (*self.0).as_ref()
    }
}

/// A PNG decoded in resumable steps, for a caller that must not block for the whole image (the
/// web build, which has no threads): [`PngDecodeRows::step`] decodes rows until the caller says to
/// stop, and returns the [`Image`] once the last row and the chunks after it are read. [`decode`]
/// drives one to the end, so the two give the same image.
///
/// An interlaced (Adam7) PNG decodes in a single step: its passes land at stride positions all
/// over the image, so no row is final before the last pass.
pub struct PngDecodeRows<B: AsRef<[u8]>> {
    reader: png::Reader<Tracked<SharedInput<B>>>,
    at: std::sync::Arc<std::sync::atomic::AtomicU64>,
    bytes: SharedInput<B>,
    limits: Limits,
    buf: Vec<u8>,
    layout: ChannelLayout,
    sample: SampleType,
    /// The image's size (the first frame's, for an animation) and bytes per output row.
    width: u32,
    height: usize,
    line: usize,
    /// The decoder's whole output size, for an interlaced image's single frame decode.
    size: usize,
    rows: usize,
    /// For [`PngDecodeRows::step_stream`]: the first row of the group held in `buf`.
    group_start: usize,
    interlaced: bool,
    images: u64,
    /// The chunks after the pixels: scanned from `text_at` up to `text_end`, a batch at a time,
    /// with `text` keeping what was accepted and the decompressor's state.
    text_at: usize,
    text_end: usize,
    text: TextScan,
    /// True once the pixel reader has been finished and the tail is known.
    tail: bool,
    done: bool,
    /// Test-only count of the pixel finalization: metadata yields must not repeat it.
    #[cfg(test)]
    pixel_finalizations: u32,
}

impl<B: AsRef<[u8]>> PngDecodeRows<B> {
    /// Reads the header and refuses what [`decode`] refuses before any pixel is decoded: sizes
    /// past `limits`, and a file too small for the image it declares.
    pub fn new(bytes: B, limits: &Limits) -> Result<Self, CodecError> {
        Self::open(bytes, limits, true)
    }

    /// [`PngDecodeRows::new`], optionally without reserving the whole image: a caller that hands
    /// the rows on as they come ([`decode_rows`]) never holds them all.
    fn open(bytes: B, limits: &Limits, reserve: bool) -> Result<Self, CodecError> {
        let at = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let bytes = SharedInput(std::sync::Arc::new(bytes));
        let input = Tracked { cursor: Cursor::new(bytes.clone()), at: at.clone() };
        let mut decoder = png::Decoder::new_with_limits(input, png::Limits { bytes: limits.alloc_usize() });
        decoder.set_transformations(png::Transformations::EXPAND);
        // png's lazy text get_text() inflates without consulting its allocation limit.
        // Read text separately below, without retaining compressed copies in the decoder.
        decoder.set_ignore_text_chunk(true);
        {
            let info = decoder.read_header_info().map_err(map_png_err)?;
            let (w, h) = info.size();
            // Size checks now; the allocation budget once the output layout (channels and sample
            // size) is known below, so a Gray8 image isn't charged for RGBA16.
            limits.check_size(w, h)?;
            // zlib never beats 1032:1; refuse a file too small for the declared image before the
            // output buffer exists. Measured on the stored samples (before palette/bit expansion).
            let stored = u64::from(w).saturating_mul(u64::from(h)).saturating_mul(info.bits_per_pixel() as u64).div_ceil(8);
            Limits::check_plausible(stored, bytes.as_ref().len(), 1100)?;
        }
        let reader = decoder.read_info().map_err(map_png_err)?;
        // APNG: the IDAT image is the first frame, or an extra default image when no fcTL precedes it.
        let images = reader.info().animation_control.map_or(1, |a| u64::from(a.num_frames) + u64::from(reader.info().frame_control.is_none()));
        let (color, depth) = reader.output_color_type();
        let layout = match color {
            png::ColorType::Grayscale => ChannelLayout::Gray,
            png::ColorType::GrayscaleAlpha => ChannelLayout::GrayA,
            png::ColorType::Rgb => ChannelLayout::Rgb,
            png::ColorType::Rgba => ChannelLayout::Rgba,
            png::ColorType::Indexed => return Err(err("palette was not expanded")),
        };
        let sample = match depth {
            png::BitDepth::Eight => SampleType::U8,
            png::BitDepth::Sixteen => SampleType::U16,
            d => return Err(err(format!("unexpected output bit depth {d:?}"))),
        };
        // The frame the pixels are of: an fcTL before the IDAT overrides the header's size.
        let info = reader.info();
        let (width, height) = info.frame_control.map_or_else(|| info.size(), |f| (f.width, f.height));
        let line = reader.output_line_size(width).ok_or_else(|| err("image too large"))?;
        // The output layout is known: charge its real bytes per pixel, not RGBA16's worst case.
        limits.check(width, height, layout, sample)?;
        let height = usize::try_from(height).map_err(|_| err("image too large"))?;
        if width == 0 || height == 0 {
            return Err(err("the image is empty"));
        }
        let size = reader.output_buffer_size().ok_or_else(|| err("image too large"))?;
        if line.checked_mul(height).is_none_or(|needed| needed > size) {
            return Err(err("image too large"));
        }
        // Reserve without touching: each row is zero-filled as it is decoded, so the first slice
        // doesn't zero-fill the whole image (a stall on the web build).
        let mut buf = Vec::new();
        if reserve {
            buf.try_reserve_exact(size).map_err(|_| CodecError::LimitExceeded(format!("cannot allocate {size} bytes for the image")))?;
        }
        let interlaced = info.interlaced;
        Ok(Self {
            reader,
            at,
            bytes,
            limits: *limits,
            buf,
            layout,
            sample,
            width,
            height,
            line,
            size,
            rows: 0,
            group_start: 0,
            interlaced,
            images,
            text_at: 8,
            text_end: 0,
            text: TextScan::default(),
            tail: false,
            done: false,
            #[cfg(test)]
            pixel_finalizations: 0,
        })
    }

    /// [`PngDecodeRows::new`] without reserving the whole image, for a caller that streams the rows
    /// ([`PngDecodeRows::step_stream`]) and so never holds them all. A caller that finds it must
    /// decode the whole image after all (an interlaced file) calls
    /// [`PngDecodeRows::reserve_output`] before [`PngDecodeRows::step`].
    pub fn new_streaming(bytes: B, limits: &Limits) -> Result<Self, CodecError> {
        Self::open(bytes, limits, false)
    }

    /// Reserves the buffer [`PngDecodeRows::step`] decodes the whole image into.
    pub fn reserve_output(&mut self) -> Result<(), CodecError> {
        let size = self.size;
        self.buf.try_reserve_exact(size).map_err(|_| CodecError::LimitExceeded(format!("cannot allocate {size} bytes for the image")))
    }

    /// Whether the rows come in final order, so [`PngDecodeRows::step_stream`] can hand them on:
    /// not an interlaced file, not an APNG (its first frame may be a sub-rectangle), and not one
    /// whose `eXIf` before the image data asks for another orientation. (An `eXIf` after the image
    /// data shows only in [`PngExtras::orientation`] at the end.)
    pub fn streamable(&self) -> bool {
        let info = self.reader.info();
        let upright = info.exif_metadata.as_deref().is_none_or(|e| crate::orientation::exif_orientation(e) == 1);
        !self.interlaced && info.animation_control.is_none() && upright
    }

    /// The size and pixel format the rows of [`PngDecodeRows::step_stream`] come in.
    pub fn header(&self) -> PngHeader {
        PngHeader { width: self.width, height: u32::try_from(self.height).unwrap_or(u32::MAX), layout: self.layout, sample: self.sample }
    }

    /// The image's width and height in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, u32::try_from(self.height).unwrap_or(u32::MAX))
    }

    /// How much of the image is decoded, `0.0..=1.0`.
    pub fn progress(&self) -> f32 {
        if self.done { 1.0 } else { self.rows as f32 / self.height.max(1) as f32 }
    }

    /// Decodes at least one row (the whole image if it is interlaced), then goes on while `more()`
    /// returns true. `Some` once the image is complete, after which the decoder is spent.
    pub fn step(&mut self, more: &mut dyn FnMut() -> bool) -> Result<Option<Image>, CodecError> {
        if self.done {
            return Err(err("the image was already decoded"));
        }
        // Once the pixels are complete and the tail is known, a step only reads metadata:
        // running the pixel reader again after `begin_tail` would decode an Adam7 frame twice
        // or finish the image data again on every metadata yield.
        if !self.tail {
            if self.interlaced {
                // An Adam7 frame is decoded in one go, over the whole output buffer.
                self.buf.resize(self.size, 0);
                let out = self.reader.next_frame(&mut self.buf).map_err(map_png_err)?;
                self.buf.truncate(out.buffer_size());
                self.rows = self.height;
                if self.sample == SampleType::U16 {
                    to_native_endian(&mut self.buf);
                }
            }
            while self.rows < self.height {
                let at = self.rows.checked_mul(self.line).ok_or_else(|| err("image too large"))?;
                let end = at.checked_add(self.line).ok_or_else(|| err("image too large"))?;
                // Grow by one row at a time: the zero-fill of the output is spread over the decodes
                // instead of stalling the first slice.
                if self.buf.len() < end {
                    self.buf.resize(end, 0);
                }
                let row = self.buf.get_mut(at..end).ok_or_else(|| err("the image data is longer than the image"))?;
                self.reader.read_row(row).map_err(map_png_err)?.ok_or_else(|| err("the image data ended early"))?;
                if self.sample == SampleType::U16 {
                    to_native_endian(row);
                }
                self.rows += 1;
                if self.rows < self.height && !more() {
                    return Ok(None);
                }
            }
            if !self.interlaced {
                #[cfg(test)]
                {
                    self.pixel_finalizations += 1;
                }
                // No row is left: this reads on to the end of the image data.
                if self.reader.read_row(&mut []).map_err(map_png_err)?.is_some() {
                    return Err(err("the image has more rows than its header declares"));
                }
                self.buf.truncate(self.line.saturating_mul(self.height));
            }
            // The chunks after the pixels, one bounded batch at a time: a large text chunk never
            // makes one step block, and what was accepted stays accepted when a step stops.
            self.begin_tail()?;
        }
        while !self.text.done {
            self.text_batch()?;
            if !self.text.done && !more() {
                return Ok(None);
            }
        }
        self.done = true;
        self.finish().map(Some)
    }

    /// Decodes the rows of a [`streamable`](PngDecodeRows::streamable) file a group at a time:
    /// `sink(y0, data)` gets whole rows `y0..` in final order, `group` of them at a time (the last
    /// group may be shorter; 0 counts as 1), native-endian, exactly the samples [`decode`] would
    /// return, and the decoder keeps only that group. Decodes at least one row, then goes on while
    /// `more()` returns true; `Some` with the extras once the file is read to its end, with the
    /// resolution a format that lacks one takes from its EXIF or XMP. A damaged file fails with
    /// the error [`decode`] gives, after however many rows were delivered. An orientation (an
    /// `eXIf` after the image data may set one) is for the caller to apply.
    pub fn step_stream<E: From<CodecError>>(
        &mut self,
        group: u32,
        more: &mut dyn FnMut() -> bool,
        sink: &mut PngRowSink<'_, E>,
    ) -> Result<Option<PngExtras>, E> {
        if self.done {
            return Err(err("the image was already decoded").into());
        }
        let group = usize::try_from(group.max(1)).unwrap_or(usize::MAX);
        if !self.tail {
            while self.rows < self.height {
                let held = self.rows - self.group_start;
                let at = held.checked_mul(self.line).ok_or_else(|| err("image too large"))?;
                let end = at.checked_add(self.line).ok_or_else(|| err("image too large"))?;
                if self.buf.len() < end {
                    self.buf.try_reserve_exact(end - self.buf.len()).map_err(|_| CodecError::LimitExceeded("cannot allocate a group of rows".into()))?;
                    self.buf.resize(end, 0);
                }
                let row = self.buf.get_mut(at..end).ok_or_else(|| err("invalid PNG row group"))?;
                if self.reader.read_row(row).map_err(map_png_err)?.is_none() {
                    return Err(err("the image data ends before the last row").into());
                }
                if self.sample == SampleType::U16 {
                    to_native_endian(row);
                }
                self.rows += 1;
                if self.rows - self.group_start == group || self.rows == self.height {
                    let data = self.buf.get(..end).ok_or_else(|| err("invalid PNG row group"))?;
                    sink(u32::try_from(self.group_start).map_err(|_| err("image too large"))?, data)?;
                    self.group_start = self.rows;
                }
                if self.rows < self.height && !more() {
                    return Ok(None);
                }
            }
            // Past the last row: trailing damage fails the same way it does in `decode`.
            if self.reader.read_row(&mut []).map_err(map_png_err)?.is_some() {
                return Err(err("the image has more rows than its header declares").into());
            }
            self.begin_tail()?;
        }
        while !self.text.done {
            self.text_batch()?;
            if !self.text.done && !more() {
                return Ok(None);
            }
        }
        self.done = true;
        let mut extras = self.extras();
        // As `decode_as_with` does for every format: a resolution the format lacks comes from EXIF or XMP.
        if extras.meta.dpi.is_none() {
            extras.meta.dpi = crate::resolution::from_metadata(extras.meta.exif.as_deref(), extras.meta.xmp.as_deref());
        }
        Ok(Some(extras))
    }

    /// Finishes the pixel reader and fixes where the metadata scan ends. Tolerate malformed
    /// tails once pixels are complete, but never swallow an allocation-limit failure.
    fn begin_tail(&mut self) -> Result<(), CodecError> {
        if let Err(e @ png::DecodingError::LimitsExceeded) = self.reader.finish() {
            return Err(map_png_err(e));
        }
        self.text_end = usize::try_from(self.at.load(std::sync::atomic::Ordering::Relaxed)).map_err(|_| err("invalid PNG reader position"))?;
        self.text.remaining = self.limits.alloc_usize();
        self.tail = true;
        Ok(())
    }

    /// Reads one bounded batch of the chunks after the pixels: up to [`TEXT_CHUNK_BATCH`] chunks,
    /// or one batch of one text chunk's CRC or value while either is mid-stream.
    fn text_batch(&mut self) -> Result<(), CodecError> {
        let bytes = self.bytes.clone();
        let bytes = bytes.as_ref();
        if let Some(pending) = self.text.pending.take() {
            match pending {
                TextPending::Crc(p) => self.crc_batch(p, bytes)?,
                TextPending::Value(p) => match inflate_text(p, bytes)? {
                    TextBatch::More(p) => self.text.pending = Some(TextPending::Value(p)),
                    TextBatch::Finished(p) => {
                        let (keyword, keyword_len, kind, text) = (p.keyword, p.keyword_len, p.kind, p.out);
                        self.text.accept(keyword, keyword_len, kind, text)?;
                    }
                    TextBatch::Invalid(produced) => self.text.charge(produced)?,
                },
            }
            return Ok(());
        }
        let mut chunks = 0;
        while chunks < TEXT_CHUNK_BATCH && !self.text.done {
            if self.scan_text_chunk(bytes)? {
                break;
            }
            chunks += 1;
        }
        Ok(())
    }

    /// Hashes one bounded batch of a text chunk's data. Only a fully CRC-valid chunk is parsed
    /// into a value, so a broken chunk is still dropped before its keyword or flags are used.
    fn crc_batch(&mut self, mut p: PendingCrc, bytes: &[u8]) -> Result<(), CodecError> {
        let Some(left) = p.data.end.checked_sub(p.at) else { return Ok(()) };
        let take = left.min(TEXT_BATCH);
        let Some(end) = p.at.checked_add(take) else { return Ok(()) };
        let Some(slice) = bytes.get(p.at..end) else { return Ok(()) };
        p.crc.update(slice);
        p.at = end;
        if p.at < p.data.end {
            self.text.pending = Some(TextPending::Crc(p));
            return Ok(());
        }
        if p.crc.sum() != p.checksum {
            return Ok(());
        }
        let Some(data) = bytes.get(p.data.clone()) else { return Ok(()) };
        if let Some(value) = self.pending_text(p.kind, data, p.data.start)? {
            self.text.pending = Some(TextPending::Value(value));
        }
        Ok(())
    }

    /// The value of a CRC-valid text chunk, with its decoded keyword and where its bytes start.
    fn pending_text(&self, kind: [u8; 4], data: &[u8], data_at: usize) -> Result<Option<PendingText>, CodecError> {
        // A keyword is 1..=79 Latin-1 bytes, so only the start of the chunk can hold it.
        let Some(nul) = data.iter().take(80).position(|&b| b == 0) else { return Ok(None) };
        let Some(keyword) = data.get(..nul) else { return Ok(None) };
        let Some(value_start) = nul.checked_add(1) else { return Ok(None) };
        let Some(value) = data.get(value_start..) else { return Ok(None) };
        if keyword.is_empty() {
            return Ok(None);
        }
        let (value, compressed, latin1, kind) = match &kind {
            b"tEXt" => (value, false, true, TextKind::Plain),
            b"zTXt" => {
                let Some((&0, value)) = value.split_first() else { return Ok(None) };
                (value, true, true, TextKind::Compressed)
            }
            _ => {
                let Some(([flag, method], value)) = value.split_first_chunk::<2>() else { return Ok(None) };
                if *flag > 1 || (*flag == 1 && *method != 0) {
                    return Ok(None);
                }
                let Some((language, value)) = split_nul(value) else { return Ok(None) };
                let Some((translated, value)) = split_nul(value) else { return Ok(None) };
                if !language.is_ascii() || std::str::from_utf8(translated).is_err() {
                    return Ok(None);
                }
                (value, *flag == 1, false, TextKind::International)
            }
        };
        let keyword_len = latin1_len(keyword).ok_or_else(text_limit)?;
        let keyword: String = keyword.iter().map(|&b| char::from(b)).collect();
        let Some(value_at) = data_at.checked_add(data.len().saturating_sub(value.len())) else { return Ok(None) };
        let Some(value_end) = value_at.checked_add(value.len()) else { return Ok(None) };
        Ok(Some(PendingText {
            kind,
            keyword,
            keyword_len,
            limit: self.text.remaining.saturating_sub(keyword_len),
            input: value_at..value_end,
            at: value_at,
            latin1,
            decoder: compressed.then(|| flate2::Decompress::new(true)),
            // An uncompressed iTXt value's UTF-8 is checked batch by batch, before its budget.
            utf8: (!compressed && !latin1).then(Utf8Check::default),
            over: false,
            out: Vec::new(),
        }))
    }

    /// Reads the header of the next chunk, starting a text chunk's CRC (which then reads its
    /// bytes in batches) or stepping over any other. False when the scan can go on in this
    /// batch; true when a text chunk was started and its bytes need their own batches. Sets
    /// [`TextScan::done`] at the end of the tail, at `IEND`, or at a malformed one, as the
    /// previous one-pass reader did.
    fn scan_text_chunk(&mut self, bytes: &[u8]) -> Result<bool, CodecError> {
        let Some(rest) = bytes.get(self.text_at..self.text_end) else {
            self.text.done = true;
            return Ok(false);
        };
        let Some((length, rest)) = rest.split_first_chunk::<4>() else {
            self.text.done = true;
            return Ok(false);
        };
        let length = u32::from_be_bytes(*length);
        // PNG chunk lengths are at most 2^31-1. Broken tails remain tolerated.
        if length > i32::MAX as u32 {
            self.text.done = true;
            return Ok(false);
        }
        let Ok(length) = usize::try_from(length) else {
            self.text.done = true;
            return Ok(false);
        };
        let Some((kind, rest)) = rest.split_first_chunk::<4>() else {
            self.text.done = true;
            return Ok(false);
        };
        let Some((_data, rest)) = rest.split_at_checked(length) else {
            self.text.done = true;
            return Ok(false);
        };
        let Some((checksum, _)) = rest.split_first_chunk::<4>() else {
            self.text.done = true;
            return Ok(false);
        };
        let Some(next) = self.text_at.checked_add(12).and_then(|at| at.checked_add(length)).filter(|&at| at <= self.text_end) else {
            self.text.done = true;
            return Ok(false);
        };
        let Some(data_at) = self.text_at.checked_add(8) else {
            self.text.done = true;
            return Ok(false);
        };
        self.text_at = next;
        if kind == b"IEND" {
            self.text.done = true;
            return Ok(false);
        }
        if !matches!(kind, b"tEXt" | b"zTXt" | b"iTXt") {
            return Ok(false);
        }
        let mut crc = flate2::Crc::new();
        crc.update(kind);
        let Some(data_end) = data_at.checked_add(length) else {
            self.text.done = true;
            return Ok(false);
        };
        // The CRC of a large chunk is hashed a bounded batch at a time too, so one metadata
        // step never scans the whole chunk.
        self.text.pending =
            Some(TextPending::Crc(PendingCrc { kind: *kind, data: data_at..data_end, at: data_at, crc, checksum: u32::from_be_bytes(*checksum) }));
        Ok(true)
    }

    /// What the file holds besides its pixels, with what the metadata scan accepted.
    fn extras(&mut self) -> PngExtras {
        let info = self.reader.info();
        let icc = info.icc_profile.as_ref().map(|c| c.to_vec());
        let mut meta = Metadata { exif: info.exif_metadata.as_ref().map(|c| c.to_vec()), ..Default::default() };
        if let Some(d) = info.pixel_dims
            && d.unit == png::Unit::Meter
            && d.xppu > 0
            && d.yppu > 0
        {
            meta.dpi = Some((dpi_of_ppm(d.xppu), dpi_of_ppm(d.yppu)));
        }
        meta.text = std::mem::take(&mut self.text.plain);
        meta.text.extend(std::mem::take(&mut self.text.compressed));
        meta.text.extend(std::mem::take(&mut self.text.international));
        meta.xmp = self.text.xmp.take();
        let mut warnings = Vec::new();
        if self.images > 1 {
            warnings.push(DecodeWarning::MoreFrames { total: u32::try_from(self.images).ok() });
        }
        let orientation = meta.exif.as_deref().map_or(1, crate::orientation::exif_orientation);
        PngExtras { icc, meta, warnings, orientation }
    }

    /// The image itself, with what the metadata scan accepted.
    fn finish(&mut self) -> Result<Image, CodecError> {
        let height = u32::try_from(self.height).map_err(|_| err("image too large"))?;
        let mut img = Image::from_raw(self.width, height, self.layout, self.sample, std::mem::take(&mut self.buf))?;
        let extras = self.extras();
        img.icc = extras.icc;
        img.meta = extras.meta;
        img.warnings = extras.warnings;
        Ok(img)
    }
}

/// 16-bit samples from the file's big-endian bytes to the machine's.
fn to_native_endian(samples: &mut [u8]) {
    for c in samples.as_chunks_mut::<2>().0 {
        let v = u16::from_be_bytes([c[0], c[1]]);
        c.copy_from_slice(&v.to_ne_bytes());
    }
}

/// What a PNG holds besides its pixels. These can sit after the image data (`eXIf`, text), so
/// they are complete only once the pixels have been read.
pub struct PngExtras {
    pub icc: Option<Vec<u8>>,
    pub meta: Metadata,
    pub warnings: Vec<DecodeWarning>,
    /// The EXIF orientation (1-8) the pixels are stored in; 1 when there is none.
    pub orientation: u16,
}

/// Inspect APNG's declared images, including a separate default image, without decoding pixels.
pub(crate) fn has_additional_images(bytes: &[u8]) -> Result<bool, CodecError> {
    let limits = Limits::default();
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(bytes), png::Limits { bytes: limits.alloc_usize() });
    decoder.set_ignore_text_chunk(true);
    let reader = decoder.read_info().map_err(map_png_err)?;
    let info = reader.info();
    Ok(info.animation_control.is_some_and(|a| u64::from(a.num_frames) + u64::from(info.frame_control.is_none()) > 1))
}

/// The size and pixel format the rows of [`decode_rows`] come in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PngHeader {
    pub width: u32,
    pub height: u32,
    pub layout: ChannelLayout,
    pub sample: SampleType,
}

/// Where [`decode_rows`] sends rows: the index of the first, then whole rows.
pub type PngRowSink<'a, E> = dyn FnMut(u32, &[u8]) -> Result<(), E> + 'a;

/// Decodes a PNG a group of rows at a time, so the caller never needs the whole decoded image:
/// `begin` gets the header once and says whether to go on (after the same size and plausibility checks [`decode`] makes),
/// then `rows(y0, data)` gets whole rows `y0..` in final order, `group` of them at a time (the
/// last group may be shorter; a `group` of 0 counts as 1), native-endian, exactly the samples [`decode`] would return. Returns the extras once the file
/// has been read to its end, or `None` (having called neither closure) when the rows can't be
/// had in final order: an interlaced (Adam7) file, and an APNG, whose first frame may be a
/// sub-rectangle. A damaged file fails with the error [`decode`] gives, after however many rows
/// were delivered. The limits are checked before the first row; an orientation, which an `eXIf`
/// after the image data may set, is for the caller to apply.
pub(crate) fn decode_rows<E: From<CodecError>>(
    bytes: &[u8],
    limits: &Limits,
    group: u32,
    begin: &mut dyn FnMut(&PngHeader) -> Result<bool, E>,
    rows: &mut PngRowSink<'_, E>,
) -> Result<Option<PngExtras>, E> {
    // The limits and plausibility checks are made here, before any row; the output is not reserved.
    let mut dec = PngDecodeRows::new_streaming(bytes, limits)?;
    if !dec.streamable() || !begin(&dec.header())? {
        return Ok(None);
    }
    // With a `more` that always says go on, the first call decodes the whole file.
    dec.step_stream(group, &mut || true, rows)
}

/// The colour table of a palette PNG (`PLTE`) and the one entry its `tRNS` makes transparent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngPalette {
    pub colors: Vec<[u8; 3]>,
    pub transparent: Option<u8>,
}

/// The colour table of a palette PNG (colour type 3), read from the headers without decoding
/// pixels. `None` for other colour types, and for a `tRNS` with partial alpha or several
/// transparent entries: an Indexed Color document holds one transparent entry.
pub fn palette(bytes: &[u8]) -> Result<Option<PngPalette>, CodecError> {
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(bytes), png::Limits::default());
    decoder.set_ignore_text_chunk(true);
    let reader = decoder.read_info().map_err(map_png_err)?;
    let info = reader.info();
    if info.color_type != png::ColorType::Indexed {
        return Ok(None);
    }
    let Some(plte) = info.palette.as_deref() else { return Ok(None) };
    let colors = plte.as_chunks::<3>().0.to_vec();
    let mut transparent = None;
    for (i, &a) in info.trns.as_deref().unwrap_or_default().iter().enumerate() {
        match a {
            255 => {}
            0 if transparent.is_none() && i < colors.len() => transparent = u8::try_from(i).ok(),
            _ => return Ok(None),
        }
    }
    Ok(Some(PngPalette { colors, transparent }))
}

/// The stereo pairing a PNG declares in its `sTER` chunk (PNG Extensions 1.4.0, §3.6), read
/// from the headers without decoding pixels. `None` when the file has no CRC-valid `sTER` with a
/// defined mode byte. The first valid chunk wins, as a later one would contradict it.
pub fn stereo(bytes: &[u8]) -> Option<PngStereo> {
    let mut rest = bytes.get(8..)?;
    while let Some((length, tail)) = rest.split_first_chunk::<4>() {
        let Some((kind, tail)) = tail.split_first_chunk::<4>() else { break };
        let length = u32::from_be_bytes(*length);
        if length > i32::MAX as u32 {
            break;
        }
        let Ok(length) = usize::try_from(length) else { break };
        let Some(data) = tail.get(..length) else { break };
        let Some((checksum, next)) = tail.get(length..).and_then(<[u8]>::split_first_chunk::<4>) else { break };
        rest = next;
        if kind == b"IEND" {
            break;
        }
        if kind != b"sTER" || data.len() != 1 {
            continue;
        }
        let mut crc = flate2::Crc::new();
        crc.update(kind);
        crc.update(data);
        if crc.sum() != u32::from_be_bytes(*checksum) {
            continue;
        }
        return PngStereo::from_mode(data[0]);
    }
    None
}

fn map_png_err(e: png::DecodingError) -> CodecError {
    match e {
        png::DecodingError::LimitsExceeded => CodecError::LimitExceeded("PNG decoder memory limit".into()),
        e => err(e),
    }
}

fn text_limit() -> CodecError {
    CodecError::LimitExceeded("PNG decoded text metadata exceeds max_alloc".into())
}

fn split_nul(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let at = bytes.iter().position(|&b| b == 0)?;
    Some((bytes.get(..at)?, bytes.get(at.checked_add(1)?..)?))
}

fn latin1_len(bytes: &[u8]) -> Option<usize> {
    bytes.len().checked_add(bytes.iter().filter(|&&b| b >= 128).count())
}

/// The text chunks after the pixels, accepted one bounded batch at a time: one shared budget for
/// decoded UTF-8 keywords and values (including overwritten XMP), the CRC and decompressor of a
/// chunk kept between batches. Invalid ancillary text stays optional, as it was with png's text
/// decoder.
#[derive(Default)]
struct TextScan {
    /// UTF-8 bytes left of [`Limits::max_alloc`] for the text.
    remaining: usize,
    plain: Vec<(String, String)>,
    compressed: Vec<(String, String)>,
    international: Vec<(String, String)>,
    xmp: Option<String>,
    pending: Option<TextPending>,
    done: bool,
}

impl TextScan {
    /// Charges `produced` bytes to the shared budget: what a chunk inflated counts even when its
    /// text is then dropped (bad UTF-8, a corrupt or truncated stream), or every rejected chunk
    /// could inflate up to the whole remaining budget again.
    fn charge(&mut self, produced: usize) -> Result<(), CodecError> {
        self.remaining = self.remaining.checked_sub(produced).ok_or_else(text_limit)?;
        Ok(())
    }

    /// Keeps a decoded value, charging the keyword and text to the shared budget; invalid UTF-8
    /// text is dropped, but what it inflated is still charged.
    fn accept(&mut self, keyword: String, keyword_len: usize, kind: TextKind, out: Vec<u8>) -> Result<(), CodecError> {
        let produced = out.len();
        let Ok(text) = String::from_utf8(out) else { return self.charge(produced) };
        self.remaining = self.remaining.checked_sub(keyword_len).and_then(|left| left.checked_sub(text.len())).ok_or_else(text_limit)?;
        match kind {
            // Some writers put XMP in Latin-1 text chunks instead of the usual iTXt.
            // Treat the reserved keyword consistently, after the same budget checks.
            _ if keyword == XMP_KEYWORD => self.xmp = Some(text),
            TextKind::Plain => self.plain.push((keyword, text)),
            TextKind::Compressed => self.compressed.push((keyword, text)),
            TextKind::International => self.international.push((keyword, text)),
        }
        Ok(())
    }
}

/// Which list a text chunk is kept in, as the previous decoder grouped them by chunk type.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TextKind {
    Plain,
    Compressed,
    International,
}

/// A text chunk being read: its CRC first, then (when valid) its value.
enum TextPending {
    Crc(PendingCrc),
    Value(PendingText),
}

/// The CRC of a text chunk's data, hashed a bounded batch at a time before the chunk is parsed.
struct PendingCrc {
    kind: [u8; 4],
    /// The chunk's data in the file, and the next byte a batch hashes.
    data: std::ops::Range<usize>,
    at: usize,
    crc: flate2::Crc,
    checksum: u32,
}

/// A text chunk's value being read, with its position in the file and (when compressed) the
/// inflator kept between batches.
struct PendingText {
    kind: TextKind,
    keyword: String,
    keyword_len: usize,
    /// The most decoded bytes this value may take: the budget left minus its keyword.
    limit: usize,
    /// The value's bytes in the file (the compressed ones for zTXt and compressed iTXt).
    input: std::ops::Range<usize>,
    /// The next byte of `input` a batch reads or inflates.
    at: usize,
    latin1: bool,
    /// Some for a compressed value.
    decoder: Option<flate2::Decompress>,
    /// Some for an uncompressed iTXt value: its UTF-8 is checked before its bytes are charged.
    utf8: Option<Utf8Check>,
    /// The value has outgrown the budget but its UTF-8 validity is still being checked, so an
    /// invalid value is still dropped instead of failing the decode.
    over: bool,
    out: Vec<u8>,
}

/// Incremental UTF-8 validation across batches, so a value that spans batches is checked as a
/// whole before it can fail the budget. A sequence straddling two batches keeps its expected
/// continuation range here.
#[derive(Default)]
struct Utf8Check {
    /// Continuation bytes still needed, and the range the next one may take.
    need: u8,
    min: u8,
    max: u8,
}

impl Utf8Check {
    /// Feeds the next bytes of the value; false at the first one that isn't valid UTF-8.
    fn feed(&mut self, bytes: &[u8]) -> bool {
        for &b in bytes {
            if self.need > 0 {
                if b < self.min || b > self.max {
                    return false;
                }
                self.need -= 1;
                (self.min, self.max) = (0x80, 0xBF);
            } else {
                match b {
                    0x00..=0x7f => {}
                    0xc2..=0xdf => {
                        self.need = 1;
                        (self.min, self.max) = (0x80, 0xBF);
                    }
                    0xe0 => {
                        self.need = 2;
                        (self.min, self.max) = (0xa0, 0xBF);
                    }
                    0xe1..=0xec | 0xee..=0xef => {
                        self.need = 2;
                        (self.min, self.max) = (0x80, 0xBF);
                    }
                    0xed => {
                        self.need = 2;
                        (self.min, self.max) = (0x80, 0x9f);
                    }
                    0xf0 => {
                        self.need = 3;
                        (self.min, self.max) = (0x90, 0xBF);
                    }
                    0xf1..=0xf3 => {
                        self.need = 3;
                        (self.min, self.max) = (0x80, 0xBF);
                    }
                    0xf4 => {
                        self.need = 3;
                        (self.min, self.max) = (0x80, 0x8f);
                    }
                    _ => return false,
                }
            }
        }
        true
    }

    /// Whether the bytes so far end on a whole character.
    fn complete(&self) -> bool {
        self.need == 0
    }
}

/// What one batch of a text value produced.
enum TextBatch {
    /// The value needs another batch.
    More(PendingText),
    /// Every byte of the value was read; `out` is its UTF-8 text.
    Finished(PendingText),
    /// A broken chunk: drop it, as the previous reader did.
    /// The bytes it inflated, which are charged to the budget even though the text is dropped.
    Invalid(usize),
}

/// How much of one text value one batch reads or inflates.
const TEXT_BATCH: usize = 8192;
/// How many chunks one batch scans when it isn't continuing a text value.
const TEXT_CHUNK_BATCH: usize = 64;

/// Reads one batch of a text value. Inflation uses a fixed scratch buffer and checks each batch
/// against the budget before growing the output, so no input-sized allocation is made first.
fn inflate_text(mut p: PendingText, bytes: &[u8]) -> Result<TextBatch, CodecError> {
    let Some(input) = bytes.get(p.input.clone()) else { return Ok(TextBatch::Invalid(p.out.len())) };
    let mut scratch = [0; TEXT_BATCH];
    if let Some(decoder) = p.decoder.as_mut() {
        let Some(tail) = input.get(p.at.saturating_sub(p.input.start)..) else { return Ok(TextBatch::Invalid(p.out.len())) };
        let start_in = decoder.total_in();
        let start_out = decoder.total_out();
        let status = decoder.decompress(tail, &mut scratch, flate2::FlushDecompress::None);
        let used = usize::try_from(decoder.total_in().saturating_sub(start_in)).unwrap_or(usize::MAX);
        let made = usize::try_from(decoder.total_out().saturating_sub(start_out)).unwrap_or(usize::MAX);
        let Ok(status) = status else {
            // The backend may write a batch before it reports corruption: charge it too, Latin-1
            // text at its UTF-8 length as on the success path.
            let charged = match scratch.get(..made) {
                Some(batch) if p.latin1 => latin1_len(batch).unwrap_or(usize::MAX),
                _ => made,
            };
            return Ok(TextBatch::Invalid(p.out.len().saturating_add(charged)));
        };
        let (Some(used), Some(batch)) = (p.at.checked_add(used), scratch.get(..made)) else { return Ok(TextBatch::Invalid(p.out.len())) };
        p.at = used;
        if !append_checked(&mut p.out, &mut p.utf8, &mut p.over, batch, p.latin1, p.limit)? {
            return Ok(TextBatch::Invalid(p.out.len()));
        }
        if status == flate2::Status::StreamEnd {
            return Ok(TextBatch::Finished(p));
        }
        if start_in == decoder.total_in() && start_out == decoder.total_out() {
            return Ok(TextBatch::Invalid(p.out.len())); // Truncated stream: never keep partial text.
        }
    } else {
        let remaining = input.len().saturating_sub(p.at.saturating_sub(p.input.start));
        let take = remaining.min(TEXT_BATCH);
        let start = p.at.saturating_sub(p.input.start);
        let Some(batch) = input.get(start..start.saturating_add(take)) else { return Ok(TextBatch::Invalid(0)) };
        p.at = p.at.saturating_add(take);
        if !append_checked(&mut p.out, &mut p.utf8, &mut p.over, batch, p.latin1, p.limit)? {
            // Stored text isn't inflated: nothing is charged for it.
            return Ok(TextBatch::Invalid(0));
        }
        if take == remaining {
            if p.utf8.as_ref().is_some_and(|check| !check.complete()) {
                return Ok(TextBatch::Invalid(0));
            }
            if p.over {
                return Err(text_limit());
            }
            return Ok(TextBatch::Finished(p));
        }
    }
    Ok(TextBatch::More(p))
}

/// Appends one batch of decoded bytes, refusing to grow past the budget. Every Latin-1 code
/// point fits in at most two UTF-8 bytes.
///
/// An uncompressed iTXt value is UTF-8 checked batch by batch before it is charged: an invalid
/// value is dropped even when it is over the budget, as the previous whole-value check did.
/// Its budget error is deferred until every byte has been seen and found valid.
fn append_checked(out: &mut Vec<u8>, utf8: &mut Option<Utf8Check>, over: &mut bool, bytes: &[u8], latin1: bool, limit: usize) -> Result<bool, CodecError> {
    if let Some(check) = utf8.as_mut()
        && !check.feed(bytes)
    {
        return Ok(false);
    }
    if *over {
        return Ok(true);
    }
    let added = if latin1 { latin1_len(bytes).ok_or_else(text_limit)? } else { bytes.len() };
    if out.len().checked_add(added).is_none_or(|len| len > limit) {
        if utf8.is_some() {
            *over = true;
            return Ok(true);
        }
        return Err(text_limit());
    }
    out.try_reserve(added).map_err(|_| text_limit())?;
    if latin1 {
        let mut buf = [0; 2];
        for &b in bytes {
            out.extend_from_slice(char::from(b).encode_utf8(&mut buf).as_bytes());
        }
    } else {
        out.extend_from_slice(bytes);
    }
    Ok(true)
}

fn is_latin1_keyword(k: &str) -> bool {
    !k.is_empty() && k.len() <= 79 && k.chars().all(|c| (' '..='~').contains(&c) || ('\u{a1}'..='\u{ff}').contains(&c))
}

/// Makes the PNG writer's output growth report an encode error on allocation failure.
struct FallibleOutput<'a>(&'a mut Vec<u8>);

impl Write for FallibleOutput<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.try_reserve(bytes.len()).map_err(|_| io::Error::other("not enough memory for PNG output"))?;
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn encode(src: &Image, plan: Plan, opts: &EncodeOptions) -> Result<Vec<u8>, CodecError> {
    let img = src.converted(plan.layout, plan.sample);
    let (w, h) = img.dimensions();
    let bpp = img.layout().channels() * img.sample_type().bytes();
    // Big-endian sample bytes.
    let mut data = std::borrow::Cow::Borrowed(img.data());
    if img.sample_type() == SampleType::U16 {
        to_big_endian(data.to_mut());
    }
    let mut out = Vec::new();
    {
        let mut output = FallibleOutput(&mut out);
        let mut writer = start(&mut output, (w, h), (img.layout(), img.sample_type()), img.icc.as_ref(), &img.meta, opts)?;
        if opts.png_interlaced {
            let idat = adam7_idat(&data, w as usize, h as usize, bpp, opts.png_compression)?;
            write_idat(&mut writer, &idat, MAX_IDAT)?;
        } else if let Some(idat) = parallel_idat(&data, w as usize, h as usize, bpp, opts.png_compression) {
            write_idat(&mut writer, &idat, MAX_IDAT)?;
        } else {
            writer.write_image_data(&data).map_err(|e| CodecError::encode(F, e))?;
        }
        writer.finish().map_err(|e| CodecError::encode(F, e))?;
    }
    Ok(out)
}

/// 16-bit native-endian samples to PNG's big-endian order, in place.
fn to_big_endian(data: &mut [u8]) {
    for c in data.as_chunks_mut::<2>().0 {
        let v = u16::from_ne_bytes([c[0], c[1]]);
        c.copy_from_slice(&v.to_be_bytes());
    }
}

/// Writes the signature and every chunk ahead of the image data for a `w` × `h` PNG of
/// `layout` and `sample` (already the encode plan's), with its profile and metadata.
fn start<W: Write>(
    out: W,
    (w, h): (u32, u32),
    (layout, sample): (ChannelLayout, SampleType),
    icc: Option<&Vec<u8>>,
    meta: &Metadata,
    opts: &EncodeOptions,
) -> Result<png::Writer<W>, CodecError> {
    let color = match layout {
        ChannelLayout::Gray => png::ColorType::Grayscale,
        ChannelLayout::GrayA => png::ColorType::GrayscaleAlpha,
        ChannelLayout::Rgb => png::ColorType::Rgb,
        ChannelLayout::Rgba => png::ColorType::Rgba,
        l => return Err(CodecError::encode(F, format!("unsupported layout {l:?}"))),
    };
    let depth = match sample {
        SampleType::U8 => png::BitDepth::Eight,
        SampleType::U16 => png::BitDepth::Sixteen,
        s => return Err(CodecError::encode(F, format!("unsupported sample {s:?}"))),
    };
    let mut info = png::Info::with_size(w, h);
    info.color_type = color;
    info.bit_depth = depth;
    info.interlaced = opts.png_interlaced;
    if opts.embed_icc
        && let Some(icc) = icc
    {
        info.icc_profile = Some(icc.clone().into());
    }
    if opts.embed_metadata {
        if let Some(exif) = &meta.exif {
            // The pixels are written as they are shown: never let a viewer rotate them again.
            info.exif_metadata = Some(crate::resolution::export_exif(exif, meta.dpi).into_owned().into());
        }
        if let Some((x, y)) = meta.dpi
            && x > 0.0
            && y > 0.0
        {
            info.pixel_dims =
                Some(png::PixelDimensions { xppu: (x / METERS_PER_INCH).round() as u32, yppu: (y / METERS_PER_INCH).round() as u32, unit: png::Unit::Meter });
        }
    }
    let mut encoder = png::Encoder::with_info(out, info).map_err(|e| CodecError::encode(F, e))?;
    encoder.set_compression(match opts.png_compression {
        PngCompression::None => png::Compression::NoCompression,
        PngCompression::Fast => png::Compression::Fast,
        PngCompression::Default => png::Compression::Balanced,
        PngCompression::Best => png::Compression::High,
    });
    if opts.embed_metadata {
        for (k, v) in &meta.text {
            let latin1_text = v.chars().all(|c| (c as u32) < 256);
            let res = if is_latin1_keyword(k) && latin1_text {
                encoder.add_text_chunk(k.clone(), v.clone())
            } else if is_latin1_keyword(k) {
                encoder.add_itxt_chunk(k.clone(), v.clone())
            } else {
                continue;
            };
            res.map_err(|e| CodecError::encode(F, e))?;
        }
        if let Some(xmp) = &meta.xmp {
            encoder.add_itxt_chunk(XMP_KEYWORD.into(), crate::resolution::export_xmp(xmp, meta.dpi).into_owned()).map_err(|e| CodecError::encode(F, e))?;
        }
    }
    let mut writer = encoder.write_header().map_err(|e| CodecError::encode(F, e))?;
    if let Some(stereo) = opts.png_stereo {
        // Ancillary chunk before IDAT, as the spec recommends for `sTER`.
        writer.write_chunk(png::chunk::ChunkType(*b"sTER"), &[stereo.mode()]).map_err(|e| CodecError::encode(F, e))?;
    }
    Ok(writer)
}

/// Largest IDAT chunk written. A chunk holds at most 2^31 - 1 bytes, so a zlib stream past that
/// (a multi-gigapixel image) is split over several IDAT chunks, which readers join back.
const MAX_IDAT: usize = 1 << 30;

/// `zlib` as IDAT chunks of at most `max` bytes.
fn write_idat<W: Write>(writer: &mut png::Writer<W>, zlib: &[u8], max: usize) -> Result<(), CodecError> {
    for part in zlib.chunks(max.max(1)) {
        writer.write_chunk(png::chunk::IDAT, part).map_err(|e| CodecError::encode(F, e))?;
    }
    Ok(())
}

/// A PNG encoded a band of rows at a time, for images too large to hold whole (a multi-gigapixel
/// print): each band is converted to the encode plan, filtered and deflated on all cores as it
/// arrives and written as IDAT chunks, so only about one band is ever held. The deflate bands are
/// [`parallel_idat`]'s, so the zlib stream is the one [`crate::encode`] writes for a large image.
/// Never interlaced (Adam7 needs the whole image).
pub struct PngRows<W: Write> {
    writer: png::Writer<W>,
    plan: Plan,
    width: u32,
    height: u32,
    /// Rows received so far.
    rows: u32,
    level: PngCompression,
    /// Converted, big-endian rows that don't fill a deflate band yet.
    pending: Vec<u8>,
    /// The last row deflated: the row above `pending` for the filters.
    prev: Vec<u8>,
    adler: u32,
    /// Whether the zlib header is written.
    started: bool,
}

impl<W: Write> PngRows<W> {
    /// Writes the header of a `first.width()` × `height` PNG to `out`. The layout, sample type,
    /// profile and metadata come from `first`, the first band (not written yet: [`Self::push`]
    /// it like the others), through the same plan as [`crate::encode`].
    pub fn new(out: W, first: &Image, height: u32, opts: &EncodeOptions) -> Result<Self, CodecError> {
        if opts.png_interlaced {
            return Err(CodecError::unsupported(F, "an interlaced PNG can't be written a band of rows at a time"));
        }
        let width = first.width();
        if width == 0 || height == 0 {
            return Err(CodecError::InvalidImage("cannot encode an empty image".into()));
        }
        let plan = crate::fidelity::plan(first, F, opts);
        let writer = start(out, (width, height), (plan.layout, plan.sample), first.icc.as_ref(), &first.meta, opts)?;
        Ok(Self { writer, plan, width, height, rows: 0, level: opts.png_compression, pending: Vec::new(), prev: Vec::new(), adler: 1, started: false })
    }

    /// Encodes the next rows of the image: `band` is as wide as the image, in any layout and
    /// sample type (converted like the first band).
    pub fn push(&mut self, band: &Image) -> Result<(), CodecError> {
        if band.width() != self.width {
            return Err(CodecError::InvalidImage(format!("a {} px wide band for a {} px wide image", band.width(), self.width)));
        }
        let rows = self.rows.checked_add(band.height()).filter(|&r| r <= self.height);
        let Some(rows) = rows else {
            return Err(CodecError::InvalidImage(format!("more than the image's {} rows", self.height)));
        };
        let img = band.converted(self.plan.layout, self.plan.sample);
        let at = self.pending.len();
        self.pending.try_reserve(img.data().len()).map_err(|_| CodecError::encode(F, "not enough memory for a band of rows"))?;
        self.pending.extend_from_slice(img.data());
        if self.plan.sample == SampleType::U16 {
            to_big_endian(self.pending.get_mut(at..).unwrap_or_default());
        }
        self.rows = rows;
        self.deflate_pending()
    }

    /// Deflates the whole bands held, and the rest once every row is in.
    fn deflate_pending(&mut self) -> Result<(), CodecError> {
        let bpp = self.plan.layout.channels() * self.plan.sample.bytes();
        let stride = self.width as usize * bpp;
        let per_band = band_rows(stride);
        let held = self.pending.len() / stride.max(1);
        let finish = self.rows == self.height;
        let take = if finish { held } else { held / per_band * per_band };
        if take == 0 {
            return Ok(());
        }
        let rows = self.pending.get(..take * stride).unwrap_or_default();
        let prev = (!self.prev.is_empty()).then_some(self.prev.as_slice());
        let parts = deflate_rows(rows, prev, stride, bpp, per_band, deflate_level(self.level), finish, encode_threads())
            .ok_or_else(|| CodecError::encode(F, "a band of rows could not be compressed"))?;
        let mut zlib = Vec::new();
        zlib.try_reserve_exact(parts.iter().map(|p| p.z.len()).sum::<usize>() + 6).map_err(|_| CodecError::encode(F, "not enough memory for PNG data"))?;
        if !self.started {
            zlib.extend_from_slice(&zlib_header(self.level));
            self.started = true;
        }
        for p in &parts {
            zlib.extend_from_slice(&p.z);
            self.adler = adler32_combine(self.adler, p.adler, p.len);
        }
        if finish {
            zlib.extend_from_slice(&self.adler.to_be_bytes());
        }
        write_idat(&mut self.writer, &zlib, MAX_IDAT)?;
        self.prev.clear();
        self.prev.extend_from_slice(rows.get(rows.len() - stride..).unwrap_or_default());
        self.pending.drain(..take * stride);
        Ok(())
    }

    /// Ends the file. Fails when fewer rows than the image's height were pushed.
    pub fn finish(self) -> Result<(), CodecError> {
        if self.rows != self.height || !self.pending.is_empty() {
            return Err(CodecError::InvalidImage(format!("{} of the image's {} rows were written", self.rows, self.height)));
        }
        self.writer.finish().map_err(|e| CodecError::encode(F, e))
    }
}

/// Build the zlib stream for an Adam7-interlaced image (filter type 0).
fn adam7_idat(data: &[u8], w: usize, h: usize, bpp: usize, level: PngCompression) -> Result<Vec<u8>, CodecError> {
    // (x0, y0, dx, dy) per pass.
    const PASSES: [(usize, usize, usize, usize); 7] = [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)];
    let mut raw = Vec::new();
    raw.try_reserve_exact(data.len() + h * 7).map_err(|_| CodecError::encode(F, "not enough memory for PNG rows"))?;
    for (x0, y0, dx, dy) in PASSES {
        if x0 >= w || y0 >= h {
            continue;
        }
        let mut y = y0;
        while y < h {
            raw.push(0u8);
            let mut x = x0;
            while x < w {
                let o = (y * w + x) * bpp;
                raw.extend_from_slice(&data[o..o + bpp]);
                x += dx;
            }
            y += dy;
        }
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), deflate_level(level));
    z.write_all(&raw).map_err(|e| CodecError::encode(F, e))?;
    z.finish().map_err(|e| CodecError::encode(F, e))
}

/// The deflate level of [`PngCompression::Fast`]: zlib-rs level 1 writes static Huffman blocks
/// only (20-30% bigger on filtered photos); level 3 is the lowest that is never bigger than
/// miniz's level 1 was, at about twice its time.
const FAST: flate2::Compression = flate2::Compression::new(3);

/// Images at least this large (bytes) are filtered and deflated on all cores.
const PARALLEL_MIN_BYTES: usize = 4 << 20;

/// The zlib stream of a non-interlaced image, built in parallel (`None` below
/// [`PARALLEL_MIN_BYTES`], on wasm, or without compression): rows get the adaptive filter
/// (minimum sum of absolute differences over the five PNG filters, as libpng and the `png`
/// crate do), then bands of rows are deflated independently at the requested level, each
/// ending on a sync flush so the raw deflate streams concatenate into one (as `pigz` does).
/// A 36 MP RGB export drops from ~9 s to well under a second on 12 cores; the output is a few
/// percent larger (no dictionary across bands).
fn parallel_idat(data: &[u8], w: usize, h: usize, bpp: usize, level: PngCompression) -> Option<Vec<u8>> {
    if cfg!(target_arch = "wasm32") || data.len() < PARALLEL_MIN_BYTES || level == PngCompression::None || w == 0 || h == 0 {
        return None;
    }
    banded_idat(data, w, h, bpp, level, encode_threads())
}

/// Filtered bytes per deflate band. The bands depend only on the image, never on the number of
/// threads, so the same image and options give the same bytes whatever the machine's core count.
const BAND_BYTES: usize = 512 << 10;

/// The deflate effort for `level`.
fn deflate_level(level: PngCompression) -> flate2::Compression {
    match level {
        PngCompression::None => flate2::Compression::none(),
        PngCompression::Fast => FAST,
        PngCompression::Default => flate2::Compression::default(),
        PngCompression::Best => flate2::Compression::best(),
    }
}

/// The zlib stream header: deflate, 32 K window; FLEVEL only advertises the effort.
fn zlib_header(level: PngCompression) -> [u8; 2] {
    match level {
        PngCompression::None | PngCompression::Fast => [0x78, 0x01],
        PngCompression::Best => [0x78, 0xDA],
        PngCompression::Default => [0x78, 0x9C],
    }
}

/// Rows per deflate band for rows of `stride` bytes.
fn band_rows(stride: usize) -> usize {
    (BAND_BYTES / stride.saturating_add(1)).max(1)
}

/// Worker threads for the banded encoders: one on wasm (no threads), else the machine's cores.
fn encode_threads() -> usize {
    if cfg!(target_arch = "wasm32") { 1 } else { std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 32) }
}

/// [`parallel_idat`] on `threads` worker threads.
fn banded_idat(data: &[u8], w: usize, h: usize, bpp: usize, level: PngCompression, threads: usize) -> Option<Vec<u8>> {
    let stride = w * bpp;
    let parts = deflate_rows(data.get(..h * stride)?, None, stride, bpp, band_rows(stride), deflate_level(level), true, threads)?;
    let mut out = Vec::new();
    out.try_reserve_exact(parts.iter().map(|p| p.z.len()).sum::<usize>() + 6).ok()?;
    out.extend_from_slice(&zlib_header(level));
    let mut adler = 1u32;
    for p in &parts {
        out.extend_from_slice(&p.z);
        adler = adler32_combine(adler, p.adler, p.len);
    }
    out.extend_from_slice(&adler.to_be_bytes());
    Some(out)
}

/// One band of rows filtered and deflated: the raw deflate bytes, and the Adler-32 and length of
/// the filtered bytes they hold.
struct Deflated {
    z: Vec<u8>,
    adler: u32,
    len: usize,
}

/// `rows` (whole rows of `stride` bytes; `prev` is the row above the first, if any) filtered and
/// deflated in independent bands of `band_rows` rows on up to `threads` workers (inline for one).
/// The last band finishes the deflate stream when `finish`; every other band ends on a sync flush.
/// `None` when a band did not deflate in one call ([`deflate_band`]).
#[allow(clippy::too_many_arguments)]
fn deflate_rows(
    rows: &[u8],
    prev: Option<&[u8]>,
    stride: usize,
    bpp: usize,
    band_rows: usize,
    lvl: flate2::Compression,
    finish: bool,
    threads: usize,
) -> Option<Vec<Deflated>> {
    let h = rows.len() / stride.max(1);
    let bands: Vec<(usize, usize)> = (0..h).step_by(band_rows.max(1)).map(|y| (y, (y + band_rows).min(h))).collect();
    let band = |&(y0, y1): &(usize, usize)| -> Option<Deflated> {
        let mut filtered = Vec::new();
        filtered.try_reserve_exact((y1 - y0) * (stride + 1)).ok()?;
        let mut scratch = Vec::new();
        scratch.try_reserve_exact(stride).ok()?;
        scratch.resize(stride, 0);
        for y in y0..y1 {
            let row = rows.get(y * stride..(y + 1) * stride)?;
            let above = if y > 0 { rows.get((y - 1) * stride..y * stride) } else { prev };
            filter_row(row, above, bpp, &mut scratch, &mut filtered);
        }
        let adler = adler32(1, &filtered);
        let z = deflate_band(&filtered, lvl, finish && y1 == h)?;
        Some(Deflated { z, adler, len: filtered.len() })
    };
    if threads <= 1 || bands.len() <= 1 {
        return bands.iter().map(band).collect();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut parts: Vec<(usize, Deflated)> = std::thread::scope(|sc| {
        let workers: Vec<_> = (0..threads.min(bands.len()))
            .map(|_| {
                sc.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(b) = bands.get(i) else { break };
                        let Some(d) = band(b) else { return Vec::new() };
                        out.push((i, d));
                    }
                    out
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().unwrap_or_default()).collect()
    });
    if parts.len() != bands.len() {
        return None;
    }
    parts.sort_by_key(|p| p.0);
    Some(parts.into_iter().map(|p| p.1).collect())
}

/// Raw deflate of one band: ends on a sync flush, or finishes the stream for the `last` band.
/// The output buffer is sized past deflate's worst case so that one call takes the whole band
/// and completes the flush: when a sync flush fills the buffer, miniz_oxide (flate2's former Rust
/// backend) can return before compressing the last lookahead bytes, and the next call only
/// drains the pending output, so a resumed flush silently drops data. `None` (the caller then
/// uses the serial encoder) if the single call did not finish.
fn deflate_band(filtered: &[u8], lvl: flate2::Compression, last: bool) -> Option<Vec<u8>> {
    let mut c = flate2::Compress::new(lvl, false);
    // Stored blocks cost 5 bytes per 64 KiB, the flush an empty stored block: 1/8 is ample.
    let mut z = Vec::new();
    z.try_reserve_exact(filtered.len() + filtered.len() / 8 + 1024).ok()?;
    let flush = if last { flate2::FlushCompress::Finish } else { flate2::FlushCompress::Sync };
    let status = c.compress_vec(filtered, &mut z, flush).ok()?;
    let done = if last { status == flate2::Status::StreamEnd } else { c.total_in() as usize == filtered.len() && z.len() < z.capacity() };
    done.then_some(z)
}

/// One scanline with the adaptive filter choice, appended to `out` (type byte + data).
fn filter_row(row: &[u8], prev: Option<&[u8]>, bpp: usize, scratch: &mut [u8], out: &mut Vec<u8>) {
    let n = row.len();
    let up = |i: usize| prev.map_or(0, |p| p[i]);
    let left = |i: usize| if i >= bpp { row[i - bpp] } else { 0 };
    let ul = |i: usize| if i >= bpp { prev.map_or(0, |p| p[i - bpp]) } else { 0 };
    let paeth = |a: u8, b: u8, c: u8| {
        let p = i16::from(a) + i16::from(b) - i16::from(c);
        let (pa, pb, pc) = ((p - i16::from(a)).abs(), (p - i16::from(b)).abs(), (p - i16::from(c)).abs());
        if pa <= pb && pa <= pc {
            a
        } else if pb <= pc {
            b
        } else {
            c
        }
    };
    let cost = |v: &[u8]| v.iter().map(|&b| u64::from((b as i8).unsigned_abs())).sum::<u64>();
    let mut best = cost(row);
    let start = out.len();
    out.push(0);
    out.extend_from_slice(row);
    for kind in 1u8..=4 {
        if best == 0 {
            break;
        }
        let mut c = 0u64;
        for i in 0..n {
            let x = row[i];
            scratch[i] = match kind {
                1 => x.wrapping_sub(left(i)),
                2 => x.wrapping_sub(up(i)),
                3 => x.wrapping_sub(((u16::from(left(i)) + u16::from(up(i))) / 2) as u8),
                _ => x.wrapping_sub(paeth(left(i), up(i), ul(i))),
            };
            c += u64::from((scratch[i] as i8).unsigned_abs());
            if c >= best {
                break;
            }
        }
        if c < best {
            best = c;
            out[start] = kind;
            out[start + 1..].copy_from_slice(&scratch[..n]);
        }
    }
}

const ADLER_MOD: u32 = 65_521;

fn adler32(start: u32, data: &[u8]) -> u32 {
    let (mut a, mut b) = (start & 0xffff, start >> 16);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += u32::from(x);
            b += a;
        }
        a %= ADLER_MOD;
        b %= ADLER_MOD;
    }
    (b << 16) | a
}

/// Adler-32 of A ‖ B from adler(A), adler(B) and |B| (zlib's `adler32_combine`).
fn adler32_combine(a1: u32, a2: u32, len2: usize) -> u32 {
    let rem = (len2 % ADLER_MOD as usize) as u64;
    let m = u64::from(ADLER_MOD);
    let (s1a, s2a) = (u64::from(a1 & 0xffff), u64::from(a1 >> 16));
    let (s1b, s2b) = (u64::from(a2 & 0xffff), u64::from(a2 >> 16));
    let s1 = (s1a + s1b + m - 1) % m;
    let s2 = (s2a + s2b + (rem * s1a) % m + m - rem) % m;
    ((s2 as u32) << 16) | s1 as u32
}

/// Writes an 8-bit palette PNG (PNG-8): `indices` row-major, `palette` RGB entries (≤ 256),
/// optional fully transparent entry (`tRNS`) and optional resolution in dpi (`pHYs`, written when
/// both axes are positive and finite). Used for Indexed Color documents.
pub fn encode_indexed(
    width: u32,
    height: u32,
    indices: &[u8],
    palette: &[[u8; 3]],
    transparent: Option<u8>,
    dpi: Option<(f32, f32)>,
    stereo: Option<PngStereo>,
) -> Result<Vec<u8>, CodecError> {
    if width == 0 || height == 0 || indices.len() != width as usize * height as usize {
        return Err(CodecError::InvalidImage("index data does not match the size".into()));
    }
    if palette.is_empty() || palette.len() > 256 || indices.iter().any(|&i| usize::from(i) >= palette.len()) {
        return Err(CodecError::InvalidImage("palette must have 1..=256 entries covering every index".into()));
    }
    let mut info = png::Info::with_size(width, height);
    info.color_type = png::ColorType::Indexed;
    info.bit_depth = png::BitDepth::Eight;
    info.palette = Some(palette.iter().flatten().copied().collect::<Vec<u8>>().into());
    if let Some(t) = transparent.filter(|&t| usize::from(t) < palette.len()) {
        let mut trns = vec![255u8; usize::from(t) + 1];
        trns[usize::from(t)] = 0;
        info.trns = Some(trns.into());
    }
    if let Some((x, y)) = dpi
        && x.is_finite()
        && y.is_finite()
        && x > 0.0
        && y > 0.0
    {
        info.pixel_dims =
            Some(png::PixelDimensions { xppu: (x / METERS_PER_INCH).round() as u32, yppu: (y / METERS_PER_INCH).round() as u32, unit: png::Unit::Meter });
    }
    let mut out = Vec::new();
    {
        let encoder = png::Encoder::with_info(&mut out, info).map_err(|e| CodecError::encode(F, e))?;
        let mut writer = encoder.write_header().map_err(|e| CodecError::encode(F, e))?;
        if let Some(stereo) = stereo {
            writer.write_chunk(png::chunk::ChunkType(*b"sTER"), &[stereo.mode()]).map_err(|e| CodecError::encode(F, e))?;
        }
        writer.write_image_data(indices).map_err(|e| CodecError::encode(F, e))?;
        writer.finish().map_err(|e| CodecError::encode(F, e))?;
    }
    Ok(out)
}

#[cfg(test)]
mod stereo_tests {
    use super::*;

    /// One CRC-valid chunk as it appears in a file.
    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let mut crc = flate2::Crc::new();
        crc.update(kind);
        crc.update(data);
        out.extend_from_slice(&crc.sum().to_be_bytes());
        out
    }

    fn png_with(stereo: &[u8]) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        out.extend_from_slice(&chunk(b"IHDR", &[0, 0, 0, 2, 0, 0, 0, 1, 8, 2, 0, 0, 0]));
        if !stereo.is_empty() {
            out.extend_from_slice(&chunk(b"sTER", stereo));
        }
        out.extend_from_slice(&chunk(b"IDAT", &[0x78, 0x01, 0x01, 0x00, 0xff, 0xff, 0, 3, 1, 0, 0]));
        out.extend_from_slice(&chunk(b"IEND", &[]));
        out
    }

    #[test]
    fn reads_both_defined_views() {
        assert_eq!(stereo(&png_with(&[0])), Some(PngStereo::CrossFuse));
        assert_eq!(stereo(&png_with(&[1])), Some(PngStereo::DivergingFuse));
        assert_eq!(stereo(&png_with(&[])), None);
        // Reserved and unknown mode bytes are not a pairing we can keep.
        assert_eq!(stereo(&png_with(&[2])), None);
        assert_eq!(stereo(&png_with(&[255])), None);
        // A two-byte payload is not sTER data.
        assert_eq!(stereo(&png_with(&[0, 0])), None);
        assert_eq!(stereo(b"not a png"), None);
    }

    #[test]
    fn ster_mode_bytes_describe_eye_order_not_vertical_packing() {
        // PNG Extensions 1.4.0 §3.6: both views occupy horizontal halves.
        // Each view is eight columns, so the second starts on an eight-column boundary.
        for (mode, left_pixel, right_pixel) in [(0, [0, 0, 255], [255, 0, 0]), (1, [255, 0, 0], [0, 0, 255])] {
            let pixels: Vec<u8> = [left_pixel; 8].into_iter().chain([right_pixel; 8]).flatten().collect();
            let img = Image::from_raw(16, 1, ChannelLayout::Rgb, SampleType::U8, pixels.clone()).unwrap();
            let pairing = if mode == 0 { PngStereo::CrossFuse } else { PngStereo::DivergingFuse };
            let opts = EncodeOptions { png_stereo: Some(pairing), ..Default::default() };
            let bytes = crate::encode(&img, Format::Png, &opts).unwrap();
            let chunk = bytes.windows(4).position(|w| w == b"sTER").unwrap();
            assert_eq!(bytes[chunk + 4], mode, "the specification's literal mode byte");
            let decoded = decode(&bytes, &Limits::default()).unwrap();
            assert_eq!(decoded.dimensions(), (16, 1));
            assert_eq!(decoded.data(), pixels);
        }
    }

    #[test]
    fn a_broken_crc_is_not_a_stereo_mark() {
        let mut bytes = png_with(&[0]);
        let kind = bytes.windows(4).position(|w| w == b"sTER").unwrap();
        // The CRC trails the type and the one-byte payload of the chunk.
        bytes[kind + 8] ^= 0xff;
        assert_eq!(stereo(&bytes), None);
    }

    #[test]
    fn encode_writes_the_chunk_before_idat_and_it_reads_back() {
        let img = Image::from_raw(2, 1, ChannelLayout::Rgb, SampleType::U8, vec![9; 6]).unwrap();
        for view in [PngStereo::CrossFuse, PngStereo::DivergingFuse] {
            let opts = EncodeOptions { png_stereo: Some(view), ..Default::default() };
            let bytes = crate::encode(&img, Format::Png, &opts).unwrap();
            assert_eq!(stereo(&bytes), Some(view));
            let back = decode(&bytes, &Limits::default()).unwrap();
            assert_eq!(back.data(), img.data());
            let ter = bytes.windows(4).position(|w| w == b"sTER").unwrap();
            let idat = bytes.windows(4).position(|w| w == b"IDAT").unwrap();
            assert!(ter < idat, "sTER before IDAT");
        }
        let plain = crate::encode(&img, Format::Png, &EncodeOptions::default()).unwrap();
        assert_eq!(stereo(&plain), None);
    }

    #[test]
    fn interlaced_and_indexed_encoders_keep_the_mark() {
        let img = Image::from_raw(2, 2, ChannelLayout::Rgb, SampleType::U8, vec![9; 12]).unwrap();
        let opts = EncodeOptions { png_stereo: Some(PngStereo::DivergingFuse), png_interlaced: true, ..Default::default() };
        let bytes = crate::encode(&img, Format::Png, &opts).unwrap();
        assert_eq!(stereo(&bytes), Some(PngStereo::DivergingFuse));
        let pal = [[255, 0, 0], [0, 0, 255]];
        let bytes = encode_indexed(2, 1, &[0, 1], &pal, None, None, Some(PngStereo::CrossFuse)).unwrap();
        assert_eq!(stereo(&bytes), Some(PngStereo::CrossFuse));
        assert_eq!(palette(&bytes).unwrap().map(|p| p.colors), Some(pal.to_vec()));
    }
}

#[cfg(test)]
mod indexed_tests {
    use super::*;

    #[test]
    fn png8_roundtrips_through_the_decoder() {
        let pal = [[255, 0, 0], [0, 0, 255], [0, 0, 0]];
        let idx = [0u8, 1, 2, 1, 0, 2];
        let bytes = encode_indexed(3, 2, &idx, &pal, Some(2), None, None).unwrap();
        let img = decode(&bytes, &Limits::default()).unwrap();
        assert_eq!(img.dimensions(), (3, 2));
        let px = img.convert(ChannelLayout::Rgba, SampleType::U8);
        assert_eq!(&px.data()[..8], &[255, 0, 0, 255, 0, 0, 255, 255]);
        assert_eq!(px.data()[11], 0, "transparent entry");
        assert!(encode_indexed(3, 2, &[9; 6], &pal, None, None, None).is_err());
        assert!(encode_indexed(3, 3, &idx, &pal, None, None, None).is_err());
    }

    fn png(color: png::ColorType, trns: Option<&[u8]>) -> Vec<u8> {
        let mut out = Vec::new();
        let mut e = png::Encoder::new(&mut out, 2, 1);
        e.set_color(color);
        e.set_depth(png::BitDepth::Eight);
        e.set_palette(vec![1, 2, 3, 4, 5, 6]);
        if let Some(t) = trns {
            e.set_trns(t.to_vec());
        }
        let data = if color == png::ColorType::Rgb { vec![0; 6] } else { vec![0, 1] };
        e.write_header().unwrap().write_image_data(&data).unwrap();
        out
    }

    #[test]
    fn palette_reads_the_table_and_its_one_transparent_entry() {
        let bytes = encode_indexed(2, 1, &[0, 1], &[[1, 2, 3], [4, 5, 6]], Some(1), None, None).unwrap();
        let colors = vec![[1, 2, 3], [4, 5, 6]];
        assert_eq!(palette(&bytes).unwrap(), Some(PngPalette { colors: colors.clone(), transparent: Some(1) }));
        assert_eq!(palette(&png(png::ColorType::Indexed, None)).unwrap(), Some(PngPalette { colors, transparent: None }));
        // A truecolour PNG's suggested palette is not a colour table.
        assert_eq!(palette(&png(png::ColorType::Rgb, None)).unwrap(), None);
        // Partial alpha or two transparent entries don't fit one transparent index.
        assert_eq!(palette(&png(png::ColorType::Indexed, Some(&[255, 128]))).unwrap(), None);
        assert_eq!(palette(&png(png::ColorType::Indexed, Some(&[0, 0]))).unwrap(), None);
    }
}

#[cfg(test)]
mod parallel_tests {
    use super::*;

    #[test]
    #[ignore = "manual 24 MP release benchmark"]
    fn benchmark_filter_selection() {
        use std::{hint::black_box, time::Instant};
        for pattern in ["zero", "identical", "varied"] {
            let row: Vec<u8> = (0..6000 * 4).map(|i| if pattern == "zero" { 0 } else { (i * 17 + i / 13) as u8 }).collect();
            let prev: Vec<_> = row.iter().map(|v| if pattern == "identical" { *v } else { v.wrapping_add(3) }).collect();
            let mut scratch = vec![0; row.len()];
            let mut out = Vec::with_capacity(row.len() + 1);
            for trial in 0..6 {
                type Filter = fn(&[u8], Option<&[u8]>, usize, &mut [u8], &mut Vec<u8>);
                let mut paths: [(&str, Filter); 2] = [("main", previous_filter_row), ("split", filter_row)];
                if trial % 2 == 1 {
                    paths.reverse();
                }
                for (name, path) in paths {
                    let t = Instant::now();
                    for _ in 0..4000 {
                        out.clear();
                        black_box(path)(black_box(&row), Some(black_box(&prev)), 4, &mut scratch, &mut out);
                        black_box(&out);
                    }
                    println!("filter,{pattern},{trial},{name},{:.6}", t.elapsed().as_secs_f64() * 1000.0);
                }
            }
        }
    }

    // The main-branch implementation before this refactor.
    fn previous_filter_row(row: &[u8], prev: Option<&[u8]>, bpp: usize, scratch: &mut [u8], out: &mut Vec<u8>) {
        let n = row.len();
        let up = |i: usize| prev.map_or(0, |p| p[i]);
        let left = |i: usize| if i >= bpp { row[i - bpp] } else { 0 };
        let ul = |i: usize| if i >= bpp { prev.map_or(0, |p| p[i - bpp]) } else { 0 };
        let paeth = |a: u8, b: u8, c: u8| {
            let p = i16::from(a) + i16::from(b) - i16::from(c);
            let (pa, pb, pc) = ((p - i16::from(a)).abs(), (p - i16::from(b)).abs(), (p - i16::from(c)).abs());
            if pa <= pb && pa <= pc {
                a
            } else if pb <= pc {
                b
            } else {
                c
            }
        };
        let cost = |v: &[u8]| v.iter().map(|&b| u64::from((b as i8).unsigned_abs())).sum::<u64>();
        let mut best = (cost(row), 0u8);
        for kind in 1u8..=4 {
            for i in 0..n {
                let x = row[i];
                scratch[i] = match kind {
                    1 => x.wrapping_sub(left(i)),
                    2 => x.wrapping_sub(up(i)),
                    3 => x.wrapping_sub(((u16::from(left(i)) + u16::from(up(i))) / 2) as u8),
                    _ => x.wrapping_sub(paeth(left(i), up(i), ul(i))),
                };
            }
            let c = cost(&scratch[..n]);
            if c < best.0 {
                best = (c, kind);
            }
        }
        out.push(best.1);
        if best.1 == 0 {
            out.extend_from_slice(row);
            return;
        }
        for i in 0..n {
            let x = row[i];
            scratch[i] = match best.1 {
                1 => x.wrapping_sub(left(i)),
                2 => x.wrapping_sub(up(i)),
                3 => x.wrapping_sub(((u16::from(left(i)) + u16::from(up(i))) / 2) as u8),
                _ => x.wrapping_sub(paeth(left(i), up(i), ul(i))),
            };
        }
        out.extend_from_slice(&scratch[..n]);
    }

    #[test]
    fn adaptive_filter_preserves_minimum_cost_and_ties() {
        for bpp in [1, 2, 3, 4, 6, 8] {
            for seed in 0..32usize {
                let row: Vec<u8> = (0..137).map(|i| (i * seed + i / 5) as u8).collect();
                let shifted: Vec<_> = row.iter().map(|v| v.wrapping_add(seed as u8)).collect();
                for prev in [None, Some(row.as_slice()), Some(shifted.as_slice())] {
                    let mut expected = vec![99];
                    let mut actual = expected.clone();
                    let mut scratch = vec![0; row.len()];
                    previous_filter_row(&row, prev, bpp, &mut scratch, &mut expected);
                    filter_row(&row, prev, bpp, &mut scratch, &mut actual);
                    assert_eq!(actual, expected, "bpp={bpp}, seed={seed}");
                }
            }
        }
        let mut out = Vec::new();
        filter_row(&[0; 16], None, 4, &mut [0; 16], &mut out);
        assert_eq!(out, vec![0; 17]);
        out.clear();
        filter_row(&[1; 16], Some(&[0; 16]), 1, &mut [0; 16], &mut out);
        assert_eq!(out, [vec![1, 1], vec![0; 15]].concat());
    }

    #[test]
    fn adler_combine_matches_one_pass() {
        let a: Vec<u8> = (0..100_000u32).map(|i| (i * 7 + i / 13) as u8).collect();
        let b: Vec<u8> = (0..70_001u32).map(|i| (i * 31) as u8).collect();
        let whole: Vec<u8> = a.iter().chain(&b).copied().collect();
        assert_eq!(adler32_combine(adler32(1, &a), adler32(1, &b), b.len()), adler32(1, &whole));
    }

    #[test]
    fn bands_that_end_just_past_a_block_keep_all_their_data() {
        // Deflate output ~0.6 of the input, cut just past one of miniz_oxide's block
        // boundaries: a sync flush that fills the output buffer there used to lose the
        // last lookahead bytes, corrupting every band after it.
        for (k, len) in [(20u32, 136_916usize), (20, 345_044), (24, 195_796), (28, 189_142)] {
            let mut x = 0x9E37_79B9u32 ^ k;
            let data: Vec<u8> = (0..len)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    ((x >> 8) % k) as u8
                })
                .collect();
            for last in [false, true] {
                let z = deflate_band(&data, flate2::Compression::default(), last).unwrap();
                let mut d = flate2::Decompress::new(false);
                let mut back = Vec::with_capacity(len + 1);
                d.decompress_vec(&z, &mut back, flate2::FlushDecompress::Sync).unwrap();
                assert!(back == data, "k {k} len {len} last {last}: {} of {len} bytes", back.len());
            }
        }
    }

    /// The bytes depend on the image, not on the machine: one, two, seven and 32 worker threads
    /// (and the machine's own count, through `encode`) give the same stream.
    #[test]
    fn the_parallel_encoder_gives_the_same_bytes_on_any_thread_count() {
        let (w, h, bpp) = (1201usize, 1013usize, 4usize);
        let px: Vec<u8> =
            (0..w * h * bpp).map(|i| ((i / 5) as u32).wrapping_mul(2_654_435_761).rotate_left(i as u32 % 11) as u8 / 5 + (i % 89) as u8).collect();
        assert!(px.len() >= PARALLEL_MIN_BYTES && px.len() > 2 * BAND_BYTES);
        for level in [PngCompression::Fast, PngCompression::Default, PngCompression::Best] {
            let one = banded_idat(&px, w, h, bpp, level, 1).unwrap();
            for threads in [2, 7, 32] {
                assert!(banded_idat(&px, w, h, bpp, level, threads).unwrap() == one, "{level:?}: {threads} threads differ from one");
            }
            assert!(parallel_idat(&px, w, h, bpp, level).unwrap() == one, "{level:?}: this machine's thread count differs from one");
            let img = Image::from_raw(w as u32, h as u32, ChannelLayout::Rgba, SampleType::U8, px.clone()).unwrap();
            let opts = EncodeOptions { png_compression: level, ..Default::default() };
            let png = crate::encode(&img, Format::Png, &opts).unwrap();
            assert!(png.windows(one.len()).any(|c| c == one.as_slice()), "{level:?}: the file carries the stream");
            assert_eq!(decode(&png, &Limits::default()).unwrap().data(), img.data());
        }
    }

    #[test]
    fn large_images_roundtrip_through_the_parallel_encoder() {
        for (layout, sample, ch, bytes) in [(ChannelLayout::Rgb, SampleType::U8, 3usize, 1usize), (ChannelLayout::Rgba, SampleType::U16, 4, 2)] {
            let (w, h) = (1531u32, 977u32);
            let n = w as usize * h as usize * ch * bytes;
            let px: Vec<u8> = (0..n).map(|i| ((i / 3) as u32).wrapping_mul(2_654_435_761).rotate_left(i as u32 % 7) as u8 / 3 + (i % 97) as u8).collect();
            let img = Image::from_raw(w, h, layout, sample, px.clone()).unwrap();
            assert!(n >= PARALLEL_MIN_BYTES || sample == SampleType::U8);
            for level in [PngCompression::Fast, PngCompression::Default] {
                let opts = EncodeOptions { png_compression: level, ..Default::default() };
                let bytes = crate::encode(&img, Format::Png, &opts).unwrap();
                let back = decode(&bytes, &Limits::default()).unwrap();
                assert_eq!(back.dimensions(), (w, h));
                assert_eq!(back.convert(layout, sample).data(), img.data(), "{layout:?} {sample:?} {level:?}");
                // The oracle decoder agrees.
                let o = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).unwrap();
                assert_eq!((o.width(), o.height()), (w, h));
            }
        }
    }
}

#[cfg(test)]
mod streamed_tests {
    use super::*;

    /// (bytes ahead of the first IDAT, the IDAT payloads joined, how many IDAT chunks).
    fn split(png: &[u8]) -> (Vec<u8>, Vec<u8>, usize) {
        let (mut at, mut head, mut zlib, mut count) = (8, None, Vec::new(), 0);
        while at + 12 <= png.len() {
            let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            if &png[at + 4..at + 8] == b"IDAT" {
                head.get_or_insert_with(|| png[..at].to_vec());
                zlib.extend_from_slice(&png[at + 8..at + 8 + len]);
                count += 1;
            }
            at += 12 + len;
        }
        (head.unwrap(), zlib, count)
    }

    fn noise(n: usize, seed: u32) -> Vec<u8> {
        (0..n).map(|i| ((i / 3) as u32 ^ seed).wrapping_mul(2_654_435_761).rotate_left(i as u32 % 7) as u8 / 3 + (i % 97) as u8).collect()
    }

    /// `img` pushed in bands of `heights` rows (the last one takes the rest).
    fn streamed(img: &Image, heights: &[u32], opts: &EncodeOptions) -> Vec<u8> {
        let (w, h) = img.dimensions();
        let row = img.data().len() / h as usize;
        let mut bands = Vec::new();
        let mut y = 0;
        for &n in heights.iter().chain(std::iter::once(&u32::MAX)) {
            let n = n.min(h - y);
            if n == 0 {
                continue;
            }
            let data = img.data()[y as usize * row..(y + n) as usize * row].to_vec();
            bands.push(Image::from_raw(w, n, img.layout(), img.sample_type(), data).unwrap().with_icc(img.icc.clone()).with_meta(img.meta.clone()));
            y += n;
        }
        let mut out = Vec::new();
        let mut rows = PngRows::new(&mut out, &bands[0], h, opts).unwrap();
        for b in &bands {
            rows.push(b).unwrap();
        }
        rows.finish().unwrap();
        out
    }

    /// Bands of any height give the zlib stream, header chunks and pixels the whole-image encoder
    /// gives, at 8 and 16 bits, with a profile, resolution, text and XMP.
    #[test]
    fn streamed_rows_match_the_whole_image_encoder() {
        for (layout, sample, ch, bytes) in [(ChannelLayout::Rgba, SampleType::U8, 4usize, 1usize), (ChannelLayout::Rgb, SampleType::U16, 3, 2)] {
            let (w, h) = (1201u32, 1013u32);
            let meta = Metadata {
                dpi: Some((300.0, 300.0)),
                text: vec![("Title".into(), "Tent".into())],
                xmp: Some("<x:xmpmeta xmlns:x='adobe:ns:meta/'/>".into()),
                ..Default::default()
            };
            let img = Image::from_raw(w, h, layout, sample, noise(w as usize * h as usize * ch * bytes, 7))
                .unwrap()
                .with_icc(Some((0..600u32).map(|i| (i % 251) as u8).collect()))
                .with_meta(meta);
            for level in [PngCompression::Fast, PngCompression::Default] {
                let opts = EncodeOptions { png_compression: level, ..Default::default() };
                let whole = crate::encode(&img, Format::Png, &opts).unwrap();
                let (head, zlib, _) = split(&whole);
                for heights in [&[1u32, 77, 300][..], &[256, 256, 256], &[1013]] {
                    let png = streamed(&img, heights, &opts);
                    let (h2, z2, _) = split(&png);
                    assert!(h2 == head, "{layout:?} {level:?} {heights:?}: header chunks differ");
                    assert!(z2 == zlib, "{layout:?} {level:?} {heights:?}: zlib stream differs");
                    assert_eq!(decode(&png, &Limits::default()).unwrap().data(), img.data());
                }
            }
        }
    }

    /// Small images, no compression and bands in another sample type (converted by the plan).
    #[test]
    fn small_uncompressed_and_converted_bands_round_trip() {
        let img = Image::from_raw(3, 5, ChannelLayout::GrayA, SampleType::U8, noise(30, 3)).unwrap();
        for level in [PngCompression::None, PngCompression::Best] {
            let opts = EncodeOptions { png_compression: level, ..Default::default() };
            let png = streamed(&img, &[2, 1], &opts);
            assert_eq!(decode(&png, &Limits::default()).unwrap().data(), img.data(), "{level:?}");
        }
        // Float bands are written with 16 bits, as `encode` writes them.
        let f = img.convert(ChannelLayout::GrayA, SampleType::F32);
        let png = streamed(&f, &[4], &EncodeOptions::default());
        let back = decode(&png, &Limits::default()).unwrap();
        assert_eq!(back.sample_type(), SampleType::U16);
        assert_eq!(back.convert(ChannelLayout::GrayA, SampleType::U8).data(), img.data());
    }

    /// A zlib stream longer than one chunk may hold is split over IDAT chunks that read back.
    #[test]
    fn long_streams_split_over_several_idat_chunks() {
        let img = Image::from_raw(64, 40, ChannelLayout::Rgb, SampleType::U8, noise(64 * 40 * 3, 9)).unwrap();
        let whole = crate::encode(&img, Format::Png, &EncodeOptions::default()).unwrap();
        let (_, zlib, _) = split(&whole);
        let mut out = Vec::new();
        let mut w = start(&mut out, (64, 40), (ChannelLayout::Rgb, SampleType::U8), None, &Metadata::default(), &EncodeOptions::default()).unwrap();
        write_idat(&mut w, &zlib, 100).unwrap();
        w.finish().unwrap();
        let (_, joined, count) = split(&out);
        assert_eq!(count, zlib.len().div_ceil(100));
        assert!(joined == zlib);
        assert_eq!(decode(&out, &Limits::default()).unwrap().data(), img.data());
    }

    /// Bands that don't fit the image, missing rows and interlacing are errors, not panics.
    #[test]
    fn bands_that_dont_fit_are_errors() {
        let band = Image::from_raw(4, 2, ChannelLayout::Rgb, SampleType::U8, vec![0; 24]).unwrap();
        let opts = EncodeOptions::default();
        assert!(PngRows::new(Vec::new(), &band, 0, &opts).is_err());
        assert!(PngRows::new(Vec::new(), &band, 3, &EncodeOptions { png_interlaced: true, ..Default::default() }).is_err());
        let mut rows = PngRows::new(Vec::new(), &band, 3, &opts).unwrap();
        rows.push(&band).unwrap();
        // Two more rows than the image holds.
        assert!(rows.push(&band).is_err());
        let narrow = Image::from_raw(3, 1, ChannelLayout::Rgb, SampleType::U8, vec![0; 9]).unwrap();
        assert!(rows.push(&narrow).is_err());
        // Two of three rows.
        assert!(rows.finish().is_err());
    }
}

#[cfg(test)]
mod rows_tests {
    use super::*;

    /// The pixels `decode` read before it was built on [`PngDecodeRows`]: the whole image in one
    /// `next_frame` call, then the byte order of 16-bit samples.
    fn one_call(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), CodecError> {
        let mut decoder = png::Decoder::new_with_limits(Cursor::new(bytes), png::Limits { bytes: Limits::default().alloc_usize() });
        decoder.set_transformations(png::Transformations::EXPAND);
        decoder.set_ignore_text_chunk(true);
        let mut reader = decoder.read_info().map_err(map_png_err)?;
        let mut buf = vec![0u8; reader.output_buffer_size().ok_or_else(|| err("image too large"))?];
        let out = reader.next_frame(&mut buf).map_err(map_png_err)?;
        buf.truncate(out.buffer_size());
        if reader.output_color_type().1 == png::BitDepth::Sixteen {
            to_native_endian(&mut buf);
        }
        Ok((out.width, out.height, buf))
    }

    fn image(w: u32, h: u32, layout: ChannelLayout, sample: SampleType) -> Image {
        let len = w as usize * h as usize * layout.channels() * sample.bytes();
        let data: Vec<u8> = (0..len).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
        Image::from_raw(w, h, layout, sample, data).unwrap()
    }

    #[test]
    fn rows_give_the_pixels_of_a_single_call() {
        for layout in [ChannelLayout::Gray, ChannelLayout::GrayA, ChannelLayout::Rgb, ChannelLayout::Rgba] {
            for sample in [SampleType::U8, SampleType::U16] {
                for interlaced in [false, true] {
                    let img = image(33, 21, layout, sample);
                    let opts = EncodeOptions { png_interlaced: interlaced, ..Default::default() };
                    let bytes = crate::encode(&img, Format::Png, &opts).unwrap();
                    let (w, h, pixels) = one_call(&bytes).unwrap();
                    for per_call in [1usize, 7, usize::MAX] {
                        let mut rows = PngDecodeRows::new(bytes.as_slice(), &Limits::default()).unwrap();
                        let mut left = per_call;
                        let got = loop {
                            left -= 1;
                            let mut more = || {
                                left = left.saturating_sub(1);
                                left > 0
                            };
                            if let Some(img) = rows.step(&mut more).unwrap() {
                                break img;
                            }
                            left = per_call;
                        };
                        assert_eq!((got.width(), got.height()), (w, h));
                        assert_eq!(got.data(), pixels.as_slice(), "{layout:?} {sample:?} interlaced={interlaced} per_call={per_call}");
                        assert_eq!(got.data(), img.data());
                    }
                }
            }
        }
    }

    /// A metadata yield after the last row must not finish the pixel reader again on every call.
    #[test]
    fn metadata_yields_do_not_finalize_the_pixels_again() {
        let text = "A".repeat(20_000);
        let img = image(8, 6, ChannelLayout::Rgb, SampleType::U8).with_meta(Metadata { text: vec![("Comment".into(), text.clone())], ..Default::default() });
        let bytes = crate::encode(&img, Format::Png, &EncodeOptions::default()).unwrap();
        let mut rows = PngDecodeRows::new(bytes.as_slice(), &Limits::default()).unwrap();
        let mut calls = 0;
        let got = loop {
            calls += 1;
            if let Some(img) = rows.step(&mut || false).unwrap() {
                break img;
            }
            assert!(calls < 10_000, "the decoder never finished");
        };
        assert!(calls > 1, "the metadata took more than the pixel step");
        assert_eq!(rows.pixel_finalizations, 1, "a metadata yield must not finalize the pixels again");
        assert_eq!(got.meta.text, vec![("Comment".into(), text)]);
    }
    /// A compressed iTXt chunk whose text inflates to `len` bytes of invalid UTF-8.
    fn bad_itxt(len: usize) -> Vec<u8> {
        use std::io::Write;
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        z.write_all(&vec![0xff; len]).unwrap();
        let mut data = b"Comment\0\x01\x00\0\0".to_vec();
        data.extend_from_slice(&z.finish().unwrap());
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(b"iTXt");
        out.extend_from_slice(&data);
        let mut crc = flate2::Crc::new();
        crc.update(b"iTXt");
        crc.update(&data);
        out.extend_from_slice(&crc.sum().to_be_bytes());
        out
    }

    /// A small PNG with `chunks` placed before its `IEND`.
    fn png_with(chunks: &[u8]) -> Vec<u8> {
        let png = crate::encode(&image(4, 4, ChannelLayout::Rgb, SampleType::U8), Format::Png, &EncodeOptions::default()).unwrap();
        let (head, iend) = png.split_at(png.len() - 12);
        [head, chunks, iend].concat()
    }

    #[test]
    fn dropped_text_still_counts_against_the_budget() {
        let limits = Limits { max_alloc: 1 << 20, ..Limits::default() };
        let decode = |bytes: &[u8]| PngDecodeRows::new(bytes, &limits)?.step(&mut || true)?.ok_or_else(|| err("unfinished"));
        let one = decode(&png_with(&bad_itxt(100_000))).unwrap();
        assert!(one.meta.text.is_empty(), "invalid UTF-8 is dropped");
        // Twenty chunks inflate 2 MB between them: each is dropped, but the work is charged.
        assert!(matches!(decode(&png_with(&bad_itxt(100_000).repeat(20))), Err(CodecError::LimitExceeded(_))));
    }
}
