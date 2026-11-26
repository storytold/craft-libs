//! Motion JPEG (MJPEG): every picture of the stream is a JPEG image (ITU-T T.81 | ISO/IEC 10918-1),
//! so this module is a frame-level wrapper over `craft-jpeg`, which does the decoding and the
//! baseline encoding. There is no second JPEG implementation here.
//!
//! - [`Decoder`] decodes one packet to a [`Frame`]: baseline, extended-sequential (8 and 12 bit)
//!   and progressive Huffman JPEG, grayscale or YCbCr (as RGB) with any chroma subsampling. Packets
//!   of AVI `MJPG` streams (the "AVI1" framing) often leave out the Huffman tables: the Annex K
//!   tables are put in when a packet has none.
//! - [`Decoder::decode_into`] decodes one packet straight into a caller's buffer of RGBA or BGRA
//!   pixels, with any row stride, and [`picture_size`] reads the size to make that buffer from
//!   the headers.
//! - [`Encoder`] writes each frame as a complete baseline JPEG (SOI to EOI, tables in the packet)
//!   with Huffman tables made for the picture, which is what AVI `MJPG`, QuickTime `jpeg`, Matroska
//!   `V_MJPEG` and the other containers carry as a sample.
//! - [`is_packet`] tells a JPEG packet from other data.
//!
//! Not here: the two-field packets of interlaced MJPEG (QuickTime MJPEG-A and -B, AVI field
//! pairs), lossless JPEG (SOF3), arithmetic coding and the hierarchical process, which
//! `craft-jpeg` does not decode. Such packets are errors, never panics.

use craft_jpeg::Limits;
pub use craft_jpeg::{ChromaSubsampling, Samples};

use crate::budget::DECODE_MEMORY_BUDGET;

/// Codec errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The packet is not a decodable JPEG.
    #[error("invalid MJPEG packet: {0}")]
    Invalid(String),
    /// A JPEG process this module does not decode.
    #[error("unsupported MJPEG feature: {0}")]
    Unsupported(String),
    /// The picture is larger than the memory budget or [`Decoder::max_pixels`] allows.
    #[error("MJPEG picture above the limits: {0}")]
    Limit(String),
    /// The encoder was given a frame or settings it cannot encode.
    #[error("invalid encoder input: {0}")]
    Input(String),
}

/// `Result` of this module.
pub type Result<T> = std::result::Result<T, Error>;

impl From<craft_jpeg::Error> for Error {
    fn from(e: craft_jpeg::Error) -> Self {
        match e {
            craft_jpeg::Error::Malformed(m) => Error::Invalid(m),
            craft_jpeg::Error::Unsupported(m) => Error::Unsupported(m),
            craft_jpeg::Error::LimitExceeded(m) => Error::Limit(m),
            _ => Error::Invalid("undecodable JPEG".to_string()),
        }
    }
}

/// One decoded picture: interleaved samples, rows top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Samples per pixel: 1 for grayscale, 3 for RGB (a YCbCr picture is converted, JFIF / T.871).
    pub channels: u8,
    /// `U8` for 8-bit pictures; `U16` for 12-bit ones, scaled to the 16-bit range.
    pub samples: Samples,
    /// The entropy-coded data was damaged or cut short: what could be decoded is kept, the rest
    /// of that picture is grey, and the stream goes on with the next packet.
    pub damaged: bool,
}

/// Byte order of the four-byte pixels [`Decoder::decode_into`] writes. Alpha is always 255.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    /// Red, green, blue, alpha.
    Rgba8,
    /// Blue, green, red, alpha.
    Bgra8,
}

/// The picture [`Decoder::decode_into`] wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    /// The entropy-coded data was damaged or cut short, as for [`Frame::damaged`].
    pub damaged: bool,
}

/// Whether `packet` starts like a JPEG image (the SOI marker followed by another marker).
pub fn is_packet(packet: &[u8]) -> bool {
    matches!(packet, [0xFF, 0xD8, 0xFF, ..])
}

