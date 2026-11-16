//! WAVE format constants shared by the reader and the writer (Microsoft multimedia specification).

pub(crate) const WAVE_FORMAT_PCM: u16 = 1;
pub(crate) const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
pub(crate) const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
/// Tail of the KSDATAFORMAT_SUBTYPE_* GUIDs (bytes 2..16); bytes 0..2 hold the format tag.
pub(crate) const GUID_TAIL: [u8; 14] = [0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71];
