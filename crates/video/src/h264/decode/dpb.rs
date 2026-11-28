//! Decoded picture buffer: reference marking (8.2.5), reference list construction (8.2.4) and output
//! ordering / bumping (C.4).
//!
//! The buffer only does bookkeeping on picture headers; what a picture is (`F`: the software
//! decoder's `FrameRef`, a hardware decoder's surface) is up to the decoder, so
//! hardware decoders driven by the same headers (VA-API) share this logic and output order.

use crate::h264::decode::error::{Result, ensure, invalid};
use crate::h264::decode::slice::{Mmco, SliceHeader, SliceType};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefMark {
    Unused,
    Short,
    Long,
}

/// The metadata of a field pair: the first field's, marked draft when either field was decoded in
/// draft mode (the mode can change between the two fields).
fn paired_meta(first: Arc<OutputMeta>, second: &OutputMeta) -> Arc<OutputMeta> {
    if second.draft && !first.draft { Arc::new(OutputMeta { draft: true, ..(*first).clone() }) } else { first }
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
    /// PicOrderCnt of the frame, field pair or non-paired field: the smallest order count of the
    /// fields present (8.2.1).
    pub poc: i32,
    pub frame_num: u32,
    pub long_term_frame_idx: u32,
    /// Marking of the frame or complementary field pair as a whole: Short or Long only when both
    /// fields carry that mark (8.2.5, NOTE 2). Frame decoding uses this.
    pub mark: RefMark,
    pub needed_for_output: bool,
    pub non_existing: bool,
    pub meta: Arc<OutputMeta>,
    /// Marking of each field [top, bottom]; equal to `mark` for frames.
    pub field_mark: [RefMark; 2],
    /// TopFieldOrderCnt and BottomFieldOrderCnt.
    pub field_poc: [i32; 2],
    /// Fields present: bit 0 top, bit 1 bottom (3 for frames and complementary field pairs).
    pub fields: u8,
    /// Decoded as field pictures.
    pub field_pic: bool,
    /// The first field of a frame buffer whose second field may still follow: not output yet.
    pub open: bool,
}

impl<F> DpbEntry<F> {
    /// A frame entry (both fields present and marked alike).
    #[allow(clippy::too_many_arguments)]
    fn frame_entry(
        frame: F,
        field_poc: [i32; 2],
        frame_num: u32,
        long_term_frame_idx: u32,
        mark: RefMark,
        needed_for_output: bool,
        non_existing: bool,
        meta: Arc<OutputMeta>,
    ) -> Self {
        DpbEntry {
            frame,
            poc: field_poc[0].min(field_poc[1]),
            frame_num,
            long_term_frame_idx,
            mark,
            needed_for_output,
            non_existing,
            meta,
            field_mark: [mark; 2],
            field_poc,
            fields: 3,
            field_pic: false,
            open: false,
        }
    }

    /// At least one field is used for reference.
    pub fn is_ref(&self) -> bool {
        self.field_mark.iter().any(|m| *m != RefMark::Unused)
    }

    /// At least one field is marked `m`.
    fn any_field(&self, m: RefMark) -> bool {
        self.field_mark.contains(&m)
    }

    /// Mark the frame and both its fields.
    fn set_mark(&mut self, m: RefMark) {
        self.field_mark = [m; 2];
        self.sync_mark();
    }

    /// Mark the fields carrying `m` as unused.
    fn unmark(&mut self, m: RefMark) {
        for f in &mut self.field_mark {
            if *f == m {
                *f = RefMark::Unused;
            }
        }
        self.sync_mark();
    }

    /// Recompute the frame marking from the field markings.
    fn sync_mark(&mut self) {
        let [t, b] = self.field_mark;
        self.mark = if self.fields == 3 && t == b { t } else { RefMark::Unused };
    }
}

pub struct Dpb<F> {
    pub entries: Vec<DpbEntry<F>>,
    /// None = "no long-term frame indices".
    pub max_long_term_frame_idx: Option<u32>,
    pub capacity: usize,
    pub max_reorder: usize,
    /// pic_order_cnt_type is 0: "non-existing" frames have no order count and stay out of the
    /// lists of B slices (8.2.4.2.3, 8.2.4.2.4).
    pub poc_type0: bool,
    /// A non-reference first field that C.4.5.2 outputs directly (the DPB is full and it precedes
    /// everything waiting), held outside the DPB until its second field or the next picture.
    held: Option<HeldField<F>>,
}

/// See [`Dpb::held`].
struct HeldField<F> {
    frame: F,
    poc: i32,
    meta: Arc<OutputMeta>,
    parity: usize,
    frame_num: u32,
    /// Its second field may still follow.
    open: bool,
}

/// A picture leaving the DPB for output.
pub struct Output<F> {
    pub frame: F,
    pub poc: i32,
    pub meta: Arc<OutputMeta>,
}

impl<F: Clone> Dpb<F> {
    pub fn new() -> Self {
        Dpb { entries: Vec::new(), max_long_term_frame_idx: None, capacity: 16, max_reorder: 16, poc_type0: false, held: None }
    }

    fn frame_num_wrap(e: &DpbEntry<F>, cur_frame_num: u32, max_frame_num: u32) -> i32 {
        if e.frame_num > cur_frame_num { e.frame_num as i32 - max_frame_num as i32 } else { e.frame_num as i32 }
    }

    fn num_refs(&self) -> usize {
        self.entries.iter().filter(|e| e.is_ref()).count()
    }

    /// Remove entries not needed for output and not used for reference.
    fn prune(&mut self) {
        self.entries.retain(|e| e.needed_for_output || e.is_ref());
    }

    /// Output the picture with the smallest POC (C.4.5.3 "bumping"). Returns false if none waits.
    fn bump(&mut self, out: &mut Vec<Output<F>>) -> bool {
        let Some(i) = self.entries.iter().enumerate().filter(|(_, e)| e.needed_for_output && !e.open).min_by_key(|(_, e)| e.poc).map(|(i, _)| i)
        else {
            return false;
        };
        let e = &mut self.entries[i];
        e.needed_for_output = false;
        out.push(Output { frame: e.frame.clone(), poc: e.poc, meta: e.meta.clone() });
        self.prune();
        true
    }

    /// The field held for direct output ([`Dpb::held`]), if any.
    pub fn held(&self) -> Option<&F> {
        self.held.as_ref().map(|h| &h.frame)
    }

    /// Output a held field ([`Dpb::held`]) whose second field can no longer follow.
    fn release_held(&mut self, out: &mut Vec<Output<F>>) {
        if let Some(h) = self.held.take_if(|h| !h.open) {
            out.push(Output { frame: h.frame, poc: h.poc, meta: h.meta });
        }
    }

    /// Output everything in POC order and clear non-reference entries.
    pub fn flush(&mut self, out: &mut Vec<Output<F>>) {
        self.close_open();
        self.release_held(out);
        while self.bump(out) {}
        self.prune();
    }

