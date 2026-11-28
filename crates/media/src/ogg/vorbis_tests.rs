//! Vorbis modes and timing on synthetic headers and packets.

use super::vorbis::*;
use super::*;

/// Bits written least significant first, as Vorbis reads them.
#[derive(Default)]
struct BitWriter {
    bytes: Vec<u8>,
    bits: u64,
}

impl BitWriter {
    fn put(&mut self, value: u32, n: u32) -> &mut Self {
        for k in 0..n {
            if self.bits.is_multiple_of(8) {
                self.bytes.push(0);
            }
            let bit = ((value >> k) & 1) as u8;
            let last = self.bytes.len() - 1;
            self.bytes[last] |= bit << (self.bits % 8);
            self.bits += 1;
        }
        self
    }
}

/// An identification header: one channel at 44.1 kHz, blocks of 256 and 2048 samples.
fn identification() -> Vec<u8> {
    let mut h = b"\x01vorbis".to_vec();
    h.extend_from_slice(&0u32.to_le_bytes());
    h.push(1);
    h.extend_from_slice(&44_100u32.to_le_bytes());
    h.extend_from_slice(&[0; 12]);
    h.push(0xB8);
    h.push(1);
    h
}

/// The smallest setup header: one codebook, floor 1, residue 0 and mapping, then `modes` with the
/// given block flags.
fn setup(modes: &[bool]) -> Vec<u8> {
    setup_with(modes, Codebook { dims: 1, lookup: 0 }, 0, 0)
}

/// The codebook of [`setup_with`]: 2 entries of length 1 with `dims` dimensions and lookup type
/// `lookup` (0, or 1 with 1-bit values).
#[derive(Clone, Copy)]
struct Codebook {
    dims: u32,
    lookup: u32,
}

/// [`setup`] with the given codebook and the floor and residue its one mapping's submap names.
fn setup_with(modes: &[bool], book: Codebook, floor: u32, residue: u32) -> Vec<u8> {
    let plain_floor = |w: &mut BitWriter| {
        // one floor 1 with no partitions: multiplier and range bits
        w.put(0, 6).put(1, 16).put(0, 5).put(0, 2).put(0, 4);
    };
    let plain_residue = |w: &mut BitWriter| {
        // one residue 0 with one classification, classbook 0 and no books
        w.put(0, 6).put(0, 16).put(0, 24).put(0, 24).put(0, 24).put(0, 6).put(0, 8).put(0, 3).put(0, 1);
    };
    setup_parts(modes, book, floor, residue, &plain_floor, &plain_residue)
}

/// [`setup_with`] with the floor and residue configurations written by the given closures, each
/// starting with its count of 0 (one entry).
fn setup_parts(
    modes: &[bool],
    book: Codebook,
    floor: u32,
    residue: u32,
    write_floor: &dyn Fn(&mut BitWriter),
    write_residue: &dyn Fn(&mut BitWriter),
) -> Vec<u8> {
    let mut w = BitWriter::default();
    // one codebook: 2 entries of length 1
    w.put(0, 8).put(0x56_4342, 24).put(book.dims, 16).put(2, 24).put(0, 1).put(0, 1).put(0, 5).put(0, 5).put(book.lookup, 4);
    if book.lookup == 1 {
        // minimum value, delta, 1-bit values, no sequence flag, then the values themselves
        w.put(0, 32).put(0, 32).put(0, 4).put(0, 1);
        let values = if book.dims == 0 { 0 } else { lookup1_values(2, book.dims) };
        for _ in 0..values {
            w.put(0, 1);
        }
    }
    // one time-domain transform
    w.put(0, 6).put(0, 16);
    write_floor(&mut w);
    write_residue(&mut w);
    // one mapping: one submap, no coupling, reserved bits, the submap's time, floor and residue
    w.put(0, 6).put(0, 16).put(0, 1).put(0, 1).put(0, 2).put(0, 8).put(floor, 8).put(residue, 8);
    w.put(modes.len() as u32 - 1, 6);
    for &long in modes {
        w.put(u32::from(long), 1).put(0, 16).put(0, 16).put(0, 8);
    }
    w.put(1, 1);
    let mut h = b"\x05vorbis".to_vec();
    h.extend_from_slice(&w.bytes);
    h
}

