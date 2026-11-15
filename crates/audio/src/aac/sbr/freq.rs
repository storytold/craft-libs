//! Frequency band tables (ISO/IEC 14496-3 §4.6.18.3.2) and the patches and limiter bands built on
//! them (§4.6.18.6.3, §4.6.18.7.3). Every table holds QMF channel borders.

use super::Header;

/// The most patches the HF generator may build.
const MAX_PATCHES: usize = 5;

/// The tables of one SBR header at one output rate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FreqTables {
    /// First QMF channel of the master table (the start of the SBR range before the crossover).
    pub(crate) k0: usize,
    /// First QMF channel of the SBR range.
    pub(crate) kx: usize,
    /// Channels in the SBR range (`kx + m` is its end).
    pub(crate) m: usize,
    /// High-resolution envelope bands (`n_high + 1` borders).
    pub(crate) high: Vec<usize>,
    /// Low-resolution envelope bands.
    pub(crate) low: Vec<usize>,
    /// Noise floor bands (1 to 5).
    pub(crate) noise: Vec<usize>,
    /// Limiter bands.
    pub(crate) lim: Vec<usize>,
    /// `(first source channel, channels)` of each patch, in order from `kx` up.
    pub(crate) patches: Vec<(usize, usize)>,
}

/// Round half up, as the specification's `NINT`.
fn nint(x: f64) -> i64 {
    (x + 0.5).floor() as i64
}

/// The start channel offsets of `bs_start_freq`, by output rate (§4.6.18.3.2.1).
fn start_offsets(fs: u32) -> &'static [i8; 16] {
    match fs {
        0..=16_000 => &[-8, -7, -6, -5, -4, -3, -2, -1, 0, 1, 2, 3, 4, 5, 6, 7],
        16_001..=22_050 => &[-5, -4, -3, -2, -1, 0, 1, 2, 3, 4, 5, 6, 7, 9, 11, 13],
        22_051..=24_000 => &[-5, -3, -2, -1, 0, 1, 2, 3, 4, 5, 6, 7, 9, 11, 13, 16],
        24_001..=32_000 => &[-6, -4, -2, -1, 0, 1, 2, 3, 4, 5, 6, 7, 9, 11, 13, 16],
        32_001..=64_000 => &[-4, -2, -1, 0, 1, 2, 3, 4, 5, 6, 7, 9, 11, 13, 16, 20],
        _ => &[-2, -1, 0, 1, 2, 3, 4, 5, 6, 7, 9, 11, 13, 16, 20, 24],
    }
}

/// `NINT(f · 128 / fs)` for the 3/4/5 kHz (start) or 6/8/10 kHz (stop) minimum at `fs`.
fn min_channel(fs: u32, low: f64, mid: f64, high: f64) -> i64 {
    let f = if fs < 32_000 {
        low
    } else if fs < 64_000 {
        mid
    } else {
        high
    };
    nint(f * 128.0 / f64::from(fs))
}

/// The start and stop channels `k0`, `k2` of the master table.
fn start_stop(h: &Header, fs: u32) -> Option<(usize, usize)> {
    let k0 = min_channel(fs, 3000.0, 4000.0, 5000.0) + i64::from(start_offsets(fs)[usize::from(h.start_freq & 15)]);
    let k2 = match h.stop_freq {
        14 => 2 * k0,
        15 => 3 * k0,
        s => {
            let stop_min = min_channel(fs, 6000.0, 8000.0, 10_000.0);
            if stop_min <= 0 {
                return None;
            }
            let ratio = 64.0 / stop_min as f64;
            let at = |p: i64| nint(stop_min as f64 * ratio.powf(p as f64 / 13.0));
            let mut dk: Vec<i64> = (0..13).map(|p| at(p + 1) - at(p)).collect();
            dk.sort_unstable();
            stop_min + dk.iter().take(usize::from(s)).sum::<i64>()
        }
    }
    .min(64);
    // the low band reaches only the analysis bank's 32 channels (`kx` is checked against them
    // once the crossover is known)
    (1..=32).contains(&k0).then_some(())?;
    (k2 > k0).then_some((k0 as usize, k2 as usize))
}

