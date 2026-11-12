use super::*;
use craft_tiff::{IfdBuilder, ImageData, TiffWriter, Value};

fn fake_jpeg(n: usize) -> Vec<u8> {
    let mut j = vec![0xff, 0xd8, 0xff, 0xc0, 0x00, 0x0b, 8, 0, 1, 0, 1, 1, 1, 0x11, 0];
    j.extend(std::iter::repeat_n(0x55u8, n));
    j.extend_from_slice(&[0xff, 0xd9]);
    j
}

/// Preview extraction only serves single-chunk JPEG-compressed IFDs and asks the `single_chunk` first:
/// a TIFF whose strip array declares 100 000 one-row strips and no StripByteCounts must not build their
/// descriptor vector, and of course finds no preview in it.
#[test]
fn a_multi_strip_jpeg_ifd_costs_no_descriptor_vector() {
    let mut ifd = IfdBuilder::new();
    ifd.set(t::COMPRESSION, Value::Short(vec![7]));
    ifd.set(t::IMAGE_WIDTH, Value::Long(vec![16]));
    ifd.set(t::IMAGE_LENGTH, Value::Long(vec![100_000]));
    ifd.set(t::ROWS_PER_STRIP, Value::Short(vec![1]));
    ifd.set(t::STRIP_OFFSETS, Value::Byte((0..100_000).map(|i| (i % 255 + 1) as u8).collect()));
    let file = TiffWriter::default().write(&[ifd]).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || tx.send(embedded_preview(&file)));
    assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(2)), Ok(None));
}

/// A JPEG like [`fake_jpeg`] whose frame header says `w` x `h` with `nc` components.
fn sized_jpeg(w: u16, h: u16, nc: u8, n: usize) -> Vec<u8> {
    let mut j = vec![0xff, 0xd8, 0xff, 0xc0, 0x00, 8 + 3 * nc, 8];
    j.extend_from_slice(&h.to_be_bytes());
    j.extend_from_slice(&w.to_be_bytes());
    j.push(nc);
    for c in 0..nc {
        j.extend_from_slice(&[c + 1, 0x11, 0]);
    }
    j.extend(std::iter::repeat_n(0x55u8, n));
    j.extend_from_slice(&[0xff, 0xd9]);
    j
}

/// A TIFF raw holding `jpegs` as JPEG-compressed strips: the first in IFD0, the others in sub-IFDs.
fn tiff_with_jpegs(jpegs: &[&[u8]]) -> Vec<u8> {
    let ifd = |j: &[u8]| {
        let mut ifd = IfdBuilder::new();
        ifd.set(t::COMPRESSION, Value::Short(vec![6]));
        ifd.set(t::IMAGE_WIDTH, Value::Long(vec![1]));
        ifd.set(t::IMAGE_LENGTH, Value::Long(vec![1]));
        ifd.set_image(ImageData::Strips { rows_per_strip: 1, strips: vec![j.to_vec()] });
        ifd
    };
    let mut ifd0 = ifd(jpegs[0]);
    for j in &jpegs[1..] {
        ifd0.add_sub_ifd(ifd(j));
    }
    TiffWriter::default().write(&[ifd0]).unwrap()
}