const SHORT: u8 = 0b00;
const LONG: u8 = 0b10;

/// A Vorbis stream whose packets are laid out in `pages`: each page is its packets' first bytes and
/// the page's granule position; the last page is the end of the stream.
fn stream(pages: &[(&[u8], i64)]) -> Stream {
    let mut packets = Vec::new();
    for (k, &(firsts, granule)) in pages.iter().enumerate() {
        for (i, &first) in firsts.iter().enumerate() {
            let last = i + 1 == firsts.len();
            packets.push(Packet {
                parts: Vec::new(),
                size: 10,
                granule: last.then_some(granule),
                page: k as u32,
                eos: k + 1 == pages.len(),
                prefix: [first, 0],
            });
        }
    }
    Stream {
        serial: 1,
        codec: Codec::Vorbis,
        // three modes, so mode 3 is one the header does not have
        headers: vec![identification(), b"\x03vorbis".to_vec(), setup(&[false, true, true])],
        packets,
        pages: pages.len() as u32,
        last_granule: pages.last().map(|p| p.1),
        saw_eos: true,
        header_count: 3,
        header_bytes: 0,
        next_seq: None,
        started: true,
    }
}

#[test]
fn modes_come_from_the_end_of_the_setup_header() {
    let modes = VorbisModes::parse(&identification(), &setup(&[false, true, true])).unwrap();
    assert_eq!(modes.blocksizes, [256, 2048]);
    assert_eq!(modes.long, [false, true, true]);
    // two bits of mode number after the packet-type bit
    assert_eq!(modes.blocksize(0b000), Some(256));
    assert_eq!(modes.blocksize(0b100), Some(2048));
    assert_eq!(modes.blocksize(0b110), None, "a mode the header does not have");
    assert_eq!(modes.blocksize(0b001), None, "not an audio packet");
}

#[test]
fn damaged_headers_give_no_modes() {
    let id = identification();
    let full = setup(&[false, true]);
    for cut in 7..full.len() {
        assert_eq!(VorbisModes::parse(&id, &full[..cut]), None, "setup cut to {cut} bytes");
    }
    let mut no_framing = full.clone();
    *no_framing.last_mut().unwrap() = 0;
    assert_eq!(VorbisModes::parse(&id, &no_framing), None);
    let mut long_first = id.clone();
    long_first[28] = 0x8B;
    assert_eq!(VorbisModes::parse(&long_first, &full), None, "short block larger than the long one");
    let mut version_1 = id.clone();
    version_1[7] = 1;
    assert_eq!(VorbisModes::parse(&version_1, &full), None, "not Vorbis I");
}

#[test]
fn a_zero_sample_rate_gives_no_modes() {
    let full = setup(&[false, true]);
    assert!(VorbisModes::parse(&identification(), &full).is_some());
    let mut no_rate = identification();
    no_rate[12..16].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(VorbisModes::parse(&no_rate, &full), None);
}

#[test]
fn a_mapping_names_only_configured_floors_and_residues() {
    let id = identification();
    let plain = Codebook { dims: 1, lookup: 0 };
    assert!(VorbisModes::parse(&id, &setup_with(&[false, true], plain, 0, 0)).is_some());
    assert_eq!(VorbisModes::parse(&id, &setup_with(&[false, true], plain, 1, 0)), None, "floor 1 of one floor");
    assert_eq!(VorbisModes::parse(&id, &setup_with(&[false, true], plain, 0, 1)), None, "residue 1 of one residue");
}

/// The setup of [`setup_with`] with one floor 1 (partition list, then classes) and one residue.
fn books_setup(floor: &dyn Fn(&mut BitWriter), residue: &dyn Fn(&mut BitWriter)) -> Option<VorbisModes> {
    let plain = Codebook { dims: 1, lookup: 0 };
    VorbisModes::parse(&identification(), &setup_parts(&[false, true], plain, 0, 0, floor, residue))
}

