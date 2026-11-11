//! The colour description in an HEVC sequence parameter set's VUI (Rec. ITU-T H.265, 7.3.2.2 and
//! Annex E): the fallback a HEIF reader uses when the item carries no `colr` box of type `nclx`.
//! iPhone photos are the common case: they label their colour with an ICC profile only and say in
//! the VUI that their YCbCr is full range.
//!
//! Only the syntax up to `matrix_coeffs` is read. Anything this parser can't follow returns `None`
//! and the caller falls back to the H.265 defaults; it never fails a decode.

/// What the VUI's `video_signal_type` says (H.265 E.3.1). Absent fields keep the defaults the
/// standard infers: limited range, every code point 2 ("unspecified").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signal {
    pub full_range: bool,
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
}

impl Default for Signal {
    fn default() -> Self {
        Signal { full_range: false, primaries: 2, transfer: 2, matrix: 2 }
    }
}

/// NAL unit type of a sequence parameter set.
const SPS_NUT: u8 = 33;

/// Reads the signal description from the SPS among `parameter_sets` (NAL units with their
/// two-byte header, as an `hvcC` record stores them). `None`: no SPS, or one this reader can't
/// follow; `Some(Signal::default())`: an SPS without VUI colour fields.
pub fn signal(parameter_sets: &[&[u8]]) -> Option<Signal> {
    let sps = parameter_sets.iter().find(|n| n.first().is_some_and(|b| (b >> 1) & 0x3f == SPS_NUT))?;
    parse_sps(&unescape(sps.get(2..)?))
}

/// The uncropped picture size (`pic_width/height_in_luma_samples`) of every SPS among
/// `parameter_sets`: what the decoder allocates before it applies the conformance window, so the
/// pixel budget is checked against it (heic-rs reports only the first SPS, cropped). `None`: an
/// SPS this reader can't follow.
pub fn coded_sizes(parameter_sets: &[&[u8]]) -> Option<Vec<(u32, u32)>> {
    parameter_sets
        .iter()
        .filter(|n| n.first().is_some_and(|b| (b >> 1) & 0x3f == SPS_NUT))
        .map(|sps| {
            let rbsp = unescape(sps.get(2..)?);
            let r = &mut Bits { data: &rbsp, pos: 0 };
            sps_size(r).map(|(w, h, _)| (w, h))
        })
        .collect()
}

/// Reads an SPS up to and including the picture size, leaving `r` after it: width, height and
/// `sps_max_sub_layers_minus1`.
fn sps_size(r: &mut Bits) -> Option<(u32, u32, u32)> {
    r.skip(4)?; // sps_video_parameter_set_id
    let max_sub_layers_minus1 = r.u(3)?;
    if max_sub_layers_minus1 > 6 {
        return None;
    }
    r.skip(1)?; // sps_temporal_id_nesting_flag
    profile_tier_level(r, max_sub_layers_minus1)?;
    r.ue()?; // sps_seq_parameter_set_id
    if r.ue()? == 3 {
        r.skip(1)?; // separate_colour_plane_flag
    }
    let width = r.ue()?; // pic_width_in_luma_samples
    let height = r.ue()?; // pic_height_in_luma_samples
    Some((width, height, max_sub_layers_minus1))
}

/// Removes emulation prevention bytes (`00 00 03` → `00 00`).
fn unescape(nal: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(nal.len());
    let mut zeros = 0usize;
    for &b in nal {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        out.push(b);
    }
    out
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn u(&mut self, n: u32) -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            let byte = *self.data.get(self.pos / 8)?;
            v = (v << 1) | u32::from((byte >> (7 - self.pos % 8)) & 1);
            self.pos += 1;
        }
        Some(v)
    }

    fn flag(&mut self) -> Option<bool> {
        self.u(1).map(|b| b == 1)
    }

    fn skip(&mut self, n: usize) -> Option<()> {
        self.pos = self.pos.checked_add(n)?;
        (self.pos <= self.data.len().saturating_mul(8)).then_some(())
    }

    /// `ue(v)`; values past 32 bits are refused (no valid field here needs them).
    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0u32;
        while !self.flag()? {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        let rest = self.u(zeros)?;
        ((1u64 << zeros) - 1 + u64::from(rest)).try_into().ok()
    }

    fn se(&mut self) -> Option<()> {
        self.ue().map(|_| ())
    }
}

