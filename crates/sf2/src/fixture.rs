//! Test support (feature `test-support`): writes a small, valid SoundFont 2 file, so that crates
//! which read SoundFonts can test with a real file built in code instead of a binary fixture.

/// One sample and the instrument zone that plays it.
pub struct Sample {
    pub data: Vec<i16>,
    pub rate: u32,
    /// The key the sample was recorded at.
    pub pitch: u8,
    /// The zone's generators as (number, amount); the loop points are the whole sample.
    pub gens: Vec<(u16, i16)>,
}

/// The amount of a range generator.
pub fn range(low: u8, high: u8) -> i16 {
    i16::from_le_bytes([low, high])
}

fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = [&id[..], &(body.len() as u32).to_le_bytes(), body].concat();
    out.resize(out.len() + body.len() % 2, 0);
    out
}

fn list(kind: &[u8; 4], chunks: &[Vec<u8>]) -> Vec<u8> {
    chunk(b"LIST", &[&kind[..], &chunks.concat()].concat())
}

fn named(name: &str) -> [u8; 20] {
    std::array::from_fn(|i| name.as_bytes().get(i).copied().unwrap_or(0))
}

/// A font named "Tiny" with one instrument (a zone per sample) and one preset, "Test", bank 0
/// program 5, made of `copies` zones that each carry `preset_gens` and point at the instrument.
pub fn build(samples: &[Sample], preset_gens: &[(u16, i16)], copies: usize) -> Vec<u8> {
    let generator = |(number, amount): &(u16, i16)| [number.to_le_bytes(), amount.to_le_bytes()].concat();
    // A bag is (first generator, first modulator); one more than the zones says where the last ends.
    let bags = |zones: usize, each: &dyn Fn(usize) -> usize| {
        let mut first = 0;
        let mut bag = |zone: usize| {
            first += each(zone);
            [((first - each(zone)) as u16).to_le_bytes(), [0; 2]].concat()
        };
        (0..=zones).flat_map(&mut bag).collect::<Vec<u8>>()
    };
    let (mut smpl, mut shdr, mut igen) = (Vec::new(), Vec::new(), Vec::new());
    for (i, sample) in samples.iter().enumerate() {
        let (start, end) = (smpl.len() as u32 / 2, (smpl.len() / 2 + sample.data.len()) as u32);
        smpl.extend(sample.data.iter().flat_map(|s| s.to_le_bytes()));
        // The specification asks for 46 frames of silence after every sample.
        smpl.resize(smpl.len() + 92, 0);
        shdr.extend(named(&format!("Sample {i}")));
        shdr.extend([start, end, start, end, sample.rate].iter().flat_map(|field| field.to_le_bytes()));
        shdr.extend([sample.pitch, 0, 0, 0, 1, 0]);
        igen.extend(sample.gens.iter().chain(&[(53, i as i16)]).flat_map(generator));
    }
    shdr.extend(named("EOS").iter().chain(&[0; 26]));
    igen.extend([0; 4]);
    let pgen = [preset_gens.iter().chain(&[(41, 0)]).flat_map(generator).collect::<Vec<u8>>().repeat(copies), vec![0; 4]].concat();
    let header =
        |name: &str, program: u16, bag: usize| [&named(name)[..], &program.to_le_bytes(), &[0; 2], &(bag as u16).to_le_bytes(), &[0; 12]].concat();
    let instrument = |name: &str, bag: usize| [&named(name)[..], &(bag as u16).to_le_bytes()].concat();
    let pdta = [
        chunk(b"phdr", &[header("Test", 5, 0), header("EOP", 0, copies)].concat()),
        chunk(b"pbag", &bags(copies, &|_| preset_gens.len() + 1)),
        chunk(b"pmod", &[0; 10]),
        chunk(b"pgen", &pgen),
        chunk(b"inst", &[instrument("Instrument", 0), instrument("EOI", samples.len())].concat()),
        chunk(b"ibag", &bags(samples.len(), &|zone| samples.get(zone).map_or(0, |s| s.gens.len() + 1))),
        chunk(b"imod", &[0; 10]),
        chunk(b"igen", &igen),
        chunk(b"shdr", &shdr),
    ];
    let info = [chunk(b"ifil", &[2, 0, 1, 0]), chunk(b"isng", b"EMU8000\0"), chunk(b"INAM", b"Tiny\0")];
    chunk(b"RIFF", &[&b"sfbk"[..], &list(b"INFO", &info), &list(b"sdta", &[chunk(b"smpl", &smpl)]), &list(b"pdta", &pdta)].concat())
}