/// The master table for `bs_freq_scale` 0: bands of one or two channels.
fn master_linear(k0: usize, k2: usize, alter_scale: bool) -> Option<Vec<usize>> {
    let (dk, bands) = if alter_scale { (2, 2 * nint((k2 - k0) as f64 / 4.0) as usize) } else { (1, 2 * ((k2 - k0) / 2)) };
    if bands == 0 {
        return None;
    }
    let mut v = vec![dk as i64; bands];
    let mut diff = k2 as i64 - (k0 + bands * dk) as i64;
    // take the difference from the lowest bands (too wide) or give it to the highest (too narrow)
    let mut k = if diff < 0 { 0 } else { bands - 1 };
    while diff != 0 {
        let step = if diff < 0 { 1 } else { -1 };
        *v.get_mut(k)? -= step;
        diff += step;
        k = if step == 1 { k + 1 } else { k.checked_sub(1)? };
    }
    cumulative(k0, &v)
}

/// The borders `start, start + d[0], …`; `None` unless every band is at least a channel wide.
fn cumulative(start: usize, d: &[i64]) -> Option<Vec<usize>> {
    let mut out = Vec::with_capacity(d.len() + 1);
    out.push(start);
    let mut at = start as i64;
    for &d in d {
        if d <= 0 {
            return None;
        }
        at += d;
        out.push(at as usize);
    }
    Some(out)
}

/// The band widths of `2 · NINT(bands · log2(hi / lo) / (2 · warp))` logarithmic bands.
fn log_widths(lo: usize, hi: usize, bands: f64, warp: f64) -> Vec<i64> {
    let ratio = hi as f64 / lo as f64;
    let n = 2 * nint(bands * ratio.log2() / (2.0 * warp));
    let at = |k: i64| nint(lo as f64 * ratio.powf(k as f64 / n as f64));
    let mut d: Vec<i64> = (0..n.max(0)).map(|k| at(k + 1) - at(k)).collect();
    d.sort_unstable();
    d
}

/// The master table for `bs_freq_scale` 1–3: 12, 10 or 8 bands per octave, warped above
/// `2 · k0` by `bs_alter_scale`.
fn master_log(k0: usize, k2: usize, freq_scale: u8, alter_scale: bool) -> Option<Vec<usize>> {
    let bands = [12.0, 10.0, 8.0][usize::from(freq_scale.clamp(1, 3) - 1)];
    let two_regions = k2 as f64 / k0 as f64 > 2.2449;
    let k1 = if two_regions { 2 * k0 } else { k2 };
    let d0 = log_widths(k0, k1, bands, 1.0);
    let mut table = cumulative(k0, &d0)?;
    if table.len() < 2 {
        return None;
    }
    if two_regions {
        let mut d1 = log_widths(k1, k2, bands, if alter_scale { 1.3 } else { 1.0 });
        let max0 = d0.iter().copied().max()?;
        if d1.first().is_some_and(|&d| d < max0) {
            // widen the first upper band to the widest lower one, at the cost of the last
            let change = max0 - d1[0];
            d1[0] = max0;
            *d1.last_mut()? -= change;
            d1.sort_unstable();
        }
        table.extend(cumulative(k1, &d1)?.into_iter().skip(1));
    }
    Some(table)
}

impl FreqTables {
    /// The tables of `h` at output rate `fs`, or `None` if the header asks for an invalid layout.
    pub(crate) fn new(h: &Header, fs: u32) -> Option<FreqTables> {
        let (k0, k2) = start_stop(h, fs)?;
        let master = if h.freq_scale == 0 { master_linear(k0, k2, h.alter_scale)? } else { master_log(k0, k2, h.freq_scale, h.alter_scale)? };
        let n_master = master.len().checked_sub(1)?;
        let xover = usize::from(h.xover_band);
        if xover >= n_master {
            return None;
        }
        let high: Vec<usize> = master.get(xover..)?.to_vec();
        let n_high = high.len() - 1;
        let (kx, end) = (high[0], *high.last()?);
        if kx > 32 || end > 64 || end <= kx {
            return None;
        }
        let m = end - kx;
        // every other border, keeping the first (and the second when the count is odd)
        let n_low = n_high - n_high / 2;
        let low: Vec<usize> = (0..=n_low).map(|k| high[if k == 0 { 0 } else { 2 * k - (n_high & 1) }]).collect();
        let n_q = if h.noise_bands == 0 { 1 } else { (nint(f64::from(h.noise_bands) * (end as f64 / kx as f64).log2()) as usize).max(1) };
        if n_q > 5 || n_q > n_low {
            return None;
        }
        let mut noise = vec![low[0]];
        let mut i = 0;
        for k in 1..=n_q {
            i += (n_low - i) / (n_q + 1 - k);
            noise.push(*low.get(i)?);
        }
        let patches = patches(&master, k0, kx, m, fs)?;
        let lim = limiter(&low, &patches, kx, h.limiter_bands);
        Some(FreqTables { k0, kx, m, high, low, noise, lim, patches })
    }