    /// IDR: mark all references unused; output (or drop) prior pictures.
    pub fn idr(&mut self, no_output_of_prior_pics: bool, out: &mut Vec<Output<F>>) {
        self.close_open();
        self.release_held(out);
        for e in &mut self.entries {
            e.set_mark(RefMark::Unused);
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
        // numShortTerm + numLongTerm: an entry with a short-term and a long-term field counts in
        // both (8.2.5.3)
        let count =
            |d: &Self| d.entries.iter().map(|e| usize::from(e.any_field(RefMark::Short)) + usize::from(e.any_field(RefMark::Long))).sum::<usize>();
        while count(self) >= limit {
            let victim = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.any_field(RefMark::Short))
                .min_by_key(|(_, e)| Self::frame_num_wrap(e, cur_frame_num, max_frame_num))
                .map(|(i, _)| i);
            match victim {
                Some(i) => self.entries[i].set_mark(RefMark::Unused),
                None => break,
            }
        }
        self.prune();
    }

    /// Insert "non-existing" frames for a frame_num gap (8.2.5.2). `make_frame` gives each one's
    /// picture and POC.
    pub fn fill_frame_num_gap(
        &mut self,
        prev_ref_frame_num: u32,
        frame_num: u32,
        max_frame_num: u32,
        max_num_ref_frames: usize,
        make_frame: &mut dyn FnMut(u32) -> (F, i32, Arc<OutputMeta>),
        poc_state: &mut dyn FnMut(u32),
    ) {
        self.close_open();
        let mut unused = (prev_ref_frame_num + 1) % max_frame_num;
        // Missing frames before the last `max_num_ref_frames` would be dropped by the sliding window
        // (oldest FrameNumWrap first) before the gap ends, so only their POC / frame_num bookkeeping
        // runs; building pictures for them would cost up to max_frame_num frames per access unit.
        let gap = if frame_num >= unused { frame_num - unused } else { (frame_num % max_frame_num) + max_frame_num - unused };
        let skip = gap.saturating_sub(max_num_ref_frames.max(1).try_into().unwrap_or(u32::MAX));
        let mut guard = 0;
        while unused != frame_num && guard < max_frame_num {
            if guard < skip {
                poc_state(unused);
                unused = (unused + 1) % max_frame_num;
                guard += 1;
                continue;
            }
            self.sliding_window(unused, max_frame_num, max_num_ref_frames);
            let (frame, poc, meta) = make_frame(unused);
            poc_state(unused);
            self.entries.push(DpbEntry::frame_entry(frame, [poc; 2], unused, 0, RefMark::Short, false, true, meta));
            unused = (unused + 1) % max_frame_num;
            guard += 1;
        }
    }

    /// Build RefPicList0/1 for a slice of a frame (8.2.4.2.1, 8.2.4.2.3, 8.2.4.3). `to_ref` maps a
    /// DPB entry to the reference type used by the caller. For slices of field pictures use
    /// [`Dpb::build_field_ref_lists`].
    pub fn build_ref_lists<R>(&self, sh: &SliceHeader, cur_poc: i32, max_frame_num: u32, to_ref: impl Fn(&DpbEntry<F>) -> R) -> Result<[Vec<R>; 2]> {
        ensure!(!sh.field_pic, "frame reference lists built for a field picture");
        self.build_lists(sh, cur_poc, max_frame_num, |e, _| to_ref(e))
    }

    /// Build RefPicList0/1 for a slice of a frame or of a field picture (8.2.4.2 to 8.2.4.3).
    /// `to_ref` maps a DPB entry and, for a field picture, the parity of the referenced field
    /// (0 top, 1 bottom) to the reference type used by the caller.
    pub fn build_field_ref_lists<R>(
        &self,
        sh: &SliceHeader,
        cur_poc: i32,
        max_frame_num: u32,
        to_ref: impl Fn(&DpbEntry<F>, Option<usize>) -> R,
    ) -> Result<[Vec<R>; 2]> {
        self.build_lists(sh, cur_poc, max_frame_num, to_ref)
    }

