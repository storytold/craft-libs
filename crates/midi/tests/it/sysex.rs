use craft_midi::{MAX_SYSEX_BYTES, MidiError, Sequence, SysExEvent, read_smf, read_smf_lenient, write_smf};

fn file(format: u16, bodies: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"MThd\0\0\0\x06".to_vec();
    bytes.extend_from_slice(&format.to_be_bytes());
    bytes.extend_from_slice(&(bodies.len() as u16).to_be_bytes());
    bytes.extend_from_slice(&960u16.to_be_bytes());
    for body in bodies {
        bytes.extend_from_slice(b"MTrk");
        bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
        bytes.extend_from_slice(body);
    }
    bytes
}

#[test]
fn sysex_only_first_track_is_not_folded_into_the_conductor() {
    let bytes = file(1, &[vec![0, 0xF0, 2, 0x43, 0xF7, 0, 0xFF, 0x2F, 0], vec![0, 0x90, 60, 100, 1, 0x80, 60, 64, 0, 0xFF, 0x2F, 0]]);
    let smf = read_smf(&bytes).unwrap();
    assert_eq!(smf.tracks.len(), 2);
    assert_eq!(smf.tracks[0].sequence.sysex[0].data, [0x43, 0xF7]);
    let back = read_smf(&write_smf(&smf).unwrap()).unwrap();
    assert_eq!(back, smf);
}

#[test]
fn split_f0_f7_and_escape_packets_keep_bytes_and_ticks_in_both_readers() {
    let bytes = file(0, &[vec![0, 0xF0, 3, 0x43, 0x10, 0x4C, 0x87, 0x40, 0xF7, 5, 0, 0, 0x7E, 0, 0xF7, 0, 0xF7, 1, 0xF8, 0, 0xFF, 0x2F, 0]]);
    let smf = read_smf(&bytes).unwrap();
    let packets = &smf.tracks[0].sequence.sysex;
    assert_eq!(packets.iter().map(|p| (p.tick, p.status)).collect::<Vec<_>>(), [(0, 0xF0), (960, 0xF7), (960, 0xF7)]);
    assert_eq!(packets[2].data, [0xF8]);
    assert_eq!(smf.tracks[0].sequence.end_tick(), 960);
    assert_eq!(read_smf_lenient(&bytes).unwrap().smf, smf);
    assert_eq!(read_smf(&write_smf(&smf).unwrap()).unwrap(), smf);
}

#[test]
fn truncated_packet_is_not_invented_by_lenient_import() {
    let bytes = file(0, &[vec![0, 0xF0, 1, 0x43, 0, 0xF7, 4, 0xF7]]);
    assert!(read_smf(&bytes).is_err());
    let read = read_smf_lenient(&bytes).unwrap();
    assert_eq!(read.smf.tracks[0].sequence.sysex.len(), 1);
    assert!(!read.warnings.is_empty());
}

#[test]
fn sysex_limit_is_shared_across_tracks_and_reported_by_lenient_import() {
    let mut body = vec![0, 0xF0, 0x88, 0x80, 0x80, 0]; // 16 MiB VLQ
    body.resize(body.len() + MAX_SYSEX_BYTES, 0x01);
    body.extend_from_slice(&[0, 0xFF, 0x2F, 0]);
    let bytes = file(1, &[body, vec![0, 0xF7, 1, 0xF7, 0, 0xFF, 0x2F, 0]]);
    assert_eq!(read_smf(&bytes), Err(MidiError::SysExTooLarge(MAX_SYSEX_BYTES)));
    assert!(read_smf_lenient(&bytes).unwrap().over_limit);
}

#[test]
fn same_tick_setup_packets_precede_program_changes_and_notes() {
    let mut smf = craft_midi::Smf::default();
    let mut sequence = Sequence::default();
    sequence.sysex.push(SysExEvent { tick: 0, status: 0xF0, data: vec![0x43, 0xF7] });
    sequence.ctrls.push(craft_midi::CtrlEvent { tick: 0, channel: 0, kind: craft_midi::CtrlKind::Program, value: 5 });
    sequence.notes.push(craft_midi::Note { start: 0, length: 960, channel: 0, pitch: 60, velocity: 100, release_velocity: 64 });
    smf.tracks.push(craft_midi::SmfTrack { name: String::new(), sequence });
    let bytes = write_smf(&smf).unwrap();
    let packet = bytes.windows(4).position(|w| w == [0xF0, 2, 0x43, 0xF7]).unwrap();
    let program = bytes.windows(2).position(|w| w == [0xC0, 5]).unwrap();
    let note = bytes.windows(3).position(|w| w == [0x90, 60, 100]).unwrap();
    assert!(packet < program && program < note);
}