    /// High-resolution envelope bands.
    pub(crate) fn n_high(&self) -> usize {
        self.high.len() - 1
    }

    /// Noise floor bands.
    pub(crate) fn n_q(&self) -> usize {
        self.noise.len() - 1
    }

    /// The envelope band table of a resolution (`true`: high).
    pub(crate) fn bands(&self, high_res: bool) -> &[usize] {
        if high_res { &self.high } else { &self.low }
    }
}

/// Map the low band onto the SBR range (§4.6.18.6.3): runs of source channels, each ending below
/// `k0`, that together cover `kx .. kx + m`.
fn patches(master: &[usize], k0: usize, kx: usize, m: usize, fs: u32) -> Option<Vec<(usize, usize)>> {
    let n_master = master.len() - 1;
    let goal = nint(2.048e6 / f64::from(fs)).max(0) as usize;
    let mut k = if goal < kx + m { master.iter().position(|&f| f >= goal).unwrap_or(n_master) } else { n_master };
    let (mut msb, mut usb) = (k0, kx);
    let mut out: Vec<(usize, usize)> = Vec::new();
    // each pass either adds a patch or lowers `msb` to `kx`; a stream that does neither twice
    // running cannot finish. One patch over the limit may still be a short last one that is
    // dropped below.
    for _ in 0..2 * (MAX_PATCHES + 2) {
        // the highest master border the patch can reach from below k0
        let mut j = k + 1;
        let (sb, odd) = loop {
            j = j.checked_sub(1)?;
            let sb = *master.get(j)?;
            let odd = (sb + k0) % 2;
            if sb + odd < k0 + msb {
                break (sb, odd);
            }
        };
        let count = sb.saturating_sub(usb);
        if count > 0 {
            let start = (k0 - odd).checked_sub(count)?;
            out.push((start, count));
            if out.len() > MAX_PATCHES + 1 {
                return None;
            }
            (usb, msb) = (sb, sb);
        } else {
            msb = kx;
        }
        if master.get(k)?.saturating_sub(sb) < 3 {
            k = n_master;
        }
        if sb == kx + m {
            if out.len() > 1 && out.last().is_some_and(|p| p.1 < 3) {
                out.pop();
            }
            return (!out.is_empty() && out.len() <= MAX_PATCHES).then_some(out);
        }
    }
    None
}

