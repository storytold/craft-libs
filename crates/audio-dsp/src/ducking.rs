//! Offline analysis for automatic ducking: level envelope of the trigger audio
//! (dialogue, SFX…), activity detection with blip removal and gap bridging, and the volume
//! keyframes that duck a music clip under the detected regions.
//!
//! Pure functions on plain slices; times are seconds, levels dBFS / dB.

/// Level floor (dBFS) for silence.
pub const FLOOR_DB: f32 = -120.0;

/// Samples [`envelope_db`] advances per value: `hop_s` rounded to whole samples, at least one.
fn hop_samples(sample_rate: f32, hop_s: f32) -> usize {
    // `as usize` saturates and reads NaN as 0.
    ((hop_s * sample_rate).round() as usize).max(1)
}

/// The hop, in seconds, that [`envelope_db`] really advances by over `frames` frames at
/// `sample_rate`: `hop_s` rounded to whole samples, and no longer than the signal. Pass it to
/// [`activity`] so the regions line up with the audio.
pub fn envelope_hop_s(sample_rate: f32, hop_s: f32, frames: usize) -> f32 {
    let sr = crate::sane_sample_rate(sample_rate);
    (hop_samples(sr, hop_s).min(frames.max(1)) as f64 / sr as f64) as f32
}

/// RMS level (dBFS) per hop: value `k` is the RMS of the window of `window_s` seconds centered on
/// `k · hop` (clipped at the signal edges), where `hop` is `hop_s` rounded to whole samples
/// ([`envelope_hop_s`]), with the power averaged over channels. The result has `ceil(len / hop)`
/// entries; silence reads [`FLOOR_DB`].
pub fn envelope_db(channels: &[&[f32]], sample_rate: f32, hop_s: f32, window_s: f32) -> Vec<f32> {
    let len = channels.iter().map(|c| c.len()).min().unwrap_or(0);
    if len == 0 || channels.is_empty() {
        return Vec::new();
    }
    let sample_rate = crate::sane_sample_rate(sample_rate);
    // Neither width needs to exceed the signal.
    let hop = hop_samples(sample_rate, hop_s).min(len);
    let half = ((window_s * sample_rate / 2.0).round() as usize).clamp(1, len);
    // prefix sums of the channel-averaged power
    let nch = channels.len() as f64;
    let mut pre = Vec::with_capacity(len + 1);
    pre.push(0.0f64);
    let mut acc = 0.0f64;
    for i in 0..len {
        // A non-finite sample counts as silence.
        let p: f64 = channels.iter().map(|c| c[i] as f64).filter(|v| v.is_finite()).map(|v| v * v).sum::<f64>() / nch;
        acc += p;
        pre.push(acc);
    }
    (0..len.div_ceil(hop))
        .map(|k| {
            let c = k * hop;
            let (a, b) = (c.saturating_sub(half), (c + half).min(len));
            let ms = (pre[b] - pre[a]).max(0.0) / (b - a).max(1) as f64;
            if ms <= 1e-12 { FLOOR_DB } else { ((10.0 * ms.log10()) as f32).max(FLOOR_DB) }
        })
        .collect()
}