#[test]
fn format_zero_cross_track_note_off_precedes_sysex_at_the_same_tick() {
    for reverse in [false, true] {
        let mut packets = Sequence::default();
        packets.sysex.push(SysExEvent { tick: 960, status: 0xF0, data: vec![0x43, 0xF7] });
        packets.sysex.push(SysExEvent { tick: 960, status: 0xF7, data: vec![0xF8] });
        let mut notes = Sequence::default();
        notes.notes.push(craft_midi::Note { start: 0, length: 960, channel: 0, pitch: 60, velocity: 100, release_velocity: 64 });
        notes.notes.push(craft_midi::Note { start: 960, length: 960, channel: 0, pitch: 64, velocity: 100, release_velocity: 64 });
        notes.ctrls.push(craft_midi::CtrlEvent { tick: 960, channel: 0, kind: craft_midi::CtrlKind::Program, value: 5 });
        let mut tracks =
            vec![craft_midi::SmfTrack { name: String::new(), sequence: packets }, craft_midi::SmfTrack { name: String::new(), sequence: notes }];
        if reverse {
            tracks.reverse();
        }
        let bytes = write_smf(&craft_midi::Smf { format: 0, tracks, ..Default::default() }).unwrap();
        let position = |message: &[u8]| bytes.windows(message.len()).position(|w| w == message).unwrap();
        let off = position(&[0x80, 60, 64]);
        let first = position(&[0xF0, 2, 0x43, 0xF7]);
        let second = position(&[0xF7, 1, 0xF8]);
        let program = position(&[0xC0, 5]);
        let on = position(&[0x90, 64, 100]);
        assert!(
            off < first && first < second && second < program && program < on,
            "cross-track order must be note-off, stable SysEx packets, controllers, note-on (reverse={reverse})"
        );
    }
}

#[test]
fn writer_rejects_invalid_status_and_excess_payload() {
    let mut smf = craft_midi::Smf::default();
    smf.tracks.push(craft_midi::SmfTrack::default());
    smf.tracks[0].sequence.sysex.push(SysExEvent { tick: 0, status: 0x90, data: vec![] });
    assert!(write_smf(&smf).is_err());
    let event = &mut smf.tracks[0].sequence.sysex[0];
    event.status = 0xF0;
    event.data.resize(MAX_SYSEX_BYTES + 1, 0);
    assert_eq!(write_smf(&smf), Err(MidiError::SysExTooLarge(MAX_SYSEX_BYTES)));
}