#[test]
fn residue_books_name_configured_codebooks() {
    let residue = |classbook: u32, book: u32| {
        move |w: &mut BitWriter| {
            // one classification with the first cascade bit set, which names one book
            w.put(0, 6).put(0, 16).put(0, 24).put(0, 24).put(0, 24).put(0, 6).put(classbook, 8).put(1, 3).put(0, 1);
            w.put(book, 8);
        }
    };
    let floor = |w: &mut BitWriter| {
        w.put(0, 6).put(1, 16).put(0, 5).put(0, 2).put(0, 4);
    };
    assert!(books_setup(&floor, &residue(0, 0)).is_some());
    assert_eq!(books_setup(&floor, &residue(1, 0)), None, "classbook 1 of one codebook");
    assert_eq!(books_setup(&floor, &residue(0, 1)), None, "book 1 of one codebook");
}

#[test]
fn floor_books_name_configured_codebooks() {
    let residue = |w: &mut BitWriter| {
        w.put(0, 6).put(0, 16).put(0, 24).put(0, 24).put(0, 24).put(0, 6).put(0, 8).put(0, 3).put(0, 1);
    };
    // floor 0: order, rate, bark map size, amplitude bits and offset, one book
    let floor0 = |book: u32| {
        move |w: &mut BitWriter| {
            w.put(0, 6).put(0, 16).put(0, 8).put(0, 16).put(0, 16).put(0, 6).put(0, 8).put(0, 4).put(book, 8);
        }
    };
    assert!(books_setup(&floor0(0), &residue).is_some());
    assert_eq!(books_setup(&floor0(1), &residue), None);
    // floor 1: one partition of class 0 with one dimension and 1 subclass bit, a master book and
    // two subclass books (number plus one), then 2 range bits and one X value
    let floor1 = |master: u32, sub: u32, x: u32| {
        move |w: &mut BitWriter| {
            w.put(0, 6).put(1, 16).put(1, 5).put(0, 4).put(0, 3).put(1, 2).put(master, 8).put(sub, 8).put(0, 8);
            w.put(0, 2).put(2, 4).put(x, 2);
        }
    };
    assert!(books_setup(&floor1(0, 1, 1), &residue).is_some());
    assert!(books_setup(&floor1(0, 0, 1), &residue).is_some(), "no subclass book");
    assert_eq!(books_setup(&floor1(1, 1, 1), &residue), None, "master book 1");
    assert_eq!(books_setup(&floor1(0, 2, 1), &residue), None, "subclass book 1");
    assert_eq!(books_setup(&floor1(0, 1, 0), &residue), None, "X value equal to the implicit 0");
    assert_eq!(books_setup(&floor1(0, 1, 1), &residue).map(|m| m.long), Some(vec![false, true]));
}

#[test]
fn floor_one_x_values_are_distinct() {
    let residue = |w: &mut BitWriter| {
        w.put(0, 6).put(0, 16).put(0, 24).put(0, 24).put(0, 24).put(0, 6).put(0, 8).put(0, 3).put(0, 1);
    };
    // one partition of class 0 with two dimensions and no subclasses, 3 range bits, two X values
    let floor1 = |a: u32, b: u32| {
        move |w: &mut BitWriter| {
            w.put(0, 6).put(1, 16).put(1, 5).put(0, 4).put(1, 3).put(0, 2).put(0, 8);
            w.put(0, 2).put(3, 4).put(a, 3).put(b, 3);
        }
    };
    assert!(books_setup(&floor1(3, 5), &residue).is_some());
    assert_eq!(books_setup(&floor1(3, 3), &residue), None, "repeated X value");
    assert_eq!(books_setup(&floor1(0, 5), &residue), None, "X value equal to the implicit 0");
}

#[test]
fn a_codebook_of_no_dimensions_has_no_lookup_values() {
    let id = identification();
    for lookup in [0, 1] {
        let modes = VorbisModes::parse(&id, &setup_with(&[false, true], Codebook { dims: 0, lookup }, 0, 0));
        assert_eq!(modes.map(|m| m.long), Some(vec![false, true]), "lookup type {lookup}");
        let one = VorbisModes::parse(&id, &setup_with(&[false, true], Codebook { dims: 1, lookup }, 0, 0));
        assert_eq!(one.map(|m| m.long), Some(vec![false, true]), "lookup type {lookup}, one dimension");
    }
}