/// The width and height in the frame header (SOFn) of `packet`, read from the marker segments
/// before it without decoding anything, to size the buffer for [`Decoder::decode_into`].
///
/// A height of 0 (the height given later in a DNL marker) is [`Error::Unsupported`], as the
/// decoder does not decode such frames.
pub fn picture_size(packet: &[u8]) -> Result<(u32, u32)> {
    if !is_packet(packet) {
        return Err(Error::Invalid("the packet does not start with a JPEG SOI marker".to_string()));
    }
    let mut at = 2usize;
    while let Some(&v) = packet.get(at) {
        if v != 0xFF {
            at += 1;
            continue;
        }
        let Some(&m) = packet.get(at + 1) else { break };
        match m {
            0xFF => at += 1,
            0x00 | 0x01 | 0xD0..=0xD8 => at += 2,
            0xD9 | 0xDA => break,
            _ => {
                let len = packet.get(at + 2..at + 4).map(|b| usize::from(u16::from_be_bytes([b[0], b[1]])));
                let Some(len) = len.filter(|&l| l >= 2) else { return Err(Error::Invalid("bad marker segment length".to_string())) };
                let seg =
                    packet.get(at + 4..at + 2 + len).ok_or_else(|| Error::Invalid("marker segment runs past the end of the packet".to_string()))?;
                if matches!(m, 0xC0..=0xCF) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
                    let [_, h0, h1, w0, w1, ..] = *seg else { return Err(Error::Invalid("frame header too short".to_string())) };
                    let (width, height) = (u32::from(u16::from_be_bytes([w0, w1])), u32::from(u16::from_be_bytes([h0, h1])));
                    if width == 0 {
                        return Err(Error::Invalid("frame header with a width of 0".to_string()));
                    }
                    if height == 0 {
                        return Err(Error::Unsupported("JPEG with the height in a DNL marker".to_string()));
                    }
                    return Ok((width, height));
                }
                at += 2 + len;
            }
        }
    }
    Err(Error::Invalid("no frame header before the scan".to_string()))
}

/// Whether the first scan of `packet` is followed by EOI or by the end of the packet: no other
/// marker after its entropy-coded data, restart markers aside. The band decoder reads only the
/// first scan, so a packet with more after it is decoded whole, as [`Decoder::decode`] does.
fn first_scan_is_last(packet: &[u8]) -> bool {
    let mut at = 2usize;
    loop {
        match (packet.get(at), packet.get(at + 1)) {
            (Some(0xFF), Some(0xFF)) => at += 1,
            (Some(0xFF), Some(0x00 | 0x01 | 0xD0..=0xD8)) => at += 2,
            (Some(0xFF), Some(&m)) => {
                let Some(len) = packet.get(at + 2..at + 4).map(|b| usize::from(u16::from_be_bytes([b[0], b[1]]))) else { return false };
                at += 2 + len;
                if m == 0xDA {
                    break;
                }
            }
            (Some(_), _) => at += 1,
            (None, _) => return false,
        }
    }
    while let Some(p) = packet.get(at..).and_then(|r| r.iter().position(|&v| v == 0xFF)) {
        at += p;
        match packet.get(at + 1) {
            None => return true,
            Some(0x00 | 0xD0..=0xD7) => at += 2,
            Some(0xFF) => at += 1,
            Some(&m) => return m == 0xD9,
        }
    }
    true
}

/// Checks that `len` bytes hold `height` rows `stride` bytes apart, the last one `width × 4`
/// bytes long.
fn check_buffer(width: u32, height: u32, len: usize, stride: usize) -> Result<()> {
    let row = (width as usize).checked_mul(4).ok_or_else(|| Error::Input("row size overflows".to_string()))?;
    if stride < row {
        return Err(Error::Input(format!("a stride of {stride} bytes is shorter than a row of {width} pixels ({row} bytes)")));
    }
    let need = stride.checked_mul(height.saturating_sub(1) as usize).and_then(|n| n.checked_add(row));
    match need {
        Some(need) if need <= len => Ok(()),
        _ => Err(Error::Input(format!("a buffer of {len} bytes cannot hold {width} × {height} pixels at a stride of {stride} bytes"))),
    }
}

/// Samples per pixel that [`Decoder::decode_into`] converts: gray, RGB and CMYK.
fn check_components(components: u8) -> Result<()> {
    match components {
        1 | 3 | 4 => Ok(()),
        n => Err(Error::Unsupported(format!("{n}-component pictures cannot be converted to RGBA"))),
    }
}

/// One row of interleaved 8-bit samples (`components` per pixel) into four-byte pixels.
fn put_row(src: &[u8], components: u8, format: PixelFormat, dst: &mut [u8]) {
    let (r, b) = match format {
        PixelFormat::Rgba8 => (0, 2),
        PixelFormat::Bgra8 => (2, 0),
    };
    let dst = dst.as_chunks_mut::<4>().0.iter_mut();
    match components {
        1 => dst.zip(src).for_each(|(o, &g)| *o = [g, g, g, 255]),
        3 => dst.zip(src.as_chunks::<3>().0).for_each(|(o, p)| {
            o[r] = p[0];
            o[1] = p[1];
            o[b] = p[2];
            o[3] = 255;
        }),
        // Ink amounts, as `craft_jpeg::to_rgba8` converts them.
        _ => dst.zip(src.as_chunks::<4>().0).for_each(|(o, p)| {
            let k = 255 - u16::from(p[3]);
            let ink = |c: u8| ((255 - u16::from(c)) * k / 255) as u8;
            o[r] = ink(p[0]);
            o[1] = ink(p[1]);
            o[b] = ink(p[2]);
            o[3] = 255;
        }),
    }
}

