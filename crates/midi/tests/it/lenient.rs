//! `read_smf_lenient` on damaged files, instrument names, the end tick and format-0 writing. The damaged-file
//! cases come from LoopCraft's reader, whose behaviour the lenient reader keeps.

use craft_midi::{
    LenientRead, MAX_EVENTS, MAX_TRACKS, MAX_WARNINGS, MidiError, Note, Sequence, Smf, SmfTrack, read_smf, read_smf_lenient, write_smf,
};

fn chunk(out: &mut Vec<u8>, id: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(body);
}

/// A file of the given format and division with these track bodies (End of Track is added).
fn file(format: u16, division: u16, tracks: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    let header = [format.to_be_bytes(), (tracks.len() as u16).to_be_bytes(), division.to_be_bytes()].concat();
    chunk(&mut out, b"MThd", &header);
    for body in tracks {
        chunk(&mut out, b"MTrk", &[body, &[0x00, 0xFF, 0x2F, 0x00][..]].concat());
    }
    out
}

/// The pitches of each track that has notes.
fn keys(smf: &Smf) -> Vec<Vec<u8>> {
    smf.tracks.iter().filter(|t| !t.sequence.notes.is_empty()).map(|t| t.sequence.notes.iter().map(|n| n.pitch).collect()).collect()
}

const ONE: &[u8] = &[0x00, 0x90, 60, 100, 0x60, 0x80, 60, 0];
const TWO: &[u8] = &[0x00, 0x91, 62, 100, 0x60, 0x81, 62, 0];

#[test]
fn a_sound_file_reads_the_same_either_way_without_warnings() {
    let good = file(1, 96, &[ONE, TWO]);
    let LenientRead { smf, warnings, end, .. } = read_smf_lenient(&good).unwrap();
    assert_eq!((keys(&smf), warnings.len()), (vec![vec![60], vec![62]], 0));
    assert_eq!(smf, read_smf(&good).unwrap());
    assert_eq!(end, 960, "the End of Track tick, at 960 ticks a quarter");
}

#[test]
fn damaged_files_give_what_can_be_read_and_say_what_could_not() {
    let good = file(1, 96, &[ONE, TWO]);
    let both = vec![vec![60], vec![62]];

    // Garbage after the last chunk, long or short: strict refuses, lenient warns once.
    for junk in [&b"\0\0\0"[..], &b"garbage that is no chunk at all"[..], &[0xFF; 9][..]] {
        let bytes = [&good[..], junk].concat();
        assert!(read_smf(&bytes).is_err(), "{junk:?}");
        let LenientRead { smf, warnings, .. } = read_smf_lenient(&bytes).unwrap();
        assert_eq!((keys(&smf), warnings.len()), (both.clone(), 1), "{junk:?}: {warnings:?}");
    }
    // A chunk of another kind between the tracks is skipped without a word.
    let mut alien = file(1, 96, &[ONE]);
    chunk(&mut alien, b"XFIH", &[1, 2, 3]);
    chunk(&mut alien, b"MTrk", &[TWO, &[0x00, 0xFF, 0x2F, 0x00][..]].concat());
    let LenientRead { smf, warnings, .. } = read_smf_lenient(&alien).unwrap();
    assert_eq!((keys(&smf), warnings.len()), (both.clone(), 0));

    // The first track's length says too much (it swallows the second), too little, or nothing.
    let len_at = 14 + 4;
    for (len, tracks) in [(15u32, both.clone()), (200, both.clone()), (u32::MAX, both.clone()), (6, both.clone()), (0, vec![vec![62]])] {
        let mut bad = good.clone();
        bad[len_at..len_at + 4].copy_from_slice(&len.to_be_bytes());
        let LenientRead { smf, warnings, .. } = read_smf_lenient(&bad).unwrap();
        assert_eq!(keys(&smf), tracks, "length {len}: {warnings:?}");
        assert!(!warnings.is_empty(), "length {len}");
    }
    // A file cut inside its last track keeps the notes before the cut, the open one ending there.
    let LenientRead { smf: cut, warnings, .. } = read_smf_lenient(&good[..good.len() - 7]).unwrap();
    assert!(read_smf(&good[..good.len() - 7]).is_err());
    assert_eq!((keys(&cut), cut.tracks[1].sequence.notes[0].length), (both.clone(), 960));
    assert!(warnings.iter().any(|w| w.contains("track 2")), "{warnings:?}");
    // A data byte before any status, and a system message that has no place in a file.
    for bad in [&[0x00, 60, 100][..], &[0x00, 0x90, 60, 100, 0x00, 0xF8, 0x00, 0x90, 62, 100][..]] {
        let bytes = file(1, 96, &[bad, TWO]);
        assert!(read_smf(&bytes).is_err());
        let LenientRead { smf, warnings, .. } = read_smf_lenient(&bytes).unwrap();
        assert!(warnings.iter().any(|w| w.contains("no event")) && keys(&smf).last() == Some(&vec![62]), "{warnings:?}");
    }
    // A header with a wrong length, and one that is longer than six bytes.
    let mut long = good.clone();
    long[4..8].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(matches!(read_smf(&long), Err(MidiError::BadChunkLength { .. })));
    assert_eq!(keys(&read_smf_lenient(&long).unwrap().smf), both);
    let mut roomy = Vec::new();
    chunk(&mut roomy, b"MThd", &[0, 1, 0, 1, 0, 96, 9, 9]);
    roomy.extend_from_slice(&good[14..]);
    assert_eq!(keys(&read_smf_lenient(&roomy).unwrap().smf), both);
    // Formats: 2 is read side by side, an unknown one as 1, both with a word about it.
    let two = read_smf_lenient(&file(2, 96, &[ONE, TWO])).map(|r| (r.smf.format, r.smf.tracks.len(), r.warnings.len()));
    assert_eq!(two, Ok((2, 2, 1)));
    assert_eq!(read_smf_lenient(&file(7, 96, &[ONE])).map(|r| (r.smf.format, r.warnings.len())), Ok((1, 1)));
    assert!(read_smf(&file(7, 96, &[ONE])).is_err());
    // Not a MIDI file at all stays an error.
    assert_eq!(read_smf_lenient(b"RIFFxxxxWAVE").err(), Some(MidiError::NotSmf));
    assert!(read_smf_lenient(b"MThd\0\0\0\x06\0\x01").is_err());
}