#[test]
fn the_first_packet_primes_and_the_rest_overlap_by_quarter_blocks() {
    let t = VorbisTiming::of(&stream(&[(&[SHORT, LONG], 576), (&[LONG, LONG, SHORT], 3200)])).unwrap();
    assert_eq!(t.durations, [0, 64 + 512, 1024, 1024, 512 + 64]);
    assert_eq!(t.starts, [0, 0, 576, 1600, 2624]);
    assert_eq!((t.skip, t.first, t.total), (0, 0, 3200));
}

#[test]
fn a_negative_start_discards_leading_samples() {
    let t = VorbisTiming::of(&stream(&[(&[SHORT, LONG], 476), (&[LONG, SHORT], 2076)])).unwrap();
    assert_eq!(t.starts[1], -100);
    assert_eq!((t.skip, t.first), (100, 0));
    assert_eq!(t.total, 2076);
}

#[test]
fn packets_that_are_not_audio_do_not_shrink_the_leading_discard() {
    let t = VorbisTiming::of(&stream(&[(&[0xFF, SHORT, LONG], 476), (&[LONG, SHORT], 2076)])).unwrap();
    assert_eq!(&t.durations[..3], [0, 0, 576]);
    assert_eq!(t.starts[2], -100);
    assert_eq!((t.skip, t.first, t.total), (100, 0, 2076));
}

#[test]
fn a_packet_too_short_for_its_window_flags_returns_nothing() {
    // 64 modes: six bits of mode number, so a long block's two window flags reach the second byte
    let mut modes = vec![false; 64];
    modes[1] = true;
    let mut s = stream(&[(&[SHORT, LONG, SHORT], 1000)]);
    s.headers[2] = setup(&modes);
    s.packets[1].size = 1;
    let t = VorbisTiming::of(&s).unwrap();
    assert_eq!(t.durations, [0, 0, 128], "the cut long packet is skipped and the next one follows the first");
    s.packets[1].size = 2;
    let t = VorbisTiming::of(&s).unwrap();
    assert_eq!(t.durations, [0, 64 + 512, 512 + 64]);
}

#[test]
fn a_positive_start_is_a_time_offset() {
    let t = VorbisTiming::of(&stream(&[(&[SHORT, LONG], 44_676), (&[LONG, SHORT], 46_276)])).unwrap();
    assert_eq!((t.skip, t.first), (0, 44_100));
    assert_eq!(t.starts[1], 44_100);
    assert_eq!(t.total, 46_276 - 44_100);
}

#[test]
fn a_page_with_only_the_priming_packet_does_not_place_the_stream() {
    let negative = VorbisTiming::of(&stream(&[(&[SHORT], 0), (&[LONG], 476), (&[LONG, SHORT], 2076)])).unwrap();
    assert_eq!(negative.starts[1], -100);
    assert_eq!((negative.skip, negative.first, negative.total), (100, 0, 2076));
    assert!(!negative.first_damaged, "a priming page above a negative start is not damage");
    assert_eq!(negative.starts[0], 0);
    let offset = VorbisTiming::of(&stream(&[(&[SHORT], 0), (&[LONG], 44_676), (&[LONG, SHORT], 46_276)])).unwrap();
    assert_eq!(offset.starts[1], 44_100);
    assert_eq!((offset.skip, offset.first, offset.total), (0, 44_100, 2176));
}

#[test]
fn the_final_page_trims_the_end() {
    let full = VorbisTiming::of(&stream(&[(&[SHORT, LONG], 576), (&[LONG, SHORT], 2176)])).unwrap();
    let trimmed = VorbisTiming::of(&stream(&[(&[SHORT, LONG], 576), (&[LONG, SHORT], 2000)])).unwrap();
    assert_eq!(trimmed.starts, full.starts, "trimming moves no packet");
    assert_eq!(trimmed.total, full.total - 176);
}

