//! Streaming sample-rate conversion for playback, when the output device runs at another rate
//! than the sequence (a 44.1 kHz device playing a 48 kHz sequence).
//!
//! Every output sample's source position comes from its absolute output frame, as an exact
//! rational (`frame × from / to`), and is linearly interpolated, so the output is identical however
//! the stream is cut into blocks: no phase jump at block boundaries. Source frames are pulled in
//! contiguous, non-overlapping spans, so whatever produces them (the mixer, with its effect state)
//! sees one continuous stream. Output lags the source by one source sample (≈21 µs at 48 kHz),
//! which is what lets each block interpolate without reading ahead.
//!
//! When the rate goes down, the source first passes an elliptic low-pass (9th order, 0.1 dB
//! ripple up to 0.42 × the output rate, 80 dB stop band) as it is pulled, so content above the
//! output's Nyquist frequency does not fold back into the audible band. The filter delays the
//! source a little more (its group delay, a few output samples in the pass band) and starts from
//! silence after a seek.

use crate::biquad::Biquad;
use crate::design::{Band, Family, Sections, Spec, design};

/// Converts a planar stream from `from` Hz to `to` Hz, block by block.
#[derive(Debug, Clone)]
pub struct StreamResampler {
    from: u32,
    to: u32,
    /// Source frames `[held_start, held_start + held[c].len())` kept from the last pull.
    held_start: i64,
    held: Vec<Vec<f32>>,
    /// The output frame the next block is expected to start at (contiguous playback).
    next_out: Option<i64>,
    /// The anti-aliasing low-pass when the rate goes down, and its state per channel.
    aa: Option<Sections>,
    aa_state: Vec<Vec<Biquad>>,
}

/// Maximum source frames fetched in a single resampler pull.
pub const MAX_RESAMPLE_SOURCE_SPAN: usize = 384_000 * 10;

/// Largest output position, either side of 0, that [`StreamResampler::process`] reads (about 23
/// years at 1.536 MHz); a position or block end beyond it is clamped to it. Source positions then
/// stay far inside `i64`.
pub const MAX_RESAMPLE_POSITION: i64 = 1 << 50;

/// Most output frames one [`StreamResampler::process`] call returns.
pub const MAX_RESAMPLE_BLOCK: usize = 65_536;

/// Most source samples, over all channels, the resampler holds at once (16 MiB of `f32`).
pub const MAX_RESAMPLE_HELD: usize = 1 << 22;

impl StreamResampler {
    /// Both rates are clamped to [`crate::MIN_SAMPLE_RATE`] ..= [`crate::MAX_SAMPLE_RATE`] (1 kHz
    /// to 1.536 MHz), so one output frame never needs more than 1,536 source frames.
    pub fn new(from: u32, to: u32) -> Self {
        let sane = |r: u32| r.clamp(crate::MIN_SAMPLE_RATE as u32, crate::MAX_SAMPLE_RATE as u32);
        let (from, to) = (sane(from), sane(to));
        let aa = (from > to).then(|| {
            let spec = Spec { family: Family::Elliptic, band: Band::LowPass, order: 9, f1: 0.42 * to as f64, f2: 0.0, ripple_db: 0.1, stop_db: 80.0 };
            design(&spec, from as f64)
        });
        Self { from, to, held_start: 0, held: Vec::new(), next_out: None, aa, aa_state: Vec::new() }
    }

    /// Append `want` frames of `got` to the held source, through the anti-aliasing filter when
    /// there is one. The first span after a seek sets the channel count and starts the filter
    /// from silence.
    fn append(&mut self, got: &[Vec<f32>], want: usize) {
        if self.held.is_empty() {
            let channels = crate::sane_channels(got.len());
            self.held = vec![Vec::new(); channels];
            let sections = self.aa.map_or(0, |s| s.n);
            self.aa_state = vec![vec![Biquad::new(); sections]; channels];
        }
        for (c, (h, state)) in self.held.iter_mut().zip(&mut self.aa_state).enumerate() {
            let src = got.get(c).map(Vec::as_slice).unwrap_or_default();
            let raw = (0..want).map(|i| src.get(i).copied().filter(|v| v.is_finite()).unwrap_or(0.0));
            match &self.aa {
                Some(aa) => h.extend(raw.map(|x| {
                    let mut y = f64::from(x);
                    for (s, coeffs) in state.iter_mut().zip(aa.as_slice()) {
                        y = s.tick(coeffs, y);
                    }
                    y as f32
                })),
                None => h.extend(raw),
            }
        }
    }

