//! SoundFont 2 (`.sf2`) reader, written from the SoundFont 2.04 Technical Specification (E-mu
//! Systems / Creative Technology, 2006). A file is a RIFF `sfbk` form of three lists: `INFO` (the
//! name is read from it), `sdta` (`smpl`: every sample end to end, 16-bit mono) and `pdta` (the
//! presets `phdr`/`pbag`/`pgen`, the instruments `inst`/`ibag`/`igen` and the sample headers
//! `shdr`).
//!
//! A preset is a list of zones that each point at an instrument; an instrument is a list of zones
//! that each point at a sample. A zone holds generators (numbered settings), and a first zone
//! that points at nothing is global: the default for the zones after it. Instrument generators
//! are absolute, preset generators are added to them, and key and velocity ranges intersect.
//! Every pair of a preset zone and an instrument zone that can sound becomes one [`Region`].
//!
//! Honoured: keyRange, velRange, instrument, sampleID, overridingRootKey, coarseTune and fineTune
//! (with the sample's own pitch correction), sampleModes (loop or not), the start, end and loop
//! address offsets (fine and coarse), pan, initialAttenuation and the volume envelope's attack,
//! decay, sustain and release. Ignored: modulators (`pmod`, `imod`), the filter, the modulation
//! envelope and both LFOs, the chorus and reverb sends, the volume envelope's delay, hold and key
//! scaling, scaleTuning, exclusiveClass, the fixed key and velocity, the 24-bit extension
//! (`sm24`) and samples in ROM. A file whose chunks run past its end is rejected, not played in part.
//!
//! Input is hostile: every offset is checked against the file, the sample pool is capped at
//! [`MAX_POOL_FRAMES`], and a file that points many zones at the same long lists is refused
//! ([`Error::TooManyZones`]) rather than multiplied out. Malformed input is an error, never a panic.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::cell::Cell;

#[cfg(any(test, feature = "test-support"))]
pub mod fixture;

/// The sample pool is read up to this many frames (about 1.5 hours at 48 kHz); regions are
/// clamped to what was read.
pub const MAX_POOL_FRAMES: usize = 1 << 28;

/// Why a file is not a SoundFont that can be played.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Not a SoundFont 2 file, or a damaged one.
    #[error("{0}")]
    Malformed(String),
    /// More zones than a font may have (a file that multiplies its zones).
    #[error("the SoundFont has too many zones")]
    TooManyZones,
    /// Nothing in the file can sound.
    #[error("the SoundFont holds no playable sample")]
    Empty,
}

/// A volume envelope: times in seconds, sustain as a linear level (0..=1).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Envelope {
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}

/// One stretch of the sample pool, as a preset plays it over a range of keys and velocities.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    /// The lowest and highest key, and the lowest and highest velocity (0..=127), that sound it.
    pub keys: (u8, u8),
    pub velocities: (u8, u8),
    /// The pool frames it plays, end exclusive, and the frames it loops over while the note is held.
    pub span: (usize, usize),
    pub looped: Option<(usize, usize)>,
    /// The rate the sample was recorded at, in Hz.
    pub sample_rate: u32,
    /// The (fractional) key that plays the sample at that rate.
    pub root: f32,
    /// -1 (left) to 1 (right).
    pub pan: f32,
    /// Linear gain, at most 1.
    pub gain: f32,
    pub envelope: Envelope,
}

/// A preset (a patch): its bank and program number and the regions it sounds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Preset {
    pub name: String,
    pub bank: u16,
    pub program: u16,
    pub regions: Vec<Region>,
}

/// A SoundFont as a player needs it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SoundFont {
    /// The `INAM` name of the file (empty when it has none).
    pub name: String,
    /// Every sample of the file end to end, mono, at full scale ±1; the regions say which frames
    /// are whose and at what rate.
    pub pool: Vec<f32>,
    /// Ordered by bank, then program.
    pub presets: Vec<Preset>,
}

fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Record sizes of the `pdta` chunks, in bytes.
const PHDR: usize = 38;
const BAG: usize = 4;
const GEN: usize = 4;
const INST: usize = 22;
const SHDR: usize = 46;

