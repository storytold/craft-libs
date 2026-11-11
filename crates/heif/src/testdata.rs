//! Synthetic HEIF files for tests (feature `testdata`): lossless, so every decoded sample is known.
//!
//! Each coded picture is one 32 × 32 coding tree unit stored as PCM samples (H.265 7.3.8.7):
//! the bitstream carries the YCbCr values verbatim, so the decoder's HEVC stage reproduces them
//! exactly and a test checks everything after it (grid composition, colour, transforms,
//! metadata) against values it chose. The few CABAC bins around the PCM data are written with a
//! minimal arithmetic encoder (9.3.4.4). Larger images are grids of such pictures, which is how
//! iPhones store photos too (with 512 × 512 tiles).
//!
//! Not a general HEIF or HEVC writer: 4:2:0 or monochrome, 8 or 10 bits, a CTU per picture.

/// Side of every coded picture.
pub const TILE: u32 = 32;

/// YCbCr samples at a luma position (chroma is taken at even positions, 4:2:0).
pub type Sampler<'a> = &'a dyn Fn(u32, u32) -> [u16; 3];

/// A property of the primary image.
#[derive(Debug, Clone, PartialEq)]
pub enum Prop {
    /// `colr` of type `prof`.
    Icc(Vec<u8>),
    /// `colr` of type `nclx`: primaries, transfer, matrix, full range.
    Nclx(u16, u16, u16, bool),
    /// `irot`: quarter turns counter-clockwise.
    Irot(u8),
    /// `imir` with this axis.
    Imir(u8),
    /// `clap`: width, height, horizontal and vertical offset (whole pixels).
    Clap(u32, u32, i32, i32),
}

/// What [`build`] writes.
pub struct Spec<'a> {
    /// Displayed (grid output) size, before transforms.
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub sample: Sampler<'a>,
    /// Properties of the primary image, in association order.
    pub props: Vec<Prop>,
    /// EXIF as a TIFF structure (written with HEIF's 4-byte offset header).
    pub exif: Option<Vec<u8>>,
    /// An alpha auxiliary image (monochrome, full range).
    pub alpha: Option<&'a dyn Fn(u32, u32) -> u16>,
    /// A thumbnail item of half the size with these samples (in its own coordinates).
    pub thumbnail: Option<Sampler<'a>>,
    /// Auxiliary images of these types the decoder must ignore (HDR gain map, depth…).
    pub ignored_aux: Vec<&'static str>,
}

impl<'a> Spec<'a> {
    pub fn new(width: u32, height: u32, sample: Sampler<'a>) -> Spec<'a> {
        Spec { width, height, bit_depth: 8, sample, props: Vec::new(), exif: None, alpha: None, thumbnail: None, ignored_aux: Vec::new() }
    }
}

#[derive(Default)]
struct Bits {
    bytes: Vec<u8>,
    n: usize,
}

impl Bits {
    fn put(&mut self, v: u32, len: u32) {
        for i in (0..len).rev() {
            if self.n.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if (v >> i) & 1 == 1
                && let Some(b) = self.bytes.last_mut()
            {
                *b |= 0x80 >> (self.n % 8);
            }
            self.n += 1;
        }
    }

    fn ue(&mut self, v: u32) {
        let x = u64::from(v) + 1;
        let len = 64 - x.leading_zeros();
        self.put(0, len - 1);
        self.put((x & 0xffff_ffff) as u32, len);
    }

    fn align_zero(&mut self) {
        while !self.n.is_multiple_of(8) {
            self.put(0, 1);
        }
    }

    fn trailing(&mut self) {
        self.put(1, 1);
        self.align_zero();
    }
}

/// The CABAC arithmetic encoder of H.265 9.3.4.4, for the bins these pictures need.
struct Cabac {
    low: u32,
    range: u32,
    first: bool,
    outstanding: u32,
}

impl Cabac {
    fn new() -> Cabac {
        Cabac { low: 0, range: 510, first: true, outstanding: 0 }
    }

    fn put_bit(&mut self, out: &mut Bits, b: u32) {
        if self.first {
            self.first = false;
        } else {
            out.put(b, 1);
        }
        while self.outstanding > 0 {
            out.put(1 - b, 1);
            self.outstanding -= 1;
        }
    }

    fn renorm(&mut self, out: &mut Bits) {
        while self.range < 256 {
            if self.low < 256 {
                self.put_bit(out, 0);
            } else if self.low >= 512 {
                self.low -= 512;
                self.put_bit(out, 1);
            } else {
                self.low -= 256;
                self.outstanding += 1;
            }
            self.range <<= 1;
            self.low <<= 1;
        }
    }

