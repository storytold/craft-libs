//! Sierra VMD sound decoder (feature `vmd-decode`): the 8-bit PCM and 16-bit DPCM sound of VMD
//! (Video and Music Data) files, as in Sierra's and Coktel Vision's 1990s games.
//!
//! Clean-room, from the public format description at <https://wiki.multimedia.cx/index.php/VMD>
//! ("Description of the Sierra Video and Music Data (VMD) Format", Mike Melanson and Vladimir
//! Gneushev). The description gives the coding but not the size of a block; the sizes here are
//! what ffmpeg was seen to expect of files made by this crate's tests:
//!
//! | Sound | Bytes in a block of `n` samples a channel |
//! |---|---|
//! | 8-bit | `n` for each channel, interleaved |
//! | 16-bit | 2 for each channel's first sample, then one code for each further sample, interleaved |
//!
//! The header's frame length is `n`, except in old-style stereo (audio flag 0x8000), where it
//! counts both channels. Old-style 16-bit stereo blocks begin with their first samples like the
//! others, which is what ffmpeg reads; the description says they carry the last block's samples
//! over instead. No such file was at hand to settle it.
//!
//! A [`Decoder`] is made from the 816-byte file header. [`Decoder::decode`] takes a sound
//! frame's flags (from its record in the file's table of contents; `craft_media::vmd` reads it)
//! and its data, and returns interleaved `i16` samples: one block of sound or silence, or, for a
//! frame with a silence mask (the first sound frame of a file), several. 8-bit samples are widened to 16 bits. Blocks do not depend
//! on each other, so decoding can start at any frame.
//!
//! Not decoded: IMA ADPCM sound (audio flag 0x10) and the external codecs of later Coktel Vision
//! files ([`Error::Unsupported`]).
//!
//! Input is treated as hostile: a block is at most 65 535 samples a channel and is decoded only
//! if all of it is there; a frame with a silence mask yields at most 32 blocks (the width of
//! the mask), any other frame as many whole blocks as its data holds. No `unsafe`, no panics.

#![warn(missing_docs)]

/// Bytes of the VMD file header a [`Decoder`] is made from.
pub const HEADER_LEN: usize = 816;

