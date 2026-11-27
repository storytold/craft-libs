//! The video buffering verifier (H.262 Annex C) as the encoder runs it: a model of the decoder's
//! input buffer, in decoding order, that says how large the next picture may be.
//!
//! Bits enter the buffer at `rate`. Each picture is removed whole at its decoding time, one
//! picture interval after the one before. With a constant bitrate (`vbv_delay` in every picture
//! header) the buffer must neither underflow (a picture not yet complete at its decoding time)
//! nor overflow. With a variable bitrate (`vbv_delay` 0xFFFF) the decoder fills the buffer until
//! it is full before the first picture, input stops while it is full, and only underflow is
//! possible.
//!
//! Fullness is counted in units of 1/`fps_num` bit, so one picture interval adds exactly
//! `rate · fps_den` of them and the model never rounds.

/// `vbv_delay` for a variable bitrate stream.
pub(crate) const VBR_DELAY: u16 = 0xFFFF;

/// The largest `vbv_delay` a constant bitrate stream can signal (0xFFFF means variable).
const MAX_DELAY: u64 = 0xFFFE;

pub(crate) struct Vbv {
    /// Buffer size in bits.
    size: u64,
    /// Bits per second: constant, or the peak at which the buffer fills.
    rate: u64,
    fps_num: u64,
    fps_den: u64,
    cbr: bool,
    /// Fullness just before the next picture is removed, in 1/`fps_num` bits.
    full: u64,
}

impl Vbv {
    /// A buffer of `size` bits filled at `rate` bit/s, `fps_num / fps_den` pictures a second.
    /// With a constant bitrate the first picture is decoded once `start` bits are in.
    pub fn new(size: u64, rate: u64, (fps_num, fps_den): (u32, u32), cbr: bool, start: u64) -> Self {
        let (fps_num, fps_den) = (u64::from(fps_num.max(1)), u64::from(fps_den.max(1)));
        let start = if cbr { start.min(size) } else { size };
        Self { size, rate, fps_num, fps_den, cbr, full: start.saturating_mul(fps_num) }
    }

    /// Bits that enter the buffer in one picture interval, rounded down.
    pub fn per_picture(&self) -> u64 {
        self.rate.saturating_mul(self.fps_den) / self.fps_num
    }

    /// The most bits the next picture may have: what is in the buffer at its decoding time.
    pub fn max_bits(&self) -> u64 {
        self.full / self.fps_num
    }

    /// The fewest bits the next picture may have: with a constant bitrate, removing fewer would
    /// let the buffer overflow before the picture after it.
    pub fn min_bits(&self) -> u64 {
        if !self.cbr {
            return 0;
        }
        let next = self.full.saturating_add(self.rate.saturating_mul(self.fps_den));
        next.saturating_sub(self.size.saturating_mul(self.fps_num)).div_ceil(self.fps_num)
    }

    /// `vbv_delay` of the next picture, whose data up to the end of its picture start code is
    /// `head_bits` long: the 90 kHz ticks from the arrival of that start code to its decoding.
    pub fn delay(&self, head_bits: u64) -> u16 {
        if !self.cbr {
            return VBR_DELAY;
        }
        let after = u128::from(self.full.saturating_sub(head_bits.saturating_mul(self.fps_num)));
        let ticks = after * 90_000 / (u128::from(self.rate.max(1)) * u128::from(self.fps_num));
        ticks.min(u128::from(MAX_DELAY)) as u16
    }

    /// Remove the next picture (`bits` long, at most [`Vbv::max_bits`]).
    pub fn remove(&mut self, bits: u64) {
        let left = self.full.saturating_sub(bits.saturating_mul(self.fps_num));
        let next = left.saturating_add(self.rate.saturating_mul(self.fps_den));
        self.full = if self.cbr { next } else { next.min(self.size.saturating_mul(self.fps_num)) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_constant_bitrate_buffer_bounds_both_sides() {
        // 4000 bits a picture into 10000 bits, starting at 6000
        let mut v = Vbv::new(10_000, 100_000, (25, 1), true, 6000);
        assert_eq!((v.per_picture(), v.max_bits(), v.min_bits()), (4000, 6000, 0));
        // 6000 - 1000 + 4000 = 9000: then at least 3000 must go, or 9000 - b + 4000 > 10000
        v.remove(1000);
        assert_eq!((v.max_bits(), v.min_bits()), (9000, 3000));
        v.remove(3000);
        assert_eq!(v.max_bits(), 10_000);
        // the delay of a picture whose start code ends 100 bits in: 9900 bits at 100 kbit/s
        assert_eq!(v.delay(100), 8910);
    }

    #[test]
    fn a_variable_bitrate_buffer_stops_filling_when_full() {
        let mut v = Vbv::new(10_000, 100_000, (25, 1), false, 0);
        assert_eq!((v.max_bits(), v.min_bits(), v.delay(0)), (10_000, 0, VBR_DELAY));
        v.remove(1000);
        assert_eq!(v.max_bits(), 10_000);
        v.remove(9000);
        assert_eq!(v.max_bits(), 5000);
    }

    #[test]
    fn fractional_frame_rates_do_not_drift() {
        // 30000/1001 pictures a second at 1 Mbit/s: 33366.67 bits a picture
        let mut v = Vbv::new(2_000_000, 1_000_000, (30000, 1001), true, 1_000_000);
        for _ in 0..30_000 {
            v.remove(33_366);
        }
        // 30000 pictures leave 0.666… bit each in the buffer: exactly 20000 bits
        assert_eq!(v.max_bits(), 1_020_000);
    }
}