#[test]
fn names_instruments_and_lyrics() {
    let body = [
        &[0x00, 0xFF, 0x03, 0x04][..],
        b"Lead",
        &[0x00, 0xFF, 0x04, 0x03],
        b"Sax",
        &[0x00, 0xFF, 0x05, 0x02, b'l', b'a', 0x00, 0x90, 60, 1, 0x01, 60, 0],
    ]
    .concat();
    let LenientRead { smf, warnings, instruments, .. } = read_smf_lenient(&file(1, 96, &[&body])).unwrap();
    assert_eq!((smf.tracks[0].name.as_str(), instruments), ("Lead", vec!["Sax".to_string()]));
    assert_eq!(warnings, ["1 lyrics were left out"]);
    // One instrument entry per kept track: the folded conductor track has none.
    let conductor = [0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20];
    let read = read_smf_lenient(&file(1, 96, &[&conductor, &body, &[0x00, 0x90, 62, 1, 0x01, 62, 0]])).unwrap();
    assert_eq!((read.smf.tracks.len(), read.instruments), (2, vec!["Sax".to_string(), String::new()]));
}

#[test]
fn a_file_of_any_size_is_read_into_a_bounded_number_of_events_and_warnings() {
    // Three bytes a note-on under running status: more events than are read.
    let mut body = vec![0x00, 0x90, 60, 100];
    body.extend((0..MAX_EVENTS + 1000).flat_map(|i| [0x00, (i % 128) as u8, 1]));
    let LenientRead { smf, warnings, .. } = read_smf_lenient(&file(1, 96, &[&body])).unwrap();
    assert_eq!(smf.tracks[0].sequence.notes.len(), MAX_EVENTS);
    assert!(warnings.iter().any(|w| w.contains("more events")), "{warnings:?}");
    // Tracks of nothing, by the thousand: the count stops the reading.
    let LenientRead { smf, warnings, .. } = read_smf_lenient(&file(1, 96, &vec![&[][..]; MAX_TRACKS + 5])).unwrap();
    assert!(smf.tracks.len() < MAX_TRACKS && warnings.iter().any(|w| w.contains("tracks")));
    // Every track damaged: never more than MAX_WARNINGS words.
    let LenientRead { warnings, .. } = read_smf_lenient(&file(1, 96, &vec![&[0x00, 60][..]; 100])).unwrap();
    assert_eq!(warnings.len(), MAX_WARNINGS);
}

#[test]
fn damaged_files_never_panic() {
    let good = file(1, 96, &[ONE, TWO]);
    for cut in 0..good.len() {
        let _ = read_smf_lenient(&good[..cut]);
    }
    for i in 0..good.len() {
        for mask in [0xFF, 0x80, 0x01] {
            let mut bad = good.clone();
            bad[i] ^= mask;
            let _ = read_smf_lenient(&bad);
        }
    }
}

#[test]
fn format_0_is_written_as_one_track_and_reads_back() {
    let note = |pitch, channel, start| Note { pitch, velocity: 100, release_velocity: 64, channel, start, length: 480 };
    let track = |name: &str, n: Note| SmfTrack { name: name.into(), sequence: Sequence { notes: vec![n], ctrls: vec![] } };
    let smf = Smf {
        format: 0,
        tracks: vec![track("A", note(60, 0, 0)), track("B", note(64, 1, 960))],
        tempos: vec![(0, 120.0)],
        markers: vec![(960, "B".into())],
        ..Smf::default()
    };
    let bytes = write_smf(&smf).unwrap();
    assert_eq!(&bytes[8..12], &[0, 0, 0, 1], "format 0, one track");
    let back = read_smf(&bytes).unwrap();
    assert_eq!((back.format, back.tracks.len()), (0, 1));
    let notes: Vec<(u8, u8, i64)> = back.tracks[0].sequence.notes.iter().map(|n| (n.pitch, n.channel, n.start)).collect();
    assert_eq!(notes, [(60, 0, 0), (64, 1, 960)]);
    assert_eq!((back.tempos.len(), back.markers.clone()), (1, vec![(960, "B".to_string())]));
    // Any other format is written as format 1.
    assert_eq!(&write_smf(&Smf { format: 2, ..smf }).unwrap()[8..12], &[0, 1, 0, 3]);
}
