//! Decoded picture buffer: reference marking (8.2.5), reference list construction (8.2.4) and output
//! ordering / bumping (C.4).
//!
//! The buffer only does bookkeeping on picture headers; what a picture is (`F`: the software
//! decoder's `FrameRef`, a hardware decoder's surface) is up to the decoder, so
//! hardware decoders driven by the same headers (VA-API) share this logic and output order.

use crate::h264::decode::error::{Result, ensure};
use crate::h264::decode::slice::{Mmco, SliceHeader, SliceType};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefMark {
    Unused,
    Short,
    Long,
}

/// Output-related metadata attached to a decoded frame.
#[derive(Clone, Debug)]
pub struct OutputMeta {
    pub pts: i64,
    pub key: bool,
    pub crop: (u32, u32, u32, u32),
    pub full_range: bool,
    pub colour_primaries: u8,
    pub transfer_characteristics: u8,
    pub matrix_coefficients: u8,
    pub sar: (u16, u16),
    /// Decoded without deblocking (draft mode, non-reference picture).
    pub draft: bool,
}

pub struct DpbEntry<F> {
    pub frame: F,
    pub poc: i32,
    pub frame_num: u32,
    pub long_term_frame_idx: u32,
    pub mark: RefMark,
    pub needed_for_output: bool,
    pub non_existing: bool,
    pub meta: Arc<OutputMeta>,
}

pub struct Dpb<F> {
    pub entries: Vec<DpbEntry<F>>,
    /// None = "no long-term frame indices".
    pub max_long_term_frame_idx: Option<u32>,
    pub capacity: usize,
    pub max_reorder: usize,
}

/// A picture leaving the DPB for output.
pub struct Output<F> {
    pub frame: F,
    pub poc: i32,
    pub meta: Arc<OutputMeta>,
}

impl<F: Clone> Dpb<F> {
    pub fn new() -> Self {
        Dpb { entries: Vec::new(), max_long_term_frame_idx: None, capacity: 16, max_reorder: 16 }
    }

    fn frame_num_wrap(e: &DpbEntry<F>, cur_frame_num: u32, max_frame_num: u32) -> i32 {
        if e.frame_num > cur_frame_num { e.frame_num as i32 - max_frame_num as i32 } else { e.frame_num as i32 }
    }

    fn num_refs(&self) -> usize {
        self.entries.iter().filter(|e| e.mark != RefMark::Unused).count()
    }

    /// Remove entries not needed for output and not used for reference.
    fn prune(&mut self) {
        self.entries.retain(|e| e.needed_for_output || e.mark != RefMark::Unused);
    }

    /// Output the picture with the smallest POC (C.4.5.3 "bumping"). Returns false if none waits.
    fn bump(&mut self, out: &mut Vec<Output<F>>) -> bool {
        let Some(i) = self.entries.iter().enumerate().filter(|(_, e)| e.needed_for_output).min_by_key(|(_, e)| e.poc).map(|(i, _)| i) else {
            return false;
        };
        let e = &mut self.entries[i];
        e.needed_for_output = false;
        out.push(Output { frame: e.frame.clone(), poc: e.poc, meta: e.meta.clone() });
        self.prune();
        true
    }

    /// Output everything in POC order and clear non-reference entries.
    pub fn flush(&mut self, out: &mut Vec<Output<F>>) {
        while self.bump(out) {}
        self.prune();
    }

    /// IDR: mark all references unused; output (or drop) prior pictures.
    pub fn idr(&mut self, no_output_of_prior_pics: bool, out: &mut Vec<Output<F>>) {
        for e in &mut self.entries {
            e.mark = RefMark::Unused;
        }
        if no_output_of_prior_pics {
            self.entries.clear();
        } else {
            self.flush(out);
        }
        self.max_long_term_frame_idx = None;
    }