/// A stand-in decodes the smallest JPEG that covers its box and shows the same picture as the largest: not a
/// thumbnail of another shape (a letterboxed one) or a grey mask, and the largest when no smaller one is big enough.
#[test]
fn a_stand_in_takes_the_smallest_covering_jpeg_of_the_same_shape() {
    let (big, small, boxed) = (sized_jpeg(9984, 6656, 3, 500), sized_jpeg(1616, 1080, 3, 50), sized_jpeg(160, 120, 3, 10));
    // a ProRAW DNG keeps grey semantic masks of the picture's shape as JPEGs too
    let mask = sized_jpeg(768, 512, 1, 20);
    let file = tiff_with_jpegs(&[&small, &big, &boxed, &mask]);
    assert_eq!(embedded_preview(&file).unwrap(), big, "unchanged: the largest");
    assert_eq!(embedded_preview_covering(&file, 1600).unwrap(), small, "1616 px covers a 1600 px box");
    assert_eq!(embedded_preview_covering(&file, 1616).unwrap(), small);
    assert_eq!(embedded_preview_covering(&file, 1617).unwrap(), big, "too small: the largest");
    assert_eq!(embedded_preview_covering(&file, u32::MAX).unwrap(), big);
    assert_eq!(embedded_preview_covering(&file, 100).unwrap(), small, "the 4:3 thumbnail and the grey mask are not this picture");
    // a small copy of the same shape (within 1 %) is taken
    let thumb = sized_jpeg(160, 107, 3, 10);
    let file = tiff_with_jpegs(&[&small, &big, &thumb]);
    assert_eq!(embedded_preview_covering(&file, 100).unwrap(), thumb);
    assert_eq!(embedded_preview_covering(&tiff_with_jpegs(&[&big]), 100).unwrap(), big, "one preview: that one");
    for n in (0..file.len()).step_by(7) {
        let _ = embedded_preview_covering(&file[..n], 100);
    }
}

/// A CR2 shot in Adobe RGB writes EXIF ColorSpace Uncalibrated with InteropIndex "R98", not "R03": the Canon
/// maker note (0x00b4) says Adobe RGB (storytold/lightcraft#2654). Another make's note is not read for it.
#[test]
fn cr2_preview_colour_space_comes_from_the_canon_maker_note() {
    use PreviewColorSpace::{AdobeRgb, Srgb};
    let cr2 = |make: &str, exif_space: u16, note: Option<u16>| {
        let mut exif = IfdBuilder::new();
        exif.set(0xA001, Value::Short(vec![exif_space]));
        exif.set_child(t::INTEROP_IFD, IfdBuilder::new().with(0x0001, Value::Undefined(b"R98".to_vec())));
        if let Some(v) = note {
            // a plain one-entry IFD, its SHORT value inline
            let mut mn = 1u16.to_le_bytes().to_vec();
            mn.extend([0x00b4u16.to_le_bytes(), 3u16.to_le_bytes()].concat());
            mn.extend(1u32.to_le_bytes());
            mn.extend([v.to_le_bytes(), [0, 0]].concat());
            mn.extend(0u32.to_le_bytes());
            exif.set(t::MAKER_NOTE, Value::Undefined(mn));
        }
        let mut ifd0 = IfdBuilder::new().with(t::MAKE, Value::Ascii(make.into()));
        ifd0.set_child(t::EXIF_IFD, exif);
        TiffWriter::new(craft_tiff::ByteOrder::Little, false).write(&[ifd0]).unwrap()
    };
    assert_eq!(embedded_preview_color_space(&cr2("Canon", 0xFFFF, Some(2))), Some(AdobeRgb));
    assert_eq!(embedded_preview_color_space(&cr2("Canon", 1, Some(1))), Some(Srgb));
    assert_eq!(embedded_preview_color_space(&cr2("Canon", 1, None)), Some(Srgb), "no maker note: the EXIF");
    assert_eq!(embedded_preview_color_space(&cr2("Canon", 0xFFFF, None)), None, "R98 is not Adobe RGB's R03");
    assert_eq!(embedded_preview_color_space(&cr2("Canon", 0xFFFF, Some(0xFFFF))), None);
    assert_eq!(embedded_preview_color_space(&cr2("Leica", 0xFFFF, Some(2))), None, "only a Canon note");
}

/// A structurally complete DCT JPEG (no real image data): `nc` components, `n` entropy bytes.
fn scan_jpeg(nc: u8, n: usize) -> Vec<u8> {
    let mut j = vec![0xff, 0xd8, 0xff, 0xdb, 0x00, 0x43, 0x00];
    j.extend(std::iter::repeat_n(8u8, 64));
    j.extend_from_slice(&[0xff, 0xc0, 0x00, 8 + 3 * nc, 8, 0, 16, 0, 24, nc]);
    for c in 0..nc {
        j.extend_from_slice(&[c + 1, 0x11, 0]);
    }
    j.extend_from_slice(&[0xff, 0xda, 0x00, 6 + 2 * nc, nc]);
    for c in 0..nc {
        j.extend_from_slice(&[c + 1, 0]);
    }
    j.extend_from_slice(&[0, 63, 0]);
    j.extend(std::iter::repeat_n(0x55u8, n));
    j.extend_from_slice(&[0xff, 0x00, 0x12, 0xff, 0xd0, 0x34, 0xff, 0xd9]);
    j
}

