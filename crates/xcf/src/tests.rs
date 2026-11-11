//! Synthetic XCF files (written here, from the specification) for the reader: structure,
//! every compression, precisions, groups, masks, channels, limits and malformed input.

use super::tiles::rle_decode;
use super::*;

/// A small XCF writer for tests: version 11+ layout (64-bit pointers) by default.
pub(super) struct W {
    pub out: Vec<u8>,
    pub wide: bool,
}

impl W {
    pub fn new(version: u32) -> Self {
        let mut w = W { out: Vec::new(), wide: version >= 11 };
        w.out.extend_from_slice(b"gimp xcf ");
        if version == 0 {
            w.out.extend_from_slice(b"file\0");
        } else {
            w.out.extend_from_slice(format!("v{version:03}\0").as_bytes());
        }
        w
    }
    pub fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_be_bytes());
    }
    pub fn ptr(&mut self, v: usize) {
        if self.wide {
            self.out.extend_from_slice(&(v as u64).to_be_bytes());
        } else {
            self.u32(v as u32);
        }
    }
    pub fn string(&mut self, s: &str) {
        if s.is_empty() {
            self.u32(0);
        } else {
            self.u32(s.len() as u32 + 1);
            self.out.extend_from_slice(s.as_bytes());
            self.out.push(0);
        }
    }
    pub fn prop(&mut self, kind: u32, payload: &[u8]) {
        self.u32(kind);
        self.u32(payload.len() as u32);
        self.out.extend_from_slice(payload);
    }
    pub fn prop_u32(&mut self, kind: u32, v: u32) {
        self.prop(kind, &v.to_be_bytes());
    }
    pub fn end(&mut self) {
        self.u32(0);
        self.u32(0);
    }
    /// Reserves a pointer slot, returning where to patch it.
    pub fn slot(&mut self) -> usize {
        let at = self.out.len();
        self.ptr(0);
        at
    }
    pub fn patch(&mut self, slot: usize, target: usize) {
        if self.wide {
            self.out[slot..slot + 8].copy_from_slice(&(target as u64).to_be_bytes());
        } else {
            self.out[slot..slot + 4].copy_from_slice(&(target as u32).to_be_bytes());
        }
    }
}

/// RLE-encodes one tile of `bpp`-byte pixels, plane by plane, with runs and literals.
pub(super) fn rle(pixels: &[u8], bpp: usize) -> Vec<u8> {
    let n = pixels.len() / bpp;
    let mut out = Vec::new();
    for b in 0..bpp {
        let plane: Vec<u8> = (0..n).map(|i| pixels[i * bpp + b]).collect();
        let mut i = 0;
        while i < n {
            let v = plane[i];
            let mut run = 1;
            while i + run < n && plane[i + run] == v && run < 65535 {
                run += 1;
            }
            if run >= 2 {
                if run <= 127 {
                    out.push((run - 1) as u8);
                } else {
                    out.push(127);
                    out.push((run / 256) as u8);
                    out.push((run % 256) as u8);
                }
                out.push(v);
                i += run;
            } else {
                let start = i;
                while i < n && (i + 1 >= n || plane[i + 1] != plane[i]) && i - start < 127 {
                    i += 1;
                }
                let len = i - start;
                out.push((256 - len) as u8);
                out.extend_from_slice(&plane[start..i]);
            }
        }
    }
    out
}

/// Writes a hierarchy for `w`×`h` pixels of `bpp` bytes (`data` row-major), returning its offset.
pub(super) fn hierarchy(w: &mut W, width: u32, height: u32, bpp: usize, data: &[u8], compression: Compression) -> usize {
    let at = w.out.len();
    w.u32(width);
    w.u32(height);
    w.u32(bpp as u32);
    let level_slot = w.slot();
    w.ptr(0);
    let level = w.out.len();
    w.patch(level_slot, level);
    w.u32(width);
    w.u32(height);
    let across = width.div_ceil(TILE) as usize;
    let down = height.div_ceil(TILE) as usize;
    let slots: Vec<usize> = (0..across * down).map(|_| w.slot()).collect();
    w.ptr(0);
    for (i, slot) in slots.into_iter().enumerate() {
        let (tx, ty) = ((i % across) as u32, (i / across) as u32);
        let (tw, th) = ((width - tx * TILE).min(TILE), (height - ty * TILE).min(TILE));
        let mut tile = Vec::new();
        for py in 0..th {
            for px in 0..tw {
                let src = ((ty * TILE + py) as usize * width as usize + (tx * TILE + px) as usize) * bpp;
                tile.extend_from_slice(&data[src..src + bpp]);
            }
        }
        let encoded = match compression {
            Compression::None => tile,
            Compression::Rle => rle(&tile, bpp),
            Compression::Zlib => {
                use std::io::Write;
                let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(&tile).unwrap();
                e.finish().unwrap()
            }
        };
        let here = w.out.len();
        w.patch(slot, here);
        w.out.extend_from_slice(&encoded);
    }
    at
}

/// A layer to write.
pub(super) struct TestLayer {
    pub name: &'static str,
    pub width: u32,
    pub height: u32,
    pub kind: u32,
    pub x: i32,
    pub y: i32,
    pub opacity: f32,
    pub visible: bool,
    pub mode: u32,
    pub props: Vec<(u32, Vec<u8>)>,
    pub data: Vec<u8>,
    pub mask: Option<Vec<u8>>,
    pub apply_mask: bool,
}

impl TestLayer {
    pub fn rgba(name: &'static str, width: u32, height: u32, data: Vec<u8>) -> Self {
        TestLayer {
            name,
            width,
            height,
            kind: 1,
            x: 0,
            y: 0,
            opacity: 1.0,
            visible: true,
            mode: 28,
            props: Vec::new(),
            data,
            mask: None,
            apply_mask: true,
        }
    }
}

pub(super) struct TestImage {
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub base_type: u32,
    pub precision: u32,
    pub compression: Compression,
    pub image_props: Vec<(u32, Vec<u8>)>,
    pub layers: Vec<TestLayer>,
    pub channels: Vec<(&'static str, Vec<u8>, bool)>,
}

impl TestImage {
    pub fn rgb8(width: u32, height: u32) -> Self {
        TestImage {
            version: 11,
            width,
            height,
            base_type: 0,
            precision: 150,
            compression: Compression::Rle,
            image_props: Vec::new(),
            layers: Vec::new(),
            channels: Vec::new(),
        }
    }

