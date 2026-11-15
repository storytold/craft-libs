use craft_gif::write::{AnimationWriter, Disposal, Error, Loops, Options, RgbaFrame, animation, animation_with_loops, prepare_frame};

const RED: [u8; 4] = [255, 0, 0, 255];

#[test]
fn total_plays_map_to_gif_repeat_counts_at_the_boundaries() {
    for (loops, repeats) in [
        (Loops::Once, None),
        (Loops::Forever, Some(0)),
        (Loops::Times(1), None),
        (Loops::Times(2), Some(1)),
        (Loops::Times(3), Some(2)),
        (Loops::Times(65_536), Some(u16::MAX)),
    ] {
        let bytes = animation_with_loops(1, 1, &[RgbaFrame { pixels: &RED, delay_cs: 3 }], Options::default(), loops).unwrap();
        let info = craft_gif::scan(&bytes, 1).unwrap();
        assert_eq!(info.loop_count, repeats, "{loops:?}");
        assert_eq!(info.frames[0].delay_cs, 3);
    }
}

#[test]
fn invalid_play_counts_are_rejected_before_writing_a_header() {
    for count in [0, 65_537, u32::MAX] {
        let mut output = Vec::new();
        let result = AnimationWriter::new_with_loops(&mut output, 1, 1, Options::default(), Loops::Times(count));
        assert!(matches!(&result, Err(Error::Encode(message)) if message.contains("1..=65536")), "{count}");
        drop(result);
        assert!(output.is_empty(), "invalid input must leave the output untouched");
    }
}

#[test]
fn explicit_loops_override_the_legacy_boolean() {
    for loop_forever in [false, true] {
        let options = Options { loop_forever, disposal: Disposal::Keep, speed: 10 };
        let bytes = animation_with_loops(1, 1, &[RgbaFrame { pixels: &RED, delay_cs: 1 }], options, Loops::Times(3)).unwrap();
        assert_eq!(craft_gif::scan(&bytes, 1).unwrap().loop_count, Some(2));
    }
}

#[test]
fn legacy_options_keep_the_unmodified_writers_bytes() {
    // Captured from unmodified craft-gif at cae7c1d8a6fec9a9ee136169918c5dd20ca509a0.
    // Keep every Options field: callers using existing struct literals must still compile.
    for (loop_forever, disposal, expected) in [
        (false, Disposal::Unspecified, "4749463839610100010080000000000000000021f90400070000002c000000000100010080ff000000000002024401003b"),
        (false, Disposal::Keep, "4749463839610100010080000000000000000021f90404070000002c000000000100010080ff000000000002024401003b"),
        (false, Disposal::Background, "4749463839610100010080000000000000000021f90408070000002c000000000100010080ff000000000002024401003b"),
        (
            true,
            Disposal::Unspecified,
            "4749463839610100010080000000000000000021ff0b4e45545343415045322e30030100000021f90400070000002c000000000100010080ff000000000002024401003b",
        ),
        (
            true,
            Disposal::Keep,
            "4749463839610100010080000000000000000021ff0b4e45545343415045322e30030100000021f90404070000002c000000000100010080ff000000000002024401003b",
        ),
        (
            true,
            Disposal::Background,
            "4749463839610100010080000000000000000021ff0b4e45545343415045322e30030100000021f90408070000002c000000000100010080ff000000000002024401003b",
        ),
    ] {
        let options = Options { loop_forever, disposal, speed: 10 };
        let bytes = animation(1, 1, &[RgbaFrame { pixels: &RED, delay_cs: 7 }], options).unwrap();
        let hex = bytes.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        assert_eq!(hex, expected);
    }
}

#[test]
fn rgba_and_prepared_overrides_apply_to_only_the_requested_frame() {
    let mut writer = AnimationWriter::new(Vec::new(), 1, 1, Options::default()).unwrap();
    writer.write_frame_with_disposal(RgbaFrame { pixels: &RED, delay_cs: 2 }, Disposal::Background).unwrap();
    writer.write_prepared_frame_with_disposal(prepare_frame(1, 1, RED.to_vec(), 10).unwrap(), 4, Disposal::Unspecified).unwrap();
    writer.write_frame(RgbaFrame { pixels: &RED, delay_cs: 6 }).unwrap();
    let bytes = writer.finish().unwrap();
    let info = craft_gif::scan(&bytes, 3).unwrap();
    assert_eq!(info.frames.iter().map(|frame| frame.disposal).collect::<Vec<_>>(), [2, 0, 1]);
    assert_eq!(info.frames.iter().map(|frame| frame.delay_cs).collect::<Vec<_>>(), [2, 4, 6]);
}

#[test]
fn override_paths_keep_frame_validation_and_the_frame_index() {
    let mut writer = AnimationWriter::new(Vec::new(), 1, 1, Options::default()).unwrap();
    writer.write_frame(RgbaFrame { pixels: &RED, delay_cs: 1 }).unwrap();
    assert!(matches!(
        writer.write_frame_with_disposal(RgbaFrame { pixels: &[], delay_cs: 1 }, Disposal::Background),
        Err(Error::FrameLength { index: 1, actual: 0, expected: 4 })
    ));
    let wrong_size = prepare_frame(2, 1, vec![255; 8], 10).unwrap();
    assert!(matches!(writer.write_prepared_frame_with_disposal(wrong_size, 1, Disposal::Background), Err(Error::PreparedDimensions { .. })));
    writer.write_prepared_frame(prepare_frame(1, 1, RED.to_vec(), 10).unwrap(), 2).unwrap();
    assert_eq!(craft_gif::scan(&writer.finish().unwrap(), 3).unwrap().frames.len(), 2);
}

#[cfg(feature = "frames")]
#[test]
fn clearing_the_frame_before_transparency_prevents_ghost_pixels() {
    let first = [255, 0, 0, 255, 255, 0, 0, 255];
    let second = [0, 0, 0, 0, 0, 0, 255, 255];
    let mut writer = AnimationWriter::new_with_loops(Vec::new(), 2, 1, Options::default(), Loops::Times(3)).unwrap();
    writer.write_frame_with_disposal(RgbaFrame { pixels: &first, delay_cs: 3 }, Disposal::Background).unwrap();
    writer.write_prepared_frame_with_disposal(prepare_frame(2, 1, second.to_vec(), 10).unwrap(), 5, Disposal::Background).unwrap();
    let bytes = writer.finish().unwrap();
    let decoded = craft_gif::frames::read_frames(&bytes, &craft_gif::frames::FrameLimits::default()).unwrap();
    assert_eq!(decoded.frames[0].rgba, first);
    assert_eq!(decoded.frames[1].rgba, second);
    assert_eq!(decoded.frames.iter().map(|frame| frame.delay_ms).collect::<Vec<_>>(), [30, 50]);
}

#[test]
fn the_explicit_loop_constructor_propagates_output_errors() {
    struct RejectWrites;
    impl std::io::Write for RejectWrites {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("test output failure"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let result = AnimationWriter::new_with_loops(RejectWrites, 1, 1, Options::default(), Loops::Times(3));
    assert!(matches!(result, Err(Error::Encode(_))));
}