/// `units` copies of a 200-byte record whose `d8 ff db` start runs each walk on through the
/// starts after it (a crafted Minolta-like container).
fn many_fake_jpegs(units: usize) -> Vec<u8> {
    let mut unit = vec![0u8, 0xd8, 0xff, 0xdb, 0x00, 0x02, 0xff, 0xda, 0x00, 0x02];
    unit.extend(std::iter::repeat_n(0x55u8, 190));
    let mut f = b"\0MRM\0\x01\0\0".to_vec();
    f.reserve(units * unit.len());
    for _ in 0..units {
        f.extend_from_slice(&unit);
    }
    f
}

/// The production scan's budget policy, restated here so the tests assert an expected bound
/// independently instead of calling the helper under test: eight times the file's size, at
/// least 1 MiB.
fn policy_budget(len: usize) -> u64 {
    (len as u64 * 8).max(1 << 20)
}

/// A crafted 40 MB container full of JPEG-like starts (each walk running on through the starts
/// after it) is scanned in bounded work: the walks share a budget (without it this file would
/// walk about 8·10^10 bytes), so the charged work stays within the policy budget plus one walk's
/// reach. No wall clock: this cannot fail on a slow runner, and removing the budget shows up as
/// the work exploding.
#[test]
fn a_scan_through_many_fake_jpegs_stays_linear() {
    let f = many_fake_jpegs(200_000);
    assert_eq!(f.len(), 40_000_008, "the adversarial fixture the budget was measured on");
    let (found, walked) = scan_with_budget(&f, policy_budget(f.len()) as usize);
    assert_eq!(found, None);
    let bound = policy_budget(f.len()) + f.len() as u64;
    assert!(walked <= bound, "walked {walked} bytes of a {} byte file (bound {bound})", f.len());
}

/// A walk whose final entropy search finds no `ff` has examined the rest of the file, so that
/// search is charged to the budget before the walk fails. Without the charge the 40 MB tail is
/// scanned for free and only the few KB before it are reported, while `left` stays positive and
/// the budget no longer bounds the work.
#[test]
fn a_failed_terminal_search_is_charged_to_the_budget() {
    let mut f = many_fake_jpegs(20);
    f.extend(std::iter::repeat_n(0x55u8, 40_000_000));
    let (found, walked) = scan_with_budget(&f, 1);
    assert_eq!(found, None);
    assert!(walked >= 40_000_000, "terminal search was not charged: {walked} of {} bytes", f.len());
}

/// The production wrapper stops at the same shared budget. Eighteen fake candidates each declare
/// a segment length that makes their walk jump 0xFFF0 bytes, exhausting the tiny file's 1 MiB
/// allowance before the valid JPEG after them: the wrapper finds nothing. A `usize::MAX` budget
/// (or no budget at all) would walk on and return that JPEG.
#[test]
fn the_production_wrapper_stops_at_the_shared_budget() {
    let mut f = b"\0MRM\0\x01\0\0".to_vec();
    for _ in 0..18 {
        f.extend_from_slice(&[0x00, 0xd8, 0xff, 0xe1, 0xff, 0xf0]);
    }
    f.extend_from_slice(&scan_jpeg(3, 100));
    assert_eq!(scan_for_jpeg(&f), None);
}