    pub fn rates(&self) -> (u32, u32) {
        (self.from, self.to)
    }

    /// The source frame whose sample lands at (or just before) output frame `out`, and the
    /// remainder of the exact position in units of `1 / to` source frames.
    fn source_index(&self, out: i64) -> (i64, i64) {
        let p = out as i128 * self.from as i128;
        let to = self.to as i128;
        (p.div_euclid(to) as i64, p.rem_euclid(to) as i64)
    }

    /// Output frames `[out, out + n)`, planar. `pull(start, frames)` returns source frames
    /// `[start, start + frames)`, planar (missing samples read as silence). A block that does not
    /// continue the previous one (a seek) starts the stream afresh.
    ///
    /// A block longer than [`MAX_RESAMPLE_BLOCK`] frames returns its first `MAX_RESAMPLE_BLOCK`
    /// frames. The block is resampled in parts whose source span holds at most
    /// [`MAX_RESAMPLE_HELD`] samples across 64 channels, so a pull asks for at most
    /// `MAX_RESAMPLE_HELD / 64` frames.
    pub fn process(&mut self, out: i64, n: usize, mut pull: impl FnMut(i64, usize) -> Vec<Vec<f32>>) -> Vec<Vec<f32>> {
        let n = n.min(MAX_RESAMPLE_BLOCK);
        if n == 0 {
            return vec![Vec::new(); self.held.len().max(1)];
        }
        let ratio = self.from.div_ceil(self.to) as usize + 2;
        let part = (MAX_RESAMPLE_HELD / crate::MAX_CHANNELS / ratio).max(1);
        let mut dst: Vec<Vec<f32>> = Vec::new();
        let mut done = 0;
        while done < n {
            let len = part.min(n - done);
            let got = self.block(out.saturating_add(done as i64), len, &mut pull);
            if dst.is_empty() {
                dst = got.iter().map(|_| Vec::with_capacity(n)).collect();
            }
            for (d, g) in dst.iter_mut().zip(&got) {
                d.extend_from_slice(g);
            }
            done += len;
        }
        dst
    }

    fn block(&mut self, out: i64, n: usize, pull: &mut impl FnMut(i64, usize) -> Vec<Vec<f32>>) -> Vec<Vec<f32>> {
        let out = out.clamp(-MAX_RESAMPLE_POSITION, MAX_RESAMPLE_POSITION);
        let end = out.saturating_add(i64::try_from(n - 1).unwrap_or(i64::MAX)).min(MAX_RESAMPLE_POSITION);
        let (first, first_rem) = self.source_index(out);
        let (last, _) = self.source_index(end);
        // interpolation reads source frames first - 1 ..= last
        if self.next_out != Some(out) || self.held.is_empty() {
            self.held_start = first - 1;
            self.held.clear();
        }
        // A block that needs more than one span pulls several, in order.
        loop {
            let held_end = self.held_start + self.held.first().map_or(0, Vec::len) as i64;
            if last < held_end {
                break;
            }
            let want = ((last + 1 - held_end) as usize).min(MAX_RESAMPLE_SOURCE_SPAN);
            let got = pull(held_end, want);
            self.append(&got, want);
        }
        let mut dst = vec![vec![0.0f32; n]; self.held.len()];
        let to = self.to as i64;
        let (step, rem_step) = ((self.from / self.to) as i64, (self.from % self.to) as i64);
        let (mut s, mut rem) = (first, first_rem);
        // Frames past `end` (the position limit) stay at its position.
        let advancing = (end - out) as usize;
        for i in 0..n {
            let frac = (rem as f64 / to as f64) as f32;
            // held starts at first - 1, so the earlier frame is always held
            let k = usize::try_from(s - self.held_start).unwrap_or(0);
            for (d, h) in dst.iter_mut().zip(&self.held) {
                let a = k.checked_sub(1).and_then(|j| h.get(j)).copied().unwrap_or(0.0);
                let b = h.get(k).copied().unwrap_or(0.0);
                if let Some(x) = d.get_mut(i) {
                    *x = a + (b - a) * frac;
                }
            }
            if i >= advancing {
                continue;
            }
            s = s.wrapping_add(step);
            rem += rem_step;
            if rem >= to {
                rem -= to;
                s = s.wrapping_add(1);
            }
        }
        // keep what the next block can still read: from its first frame - 1 on
        let (next_first, _) = self.source_index(out.saturating_add(n as i64));
        let drop = (next_first - 1 - self.held_start).clamp(0, self.held.first().map_or(0, Vec::len) as i64) as usize;
        for h in &mut self.held {
            h.drain(..drop);
        }
        self.held_start += drop as i64;
        self.next_out = Some(out.saturating_add(n as i64));
        dst
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sine at `hz`, sampled at `rate`, from absolute frame `start`; logs every pull.
    fn sine(hz: f64, rate: u32, pulls: &mut Vec<(i64, usize)>) -> impl FnMut(i64, usize) -> Vec<Vec<f32>> + '_ {
        move |start, frames| {
            pulls.push((start, frames));
            let ch: Vec<f32> =
                (0..frames).map(|i| (2.0 * std::f64::consts::PI * hz * (start + i as i64) as f64 / rate as f64).sin() as f32).collect();
            vec![ch.clone(), ch]
        }
    }