/// `profile_tier_level(1, max_sub_layers_minus1)` (7.3.3): only skipped.
fn profile_tier_level(r: &mut Bits, max_sub_layers_minus1: u32) -> Option<()> {
    r.skip(88)?; // general profile space .. general_inbld/reserved flag
    r.skip(8)?; // general_level_idc
    let mut present = [(false, false); 8];
    for p in present.iter_mut().take(max_sub_layers_minus1 as usize) {
        *p = (r.flag()?, r.flag()?);
    }
    if max_sub_layers_minus1 > 0 {
        r.skip(2 * (8 - max_sub_layers_minus1 as usize))?; // reserved_zero_2bits
    }
    for &(profile, level) in present.iter().take(max_sub_layers_minus1 as usize) {
        if profile {
            r.skip(88)?;
        }
        if level {
            r.skip(8)?;
        }
    }
    Some(())
}

/// `scaling_list_data()` (7.3.4): only skipped.
fn scaling_list_data(r: &mut Bits) -> Option<()> {
    for size_id in 0..4u32 {
        let step = if size_id == 3 { 3 } else { 1 };
        for _ in (0..6).step_by(step) {
            if !r.flag()? {
                r.ue()?; // scaling_list_pred_matrix_id_delta
            } else {
                let coefs = 64.min(1 << (4 + (size_id << 1)));
                if size_id > 1 {
                    r.se()?; // scaling_list_dc_coef_minus8
                }
                for _ in 0..coefs {
                    r.se()?; // scaling_list_delta_coef
                }
            }
        }
    }
    Some(())
}

/// `st_ref_pic_set(idx)` (7.3.7) as it appears in an SPS; returns `NumDeltaPocs[idx]`.
fn st_ref_pic_set(r: &mut Bits, idx: usize, num_delta_pocs: &[u32]) -> Option<u32> {
    if idx != 0 && r.flag()? {
        // inter_ref_pic_set_prediction_flag: predicted from the previous set (in an SPS,
        // delta_idx_minus1 is absent and RefRpsIdx = idx - 1).
        r.skip(1)?; // delta_rps_sign
        r.ue()?; // abs_delta_rps_minus1
        let reference = *num_delta_pocs.get(idx - 1)?;
        let mut count = 0u32;
        for _ in 0..=reference {
            let used = r.flag()?;
            let kept = used || r.flag()?; // use_delta_flag when not used_by_curr_pic_flag
            count += u32::from(kept);
        }
        return Some(count);
    }
    let negative = r.ue()?;
    let positive = r.ue()?;
    let total = negative.checked_add(positive)?;
    if total > 32 {
        return None; // at most 16 + 16 for any level
    }
    for _ in 0..total {
        r.ue()?; // delta_poc_s*_minus1
        r.skip(1)?; // used_by_curr_pic_s*_flag
    }
    Some(total)
}

fn parse_sps(rbsp: &[u8]) -> Option<Signal> {
    let r = &mut Bits { data: rbsp, pos: 0 };
    let (_, _, max_sub_layers_minus1) = sps_size(r)?;
    parse_sps_parameters(r, max_sub_layers_minus1)?;
    parse_vui(r)
}

