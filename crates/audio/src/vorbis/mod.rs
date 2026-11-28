//! Vorbis I: the pure-Rust [`decoder`] (feature `vorbis-decode`) and [`encoder`] (feature
//! `vorbis-encode`). `craft_audio::decode` still reads Vorbis files through symphonia.
//!
//! Both work in Vorbis channel order (Vorbis I §4.3.9). [`wav_order`] maps decoded channels to the
//! WAV order the rest of craft-audio uses, and [`layout`] gives the speakers they then feed.

use crate::ChannelLayout;

#[cfg(feature = "vorbis-decode")]
pub mod decoder;
#[cfg(feature = "vorbis-encode")]
pub mod encoder;

/// Channel order for Vorbis surround (Vorbis I §4.3.9: L C R for three, L C R Ls Rs for five,
/// L C R Ls Rs LFE for 5.1, L C R Ls Rs Cb LFE for 6.1, L C R Ls Rs Lb Rb LFE for 7.1): output
/// channel → decoded channel, giving the WAV order (L R C LFE back… side…) of [`layout`]. `None` for
/// 1, 2 and 4 channels, which are already in WAV order, and for more than 8, whose order the
/// specification leaves to the application.
///
/// ```
/// let vorbis: Vec<Vec<f32>> = (0..6).map(|c| vec![c as f32]).collect(); // L C R Ls Rs LFE
/// let order = craft_audio::vorbis::wav_order(6).unwrap();
/// let wav: Vec<Vec<f32>> = order.iter().map(|&c| vorbis[c].clone()).collect(); // L R C LFE Ls Rs
/// assert_eq!(wav[3], vec![5.0]);
/// ```
pub fn wav_order(channels: usize) -> Option<&'static [usize]> {
    Some(match channels {
        3 => &[0, 2, 1],
        5 => &[0, 2, 1, 3, 4],
        6 => &[0, 2, 1, 5, 3, 4],
        7 => &[0, 2, 1, 6, 5, 3, 4],
        8 => &[0, 2, 1, 7, 5, 6, 3, 4],
        _ => return None,
    })
}

/// The speakers of a Vorbis stream with `channels` channels, in WAV order (after [`wav_order`]):
/// mono, stereo, L R C, quad, 5.0, 5.1, 6.1 with a back center and side surrounds, and 7.1 with
/// backs and sides. Other counts are unordered. For 7 channels this differs from
/// [`ChannelLayout::default_for`], which puts the surrounds at the back.
pub fn layout(channels: u16) -> ChannelLayout {
    match channels {
        1 => ChannelLayout::MONO,
        2 => ChannelLayout::STEREO,
        3 => ChannelLayout::SURROUND_3_0,
        4 => ChannelLayout::QUAD,
        5 => ChannelLayout::SURROUND_5_0,
        6 => ChannelLayout::SURROUND_5_1,
        7 => ChannelLayout::SURROUND_6_1,
        8 => ChannelLayout::SURROUND_7_1,
        _ => ChannelLayout::unordered(channels),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Speaker::{self, *};

    /// Vorbis I §4.3.9, one row per channel count, in the order a stream carries the channels.
    const VORBIS_ORDER: [&[Speaker]; 8] = [
        &[FrontCenter],
        &[FrontLeft, FrontRight],
        &[FrontLeft, FrontCenter, FrontRight],
        &[FrontLeft, FrontRight, BackLeft, BackRight],
        &[FrontLeft, FrontCenter, FrontRight, BackLeft, BackRight],
        &[FrontLeft, FrontCenter, FrontRight, BackLeft, BackRight, Lfe],
        &[FrontLeft, FrontCenter, FrontRight, SideLeft, SideRight, BackCenter, Lfe],
        &[FrontLeft, FrontCenter, FrontRight, SideLeft, SideRight, BackLeft, BackRight, Lfe],
    ];

    #[test]
    fn reordered_channels_feed_the_layout_speakers() {
        for (i, stream) in VORBIS_ORDER.iter().enumerate() {
            let channels = i + 1;
            let wav: Vec<Speaker> = match wav_order(channels) {
                Some(order) => order.iter().map(|&c| stream[c]).collect(),
                None => stream.to_vec(),
            };
            assert_eq!(wav, layout(channels as u16).speakers(), "{channels} channels");
        }
    }
}