#[cfg(feature = "serde")]
#[test]
fn old_sequences_load_and_sysex_packets_survive_json() {
    let old: Sequence = serde_json::from_str(r#"{"notes":[],"ctrls":[]}"#).unwrap();
    assert!(old.sysex.is_empty());
    assert_eq!(serde_json::to_string(&old).unwrap(), r#"{"notes":[],"ctrls":[]}"#);
    let mut sequence = old;
    sequence.sysex.push(SysExEvent { tick: 10, status: 0xF0, data: vec![0x43, 0xF7] });
    sequence.sysex.push(SysExEvent { tick: 10, status: 0xF7, data: vec![] });
    sequence.sort();
    assert_eq!(serde_json::from_str::<Sequence>(&serde_json::to_string(&sequence).unwrap()).unwrap(), sequence);
    assert_eq!(sequence.remove_duplicates(), 0, "stateful packets must not be deduplicated");
    assert_eq!(sequence.sysex.len(), 2);
}

#[test]
fn rejects_a_format_zero_channel_event_inside_split_sysex() {
    let sysex = vec![0, 0xF0, 1, 0x43, 10, 0xF7, 2, 0x10, 0xF7, 0, 0xFF, 0x2F, 0];
    for (program_tick, valid_merge) in [(5, false), (10, true), (11, true)] {
        let program = vec![program_tick, 0xC0, 7, 0, 0xFF, 0x2F, 0];
        let mut smf = read_smf(&file(1, &[sysex.clone(), program])).unwrap();
        assert!(write_smf(&smf).is_ok());
        smf.format = 0;
        assert_eq!(write_smf(&smf).is_ok(), valid_merge);
    }
}

#[test]
fn split_sysex_closes_before_a_same_tick_note_off() {
    let body = vec![0, 0x90, 60, 100, 5, 0xF0, 1, 0x43, 5, 0xF7, 2, 0x10, 0xF7, 0, 0x80, 60, 64, 0, 0xFF, 0x2F, 0];
    for format in [0, 1] {
        let smf = read_smf(&file(format, std::slice::from_ref(&body))).unwrap();
        let bytes = write_smf(&smf).unwrap();
        assert!(bytes.windows(8).any(|w| w == [0xF7, 2, 0x10, 0xF7, 0, 0x80, 60, 64]));
        assert_eq!(read_smf(&bytes).unwrap(), smf);
    }
}

#[test]
fn split_sysex_opens_after_a_same_tick_program_change() {
    for gap in [0, 10] {
        let body = vec![0, 0xC0, 7, 0, 0xF0, 1, 0x43, gap, 0xF7, 2, 0x10, 0xF7, 0, 0xFF, 0x2F, 0];
        for format in [0, 1] {
            let smf = read_smf(&file(format, std::slice::from_ref(&body))).unwrap();
            let bytes = write_smf(&smf).unwrap();
            assert!(bytes.windows(10).any(|w| w == [0xC0, 7, 0, 0xF0, 1, 0x43, gap, 0xF7, 2, 0x10]));
            assert_eq!(read_smf(&bytes).unwrap(), smf);
        }
    }
}

#[test]
fn legacy_f0_packets_without_a_continuation_survive_export() {
    let body = vec![0, 0xF0, 4, 0x7E, 0x7F, 0x09, 1, 5, 0x90, 60, 100, 5, 0x80, 60, 64, 0, 0xFF, 0x2F, 0];
    for format in [0, 1] {
        let smf = read_smf(&file(format, std::slice::from_ref(&body))).unwrap();
        assert_eq!(read_smf(&write_smf(&smf).unwrap()).unwrap(), smf);
    }

    // A new F0 starts its own message even when the previous packet had no terminator.
    let control_and_setup = vec![5, 0xB0, 7, 90, 5, 0xF0, 2, 0x41, 0xF7, 0, 0xFF, 0x2F, 0];
    let mut smf = read_smf(&file(1, &[body, control_and_setup])).unwrap();
    smf.format = 0;
    let written = read_smf(&write_smf(&smf).unwrap()).unwrap();
    let sequence = &written.tracks[0].sequence;
    assert_eq!(sequence.sysex, [smf.tracks[0].sequence.sysex[0].clone(), smf.tracks[1].sequence.sysex[0].clone()]);
    assert_eq!(sequence.notes, smf.tracks[0].sequence.notes);
    assert_eq!(sequence.ctrls, smf.tracks[1].sequence.ctrls);
}

#[test]
fn later_same_tick_sysex_packets_keep_their_source_order() {
    let body = vec![0, 0xC0, 7, 0, 0xF0, 1, 0x43, 0, 0xF7, 2, 0x10, 0xF7, 0, 0xF0, 2, 0x41, 0xF7, 0, 0xFF, 0x2F, 0];
    for format in [0, 1] {
        let smf = read_smf(&file(format, std::slice::from_ref(&body))).unwrap();
        let bytes = write_smf(&smf).unwrap();
        assert_eq!(read_smf(&bytes).unwrap(), smf);
        assert!(bytes.windows(body.len()).any(|window| window == body));
    }
}

#[test]
fn nonterminating_continuations_still_block_later_channel_events() {
    for format in [0, 1] {
        for terminated in [false, true] {
            let mut body = vec![0, 0xF0, 1, 0x43, 10, 0xF7, 1 + u8::from(terminated), 0x10];
            if terminated {
                body.push(0xF7);
            }
            body.extend([10, 0xC0, 7, 0, 0xFF, 0x2F, 0]);
            let smf = read_smf(&file(format, &[body])).unwrap();
            let result = write_smf(&smf);
            if terminated {
                assert_eq!(read_smf(&result.unwrap()).unwrap(), smf);
            } else {
                assert!(matches!(result, Err(MidiError::Write(message)) if message.contains("interleave")));
            }
        }
    }
}