    fn convert(from: u32, to: u32, blocks: &[usize], total: usize) -> (Vec<f32>, Vec<(i64, usize)>) {
        let mut r = StreamResampler::new(from, to);
        let mut pulls = Vec::new();
        let mut out = Vec::new();
        let mut pos = 0i64;
        let mut k = 0;
        while (pos as usize) < total {
            let n = blocks[k % blocks.len()].min(total - pos as usize);
            let b = r.process(pos, n, sine(1000.0, from, &mut pulls));
            assert_eq!(b.len(), 2);
            assert!(b.iter().all(|c| c.len() == n));
            out.extend_from_slice(&b[0]);
            pos += n as i64;
            k += 1;
        }
        (out, pulls)
    }

    #[test]
    fn output_is_the_same_however_the_stream_is_cut() {
        for (from, to) in [(48_000, 44_100), (44_100, 48_000), (48_000, 96_000), (96_000, 48_000)] {
            let (whole, _) = convert(from, to, &[20_000], 20_000);
            for blocks in [&[512][..], &[333], &[1, 7, 512, 64]] {
                let (cut, _) = convert(from, to, blocks, 20_000);
                assert_eq!(cut, whole, "{from}→{to} in blocks {blocks:?}");
            }
        }
    }

    /// `process` as it was with one i128 division per output sample.
    fn reference_process(r: &mut StreamResampler, out: i64, n: usize, mut pull: impl FnMut(i64, usize) -> Vec<Vec<f32>>) -> Vec<Vec<f32>> {
        let (from, to) = (r.from as i128, r.to as i128);
        let index = |out: i64| {
            let p = out as i128 * from;
            (p.div_euclid(to) as i64, (p.rem_euclid(to) as f64 / to as f64) as f32)
        };
        if n == 0 {
            return vec![Vec::new(); r.held.len().max(1)];
        }
        let (first, _) = index(out);
        let (last, _) = index(out.saturating_add(n as i64 - 1));
        if r.next_out != Some(out) || r.held.is_empty() {
            r.held_start = first - 1;
            r.held.clear();
        }
        let held_end = r.held_start + r.held.first().map_or(0, Vec::len) as i64;
        if last + 1 > held_end {
            let want = ((last + 1 - held_end) as usize).min(MAX_RESAMPLE_SOURCE_SPAN);
            let got = pull(held_end, want);
            r.append(&got, want);
        }
        let mut dst = vec![vec![0.0f32; n]; r.held.len()];
        for (i, frame) in (out..).take(n).enumerate() {
            let (s, frac) = index(frame);
            let k = usize::try_from(s - r.held_start).unwrap_or(0);
            for (d, h) in dst.iter_mut().zip(&r.held) {
                let a = k.checked_sub(1).and_then(|j| h.get(j)).copied().unwrap_or(0.0);
                let b = h.get(k).copied().unwrap_or(0.0);
                if let Some(x) = d.get_mut(i) {
                    *x = a + (b - a) * frac;
                }
            }
        }
        let (next_first, _) = index(out.saturating_add(n as i64));
        let drop = (next_first - 1 - r.held_start).clamp(0, r.held.first().map_or(0, Vec::len) as i64) as usize;
        for h in &mut r.held {
            h.drain(..drop);
        }
        r.held_start += drop as i64;
        r.next_out = Some(out.saturating_add(n as i64));
        dst
    }

