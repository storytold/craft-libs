use craft_midi::{CtrlEvent, CtrlKind, Note, Sequence};

fn n(pitch: u8, start: i64, length: i64) -> Note {
    Note { pitch, velocity: 100, release_velocity: 64, channel: 0, start, length }
}

fn starts(notes: &[Note]) -> Vec<i64> {
    notes.iter().map(|n| n.start).collect()
}

#[test]
fn sequence_helpers() {
    let mut seq = Sequence {
        notes: vec![n(64, 100, 50), n(60, 100, 10), n(60, 100, 30), n(60, 0, 10), Note { channel: 2, ..n(60, 0, 10) }],
        ctrls: vec![
            CtrlEvent { tick: 500, channel: 0, kind: CtrlKind::Cc(1), value: 1 },
            CtrlEvent { tick: 5, channel: 0, kind: CtrlKind::Cc(1), value: 2 },
            CtrlEvent { tick: 500, channel: 0, kind: CtrlKind::Cc(1), value: 3 },
            CtrlEvent { tick: 500, channel: 0, kind: CtrlKind::Cc(2), value: 4 },
        ],
    };
    assert_eq!(seq.end_tick(), 500);
    seq.sort();
    assert_eq!(starts(&seq.notes), vec![0, 0, 100, 100, 100]);
    assert_eq!(seq.notes[2].pitch, 60);
    assert_eq!(seq.notes_in(0, 100).count(), 2);
    assert_eq!(seq.notes_overlapping(105, 106).count(), 3);

    let removed = seq.remove_duplicates();
    assert_eq!(removed, 2);
    assert_eq!(seq.notes.len(), 4);
    let kept = seq.notes.iter().find(|n| n.start == 100 && n.pitch == 60).unwrap();
    assert_eq!(kept.length, 30);
    assert_eq!(seq.ctrls.iter().map(|c| c.value).collect::<Vec<_>>(), vec![2, 3, 4]);
    assert_eq!(seq.remove_duplicates(), 0);
    assert_eq!(Sequence::default().end_tick(), 0);
}

#[cfg(feature = "serde")]
#[test]
fn sequence_serde_round_trip() {
    let seq = Sequence { notes: vec![n(60, 0, 1)], ctrls: vec![CtrlEvent { tick: 1, channel: 0, kind: CtrlKind::PolyPressure(60), value: 9 }] };
    let json = serde_json::to_string(&seq).unwrap();
    assert_eq!(serde_json::from_str::<Sequence>(&json).unwrap(), seq);
}

#[test]
fn remove_duplicates_handles_a_large_same_tick_controller_group() {
    // Every channel x controller number at one tick, twice: the second copy of each wins. A
    // rescan of the tick's kept events per incoming event would make this quadratic.
    let keys: Vec<(u8, u8)> = (0..=255u8).flat_map(|ch| (0..=255u8).map(move |cc| (ch, cc))).collect();
    let ctrls = [0, 1]
        .iter()
        .flat_map(|&pass| keys.iter().map(move |&(channel, cc)| CtrlEvent { tick: 7, channel, kind: CtrlKind::Cc(cc), value: pass }))
        .collect();
    let mut seq = Sequence { notes: Vec::new(), ctrls };
    let start = std::time::Instant::now();
    assert_eq!(seq.remove_duplicates(), keys.len());
    assert!(start.elapsed() < std::time::Duration::from_secs(10), "{:?}", start.elapsed());
    assert_eq!(seq.ctrls.len(), keys.len());
    assert!(seq.ctrls.iter().all(|c| c.value == 1));
}

#[test]
fn remove_duplicates_matches_a_per_tick_rescan_on_mixed_ticks() {
    // The straightforward per-tick rescan the indexed version replaced, kept as an oracle.
    fn reference(mut ctrls: Vec<CtrlEvent>) -> Vec<CtrlEvent> {
        ctrls.sort_by_key(|c| c.tick);
        let key = |k: CtrlKind| match k {
            CtrlKind::Cc(n) => (0, n),
            CtrlKind::PitchBend => (1, 0),
            CtrlKind::ChannelPressure => (2, 0),
            CtrlKind::PolyPressure(p) => (3, p),
            CtrlKind::Program => (4, 0),
        };
        let mut kept: Vec<CtrlEvent> = Vec::new();
        for c in ctrls {
            match kept.iter_mut().rev().take_while(|k| k.tick == c.tick).find(|k| k.channel == c.channel && key(k.kind) == key(c.kind)) {
                Some(slot) => *slot = c,
                None => kept.push(c),
            }
        }
        kept
    }

    // Few ticks, channels and kinds, so the same key recurs both within a tick (interleaved with
    // other keys) and across ticks; unsorted input from a fixed xorshift stream.
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let kinds = |r: u64| match r % 5 {
        0 => CtrlKind::Cc((r >> 8) as u8 % 4),
        1 => CtrlKind::PitchBend,
        2 => CtrlKind::ChannelPressure,
        3 => CtrlKind::PolyPressure((r >> 8) as u8 % 4),
        _ => CtrlKind::Program,
    };
    for round in 0..200 {
        let mut ctrls = Vec::new();
        for i in 0..(next() % 64) {
            let r = next();
            ctrls.push(CtrlEvent { tick: (r % 6) as i64 - 2, channel: (r >> 16) as u8 % 3, kind: kinds(r >> 24), value: (round * 100 + i) as i32 });
        }
        let mut seq = Sequence { notes: Vec::new(), ctrls: ctrls.clone() };
        let removed = seq.remove_duplicates();
        let expected = reference(ctrls.clone());
        assert_eq!(seq.ctrls, expected, "round {round}");
        assert_eq!(removed, ctrls.len() - expected.len(), "round {round}");
    }
}