/// A Motion JPEG decoder. Every packet is independent, so one decoder can decode packets in any
/// order and from several threads.
#[derive(Clone, Debug)]
pub struct Decoder {
    limits: Limits,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    /// A decoder whose pictures stay within the decoding memory budget of the crate.
    pub fn new() -> Self {
        let mut limits = Limits::default();
        limits.max_alloc = DECODE_MEMORY_BUDGET;
        Decoder { limits }
    }

    /// Refuse pictures of more than `pixels` pixels (`width × height`).
    pub fn max_pixels(mut self, pixels: u64) -> Self {
        self.limits.max_pixels = pixels;
        self
    }

    pub(crate) fn limit_allocation(&mut self, max_bytes: u64) {
        self.limits.max_alloc = self.limits.max_alloc.min(max_bytes);
    }

    /// Decode one packet.
    pub fn decode(&self, packet: &[u8]) -> Result<Frame> {
        if !is_packet(packet) {
            return Err(Error::Invalid("the packet does not start with a JPEG SOI marker".to_string()));
        }
        // An Adobe APP14 segment decides whether 3 and 4 components are YCbCr/YCCK or RGB/CMYK:
        // RGB packets with numeric component ids are only told apart from YCbCr by it.
        let adobe = craft_jpeg::adobe_transform(packet);
        // AVI `MJPG` packets may leave out the Huffman tables; every other packet keeps its own
        let packet = craft_jpeg::with_avi1_default_huffman(packet);
        let d = craft_jpeg::decode(&packet, &self.limits, adobe)?;
        Ok(Frame { width: d.width, height: d.height, channels: d.components, samples: d.samples, damaged: d.damaged })
    }

    /// Decode one packet into `out` as four-byte `format` pixels, rows top to bottom and
    /// `stride` bytes apart. Row `y` starts at `y × stride`; the bytes past each row's
    /// `width × 4` are left as they were. [`picture_size`] gives the size to make `out` for.
    ///
    /// Gray pictures become gray pixels and CMYK ones are converted as
    /// `craft_jpeg::to_rgba8` converts them. 12-bit pictures are rounded to 8 bits.
    ///
    /// Sequential 8-bit packets with one scan that holds every component, which is what cameras
    /// and AVI `MJPG` streams send, are decoded a band of rows at a time straight into `out`
    /// (`craft_jpeg::Bands`), so no buffer the size of the picture is made. Their pixels are
    /// those of [`Decoder::decode`] for intact data. In damaged data the band decoder starts
    /// again at the next restart marker, and a packet whose data fails at its first code gives a
    /// gray, damaged picture where [`Decoder::decode`] returns an error. Other packets
    /// (progressive, 12-bit, components in separate scans, any marker but EOI after the scan)
    /// are decoded whole first.
    ///
    /// A buffer too small for the picture, or a stride shorter than a row, is
    /// [`Error::Input`], and `out` is not written then.
    pub fn decode_into(&self, packet: &[u8], format: PixelFormat, out: &mut [u8], stride: usize) -> Result<Picture> {
        if !is_packet(packet) {
            return Err(Error::Invalid("the packet does not start with a JPEG SOI marker".to_string()));
        }
        let adobe = craft_jpeg::adobe_transform(packet);
        let packet = craft_jpeg::with_avi1_default_huffman(packet);
        let bands = first_scan_is_last(&packet).then(|| craft_jpeg::Bands::new(&packet, &self.limits, adobe).ok()).flatten();
        let Some(bands) = bands else {
            return self.decode_whole_into(&packet, adobe, format, out, stride);
        };
        let (width, height, components) = (bands.width(), bands.height(), bands.components());
        check_components(components)?;
        check_buffer(width, height, out.len(), stride)?;
        let (row_in, row_out) = (width as usize * usize::from(components), width as usize * 4);
        let mut reader = bands.reader();
        while let Some(band) = reader.next_band()? {
            for (r, src) in band.samples.chunks_exact(row_in).enumerate() {
                let at = (band.y as usize + r) * stride;
                let dst = out.get_mut(at..at + row_out).ok_or_else(|| Error::Input("buffer too small".to_string()))?;
                put_row(src, components, format, dst);
            }
        }
        Ok(Picture { width, height, damaged: reader.damaged() })
    }