/// The shared budget is what bounds the work, and the counter reports the work itself. The same
/// generator at 200 starts is a 40 KB file: without the budget (never decremented, or restarted
/// for each candidate) the scan walks all 200 walks, 4 MB, instead of about 1 MB — so a negative
/// run costs megabytes, not the 8·10^10 bytes of the 40 MB fixture.
#[test]
fn the_scan_budget_bounds_the_work_a_crafted_file_can_cause() {
    let f = many_fake_jpegs(200);
    let budget = policy_budget(f.len());
    let (found, walked) = scan_with_budget(&f, budget as usize);
    assert_eq!(found, None);
    assert!(walked >= (f.len() as u64) / 2, "the walks ran: {walked} bytes of {}", f.len());
    assert!(walked <= budget + (f.len() as u64), "walked {walked} with a {budget} byte budget");
    // The counter records the work, not the unspent budget: a budget of one file size is spent
    // by the first walk, and the second walk is still counted in full (it overshoots).
    let (_, spent) = scan_with_budget(&f, f.len());
    let len = f.len() as u64;
    assert!(spent > len && spent <= 2 * len, "spent {spent} with a {len}-byte budget");
    // No candidate, no work counted.
    let none = b"no jpeg here";
    assert_eq!(scan_with_budget(none, policy_budget(none.len()) as usize).1, 0);
    // One real candidate: the counter counts exactly the reach of its walk.
    let mut one = vec![0u8];
    let jpeg = scan_jpeg(3, 100);
    one.extend_from_slice(&jpeg);
    let (found, walked) = scan_with_budget(&one, policy_budget(one.len()) as usize);
    assert!(found.is_some());
    assert_eq!(walked, jpeg.len() as u64, "one walk of the JPEG's {} bytes", jpeg.len());
    // The production wrapper takes the same path and returns the counter's preview.
    assert_eq!(scan_for_jpeg(&one).as_deref(), found.as_deref());
}

/// Containers whose structure we do not walk give up their largest colour JPEG, found by scanning.
#[test]
fn scanned_containers_give_the_largest_colour_jpeg() {
    let (small, big, mosaic) = (scan_jpeg(3, 100), scan_jpeg(3, 900), scan_jpeg(1, 5000));
    let heads: [&[u8]; 3] = [b"II\x1a\0\0\0HEAPCCDR", b"\0MRM\0\x01\0\0", b"FOVb\x02\0\x02\0"];
    for head in heads {
        let mut f = head.to_vec();
        for part in [&small, &mosaic, &big] {
            f.extend_from_slice(&[0xff, 0xd8, 0xff, 0x00, 0x00]); // a stray SOI-like run: not a JPEG
            f.extend_from_slice(part);
            f.extend_from_slice(&[0u8; 7]);
        }
        // CRW and MRW decode; their previews are still found by the scan, the others stay preview-only containers
        assert!(crate::probe(&f).is_some_and(|p| !p.is_supported() || p == crate::RawFormat::Crw || p == crate::RawFormat::Mrw), "{head:?}");
        assert_eq!(embedded_preview(&f).as_deref(), Some(&big[..]), "{head:?}");
        for n in (0..f.len()).step_by(11) {
            let _ = embedded_preview(&f[..n]);
        }
    }
    // a preview whose first byte was overwritten (Minolta and Epson files store 00, 02 or ee there) is restored
    let mut f = b"\0MRM\0\x01\0\0".to_vec();
    f.push(0x02);
    f.extend_from_slice(&big[1..]);
    assert_eq!(embedded_preview(&f).as_deref(), Some(&big[..]));
    // only a one-component JPEG, or none: nothing
    let mut f = b"FOVb\x02\0\x02\0".to_vec();
    f.extend_from_slice(&mosaic);
    assert_eq!(embedded_preview(&f), None);
}

