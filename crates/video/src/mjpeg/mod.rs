//! Motion JPEG (MJPEG): every picture of the stream is a JPEG image (ITU-T T.81 | ISO/IEC 10918-1),
//! so this module is a frame-level wrapper over `craft-jpeg`, which does the decoding and the
//! baseline encoding. There is no second JPEG implementation here.
//!
//! - [`Decoder`] decodes one packet to a [`Frame`]: baseline, extended-sequential (8 and 12 bit)
//!   and progressive Huffman JPEG, grayscale or YCbCr (as RGB) with any chroma subsampling. Packets
//!   of AVI `MJPG` streams (the "AVI1" framing) often leave out the Huffman tables: the Annex K
//!   tables are put in when a packet has none.
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

/// Whether `packet` starts like a JPEG image (the SOI marker followed by another marker).
pub fn is_packet(packet: &[u8]) -> bool {
    matches!(packet, [0xFF, 0xD8, 0xFF, ..])
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