fn parse_sps_parameters(r: &mut Bits<'_>, max_sub_layers_minus1: u32) -> Option<()> {
    if r.flag()? {
        for _ in 0..4 {
            r.ue()?; // conf_win_*_offset
        }
    }
    r.ue()?; // bit_depth_luma_minus8
    r.ue()?; // bit_depth_chroma_minus8
    let log2_max_poc_lsb = r.ue()?.checked_add(4)?;
    if log2_max_poc_lsb > 16 {
        return None;
    }
    let first = if r.flag()? { 0 } else { max_sub_layers_minus1 };
    for _ in first..=max_sub_layers_minus1 {
        r.ue()?; // sps_max_dec_pic_buffering_minus1
        r.ue()?; // sps_max_num_reorder_pics
        r.ue()?; // sps_max_latency_increase_plus1
    }
    for _ in 0..6 {
        r.ue()?; // coding/transform block sizes, transform hierarchy depths
    }
    if r.flag()? && r.flag()? {
        // scaling_list_enabled_flag, sps_scaling_list_data_present_flag
        scaling_list_data(r)?;
    }
    r.skip(2)?; // amp_enabled_flag, sample_adaptive_offset_enabled_flag
    if r.flag()? {
        // pcm_enabled_flag
        r.skip(8)?; // pcm sample bit depths
        r.ue()?;
        r.ue()?;
        r.skip(1)?; // pcm_loop_filter_disabled_flag
    }
    let sets = r.ue()? as usize;
    if sets > 64 {
        return None;
    }
    let mut num_delta_pocs = Vec::with_capacity(sets);
    for idx in 0..sets {
        let n = st_ref_pic_set(r, idx, &num_delta_pocs)?;
        num_delta_pocs.push(n);
    }
    if r.flag()? {
        // long_term_ref_pics_present_flag
        let n = r.ue()?;
        if n > 32 {
            return None;
        }
        for _ in 0..n {
            r.skip(log2_max_poc_lsb as usize + 1)?; // lt_ref_pic_poc_lsb_sps, used_by_curr_pic_lt_sps_flag
        }
    }
    r.skip(2)?; // sps_temporal_mvp_enabled_flag, strong_intra_smoothing_enabled_flag
    Some(())
}