    #[test]
    fn stepped_source_positions_are_bit_identical_to_per_sample_division() {
        let bits = |b: &[Vec<f32>]| b.iter().map(|c| c.iter().map(|v| v.to_bits()).collect::<Vec<_>>()).collect::<Vec<_>>();
        for (from, to) in [
            (48_000, 44_100),
            (44_100, 48_000),
            (96_000, 48_000),
            (48_000, 48_000),
            (48_000, 96_000),
            (44_100, 22_050),
            (7, 384_000),
            (176_400, 1_001),
        ] {
            let (mut new, mut old) = (StreamResampler::new(from, to), StreamResampler::new(from, to));
            let mut k = 0;
            for start in [0i64, -5_000, 1_000_000_007, -3, i64::MAX / 2_000_000] {
                let mut pos = start;
                for _ in 0..12 {
                    let n = [1, 7, 333, 512, 64][k % 5];
                    k += 1;
                    let (mut pa, mut pb) = (Vec::new(), Vec::new());
                    let a = new.process(pos, n, sine(440.0, from, &mut pa));
                    let b = reference_process(&mut old, pos, n, sine(440.0, from, &mut pb));
                    assert_eq!(bits(&a), bits(&b), "{from}→{to} at {pos} + {n}");
                    // `process` may split a span into several consecutive pulls.
                    let merged = pa.iter().fold(Vec::<(i64, usize)>::new(), |mut m, &(s, f)| {
                        match m.last_mut() {
                            Some(last) if last.0 + last.1 as i64 == s => last.1 += f,
                            _ => m.push((s, f)),
                        }
                        m
                    });
                    assert_eq!(merged, pb);
                    pos += n as i64;
                }
            }
        }
    }

    #[test]
    fn a_tone_converts_to_the_same_tone() {
        // 1 kHz at 44.1 kHz played on a 48 kHz device: within linear interpolation's error of the
        // ideal tone, one source sample late
        let (from, to) = (44_100u32, 48_000u32);
        let (out, _) = convert(from, to, &[512], 20_000);
        for (o, x) in out.iter().enumerate().skip(2) {
            let t = o as f64 / to as f64 - 1.0 / from as f64;
            let want = (2.0 * std::f64::consts::PI * 1000.0 * t).sin() as f32;
            assert!((x - want).abs() < 3e-3, "frame {o}: {x} vs {want}");
        }
        // Going down, the low-pass keeps the tone's level (0.1 dB ripple) and frequency.
        let (from, to) = (48_000u32, 44_100u32);
        let (out, _) = convert(from, to, &[512], 20_000);
        let a = crate::testutil::tone_amplitude(&out[4_000..], 1000.0, to as f64);
        assert!((a - 1.0).abs() < 0.015, "{a}");
    }

    #[test]
    fn going_down_does_not_fold_content_above_the_output_nyquist_back() {
        // 30 kHz at 96 kHz on a 48 kHz device would land at 18 kHz without the low-pass.
        let (from, to) = (96_000u32, 48_000u32);
        let mut r = StreamResampler::new(from, to);
        let mut pulls = Vec::new();
        let out = r.process(0, 24_000, |s, f| {
            pulls.push((s, f));
            let ch: Vec<f32> = (0..f).map(|i| (2.0 * std::f64::consts::PI * 30_000.0 * (s + i as i64) as f64 / from as f64).sin() as f32).collect();
            vec![ch]
        });
        let alias = crate::testutil::tone_amplitude(&out[0][4_000..], 18_000.0, to as f64);
        assert!(alias < 1e-3, "the 18 kHz alias is at {alias}");
    }

    #[test]
    fn the_source_is_pulled_as_one_continuous_stream() {
        for (from, to) in [(48_000, 44_100), (44_100, 48_000)] {
            let (_, pulls) = convert(from, to, &[512, 333], 20_000);
            for w in pulls.windows(2) {
                assert_eq!(w[0].0 + w[0].1 as i64, w[1].0, "{from}→{to}: {:?} then {:?}", w[0], w[1]);
            }
        }
    }