/// Generator numbers (specification section 8.1.2); `GENERATORS` is one past the last defined.
const START: usize = 0;
const END: usize = 1;
const LOOP_START: usize = 2;
const LOOP_END: usize = 3;
const START_COARSE: usize = 4;
const END_COARSE: usize = 12;
const PAN: usize = 17;
const ATTACK: usize = 34;
const DECAY: usize = 36;
const SUSTAIN: usize = 37;
const RELEASE: usize = 38;
const INSTRUMENT: usize = 41;
const KEY_RANGE: usize = 43;
const VEL_RANGE: usize = 44;
const LOOP_START_COARSE: usize = 45;
const ATTENUATION: usize = 48;
const LOOP_END_COARSE: usize = 50;
const COARSE_TUNE: usize = 51;
const FINE_TUNE: usize = 52;
const SAMPLE_ID: usize = 53;
const SAMPLE_MODES: usize = 54;
const ROOT_KEY: usize = 58;
const GENERATORS: usize = 59;

/// The `sfSampleType` bit of a sample that lives in ROM, not in the file.
const ROM: usize = 0x8000;
/// A hostile file can point every zone of every preset at the same long lists: reading stops
/// after this many generators, and a file may not make more regions than this.
const MAX_WORK: usize = 1 << 24;
const MAX_REGIONS: usize = 1 << 16;

/// The generators of one zone, by number.
type Zone = [Option<i16>; GENERATORS];

/// The `N` bytes at `at`; zeros when they are not there.
fn le<const N: usize>(bytes: &[u8], at: usize) -> [u8; N] {
    bytes.get(at..).and_then(|b| b.first_chunk()).copied().unwrap_or([0; N])
}

fn u16_at(bytes: &[u8], at: usize) -> usize {
    usize::from(u16::from_le_bytes(le(bytes, at)))
}

fn u32_at(bytes: &[u8], at: usize) -> usize {
    u32::from_le_bytes(le(bytes, at)) as usize
}

/// A fixed-size, NUL-terminated name.
fn name(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes.split(|b| *b == 0).next().unwrap_or_default()).trim().to_string()
}

/// The body of the first chunk called `id` in `data`, a run of RIFF chunks. With `form` it is the
/// first whose body starts with those four bytes (a `RIFF` or `LIST` of that type), without them.
fn chunk<'a>(mut data: &'a [u8], id: &[u8; 4], form: Option<&[u8; 4]>) -> Option<&'a [u8]> {
    loop {
        let size = u32_at(data.get(..8)?, 4);
        let body = data.get(8..)?.get(..size)?;
        match form {
            None if data.starts_with(id) => return Some(body),
            Some(form) if data.starts_with(id) && body.starts_with(form) => return body.get(4..),
            // Chunks are padded to an even length.
            _ => data = data.get(8..)?.get(size.checked_add(size & 1)?..)?,
        }
    }
}

/// The zones `from..to` of a bag chunk that point at something (their generator `link` says at
/// what), each laid over the global zone before it.
fn zones<'a>(
    bags: &'a [u8],
    gens: &'a [u8],
    (from, to): (usize, usize),
    link: usize,
    work: &'a Cell<usize>,
) -> impl Iterator<Item = (usize, Zone)> + 'a {
    // A zone's generators run from its bag's index to the next bag's.
    let bag = move |i: usize| bags.get(i.checked_mul(BAG)?..).filter(|b| b.len() >= BAG).map(|b| u16_at(b, 0));
    let mut global: Zone = [None; GENERATORS];
    (from..to)
        .map_while(move |i| {
            let (first, end) = (bag(i)?, bag(i + 1)?);
            let count = end.saturating_sub(first);
            work.set(work.get().saturating_sub(count + 1));
            if work.get() == 0 {
                return None;
            }
            let mut zone: Zone = [None; GENERATORS];
            for generator in gens.get(first * GEN..)?.as_chunks::<GEN>().0.iter().take(count) {
                if let Some(slot) = zone.get_mut(u16_at(generator, 0)) {
                    *slot = Some(i16::from_le_bytes(le(generator, 2)));
                }
            }
            Some(zone)
        })
        .enumerate()
        .filter_map(move |(n, zone)| match zone.get(link).copied().flatten() {
            Some(target) => Some((usize::from(target as u16), std::array::from_fn(|g| zone[g].or(global[g])))),
            None => {
                // Only a first zone is global; a later one that points at nothing means nothing.
                if n == 0 {
                    global = zone;
                }
                None
            }
        })
}