    pub fn write(&self) -> Vec<u8> {
        let mut w = W::new(self.version);
        w.u32(self.width);
        w.u32(self.height);
        w.u32(self.base_type);
        if self.version >= 4 {
            w.u32(self.precision);
        }
        let comp = match self.compression {
            Compression::None => 0u8,
            Compression::Rle => 1,
            Compression::Zlib => 2,
        };
        w.prop(17, &[comp]);
        for (k, v) in &self.image_props {
            w.prop(*k, v);
        }
        w.end();
        let layer_slots: Vec<usize> = self.layers.iter().map(|_| w.slot()).collect();
        w.ptr(0);
        let channel_slots: Vec<usize> = self.channels.iter().map(|_| w.slot()).collect();
        w.ptr(0);
        if self.version >= 18 {
            w.ptr(0);
        }
        let bytes = match self.precision {
            100 | 150 => 1,
            200 | 250 | 500 | 550 => 2,
            300 | 350 | 600 | 650 => 4,
            _ => 8,
        };
        for (layer, slot) in self.layers.iter().zip(layer_slots) {
            let at = w.out.len();
            w.patch(slot, at);
            w.u32(layer.width);
            w.u32(layer.height);
            w.u32(layer.kind);
            w.string(layer.name);
            w.prop_u32(6, (layer.opacity * 255.0).round() as u32);
            w.prop(33, &layer.opacity.to_be_bytes());
            w.prop_u32(8, u32::from(layer.visible));
            w.prop_u32(7, layer.mode);
            let mut off = Vec::new();
            off.extend_from_slice(&layer.x.to_be_bytes());
            off.extend_from_slice(&layer.y.to_be_bytes());
            w.prop(15, &off);
            if layer.mask.is_some() {
                w.prop_u32(11, u32::from(layer.apply_mask));
            }
            for (k, v) in &layer.props {
                w.prop(*k, v);
            }
            w.end();
            let hslot = w.slot();
            let mslot = w.slot();
            if self.version >= 20 {
                w.ptr(0);
            }
            let channels = match layer.kind {
                0 => 3,
                1 => 4,
                2 | 4 => 1,
                _ => 2,
            };
            if !layer.data.is_empty() {
                let h = hierarchy(&mut w, layer.width, layer.height, channels * bytes, &layer.data, self.compression);
                w.patch(hslot, h);
            }
            if let Some(mask) = &layer.mask {
                let at = w.out.len();
                w.patch(mslot, at);
                w.u32(layer.width);
                w.u32(layer.height);
                w.string("mask");
                w.end();
                let hs = w.slot();
                let h = hierarchy(&mut w, layer.width, layer.height, bytes, mask, self.compression);
                w.patch(hs, h);
            }
        }
        for ((name, data, selection), slot) in self.channels.iter().zip(channel_slots) {
            let at = w.out.len();
            w.patch(slot, at);
            w.u32(self.width);
            w.u32(self.height);
            w.string(name);
            if *selection {
                w.prop(4, &[]);
            }
            w.end();
            let hs = w.slot();
            let h = hierarchy(&mut w, self.width, self.height, bytes, data, self.compression);
            w.patch(hs, h);
        }
        w.out
    }
}

const GO: &dyn Fn() -> bool = &|| true;

fn open(bytes: &[u8]) -> Result<Xcf<'_>> {
    Xcf::parse(bytes)
}

/// Parses and decodes everything, as an application would.
fn read_all(bytes: &[u8]) -> Result<usize> {
    let xcf = Xcf::parse(bytes)?;
    let mut budget = MAX_PIXEL_BYTES;
    let mut samples = 0;
    for i in 0..xcf.image().layers.len() {
        samples += xcf.layer_pixels(i, &mut budget, GO)?.map_or(0, |p| p.data.len());
        samples += xcf.layer_mask(i, &mut budget, GO)?.map_or(0, |(_, p)| p.data.len());
    }
    for i in 0..xcf.image().channel_count {
        samples += xcf.channel(i, &mut budget, GO)?.1.data.len();
    }
    xcf.image().tree();
    Ok(samples)
}

fn rgba(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let f = &f;
    (0..h).flat_map(|y| (0..w).flat_map(move |x| f(x, y))).collect()
}

fn pixels(xcf: &Xcf<'_>, layer: usize) -> Pixels {
    xcf.layer_pixels(layer, &mut MAX_PIXEL_BYTES.clone(), GO).unwrap().unwrap()
}

/// The samples of pixel (x, y), in the drawable's own coordinates.
fn at(px: &Pixels, x: u32, y: u32) -> &[f32] {
    let i = (y * px.width + x) as usize * px.channels;
    &px.data[i..i + px.channels]
}

fn path(indices: &[u32]) -> (u32, Vec<u8>) {
    (30, indices.iter().flat_map(|v| v.to_be_bytes()).collect())
}

fn names(image: &Image, nodes: &[TreeNode]) -> Vec<String> {
    nodes.iter().map(|n| image.layers[n.layer].name.clone()).collect()
}

#[test]
fn a_layer_decodes_with_every_compression() {
    for compression in [Compression::None, Compression::Rle, Compression::Zlib] {
        let mut img = TestImage::rgb8(70, 66);
        img.compression = compression;
        img.layers.push(TestLayer::rgba("A", 70, 66, rgba(70, 66, |x, y| [x as u8 * 3, y as u8 * 2, (x + y) as u8, 255 - x as u8])));
        let bytes = img.write();
        let xcf = open(&bytes).unwrap();
        let image = xcf.image();
        assert_eq!((image.width, image.height, image.version, image.compression), (70, 66, 11, compression));
        assert_eq!((image.base_type, image.precision), (BaseType::Rgb, Precision::GAMMA8));
        assert_eq!(image.layers.len(), 1, "{compression:?}");
        let px = pixels(&xcf, 0);
        assert_eq!((px.width, px.height, px.channels), (70, 66, 4));
        for (x, y) in [(0, 0), (63, 0), (64, 0), (69, 65), (10, 64)] {
            let want = [x as f32 * 3.0, y as f32 * 2.0, (x + y) as f32, 255.0 - x as f32].map(|v| v / 255.0);
            assert_eq!(at(&px, x, y), want, "{compression:?} at {x},{y}");
        }
    }
}

#[test]
fn every_truncation_errors_never_panics() {
    let mut img = TestImage::rgb8(20, 20);
    let mut l = TestLayer::rgba("A", 20, 20, rgba(20, 20, |x, y| [x as u8, y as u8, 0, 255]));
    l.mask = Some(vec![9; 400]);
    img.layers.push(l);
    img.channels.push(("Extra", vec![3; 400], false));
    let bytes = img.write();
    for cut in 0..bytes.len() {
        assert!(read_all(&bytes[..cut]).is_err(), "cut at {cut}");
    }
    assert_eq!(read_all(&bytes), Ok(1600 + 400 + 400));
}