    /// Sliding window marking (8.2.5.3).
    fn sliding_window(&mut self, cur_frame_num: u32, max_frame_num: u32, max_num_ref_frames: usize) {
        let limit = max_num_ref_frames.max(1);
        while self.num_refs() >= limit {
            let victim = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.mark == RefMark::Short)
                .min_by_key(|(_, e)| Self::frame_num_wrap(e, cur_frame_num, max_frame_num))
                .map(|(i, _)| i);
            match victim {
                Some(i) => self.entries[i].mark = RefMark::Unused,
                None => break,
            }
        }
        self.prune();
    }

    /// Insert "non-existing" frames for a frame_num gap (8.2.5.2). `make_frame` gives each one's
    /// picture and POC.
    ///
    /// Only the last `max_num_ref_frames` missing frames are built; the sliding window would drop
    /// the others before the gap ends. `poc_state` is not called once per missing frame: it sees the
    /// first skipped frame (when frames are skipped) and then each built frame, in decoding order.
    /// That suffices for state that only changes where frame_num decreases, like
    /// `PocState::update_gap_frame` (FrameNumOffset, 8.2.1), and keeps the work per gap bounded by
    /// `max_num_ref_frames` instead of MaxFrameNum.
    pub fn fill_frame_num_gap(
        &mut self,
        prev_ref_frame_num: u32,
        frame_num: u32,
        max_frame_num: u32,
        max_num_ref_frames: usize,
        make_frame: &mut dyn FnMut(u32) -> (F, i32, Arc<OutputMeta>),
        poc_state: &mut dyn FnMut(u32),
    ) {
        let mut unused = (prev_ref_frame_num + 1) % max_frame_num;
        // Missing frames before the last `max_num_ref_frames` would be dropped by the sliding window
        // (oldest FrameNumWrap first) before the gap ends, so no picture is built for them; building
        // pictures for them would cost up to max_frame_num frames per access unit.
        let gap = if frame_num >= unused { frame_num - unused } else { (frame_num % max_frame_num) + max_frame_num - unused };
        let skip = gap.saturating_sub(max_num_ref_frames.max(1).try_into().unwrap_or(u32::MAX));
        if skip > 0 {
            // Their POC bookkeeping (FrameNumOffset, 8.2.1) only grows when frame_num decreases,
            // which within one gap (shorter than MaxFrameNum) happens at most once, at the wrap
            // to 0. At least one built frame follows the skipped run, so visiting the first
            // skipped frame and then the built ones in order sees that decrease exactly when it
            // lies inside the skipped run: the state matches visiting every missing frame, in
            // constant time for any gap length, whatever frame_num the POC state saw last.
            let first = unused;
            let last = first.saturating_add(skip - 1) % max_frame_num;
            poc_state(first);
            unused = (last + 1) % max_frame_num;
        }
        let mut guard = skip;
        while unused != frame_num && guard < max_frame_num {
            self.sliding_window(unused, max_frame_num, max_num_ref_frames);
            let (frame, poc, meta) = make_frame(unused);
            poc_state(unused);
            self.entries.push(DpbEntry {
                poc,
                frame,
                frame_num: unused,
                long_term_frame_idx: 0,
                mark: RefMark::Short,
                needed_for_output: false,
                non_existing: true,
                meta,
            });
            unused = (unused + 1) % max_frame_num;
            guard += 1;
        }
    }

    /// Initial reference picture lists (8.2.4.2) + modification (8.2.4.3) for a frame slice, each
    /// entry made by `to_ref` from the DPB entry it refers to.
    pub fn build_ref_lists<R>(&self, sh: &SliceHeader, cur_poc: i32, max_frame_num: u32, to_ref: impl Fn(&DpbEntry<F>) -> R) -> Result<[Vec<R>; 2]> {
        let mut lists: [Vec<R>; 2] = [Vec::new(), Vec::new()];
        if sh.slice_type.is_intra() {
            return Ok(lists);
        }
        let cur_fn = sh.frame_num;
        ensure!(max_frame_num > 0 && max_frame_num <= (1 << 16) && cur_fn < max_frame_num, "frame number out of range");
        let pic_num = |e: &DpbEntry<F>| Self::frame_num_wrap(e, cur_fn, max_frame_num);
        let short: Vec<&DpbEntry<F>> = self.entries.iter().filter(|e| e.mark == RefMark::Short).collect();
        let mut long: Vec<&DpbEntry<F>> = self.entries.iter().filter(|e| e.mark == RefMark::Long).collect();
        long.sort_by_key(|e| e.long_term_frame_idx);
        let mut entry_lists: [Vec<&DpbEntry<F>>; 2] = [Vec::new(), Vec::new()];
        if sh.slice_type != SliceType::B {
            let mut s = short.clone();
            s.sort_by_key(|e| std::cmp::Reverse(pic_num(e)));
            entry_lists[0] = s.into_iter().chain(long.iter().copied()).collect();
        } else {
            let mut before: Vec<&DpbEntry<F>> = short.iter().copied().filter(|e| e.poc < cur_poc).collect();
            let mut after: Vec<&DpbEntry<F>> = short.iter().copied().filter(|e| e.poc > cur_poc).collect();
            before.sort_by_key(|e| std::cmp::Reverse(e.poc));
            after.sort_by_key(|e| e.poc);
            entry_lists[0] = before.iter().chain(after.iter()).chain(long.iter()).copied().collect();
            entry_lists[1] = after.iter().chain(before.iter()).chain(long.iter()).copied().collect();
            if entry_lists[1].len() > 1 && entry_lists[0].len() == entry_lists[1].len() {
                let same = entry_lists[0].iter().zip(entry_lists[1].iter()).all(|(a, b)| std::ptr::eq(*a, *b));
                if same {
                    entry_lists[1].swap(0, 1);
                }
            }
        }
        let nlists = if sh.slice_type == SliceType::B { 2 } else { 1 };
        for l in 0..nlists {
            let n = sh.num_ref_idx_active[l] as usize;
            let mut list: Vec<Option<&DpbEntry<F>>> = entry_lists[l].iter().copied().map(Some).collect();
            list.truncate(n);
            list.resize(n, None);
            if !sh.ref_pic_list_mod[l].is_empty() {
                let max_pic_num = max_frame_num as i32;
                let curr_pic_num = cur_fn as i32;
                let mut pred = curr_pic_num;
                let mut ref_idx = 0usize;
                for m in &sh.ref_pic_list_mod[l] {
                    ensure!(ref_idx < n, "too many ref_pic_list_modification entries");
                    ensure!(m.idc >= 2 || m.value < max_frame_num, "abs_diff_pic_num_minus1 out of range");
                    list.push(None); // temporarily one longer
                    let (target, is_long): (Option<&DpbEntry<F>>, bool) = match m.idc {
                        0 | 1 => {
                            let d = m.value as i32 + 1;
                            let no_wrap = if m.idc == 0 {
                                if pred - d < 0 { pred - d + max_pic_num } else { pred - d }
                            } else if pred + d >= max_pic_num {
                                pred + d - max_pic_num
                            } else {
                                pred + d
                            };
                            pred = no_wrap;
                            let pn = if no_wrap > curr_pic_num { no_wrap - max_pic_num } else { no_wrap };
                            (short.iter().copied().find(|e| pic_num(e) == pn), false)
                        }
                        2 => (long.iter().copied().find(|e| e.long_term_frame_idx == m.value), true),
                        _ => continue,
                    };
                    let Some(target) = target else {
                        // Missing picture: keep the list unchanged (error concealment).
                        list.pop();
                        ref_idx += 1;
                        continue;
                    };
                    for c in (ref_idx + 1..=n).rev() {
                        list[c] = list[c - 1];
                    }
                    list[ref_idx] = Some(target);
                    ref_idx += 1;
                    let mut nidx = ref_idx;
                    for c in ref_idx..=n {
                        let keep = match list[c] {
                            None => true,
                            Some(e) => {
                                if is_long {
                                    !(e.mark == RefMark::Long && e.long_term_frame_idx == target.long_term_frame_idx)
                                } else {
                                    !(e.mark == RefMark::Short && std::ptr::eq(e, target))
                                }
                            }
                        };
                        if keep {
                            list[nidx] = list[c];
                            nidx += 1;
                        }
                    }
                    list.truncate(n);
                }
            }
            // Fill missing entries (should not happen in conforming streams) with the first valid entry.
            let fallback = list.iter().flatten().next().copied().or_else(|| self.entries.last());
            let out: Vec<R> = list.iter().filter_map(|e| e.or(fallback)).map(&to_ref).collect();
            lists[l] = out;
        }
        Ok(lists)
    }

    /// Adaptive memory control (8.2.5.4). Returns true if MMCO 5 was executed.
    fn apply_mmcos(&mut self, mmcos: &[Mmco], cur_frame_num: u32, max_frame_num: u32, cur_long: &mut Option<u32>) -> Result<bool> {
        let mut had5 = false;
        let curr_pic_num = i64::from(cur_frame_num);
        for m in mmcos {
            match m.op {
                1 => {
                    let pn = curr_pic_num - (i64::from(m.difference_of_pic_nums_minus1) + 1);
                    for e in &mut self.entries {
                        if e.mark == RefMark::Short && i64::from(Self::frame_num_wrap(e, cur_frame_num, max_frame_num)) == pn {
                            e.mark = RefMark::Unused;
                        }
                    }
                }
                2 => {
                    for e in &mut self.entries {
                        if e.mark == RefMark::Long && e.long_term_frame_idx == m.long_term_pic_num {
                            e.mark = RefMark::Unused;
                        }
                    }
                }
                3 => {
                    ensure!(
                        self.max_long_term_frame_idx.is_some_and(|max| m.long_term_frame_idx <= max),
                        "MMCO long-term index exceeds the configured limit"
                    );
                    let pn = curr_pic_num - (i64::from(m.difference_of_pic_nums_minus1) + 1);
                    for e in &mut self.entries {
                        if e.mark == RefMark::Long && e.long_term_frame_idx == m.long_term_frame_idx {
                            e.mark = RefMark::Unused;
                        }
                    }
                    for e in &mut self.entries {
                        if e.mark == RefMark::Short && i64::from(Self::frame_num_wrap(e, cur_frame_num, max_frame_num)) == pn {
                            e.mark = RefMark::Long;
                            e.long_term_frame_idx = m.long_term_frame_idx;
                        }
                    }
                }
                4 => {
                    let max = m.max_long_term_frame_idx_plus1;
                    for e in &mut self.entries {
                        if e.mark == RefMark::Long && (max == 0 || e.long_term_frame_idx > max - 1) {
                            e.mark = RefMark::Unused;
                        }
                    }
                    self.max_long_term_frame_idx = if max == 0 { None } else { Some(max - 1) };
                }
                5 => {
                    for e in &mut self.entries {
                        e.mark = RefMark::Unused;
                    }
                    self.max_long_term_frame_idx = None;
                    had5 = true;
                }
                6 => {
                    ensure!(
                        self.max_long_term_frame_idx.is_some_and(|max| m.long_term_frame_idx <= max),
                        "MMCO long-term index exceeds the configured limit"
                    );
                    for e in &mut self.entries {
                        if e.mark == RefMark::Long && e.long_term_frame_idx == m.long_term_frame_idx {
                            e.mark = RefMark::Unused;
                        }
                    }
                    *cur_long = Some(m.long_term_frame_idx);
                }
                _ => {}
            }
        }
        self.prune();
        Ok(had5)
    }

    /// Reference marking for the just-decoded picture and insertion into the DPB with output bumping.
    /// Returns pictures ready for output.
    pub fn store_picture(
        &mut self,
        sh: &SliceHeader,
        frame: F,
        poc: i32,
        (max_frame_num, max_num_ref_frames): (u32, usize),
        meta: Arc<OutputMeta>,
        out: &mut Vec<Output<F>>,
    ) -> Result<()> {
        let mut mark = RefMark::Unused;
        let mut long_idx = 0;
        let mut had5 = false;
        if sh.nal_ref_idc != 0 {
            if sh.idr {
                if sh.long_term_reference {
                    mark = RefMark::Long;
                    self.max_long_term_frame_idx = Some(0);
                } else {
                    mark = RefMark::Short;
                    self.max_long_term_frame_idx = None;
                }
            } else {
                let mut cur_long = None;
                if sh.adaptive_ref_pic_marking {
                    had5 = self.apply_mmcos(&sh.mmcos, sh.frame_num, max_frame_num, &mut cur_long)?;
                } else {
                    self.sliding_window(sh.frame_num, max_frame_num, max_num_ref_frames);
                }
                if let Some(i) = cur_long {
                    mark = RefMark::Long;
                    long_idx = i;
                } else {
                    mark = RefMark::Short;
                    // make room if MMCOs left the DPB over-full (non-conforming streams)
                    if self.num_refs() >= max_num_ref_frames.max(1) && sh.adaptive_ref_pic_marking {
                        self.sliding_window(sh.frame_num, max_frame_num, max_num_ref_frames);
                    }
                }
            }
        }
        if mark == RefMark::Long {
            ensure!(self.num_refs() < max_num_ref_frames.max(1), "long-term reference exceeds max_num_ref_frames");
        }
        if had5 {
            // All earlier pictures are output before the current one (C.4.4).
            self.flush(out);
        }
        let frame_num = if had5 { 0 } else { sh.frame_num };
        // Bumping until there is space (C.4.5.1 / C.4.5.2).
        if mark == RefMark::Unused {
            loop {
                let waiting = self.entries.len();
                if waiting < self.capacity {
                    break;
                }
                // non-reference picture with the smallest POC is output directly
                let min_poc = self.entries.iter().filter(|e| e.needed_for_output).map(|e| e.poc).min();
                if min_poc.is_none_or(|m| poc < m) {
                    out.push(Output { frame, poc, meta });
                    return Ok(());
                }
                if !self.bump(out) {
                    break;
                }
            }
        } else {
            while self.entries.len() >= self.capacity {
                if !self.bump(out) {
                    break;
                }
            }
        }
        self.entries.push(DpbEntry {
            frame,
            poc,
            frame_num,
            long_term_frame_idx: long_idx,
            mark,
            needed_for_output: true,
            non_existing: false,
            meta,
        });
        // Low-delay output: honour max_num_reorder_frames.
        while self.entries.iter().filter(|e| e.needed_for_output).count() > self.max_reorder {
            if !self.bump(out) {
                break;
            }
        }
        Ok(())
    }
}