/// The region an instrument zone makes inside a preset zone with the sample `header` describes,
/// if it can sound. `pool` is the length of the sample pool in frames.
fn region(preset: &Zone, instrument: &Zone, header: &[u8], pool: usize) -> Option<Region> {
    let own = |g: usize| i64::from(instrument[g].unwrap_or(0));
    let sum = |g: usize, default: i16| i32::from(instrument[g].unwrap_or(default)) + i32::from(preset[g].unwrap_or(0));
    let range = |g: usize| {
        let [(low, high), (from, to)] = [instrument, preset].map(|zone| zone[g].map_or((0, 127), |v| (v.to_le_bytes()[0], v.to_le_bytes()[1])));
        (low.max(from), high.min(to))
    };
    // An address in the pool: the header's, moved by the zone's fine and coarse (32768 frames) offsets.
    let address =
        |field: usize, fine: usize, coarse: usize| (u32_at(header, field) as i64 + own(fine) + 32768 * own(coarse)).clamp(0, pool as i64) as usize;
    let (keys, velocities) = (range(KEY_RANGE), range(VEL_RANGE));
    let span = (address(20, START, START_COARSE), address(24, END, END_COARSE));
    let around = (address(28, LOOP_START, LOOP_START_COARSE), address(32, LOOP_END, LOOP_END_COARSE));
    let sample_rate = u32::from_le_bytes(le(header, 36));
    if keys.0 > keys.1 || velocities.0 > velocities.1 || span.0 >= span.1 || sample_rate == 0 || u16_at(header, 44) & ROM != 0 {
        return None;
    }
    // Modes 1 and 3 loop; 3 would play on past the loop once released, which the envelope cuts short here.
    let looped = (own(SAMPLE_MODES) & 1 == 1 && span.0 <= around.0 && around.0 < around.1 && around.1 <= span.1).then_some(around);
    let [pitch, correction] = le(header, 40);
    let root = match (instrument[ROOT_KEY], pitch) {
        (Some(key @ 0..=127), _) => f32::from(key),
        (_, key @ 0..=127) => f32::from(key),
        // 255 marks an unpitched sample.
        _ => 60.0,
    };
    let cents = sum(COARSE_TUNE, 0) * 100 + sum(FINE_TUNE, 0) + i32::from(correction as i8);
    // Times are in timecents (1200 per doubling of a second), levels in centibels of attenuation.
    let secs = |g: usize| 2f32.powf(sum(g, -12000).clamp(-12000, 8000) as f32 / 1200.0);
    let level = |centibels: i32| db_to_gain(-(centibels.clamp(0, 1440) as f32) / 10.0);
    Some(Region {
        keys,
        velocities,
        span,
        looped,
        sample_rate,
        root: root - cents as f32 / 100.0,
        pan: (sum(PAN, 0) as f32 / 500.0).clamp(-1.0, 1.0),
        gain: level(sum(ATTENUATION, 0)),
        envelope: Envelope { attack: secs(ATTACK), decay: secs(DECAY), sustain: level(sum(SUSTAIN, 0)), release: secs(RELEASE) },
    })
}

/// The chunk `id` of a list, which a SoundFont must have.
fn need<'a>(list: Option<&'a [u8]>, id: &[u8; 4]) -> Result<&'a [u8], Error> {
    list.and_then(|l| chunk(l, id, None)).ok_or_else(|| Error::Malformed(format!("the SoundFont has no '{}' chunk", String::from_utf8_lossy(id))))
}

/// Reads a SoundFont 2 file. See the module documentation for what is honoured.
pub fn read(bytes: &[u8]) -> Result<SoundFont, Error> {
    read_within(bytes, MAX_POOL_FRAMES)
}