    fn build_lists<R>(
        &self,
        sh: &SliceHeader,
        cur_poc: i32,
        max_frame_num: u32,
        to_ref: impl Fn(&DpbEntry<F>, Option<usize>) -> R,
    ) -> Result<[Vec<R>; 2]> {
        let mut lists: [Vec<R>; 2] = [Vec::new(), Vec::new()];
        if sh.slice_type.is_intra() {
            return Ok(lists);
        }
        let cur_fn = sh.frame_num;
        ensure!(max_frame_num > 0 && max_frame_num <= (1 << 16) && cur_fn < max_frame_num, "frame number out of range");
        let field = sh.field_pic;
        let cur_par = sh.bottom_field as usize;
        let wrap = |e: &DpbEntry<F>| Self::frame_num_wrap(e, cur_fn, max_frame_num);
        // A list item: an entry and, in a field picture, the parity of one of its fields.
        type Item<'a, F> = (&'a DpbEntry<F>, Option<usize>);
        let marked = |it: &Item<F>, m: RefMark| match it.1 {
            None => it.0.mark == m,
            Some(p) => it.0.field_mark[p] == m,
        };
        // PicNum and LongTermPicNum (8.2.4.1)
        let same = |it: &Item<F>| i32::from(it.1 == Some(cur_par));
        let pic_num = |it: &Item<F>| if field { 2 * wrap(it.0) + same(it) } else { wrap(it.0) };
        let long_pic_num = |it: &Item<F>| {
            let idx = it.0.long_term_frame_idx as i64;
            if field { 2 * idx + i64::from(same(it)) } else { idx }
        };
        // Reference entries ("frames" in 8.2.4.2.2 and 8.2.4.2.4): for fields any entry with a
        // field of that marking, for frames only frames and complementary pairs marked as a whole.
        let refs = |m: RefMark| -> Vec<&DpbEntry<F>> { self.entries.iter().filter(|e| if field { e.any_field(m) } else { e.mark == m }).collect() };
        let short = refs(RefMark::Short);
        let mut long = refs(RefMark::Long);
        long.sort_by_key(|e| e.long_term_frame_idx);
        let mut entry_lists: [Vec<&DpbEntry<F>>; 2] = [Vec::new(), Vec::new()];
        if sh.slice_type != SliceType::B {
            let mut s = short.clone();
            s.sort_by_key(|e| std::cmp::Reverse(wrap(e)));
            entry_lists[0] = s;
        } else {
            // frames: before is PicOrderCnt < current; fields: <= current (8.2.4.2.4)
            let is_before = |e: &&DpbEntry<F>| if field { e.poc <= cur_poc } else { e.poc < cur_poc };
            let ordered = |e: &&&DpbEntry<F>| !(self.poc_type0 && e.non_existing);
            let mut before: Vec<&DpbEntry<F>> = short.iter().filter(ordered).copied().filter(is_before).collect();
            let mut after: Vec<&DpbEntry<F>> = short.iter().filter(ordered).copied().filter(|e| !is_before(e)).collect();
            if !field {
                after.retain(|e| e.poc > cur_poc);
            }
            before.sort_by_key(|e| std::cmp::Reverse(e.poc));
            after.sort_by_key(|e| e.poc);
            entry_lists[0] = before.iter().chain(after.iter()).copied().collect();
            entry_lists[1] = after.iter().chain(before.iter()).copied().collect();
        }
        let nlists = if sh.slice_type == SliceType::B { 2 } else { 1 };
        // Initial lists: short-term then long-term entries; in a field picture each expanded into
        // fields of alternating parity (8.2.4.2.5).
        let mut init: [Vec<Item<F>>; 2] = [Vec::new(), Vec::new()];
        for l in 0..nlists {
            if field {
                init[l] = alternate_fields(&entry_lists[l], cur_par, RefMark::Short);
                init[l].extend(alternate_fields(&long, cur_par, RefMark::Long));
            } else {
                init[l] = entry_lists[l].iter().chain(long.iter()).map(|e| (*e, None)).collect();
            }
        }
        if nlists == 2 && init[1].len() > 1 && init[0].len() == init[1].len() {
            let same = init[0].iter().zip(init[1].iter()).all(|(a, b)| std::ptr::eq(a.0, b.0) && a.1 == b.1);
            if same {
                init[1].swap(0, 1);
            }
        }
        let (max_pic_num, curr_pic_num) =
            if field { (2 * max_frame_num as i32, 2 * cur_fn as i32 + 1) } else { (max_frame_num as i32, cur_fn as i32) };
        // every short-term and long-term reference picture, for the modification process
        let pictures = |m: RefMark| -> Vec<Item<F>> {
            let es = if m == RefMark::Short { &short } else { &long };
            if field {
                es.iter().flat_map(|e| [(*e, Some(0)), (*e, Some(1))]).filter(|it| marked(it, m)).collect()
            } else {
                es.iter().map(|e| (*e, None)).collect()
            }
        };
        let short_pics = pictures(RefMark::Short);
        let long_pics = pictures(RefMark::Long);
        for l in 0..nlists {
            let n = sh.num_ref_idx_active[l] as usize;
            let mut list: Vec<Option<Item<F>>> = init[l].iter().copied().map(Some).collect();
            list.truncate(n);
            list.resize(n, None);
            if !sh.ref_pic_list_mod[l].is_empty() {
                let mut pred = curr_pic_num;
                let mut ref_idx = 0usize;
                for m in &sh.ref_pic_list_mod[l] {
                    ensure!(ref_idx < n, "too many ref_pic_list_modification entries");
                    ensure!(m.idc >= 2 || (m.value as i64) < i64::from(max_pic_num), "abs_diff_pic_num_minus1 out of range");
                    list.push(None); // temporarily one longer
                    let target: Option<Item<F>> = match m.idc {
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
                            short_pics.iter().copied().find(|it| pic_num(it) == pn)
                        }
                        2 => long_pics.iter().copied().find(|it| long_pic_num(it) == i64::from(m.value)),
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
                        // drop the later copy of the inserted picture (8-37, 8-38)
                        let keep = list[c].is_none_or(|it| !(std::ptr::eq(it.0, target.0) && it.1 == target.1));
                        if keep {
                            list[nidx] = list[c];
                            nidx += 1;
                        }
                    }
                    list.truncate(n);
                }
            }
            // Fill missing entries (should not happen in conforming streams) with the first valid entry.
            let fallback = list.iter().flatten().next().copied().or_else(|| self.entries.last().map(|e| (e, field.then_some(cur_par))));
            let out: Vec<R> = list.iter().filter_map(|e| e.or(fallback)).map(|(e, p)| to_ref(e, p)).collect();
            lists[l] = out;
        }
        Ok(lists)
    }

    /// Apply the MMCOs of the current picture (8.2.5.4). `field` is the parity of the current
    /// field in a field picture; `own` is the DPB index of the first field of the current
    /// picture's frame when the current picture is a second field.
    fn apply_mmcos(
        &mut self,
        mmcos: &[Mmco],
        (cur_frame_num, max_frame_num): (u32, u32),
        field: Option<usize>,
        own: Option<usize>,
        cur_long: &mut Option<u32>,
    ) -> Result<bool> {
        let mut had5 = false;
        let curr_pic_num = match field {
            None => i64::from(cur_frame_num),
            Some(_) => 2 * i64::from(cur_frame_num) + 1,
        };
        // PicNum and LongTermPicNum of field `p` of an entry, or of the frame for `None` (8.2.4.1)
        let same = |p: usize| i64::from(Some(p) == field);
        let pic_num = |e: &DpbEntry<F>, p: Option<usize>| {
            let w = i64::from(Self::frame_num_wrap(e, cur_frame_num, max_frame_num));
            p.map_or(w, |p| 2 * w + same(p))
        };
        let long_pic_num = |e: &DpbEntry<F>, p: Option<usize>| {
            let idx = i64::from(e.long_term_frame_idx);
            p.map_or(idx, |p| 2 * idx + same(p))
        };
        // the reference pictures a picture number can name: frames, or fields
        let views: &[Option<usize>] = if field.is_some() { &[Some(0), Some(1)] } else { &[None] };
        let marked = |e: &DpbEntry<F>, p: Option<usize>, m: RefMark| p.map_or(e.mark == m, |p| e.field_mark[p] == m);
        for m in mmcos {
            match m.op {
                1 | 2 => {
                    let (want, num) = if m.op == 1 {
                        (RefMark::Short, curr_pic_num - (i64::from(m.difference_of_pic_nums_minus1) + 1))
                    } else {
                        (RefMark::Long, i64::from(m.long_term_pic_num))
                    };
                    for e in &mut self.entries {
                        for &p in views {
                            let n = if m.op == 1 { pic_num(e, p) } else { long_pic_num(e, p) };
                            if marked(e, p, want) && n == num {
                                // a field leaves the marking of the other field of its frame as it is
                                match p {
                                    None => e.set_mark(RefMark::Unused),
                                    Some(p) => {
                                        e.field_mark[p] = RefMark::Unused;
                                        e.sync_mark();
                                    }
                                }
                            }
                        }
                    }
                }
                3 => {
                    ensure!(
                        self.max_long_term_frame_idx.is_some_and(|max| m.long_term_frame_idx <= max),
                        "MMCO long-term index exceeds the configured limit"
                    );
                    let pn = curr_pic_num - (i64::from(m.difference_of_pic_nums_minus1) + 1);
                    let target = self
                        .entries
                        .iter()
                        .enumerate()
                        .find_map(|(i, e)| views.iter().find(|&&p| marked(e, p, RefMark::Short) && pic_num(e, p) == pn).map(|&p| (i, p)));
                    for (i, e) in self.entries.iter_mut().enumerate() {
                        // a field of the same frame as the target keeps its long-term index
                        let own_frame = field.is_some() && target.is_some_and(|(t, _)| t == i);
                        if e.any_field(RefMark::Long) && e.long_term_frame_idx == m.long_term_frame_idx && !own_frame {
                            e.unmark(RefMark::Long);
                        }
                    }
                    if let Some((i, p)) = target
                        && let Some(e) = self.entries.get_mut(i)
                    {
                        match p {
                            None => e.set_mark(RefMark::Long),
                            Some(p) => {
                                e.field_mark[p] = RefMark::Long;
                                e.sync_mark();
                            }
                        }
                        e.long_term_frame_idx = m.long_term_frame_idx;
                    }
                }
                4 => {
                    let max = m.max_long_term_frame_idx_plus1;
                    for e in &mut self.entries {
                        if e.any_field(RefMark::Long) && (max == 0 || e.long_term_frame_idx > max - 1) {
                            e.unmark(RefMark::Long);
                        }
                    }
                    self.max_long_term_frame_idx = if max == 0 { None } else { Some(max - 1) };
                }
                5 => {
                    for e in &mut self.entries {
                        e.set_mark(RefMark::Unused);
                    }
                    self.max_long_term_frame_idx = None;
                    had5 = true;
                }
                6 => {
                    ensure!(
                        self.max_long_term_frame_idx.is_some_and(|max| m.long_term_frame_idx <= max),
                        "MMCO long-term index exceeds the configured limit"
                    );
                    for (i, e) in self.entries.iter_mut().enumerate() {
                        // the first field of the current frame keeps its long-term index
                        if e.any_field(RefMark::Long) && e.long_term_frame_idx == m.long_term_frame_idx && own != Some(i) {
                            e.unmark(RefMark::Long);
                        }
                    }
                    *cur_long = Some(m.long_term_frame_idx);
                }
                _ => {}
            }
        }
        // (the first field of the current frame waits for output, so it stays)
        self.prune();
        Ok(had5)
    }

    /// Mark the "open" first field (if any) as not awaiting its second field any more.
    fn close_open(&mut self) {
        for e in &mut self.entries {
            e.open = false;
        }
        if let Some(h) = &mut self.held {
            h.open = false;
        }
    }

    /// Reference marking of the current picture (8.2.5.1) apart from the case of a second field
    /// completing a reference field pair: runs the sliding window or the MMCOs and returns the
    /// current picture's marking, its LongTermFrameIdx and whether MMCO 5 ran.
    fn mark_current(
        &mut self,
        sh: &SliceHeader,
        (max_frame_num, max_num_ref_frames): (u32, usize),
        own: Option<usize>,
    ) -> Result<(RefMark, u32, bool)> {
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
                    let field = sh.field_pic.then_some(sh.bottom_field as usize);
                    had5 = self.apply_mmcos(&sh.mmcos, (sh.frame_num, max_frame_num), field, own, &mut cur_long)?;
                } else {
                    self.sliding_window(sh.frame_num, max_frame_num, max_num_ref_frames);
                }
                if let Some(i) = cur_long {
                    mark = RefMark::Long;
                    long_idx = i;
                } else {
                    mark = RefMark::Short;
                    // make room if MMCOs left the DPB over-full (non-conforming streams)
                    // (a second field adds no frame)
                    if own.is_none() && self.num_refs() >= max_num_ref_frames.max(1) && sh.adaptive_ref_pic_marking {
                        self.sliding_window(sh.frame_num, max_frame_num, max_num_ref_frames);
                    }
                }
            }
        }
        // a second field joins a frame that is counted already (8.2.5.4.6)
        if mark == RefMark::Long {
            ensure!(self.num_refs() + usize::from(own.is_none()) <= max_num_ref_frames.max(1), "long-term reference exceeds max_num_ref_frames");
        }
        Ok((mark, long_idx, had5))
    }

    /// Output pictures until fewer than `max_reorder` wait (max_num_reorder_frames).
    fn bump_reorder(&mut self, out: &mut Vec<Output<F>>) {
        while self.entries.iter().filter(|e| e.needed_for_output && !e.open).count() > self.max_reorder {
            if !self.bump(out) {
                break;
            }
        }
    }

    /// Reference marking for the just-decoded frame and insertion into the DPB with output bumping.
    /// Returns pictures ready for output. `poc` is the frame's PicOrderCnt; [`Dpb::store_frame`]
    /// also records the order counts of its two fields.
    pub fn store_picture(
        &mut self,
        sh: &SliceHeader,
        frame: F,
        poc: i32,
        limits: (u32, usize),
        meta: Arc<OutputMeta>,
        out: &mut Vec<Output<F>>,
    ) -> Result<()> {
        self.store_frame(sh, frame, [poc; 2], limits, meta, out)
    }

    /// [`Dpb::store_picture`] with TopFieldOrderCnt and BottomFieldOrderCnt of the frame.
    pub fn store_frame(
        &mut self,
        sh: &SliceHeader,
        frame: F,
        field_poc: [i32; 2],
        limits: (u32, usize),
        meta: Arc<OutputMeta>,
        out: &mut Vec<Output<F>>,
    ) -> Result<()> {
        ensure!(!sh.field_pic, "a field picture stored as a frame (use store_field)");
        self.close_open();
        // (a failed marking keeps the held field for the next picture or the flush)
        let (mark, long_idx, had5) = self.mark_current(sh, limits, None)?;
        self.release_held(out);
        if had5 {
            // All earlier pictures are output before the current one (C.4.4).
            self.flush(out);
        }
        let poc = field_poc[0].min(field_poc[1]);
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
        self.entries.push(DpbEntry::frame_entry(frame, field_poc, frame_num, long_idx, mark, true, false, meta));
        // Low-delay output: honour max_num_reorder_frames.
        self.bump_reorder(out);
        Ok(())
    }

    /// Reference marking and storage of a decoded field of parity `parity` (0 top, 1 bottom) with
    /// order count `poc`. A field completing a complementary field pair with the first field just
    /// stored (3.36, 3.37) joins that field's frame buffer, `pair` merging it into the stored
    /// picture (C.4.5); any other field takes a frame buffer of its own and stays there, not
    /// output, until the next picture shows whether a second field follows. `fits` tells whether a
    /// stored first field can take this field as its second (same picture size).
    #[allow(clippy::too_many_arguments)]
    pub fn store_field(
        &mut self,
        sh: &SliceHeader,
        field: F,
        parity: usize,
        poc: i32,
        limits: (u32, usize),
        meta: Arc<OutputMeta>,
        out: &mut Vec<Output<F>>,
        fits: impl Fn(&F) -> bool,
        pair: impl FnOnce(&mut F, F),
    ) -> Result<()> {
        ensure!(sh.field_pic, "a frame stored as a field (use store_frame)");
        let parity = parity & 1;
        let is_ref = sh.nal_ref_idc != 0;
        let pairs_held = self
            .held
            .as_ref()
            .is_some_and(|h| h.open && h.parity != parity && h.frame_num == sh.frame_num && !is_ref && !sh.idr && !sh.has_mmco5() && fits(&h.frame));
        if pairs_held && let Some(mut h) = self.held.take() {
            // the second field of a held non-reference pair: the pair is output directly
            pair(&mut h.frame, field);
            out.push(Output { frame: h.frame, poc: h.poc.min(poc), meta: paired_meta(h.meta, &meta) });
            return Ok(());
        }
        let second = self.entries.iter().position(|e| {
            e.open
                && e.fields == 1 << (parity ^ 1)
                && e.frame_num == sh.frame_num
                && !sh.idr
                && !sh.has_mmco5()
                && (e.field_mark[parity ^ 1] != RefMark::Unused) == is_ref
                && fits(&e.frame)
        });
        if let Some(i) = second {
            // 8.2.5.3: the second field of a reference pair whose first field is short-term is
            // marked short-term without the sliding window.
            let first_short = self.entries[i].field_mark[parity ^ 1] == RefMark::Short;
            let (mark, long_idx) = if !is_ref {
                (RefMark::Unused, 0)
            } else if first_short && !sh.adaptive_ref_pic_marking {
                (RefMark::Short, 0)
            } else {
                // a second field never carries MMCO 5 (it would not pair)
                let (m, idx, _) = self.mark_current(sh, limits, Some(i))?;
                (m, idx)
            };
            // the marking may have dropped other entries; find the first field again (it waits for
            // output, so it is still there)
            let Some(e) = self.entries.iter_mut().find(|e| e.open) else {
                return invalid("first field of a field pair lost");
            };
            e.field_mark[parity] = mark;
            if mark == RefMark::Long {
                e.long_term_frame_idx = long_idx;
            }
            e.field_poc[parity] = poc;
            e.poc = e.field_poc[0].min(e.field_poc[1]);
            e.fields = 3;
            e.open = false;
            e.sync_mark();
            e.meta = paired_meta(e.meta.clone(), &meta);
            pair(&mut e.frame, field);
            self.bump_reorder(out);
            return Ok(());
        }
        self.close_open();
        // (a failed marking keeps the held field for the next picture or the flush)
        let (mark, long_idx, had5) = self.mark_current(sh, limits, None)?;
        self.release_held(out);
        if had5 {
            // All earlier pictures are output before the current one (C.4.4).
            self.flush(out);
        }
        let frame_num = if had5 { 0 } else { sh.frame_num };
        // A first field always needs a frame buffer of its own, the second field may join it.
        // With the DPB full, a non-reference field that precedes everything waiting is output
        // directly (C.4.5.2); it is held until its second field shows up to be output with it.
        while self.entries.len() >= self.capacity {
            if mark == RefMark::Unused {
                let min_poc = self.entries.iter().filter(|e| e.needed_for_output).map(|e| e.poc).min();
                if min_poc.is_none_or(|m| poc < m) {
                    self.held = Some(HeldField { frame: field, poc, meta, parity, frame_num, open: true });
                    return Ok(());
                }
            }
            if !self.bump(out) {
                break;
            }
        }
        let mut field_mark = [RefMark::Unused; 2];
        field_mark[parity] = mark;
        let mut e = DpbEntry::frame_entry(field, [poc; 2], frame_num, long_idx, mark, true, false, meta);
        e.field_mark = field_mark;
        e.fields = 1 << parity;
        e.field_pic = true;
        e.open = true;
        e.sync_mark();
        self.entries.push(e);
        self.bump_reorder(out);
        Ok(())
    }
}