/// The limiter band table (§4.6.18.7.3): one band over the range, or about 1.2, 2 or 3 bands per
/// octave on the low-resolution and patch borders.
fn limiter(low: &[usize], patches: &[(usize, usize)], kx: usize, limiter_bands: u8) -> Vec<usize> {
    let (Some(&first), Some(&last)) = (low.first(), low.last()) else {
        return Vec::new();
    };
    if limiter_bands == 0 {
        return vec![first, last];
    }
    let per_octave = [1.2, 2.0, 3.0][usize::from(limiter_bands.clamp(1, 3) - 1)];
    let mut borders = vec![kx];
    for p in patches {
        borders.push(borders.last().copied().unwrap_or(kx) + p.1);
    }
    let mut t: Vec<usize> = low.to_vec();
    t.extend(borders.iter().skip(1).take(patches.len().saturating_sub(1)));
    t.sort_unstable();
    let is_border = |v: usize| borders.contains(&v);
    let mut k = 1;
    while k < t.len() {
        let octaves = if t[k - 1] == 0 { f64::INFINITY } else { (t[k] as f64 / t[k - 1] as f64).log2() };
        if octaves * per_octave >= 0.49 {
            k += 1;
        } else if t[k] == t[k - 1] || !is_border(t[k]) {
            t.remove(k);
        } else if !is_border(t[k - 1]) {
            t.remove(k - 1);
        } else {
            k += 1;
        }
    }
    if t.len() < 2 {
        return vec![first, last];
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(start: u8, stop: u8, xover: u8, freq_scale: u8, alter: bool, noise: u8) -> Header {
        Header {
            amp_res: true,
            start_freq: start,
            stop_freq: stop,
            xover_band: xover,
            freq_scale,
            alter_scale: alter,
            noise_bands: noise,
            limiter_bands: 2,
            limiter_gains: 2,
            interpol_freq: true,
            smoothing_mode: true,
        }
    }

    /// Every header field combination at every output rate gives tables that are strictly
    /// increasing, start at `kx`, end at `kx + m`, keep the low band within the analysis bank and
    /// patch from below `k0` (or is refused).
    #[test]
    fn every_header_gives_consistent_tables_or_none() {
        let mut valid = 0;
        for fs in [16_000, 22_050, 24_000, 32_000, 44_100, 48_000, 64_000, 88_200, 96_000] {
            for start in 0..16 {
                for stop in 0..16 {
                    for xover in 0..8 {
                        for fsc in 0..4 {
                            for alter in [false, true] {
                                for noise in 0..4 {
                                    let Some(t) = FreqTables::new(&header(start, stop, xover, fsc, alter, noise), fs) else { continue };
                                    valid += 1;
                                    for v in [&t.high, &t.low, &t.noise, &t.lim] {
                                        assert!(v.windows(2).all(|w| w[0] < w[1]), "{v:?}");
                                        assert_eq!(v[0], t.kx);
                                    }
                                    for v in [&t.high, &t.low, &t.noise] {
                                        assert_eq!(*v.last().unwrap(), t.kx + t.m);
                                    }
                                    assert!(t.kx <= 32 && t.kx + t.m <= 64 && t.n_q() <= 5);
                                    // a last patch of under 3 channels is dropped, and the limiter
                                    // bands may then stop short of the range's end
                                    assert!(t.patches.iter().map(|p| p.1).sum::<usize>() <= t.m);
                                    assert!(t.lim.len() >= 2 && *t.lim.last().unwrap() <= t.kx + t.m);
                                    assert!(t.patches.iter().all(|&(s, n)| s + n <= t.k0));
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(valid > 10_000, "{valid}");
    }

    /// The highest start a header can give, 32 at 24 kHz (start 15), is valid: with stop 14
    /// (twice the start) and linear bands it covers channels 32 to 64.
    #[test]
    fn a_start_at_channel_32_is_accepted() {
        let t = FreqTables::new(&header(15, 14, 0, 0, false, 2), 24_000).unwrap();
        assert_eq!((t.k0, t.kx, t.m), (32, 32, 32));
    }

    /// At 96 kHz this header builds patches of 6, 6, 6, 6, 5 and 2 channels: the short sixth is
    /// dropped, as any short last patch is, and the five left are within the limit.
    #[test]
    fn a_short_sixth_patch_is_dropped_before_the_limit() {
        let t = FreqTables::new(&header(3, 9, 0, 0, false, 2), 96_000).unwrap();
        assert_eq!((t.k0, t.kx + t.m), (8, 39));
        assert_eq!(t.patches.iter().map(|p| p.1).collect::<Vec<_>>(), [6, 6, 6, 6, 5]);
    }

    /// A typical 44.1 kHz header: start channel 12 + offset, stop near 64, three noise bands.
    #[test]
    fn a_44k_header_has_the_expected_layout() {
        let t = FreqTables::new(&header(5, 9, 0, 2, true, 2), 44_100).unwrap();
        // startMin 12 + offset 2 = 14
        assert_eq!((t.k0, t.kx), (14, 14));
        assert_eq!(t.low.len() - 1, t.n_high() - t.n_high() / 2);
        assert!(t.n_q() >= 1);
    }
}