    /// [`Decoder::decode_into`] for packets the band decoder does not take: the whole picture
    /// first, then its rows.
    fn decode_whole_into(&self, packet: &[u8], adobe: Option<u8>, format: PixelFormat, out: &mut [u8], stride: usize) -> Result<Picture> {
        // The buffer is checked against the frame header before the picture is decoded, and
        // against the decoded size after. A picture above the pixel limit is left to the decoder,
        // which reports the limit, as the band path does.
        if let Ok((width, height)) = picture_size(packet)
            && u64::from(width) * u64::from(height) <= self.limits.max_pixels
        {
            check_buffer(width, height, out.len(), stride)?;
        }
        let d = craft_jpeg::decode(packet, &self.limits, adobe)?;
        check_components(d.components)?;
        check_buffer(d.width, d.height, out.len(), stride)?;
        let (row_in, row_out) = (d.width as usize * usize::from(d.components), d.width as usize * 4);
        let mut wide = Vec::new();
        for y in 0..d.height as usize {
            let src = match &d.samples {
                Samples::U8(s) => s.get(y * row_in..(y + 1) * row_in),
                Samples::U16(s) => s.get(y * row_in..(y + 1) * row_in).map(|s| {
                    wide.clear();
                    wide.extend(s.iter().map(|&v| ((u32::from(v) * 255 + 32_767) / 65_535) as u8));
                    wide.as_slice()
                }),
            };
            let src = src.ok_or_else(|| Error::Invalid("decoded picture shorter than its size".to_string()))?;
            let dst = out.get_mut(y * stride..y * stride + row_out).ok_or_else(|| Error::Input("buffer too small".to_string()))?;
            put_row(src, d.components, format, dst);
        }
        Ok(Picture { width: d.width, height: d.height, damaged: d.damaged })
    }
}

/// Encoder settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncoderConfig {
    /// JPEG quality, 1 (smallest) to 100 (best); the usual scale on the Annex K tables.
    pub quality: u8,
    /// Chroma sampling of colour pictures; ignored for grayscale.
    pub subsampling: ChromaSubsampling,
}

impl Default for EncoderConfig {
    fn default() -> Self {
        EncoderConfig { quality: 90, subsampling: ChromaSubsampling::S420 }
    }
}

/// A Motion JPEG encoder: each frame becomes one baseline JPEG packet.
#[derive(Clone, Copy, Debug, Default)]
pub struct Encoder {
    cfg: EncoderConfig,
}

impl Encoder {
    /// An encoder, or the setting it cannot carry.
    pub fn new(cfg: EncoderConfig) -> Result<Encoder> {
        if !(1..=100).contains(&cfg.quality) {
            return Err(Error::Input(format!("quality {} is not in 1..=100", cfg.quality)));
        }
        Ok(Encoder { cfg })
    }

    /// Encode `width × height` interleaved 8-bit pixels of `channels` samples (1 gray, 3 RGB, or 4
    /// RGBA with the alpha dropped).
    pub fn encode(&self, width: u32, height: u32, channels: u8, pixels: &[u8]) -> Result<Vec<u8>> {
        craft_jpeg::encode_baseline(pixels, width as usize, height as usize, usize::from(channels), self.cfg.quality, self.cfg.subsampling, &[])
            .map_err(|e| Error::Input(e.to_string()))
    }

    /// Encode a decoded [`Frame`] of 8-bit samples.
    pub fn encode_frame(&self, frame: &Frame) -> Result<Vec<u8>> {
        match &frame.samples {
            Samples::U8(p) => self.encode(frame.width, frame.height, frame.channels, p),
            _ => Err(Error::Input("only 8-bit frames can be encoded".to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets_start_with_soi() {
        assert!(is_packet(&[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(!is_packet(&[0xFF, 0xD8]));
        assert!(!is_packet(b"RIFF"));
        assert!(Decoder::new().decode(b"not a jpeg").is_err());
    }

    #[test]
    fn quality_is_checked() {
        assert!(Encoder::new(EncoderConfig { quality: 0, ..Default::default() }).is_err());
        assert!(Encoder::new(EncoderConfig { quality: 101, ..Default::default() }).is_err());
        assert!(Encoder::new(EncoderConfig::default()).is_ok());
    }
}