fn read_within(bytes: &[u8], max_frames: usize) -> Result<SoundFont, Error> {
    let riff = chunk(bytes, b"RIFF", Some(b"sfbk")).ok_or_else(|| Error::Malformed("not a SoundFont 2 file, or one cut short".into()))?;
    let list = |kind: &[u8; 4]| chunk(riff, b"LIST", Some(kind));
    let pdta = list(b"pdta");
    let smpl = need(list(b"sdta"), b"smpl")?;
    let (phdr, pbag, pgen) = (need(pdta, b"phdr")?, need(pdta, b"pbag")?, need(pdta, b"pgen")?);
    let (inst, ibag, igen) = (need(pdta, b"inst")?, need(pdta, b"ibag")?, need(pdta, b"igen")?);
    let headers: Vec<&[u8]> = need(pdta, b"shdr")?.as_chunks::<SHDR>().0.iter().map(|h| &h[..]).collect();
    let instruments: Vec<&[u8]> = inst.as_chunks::<INST>().0.iter().map(|i| &i[..]).collect();
    let pool: Vec<f32> = smpl.as_chunks::<2>().0.iter().take(max_frames).map(|s| f32::from(i16::from_le_bytes(*s)) / 32768.0).collect();

    let (work, mut made) = (Cell::new(MAX_WORK), 0usize);
    let mut presets = Vec::new();
    // Every list ends with a terminal record, which is there to say where the last real one ends.
    for pair in phdr.as_chunks::<PHDR>().0.windows(2) {
        let [head, next] = pair else { continue };
        let (program, bank) = (u16::from_le_bytes(le(head, 20)), u16::from_le_bytes(le(head, 22)));
        let mut preset = Preset { name: name(head.get(..20).unwrap_or_default()), bank, program, regions: Vec::new() };
        for (target, zone) in zones(pbag, pgen, (u16_at(head, 24), u16_at(next, 24)), INSTRUMENT, &work) {
            let (Some(from), Some(to)) = (instruments.get(target), instruments.get(target + 1)) else { continue };
            for (sample, own) in zones(ibag, igen, (u16_at(from, 20), u16_at(to, 20)), SAMPLE_ID, &work) {
                preset.regions.extend(headers.get(sample).and_then(|header| region(&zone, &own, header, pool.len())));
            }
            if work.get() == 0 || made + preset.regions.len() > MAX_REGIONS {
                return Err(Error::TooManyZones);
            }
        }
        made += preset.regions.len();
        presets.push(preset);
    }
    if presets.iter().all(|p| p.regions.is_empty()) {
        return Err(Error::Empty);
    }
    presets.sort_by_key(|p| (p.bank, p.program));
    let title = list(b"INFO").and_then(|info| chunk(info, b"INAM", None)).map(name).unwrap_or_default();
    Ok(SoundFont { name: title, pool, presets })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{Sample, build, range};

    /// xorshift32: the same bytes on every run.
    struct Noise(u32);

    impl Noise {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            self.0 as usize % n
        }
    }

    fn tiny() -> Vec<u8> {
        let a = Sample {
            data: (0..100).map(|i| i * 300).collect(),
            rate: 22050,
            pitch: 60,
            gens: vec![(43, range(0, 63)), (54, 1), (2, 10), (3, -10), (17, -250), (48, 200), (37, 200), (38, 1200)],
        };
        let b = Sample { data: vec![-32768; 80], rate: 44100, pitch: 72, gens: vec![(43, range(64, 127)), (58, 70), (51, 1), (52, -50)] };
        build(&[a, b], &[(52, 25), (44, range(0, 100))], 1)
    }

    #[test]
    fn a_tiny_font_becomes_its_regions() {
        let font = read(&tiny()).unwrap();
        assert_eq!((font.name.as_str(), font.presets.len(), font.pool.len()), ("Tiny", 1, 100 + 46 + 80 + 46));
        assert_eq!((font.pool[1], font.pool[146]), (300.0 / 32768.0, -1.0));
        let preset = &font.presets[0];
        assert_eq!((preset.name.as_str(), preset.bank, preset.program, preset.regions.len()), ("Test", 0, 5, 2));
        let near = |a: f32, b: f32| (a - b).abs() < 1e-3 * b.abs().max(1.0);

        let low = &preset.regions[0];
        assert_eq!((low.keys, low.velocities, low.span, low.looped, low.sample_rate), ((0, 63), (0, 100), (0, 100), Some((10, 90)), 22050));
        // The preset's 25 cents are added to the instrument's none; 200 centibels are 20 dB.
        assert!(near(low.root, 59.75) && near(low.pan, -0.5) && near(low.gain, 0.1), "{low:?}");
        assert!(near(low.envelope.release, 2.0) && near(low.envelope.sustain, 0.1) && near(low.envelope.attack, 2f32.powf(-10.0)), "{low:?}");

        let high = &preset.regions[1];
        assert_eq!((high.keys, high.span, high.looped, high.sample_rate), ((64, 127), (146, 226), None, 44100));
        // Root key 70 overrides the sample's 72; a semitone up, 50 cents down and the preset's 25 up.
        assert!(near(high.root, 70.0 - 0.75) && high.pan == 0.0 && high.gain == 1.0 && high.envelope.sustain == 1.0, "{high:?}");

        let within = |(low, high): (u8, u8), x: u8| (low..=high).contains(&x);
        let spans = |key: u8, velocity: u8| {
            preset.regions.iter().filter(|r| within(r.keys, key) && within(r.velocities, velocity)).map(|r| r.span).collect::<Vec<_>>()
        };
        assert_eq!((spans(63, 100), spans(64, 1), spans(127, 100)), (vec![(0, 100)], vec![(146, 226)], vec![(146, 226)]));
        assert!(spans(64, 101).is_empty(), "outside the preset's velocity range");
    }

    /// What every font that parses must keep, however damaged the file was.
    fn sound(font: &SoundFont) {
        let frames = font.pool.len();
        for r in font.presets.iter().flat_map(|p| &p.regions) {
            assert!(r.span.0 < r.span.1 && r.span.1 <= frames && r.sample_rate > 0, "{r:?}");
            assert!(r.looped.is_none_or(|(a, b)| r.span.0 <= a && a < b && b <= r.span.1), "{r:?}");
            assert!(r.root.is_finite() && (-1.0..=1.0).contains(&r.pan) && (0.0..=1.0).contains(&r.gain), "{r:?}");
            let e = r.envelope;
            assert!([e.attack, e.decay, e.release].iter().all(|t| t.is_finite() && *t > 0.0) && (0.0..=1.0).contains(&e.sustain), "{r:?}");
        }
    }

    #[test]
    fn truncated_files_are_errors() {
        let file = tiny();
        for cut in 0..file.len() {
            assert!(read(&file[..cut]).is_err(), "cut at {cut} of {}", file.len());
        }
        assert!(read(&[file.clone(), vec![0; 7]].concat()).is_ok(), "junk after the form is not part of it");
    }

    #[test]
    fn damaged_files_are_errors_or_sound_fonts_never_panics() {
        let file = tiny();
        let check = |bytes: &[u8]| read(bytes).iter().for_each(sound);
        for at in 0..file.len() {
            for mask in [0xFF, 0x80, 0x01] {
                let mut bytes = file.clone();
                bytes[at] ^= mask;
                check(&bytes);
            }
        }
        // A few bytes at a time, anywhere, set to anything.
        let mut noise = Noise(0x9E37_79B9);
        let mut pick = |below: usize| noise.below(below);
        for _ in 0..4000 {
            let mut bytes = file.clone();
            for _ in 0..1 + pick(4) {
                bytes[pick(file.len())] = pick(256) as u8;
            }
            check(&bytes);
        }
        for junk in [&b""[..], b"RIFF", b"RIFF\xff\xff\xff\xffsfbkLIST", b"RIFF\x04\0\0\0sfbk", &[0xFF; 300]] {
            assert!(read(junk).is_err());
        }
    }

    #[test]
    fn the_pool_is_read_up_to_its_limit_and_regions_stay_inside_it() {
        let font = read_within(&tiny(), 120).unwrap();
        assert_eq!(font.pool.len(), 120);
        sound(&font);
        // The second sample starts past the limit: only the first can sound.
        assert_eq!(font.presets[0].regions.iter().map(|r| r.span).collect::<Vec<_>>(), vec![(0, 100)]);
    }

    #[test]
    fn a_file_that_multiplies_its_zones_is_refused() {
        // Forty thousand preset zones, each pointing at the same two-zone instrument.
        let sample = |keys| Sample { data: vec![1000; 8], rate: 8000, pitch: 60, gens: vec![(43, keys)] };
        let file = build(&[sample(range(0, 60)), sample(range(61, 127))], &[], 40_000);
        assert_eq!(read(&file), Err(Error::TooManyZones));
        assert_eq!(read(&build(&[sample(range(0, 60))], &[], 3)).unwrap().presets[0].regions.len(), 3);
        // A font nothing of which can sound holds no audio.
        assert_eq!(read(&build(&[sample(range(70, 60))], &[], 1)), Err(Error::Empty));
    }
}