impl<F: Clone> Default for Dpb<F> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::h264::decode::slice::PocState;

    fn meta() -> Arc<OutputMeta> {
        Arc::new(OutputMeta {
            pts: 0,
            key: false,
            crop: (0, 0, 0, 0),
            full_range: false,
            colour_primaries: 2,
            transfer_characteristics: 2,
            matrix_coefficients: 2,
            sar: (1, 1),
            draft: false,
        })
    }

    fn reference_slice() -> SliceHeader {
        SliceHeader {
            nal_ref_idc: 1,
            nal_unit_type: 1,
            idr: false,
            first_mb_in_slice: 0,
            slice_type_raw: 0,
            slice_type: SliceType::P,
            pps_id: 0,
            colour_plane_id: 0,
            frame_num: 0,
            field_pic: false,
            bottom_field: false,
            idr_pic_id: 0,
            pic_order_cnt_lsb: 0,
            delta_pic_order_cnt_bottom: 0,
            delta_pic_order_cnt: [0; 2],
            redundant_pic_cnt: 0,
            direct_spatial_mv_pred: false,
            num_ref_idx_active: [1, 0],
            ref_pic_list_mod: [Vec::new(), Vec::new()],
            pred_weight_table: None,
            no_output_of_prior_pics: false,
            long_term_reference: false,
            adaptive_ref_pic_marking: false,
            mmcos: Vec::new(),
            cabac_init_idc: 0,
            slice_qp_delta: 0,
            sp_for_switch: false,
            slice_qs_delta: 0,
            disable_deblocking_filter_idc: 0,
            slice_alpha_c0_offset_div2: 0,
            slice_beta_offset_div2: 0,
            slice_group_change_cycle: 0,
            header_bits: 0,
        }
    }

    #[test]
    fn large_picture_number_differences_do_not_overflow() {
        for op in [1, 3] {
            for difference in [i32::MAX as u32, 1 << 31, u32::MAX] {
                let mut dpb: Dpb<u32> = Dpb::new();
                dpb.max_long_term_frame_idx = Some(0);
                let op = Mmco {
                    op,
                    difference_of_pic_nums_minus1: difference,
                    long_term_pic_num: 0,
                    long_term_frame_idx: 0,
                    max_long_term_frame_idx_plus1: 0,
                };
                assert!(dpb.apply_mmcos(&[op], 5, 16, &mut None).is_ok());
            }
        }
    }

    #[test]
    fn invalid_reference_list_differences_are_rejected() {
        use crate::h264::decode::slice::RefPicListMod;
        for idc in [0, 1] {
            for value in [0, 15] {
                let mut sh = reference_slice();
                sh.frame_num = 5;
                sh.ref_pic_list_mod[0].push(RefPicListMod { idc, value });
                let dpb: Dpb<u32> = Dpb::new();
                assert!(dpb.build_ref_lists(&sh, 0, 16, |entry| entry.frame).is_ok());
            }
            for value in [16, i32::MAX as u32, 1 << 31, u32::MAX] {
                let mut sh = reference_slice();
                sh.frame_num = 5;
                sh.ref_pic_list_mod[0].push(RefPicListMod { idc, value });
                let dpb: Dpb<u32> = Dpb::new();
                assert!(dpb.build_ref_lists(&sh, 0, 16, |entry| entry.frame).is_err());
            }
        }
    }

    /// A huge frame_num jump must only materialise the frames the sliding window keeps (#2779).
    #[test]
    fn frame_num_gap_materialises_at_most_max_num_ref_frames() {
        let mut dpb: Dpb<u32> = Dpb::new();
        let m = meta();
        let mut made = 0u32;
        let mut make = |f: u32| {
            made += 1;
            (f, 0, m.clone())
        };
        let mut updated = Vec::new();
        let mut upd = |f: u32| updated.push(f);
        dpb.fill_frame_num_gap(0, 32768, 1 << 16, 4, &mut make, &mut upd);
        assert!(made <= 4, "gap fill materialised {made} frames, expected <= 4");
        // the skipped frames 1..=32763 cost one POC update, the four built frames one each
        assert_eq!(updated, vec![1, 32764, 32765, 32766, 32767]);
        let mut refs: Vec<u32> = dpb.entries.iter().filter(|e| e.mark == RefMark::Short).map(|e| e.frame_num).collect();
        refs.sort_unstable();
        assert_eq!(refs, vec![32764, 32765, 32766, 32767]);
        assert!(dpb.entries.iter().all(|e| e.frame == e.frame_num));
    }

    /// A sequence parameter set with a `bits`-bit frame_num and POC type `poc_type`.
    fn gap_sps(bits: u32, poc_type: u32) -> crate::h264::decode::params::Sps {
        let mut w = craft_media::bitstream::BitWriter::new();
        w.write_bits(66, 8);
        w.write_bits(0, 8);
        w.write_bits(30, 8);
        w.write_ue(0); // sps id
        w.write_ue(bits - 4); // log2_max_frame_num - 4
        w.write_ue(poc_type);
        match poc_type {
            0 => w.write_ue(0),
            1 => {
                w.write_bit(false);
                w.write_se(-1); // offset_for_non_ref_pic
                w.write_se(1); // offset_for_top_to_bottom_field
                w.write_ue(1);
                w.write_se(2);
            }
            _ => {}
        }
        w.write_ue(1); // max_num_ref_frames
        w.write_bit(true); // gaps allowed
        w.write_ue(0);
        w.write_ue(0);
        w.write_bit(true);
        w.write_bit(true);
        w.write_bit(false);
        w.write_bit(false);
        w.rbsp_trailing();
        crate::h264::decode::params::Sps::parse(&w.finish()).expect("sps")
    }

    /// Fills the gap from `prev_ref` to `frame_num` in a DPB pre-filled with short-term references,
    /// driving the real `PocState::update_gap_frame` from `prior`, and checks the POC state, the
    /// POC of the picture after the gap (each POC type) and the built frames against visiting every
    /// missing frame (`every`: the state after visiting them all).
    fn check_gap_against_every_frame(
        sps: &[crate::h264::decode::params::Sps; 3],
        prev_ref: u32,
        frame_num: u32,
        mnrf: usize,
        prior: &PocState,
        every: &PocState,
    ) {
        let max = sps[0].max_frame_num();
        let m = meta();
        let mut dpb: Dpb<u32> = Dpb::new();
        for k in 0..mnrf.max(1) as u32 {
            let f = (prev_ref + max - k % max) % max;
            dpb.entries.push(DpbEntry {
                frame: f,
                poc: 0,
                frame_num: f,
                long_term_frame_idx: 0,
                mark: RefMark::Short,
                needed_for_output: false,
                non_existing: false,
                meta: m.clone(),
            });
        }
        let mut state = prior.clone();
        let mut calls = 0usize;
        let mut make = |f: u32| (f, 0, m.clone());
        let mut upd = |f: u32| {
            calls += 1;
            state.update_gap_frame(f, &sps[0]);
        };
        dpb.fill_frame_num_gap(prev_ref, frame_num, max, mnrf, &mut make, &mut upd);
        let at = || format!("MaxFrameNum {max} prev_ref {prev_ref} frame_num {frame_num} mnrf {mnrf} prior {prior:?}");
        assert_eq!(&state, every, "{}", at());
        let mut next = reference_slice();
        next.frame_num = frame_num;
        for s in sps {
            assert_eq!(state.compute(&next, s), every.compute(&next, s), "{}", at());
        }
        let gap = (frame_num + max - prev_ref - 1) % max;
        let built = gap.min(mnrf.max(1) as u32);
        assert!(calls <= 1 + built as usize, "{calls} POC updates, {}", at());
        let mut short: Vec<u32> = dpb.entries.iter().filter(|e| e.mark == RefMark::Short).map(|e| e.frame_num).collect();
        short.sort_unstable();
        let mut want: Vec<u32> = (0..built).map(|k| (frame_num + max - 1 - k) % max).collect();
        want.sort_unstable();
        assert!(dpb.entries.iter().filter(|e| e.non_existing).map(|e| e.frame_num).all(|f| want.contains(&f)), "{}", at());
        assert!(short.len() <= mnrf.max(1), "{}", at());
    }

    /// Prior POC states to start a gap from: the previous picture was the previous reference or the
    /// non-reference picture after it, plus one state outside that order (alternately a previous
    /// picture with another frame_num, or one from an SPS with a longer frame_num).
    fn gap_priors(sps: &crate::h264::decode::params::Sps, prev_ref: u32, extra: u32) -> Vec<PocState> {
        let max = sps.max_frame_num();
        let odd = if prev_ref.is_multiple_of(2) { (prev_ref + extra) % max } else { max + extra };
        [prev_ref, (prev_ref + 1) % max, odd]
            .into_iter()
            .map(|f| {
                let mut p = PocState::default();
                p.update_gap_frame(f, sps);
                p
            })
            .collect()
    }

    /// The gap fill gives the POC state the same FrameNumOffset as visiting every missing frame, with
    /// the decoder's real `PocState::update_gap_frame`: exhaustively for frame_num widths 4 to 8,
    /// and for a deterministic sample biased to gaps that wrap past frame_num 0 for widths 9 to 16.
    #[test]
    fn frame_num_gap_poc_bookkeeping_matches_visiting_every_frame() {
        const MNRF: [usize; 6] = [0, 1, 2, 3, 15, 16];
        let run = |bits: u32, prev_ref: u32, gaps: &[u32], extra: u32| {
            let sps = [gap_sps(bits, 0), gap_sps(bits, 1), gap_sps(bits, 2)];
            let max = sps[0].max_frame_num();
            for prior in gap_priors(&sps[0], prev_ref, extra) {
                // the state after visiting every missing frame, advanced one gap length at a time
                let mut every = prior.clone();
                let mut visited = 0u32;
                for &gap in gaps {
                    while visited < gap {
                        every.update_gap_frame((prev_ref + 1 + visited) % max, &sps[0]);
                        visited += 1;
                    }
                    let frame_num = (prev_ref + 1 + gap) % max;
                    for mnrf in MNRF {
                        check_gap_against_every_frame(&sps, prev_ref, frame_num, mnrf, &prior, &every);
                    }
                }
            }
        };
        for bits in 4..=8u32 {
            let max = 1u32 << bits;
            for prev_ref in 0..max {
                // every gap length 1..=max-2 (frame_num is neither PrevRefFrameNum nor the next one)
                let gaps: Vec<u32> = (1..max - 1).collect();
                run(bits, prev_ref, &gaps, prev_ref % 7 + 2);
            }
        }
        let mut seed = 0x2545_f491u32;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        for bits in 9..=16u32 {
            let max = 1u32 << bits;
            for i in 0..24 {
                // most gaps start shortly before the wrap and end shortly after it
                let prev_ref = if i % 4 == 3 { rand() % max } else { max - 1 - rand() % 40 };
                let mut gaps: Vec<u32> = (0..12)
                    .map(|_| 1 + rand() % (max - 2))
                    .chain([max - prev_ref - 1, max - prev_ref, max - 2])
                    .filter(|&g| (1..max - 1).contains(&g))
                    .collect();
                gaps.sort_unstable();
                gaps.dedup();
                run(bits, prev_ref, &gaps, rand() % max);
            }
        }
    }

    /// A gap that wraps (frame_num below the previous reference's) builds the frames just before
    /// frame_num and does constant work for the rest.
    #[test]
    fn wrapping_frame_num_gap_is_filled() {
        let mut dpb: Dpb<u32> = Dpb::new();
        let m = meta();
        let mut make = |f: u32| (f, 0, m.clone());
        let mut visited = Vec::new();
        let mut upd = |f: u32| visited.push(f);
        // 65530 -> 5: frames 65531..=65535 and 0..=4 are missing
        dpb.fill_frame_num_gap(65530, 5, 1 << 16, 3, &mut make, &mut upd);
        assert_eq!(visited, vec![65531, 2, 3, 4]);
        let mut refs: Vec<u32> = dpb.entries.iter().filter(|e| e.mark == RefMark::Short).map(|e| e.frame_num).collect();
        refs.sort_unstable();
        assert_eq!(refs, vec![2, 3, 4]);
    }

    /// The longest possible gap costs a bounded number of callbacks.
    #[test]
    fn longest_frame_num_gap_does_constant_work() {
        let mut dpb: Dpb<u32> = Dpb::new();
        let m = meta();
        let mut made = 0u32;
        let mut make = |f: u32| {
            made += 1;
            (f, 0, m.clone())
        };
        let mut calls = 0u32;
        let mut upd = |_f: u32| calls += 1;
        // prev_ref 65535 -> frame_num 65534: 65534 missing frames, wrapping through 0
        dpb.fill_frame_num_gap(65535, 65534, 1 << 16, 1, &mut make, &mut upd);
        assert_eq!(made, 1);
        assert!(calls <= 2, "{calls} POC updates");
    }

    /// Existing short-term references are still evicted by a gap no longer than the window.
    #[test]
    fn short_frame_num_gap_keeps_sliding_window_semantics() {
        let mut dpb: Dpb<u32> = Dpb::new();
        let m = meta();
        for f in 0..2 {
            dpb.entries.push(DpbEntry {
                frame: f,
                poc: 0,
                frame_num: f,
                long_term_frame_idx: 0,
                mark: RefMark::Short,
                needed_for_output: false,
                non_existing: false,
                meta: m.clone(),
            });
        }
        let mut make = |f: u32| (f, 0, m.clone());
        let mut upd = |_f: u32| {};
        dpb.fill_frame_num_gap(1, 5, 16, 4, &mut make, &mut upd);
        let mut refs: Vec<u32> = dpb.entries.iter().filter(|e| e.mark == RefMark::Short).map(|e| e.frame_num).collect();
        refs.sort_unstable();
        assert_eq!(refs, vec![1, 2, 3, 4]);
    }
}