/// A corrupt raw whose preview pointer (JPEGInterchangeFormat and its length) lies past the end
/// of the file, or whose length runs past it, finds no preview and never panics; a real preview
/// beside the lying pointer is still found (FredApps on storytold/lightcraft#3080: the preview must be read with
/// bounds on its offset and size).
#[test]
fn a_preview_pointer_past_the_end_of_the_file_finds_no_preview() {
    let real = sized_jpeg(640, 480, 3, 200);
    let lying = |off: u32, len: u32, with_real: bool| {
        let mut ifd = IfdBuilder::new();
        ifd.set(t::JPEG_INTERCHANGE_FORMAT, Value::Long(vec![off]));
        ifd.set(t::JPEG_INTERCHANGE_FORMAT_LENGTH, Value::Long(vec![len]));
        if with_real {
            ifd.set(t::COMPRESSION, Value::Short(vec![6]));
            ifd.set(t::IMAGE_WIDTH, Value::Long(vec![1]));
            ifd.set(t::IMAGE_LENGTH, Value::Long(vec![1]));
            ifd.set_image(ImageData::Strips { rows_per_strip: 1, strips: vec![real.clone()] });
        }
        TiffWriter::default().write(&[ifd]).unwrap()
    };
    let short = lying(0, 0, false).len() as u32;
    for (off, len) in [(u32::MAX, u32::MAX), (u32::MAX, 16), (short + 10, 4096), (8, u32::MAX), (short - 2, u32::MAX - 1)] {
        let file = lying(off, len, false);
        assert_eq!(embedded_preview(&file), None, "pointer {off:#x}+{len:#x} in a {} byte file", file.len());
        assert_eq!(embedded_preview_covering(&file, 64), None);
        assert_eq!(embedded_preview_color_space(&file), None);
        let with_real = lying(off, len, true);
        assert_eq!(embedded_preview(&with_real).as_deref(), Some(&real[..]), "the strip preview beside pointer {off:#x}+{len:#x}");
        // and every truncation of the file (a raw cut short by a failed copy) is safe too
        for n in 0..with_real.len() {
            let _ = embedded_preview_covering(&with_real[..n], 64);
        }
    }
}

/// A preview pointer whose length runs past the end of a file that cuts its JPEG short (no end
/// marker) finds no preview, so a smaller whole one wins; an overstated length whose JPEG is whole
/// inside the file still finds it (CodeRabbit on storytold/lightcraft#3077).
#[test]
fn a_preview_cut_short_by_the_end_of_the_file_is_not_taken() {
    let big = sized_jpeg(1600, 1200, 3, 4000);
    let small = sized_jpeg(160, 120, 3, 100);
    // the small JPEG as a strip, the big one pointed to at the end of the file
    let with_tail = |tail: &[u8], declared: u32| {
        let probe = {
            let mut ifd = IfdBuilder::new();
            ifd.set(t::JPEG_INTERCHANGE_FORMAT, Value::Long(vec![0]));
            ifd.set(t::JPEG_INTERCHANGE_FORMAT_LENGTH, Value::Long(vec![0]));
            ifd.set(t::COMPRESSION, Value::Short(vec![6]));
            ifd.set(t::IMAGE_WIDTH, Value::Long(vec![1]));
            ifd.set(t::IMAGE_LENGTH, Value::Long(vec![1]));
            ifd.set_image(ImageData::Strips { rows_per_strip: 1, strips: vec![small.clone()] });
            ifd
        };
        let at = TiffWriter::default().write(&[probe]).unwrap().len() as u32;
        let mut ifd = IfdBuilder::new();
        ifd.set(t::JPEG_INTERCHANGE_FORMAT, Value::Long(vec![at]));
        ifd.set(t::JPEG_INTERCHANGE_FORMAT_LENGTH, Value::Long(vec![declared]));
        ifd.set(t::COMPRESSION, Value::Short(vec![6]));
        ifd.set(t::IMAGE_WIDTH, Value::Long(vec![1]));
        ifd.set(t::IMAGE_LENGTH, Value::Long(vec![1]));
        ifd.set_image(ImageData::Strips { rows_per_strip: 1, strips: vec![small.clone()] });
        let mut file = TiffWriter::default().write(&[ifd]).unwrap();
        assert_eq!(file.len() as u32, at, "the pointer lands right after the TIFF");
        file.extend_from_slice(tail);
        file
    };
    // the big one with a start of scan after its frame header (sized_jpeg has none), as a real JPEG
    let mut big = big;
    big.splice(21..21, [0xff, 0xda, 0x00, 0x02]);
    let declared = big.len() as u32 + 64;
    // cut short: half of the big JPEG, no end marker
    let cut = with_tail(&big[..big.len() / 2], declared);
    assert_eq!(embedded_preview(&cut).as_deref(), Some(&small[..]), "the truncated JPEG is not taken");
    // whole, its length overstated past the file's end: still the largest
    let whole = with_tail(&big, declared);
    assert_eq!(embedded_preview(&whole).as_deref(), Some(&big[..]), "a whole JPEG with an overstated length is kept");
    // an end marker inside an APP1 payload is not the JPEG's end: cut short, still not taken
    let mut with_app = vec![0xff, 0xd8, 0xff, 0xe1, 0x00, 0x08, 0x12, 0xff, 0xd9, 0x34, 0x56, 0x78];
    with_app.extend_from_slice(&big[2..]);
    let declared = with_app.len() as u32 + 64;
    let cut = with_tail(&with_app[..with_app.len() / 2], declared);
    assert_eq!(embedded_preview(&cut).as_deref(), Some(&small[..]), "an FF D9 in APP1 doesn't make a cut JPEG whole");
    // whole, with other bytes after it up to the file's end: kept, cut at its own end marker
    let mut tail = with_app.clone();
    tail.extend_from_slice(&[0x11, 0xff, 0xd9, 0x22]);
    let whole = with_tail(&tail, declared);
    assert_eq!(embedded_preview(&whole).as_deref(), Some(&with_app[..]), "a whole JPEG is cut at its own end");
    // an end marker before any start of scan: no image data, not taken (CodeRabbit on storytold/lightcraft#3080)
    let unscanned = sized_jpeg(1600, 1200, 3, 4000);
    let whole = with_tail(&unscanned, unscanned.len() as u32 + 64);
    assert_eq!(embedded_preview(&whole).as_deref(), Some(&small[..]), "a JPEG without a scan is not taken");
}