/// Errors from decoding VMD sound.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The header or a frame is not valid.
    #[error("invalid VMD sound: {0}")]
    Invalid(&'static str),
    /// Valid, but not something this decoder handles.
    #[error("unsupported VMD sound: {0}")]
    Unsupported(&'static str),
}

/// The steps of the 16-bit coding: a code's low seven bits pick one, its top bit the sign.
const STEPS: [u16; 128] = [
    0, 8, 16, 32, 48, 64, 80, 96, 112, 128, 144, 160, 176, 192, 208, 224, 240, 256, 272, 288, 304, 320, 336, 352, 368, 384, 400, 416, 432, 448, 464,
    480, 496, 512, 520, 528, 536, 544, 552, 560, 568, 576, 584, 592, 600, 608, 616, 624, 632, 640, 648, 656, 664, 672, 680, 688, 696, 704, 712, 720,
    728, 736, 744, 752, 760, 768, 776, 784, 792, 800, 808, 816, 824, 832, 840, 848, 856, 864, 872, 880, 888, 896, 904, 912, 920, 928, 936, 944, 952,
    960, 968, 976, 984, 992, 1000, 1008, 1016, 1024, 1088, 1152, 1216, 1280, 1344, 1408, 1472, 1536, 1600, 1664, 1728, 1792, 1856, 1920, 1984, 2048,
    2304, 2560, 2816, 3072, 3328, 3584, 3840, 4096, 5120, 6144, 7168, 8192, 12288, 16384,
];

/// A VMD sound decoder.
#[derive(Clone, Debug)]
pub struct Decoder {
    sample_rate: u32,
    channels: usize,
    sixteen_bit: bool,
    /// Samples of one block, all channels counted.
    block_total: usize,
}

impl Decoder {
    /// A decoder for the file whose header is `header` (its first 816 bytes; more are ignored).
    pub fn new(header: &[u8]) -> Result<Decoder, Error> {
        let h = header.get(..HEADER_LEN).ok_or(Error::Invalid("the header is shorter than 816 bytes"))?;
        let u16_at = |at: usize| u16::from_le_bytes([h[at], h[at + 1]]);
        if usize::from(u16_at(0)) != HEADER_LEN - 2 {
            return Err(Error::Invalid("not a VMD header"));
        }
        if u16_at(16) & 0x1000 == 0 || u16_at(804) == 0 {
            return Err(Error::Invalid("the file has no sound"));
        }
        let flags = u16_at(810);
        if flags & 0x10 != 0 {
            return Err(Error::Unsupported("IMA ADPCM sound"));
        }
        let length = u16_at(806) as i16;
        let frame_length = usize::from(length.unsigned_abs());
        // old-style stereo counts both channels, new-style one
        let (channels, block_total) = match flags {
            f if f & 0x8000 != 0 => (2, frame_length & !1),
            f if f & 0x200 != 0 => (2, frame_length * 2),
            _ => (1, frame_length),
        };
        Ok(Decoder { sample_rate: u32::from(u16_at(804)), channels, sixteen_bit: length < 0, block_total })
    }

    /// Samples a second.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// 1 or 2.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Bits a sample in the file: 8 (PCM) or 16 (DPCM). The decoder returns `i16` for both.
    pub fn bits(&self) -> u8 {
        if self.sixteen_bit { 16 } else { 8 }
    }

    /// Samples of one block for each channel.
    pub fn block_samples(&self) -> usize {
        self.block_total / self.channels
    }

    /// Bytes of one block of sound in a frame.
    pub fn block_bytes(&self) -> usize {
        if self.sixteen_bit { (self.block_total + self.channels).min(self.block_total * 2) } else { self.block_total }
    }

    /// Decode one sound frame: `flags` is the frame's flag byte from its record, `data` the
    /// frame data. Returns interleaved samples, [`Decoder::block_samples`] for each channel and
    /// block.
    ///
    /// - flags 1: one block of sound (every whole block, if the frame holds more);
    /// - flags 2: a 32-bit mask, then blocks of sound: the sound a player loads ahead at the
    ///   start of a file. From bit 0 up, a set bit is a block of silence and a clear bit the next
    ///   block of data, while there is one. ffmpeg plays the silent blocks first instead;
    ///   the order here is the description's;
    /// - flags 3: one block of silence.
    ///
    /// An empty frame gives no samples. With flags 1, a first block cut short is an error. A part
    /// of a block after the last whole one is left out, with flags 1 and 2.
    ///
    /// The sound of a file is its sound frames decoded in file order and played end to end from
    /// the first picture on: see `craft_media::vmd` for how that lines up with the pictures.
    pub fn decode(&self, flags: u8, data: &[u8]) -> Result<Vec<i16>, Error> {
        let mut out = Vec::new();
        match flags {
            3 => out.resize(self.block_total, 0),
            1 | 2 if data.is_empty() => {}
            1 => {
                // one block; a frame that holds more whole blocks plays them all, as in ffmpeg
                self.block(data, &mut out)?;
                let size = self.block_bytes();
                if size > 0 {
                    for more in data[size..].chunks_exact(size) {
                        self.block(more, &mut out)?;
                    }
                }
            }
            2 => {
                let (mask, mut data) = data.split_first_chunk::<4>().ok_or(Error::Invalid("the silence mask is cut short"))?;
                let mask = u32::from_le_bytes(*mask);
                // bit by bit: a set bit is a silent block, a clear one takes the next block of data
                // while there is one. The header's buffer count is not needed: game files hold
                // one block fewer than it says.
                for i in 0..32 {
                    if mask >> i & 1 == 1 {
                        out.resize(out.len() + self.block_total, 0);
                    } else if !data.is_empty() && data.len() >= self.block_bytes() {
                        self.block(data, &mut out)?;
                        data = &data[self.block_bytes()..];
                    }
                }
            }
            _ => return Err(Error::Unsupported("unknown sound frame flags")),
        }
        Ok(out)
    }

    /// Decode the block at the start of `data` onto `out`.
    fn block(&self, data: &[u8], out: &mut Vec<i16>) -> Result<(), Error> {
        let data = data.get(..self.block_bytes()).ok_or(Error::Invalid("a block of sound is cut short"))?;
        if !self.sixteen_bit {
            out.extend(data.iter().map(|&b| (i16::from(b) - 0x80) << 8));
            return Ok(());
        }
        // each channel's first sample as it is, then a signed step for each further sample
        let mut last = [0i32; 2];
        let Some((first, codes)) = data.split_at_checked(self.channels * 2) else { return Ok(()) };
        for (ch, s) in first.as_chunks::<2>().0.iter().enumerate() {
            last[ch] = i32::from(i16::from_le_bytes(*s));
            out.push(last[ch] as i16);
        }
        for (i, &code) in codes.iter().enumerate() {
            let ch = i % self.channels;
            let step = i32::from(STEPS[usize::from(code & 0x7F)]);
            last[ch] = (if code & 0x80 != 0 { last[ch] - step } else { last[ch] + step }).clamp(-32768, 32767);
            out.push(last[ch] as i16);
        }
        Ok(())
    }
}