    #[test]
    fn a_seek_starts_afresh_and_hostile_input_stays_finite() {
        let mut r = StreamResampler::new(48_000, 44_100);
        let mut pulls = Vec::new();
        let _ = r.process(0, 512, sine(1000.0, 48_000, &mut pulls));
        let _ = r.process(1_000_000, 512, sine(1000.0, 48_000, &mut pulls));
        assert_eq!(pulls.last().map(|p| p.0), Some(1_000_000 * 48_000 / 44_100 - 1));
        // a mixer returning too few channels and samples, and zero rates
        let mut r = StreamResampler::new(0, 0);
        let b = r.process(5, 64, |_, _| vec![vec![1.0; 3]]);
        assert_eq!(b.len(), 1);
        assert!(b[0].iter().all(|x| x.is_finite()));
        assert!(r.process(0, 0, |_, _| Vec::new()).iter().all(Vec::is_empty));
        // A rate far outside the range is clamped, so a short block pulls a bounded span.
        let mut r = StreamResampler::new(u32::MAX, 1);
        assert_eq!(r.rates(), (1_536_000, 1_000));
        let mut pulled = 0usize;
        let _ = r.process(0, 4, |_, frames| {
            pulled += frames;
            vec![vec![0.0; frames]]
        });
        assert!(pulled <= 4 * 1_536 + 2, "{pulled}");
        // Extreme positions are clamped instead of overflowing.
        for pos in [i64::MIN, i64::MAX, -MAX_RESAMPLE_POSITION - 1] {
            let mut r = StreamResampler::new(1_536_000, 1_000);
            let out = r.process(pos, 8, |_, frames| vec![vec![0.25; frames]]);
            assert!(out[0].iter().all(|v| v.is_finite()), "{pos}");
        }
        // Frames of a block past the limit read at the limit.
        for (from, to) in [(48_000, 48_000), (44_100, 48_000)] {
            let mut r = StreamResampler::new(from, to);
            let out = r.process(MAX_RESAMPLE_POSITION - 2, 8, |_, frames| vec![vec![0.5; frames]]);
            assert!(out[0].iter().all(|&v| v == 0.5), "{from} -> {to}: {:?}", out[0]);
        }
        // A pull that returns too many channels keeps the first `MAX_CHANNELS`.
        let mut r = StreamResampler::new(48_000, 48_000);
        let out = r.process(0, 1, |_, frames| vec![vec![0.5; frames]; 1_000]);
        assert_eq!(out.len(), crate::MAX_CHANNELS);
    }

    /// A block whose source span is longer than one pull is filled from several pulls.
    #[test]
    fn a_block_longer_than_one_pull_is_filled_completely() {
        let mut r = StreamResampler::new(1_536_000, 1_000);
        let mut spans = Vec::new();
        let out = r.process(0, 2_501, |start, frames| {
            spans.push((start, frames));
            vec![vec![1.0; frames]]
        });
        assert!(spans.len() > 1, "{spans:?}");
        assert!(spans.iter().all(|&(_, f)| f <= MAX_RESAMPLE_SOURCE_SPAN));
        assert!(spans.windows(2).all(|w| w[0].0 + w[0].1 as i64 == w[1].0), "{spans:?}");
        // The low-pass rings for a while after the stream starts, then settles on the constant;
        // no sample is left silent.
        assert!(out[0][3..].iter().all(|&x| x > 0.5), "a sample of the block was left silent");
        assert!(out[0][500..].iter().all(|&x| (x - 1.0).abs() < 1e-2), "{:?}", &out[0][500..520]);
    }

    /// The widest ratio with 64 channels holds and pulls a bounded span, and an oversized block
    /// returns `MAX_RESAMPLE_BLOCK` frames.
    #[test]
    fn source_held_and_block_length_are_bounded() {
        // At this ratio a pull holds about 42 output frames' worth of source, so 64 frames take two pulls; each output
        // frame pulls 1,536 source frames of 64 channels, so nightly runs a longer block.
        let frames = if craft_testkit::hostile::divisor() == 1 { 1_024 } else { 64 };
        let mut r = StreamResampler::new(1_536_000, 1_000);
        let mut widest = 0;
        let out = r.process(0, frames, |_, frames| {
            widest = widest.max(frames);
            vec![vec![0.5; frames]; 64]
        });
        assert_eq!((out.len(), out[0].len()), (64, frames));
        assert!(widest <= MAX_RESAMPLE_HELD / 64, "{widest}");
        assert!(r.held.iter().map(Vec::len).sum::<usize>() <= MAX_RESAMPLE_HELD);
        let mut r = StreamResampler::new(48_000, 48_000);
        let out = r.process(0, usize::MAX, |_, frames| vec![vec![0.5; frames]]);
        assert_eq!(out[0].len(), MAX_RESAMPLE_BLOCK);
        assert!(out[0].iter().all(|&x| x == 0.5));
    }
}