#[test]
fn one_page_shorter_than_its_packets_trims_the_end() {
    let t = VorbisTiming::of(&stream(&[(&[SHORT, LONG, SHORT], 1000)])).unwrap();
    assert_eq!((t.skip, t.first, t.total), (0, 0, 1000));
    assert_eq!(t.starts[1], 0);
}

#[test]
fn a_first_page_above_the_next_page_ends_where_it_starts() {
    let t = VorbisTiming::of(&stream(&[(&[SHORT, LONG], 5000), (&[LONG, LONG], 2624), (&[SHORT], 3200)])).unwrap();
    assert!(t.first_damaged);
    assert_eq!(t.starts, [0, 0, 576, 1600, 2624]);
    assert_eq!((t.skip, t.first, t.total), (0, 0, 3200));
    let sound = VorbisTiming::of(&stream(&[(&[SHORT, LONG], 576), (&[LONG, LONG], 2624), (&[SHORT], 3200)])).unwrap();
    assert!(!sound.first_damaged);
}

#[test]
fn a_first_page_above_the_final_page_is_placed_from_zero() {
    let t = VorbisTiming::of(&stream(&[(&[SHORT, LONG], 5000), (&[LONG, LONG, SHORT], 2500)])).unwrap();
    assert!(t.first_damaged);
    assert_eq!(t.starts, [0, 0, 576, 1600, 2624]);
    assert_eq!((t.skip, t.first, t.total), (0, 0, 2500), "the final page still trims the end");
}

#[test]
fn a_priming_page_does_not_hide_a_first_page_above_the_final_page() {
    let s = stream(&[(&[SHORT], 0), (&[LONG, LONG], 5000), (&[LONG, SHORT], 2500)]);
    let mut given = [0, 576, 1024, 1024, 576].into_iter();
    let t = VorbisTiming::of_with_durations(&s, |_| given.next().unwrap_or(0));
    assert!(t.first_damaged);
    assert_eq!(t.starts, [0, 0, 576, 1600, 2624]);
    assert_eq!((t.skip, t.first, t.total), (0, 0, 2500));
}

#[test]
fn given_durations_place_packets_as_the_block_sizes_do() {
    let s = stream(&[(&[SHORT, LONG], 476), (&[LONG, LONG, SHORT], 3100)]);
    let mut given = [0, 576, 1024, 1024, 576].into_iter();
    let t = VorbisTiming::of_with_durations(&s, |_| given.next().unwrap_or(0));
    assert_eq!(t, VorbisTiming::of(&s).unwrap());
    // a concealed packet keeps the duration the decoder gives it
    let mut concealed = [0, 576, 1024, 1024, 576].into_iter();
    let mut s = s;
    s.packets[3].prefix = [0xFF, 0];
    let t = VorbisTiming::of_with_durations(&s, |_| concealed.next().unwrap_or(0));
    assert_eq!(t.durations[3], 1024);
    assert_eq!(t.starts, [-100, -100, 476, 1500, 2524]);
}

#[test]
fn hostile_granules_and_packets_do_not_overflow() {
    for g in [i64::MIN, -2, i64::MAX, MAX_OPUS_GRANULE] {
        let t = VorbisTiming::of(&stream(&[(&[SHORT, LONG], g), (&[0xFF, 0b110, LONG], g)])).unwrap();
        assert_eq!(t.starts.len(), 5);
        assert_eq!(&t.durations[2..4], [0, 0], "a non-audio packet and an unknown mode return nothing");
        assert!(t.total >= 0);
    }
    let mut no_setup = stream(&[(&[SHORT], 0)]);
    no_setup.headers.truncate(2);
    assert_eq!(VorbisTiming::of(&no_setup), None);
}

#[test]
fn lookup_values_are_the_integer_root_of_the_entries() {
    assert_eq!(lookup1_values(81, 4), 3);
    assert_eq!(lookup1_values(80, 4), 2);
    assert_eq!(lookup1_values(1 << 24, 1), 1 << 24);
    assert_eq!(lookup1_values(1 << 24, 65_535), 1);
}
