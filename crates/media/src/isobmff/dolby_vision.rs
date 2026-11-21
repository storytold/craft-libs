//! Dolby Vision decoder configuration records: the `dvcC`, `dvvC` and `dvwC` boxes of a video
//! sample entry.

use crate::isobmff::bytes::FourCc;
use crate::isobmff::error::{Error, Result};

/// The Dolby Vision decoder configuration of a video sample entry: the payload of its `dvcC`,
/// `dvvC` or `dvwC` box, kept byte for byte with the box type it came from, so writing it back
/// reproduces the box exactly.
///
/// The accessors decode the record's first five bytes: the version, profile, level, the RPU, EL
/// and BL present flags, and the base layer's signal compatibility ID. The bytes after those are
/// kept as they were read and not interpreted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DolbyVisionConfig {
    box_kind: FourCc,
    record: Vec<u8>,
}

impl DolbyVisionConfig {
    /// The box types that hold the record: `dvcC` (Dolby Vision Configuration), `dvvC` (Extended
    /// Configuration) and `dvwC` (Extended Configuration 2), as registered at mp4ra.org.
    pub const BOX_KINDS: [FourCc; 3] = [FourCc(*b"dvcC"), FourCc(*b"dvvC"), FourCc(*b"dvwC")];
    /// The shortest record read: the bytes up to and including the compatibility ID.
    pub const MIN_RECORD_LEN: usize = 5;
    /// The longest record read, and the size of a new one: 24 bytes, the record's size in every
    /// file observed.
    pub const MAX_RECORD_LEN: usize = 24;

    /// The record in the payload of a `box_kind` box, kept as it is. `None` when `box_kind` is not
    /// one of [`BOX_KINDS`](Self::BOX_KINDS) or the payload is shorter than
    /// [`MIN_RECORD_LEN`](Self::MIN_RECORD_LEN) or longer than
    /// [`MAX_RECORD_LEN`](Self::MAX_RECORD_LEN).
    pub fn from_box(box_kind: FourCc, payload: &[u8]) -> Option<Self> {
        let fits = (Self::MIN_RECORD_LEN..=Self::MAX_RECORD_LEN).contains(&payload.len());
        (fits && Self::BOX_KINDS.contains(&box_kind)).then(|| DolbyVisionConfig { box_kind, record: payload.to_vec() })
    }

    /// A version 1.0 record for a new stream, [`MAX_RECORD_LEN`](Self::MAX_RECORD_LEN) bytes with
    /// every byte after the compatibility ID zero. The box type follows the profile: `dvcC` up to
    /// profile 7, `dvvC` for 8 to 10, `dvwC` above. Errors when a value does not fit its field:
    /// 7 bits for the profile, 6 for the level, 4 for the compatibility ID.
    pub fn new(profile: u8, level: u8, rpu_present: bool, el_present: bool, bl_present: bool, bl_signal_compatibility_id: u8) -> Result<Self> {
        if profile > 0x7F || level > 0x3F || bl_signal_compatibility_id > 0x0F {
            return Err(Error::Invalid(format!(
                "Dolby Vision profile {profile}, level {level} or compatibility ID {bl_signal_compatibility_id} does not fit its field"
            )));
        }
        let fields = (u16::from(profile) << 9)
            | (u16::from(level) << 3)
            | (u16::from(rpu_present) << 2)
            | (u16::from(el_present) << 1)
            | u16::from(bl_present);
        let [hi, lo] = fields.to_be_bytes();
        let mut record = vec![1, 0, hi, lo, bl_signal_compatibility_id << 4];
        record.resize(Self::MAX_RECORD_LEN, 0);
        let box_kind = FourCc(*match profile {
            0..=7 => b"dvcC",
            8..=10 => b"dvvC",
            _ => b"dvwC",
        });
        Ok(DolbyVisionConfig { box_kind, record })
    }

    /// The box type the record is stored in.
    pub fn box_kind(&self) -> FourCc {
        self.box_kind
    }

    /// The record: the box payload, byte for byte.
    pub fn record(&self) -> &[u8] {
        &self.record
    }

    /// Byte `i` of the record. Every record holds at least [`MIN_RECORD_LEN`](Self::MIN_RECORD_LEN)
    /// bytes, so the fields read never fall back to 0.
    fn byte(&self, i: usize) -> u8 {
        self.record.get(i).copied().unwrap_or(0)
    }

    /// Profile (7 bits), level (6 bits) and the RPU, EL and BL present flags, from bytes 2 and 3.
    fn fields(&self) -> u16 {
        u16::from_be_bytes([self.byte(2), self.byte(3)])
    }

    pub fn version_major(&self) -> u8 {
        self.byte(0)
    }

    pub fn version_minor(&self) -> u8 {
        self.byte(1)
    }

    /// The Dolby Vision profile (5, 7, 8, 9, 10, 20, …).
    pub fn profile(&self) -> u8 {
        (self.fields() >> 9) as u8
    }

    pub fn level(&self) -> u8 {
        ((self.fields() >> 3) & 0x3F) as u8
    }

    /// The stream carries reference processing unit (RPU) metadata.
    pub fn rpu_present(&self) -> bool {
        self.fields() & 4 != 0
    }

    /// The stream carries an enhancement layer.
    pub fn el_present(&self) -> bool {
        self.fields() & 2 != 0
    }

    /// The stream carries a base layer.
    pub fn bl_present(&self) -> bool {
        self.fields() & 1 != 0
    }