/// Regions `(start_s, end_s)` where the envelope exceeds `threshold_db`. Gaps shorter than
/// `bridge_s` are merged first, then regions shorter than `min_on_s` are dropped. A frame `k`
/// covers `[k·hop, (k+1)·hop)`. For an envelope from [`envelope_db`], pass
/// [`envelope_hop_s`] as `hop_s`.
///
/// `hop_s`, `min_on_s` and `bridge_s` are each read as the shortest decimal that gives the same
/// `f32` (0.01 is exactly 0.01 s), and every time is a whole number of hops computed in `f64`, so
/// a duration that equals a limit compares as equal and the times do not drift over a long
/// envelope.
pub fn activity(env_db: &[f32], hop_s: f32, threshold_db: f32, min_on_s: f32, bridge_s: f32) -> Vec<(f64, f64)> {
    let (hop, min_on_s, bridge_s) = (decimal(hop_s), decimal(min_on_s), decimal(bridge_s));
    let mut raw: Vec<(f64, f64)> = Vec::new();
    let mut start: Option<usize> = None;
    for (k, &v) in env_db.iter().enumerate() {
        match (v > threshold_db, start) {
            (true, None) => start = Some(k),
            (false, Some(s)) => {
                raw.push((s as f64 * hop, k as f64 * hop));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        raw.push((s as f64 * hop, env_db.len() as f64 * hop));
    }
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for r in raw {
        match merged.last_mut() {
            Some(last) if r.0 - last.1 < bridge_s - 1e-9 => last.1 = r.1,
            _ => merged.push(r),
        }
    }
    merged.retain(|(a, b)| b - a >= min_on_s - 1e-9);
    merged
}

/// `v` as the shortest decimal that rounds to it, in `f64` (`0.01f32` becomes `0.01`, not
/// `0.009999999776`).
fn decimal(v: f32) -> f64 {
    v.to_string().parse().unwrap_or(f64::from(v))
}

/// Ducking sensitivity (0…10) → detection threshold in dBFS:
/// `−20 − 4·s` (5 → −40 dBFS; higher sensitivity reacts to quieter triggers).
pub fn sensitivity_threshold_db(sensitivity: f32) -> f32 {
    -20.0 - 4.0 * sensitivity.clamp(0.0, 10.0)
}

/// Volume keyframes `(time_s, level_db)` that duck by `reduce_db` under each region: ramp from
/// `base_db` at `a − fade` down to `base_db − reduce_db` at `a`, hold until `b`, ramp back up to
/// `base_db` at `b + fade`. Regions whose ramps would overlap are merged (the level stays
/// ducked between them). Everything is clamped to `[clip_start_s, clip_end_s]`; a ramp cut by a
/// clip edge gets a keyframe at the edge with the ramp's value there (fully ducked when the region
/// itself reaches the edge). Times are strictly increasing; no regions → no keyframes. `fade_s` is
/// at least 1 ms so a ramp never collapses into two keyframes at the same time.
pub fn duck_keyframes(regions: &[(f64, f64)], clip_start_s: f64, clip_end_s: f64, base_db: f64, reduce_db: f64, fade_s: f64) -> Vec<(f64, f64)> {
    let fade = if fade_s.is_finite() { fade_s.max(1e-3) } else { 1e-3 };
    let ducked = base_db - reduce_db.abs();
    // A region just outside the clip still counts when its ramp reaches into the clip.
    let mut sorted: Vec<(f64, f64)> = regions.iter().copied().filter(|(a, b)| b > a && *b + fade > clip_start_s && *a - fade < clip_end_s).collect();
    sorted.sort_by(|x, y| x.0.total_cmp(&y.0));
    // merge regions whose ramps touch or overlap
    let mut groups: Vec<(f64, f64)> = Vec::new();
    for r in sorted {
        match groups.last_mut() {
            Some(g) if r.0 - fade <= g.1 + fade => g.1 = g.1.max(r.1),
            _ => groups.push(r),
        }
    }
    let mut out: Vec<(f64, f64)> = Vec::new();
    let mut push = |t: f64, v: f64| {
        if t < clip_start_s || t > clip_end_s {
            return;
        }
        match out.last_mut() {
            Some(last) if t <= last.0 + 1e-9 => {
                last.1 = v;
            }
            _ => out.push((t, v)),
        }
    };
    // value of the down-ramp ending at `a` / up-ramp starting at `b`, at time t
    let down = |a: f64, t: f64| base_db + (ducked - base_db) * ((t - (a - fade)) / fade).clamp(0.0, 1.0);
    let up = |b: f64, t: f64| ducked + (base_db - ducked) * ((t - b) / fade).clamp(0.0, 1.0);
    for (a, b) in groups {
        if b < clip_start_s {
            // Only the up-ramp reaches the clip.
            push(clip_start_s, up(b, clip_start_s));
            if b + fade <= clip_end_s {
                push(b + fade, base_db);
            } else {
                push(clip_end_s, up(b, clip_end_s));
            }
            continue;
        }
        if a > clip_end_s {
            // Only the down-ramp reaches the clip.
            let ramp_start = a - fade;
            push(ramp_start.max(clip_start_s), down(a, ramp_start.max(clip_start_s)));
            push(clip_end_s, down(a, clip_end_s));
            continue;
        }
        let ramp_start = a - fade;
        if ramp_start >= clip_start_s {
            push(ramp_start, base_db);
        } else {
            push(clip_start_s, down(a, clip_start_s));
        }
        push(a.max(clip_start_s), ducked);
        if b < clip_end_s {
            push(b, ducked);
            if b + fade <= clip_end_s {
                push(b + fade, base_db);
            } else {
                push(clip_end_s, up(b, clip_end_s));
            }
        } else {
            push(clip_end_s, ducked);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_levels() {
        let sr = 1000.0;
        // 1 s silence, 1 s of a ±0.1 square wave (−20 dBFS RMS)
        let x: Vec<f32> = (0..2000)
            .map(|i| {
                if i < 1000 {
                    0.0
                } else if i % 2 == 0 {
                    0.1
                } else {
                    -0.1
                }
            })
            .collect();
        let e = envelope_db(&[&x, &x], sr, 0.01, 0.02);
        assert_eq!(e.len(), 200);
        assert_eq!(e[10], FLOOR_DB);
        assert!((e[150] + 20.0).abs() < 0.01, "{}", e[150]);
        // the window straddling the edge is in between
        assert!(e[100] < -20.0 && e[100] > -30.0);
        assert!(envelope_db(&[], sr, 0.01, 0.02).is_empty());
    }

    #[test]
    fn activity_bridges_then_drops_blips() {
        let hop = 0.01;
        let mut env = vec![-80.0f32; 300];
        // speech 0.50–1.00 and 1.10–1.50 (gap 0.1 s → bridged), blip 2.00–2.03 (dropped)
        for (a, b) in [(50, 100), (110, 150), (200, 203)] {
            env[a..b].iter_mut().for_each(|v| *v = -20.0);
        }
        let r = activity(&env, hop, -40.0, 0.1, 0.2);
        assert_eq!(r.len(), 1);
        assert!((r[0].0 - 0.5).abs() < 1e-6 && (r[0].1 - 1.5).abs() < 1e-6, "{r:?}");
        // without bridging both remain, the blip still goes
        let r = activity(&env, hop, -40.0, 0.1, 0.0);
        assert_eq!(r.len(), 2);
        // a region running to the end is closed at the end
        let mut env2 = vec![-80.0f32; 10];
        env2[5..].iter_mut().for_each(|v| *v = 0.0);
        assert_eq!(activity(&env2, 0.1, -40.0, 0.0, 0.0), vec![(0.5, 1.0)]);
    }

    #[test]
    fn sensitivity_mapping() {
        assert_eq!(sensitivity_threshold_db(5.0), -40.0);
        assert_eq!(sensitivity_threshold_db(0.0), -20.0);
        assert_eq!(sensitivity_threshold_db(10.0), -60.0);
        assert_eq!(sensitivity_threshold_db(99.0), -60.0);
    }

    fn strictly_increasing(k: &[(f64, f64)]) -> bool {
        k.windows(2).all(|w| w[1].0 > w[0].0)
    }

    #[test]
    fn duck_keyframes_basic_and_merge() {
        let k = duck_keyframes(&[(2.0, 4.0)], 0.0, 10.0, 0.0, 15.0, 0.5);
        assert_eq!(k, vec![(1.5, 0.0), (2.0, -15.0), (4.0, -15.0), (4.5, 0.0)]);
        // ramps overlap (4.5 > 4.8 − 0.5 = 4.3) → stay ducked
        let k = duck_keyframes(&[(2.0, 4.0), (4.8, 6.0)], 0.0, 10.0, -3.0, 12.0, 0.5);
        assert_eq!(k, vec![(1.5, -3.0), (2.0, -15.0), (6.0, -15.0), (6.5, -3.0)]);
        // separate regions
        let k = duck_keyframes(&[(5.0, 6.0), (1.0, 2.0)], 0.0, 10.0, 0.0, 10.0, 0.25);
        assert_eq!(k.len(), 8);
        assert!(strictly_increasing(&k));
        assert!(duck_keyframes(&[], 0.0, 10.0, 0.0, 10.0, 0.5).is_empty());
    }

    #[test]
    fn duck_keyframes_clamped_to_clip() {
        // region starts before the clip: fully ducked at the start edge
        let k = duck_keyframes(&[(-1.0, 2.0)], 0.0, 10.0, 0.0, 10.0, 0.5);
        assert_eq!(k, vec![(0.0, -10.0), (2.0, -10.0), (2.5, 0.0)]);
        // ramp partially before the clip: ramp value at the edge
        let k = duck_keyframes(&[(0.25, 2.0)], 0.0, 10.0, 0.0, 10.0, 0.5);
        assert_eq!(k[0], (0.0, -5.0));
        assert_eq!(k[1], (0.25, -10.0));
        // region runs past the end: ducked at the end edge
        let k = duck_keyframes(&[(8.0, 12.0)], 0.0, 10.0, 0.0, 10.0, 0.5);
        assert_eq!(k, vec![(7.5, 0.0), (8.0, -10.0), (10.0, -10.0)]);
        // up-ramp cut by the end
        let k = duck_keyframes(&[(8.0, 9.75)], 0.0, 10.0, 0.0, 10.0, 0.5);
        assert_eq!(*k.last().unwrap(), (10.0, -5.0));
        // outside the clip: nothing; zero fade still strictly increasing
        assert!(duck_keyframes(&[(11.0, 12.0)], 0.0, 10.0, 0.0, 10.0, 0.5).is_empty());
        let k = duck_keyframes(&[(2.0, 3.0)], 0.0, 10.0, 0.0, 10.0, 0.0);
        assert!(strictly_increasing(&k) && k.len() == 4);
        // A region just past the end: its down-ramp lowers the end of the clip.
        let k = duck_keyframes(&[(10.2, 12.0)], 0.0, 10.0, 0.0, 10.0, 0.5);
        assert_eq!(k.len(), 2);
        assert!((k[0].0 - 9.7).abs() < 1e-12 && k[0].1 == 0.0, "{k:?}");
        assert!((k[1].0 - 10.0).abs() < 1e-12 && (k[1].1 + 6.0).abs() < 1e-9, "{k:?}");
        // A region just before the start: its up-ramp raises the start of the clip.
        let k = duck_keyframes(&[(-2.0, -0.25)], 0.0, 10.0, 0.0, 10.0, 0.5);
        assert_eq!(k, vec![(0.0, -5.0), (0.25, 0.0)]);
    }

    #[test]
    fn envelope_window_and_hop_of_any_size() {
        let x = vec![0.5f32; 4_800];
        for (hop, win) in [(0.01, f32::INFINITY), (0.01, 1e30), (f32::INFINITY, 0.1), (f32::NAN, f32::NAN)] {
            let e = envelope_db(&[&x], 48_000.0, hop, win);
            assert!(!e.is_empty() && e.iter().all(|v| v.is_finite()), "hop {hop} window {win}: {e:?}");
        }
        // A NaN or infinite sample does not hide the level after it.
        let mut y = x.clone();
        y[100] = f32::NAN;
        y[200] = f32::INFINITY;
        let e = envelope_db(&[&y], 48_000.0, 0.01, 0.02);
        assert!(e.iter().skip(2).all(|v| (v - 20.0 * 0.5f32.log10()).abs() < 0.1), "{e:?}");
    }

    /// A region of exactly `min_on_s` is kept and a gap of exactly `bridge_s` is not bridged: both
    /// limits are strict, and the f32 limits used to compare a little above their decimals.
    #[test]
    fn durations_equal_to_the_limits_compare_as_equal() {
        let on = |frames: &[bool]| frames.iter().map(|&a| if a { 0.0 } else { FLOOR_DB }).collect::<Vec<f32>>();
        let ten = on(&[[false; 5].as_slice(), &[true; 10], &[false; 5]].concat());
        assert_eq!(activity(&ten, 0.01, -40.0, 0.1, 0.0), vec![(0.05, 0.15)]);
        let gap = on(&[[true; 10].as_slice(), &[false; 20], &[true; 10]].concat());
        assert_eq!(activity(&gap, 0.01, -40.0, 0.0, 0.2), vec![(0.0, 0.1), (0.3, 0.4)]);
        assert_eq!(activity(&gap, 0.01, -40.0, 0.0, 0.21), vec![(0.0, 0.4)]);
    }

    /// At 44.1 kHz a one-sample hop rounded to 10 ns was 22.68 µs, so the end of a 60 s envelope
    /// landed at 60.011 s.
    #[test]
    fn a_one_sample_hop_does_not_drift() {
        let frames = 60 * 44_100;
        let hop = envelope_hop_s(44_100.0, 0.0, frames);
        let r = activity(&vec![0.0f32; frames], hop, -40.0, 0.0, 0.0);
        assert_eq!(r.len(), 1);
        assert!(r[0].0 == 0.0 && (r[0].1 - 60.0).abs() < 1e-5, "{r:?}");
    }

    #[test]
    fn regions_stay_on_the_audio_when_the_hop_is_not_whole_samples() {
        // 15 ms at 44.1 kHz is 661.5 samples; the envelope steps 662.
        let sr = 44_100.0;
        let start = 60 * 44_100;
        let mut x = vec![0.0f32; start + 44_100];
        x[start..].iter_mut().for_each(|v| *v = 0.5);
        let hop = envelope_hop_s(sr, 0.015, x.len());
        assert!((hop as f64 - 662.0 / 44_100.0).abs() < 1e-9, "{hop}");
        let env = envelope_db(&[&x], sr, 0.015, 0.03);
        let r = activity(&env, hop, -40.0, 0.1, 0.0);
        assert_eq!(r.len(), 1, "{r:?}");
        assert!((r[0].0 - 60.0).abs() < 0.016, "the region starts at {} s", r[0].0);
        // A hop longer than the clip is the clip's length.
        let clip = vec![0.5f32; 4_800];
        let hop = envelope_hop_s(48_000.0, 1.0, clip.len());
        let r = activity(&envelope_db(&[&clip], 48_000.0, 1.0, 0.02), hop, -40.0, 0.0, 0.0);
        assert_eq!(r, vec![(0.0, 0.1)]);
    }
}