    /// `part_mode` = PART_2Nx2N: the most probable symbol of a fresh context (initValue 184 at
    /// QP 26 gives pStateIdx 0, valMps 1), so the LPS range is `rangeTabLps[0][3]` = 240.
    fn part_mode_2nx2n(&mut self, out: &mut Bits) {
        let lps = [128, 176, 208, 240][((self.range >> 6) & 3) as usize];
        self.range -= lps;
        self.renorm(out);
    }

    /// A terminating bin equal to 1 (`pcm_flag`, `end_of_slice_segment_flag`), with the flush.
    fn terminate_one(&mut self, out: &mut Bits) {
        self.range -= 2;
        self.low += self.range;
        self.range = 2;
        self.renorm(out);
        self.put_bit(out, (self.low >> 9) & 1);
        out.put(((self.low >> 7) & 3) | 1, 2);
    }
}

fn nal(kind: u8, rbsp: &[u8]) -> Vec<u8> {
    let mut out = vec![kind << 1, 1];
    let mut zeros = 0;
    for &b in rbsp {
        if zeros >= 2 && b <= 3 {
            out.push(3);
            zeros = 0;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        out.push(b);
    }
    out
}

fn profile_idc(bit_depth: u8, mono: bool) -> u32 {
    match (bit_depth, mono) {
        (_, true) => 4, // range extensions
        (8, false) => 1,
        _ => 2,
    }
}

fn ptl(w: &mut Bits, idc: u32) {
    w.put(0, 2);
    w.put(0, 1);
    w.put(idc, 5);
    w.put(1 << (31 - idc), 32); // compatible with itself
    w.put(0b1001, 4); // progressive, frame only
    w.put(0, 22);
    w.put(0, 21);
    w.put(0, 1);
    w.put(93, 8); // level 3.1
}

fn parameter_sets(bit_depth: u8, mono: bool) -> [Vec<u8>; 3] {
    let idc = profile_idc(bit_depth, mono);
    let bd = u32::from(bit_depth);
    let mut v = Bits::default();
    v.put(0, 4);
    v.put(3, 2);
    v.put(0, 6);
    v.put(0, 3);
    v.put(1, 1);
    v.put(0xffff, 16);
    ptl(&mut v, idc);
    v.put(0, 1); // vps_sub_layer_ordering_info_present_flag
    v.ue(0);
    v.ue(0);
    v.ue(0);
    v.put(0, 6); // vps_max_layer_id
    v.ue(0); // vps_num_layer_sets_minus1
    v.put(0, 1); // vps_timing_info_present_flag
    v.put(0, 1); // vps_extension_flag
    v.trailing();

    let mut s = Bits::default();
    s.put(0, 4);
    s.put(0, 3);
    s.put(1, 1);
    ptl(&mut s, idc);
    s.ue(0);
    s.ue(if mono { 0 } else { 1 });
    s.ue(TILE);
    s.ue(TILE);
    s.put(0, 1); // conformance_window_flag
    s.ue(bd - 8);
    s.ue(bd - 8);
    s.ue(4);
    s.put(1, 1);
    s.ue(0);
    s.ue(0);
    s.ue(0);
    s.ue(2); // log2_min_luma_coding_block_size_minus3: 32
    s.ue(0); // log2_diff_max_min_luma_coding_block_size: CTB 32
    s.ue(0); // log2_min_luma_transform_block_size_minus2: 4
    s.ue(3); // log2_diff_max_min_luma_transform_block_size: 32
    s.ue(0);
    s.ue(0);
    s.put(0, 1); // scaling_list_enabled_flag
    s.put(0, 1); // amp_enabled_flag
    s.put(0, 1); // sample_adaptive_offset_enabled_flag
    s.put(1, 1); // pcm_enabled_flag
    s.put(bd - 1, 4); // pcm_sample_bit_depth_luma_minus1
    s.put(bd - 1, 4); // pcm_sample_bit_depth_chroma_minus1
    s.ue(2); // log2_min_pcm_luma_coding_block_size_minus3: 32
    s.ue(0); // log2_diff_max_min_pcm_luma_coding_block_size
    s.put(1, 1); // pcm_loop_filter_disabled_flag
    s.ue(0); // num_short_term_ref_pic_sets
    s.put(0, 1); // long_term_ref_pics_present_flag
    s.put(0, 1); // sps_temporal_mvp_enabled_flag
    s.put(0, 1); // strong_intra_smoothing_enabled_flag
    s.put(0, 1); // vui_parameters_present_flag
    s.put(0, 1); // sps_extension_present_flag
    s.trailing();

    let mut p = Bits::default();
    p.ue(0);
    p.ue(0);
    p.put(0, 6); // dependent slices, output flag, 3 extra header bits, sign hiding
    p.put(0, 1); // cabac_init_present_flag
    p.ue(0);
    p.ue(0);
    p.ue(0); // init_qp_minus26 (se(0) is the same bits)
    p.put(0, 3); // constrained intra, transform skip, cu_qp_delta
    p.ue(0);
    p.ue(0);
    p.put(0, 1);
    p.put(0, 2);
    p.put(0, 1); // transquant_bypass_enabled_flag
    p.put(0, 1); // tiles_enabled_flag
    p.put(0, 1); // entropy_coding_sync_enabled_flag
    p.put(0, 1); // pps_loop_filter_across_slices_enabled_flag
    p.put(1, 1); // deblocking_filter_control_present_flag
    p.put(0, 1); // deblocking_filter_override_enabled_flag
    p.put(1, 1); // pps_deblocking_filter_disabled_flag
    p.put(0, 1);
    p.put(0, 1);
    p.ue(0);
    p.put(0, 1);
    p.put(0, 1);
    p.trailing();
    [nal(32, &v.bytes), nal(33, &s.bytes), nal(34, &p.bytes)]
}

/// One coded 32 × 32 picture: the `hvcC` payload and the item data (length-prefixed NAL).
struct Coded {
    hvcc: Vec<u8>,
    data: Vec<u8>,
}

fn picture(bit_depth: u8, mono: bool, sample: &dyn Fn(u32, u32) -> [u16; 3]) -> Coded {
    let bd = u32::from(bit_depth);
    let mut w = Bits::default();
    w.put(1, 1); // first_slice_segment_in_pic_flag
    w.put(0, 1); // no_output_of_prior_pics_flag
    w.ue(0);
    w.ue(2); // I slice
    w.ue(0); // slice_qp_delta, se(0)
    w.trailing(); // byte_alignment()
    let mut c = Cabac::new();
    c.part_mode_2nx2n(&mut w);
    c.terminate_one(&mut w); // pcm_flag
    w.align_zero(); // pcm_alignment_zero_bit
    for y in 0..TILE {
        for x in 0..TILE {
            w.put(u32::from(sample(x, y)[0]), bd);
        }
    }
    if !mono {
        for k in 1..3 {
            for y in 0..TILE / 2 {
                for x in 0..TILE / 2 {
                    w.put(u32::from(sample(2 * x, 2 * y)[k]), bd);
                }
            }
        }
    }
    let mut c = Cabac::new();
    c.terminate_one(&mut w); // end_of_slice_segment_flag (its flush ends in the stop bit)
    w.align_zero();
    let slice = nal(19, &w.bytes);

    let sets = parameter_sets(bit_depth, mono);
    let mut h = vec![1u8, profile_idc(bit_depth, mono) as u8];
    h.extend_from_slice(&(1u32 << (31 - profile_idc(bit_depth, mono))).to_be_bytes());
    h.extend_from_slice(&[
        0x90,
        0,
        0,
        0,
        0,
        0,
        93,
        0xf0,
        0,
        0xfc,
        0xfc | u8::from(!mono),
        0xf8 | (bit_depth - 8),
        0xf8 | (bit_depth - 8),
        0,
        0,
        0x0f,
        3,
    ]);
    for (kind, set) in [32u8, 33, 34].into_iter().zip(&sets) {
        h.push(0x80 | kind);
        h.extend_from_slice(&1u16.to_be_bytes());
        h.extend_from_slice(&(set.len() as u16).to_be_bytes());
        h.extend_from_slice(set);
    }
    let mut data = (slice.len() as u32).to_be_bytes().to_vec();
    data.extend_from_slice(&slice);
    Coded { hvcc: h, data }
}

fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
    v.extend_from_slice(kind);
    v.extend_from_slice(payload);
    v
}

fn full_box(kind: &[u8; 4], version: u8, payload: &[u8]) -> Vec<u8> {
    let mut p = vec![version, 0, 0, 0];
    p.extend_from_slice(payload);
    bx(kind, &p)
}

struct Item {
    id: u16,
    kind: &'static [u8; 4],
    data: Vec<u8>,
    /// Indices (1-based) into the property list, with the essential flag.
    props: Vec<(u8, bool)>,
    /// Not an image of its own (a grid tile).
    hidden: bool,
}

#[derive(Default)]
struct File {
    items: Vec<Item>,
    props: Vec<Vec<u8>>,
    refs: Vec<(&'static [u8; 4], u16, Vec<u16>)>,
}

impl File {
    fn prop(&mut self, b: Vec<u8>) -> u8 {
        self.props.push(b);
        self.props.len() as u8
    }

