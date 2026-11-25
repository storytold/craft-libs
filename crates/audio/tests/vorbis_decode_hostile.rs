//! Hostile input to the Vorbis decoder: damaged headers and packets, truncations, and headers that
//! ask for absurd sizes. Every case is an error or a result, never a panic or a large allocation.

use craft_audio::vorbis::decoder::{Decoder, Error, decode_ogg};
use craft_audio::{AudioBuffer, OggVorbisOptions, encode_ogg_vorbis};

fn xorshift(seed: &mut u32) -> u32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    *seed
}

struct Parts {
    ogg: Vec<u8>,
    headers: Vec<Vec<u8>>,
    packets: Vec<Vec<u8>>,
}

fn parts(channels: usize) -> Parts {
    let signal: Vec<Vec<f32>> = (0..channels)
        .map(|c| (0..20_000).map(|i| ((i as f32 * (0.03 + 0.01 * c as f32)).sin() * 0.4) + ((i * 7 % 13) as f32 - 6.0) / 60.0).collect())
        .collect();
    let ogg =
        encode_ogg_vorbis(&AudioBuffer { sample_rate: 44_100, channels: signal }, &OggVorbisOptions::default().with_quality(4.0)).expect("encode");
    let file = craft_media::ogg::open(ogg.as_slice()).expect("open");
    let s = &file.streams[0];
    let packets = (0..s.packets.len()).map(|i| file.read_packet(ogg.as_slice(), 0, i).expect("packet")).collect();
    Parts { headers: s.headers.clone(), packets, ogg }
}

fn decoder(p: &Parts) -> Decoder {
    Decoder::new(&p.headers[0], &p.headers[1], &p.headers[2]).expect("valid headers")
}

#[test]
fn every_truncated_header_is_an_error() {
    let p = parts(2);
    for which in [0usize, 2] {
        for n in 0..p.headers[which].len() {
            let mut h = p.headers.clone();
            h[which].truncate(n);
            assert!(Decoder::new(&h[0], &h[1], &h[2]).is_err(), "header {which} cut to {n} bytes");
        }
    }
    // A damaged comment header is not fatal.
    let mut h = p.headers.clone();
    h[1].truncate(12);
    let d = Decoder::new(&h[0], &h[1], &h[2]).expect("comments are best effort");
    assert!(d.comments().comments.is_empty());
    assert!(Decoder::new(&p.headers[0], &[], &p.headers[2]).is_err(), "the comment header still needs its tag");
}

#[test]
fn damaged_headers_never_panic() {
    let p = parts(2);
    let mut seed = 99u32;
    for which in [0usize, 2] {
        let len = p.headers[which].len();
        // CRAFT_HOSTILE_DIVISOR=N (pull requests) runs 1/N of the mutations.
        for _ in 0..800usize.div_ceil(craft_media_testkit::hostile::divisor()) {
            let mut h = p.headers.clone();
            for _ in 0..1 + xorshift(&mut seed) % 3 {
                let at = xorshift(&mut seed) as usize % len;
                h[which][at] = xorshift(&mut seed) as u8;
            }
            if let Ok(mut d) = Decoder::new(&h[0], &h[1], &h[2]) {
                // A setup that survives must also survive audio packets.
                for pk in p.packets.iter().take(6) {
                    let _ = d.decode_packet(pk);
                }
            }
        }
    }
}

#[test]
fn damaged_packets_never_panic() {
    let p = parts(2);
    let mut seed = 7u32;
    let mut d = decoder(&p);
    for pk in &p.packets {
        for _ in 0..6 {
            let mut bad = pk.clone();
            match xorshift(&mut seed) % 3 {
                0 => bad.truncate(xorshift(&mut seed) as usize % (bad.len() + 1)),
                1 => {
                    let at = xorshift(&mut seed) as usize % bad.len();
                    bad[at] = xorshift(&mut seed) as u8;
                }
                _ => bad = (0..xorshift(&mut seed) % 300).map(|_| xorshift(&mut seed) as u8).collect(),
            }
            let _ = d.decode_packet(&bad);
        }
    }
    // The decoder still decodes good packets after all that.
    d.reset();
    assert!(d.decode_packet(&p.packets[3]).is_ok());
}

#[test]
fn header_packets_and_empty_packets_are_not_audio() {
    let p = parts(1);
    let mut d = decoder(&p);
    assert_eq!(d.decode_packet(&p.headers[0]), Err(Error::NotAudio));
    assert_eq!(d.decode_packet(&p.headers[2]), Err(Error::NotAudio));
    assert!(matches!(d.decode_packet(&[]), Err(Error::Packet(_))));
}

