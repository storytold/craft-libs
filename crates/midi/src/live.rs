//! Live MIDI 1.0 channel-voice message decoding.

/// Decoded MIDI 1.0 channel-voice message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelMessage {
    /// Note Off (`0x8n`, key, release velocity).
    NoteOff { channel: u8, key: u8, velocity: u8 },
    /// Note On (`0x9n`, key, velocity).
    NoteOn { channel: u8, key: u8, velocity: u8 },
    /// Polyphonic key pressure / aftertouch (`0xAn`, key, pressure).
    PolyPressure { channel: u8, key: u8, pressure: u8 },
    /// Control Change (`0xBn`, controller `0..=127`, value `0..=127`).
    ControlChange { channel: u8, controller: u8, value: u8 },
    /// Channel pressure / aftertouch (`0xDn`, pressure `0..=127`).
    ChannelPressure { channel: u8, pressure: u8 },
    /// 14-bit Pitch Bend (`0xEn`, value `0..=16383`).
    PitchBend { channel: u8, value: u16 },
}

impl ChannelMessage {
    /// Parses one MIDI 1.0 channel-voice message from raw wire bytes.
    ///
    /// Returns `None` for truncated messages, out-of-range data bytes (`> 127`),
    /// system messages (`0xF0..=0xFF`), or Program Change (`0xCn`).
    #[must_use]
    pub fn parse(message: &[u8]) -> Option<Self> {
        let &status = message.first()?;
        let channel = status & 0x0F;
        let &a = message.get(1)?;
        if a > 127 {
            return None;
        }
        match status & 0xF0 {
            0x80 => {
                let &b = message.get(2)?;
                (b <= 127).then_some(Self::NoteOff { channel, key: a, velocity: b })
            }
            0x90 => {
                let &b = message.get(2)?;
                (b <= 127).then_some(Self::NoteOn { channel, key: a, velocity: b })
            }
            0xA0 => {
                let &b = message.get(2)?;
                (b <= 127).then_some(Self::PolyPressure { channel, key: a, pressure: b })
            }
            0xB0 => {
                let &b = message.get(2)?;
                (b <= 127).then_some(Self::ControlChange { channel, controller: a, value: b })
            }
            0xD0 => Some(Self::ChannelPressure { channel, pressure: a }),
            0xE0 => {
                let &b = message.get(2)?;
                (b <= 127).then_some(Self::PitchBend { channel, value: (u16::from(b) << 7) | u16::from(a) })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ChannelMessage;

    #[test]
    fn parses_channel_voice_messages() {
        assert_eq!(ChannelMessage::parse(&[0x92, 60, 100]), Some(ChannelMessage::NoteOn { channel: 2, key: 60, velocity: 100 }));
        assert_eq!(ChannelMessage::parse(&[0x82, 60, 64]), Some(ChannelMessage::NoteOff { channel: 2, key: 60, velocity: 64 }));
        assert_eq!(ChannelMessage::parse(&[0xA5, 64, 90]), Some(ChannelMessage::PolyPressure { channel: 5, key: 64, pressure: 90 }));
        assert_eq!(ChannelMessage::parse(&[0xB1, 74, 127]), Some(ChannelMessage::ControlChange { channel: 1, controller: 74, value: 127 }));
        assert_eq!(ChannelMessage::parse(&[0xD3, 55]), Some(ChannelMessage::ChannelPressure { channel: 3, pressure: 55 }));
        assert_eq!(ChannelMessage::parse(&[0xE0, 0x00, 0x40]), Some(ChannelMessage::PitchBend { channel: 0, value: 8192 }));
        assert_eq!(ChannelMessage::parse(&[0xEF, 0x7F, 0x7F]), Some(ChannelMessage::PitchBend { channel: 15, value: 16383 }));
    }

    #[test]
    fn rejects_invalid_and_truncated_messages() {
        assert_eq!(ChannelMessage::parse(&[]), None);
        assert_eq!(ChannelMessage::parse(&[0x90, 60]), None);
        assert_eq!(ChannelMessage::parse(&[0x90, 128, 64]), None);
        assert_eq!(ChannelMessage::parse(&[0x90, 60, 128]), None);
        assert_eq!(ChannelMessage::parse(&[0xD0, 128]), None);
        assert_eq!(ChannelMessage::parse(&[0xC0, 10]), None);
        assert_eq!(ChannelMessage::parse(&[0xF8]), None);
    }
}