    fn add(&mut self, kind: &'static [u8; 4], data: Vec<u8>, props: Vec<(u8, bool)>) -> u16 {
        let id = self.items.len() as u16 + 1;
        self.items.push(Item { id, kind, data, props, hidden: false });
        id
    }

    /// A coded image of `w` × `h` (a grid when larger than a tile); returns its item id.
    fn image(&mut self, w: u32, h: u32, bit_depth: u8, mono: bool, sample: &dyn Fn(u32, u32) -> [u16; 3]) -> u16 {
        let (cols, rows) = (w.div_ceil(TILE), h.div_ceil(TILE));
        let ispe = |w: u32, h: u32| {
            let mut p = w.to_be_bytes().to_vec();
            p.extend_from_slice(&h.to_be_bytes());
            full_box(b"ispe", 0, &p)
        };
        let mut hvcc_prop = None;
        let tile_ispe = self.prop(ispe(TILE, TILE));
        let mut tiles = Vec::new();
        for r in 0..rows {
            for c in 0..cols {
                let coded = picture(bit_depth, mono, &|x, y| sample((c * TILE + x).min(w - 1), (r * TILE + y).min(h - 1)));
                let hv = *hvcc_prop.get_or_insert_with(|| self.prop(bx(b"hvcC", &coded.hvcc)));
                tiles.push(self.add(b"hvc1", coded.data, vec![(hv, true), (tile_ispe, false)]));
            }
        }
        if let ([only], true) = (tiles.as_slice(), w == TILE && h == TILE) {
            return *only;
        }
        for it in self.items.iter_mut().filter(|i| tiles.contains(&i.id)) {
            it.hidden = true;
        }
        let mut g = vec![0u8, 0, (rows - 1) as u8, (cols - 1) as u8];
        g.extend_from_slice(&(w as u16).to_be_bytes());
        g.extend_from_slice(&(h as u16).to_be_bytes());
        let grid_ispe = self.prop(ispe(w, h));
        let id = self.add(b"grid", g, vec![(grid_ispe, false)]);
        self.refs.push((b"dimg", id, tiles));
        id
    }