/// The walk to a JPEG's end refuses what isn't one and never runs past the bytes it has.
#[test]
fn the_end_of_a_jpeg_is_its_own_end_marker() {
    let jpeg_end = |s: &[u8]| super::jpeg_end(s, &mut (1usize << 20));
    assert_eq!(jpeg_end(&[0x12, 0x34, 0xff, 0xd9]), None, "not a JPEG");
    assert_eq!(jpeg_end(&[0xff, 0xd8, 0xff, 0xd9]), None, "an end before any scan: no image data");
    // a stuffed zero and a restart marker in the scan data, then the end
    assert_eq!(jpeg_end(&[0xff, 0xd8, 0xff, 0xda, 0x00, 0x02, 0x55, 0xff, 0x00, 0xff, 0xd3, 0x66, 0xff, 0xd9]), Some(14));
    // a segment length of 0 or 1, or one running past the end
    assert_eq!(jpeg_end(&[0xff, 0xd8, 0xff, 0xe1, 0x00, 0x01, 0xff, 0xd9]), None);
    assert_eq!(jpeg_end(&[0xff, 0xd8, 0xff, 0xe1, 0xff, 0xff, 0xff, 0xd9]), None);
    for n in 0..12 {
        assert_eq!(jpeg_end(&[0xff, 0xd8, 0xff, 0xe1, 0x00, 0x04, 0xff, 0xd9, 0x55, 0xff, 0xff, 0xff][..n]), None, "{n} bytes");
    }
}

/// CodeRabbit on storytold/lightcraft#3080: the walks to a JPEG's end share one budget, so a file of many pointers past
/// its end can't make each walk run the whole file: a walk that would overspend it gives up.
#[test]
fn the_walks_to_a_jpegs_end_share_a_budget() {
    use super::jpeg_end;
    let mut j = vec![0xff, 0xd8, 0xff, 0xda, 0x00, 0x02];
    j.extend(std::iter::repeat_n(0x55u8, 1000));
    j.extend_from_slice(&[0xff, 0xd9]);
    let mut budget = 1500;
    assert_eq!(jpeg_end(&j, &mut budget), Some(j.len()), "the first walk fits");
    assert!(budget < 1500 - 1000, "and is charged for its steps: {budget} left");
    assert_eq!(jpeg_end(&j, &mut budget), None, "the second would overspend what is left");
    assert_eq!(budget, 0);
}

