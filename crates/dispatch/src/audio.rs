//! Audio channel layouts and multichannel matrix routing for job specifications.

use serde::{Deserialize, Serialize};

/// Standard audio channel layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ChannelLayout {
    /// Single channel (center/mono).
    Mono = 1,
    /// 2 channels (Left, Right).
    #[default]
    Stereo = 2,
    /// 6 channels (Left, Right, Center, LFE, Left Surround, Right Surround).
    Surround51 = 6,
    /// 8 channels (Left, Right, Center, LFE, Left Surround, Right Surround, Left Rear, Right Rear).
    Surround71 = 8,
}

impl ChannelLayout {
    /// Number of discrete audio channels in this layout.
    pub fn channel_count(self) -> usize {
        self as usize
    }
}

/// Discrete audio track layout packaging modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AudioTrackLayout {
    /// Standard interleaved track layout (Mono, Stereo, 5.1, 7.1).
    #[default]
    Standard,
    /// Discrete mono tracks (e.g. 8 separate channels for broadcast MXF/BWF delivery).
    DiscreteMono,
    /// Discrete stereo pairs (e.g. 4 stereo stem pairs).
    StereoPairs,
    /// 5.1 Surround track.
    Surround51,
    /// 7.1 Surround track.
    Surround71,
}

/// Multichannel matrix router mapping input channels to output tracks and channels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioChannelMapping {
    /// Output channel count (1, 2, 6, 8, etc.).
    pub output_channels: usize,
    /// Multi-track packaging layout.
    #[serde(default)]
    pub track_layout: AudioTrackLayout,
    /// Matrix gains: `matrix[output_ch][input_ch]` where 1.0 is 0 dB, 0.0 is mute.
    pub matrix: Vec<Vec<f32>>,
}

/// Largest input channel count a routing row may grow to.
pub const MAX_INPUT_CHANNELS: usize = 256;

impl AudioChannelMapping {
    /// Creates a 1:1 identity passthrough mapping for N channels.
    pub fn new_pass_through(channels: usize) -> Self {
        let count = channels.max(1);
        let mut matrix = Vec::with_capacity(count);
        for out_idx in 0..count {
            let mut row = vec![0.0f32; count];
            if out_idx < count {
                row[out_idx] = 1.0;
            }
            matrix.push(row);
        }
        Self { output_channels: count, track_layout: AudioTrackLayout::Standard, matrix }
    }

    /// Creates a discrete mono tracks mapping (e.g. 8-channel broadcast delivery).
    pub fn new_discrete_mono(channels: usize) -> Self {
        let mut mapping = Self::new_pass_through(channels);
        mapping.track_layout = AudioTrackLayout::DiscreteMono;
        mapping
    }

    /// Retrieves the routing gain from `input_ch` to `output_ch`.
    pub fn get_gain(&self, output_ch: usize, input_ch: usize) -> f32 {
        if output_ch >= self.output_channels {
            return 0.0;
        }
        self.matrix.get(output_ch).and_then(|row| row.get(input_ch)).copied().unwrap_or(0.0)
    }

    /// Sets the routing gain from `input_ch` to `output_ch`, clamped to `0.0..=4.0`.
    ///
    /// Writes to outputs the mapping does not declare, to inputs at or beyond
    /// [`MAX_INPUT_CHANNELS`], and non-finite gains are ignored.
    pub fn set_gain(&mut self, output_ch: usize, input_ch: usize, gain: f32) {
        if output_ch >= self.output_channels || input_ch >= MAX_INPUT_CHANNELS || !gain.is_finite() {
            return;
        }
        if self.matrix.len() <= output_ch {
            self.matrix.resize(output_ch + 1, Vec::new());
        }
        let row = &mut self.matrix[output_ch];
        if row.len() <= input_ch {
            row.resize(input_ch + 1, 0.0);
        }
        row[input_ch] = gain.clamp(0.0, 4.0);
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioChannelMapping, AudioTrackLayout};

    #[test]
    fn test_channel_mapping_passthrough_and_discrete() {
        let mut map = AudioChannelMapping::new_discrete_mono(4);
        assert_eq!(map.track_layout, AudioTrackLayout::DiscreteMono);
        assert_eq!(map.get_gain(0, 0), 1.0);
        assert_eq!(map.get_gain(0, 1), 0.0);
        map.set_gain(0, 1, 0.5);
        assert_eq!(map.get_gain(0, 1), 0.5);
    }

    #[test]
    fn routing_ignores_undeclared_outputs_hostile_indices_and_nan() {
        let mut map = AudioChannelMapping::new_pass_through(2);
        map.matrix.push(vec![0.5, 0.0]); // a third row the mapping does not declare
        assert_eq!(map.get_gain(2, 0), 0.0);
        map.set_gain(0, usize::MAX, 1.0);
        map.set_gain(0, 0, f32::NAN);
        assert_eq!(map.get_gain(0, 0), 1.0);
        assert_eq!(map.matrix[0].len(), 2);
        map.set_gain(0, 0, 9.0);
        assert_eq!(map.get_gain(0, 0), 4.0);
    }
}