    fn write(self, primary: u16) -> Vec<u8> {
        let ftyp = bx(b"ftyp", b"heic\0\0\0\0mif1heic");
        let mut hdlr = vec![0u8; 4];
        hdlr.extend_from_slice(b"pict");
        hdlr.extend_from_slice(&[0; 13]);
        let hdlr = full_box(b"hdlr", 0, &hdlr);
        let pitm = full_box(b"pitm", 0, &primary.to_be_bytes());
        let mut iinf = (self.items.len() as u16).to_be_bytes().to_vec();
        for it in &self.items {
            let mut e = vec![2, 0, 0, u8::from(it.hidden)]; // version 2, flags
            e.extend_from_slice(&it.id.to_be_bytes());
            e.extend_from_slice(&[0, 0]);
            e.extend_from_slice(it.kind);
            e.push(0);
            iinf.extend_from_slice(&bx(b"infe", &e));
        }
        let iinf = full_box(b"iinf", 0, &iinf);
        let mut iref = Vec::new();
        for (kind, from, to) in &self.refs {
            let mut r = from.to_be_bytes().to_vec();
            r.extend_from_slice(&(to.len() as u16).to_be_bytes());
            for t in to {
                r.extend_from_slice(&t.to_be_bytes());
            }
            iref.extend_from_slice(&bx(kind, &r));
        }
        let iref = full_box(b"iref", 0, &iref);
        let ipco = bx(b"ipco", &self.props.concat());
        let mut ipma = (self.items.iter().filter(|i| !i.props.is_empty()).count() as u32).to_be_bytes().to_vec();
        for it in self.items.iter().filter(|i| !i.props.is_empty()) {
            ipma.extend_from_slice(&it.id.to_be_bytes());
            ipma.push(it.props.len() as u8);
            ipma.extend(it.props.iter().map(|&(i, essential)| i | if essential { 0x80 } else { 0 }));
        }
        let iprp = bx(b"iprp", &[ipco, full_box(b"ipma", 0, &ipma)].concat());
        let iloc = |base: u32| {
            let mut l = vec![0x44, 0x00];
            l.extend_from_slice(&(self.items.len() as u16).to_be_bytes());
            let mut offset = base;
            for it in &self.items {
                l.extend_from_slice(&it.id.to_be_bytes());
                l.extend_from_slice(&[0, 0, 0, 0, 0, 1]); // construction method 0, data ref 0, one extent
                l.extend_from_slice(&offset.to_be_bytes());
                l.extend_from_slice(&(it.data.len() as u32).to_be_bytes());
                offset += it.data.len() as u32;
            }
            full_box(b"iloc", 1, &l)
        };
        let meta = |base| full_box(b"meta", 0, &[hdlr.clone(), pitm.clone(), iloc(base), iinf.clone(), iref.clone(), iprp.clone()].concat());
        let start = (ftyp.len() + meta(0).len() + 8) as u32;
        let mdat = bx(b"mdat", &self.items.iter().flat_map(|i| i.data.iter().copied()).collect::<Vec<u8>>());
        [ftyp, meta(start), mdat].concat()
    }
}

/// Writes the HEIF file `spec` describes.
pub fn build(spec: &Spec<'_>) -> Vec<u8> {
    let mut f = File::default();
    let primary = f.image(spec.width, spec.height, spec.bit_depth, false, spec.sample);
    let mut assoc = Vec::new();
    for p in &spec.props {
        let b = match p {
            Prop::Icc(icc) => bx(b"colr", &[b"prof".as_slice(), icc].concat()),
            Prop::Nclx(pr, tc, mc, full) => {
                let mut c = b"nclx".to_vec();
                for v in [pr, tc, mc] {
                    c.extend_from_slice(&v.to_be_bytes());
                }
                c.push(if *full { 0x80 } else { 0 });
                bx(b"colr", &c)
            }
            Prop::Irot(a) => bx(b"irot", &[a & 3]),
            Prop::Imir(a) => bx(b"imir", &[a & 1]),
            Prop::Clap(w, h, ho, vo) => {
                let mut c = Vec::new();
                for v in [*w as i32, 1, *h as i32, 1, *ho, 1, *vo, 1] {
                    c.extend_from_slice(&v.to_be_bytes());
                }
                bx(b"clap", &c)
            }
        };
        assoc.push((f.prop(b), true, matches!(p, Prop::Clap(..))));
    }
    if let Some(it) = f.items.iter_mut().find(|i| i.id == primary) {
        it.props.extend(assoc.iter().map(|&(i, e, _)| (i, e)));
    }
    if let Some(exif) = &spec.exif {
        let id = f.add(b"Exif", [&[0u8, 0, 0, 0][..], exif].concat(), Vec::new());
        f.refs.push((b"cdsc", id, vec![primary]));
    }
    let bd = spec.bit_depth;
    let aux = |f: &mut File, urn: &str, sample: &dyn Fn(u32, u32) -> [u16; 3]| {
        let id = f.image(spec.width, spec.height, bd, true, sample);
        let auxc = f.prop(full_box(b"auxC", 0, &[urn.as_bytes(), &[0]].concat()));
        if let Some(it) = f.items.iter_mut().find(|i| i.id == id) {
            it.props.push((auxc, true));
        }
        f.refs.push((b"auxl", id, vec![primary]));
    };
    if let Some(alpha) = spec.alpha {
        aux(&mut f, "urn:mpeg:hevc:2015:auxid:1", &|x, y| [alpha(x, y), 0, 0]);
    }
    for urn in &spec.ignored_aux {
        aux(&mut f, urn, &|x, y| [((x + y) % 256) as u16, 0, 0]);
    }
    if let Some(sample) = spec.thumbnail {
        // Half the size, with the primary image's colour and orientation (not its crop).
        let id = f.image(spec.width / 2, spec.height / 2, spec.bit_depth, false, sample);
        if let Some(it) = f.items.iter_mut().find(|i| i.id == id) {
            it.props.extend(assoc.iter().filter(|a| !a.2).map(|&(i, e, _)| (i, e)));
        }
        f.refs.push((b"thmb", id, vec![primary]));
    }
    f.write(primary)
}