#[test]
fn picks_largest_dct_jpeg() {
    let small = fake_jpeg(10);
    let big = fake_jpeg(500);
    let lossless = crate::ljpeg::encode(&[1u16; 64], 8, 8, 1, 12, 1, 0);
    let mut ifd0 = IfdBuilder::new();
    ifd0.set(t::COMPRESSION, Value::Short(vec![6]));
    ifd0.set(t::IMAGE_WIDTH, Value::Long(vec![1]));
    ifd0.set(t::IMAGE_LENGTH, Value::Long(vec![1]));
    ifd0.set_image(ImageData::Strips { rows_per_strip: 1, strips: vec![small.clone()] });
    let mut raw = IfdBuilder::new();
    raw.set(t::COMPRESSION, Value::Short(vec![7]));
    raw.set(t::IMAGE_WIDTH, Value::Long(vec![8]));
    raw.set(t::IMAGE_LENGTH, Value::Long(vec![8]));
    raw.set_image(ImageData::Strips { rows_per_strip: 8, strips: vec![lossless] });
    ifd0.add_sub_ifd(raw);
    let mut sub = IfdBuilder::new();
    sub.set(t::COMPRESSION, Value::Short(vec![7]));
    sub.set(t::IMAGE_WIDTH, Value::Long(vec![2]));
    sub.set(t::IMAGE_LENGTH, Value::Long(vec![2]));
    sub.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![1]));
    sub.set_image(ImageData::Strips { rows_per_strip: 2, strips: vec![big.clone()] });
    ifd0.add_sub_ifd(sub);
    let bytes = TiffWriter::default().write(&[ifd0]).unwrap();
    assert_eq!(embedded_preview(&bytes).unwrap(), big);
    assert!(embedded_preview(b"nope").is_none());
    // every truncation is safe
    for n in 0..bytes.len() {
        let _ = embedded_preview(&bytes[..n]);
    }
}

/// A minimal RAF: the fixed header (JPEG offset/length at 84/88), the JPEG, then a raw tail the
/// caller must not have to read. `jpeg_len` is written as declared, so a test can claim more than
/// the JPEG holds the way a padded container does.
#[cfg(feature = "raf")]
fn raf_fixture(jpeg: &[u8], gap: usize, preamble: usize, declared_len: Option<usize>) -> Vec<u8> {
    let head = 160 + preamble;
    let jpeg_off = head;
    let mut f = vec![0u8; head];
    f[..16].copy_from_slice(b"FUJIFILMCCD-RAW ");
    f[84..88].copy_from_slice(&(jpeg_off as u32).to_be_bytes());
    f[88..92].copy_from_slice(&(declared_len.unwrap_or(jpeg.len()) as u32).to_be_bytes());
    // records directory and raw block are absent (offset 0), which `header` reads as "none"
    f.extend_from_slice(jpeg);
    f.extend(std::iter::repeat_n(0x33u8, gap));
    f
}