fn parse_vui(r: &mut Bits<'_>) -> Option<Signal> {
    let mut signal = Signal::default();
    if !r.flag()? {
        return Some(signal); // no VUI
    }
    if r.flag()? && r.u(8)? == 255 {
        // aspect_ratio_info_present_flag, aspect_ratio_idc == EXTENDED_SAR
        r.skip(32)?;
    }
    if r.flag()? {
        r.skip(1)?; // overscan_appropriate_flag
    }
    if r.flag()? {
        // video_signal_type_present_flag
        r.skip(3)?; // video_format
        signal.full_range = r.flag()?;
        if r.flag()? {
            signal.primaries = r.u(8)? as u8;
            signal.transfer = r.u(8)? as u8;
            signal.matrix = r.u(8)? as u8;
        }
    }
    Some(signal)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bit writer for building parameter sets in tests.
    #[derive(Default)]
    pub(crate) struct Writer {
        pub bits: Vec<bool>,
    }

    impl Writer {
        pub fn u(&mut self, v: u32, n: u32) {
            for i in (0..n).rev() {
                self.bits.push(i < 32 && (v >> i) & 1 == 1);
            }
        }
        pub fn ue(&mut self, v: u32) {
            let x = u64::from(v) + 1;
            let len = 64 - x.leading_zeros();
            self.u(0, len - 1);
            for i in (0..len).rev() {
                self.bits.push((x >> i) & 1 == 1);
            }
        }
        pub fn bytes(mut self) -> Vec<u8> {
            self.bits.push(true); // rbsp_stop_one_bit
            while !self.bits.len().is_multiple_of(8) {
                self.bits.push(false);
            }
            self.bits.chunks(8).map(|c| c.iter().fold(0u8, |a, &b| (a << 1) | u8::from(b))).collect()
        }
    }

    fn sps(vui: impl FnOnce(&mut Writer), st_sets: impl FnOnce(&mut Writer)) -> Vec<u8> {
        let mut w = Writer::default();
        w.u(0, 4);
        w.u(1, 3); // one sub-layer above the base
        w.u(1, 1);
        w.u(0, 88);
        w.u(93, 8);
        w.u(0b11, 2); // sub-layer profile and level present
        w.u(0, 2 * 7);
        w.u(0, 88);
        w.u(0, 8);
        w.ue(0);
        w.ue(1); // 4:2:0
        w.ue(4032);
        w.ue(3024);
        w.u(1, 1); // conformance window
        for _ in 0..4 {
            w.ue(3);
        }
        w.ue(0);
        w.ue(0);
        w.ue(4);
        w.u(1, 1);
        for _ in 0..2 {
            w.ue(1);
            w.ue(0);
            w.ue(0);
        }
        for v in [0, 3, 0, 3, 0, 0] {
            w.ue(v);
        }
        w.u(1, 1); // scaling lists, with data
        w.u(1, 1);
        for size_id in 0..4u32 {
            for _ in (0..6).step_by(if size_id == 3 { 3 } else { 1 }) {
                w.u(1, 1);
                if size_id > 1 {
                    w.ue(0);
                }
                for _ in 0..64.min(1 << (4 + (size_id << 1))) {
                    w.ue(0);
                }
            }
        }
        w.u(0, 2);
        w.u(1, 1); // pcm
        w.u(0x77, 8);
        w.ue(0);
        w.ue(1);
        w.u(1, 1);
        st_sets(&mut w);
        w.u(1, 1); // long-term pictures
        w.ue(1);
        w.u(0, 8 + 1);
        w.u(0, 2);
        vui(&mut w);
        let mut nal = vec![SPS_NUT << 1, 1];
        // escape like a real NAL so `signal` has to unescape
        let mut zeros = 0;
        for b in w.bytes() {
            if zeros >= 2 && b <= 3 {
                nal.push(3);
                zeros = 0;
            }
            zeros = if b == 0 { zeros + 1 } else { 0 };
            nal.push(b);
        }
        nal
    }

    fn two_rps(w: &mut Writer) {
        w.ue(2);
        w.ue(1); // set 0: one negative picture
        w.ue(0);
        w.ue(0);
        w.u(1, 1);
        w.u(1, 1); // set 1: predicted from set 0
        w.u(0, 1);
        w.ue(0);
        w.u(0, 1); // not used ...
        w.u(1, 1); // ... but kept
        w.u(1, 1);
    }

    #[test]
    fn reads_full_range_and_matrix_from_the_vui() {
        let nal = sps(
            |w| {
                w.u(1, 1); // vui present
                w.u(1, 1);
                w.u(255, 8);
                w.u(0x0001_0001, 32);
                w.u(1, 1);
                w.u(0, 1);
                w.u(1, 1); // video_signal_type_present_flag
                w.u(5, 3);
                w.u(1, 1); // full range
                w.u(1, 1);
                w.u(12, 8);
                w.u(13, 8);
                w.u(6, 8);
            },
            two_rps,
        );
        let s = signal(&[&[0x40, 1, 0xc], &nal, &[0x44, 1]]);
        assert_eq!(s, Some(Signal { full_range: true, primaries: 12, transfer: 13, matrix: 6 }));
    }

    #[test]
    fn absent_fields_take_the_standard_defaults() {
        let no_vui = sps(|w| w.u(0, 1), |w| w.ue(0));
        assert_eq!(signal(&[&no_vui]), Some(Signal::default()));
        let range_only = sps(
            |w| {
                w.u(1, 1);
                w.u(0, 2);
                w.u(1, 1);
                w.u(5, 3);
                w.u(1, 1);
                w.u(0, 1); // no colour description
            },
            |w| w.ue(0),
        );
        assert_eq!(signal(&[&range_only]), Some(Signal { full_range: true, ..Signal::default() }));
    }

    #[test]
    fn missing_or_truncated_sps_is_none() {
        assert_eq!(signal(&[]), None);
        assert_eq!(signal(&[&[0x40, 1, 2, 3]]), None);
        let nal = sps(|w| w.u(0, 1), |w| w.ue(0));
        for cut in 2..nal.len() - 2 {
            // never panics; most cuts are None
            let _ = signal(&[nal.get(..cut).unwrap_or_default()]);
        }
        assert_eq!(signal(&[&[SPS_NUT << 1, 1, 0xff, 0xff, 0xff]]), None);
    }
}
