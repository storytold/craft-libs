//! Sony XAVC timed metadata's recorded-timecode fields.
//!
//! Wire layout and accepted labels follow the existing Craft implementation in FilmCraft
//! (crates/codecs/src/mp4.rs and its synthetic mp4_rtmd integration tests, revision 3f137a7e).
//! This parser does not choose a sample or determine a track's presentation origin.

/// Bytes needed from the start of a Sony real-time metadata sample.
pub const SONY_RTMD_TIMECODE_PREFIX_LEN: usize = 18;

/// A Sony real-time metadata timecode label, before conversion to a frame count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SonyRtmdTimecode {
    /// Hour within the day.
    pub hours: u8,
    /// Minute within the hour.
    pub minutes: u8,
    /// Second within the minute.
    pub seconds: u8,
    /// Frame label within the second.
    pub frames: u8,
    /// Whether the sample signals drop-frame numbering.
    pub drop_frame: bool,
}

/// Read the recorded label from a Sony `rtmd` sample prefix.
///
/// `frame_base` is the nominal integer frame count per second for the associated video;
/// `supports_drop_frame` says whether its rate supports drop-frame numbering. Preserve the
/// recorded flag even for a rate that does not support it. Reject skipped labels only when
/// drop-frame numbering applies; choosing a timeline origin is the caller's responsibility.
pub fn parse_sony_rtmd_timecode(prefix: &[u8], frame_base: u32, supports_drop_frame: bool) -> Option<SonyRtmdTimecode> {
    let &[hours, minutes, seconds, flag, frames] = prefix.get(13..SONY_RTMD_TIMECODE_PREFIX_LEN)? else { return None };
    let drop_frame = flag != 0;
    if hours >= 24 || minutes >= 60 || seconds >= 60 || u32::from(frames) >= frame_base {
        return None;
    }
    if drop_frame && supports_drop_frame && seconds == 0 && !minutes.is_multiple_of(10) && u32::from(frames) < frame_base / 15 {
        return None;
    }
    Some(SonyRtmdTimecode { hours, minutes, seconds, frames, drop_frame })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sony_rtmd_parses_only_the_fixed_prefix() {
        let mut prefix = [0u8; SONY_RTMD_TIMECODE_PREFIX_LEN + 1];
        prefix[13..18].copy_from_slice(&[3, 37, 12, 0, 34]);
        prefix[18] = 255;
        let expected = Some(SonyRtmdTimecode { hours: 3, minutes: 37, seconds: 12, frames: 34, drop_frame: false });
        assert_eq!(parse_sony_rtmd_timecode(&prefix[..17], 60, true), None);
        assert_eq!(parse_sony_rtmd_timecode(&prefix[..18], 60, true), expected);
        assert_eq!(parse_sony_rtmd_timecode(&prefix, 60, true), expected);
    }

    #[test]
    fn sony_rtmd_rejects_out_of_range_labels() {
        for (index, value) in [(13, 24), (14, 60), (15, 60), (17, 24)] {
            let mut prefix = [0; 18];
            prefix[index] = value;
            assert_eq!(parse_sony_rtmd_timecode(&prefix, 24, false), None);
        }
        assert_eq!(parse_sony_rtmd_timecode(&[0; 18], 0, false), None);
    }

    #[test]
    fn sony_rtmd_drop_frame_labels_follow_the_rate() {
        let mut prefix = [0; 18];
        // Any nonzero flag is retained. At 59.94 the first four labels at minute one are skipped.
        prefix[13..18].copy_from_slice(&[0, 1, 0, 2, 3]);
        assert_eq!(parse_sony_rtmd_timecode(&prefix, 60, true), None);
        assert!(parse_sony_rtmd_timecode(&prefix, 24, false).is_some_and(|code| code.drop_frame));
        prefix[17] = 4;
        assert!(parse_sony_rtmd_timecode(&prefix, 60, true).is_some_and(|code| code.frames == 4 && code.drop_frame));
        prefix[14] = 10;
        prefix[17] = 0;
        assert!(parse_sony_rtmd_timecode(&prefix, 60, true).is_some_and(|code| code.frames == 0));
    }
}