/// A RAF's preview pointer sits at a fixed offset, so a header-sized prefix locates it and the
/// caller reads just that range instead of the whole file — the whole point on a network share.
#[cfg(feature = "raf")]
#[test]
fn bounded_preview_locates_a_rafs_jpeg_from_the_header_alone() {
    let jpeg = fake_jpeg(5000);
    let file = raf_fixture(&jpeg, 40_000, 0, None);
    // A prefix that covers the pointer's range: header + JPEG, and nothing of the tail.
    let prefix_len = 8192.min(file.len());
    let prefix = &file[..prefix_len];

    let range = bounded_preview(prefix).expect("a RAF's preview is found from its header");
    assert_eq!(range.offset, 160);
    assert_eq!(range.window_len, jpeg.len() as u64);
    assert!(range.offset + range.window_len <= prefix_len as u64, "the prefix covers the whole range: {range:?} of {prefix_len}");
    assert!((range.offset + range.window_len) < file.len() as u64, "the range stops short of the tail: {range:?} of {}", file.len());

    // What the caller reads, and what it holds afterwards: identical to the whole-file answer.
    let window = prefix.get(range.offset as usize..(range.offset + range.window_len) as usize).expect("the range lies inside the prefix");
    let bounded = embedded_preview_from_range(window, range).expect("the range holds the preview");
    assert_eq!(Some(bounded.clone()), embedded_preview(&file));
    assert!(bounded.ends_with(&[0xff, 0xd9]), "the preview ends at its EOI marker");

    // A prefix that stops inside the declared extent is clamped by the container reader. The
    // caller then reads a short window; what comes back is the whole JPEG it holds (the EOI
    // marker is inside), never bytes past the window.
    let short = &file[..2000];
    let clipped = bounded_preview(short).expect("a pointer before the cut is still found");
    assert!(clipped.window_len < jpeg.len() as u64, "clamped: {clipped:?}");
    let window = short.get(clipped.offset as usize..).expect("inside");
    let preview = embedded_preview_from_range(window, clipped).expect("the bytes are a JPEG start");
    // No end-of-image marker inside the window: the bytes come back as they are, exactly as the
    // whole-file path returns an unclosed JPEG. The decoder refuses it later; nothing is invented.
    assert_eq!(preview, window, "an unclosed JPEG is handed on unchanged, never padded");
    assert!(preview.len() <= window.len(), "never past the window");
}

/// The preview pointer may sit past the prefix (a long maker-note record before it), and a
/// container may declare padding after the JPEG: both stay correct, and the padding is dropped.
#[cfg(feature = "raf")]
#[test]
fn bounded_preview_handles_a_late_pointer_and_padding() {
    let jpeg = fake_jpeg(300);
    let late = raf_fixture(&jpeg, 0, 9000, None);
    let range = bounded_preview(&late).expect("the whole file is always a valid prefix");
    assert_eq!(range.offset, (160 + 9000) as u64);
    let padded = raf_fixture(&jpeg, 500, 0, Some(jpeg.len() + 500));
    let range = bounded_preview(&padded).expect("a padded extent is still located");
    assert_eq!(range.window_len, (jpeg.len() + 500) as u64);
    let window = padded.get(range.offset as usize..(range.offset + range.window_len) as usize).expect("the window is inside the file");
    let preview = embedded_preview_from_range(window, range).expect("the padding is dropped");
    assert_eq!(preview, embedded_preview(&padded).expect("the whole-file answer agrees"));
    assert!(preview.ends_with(&[0xff, 0xd9]));
}

/// Without a usable header or pointer the caller must read the file whole: the answer is `None`,
/// never a wrong range.
#[cfg(feature = "raf")]
#[test]
fn bounded_preview_declines_what_the_header_cannot_answer() {
    let jpeg = fake_jpeg(64);
    let file = raf_fixture(&jpeg, 0, 0, None);
    assert_eq!(bounded_preview(&jpeg), None, "not a raw at all");
    assert_eq!(bounded_preview(b""), None);
    if let Some(short) = file.get(..64) {
        assert_eq!(bounded_preview(short), None, "a header cut short has not shown the pointer yet");
    }
    // a pointer that claims more than the file holds: `header` clamps the slice to the buffer,
    // so the range never runs past the end (no out-of-bounds read can follow from it)
    let mut lying = raf_fixture(&jpeg, 4096, 0, None);
    lying[88..92].copy_from_slice(&(u32::MAX / 2).to_be_bytes());
    let clamped = bounded_preview(&lying).expect("a pointer inside the file is still usable");
    assert!(clamped.offset + clamped.window_len <= lying.len() as u64, "clamped to the file: {clamped:?}");
    // a range the window cannot satisfy yields nothing
    assert_eq!(embedded_preview_from_range(&jpeg, PreviewRange { offset: 0, window_len: u64::MAX }), None);
    // a TIFF-based raw: its pointer tag lives in an IFD this prefix may cut short
    let mut tiff = b"II*\0\x08\0\0\0".to_vec();
    tiff.extend_from_slice(&[0u8; 64]);
    assert_eq!(bounded_preview(&tiff), None);
}