impl<F: Clone> Default for Dpb<F> {
    fn default() -> Self {
        Self::new()
    }
}

/// Fields of the reference entries `frames` (in that order) with marking `m`, alternating in parity
/// starting with `parity`; once one parity runs out the remaining fields of the other follow in
/// order (8.2.4.2.5).
fn alternate_fields<'a, F>(frames: &[&'a DpbEntry<F>], parity: usize, m: RefMark) -> Vec<(&'a DpbEntry<F>, Option<usize>)> {
    let mut out = Vec::new();
    let mut next = [0usize; 2];
    let mut p = parity;
    loop {
        let find = |p: usize, from: usize| (from..frames.len()).find(|&i| frames[i].field_mark[p] == m);
        let pick = find(p, next[p]).map(|i| (p, i)).or_else(|| find(p ^ 1, next[p ^ 1]).map(|i| (p ^ 1, i)));
        let Some((q, i)) = pick else { break };
        out.push((frames[i], Some(q)));
        next[q] = i + 1;
        p = q ^ 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
                assert!(dpb.apply_mmcos(&[op], (5, 16), None, None, &mut None).is_ok());
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
        assert_eq!(updated, (1..32768).collect::<Vec<u32>>());
        let mut refs: Vec<u32> = dpb.entries.iter().filter(|e| e.mark == RefMark::Short).map(|e| e.frame_num).collect();
        refs.sort_unstable();
        assert_eq!(refs, vec![32764, 32765, 32766, 32767]);
        assert!(dpb.entries.iter().all(|e| e.frame == e.frame_num));
    }

    /// Existing short-term references are still evicted by a gap no longer than the window.
    #[test]
    fn short_frame_num_gap_keeps_sliding_window_semantics() {
        let mut dpb: Dpb<u32> = Dpb::new();
        let m = meta();
        for f in 0..2 {
            dpb.entries.push(DpbEntry::frame_entry(f, [0; 2], f, 0, RefMark::Short, false, false, m.clone()));
        }
        let mut make = |f: u32| (f, 0, m.clone());
        let mut upd = |_f: u32| {};
        dpb.fill_frame_num_gap(1, 5, 16, 4, &mut make, &mut upd);
        let mut refs: Vec<u32> = dpb.entries.iter().filter(|e| e.mark == RefMark::Short).map(|e| e.frame_num).collect();
        refs.sort_unstable();
        assert_eq!(refs, vec![1, 2, 3, 4]);
    }

    // --------------------------------------------------------------------------------------
    // Field pictures
    // --------------------------------------------------------------------------------------

    /// A slice header of a field picture.
    fn field_slice(frame_num: u32, bottom: bool, reference: bool, slice_type: SliceType) -> SliceHeader {
        let mut sh = reference_slice();
        sh.field_pic = true;
        sh.bottom_field = bottom;
        sh.frame_num = frame_num;
        sh.nal_ref_idc = u8::from(reference);
        sh.slice_type = slice_type;
        sh
    }

    /// Store a field whose picture is `id`; a second field merges as `first * 100 + second`.
    fn store_field(dpb: &mut Dpb<u32>, sh: &SliceHeader, id: u32, poc: i32, max_num_ref_frames: usize, out: &mut Vec<Output<u32>>) -> Result<()> {
        dpb.store_field(sh, id, sh.bottom_field as usize, poc, (16, max_num_ref_frames), meta(), out, |_| true, |f, g| *f = *f * 100 + g)
    }

    /// A stored frame (both fields) or field pair with the given markings.
    fn entry(id: u32, frame_num: u32, marks: [RefMark; 2], poc: [i32; 2]) -> DpbEntry<u32> {
        let mut e = DpbEntry::frame_entry(id, poc, frame_num, 0, RefMark::Unused, true, false, meta());
        e.field_mark = marks;
        e.sync_mark();
        e
    }

    const S: RefMark = RefMark::Short;
    const L: RefMark = RefMark::Long;
    const U: RefMark = RefMark::Unused;

    #[test]
    fn complementary_reference_fields_share_one_frame_buffer() {
        let mut dpb: Dpb<u32> = Dpb::new();
        dpb.max_reorder = 0;
        let mut out = Vec::new();
        store_field(&mut dpb, &field_slice(0, false, true, SliceType::I), 1, 4, 4, &mut out).unwrap();
        // the first field waits for the second one
        assert!(out.is_empty());
        assert!(dpb.entries[0].open);
        store_field(&mut dpb, &field_slice(0, true, true, SliceType::P), 2, 5, 4, &mut out).unwrap();
        assert_eq!(dpb.entries.len(), 1);
        let e = &dpb.entries[0];
        assert_eq!((e.frame, e.fields, e.field_mark, e.mark, e.field_poc, e.poc), (102, 3, [S, S], S, [4, 5], 4));
        assert_eq!(out.iter().map(|o| o.frame).collect::<Vec<_>>(), [102]);
    }

    #[test]
    fn bottom_field_first_pairs_too() {
        let mut dpb: Dpb<u32> = Dpb::new();
        let mut out = Vec::new();
        store_field(&mut dpb, &field_slice(3, true, false, SliceType::I), 1, 6, 4, &mut out).unwrap();
        store_field(&mut dpb, &field_slice(3, false, false, SliceType::I), 2, 7, 4, &mut out).unwrap();
        assert_eq!(dpb.entries.len(), 1);
        assert_eq!((dpb.entries[0].frame, dpb.entries[0].fields, dpb.entries[0].field_poc), (102, 3, [7, 6]));
    }

    #[test]
    fn fields_that_do_not_pair_take_frame_buffers_of_their_own() {
        // (first, second): reference status differs, same parity twice, frame_num differs, an IDR
        // second field, a second field with MMCO 5
        let cases: [(SliceHeader, SliceHeader); 5] = [
            (field_slice(1, false, true, SliceType::I), field_slice(1, true, false, SliceType::I)),
            (field_slice(1, false, true, SliceType::I), field_slice(1, false, true, SliceType::I)),
            (field_slice(1, false, true, SliceType::I), field_slice(2, true, true, SliceType::I)),
            (field_slice(1, false, true, SliceType::I), {
                let mut sh = field_slice(1, true, true, SliceType::I);
                sh.idr = true;
                sh
            }),
            (field_slice(1, false, true, SliceType::I), {
                let mut sh = field_slice(1, true, true, SliceType::I);
                sh.adaptive_ref_pic_marking = true;
                sh.mmcos = vec![Mmco {
                    op: 5,
                    difference_of_pic_nums_minus1: 0,
                    long_term_pic_num: 0,
                    long_term_frame_idx: 0,
                    max_long_term_frame_idx_plus1: 0,
                }];
                sh
            }),
        ];
        for (i, (a, b)) in cases.iter().enumerate() {
            let mut dpb: Dpb<u32> = Dpb::new();
            let mut out = Vec::new();
            store_field(&mut dpb, a, 1, 2, 4, &mut out).unwrap();
            store_field(&mut dpb, b, 2, 3, 4, &mut out).unwrap();
            dpb.flush(&mut out);
            let mut frames: Vec<u32> = out.iter().map(|o| o.frame).collect();
            frames.sort_unstable();
            assert_eq!(frames, [1, 2], "case {i}");
        }
    }

    #[test]
    fn a_field_of_another_size_does_not_join_the_first_field() {
        let mut dpb: Dpb<u32> = Dpb::new();
        let mut out = Vec::new();
        let (a, b) = (field_slice(0, false, true, SliceType::I), field_slice(0, true, true, SliceType::I));
        dpb.store_field(&a, 1, 0, 0, (16, 4), meta(), &mut out, |_| true, |f, g| *f = *f * 100 + g).unwrap();
        dpb.store_field(&b, 2, 1, 1, (16, 4), meta(), &mut out, |_| false, |f, g| *f = *f * 100 + g).unwrap();
        assert_eq!(dpb.entries.iter().map(|e| e.frame).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn a_lone_field_is_output_once_the_next_picture_shows_it_has_no_partner() {
        let mut dpb: Dpb<u32> = Dpb::new();
        dpb.max_reorder = 0;
        let mut out = Vec::new();
        store_field(&mut dpb, &field_slice(0, false, true, SliceType::I), 1, 0, 4, &mut out).unwrap();
        assert!(out.is_empty());
        let mut sh = reference_slice();
        sh.frame_num = 1;
        dpb.store_frame(&sh, 2, [2, 3], (16, 4), meta(), &mut out).unwrap();
        assert_eq!(out.iter().map(|o| o.frame).collect::<Vec<_>>(), [1, 2]);
    }

    /// With pic_order_cnt_type 0 a "non-existing" frame has no order count and is left out of B
    /// slice lists (8.2.4.2.3, frames, and 8.2.4.2.4, fields); P lists keep it (8.2.4.2.1).
    #[test]
    fn non_existing_frames_stay_out_of_b_lists_with_poc_type_0() {
        for (poc_type0, field) in [(true, false), (true, true), (false, false)] {
            let mut dpb: Dpb<u32> = Dpb::new();
            dpb.poc_type0 = poc_type0;
            dpb.entries.push(DpbEntry::frame_entry(1, [0, 0], 0, 0, RefMark::Short, false, false, meta()));
            dpb.entries.push(DpbEntry::frame_entry(2, [0, 0], 1, 0, RefMark::Short, false, true, meta()));
            dpb.entries.push(DpbEntry::frame_entry(3, [8, 8], 2, 0, RefMark::Short, false, false, meta()));
            let mut sh = if field { field_slice(3, false, false, SliceType::B) } else { reference_slice() };
            sh.slice_type = SliceType::B;
            sh.frame_num = 3;
            sh.num_ref_idx_active = [8, 8];
            let lists = dpb.build_field_ref_lists(&sh, 4, 16, |e, _| e.frame).unwrap();
            let has_gap = lists.iter().any(|l| l.contains(&2));
            assert_eq!(has_gap, !poc_type0, "poc_type0={poc_type0} field={field}");
            sh.slice_type = SliceType::P;
            let lists = dpb.build_field_ref_lists(&sh, 4, 16, |e, _| e.frame).unwrap();
            assert!(lists[0].contains(&2), "P list, field={field}");
        }
    }

    /// C.4.5.2: with the DPB full, a non-reference field pair preceding everything waiting is
    /// output directly instead of bumping the pictures after it. Two frame buffers hold reference
    /// frames with POC 0 and 8; a non-reference pair with POC 4 and 5 comes out between them, and
    /// so does a lone non-reference field.
    #[test]
    fn a_full_dpb_outputs_an_earlier_non_reference_field_pair_directly() {
        for lone in [false, true] {
            let mut dpb: Dpb<u32> = Dpb::new();
            dpb.capacity = 2;
            let mut out = Vec::new();
            let mut sh = reference_slice();
            sh.idr = true;
            dpb.store_frame(&sh, 1, [0, 0], (16, 2), meta(), &mut out).unwrap();
            let mut sh = reference_slice();
            sh.frame_num = 1;
            dpb.store_frame(&sh, 2, [8, 8], (16, 2), meta(), &mut out).unwrap();
            store_field(&mut dpb, &field_slice(2, false, false, SliceType::B), 3, 4, 2, &mut out).unwrap();
            if lone {
                let mut sh = reference_slice();
                sh.frame_num = 2;
                dpb.store_frame(&sh, 5, [12, 12], (16, 2), meta(), &mut out).unwrap();
            } else {
                store_field(&mut dpb, &field_slice(2, true, false, SliceType::B), 4, 5, 2, &mut out).unwrap();
            }
            dpb.flush(&mut out);
            let want: &[u32] = if lone { &[1, 3, 2, 5] } else { &[1, 304, 2] };
            assert_eq!(out.iter().map(|o| o.frame).collect::<Vec<_>>(), want, "lone={lone}");
        }
    }

    /// A field pair is draft when either field is, whichever way the draft mode switched between
    /// them: for a pair waiting in the DPB and for a held pair (C.4.5.2).
    #[test]
    fn a_field_pair_is_draft_when_either_field_is() {
        let with_draft = |d: bool| Arc::new(OutputMeta { draft: d, ..(*meta()).clone() });
        for held in [false, true] {
            for (d0, d1) in [(false, true), (true, false), (false, false)] {
                let mut dpb: Dpb<u32> = Dpb::new();
                // a full DPB holds the non-reference pair at POC 4 and 5 for direct output; with a
                // free frame buffer it waits in the DPB
                dpb.capacity = if held { 2 } else { 3 };
                let mut out = Vec::new();
                let mut sh = reference_slice();
                sh.idr = true;
                dpb.store_frame(&sh, 1, [0, 0], (16, 2), meta(), &mut out).unwrap();
                let mut sh = reference_slice();
                sh.frame_num = 1;
                let base = 4;
                dpb.store_frame(&sh, 2, [8, 8], (16, 2), meta(), &mut out).unwrap();
                let fields = [field_slice(2, false, false, SliceType::B), field_slice(2, true, false, SliceType::B)];
                let store = |dpb: &mut Dpb<u32>, out: &mut Vec<_>, sh: &SliceHeader, id, poc, d| {
                    dpb.store_field(sh, id, sh.bottom_field as usize, poc, (16, 2), with_draft(d), out, |_| true, |f, g| *f = *f * 100 + g).unwrap()
                };
                store(&mut dpb, &mut out, &fields[0], 3, base, d0);
                assert_eq!(dpb.held().is_some(), held);
                store(&mut dpb, &mut out, &fields[1], 4, base + 1, d1);
                dpb.flush(&mut out);
                let pair = out.iter().find(|o| o.frame == 304).unwrap();
                assert_eq!(pair.meta.draft, d0 || d1, "held={held} draft {d0} then {d1}");
            }
        }
    }

    /// A held field survives a picture whose reference marking fails: it is output by the next
    /// picture or the flush, not dropped.
    #[test]
    fn a_held_field_survives_a_failed_marking() {
        let mut dpb: Dpb<u32> = Dpb::new();
        dpb.capacity = 2;
        let mut out = Vec::new();
        let mut sh = reference_slice();
        sh.idr = true;
        dpb.store_frame(&sh, 1, [0, 0], (16, 2), meta(), &mut out).unwrap();
        let mut sh = reference_slice();
        sh.frame_num = 1;
        dpb.store_frame(&sh, 2, [8, 8], (16, 2), meta(), &mut out).unwrap();
        store_field(&mut dpb, &field_slice(2, false, false, SliceType::B), 3, 4, 2, &mut out).unwrap();
        assert_eq!(dpb.held(), Some(&3));
        // MMCO 6 without long-term frame indices fails
        let mut sh = reference_slice();
        sh.frame_num = 2;
        sh.adaptive_ref_pic_marking = true;
        sh.mmcos =
            vec![Mmco { op: 6, difference_of_pic_nums_minus1: 0, long_term_pic_num: 0, long_term_frame_idx: 0, max_long_term_frame_idx_plus1: 0 }];
        assert!(dpb.store_frame(&sh, 4, [12, 12], (16, 2), meta(), &mut out).is_err());
        // nothing was output by the failed picture: the field is still held
        assert_eq!(out.iter().map(|o| o.frame).collect::<Vec<_>>(), [1]);
        assert_eq!(dpb.held(), Some(&3));
        dpb.flush(&mut out);
        assert_eq!(out.iter().map(|o| o.frame).collect::<Vec<_>>(), [1, 3, 2]);
    }

    /// 8.2.5.3: the second field of a pair whose first field is short-term does not run the
    /// sliding window, which would drop the first field of its own frame from a full DPB.
    #[test]
    fn second_field_of_a_short_term_pair_skips_the_sliding_window() {
        let mut dpb: Dpb<u32> = Dpb::new();
        let mut out = Vec::new();
        store_field(&mut dpb, &field_slice(0, false, true, SliceType::I), 1, 0, 1, &mut out).unwrap();
        store_field(&mut dpb, &field_slice(0, true, true, SliceType::P), 2, 1, 1, &mut out).unwrap();
        assert_eq!(dpb.entries[0].field_mark, [S, S]);
    }

    /// 8.2.5.4.6: a second field marked long-term joins a frame that is counted already, so it
    /// fits a DPB that the first field fills.
    #[test]
    fn long_term_second_field_fits_a_full_dpb() {
        let mut dpb: Dpb<u32> = Dpb::new();
        let mut out = Vec::new();
        let mut first = field_slice(0, false, true, SliceType::I);
        first.idr = true;
        first.long_term_reference = true;
        store_field(&mut dpb, &first, 1, 0, 1, &mut out).unwrap();
        let mut second = field_slice(0, true, true, SliceType::P);
        second.adaptive_ref_pic_marking = true;
        second.mmcos =
            vec![Mmco { op: 6, difference_of_pic_nums_minus1: 0, long_term_pic_num: 0, long_term_frame_idx: 0, max_long_term_frame_idx_plus1: 0 }];
        store_field(&mut dpb, &second, 2, 1, 1, &mut out).unwrap();
        let e = &dpb.entries[0];
        assert_eq!((e.frame, e.field_mark, e.mark, e.long_term_frame_idx), (102, [L, L], L, 0));
    }

    /// 8.2.5.3: an entry with a short-term and a long-term field counts in numShortTerm and in
    /// numLongTerm, and the window then drops both its fields.
    #[test]
    fn sliding_window_counts_a_mixed_entry_twice() {
        let mut dpb: Dpb<u32> = Dpb::new();
        dpb.entries.push(entry(1, 0, [L, S], [0, 1]));
        dpb.sliding_window(1, 16, 2);
        assert_eq!(dpb.entries.iter().map(|e| e.field_mark).collect::<Vec<_>>(), [[U, U]]);
    }

    #[test]
    fn frames_and_fields_are_stored_through_their_own_calls() {
        let mut dpb: Dpb<u32> = Dpb::new();
        let mut out = Vec::new();
        assert!(dpb.store_picture(&field_slice(0, false, true, SliceType::I), 1, 0, (16, 4), meta(), &mut out).is_err());
        assert!(dpb.store_field(&reference_slice(), 1, 0, 0, (16, 4), meta(), &mut out, |_| true, |_, _| {}).is_err());
    }

    /// Lists of (picture, parity) for a slice of a field picture.
    fn field_lists(dpb: &Dpb<u32>, sh: &SliceHeader, poc: i32) -> [Vec<(u32, Option<usize>)>; 2] {
        dpb.build_field_ref_lists(sh, poc, 16, |e, p| (e.frame, p)).unwrap()
    }

    /// 8.2.4.2.2 and 8.2.4.2.5: frames by descending FrameNumWrap, fields alternating in parity
    /// from the current parity, skipping missing fields; when one parity runs out the other's
    /// remaining fields follow in order.
    #[test]
    fn p_field_lists_alternate_parity() {
        let mut dpb: Dpb<u32> = Dpb::new();
        dpb.entries.push(entry(1, 1, [S, S], [0, 1]));
        dpb.entries.push(entry(2, 2, [S, U], [2, 3]));
        dpb.entries.push(entry(3, 3, [S, S], [4, 5]));
        dpb.entries.push(entry(4, 0, [L, L], [6, 7]));
        let mut sh = field_slice(4, true, true, SliceType::P);
        sh.num_ref_idx_active = [7, 0];
        let [l0, _] = field_lists(&dpb, &sh, 8);
        let (t, b) = (Some(0), Some(1));
        assert_eq!(l0, [(3, b), (3, t), (1, b), (2, t), (1, t), (4, b), (4, t)]);
    }

    /// 8.2.4.2.4: frames before the current field (PicOrderCnt less than or equal) in descending
    /// order lead RefPicList0, the others lead RefPicList1.
    #[test]
    fn b_field_lists_order_by_picture_order() {
        let mut dpb: Dpb<u32> = Dpb::new();
        dpb.entries.push(entry(1, 1, [S, S], [0, 1]));
        dpb.entries.push(entry(2, 2, [S, S], [8, 9]));
        // the first field of the current frame, with the current field's order count: "less than or
        // equal" puts it before the current field
        let mut first = entry(3, 3, [S, U], [5, 5]);
        first.fields = 1;
        first.open = true;
        dpb.entries.push(first);
        let mut sh = field_slice(3, true, false, SliceType::B);
        sh.num_ref_idx_active = [5, 5];
        let [l0, l1] = field_lists(&dpb, &sh, 5);
        let (t, b) = (Some(0), Some(1));
        // frames 3, 1, 2; the current field is a bottom field and frame 3 has no bottom field yet
        assert_eq!(l0, [(1, b), (3, t), (2, b), (1, t), (2, t)]);
        // frames 2, 3, 1
        assert_eq!(l1, [(2, b), (2, t), (1, b), (3, t), (1, t)]);
    }

    /// 8.2.4.3.1 with field picture numbers: CurrPicNum = 2 * frame_num + 1, a field of the same
    /// parity has PicNum 2 * FrameNumWrap + 1, the other 2 * FrameNumWrap.
    #[test]
    fn field_list_modification_uses_field_picture_numbers() {
        use crate::h264::decode::slice::RefPicListMod;
        let mut dpb: Dpb<u32> = Dpb::new();
        dpb.entries.push(entry(1, 3, [S, S], [0, 1]));
        let mut sh = field_slice(4, false, true, SliceType::P);
        sh.num_ref_idx_active = [2, 0];
        // CurrPicNum 9; 9 - 3 = 6 names the bottom field of frame_num 3
        sh.ref_pic_list_mod[0].push(RefPicListMod { idc: 0, value: 2 });
        let [l0, _] = field_lists(&dpb, &sh, 2);
        assert_eq!(l0, [(1, Some(1)), (1, Some(0))]);
        // 9 - 2 = 7 names the top field
        sh.ref_pic_list_mod[0][0].value = 1;
        let [l0, _] = field_lists(&dpb, &sh, 2);
        assert_eq!(l0, [(1, Some(0)), (1, Some(1))]);
    }

    /// 8.2.5.4.1 to 8.2.5.4.3 in a field picture act on one field and leave the other one.
    #[test]
    fn mmcos_in_a_field_picture_mark_single_fields() {
        let mmco = |op, difference_of_pic_nums_minus1, long_term_pic_num, long_term_frame_idx| Mmco {
            op,
            difference_of_pic_nums_minus1,
            long_term_pic_num,
            long_term_frame_idx,
            max_long_term_frame_idx_plus1: 0,
        };
        // current: top field of frame_num 4, CurrPicNum 9; stored frame_num 3 (PicNum 7 top, 6 bottom)
        let run = |marks: [RefMark; 2], op: Mmco| {
            let mut dpb: Dpb<u32> = Dpb::new();
            dpb.max_long_term_frame_idx = Some(3);
            dpb.entries.push(entry(1, 3, marks, [0, 1]));
            dpb.apply_mmcos(&[op], (4, 16), Some(0), None, &mut None).unwrap();
            dpb.entries.first().map(|e| (e.field_mark, e.mark))
        };
        // MMCO 1, picNumX 6: the bottom field
        assert_eq!(run([S, S], mmco(1, 2, 0, 0)), Some(([S, U], U)));
        // MMCO 2, LongTermPicNum 2 * 0 + 1 = 1: the top field
        assert_eq!(run([L, L], mmco(2, 0, 1, 0)), Some(([U, L], U)));
        // MMCO 3, picNumX 7: the top field becomes long-term, the pair is not long-term yet
        assert_eq!(run([S, S], mmco(3, 1, 0, 2)), Some(([L, S], U)));
        // ... and once both fields are long-term, so is the pair
        assert_eq!(run([S, L], mmco(3, 1, 0, 0)), Some(([L, L], L)));
    }
}