#[test]
fn corrupted_files_never_panic() {
    let mut img = TestImage::rgb8(70, 20);
    let mut group = TestLayer::rgba("Group", 1, 1, Vec::new());
    group.props = vec![(29, vec![])];
    img.layers.push(group);
    let mut l = TestLayer::rgba("A", 70, 20, rgba(70, 20, |x, y| [x as u8, y as u8, 0, 255]));
    l.props = vec![path(&[0, 0])];
    l.mask = Some(vec![9; 1400]);
    img.layers.push(l);
    img.channels.push(("Sel", vec![3; 1400], true));
    for compression in [Compression::None, Compression::Rle, Compression::Zlib] {
        img.compression = compression;
        let bytes = img.write();
        assert!(read_all(&bytes).is_ok());
        let mut state = 0x9e37_79b9_u32;
        for _ in 0..2000 {
            let mut b = bytes.clone();
            for _ in 0..3 {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let i = state as usize % b.len();
                b[i] ^= (state >> 24) as u8 | 1;
            }
            let _ = read_all(&b);
        }
    }
}

#[test]
fn properties_offsets_opacity_mode_visibility_locks_and_tags() {
    let mut img = TestImage::rgb8(30, 20);
    let mut l = TestLayer::rgba("Top", 10, 8, rgba(10, 8, |x, y| [x as u8 * 20, y as u8 * 30, 0, 200]));
    l.x = -3;
    l.y = 15;
    l.opacity = 0.4;
    l.visible = false;
    l.mode = 30; // Multiply
    let one = 1u32.to_be_bytes().to_vec();
    l.props = vec![
        (10, one.clone()),
        (28, one.clone()),
        (32, one.clone()),
        (34, 2u32.to_be_bytes().to_vec()),
        (35, (-2i32).to_be_bytes().to_vec()),
        (36, 1i32.to_be_bytes().to_vec()),
        (37, (-1i32).to_be_bytes().to_vec()),
    ];
    img.layers.push(l);
    img.layers.push(TestLayer::rgba("Bottom", 30, 20, rgba(30, 20, |_, _| [9, 9, 9, 255])));
    let bytes = img.write();
    let xcf = open(&bytes).unwrap();
    let layers = &xcf.image().layers;
    assert_eq!(layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Top", "Bottom"], "file order: topmost first");
    let top = &layers[0];
    assert_eq!((top.mode, top.opacity, top.visible, top.x, top.y), (30, 0.4, false, -3, 15));
    assert!(top.lock_alpha && top.lock_content && top.lock_position);
    assert_eq!((top.color_tag, top.composite_mode, top.composite_space, top.blend_space), (2, -2, 1, -1));
    assert!(top.has_alpha() && top.has_pixels() && !top.has_mask() && top.apply_mask && top.expanded);
    assert!(!(top.group || top.text || top.vector || top.link || top.clipped || top.floating_selection));
    assert_eq!((top.width, top.height, top.layer_type, top.effects), (10, 8, 1, 0));
    // The layer's own pixel (5, 2); the offset is the caller's to apply.
    assert_eq!(at(&pixels(&xcf, 0), 5, 2), [100.0 / 255.0, 60.0 / 255.0, 0.0, 200.0 / 255.0]);
    let bottom = &layers[1];
    assert_eq!((bottom.mode, bottom.opacity, bottom.visible, bottom.color_tag), (28, 1.0, true, 0));
    assert!(!bottom.lock_alpha && !bottom.lock_content && !bottom.lock_position);
}

#[test]
fn groups_follow_item_paths() {
    // File order: Top, Group (0), child A [1,0], Nested group [1,1], nested child [1,1,0], Bottom.
    let mut img = TestImage::rgb8(16, 16);
    let px = |v: u8| rgba(4, 4, move |_, _| [v, v, v, 255]);
    img.layers.push(TestLayer::rgba("Top", 4, 4, px(1)));
    let mut group = TestLayer::rgba("Group", 1, 1, Vec::new());
    group.props = vec![(29, vec![]), (31, 0u32.to_be_bytes().to_vec())];
    img.layers.push(group);
    let mut a = TestLayer::rgba("A", 4, 4, px(2));
    a.props = vec![path(&[1, 0])];
    img.layers.push(a);
    let mut nested = TestLayer::rgba("Nested", 1, 1, Vec::new());
    nested.props = vec![(29, vec![]), path(&[1, 1])];
    nested.mode = 61; // Pass through
    img.layers.push(nested);
    let mut b = TestLayer::rgba("B", 4, 4, px(3));
    b.props = vec![path(&[1, 1, 0])];
    img.layers.push(b);
    img.layers.push(TestLayer::rgba("Bottom", 16, 16, rgba(16, 16, |_, _| [0, 0, 0, 255])));
    let bytes = img.write();
    let xcf = open(&bytes).unwrap();
    let image = xcf.image();
    let tree = image.tree();
    assert!(tree.misplaced.is_empty() && tree.too_deep.is_empty());
    assert_eq!(names(image, &tree.roots), ["Top", "Group", "Bottom"]);
    let g = &image.layers[tree.roots[1].layer];
    assert!(g.group && !g.expanded && !g.has_pixels() && g.mode == 28);
    assert_eq!(xcf.layer_pixels(tree.roots[1].layer, &mut 0, GO), Ok(None), "a group has no pixels and costs nothing");
    let children = &tree.roots[1].children;
    assert_eq!(names(image, children), ["A", "Nested"], "topmost first inside the group too");
    assert_eq!((image.layers[children[1].layer].mode, image.layers[children[1].layer].item_path.as_slice()), (61, &[1, 1][..]));
    assert_eq!(names(image, &children[1].children), ["B"]);
    assert_eq!(at(&pixels(&xcf, children[0].layer), 0, 0)[0], 2.0 / 255.0);
}

#[test]
fn a_path_into_nothing_lands_at_the_top_level() {
    let mut img = TestImage::rgb8(8, 8);
    let mut stray = TestLayer::rgba("Stray", 2, 2, rgba(2, 2, |_, _| [5, 5, 5, 255]));
    stray.props = vec![path(&[7, 0])];
    img.layers.push(stray);
    img.layers.push(TestLayer::rgba("Bottom", 8, 8, rgba(8, 8, |_, _| [0, 0, 0, 255])));
    let bytes = img.write();
    let tree = open(&bytes).unwrap().image().tree();
    assert_eq!((tree.roots.len(), tree.misplaced.as_slice()), (2, &[0][..]));
}

/// A path that fails below the top level (here at depth 2) still lands at the top level, so the
/// group it would have been pushed into keeps its indices for the siblings placed after it.
#[test]
fn a_path_failing_below_the_top_level_lands_at_the_top_level() {
    let mut img = TestImage::rgb8(8, 8);
    let px = |v: u8| rgba(2, 2, move |_, _| [v, v, v, 255]);
    let mut group = TestLayer::rgba("Group", 1, 1, Vec::new());
    group.props = vec![(29, vec![]), (31, 0u32.to_be_bytes().to_vec())];
    img.layers.push(group);
    let mut stray = TestLayer::rgba("Stray", 2, 2, px(5));
    stray.props = vec![path(&[0, 4, 0])];
    img.layers.push(stray);
    let mut a = TestLayer::rgba("A", 2, 2, px(6));
    a.props = vec![path(&[0, 0])];
    img.layers.push(a);
    img.layers.push(TestLayer::rgba("Bottom", 8, 8, rgba(8, 8, |_, _| [0, 0, 0, 255])));
    let bytes = img.write();
    let xcf = open(&bytes).unwrap();
    let (image, tree) = (xcf.image(), xcf.image().tree());
    assert_eq!(names(image, &tree.roots), ["Group", "Stray", "Bottom"]);
    assert_eq!(names(image, &tree.roots[0].children), ["A"], "the valid sibling placed after the stray one still lands in the group");
    assert_eq!(tree.misplaced, [1]);
}

#[test]
fn a_path_deeper_than_the_limit_moves_to_the_top_level() {
    let mut img = TestImage::rgb8(8, 8);
    let mut deep = TestLayer::rgba("Deep", 1, 1, vec![0; 4]);
    deep.props = vec![path(&[0; MAX_DEPTH + 1])];
    let mut edge = TestLayer::rgba("Edge", 1, 1, vec![0; 4]);
    edge.props = vec![path(&[0; MAX_DEPTH])];
    img.layers.extend([deep, edge]);
    let bytes = img.write();
    let tree = open(&bytes).unwrap().image().tree();
    assert_eq!((tree.roots.len(), tree.too_deep.as_slice(), tree.misplaced.as_slice()), (2, &[0][..], &[1][..]));
}

#[test]
fn masks_are_read_applied_or_not() {
    for apply in [true, false] {
        let mut img = TestImage::rgb8(20, 20);
        let mut l = TestLayer::rgba("Masked", 6, 6, rgba(6, 6, |_, _| [255, 0, 0, 255]));
        l.x = 4;
        l.y = 5;
        l.mask = Some((0..36).map(|i| (i * 7) as u8).collect());
        l.apply_mask = apply;
        img.layers.push(l);
        let bytes = img.write();
        let xcf = open(&bytes).unwrap();
        let layer = &xcf.image().layers[0];
        assert!(layer.has_mask());
        assert_eq!((layer.apply_mask, layer.x, layer.y), (apply, 4, 5));
        let (channel, px) = xcf.layer_mask(0, &mut MAX_PIXEL_BYTES.clone(), GO).unwrap().unwrap();
        assert_eq!((channel, px.width, px.height, px.channels), (Channel { name: "mask".into(), selection: false }, 6, 6, 1));
        assert_eq!(at(&px, 3, 2), [(2 * 6 + 3) as f32 * 7.0 / 255.0]);
    }
}

#[test]
fn sixteen_bit_and_float_samples_are_big_endian() {
    // 16-bit gamma integer (250): one 2x1 gray+alpha layer.
    let mut img = TestImage::rgb8(2, 1);
    img.base_type = 1;
    img.precision = 250;
    let data: Vec<u8> = [0x1234u16, 0xffff, 0x8000, 0x0001].into_iter().flat_map(u16::to_be_bytes).collect();
    let mut l = TestLayer::rgba("G", 2, 1, data);
    l.kind = 3; // gray with alpha
    img.layers.push(l);
    let bytes = img.write();
    let xcf = open(&bytes).unwrap();
    assert_eq!((xcf.image().base_type, xcf.image().precision), (BaseType::Gray, Precision { bytes: 2, float: false, linear: false }));
    let px = pixels(&xcf, 0);
    assert_eq!(px.data, [0x1234 as f32 / 65535.0, 1.0, 0x8000 as f32 / 65535.0, 1.0 / 65535.0]);

    // 32-bit float linear (600): RGB without alpha, values above 1 kept.
    let mut img = TestImage::rgb8(1, 1);
    img.precision = 600;
    let mut l = TestLayer::rgba("F", 1, 1, [0.25f32, 1.5, -0.5].into_iter().flat_map(f32::to_be_bytes).collect());
    l.kind = 0;
    img.layers.push(l);
    let bytes = img.write();
    let xcf = open(&bytes).unwrap();
    assert_eq!(xcf.image().precision, Precision { bytes: 4, float: true, linear: true });
    assert!(!xcf.image().layers[0].has_alpha());
    let px = pixels(&xcf, 0);
    assert_eq!((px.channels, px.data.as_slice()), (3, &[0.25, 1.5, -0.5][..]));

    // 16-bit float (500), 64-bit float (700) and 32-bit integer (350) decode too.
    for (precision, bytes, want) in
        [(500u32, 0x3a00u16.to_be_bytes().to_vec(), 0.75), (700, 0.75f64.to_be_bytes().to_vec(), 0.75), (350, u32::MAX.to_be_bytes().to_vec(), 1.0)]
    {
        let mut img = TestImage::rgb8(1, 1);
        img.base_type = 1;
        img.precision = precision;
        let mut l = TestLayer::rgba("F", 1, 1, bytes);
        l.kind = 2;
        img.layers.push(l);
        let bytes = img.write();
        assert_eq!(pixels(&open(&bytes).unwrap(), 0).data, [want], "precision {precision}");
    }
}

#[test]
fn half_floats_convert_exactly() {
    for (bits, want) in [
        (0x0000u16, 0.0f32),
        (0x3c00, 1.0),
        (0xc000, -2.0),
        (0x3555, (1024.0 + 341.0) / 4096.0),
        (0x7bff, 65504.0),
        (0x0400, 2f32.powi(-14)),
        (0x0001, 2f32.powi(-24)),
        (0x03ff, 1023.0 * 2f32.powi(-24)),
        (0x7c00, f32::INFINITY),
        (0xfc00, f32::NEG_INFINITY),
    ] {
        assert_eq!(half_to_f32(bits), want, "{bits:#06x}");
    }
    assert!(half_to_f32(0x8000).is_sign_negative() && half_to_f32(0x8000) == 0.0);
    assert!(half_to_f32(0x7e00).is_nan());
}

#[test]
fn precision_codes_depend_on_the_version() {
    let p = |bytes, float, linear| Precision { bytes, float, linear };
    assert_eq!(Precision::from_header(3, 999), Ok(Precision::GAMMA8));
    assert_eq!(Precision::from_header(4, 3), Ok(p(2, true, true)));
    assert_eq!(Precision::from_header(5, 500), Ok(p(4, true, true)));
    assert_eq!(Precision::from_header(6, 450), Ok(p(2, true, false)));
    assert_eq!(Precision::from_header(7, 500), Ok(p(2, true, true)));
    assert_eq!(Precision::from_header(12, 750), Ok(p(8, true, false)));
    for (version, code) in [(4, 5), (5, 600), (7, 400), (12, 0)] {
        assert!(Precision::from_header(version, code).is_err(), "{version} {code}");
    }
}

#[test]
fn indexed_images_use_the_colormap() {
    let mut img = TestImage::rgb8(2, 1);
    img.base_type = 2;
    let mut cmap = 3u32.to_be_bytes().to_vec();
    cmap.extend_from_slice(&[255, 0, 0, 0, 255, 0, 0, 0, 255]);
    img.image_props.push((1, cmap));
    let mut l = TestLayer::rgba("I", 2, 1, vec![2, 255, 1, 128]);
    l.kind = 5; // indexed with alpha
    img.layers.push(l);
    let mut plain = TestLayer::rgba("P", 2, 1, vec![0, 9]);
    plain.kind = 4; // indexed, no alpha; index 9 is past the colormap
    img.layers.push(plain);
    img.channels.push(("C", vec![255, 0], false));
    let bytes = img.write();
    let xcf = open(&bytes).unwrap();
    assert_eq!((xcf.image().base_type, xcf.image().colormap.len()), (BaseType::Indexed, 3));
    assert_eq!(pixels(&xcf, 0).data, [0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 128.0 / 255.0]);
    let p = pixels(&xcf, 1);
    assert_eq!((p.channels, p.data.as_slice()), (3, &[1.0, 0.0, 0.0, 9.0 / 255.0, 9.0 / 255.0, 9.0 / 255.0][..]));
    // Channels are plain samples even in an indexed image.
    assert_eq!(xcf.channel(0, &mut MAX_PIXEL_BYTES.clone(), GO).unwrap().1.data, [1.0, 0.0]);
}

#[test]
fn channels_are_the_extras_and_the_selection() {
    let mut img = TestImage::rgb8(4, 4);
    img.layers.push(TestLayer::rgba("L", 4, 4, rgba(4, 4, |_, _| [0, 0, 0, 255])));
    img.channels.push(("Extra", (0..16).map(|i| i as u8 * 16).collect(), false));
    img.channels.push(("Selection Mask", vec![255; 16], true));
    let bytes = img.write();
    let xcf = open(&bytes).unwrap();
    assert_eq!(xcf.image().channel_count, 2);
    let mut budget = 200;
    let (extra, px) = xcf.channel(0, &mut budget, GO).unwrap();
    assert_eq!((extra, at(&px, 3, 1)), (Channel { name: "Extra".into(), selection: false }, &[7.0 * 16.0 / 255.0][..]));
    let (selection, px) = xcf.channel(1, &mut budget, GO).unwrap();
    assert_eq!((selection.selection, px.data[0], budget), (true, 1.0, 200 - 2 * 64));
    assert!(xcf.channel(2, &mut budget, GO).is_err());
    assert!(xcf.layer_pixels(1, &mut budget, GO).is_err() && xcf.layer_mask(1, &mut budget, GO).is_err());
}

#[test]
fn image_parasites_resolution_guides_and_comment() {
    let mut img = TestImage::rgb8(10, 10);
    img.image_props.push((19, [300.0f32, 299.0].into_iter().flat_map(f32::to_be_bytes).collect()));
    let mut guides = Vec::new();
    for (position, orientation) in [(4i32, 1u8), (7, 2), (-3, 1), (5, 9)] {
        guides.extend_from_slice(&position.to_be_bytes());
        guides.push(orientation);
    }
    img.image_props.push((18, guides));
    let mut parasites = Vec::new();
    parasite_record(&mut parasites, "gimp-comment", b"hello\0");
    parasite_record(&mut parasites, "icc-profile", &[1, 2, 3]);
    img.image_props.push((21, parasites));
    // An unknown property is skipped by its length.
    img.image_props.push((99, vec![1, 2, 3, 4, 5]));
    img.layers.push(TestLayer::rgba("L", 10, 10, rgba(10, 10, |_, _| [0, 0, 0, 255])));
    let bytes = img.write();
    let xcf = open(&bytes).unwrap();
    let image = xcf.image();
    assert_eq!(image.resolution, Some((300.0, 299.0)));
    let (h, v) = (Orientation::Horizontal, Orientation::Vertical);
    assert_eq!(
        image.guides,
        [Guide { position: 4, orientation: h }, Guide { position: 7, orientation: v }],
        "a negative guide before version 15 is stray"
    );
    assert_eq!((image.comment.as_deref(), image.icc_profile.as_deref()), (Some("hello"), Some(&[1u8, 2, 3][..])));
    assert_eq!(image.path_count, 0);
    img.version = 15;
    assert_eq!(open(&img.write()).unwrap().image().guides.len(), 3, "from version 15 a negative guide is off canvas, not stray");
}

#[test]
fn clipping_text_vector_effects_and_floating_selections_are_flagged() {
    let mut img = TestImage::rgb8(4, 4);
    img.version = 20;
    let mut clipped = TestLayer::rgba("Clipped", 4, 4, rgba(4, 4, |_, _| [1, 1, 1, 255]));
    let mut parasite = Vec::new();
    parasite_record(&mut parasite, "gimphoto-clipped", &[1]);
    clipped.props = vec![(21, parasite), (26, 1u32.to_be_bytes().to_vec()), (47, vec![0; 8])];
    img.layers.push(clipped);
    let mut floating = TestLayer::rgba("Floating Selection", 2, 2, rgba(2, 2, |_, _| [2, 2, 2, 255]));
    floating.props = vec![(5, vec![0; 8])];
    img.layers.push(floating);
    let mut text = TestLayer::rgba("Bottom", 4, 4, rgba(4, 4, |_, _| [0, 0, 0, 255]));
    let mut parasite = Vec::new();
    parasite_record(&mut parasite, "gimp-text-layer", b"(text \"x\")");
    text.props = vec![(21, parasite)];
    img.layers.push(text);
    let bytes = img.write();
    let xcf = open(&bytes).unwrap();
    let image = xcf.image();
    let l = &image.layers;
    assert!(l[0].clipped && l[0].text && l[0].vector && !l[0].floating_selection);
    assert!(l[1].floating_selection && !l[1].clipped);
    assert!(l[2].text && !l[2].vector);
    assert_eq!(names(image, &image.tree().roots), ["Clipped", "Bottom"], "the floating selection is not part of the stack");
    // Version 20 layers end with a list of effect pointers.
    let effects_end = bytes.windows(8).rposition(|w| w == [0; 8]).unwrap();
    assert!(effects_end > 0);
}

#[test]
fn layer_effect_pointers_are_counted_and_bounded() {
    let layer = |effects: usize| {
        let mut w = W::new(20);
        for v in [1, 1, 0, 150] {
            w.u32(v);
        }
        w.end();
        let slot = w.slot();
        w.ptr(0);
        w.ptr(0);
        w.ptr(0); // paths
        let at = w.out.len();
        w.patch(slot, at);
        for v in [1, 1, 1] {
            w.u32(v);
        }
        w.string("fx");
        w.end();
        w.ptr(0);
        w.ptr(0);
        for _ in 0..effects {
            w.ptr(1);
        }
        w.ptr(0);
        w.out
    };
    assert_eq!(open(&layer(1024)).unwrap().image().layers[0].effects, 1024);
    assert!(open(&layer(1025)).is_err());
}

/// GIMP's link-layer length omits one u32. Trusting it shifts the hierarchy pointer by four
/// bytes, even though the cached pixels and every offset in the file are valid.
#[test]
fn linked_layers_open_cached_pixels_with_short_or_correct_property_lengths() {
    for (precision, data) in [
        (150, vec![51, 102, 153, 255]),
        (250, [13107u16, 26214, 39321, 65535].into_iter().flat_map(u16::to_be_bytes).collect()),
        (600, [0.2f32, 0.4, 0.6, 1.0].into_iter().flat_map(f32::to_be_bytes).collect()),
    ] {
        for (path, width, height) in [("assets/étiquette.svg", 2000, 1270), ("assets/label.png", 0, 0), ("", 0, 0)] {
            let mut payload = W { out: Vec::new(), wide: true };
            payload.u32(1);
            payload.string(path);
            payload.u32(width);
            payload.u32(height);
            let payload = payload.out;
            let marker = [48u32.to_be_bytes(), (payload.len() as u32).to_be_bytes()].concat();
            let mut img = TestImage::rgb8(1, 1);
            img.version = 25;
            img.precision = precision;
            let mut linked = TestLayer::rgba("Linked", 1, 1, data.clone());
            // Later properties must still be read, including unknown ones skipped by length.
            linked.props = vec![(48, payload.clone()), (99, vec![1, 2, 3]), (33, 0.375f32.to_be_bytes().to_vec()), (8, 0u32.to_be_bytes().to_vec())];
            img.layers.push(linked);
            img.layers.push(TestLayer::rgba("Bottom", 1, 1, data.clone()));
            for short in [false, true] {
                let mut bytes = img.write();
                let at = bytes.windows(marker.len()).position(|w| w == marker).unwrap();
                if short {
                    bytes[at + 4..at + 8].copy_from_slice(&((payload.len() - 4) as u32).to_be_bytes());
                }
                let xcf = open(&bytes).unwrap_or_else(|e| panic!("precision {precision}, path {path:?}, short {short}: {e}"));
                let layers = &xcf.image().layers;
                assert_eq!(layers.len(), 2);
                assert_eq!((layers[0].link, layers[0].opacity, layers[0].visible, layers[1].link), (true, 0.375, false, false));
                for layer in 0..2 {
                    for (got, expected) in pixels(&xcf, layer).data.into_iter().zip([0.2, 0.4, 0.6, 1.0]) {
                        assert!((got - expected).abs() < 1e-5, "precision {precision}: {got} != {expected}");
                    }
                }
            }
        }
    }
}

#[test]
fn truncated_and_oversized_link_layer_fields_are_errors() {
    let mut payload = W { out: Vec::new(), wide: true };
    payload.u32(1);
    payload.string("assets/label.svg");
    payload.u32(2000);
    payload.u32(1270);
    let payload = payload.out;
    let marker = [48u32.to_be_bytes(), (payload.len() as u32).to_be_bytes()].concat();
    let mut img = TestImage::rgb8(1, 1);
    img.version = 25;
    let mut linked = TestLayer::rgba("Linked", 1, 1, vec![51, 102, 153, 255]);
    linked.props.push((48, payload.clone()));
    img.layers.push(linked);
    let bytes = img.write();
    let start = bytes.windows(marker.len()).position(|w| w == marker).unwrap() + 8;
    // Cut within the flags, string length, path and either dimension.
    for end in start..start + payload.len() {
        assert!(open(&bytes[..end]).is_err(), "truncated at {end}");
    }
    for len in [(MAX_STRING + 1) as u32, u32::MAX] {
        let mut oversized = bytes.clone();
        oversized[start + 4..start + 8].copy_from_slice(&len.to_be_bytes());
        assert!(open(&oversized).is_err(), "path length {len}");
    }
}

#[test]
fn version_3_files_use_32_bit_pointers_and_legacy_modes() {
    let mut img = TestImage::rgb8(5, 5);
    img.version = 3;
    let mut l = TestLayer::rgba("Old", 5, 5, rgba(5, 5, |x, _| [x as u8, 0, 0, 255]));
    l.mode = 3; // Multiply (legacy)
    img.layers.push(l);
    let bytes = img.write();
    assert!(bytes.starts_with(b"gimp xcf v003\0"));
    let xcf = open(&bytes).unwrap();
    assert_eq!((xcf.image().version, xcf.image().precision, xcf.image().layers[0].mode), (3, Precision::GAMMA8, 3));
    assert_eq!(at(&pixels(&xcf, 0), 4, 0)[0], 4.0 / 255.0);
    // Version 0 ("file") as well.
    let mut img0 = TestImage::rgb8(2, 2);
    img0.version = 0;
    img0.layers.push(TestLayer::rgba("Oldest", 2, 2, rgba(2, 2, |_, _| [7, 7, 7, 255])));
    assert_eq!(open(&img0.write()).unwrap().image().version, 0);
    // A version newer than the reader knows still opens.
    img0.version = KNOWN_VERSION + 1;
    assert_eq!(read_all(&img0.write()), Ok(16));
}

#[test]
fn rle_long_runs_and_literals_round_trip() {
    // A tile with runs over 127 and long literal stretches, in every byte plane.
    let w = 64;
    let data = rgba(w, 64, |x, y| if y < 40 { [200, 1, 2, 255] } else { [(x * 7 + y) as u8, (x * 3) as u8, y as u8, 255] });
    assert_eq!(rle_decode(&rle(&data, 4), 4, 64 * 64).unwrap(), data);
    // Long literal (op 128) and the error cases.
    assert_eq!(rle_decode(&[128, 0, 3, 7, 8, 9], 1, 3).unwrap(), [7, 8, 9]);
    assert_eq!(rle_decode(&[127, 0, 4, 9], 1, 4).unwrap(), [9, 9, 9, 9]);
    assert_eq!(rle_decode(&[254, 1, 2, 0, 5], 1, 3).unwrap(), [1, 2, 5]);
    for bad in [&[][..], &[3], &[127, 0], &[128, 0, 3, 7], &[2, 9], &[253, 1]] {
        assert!(rle_decode(bad, 1, 2).is_err(), "{bad:?}");
    }
}

#[test]
fn limits_and_malformed_structures_are_errors() {
    // A canvas past the side limit, and one past the area limit.
    let mut big = TestImage::rgb8(1, 1);
    big.width = MAX_SIDE + 1;
    assert!(open(&big.write()).is_err());
    (big.width, big.height) = (MAX_SIDE, 224);
    assert!(open(&big.write()).is_err());
    big.height = 223;
    assert!(open(&big.write()).is_ok());
    big.height = 0;
    assert!(open(&big.write()).is_err());
    // A layer whose declared size has no pixels behind it (hierarchy pointer to garbage).
    let mut img = TestImage::rgb8(4, 4);
    img.layers.push(TestLayer::rgba("L", 4, 4, rgba(4, 4, |_, _| [0; 4])));
    let mut bytes = img.write();
    let n = bytes.len();
    bytes[n - 10..].fill(0xff);
    assert!(read_all(&bytes).is_err());
    // Wrong signature, unknown version tag, unknown compression, image type and precision.
    assert!(matches!(open(b"gimp xcf vXYZ\0"), Err(Error::Invalid(_))));
    assert!(open(b"not an xcf").is_err());
    let good = img.write();
    assert!(read_all(&good).is_ok());
    let patched = |at: usize, v: u8| {
        let mut b = good.clone();
        b[at] = v;
        open(&b).err()
    };
    assert!(patched(14 + 12 + 4 + 8, 3).is_some(), "compression");
    assert!(patched(14 + 11, 3).is_some(), "image type");
    assert!(patched(14 + 15, 1).is_some(), "precision");
    // A layer of another colour model, and an unknown layer type.
    for kind in [2, 6] {
        img.layers[0].kind = kind;
        assert!(open(&img.write()).is_err(), "layer type {kind}");
    }
    // Pixels of another size than the layer declares.
    img.layers[0].kind = 1;
    let mut bytes = img.write();
    let at = bytes.windows(12).rposition(|w| w == [0, 0, 0, 4, 0, 0, 0, 4, 0, 0, 0, 4]).unwrap();
    bytes[at + 3] = 3;
    assert!(open(&bytes).is_ok() && read_all(&bytes).is_err());
}

#[test]
fn layer_channel_property_and_metadata_counts_are_bounded() {
    let header = |w: &mut W| {
        for v in [1, 1, 0, 150] {
            w.u32(v);
        }
        w.end();
    };
    // More layer pointers, then more channel pointers, than the limits.
    for (layers, channels, ok) in [(MAX_LAYERS + 1, 0, false), (0, MAX_CHANNELS + 1, false), (0, MAX_CHANNELS, true)] {
        let mut w = W::new(11);
        header(&mut w);
        (0..layers).for_each(|_| w.ptr(1));
        w.ptr(0);
        (0..channels).for_each(|_| w.ptr(1));
        w.ptr(0);
        assert_eq!(open(&w.out).is_ok(), ok, "{layers} layers, {channels} channels");
    }
    // 4097 properties.
    let mut w = W::new(11);
    for v in [1, 1, 0, 150] {
        w.u32(v);
    }
    (0..4097).for_each(|_| w.prop(99, &[]));
    w.end();
    w.ptr(0);
    w.ptr(0);
    assert_eq!(open(&w.out).err(), Some(Error::Invalid("too many properties".into())));
    // A property declaring more than the payload limit.
    let mut w = W::new(11);
    for v in [1, 1, 0, 150, 99, MAX_PAYLOAD as u32 + 1] {
        w.u32(v);
    }
    assert!(matches!(open(&w.out), Err(Error::Invalid(m)) if m.contains("declares")));
    // Forty pointers to one layer whose name is a megabyte: its copies pass the metadata budget.
    let many = |pointers: usize| {
        let mut w = W::new(11);
        header(&mut w);
        let slots: Vec<usize> = (0..pointers).map(|_| w.slot()).collect();
        w.ptr(0);
        w.ptr(0);
        let at = w.out.len();
        slots.into_iter().for_each(|s| w.patch(s, at));
        for v in [1, 1, 1] {
            w.u32(v);
        }
        w.string(&"n".repeat(MAX_STRING - 1));
        w.end();
        w.ptr(0);
        w.ptr(0);
        w.out
    };
    assert!(open(&many(32)).is_ok());
    assert_eq!(open(&many(33)).err(), Some(Error::Invalid("layer metadata exceeds the budget".into())));
}

#[test]
fn the_pixel_budget_is_charged_before_decoding_and_decoding_can_be_cancelled() {
    let mut img = TestImage::rgb8(200, 200);
    img.layers.push(TestLayer::rgba("L", 200, 200, rgba(200, 200, |_, _| [1, 2, 3, 255])));
    let bytes = img.write();
    let xcf = open(&bytes).unwrap();
    let cost = 200 * 200 * 4 * 4;
    let mut budget = cost - 1;
    assert!(matches!(xcf.layer_pixels(0, &mut budget, GO), Err(Error::Invalid(m)) if m.contains("pixel limit")));
    let mut budget = cost + 5;
    assert!(xcf.layer_pixels(0, &mut budget, GO).is_ok());
    assert_eq!(budget, 5);
    assert_eq!(xcf.layer_pixels(0, &mut MAX_PIXEL_BYTES.clone(), &|| false), Err(Error::Cancelled));
}

#[test]
fn gzipped_files_are_recognised_by_name_and_inflated() {
    use std::io::Write;
    let mut img = TestImage::rgb8(3, 3);
    img.layers.push(TestLayer::rgba("L", 3, 3, rgba(3, 3, |_, _| [4, 5, 6, 255])));
    let plain = img.write();
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(&plain).unwrap();
    let gz = e.finish().unwrap();
    assert!(is_xcf(&plain) && !is_xcf(&gz));
    assert!(is_gzipped_xcf("a/b/photo.XCF.GZ", &gz) && is_gzipped_xcf("x.xcfgz", &gz) && is_gzipped_xcf("c:\\dir.xcf.gz\\y.xcf.gz", &gz));
    assert!(!is_gzipped_xcf("photo.xcf", &gz) && !is_gzipped_xcf("photo.xcf.gz", &plain) && !is_gzipped_xcf("dir.xcf.gz/photo.gz", &gz));
    assert_eq!(gunzip(&gz).unwrap(), plain);
    assert!(gunzip(&gz[..gz.len() / 2]).is_err());
}

/// `PROP_FLOAT_OPACITY` wins over `PROP_OPACITY` whichever comes first in the file.
#[test]
fn float_opacity_wins_over_opacity_in_either_order() {
    let mut img = TestImage::rgb8(4, 4);
    // The writer emits OPACITY (0.4 → 102) then FLOAT_OPACITY 0.4; an OPACITY 255 after them must not win.
    let mut byte_last = TestLayer::rgba("Float then byte", 4, 4, rgba(4, 4, |_, _| [1, 2, 3, 255]));
    byte_last.opacity = 0.4;
    byte_last.props = vec![(6, 255u32.to_be_bytes().to_vec())];
    // A later FLOAT_OPACITY replaces the earlier one; one that is not finite does not.
    let mut float_last = TestLayer::rgba("Byte then float", 4, 4, rgba(4, 4, |_, _| [1, 2, 3, 255]));
    float_last.opacity = 0.4;
    float_last.props = vec![(33, 0.25f32.to_be_bytes().to_vec()), (33, f32::NAN.to_be_bytes().to_vec())];
    img.layers.push(byte_last);
    img.layers.push(float_last);
    let bytes = img.write();
    let xcf = open(&bytes).unwrap();
    assert_eq!((xcf.image().layers[0].opacity, xcf.image().layers[1].opacity), (0.4, 0.25));
}

/// One PROP_PARASITES record: name, u32 flags, u32 len, then the payload.
fn parasite_record(out: &mut Vec<u8>, name: &str, payload: &[u8]) {
    out.extend_from_slice(&((name.len() as u32) + 1).to_be_bytes());
    out.extend_from_slice(name.as_bytes());
    out.push(0);
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
}

#[test]
fn duplicate_parasites_keep_order_and_last_value() {
    let mut data = Vec::new();
    parasite_record(&mut data, "gimp-comment", b"first");
    parasite_record(&mut data, "gimp-image-grid", b"grid");
    parasite_record(&mut data, "gimp-comment", b"second");
    data.extend_from_slice(&[0, 0, 0, 9, b'c', b'u', b't']);
    let out = parasites(&data);
    assert_eq!(out, [("gimp-comment".to_string(), &b"second"[..]), ("gimp-image-grid".to_string(), &b"grid"[..])]);
}

#[test]
fn a_user_unit_property_is_measured_not_trusted() {
    // PROP_USER_UNIT's length word is wrong in every release: three strings (XCF 21+) or five.
    // Which it is, is decided by whether a property header follows the third: a fourth string
    // shorter than 64 bytes would look like one, so the five-string case here has a long one.
    let long = "s".repeat(70);
    for strings in [3, 5] {
        let mut img = TestImage::rgb8(2, 2);
        let mut unit = W { out: Vec::new(), wide: true };
        unit.out.extend_from_slice(&25.4f32.to_be_bytes());
        unit.u32(2);
        (0..strings).for_each(|i| unit.string(["id", "sym", "abbr", long.as_str(), "plural"][i]));
        img.image_props.push((24, unit.out));
        img.image_props.push((19, [96.0f32, 96.0].into_iter().flat_map(f32::to_be_bytes).collect()));
        img.layers.push(TestLayer::rgba("L", 2, 2, rgba(2, 2, |_, _| [4, 5, 6, 255])));
        let mut bytes = img.write();
        let marker = 24u32.to_be_bytes();
        let at = bytes.windows(4).position(|w| w == marker).unwrap();
        bytes[at + 4..at + 8].copy_from_slice(&3u32.to_be_bytes());
        let xcf = open(&bytes).unwrap_or_else(|e| panic!("{strings} strings: {e}"));
        assert_eq!(xcf.image().resolution, Some((96.0, 96.0)), "{strings} strings");
        assert_eq!(read_all(&bytes), Ok(16));
    }
}

#[test]
fn an_rle_run_of_zero_length_is_an_error() {
    // Long runs (op 127 / 128) carry a 16-bit length; zero consumes input without producing output.
    assert!(rle_decode(&[127, 0, 0, 5, 3, 0, 9], 1, 4).is_err());
    assert!(rle_decode(&[128, 0, 0, 3, 0, 9], 1, 4).is_err());
    assert_eq!(rle_decode(&[127, 0, 4, 9], 1, 4).unwrap(), [9, 9, 9, 9]);
}

#[test]
fn channel_names_are_charged_to_the_metadata_budget() {
    // Channel pointers that all name one channel with a megabyte name: each read copies the name.
    let mut w = W::new(11);
    for v in [1, 1, 0, 150] {
        w.u32(v);
    }
    w.end();
    w.ptr(0);
    let slots: Vec<usize> = (0..40).map(|_| w.slot()).collect();
    w.ptr(0);
    let at = w.out.len();
    slots.into_iter().for_each(|s| w.patch(s, at));
    w.u32(1);
    w.u32(1);
    w.string(&"n".repeat(MAX_STRING - 1));
    w.end();
    let hs = w.slot();
    let h = hierarchy(&mut w, 1, 1, 1, &[7], Compression::None);
    w.patch(hs, h);
    let xcf = open(&w.out).unwrap();
    let mut budget = MAX_PIXEL_BYTES;
    let results: Vec<_> = (0..40).map(|i| xcf.channel(i, &mut budget, GO).map(|(c, _)| c.name.len())).collect();
    assert!(results[..32].iter().all(|r| *r == Ok(MAX_STRING - 1)), "32 copies fit the 32 MiB budget");
    assert!(results[32..].iter().all(|r| *r == Err(Error::Invalid("channel metadata exceeds the budget".into()))));
}