    /// What the base layer is compatible with (Dolby's "Dolby Vision Profiles and Levels"): 0 none
    /// (Dolby Vision only), 1 HDR10, 2 SDR, 4 HLG, 6 Ultra HD Blu-ray HDR; 3, 5, 7 and 15 are
    /// reserved, and 8 to 14 are not listed.
    pub fn bl_signal_compatibility_id(&self) -> u8 {
        self.byte(4) >> 4
    }

    /// True when the stream has a base layer that displays without the Dolby Vision metadata: its
    /// compatibility ID is HDR10 (1), SDR (2), HLG (4) or Ultra HD Blu-ray HDR (6). Profile 5
    /// (compatibility ID 0) is not.
    pub fn base_layer_compatible(&self) -> bool {
        self.bl_present() && matches!(self.bl_signal_compatibility_id(), 1 | 2 | 4 | 6)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `dvvC` payload of an original iPhone recording (profile 8.4, an HLG base layer).
    const CAMERA_DVVC: [u8; 24] = [0x01, 0x00, 0x10, 0x25, 0x40, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

    #[test]
    fn a_camera_record_reads_as_profile_8_4() {
        let dv = DolbyVisionConfig::from_box(FourCc(*b"dvvC"), &CAMERA_DVVC).unwrap();
        assert_eq!((dv.box_kind(), dv.record()), (FourCc(*b"dvvC"), &CAMERA_DVVC[..]));
        assert_eq!((dv.version_major(), dv.version_minor(), dv.profile(), dv.level()), (1, 0, 8, 4));
        assert_eq!((dv.rpu_present(), dv.el_present(), dv.bl_present()), (true, false, true));
        assert_eq!(dv.bl_signal_compatibility_id(), 4);
        assert!(dv.base_layer_compatible());
        // A new record with the same fields is the same bytes in the same box.
        assert_eq!(DolbyVisionConfig::new(8, 4, true, false, true, 4).unwrap(), dv);
    }

    /// Profile 5 has no cross-compatible base layer (compatibility ID 0).
    #[test]
    fn profile_5_has_no_displayable_base_layer() {
        let dv = DolbyVisionConfig::new(5, 4, true, false, true, 0).unwrap();
        assert_eq!(dv.box_kind(), FourCc(*b"dvcC"));
        assert_eq!(&dv.record()[..5], [1, 0, 0x0A, 0x25, 0x00]);
        assert!(!dv.base_layer_compatible());
    }

    /// Only HDR10 (1), SDR (2), HLG (4) and Ultra HD Blu-ray HDR (6) base layers display without
    /// the Dolby Vision metadata, and only when the stream has a base layer.
    #[test]
    fn base_layer_compatibility_follows_the_compatibility_id() {
        for id in 0..=15 {
            let with_bl = DolbyVisionConfig::new(8, 4, true, false, true, id).unwrap();
            assert_eq!(with_bl.bl_signal_compatibility_id(), id);
            assert_eq!(with_bl.base_layer_compatible(), matches!(id, 1 | 2 | 4 | 6), "id {id}");
            assert!(!DolbyVisionConfig::new(8, 4, true, false, false, id).unwrap().base_layer_compatible(), "id {id} without a base layer");
        }
    }

    /// A new record is version 1.0, 24 bytes, zero after the compatibility ID, in the box type its
    /// profile is stored in; values too wide for their field are refused, not cut.
    #[test]
    fn new_records_pick_the_box_type_from_the_profile() {
        for (profile, kind) in
            [(0, b"dvcC"), (5, b"dvcC"), (7, b"dvcC"), (8, b"dvvC"), (9, b"dvvC"), (10, b"dvvC"), (11, b"dvwC"), (20, b"dvwC"), (127, b"dvwC")]
        {
            let dv = DolbyVisionConfig::new(profile, 63, false, true, false, 15).unwrap();
            assert_eq!(dv.box_kind(), FourCc(*kind), "profile {profile}");
            assert_eq!(dv.record().len(), DolbyVisionConfig::MAX_RECORD_LEN);
            assert!(dv.record()[5..].iter().all(|&b| b == 0));
            assert_eq!((dv.version_major(), dv.version_minor()), (1, 0));
            assert_eq!((dv.profile(), dv.level(), dv.bl_signal_compatibility_id()), (profile, 63, 15));
            assert_eq!((dv.rpu_present(), dv.el_present(), dv.bl_present()), (false, true, false));
            assert_eq!(DolbyVisionConfig::from_box(dv.box_kind(), dv.record()), Some(dv));
        }
        assert!(DolbyVisionConfig::new(128, 4, true, false, true, 4).is_err());
        assert!(DolbyVisionConfig::new(8, 64, true, false, true, 4).is_err());
        assert!(DolbyVisionConfig::new(8, 4, true, false, true, 16).is_err());
    }

    /// Only the three configuration box types, with 5 to 24 bytes, hold a record.
    #[test]
    fn records_come_from_configuration_boxes_of_a_known_size() {
        for kind in DolbyVisionConfig::BOX_KINDS {
            assert!(DolbyVisionConfig::from_box(kind, &CAMERA_DVVC).is_some());
            assert!(DolbyVisionConfig::from_box(kind, &CAMERA_DVVC[..5]).is_some());
            assert!(DolbyVisionConfig::from_box(kind, &CAMERA_DVVC[..4]).is_none());
            assert!(DolbyVisionConfig::from_box(kind, &[CAMERA_DVVC.as_slice(), &[0]].concat()).is_none());
        }
        for kind in [b"hvcC", b"dvxC", b"DVVC"] {
            assert!(DolbyVisionConfig::from_box(FourCc(*kind), &CAMERA_DVVC).is_none());
        }
    }
}