#[test]
fn damaged_files_never_panic() {
    let p = parts(2);
    let mut seed = 31u32;
    for _ in 0..120 {
        let mut bad = p.ogg.clone();
        for _ in 0..1 + xorshift(&mut seed) % 4 {
            let at = xorshift(&mut seed) as usize % bad.len();
            bad[at] = xorshift(&mut seed) as u8;
        }
        let _ = decode_ogg(&bad);
        bad.truncate(xorshift(&mut seed) as usize % bad.len());
        let _ = decode_ogg(&bad);
    }
    assert!(matches!(decode_ogg(b"not an ogg file"), Err(Error::Ogg(_))));
    assert!(matches!(decode_ogg(&[]), Err(Error::Ogg(_))));
}

/// A bit writer for hand-built setups.
#[derive(Default)]
struct W(Vec<u8>, usize);
impl W {
    fn put(&mut self, v: u64, n: u32) {
        for i in 0..n {
            if self.1.is_multiple_of(8) {
                self.0.push(0);
            }
            let last = self.0.len() - 1;
            self.0[last] |= (((v >> i) & 1) as u8) << (self.1 % 8);
            self.1 += 1;
        }
    }
}

fn setup_with_first_book(book: impl FnOnce(&mut W)) -> Vec<u8> {
    let mut w = W::default();
    w.put(5, 8);
    for c in b"vorbis" {
        w.put(u64::from(*c), 8);
    }
    w.put(0, 8); // one book
    book(&mut w);
    w.0
}

#[test]
fn absurd_codebook_sizes_are_refused_before_allocating() {
    let p = parts(1);
    let try_setup = |setup: Vec<u8>| Decoder::new(&p.headers[0], &p.headers[1], &setup);
    // 2^24 - 1 entries.
    assert!(
        try_setup(setup_with_first_book(|w| {
            w.put(0x56_4342, 24);
            w.put(1, 16);
            w.put(0xFF_FFFF, 24);
            w.put(1, 1); // ordered
            w.put(0, 5);
        }))
        .is_err()
    );
    // An ordered book whose lengths overrun its entries.
    assert!(
        try_setup(setup_with_first_book(|w| {
            w.put(0x56_4342, 24);
            w.put(1, 16);
            w.put(8, 24);
            w.put(1, 1);
            w.put(0, 5);
            w.put(0xFF, 4); // asks for more codewords of length 1 than there are entries
        }))
        .is_err()
    );
    // A lookup table of 65535 dimensions over a million entries: 69 billion values.
    assert!(
        try_setup(setup_with_first_book(|w| {
            w.put(0x56_4342, 24);
            w.put(0xFFFF, 16);
            w.put(1 << 20, 24);
            w.put(1, 1); // ordered
            w.put(19, 5); // length 20
            w.put(1 << 20, 21); // all the entries
            w.put(2, 4); // lookup type 2
            w.put(0, 32);
            w.put(0, 32);
            w.put(0, 4);
            w.put(0, 1);
        }))
        .is_err()
    );
    // A million-entry book with a legitimate tree but a lookup table that is not in the packet.
    assert!(
        try_setup(setup_with_first_book(|w| {
            w.put(0x56_4342, 24);
            w.put(4, 16);
            w.put(1 << 20, 24);
            w.put(1, 1);
            w.put(19, 5);
            w.put(1 << 20, 21);
            w.put(1, 4); // lookup type 1
            w.put(0, 32);
            w.put(0, 32);
            w.put(15, 4);
            w.put(0, 1);
        }))
        .is_err()
    );
}

#[test]
fn identification_header_limits() {
    let p = parts(1);
    let mut h = p.headers[0].clone();
    let setup = &p.headers[2];
    h[11] = 0; // no channels
    assert!(matches!(Decoder::new(&h, &p.headers[1], setup), Err(Error::Header(_))));
    let mut h = p.headers[0].clone();
    h[28] = 0x95; // a short block size of 2^5 is below the 64 Vorbis allows
    assert!(matches!(Decoder::new(&h, &p.headers[1], setup), Err(Error::Header(_))));
    let mut h = p.headers[0].clone();
    h[7] = 1; // version 1
    assert!(matches!(Decoder::new(&h, &p.headers[1], setup), Err(Error::Header(_))));
}
