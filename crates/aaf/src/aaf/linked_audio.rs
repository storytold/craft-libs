//! WAV/PCM FileDescriptor metadata, without loading a SourceMob or touching its locators.

use super::ids::{cls, pid};

/// The two audio descriptor classes commonly attached to an external WAV source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DescriptorKind {
    /// A WAVEDescriptor (`.wav` with a RIFF header).
    Wave,
    /// A PCMDescriptor (raw PCM samples).
    Pcm,
}

impl DescriptorKind {
    /// Classify a compound-file storage's CLSID in AAF on-disk AUID byte order
    /// (little-endian GUID fields, not RFC 4122/network order).
    pub fn from_auid_le(class: [u8; 16]) -> Option<Self> {
        match class {
            cls::WAVE_DESCRIPTOR => Some(Self::Wave),
            cls::PCM_DESCRIPTOR => Some(Self::Pcm),
            _ => None,
        }
    }
}

/// One property from an already framed and caller-validated object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DescriptorProperty<'a> {
    /// The object has no such property.
    Missing,
    /// The property is stored in a form other than a plain data value.
    OtherForm,
    /// The property's raw value bytes.
    Data(&'a [u8]),
}

/// Why a descriptor field is unavailable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DescriptorFieldError {
    /// The descriptor has no such property.
    Missing,
    /// The property is stored in a form other than a plain data value.
    OtherForm,
    /// The value is shorter than the field's type.
    Truncated,
}

/// A lazy descriptor view over a caller's property store. This neither opens
/// the media file nor determines whether the AAF source is external.
pub struct AudioFileDescriptor<F> {
    kind: DescriptorKind,
    property: F,
}

impl<'a, F: Fn(u16) -> DescriptorProperty<'a>> AudioFileDescriptor<F> {
    /// Supply the property view for this class; the descriptor chooses AAF property IDs.
    pub fn new(kind: DescriptorKind, property: F) -> Self {
        Self { kind, property }
    }

    /// `FileDescriptor::SampleRate` as the signed numerator and denominator on
    /// the wire. The caller's rational policy (including zero) is not applied.
    pub fn sample_rate(&self) -> Result<(i32, i32), DescriptorFieldError> {
        let [a, b, c, d, e, f, g, h] = self.bytes(pid::SAMPLE_RATE)?;
        Ok((i32::from_le_bytes([a, b, c, d]), i32::from_le_bytes([e, f, g, h])))
    }

    /// `FileDescriptor::Length` in sample-rate units, before validating clip ranges.
    pub fn length(&self) -> Result<i64, DescriptorFieldError> {
        self.bytes(pid::FILE_LENGTH).map(i64::from_le_bytes)
    }

    /// `SoundDescriptor::Channels` for a PCM descriptor. A WAVEDescriptor has
    /// no such declared count here; an absent PCM field is also left to the caller.
    pub fn pcm_channels(&self) -> Result<Option<u32>, DescriptorFieldError> {
        if self.kind == DescriptorKind::Wave {
            return Ok(None);
        }
        match self.bytes(pid::CHANNELS) {
            Ok(bytes) => Ok(Some(u32::from_le_bytes(bytes))),
            Err(DescriptorFieldError::Missing) => Ok(None),
            Err(err) => Err(err),
        }
    }

    fn bytes<const N: usize>(&self, pid: u16) -> Result<[u8; N], DescriptorFieldError> {
        match (self.property)(pid) {
            DescriptorProperty::Data(data) => data.first_chunk().copied().ok_or(DescriptorFieldError::Truncated),
            DescriptorProperty::Missing => Err(DescriptorFieldError::Missing),
            DescriptorProperty::OtherForm => Err(DescriptorFieldError::OtherForm),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_class_and_little_endian_fields_are_typed() {
        assert_eq!(DescriptorKind::from_auid_le([1, 1, 1, 13, 1, 1, 0, 0x48, 6, 14, 43, 52, 2, 6, 1, 1]), Some(DescriptorKind::Pcm));
        assert_eq!(DescriptorKind::from_auid_le(cls::WAVE_DESCRIPTOR), Some(DescriptorKind::Wave));
        assert_eq!(DescriptorKind::from_auid_le(cls::CDCI_DESCRIPTOR), None);
        let file = AudioFileDescriptor::new(DescriptorKind::Pcm, |tag| match tag {
            pid::SAMPLE_RATE => DescriptorProperty::Data(&[0x80, 0xbb, 0, 0, 1, 0, 0, 0]),
            pid::FILE_LENGTH => DescriptorProperty::Data(&[2, 1, 0, 0, 0, 0, 0, 0, 99]),
            pid::CHANNELS => DescriptorProperty::Data(&[2, 0, 0, 0]),
            _ => DescriptorProperty::Missing,
        });
        assert_eq!(file.sample_rate(), Ok((48000, 1)));
        assert_eq!(file.length(), Ok(258));
        assert_eq!(file.pcm_channels(), Ok(Some(2)));
    }

    #[test]
    fn descriptor_fields_are_lazy_and_distinguish_missing_form_and_short_data() {
        let wave = AudioFileDescriptor::new(DescriptorKind::Wave, |tag| match tag {
            pid::SAMPLE_RATE => DescriptorProperty::OtherForm,
            pid::FILE_LENGTH => DescriptorProperty::Data(&[0; 7]),
            _ => panic!("unexpected field read"),
        });
        assert_eq!(wave.pcm_channels(), Ok(None));
        assert_eq!(wave.sample_rate(), Err(DescriptorFieldError::OtherForm));
        assert_eq!(wave.length(), Err(DescriptorFieldError::Truncated));
        let pcm = AudioFileDescriptor::new(DescriptorKind::Pcm, |_| DescriptorProperty::Missing);
        assert_eq!(pcm.pcm_channels(), Ok(None));
        assert_eq!(pcm.sample_rate(), Err(DescriptorFieldError::Missing));
    }
}
