//! Alternate SR2 distortion record: bounded decoding, and its place beside the older table. Where it is usable it
//! replaces that table's warp, never composes with it.

use super::{Directory, Found, Read, SHORT, SSHORT, SonyFieldError, combine, sr2_copy};
use crate::Opcode;
use crate::lens_metadata::{SonyCopy, SonyExtendedDistortion, SonyField, SonyLensGeometry, SonyLensStatus, SonySource};

mod warp;

fn valid<T>(field: &SonyField<T>) -> bool {
    field.source == Some(SonySource::Sr2SubIfd)
        && field.raw_ifd == SonyCopy::Absent
        && field.sr2 == SonyCopy::Valid
        && field.copies_agree.is_none()
        && !field.raw_ifd_repeated
        && !field.sr2_repeated
}

/// The alternate record can explicitly say that only the legacy table is available.
fn explicitly_unavailable(record: &SonyExtendedDistortion) -> bool {
    valid(&record.setting)
        && record.setting.value == Some([u16::MAX; 2])
        && record.table.value.is_none()
        && record.table.source.is_none()
        && record.table.raw_ifd == SonyCopy::Absent
        && record.table.sr2 == SonyCopy::Rejected(SonyFieldError::CountPrefix(0))
        && record.table.copies_agree.is_none()
        && !record.table.raw_ifd_repeated
        && !record.table.sr2_repeated
}

/// Set the record's status. `true` when it says only the older table is available, which then applies; any other
/// record owns distortion, and a broken or half-present one is never silently replaced by the older curve.
pub(super) fn settle(record: &mut SonyExtendedDistortion) -> bool {
    if explicitly_unavailable(record) {
        record.status = SonyLensStatus::NoTable;
        return true;
    }
    record.status = match (record.setting.value, record.table.value.as_ref()) {
        _ if !valid(&record.setting) || !valid(&record.table) => SonyLensStatus::Rejected,
        (Some([switch, count]), _) if !matches!(switch, 0 | 1 | 17) || !matches!(count, 11 | 16) => SonyLensStatus::Rejected,
        (_, Some(table)) if table.iter().all(|&v| v == 0) => SonyLensStatus::Neutral,
        (Some([0, _]), _) => SonyLensStatus::CameraCorrectionDisabled,
        // decoded and converted by the general map, which no camera's output has validated for this body
        _ => SonyLensStatus::NotValidatedModel,
    };
    false
}

/// The lens warp of a [`settle`]d record, whether or not the camera applied it (switch 0: Sony never warps the
/// stored samples). `None` for a rejected or neutral record and for a frame or curve the map does not cover.
pub(super) fn general_warp(record: &SonyExtendedDistortion, g: &SonyLensGeometry) -> Option<Opcode> {
    if !matches!(record.status, SonyLensStatus::CameraCorrectionDisabled | SonyLensStatus::NotValidatedModel) {
        return None;
    }
    let [_, count] = record.setting.value?;
    warp::general_opcode(record.table.value.as_ref()?, count, g)
}

fn words<'a>(dir: &Directory<'a>, entry: &'a [u8], kind: u16, count: u64) -> Result<&'a [u8], SonyFieldError> {
    let (actual_kind, actual_count) = dir.field(entry);
    if actual_kind != kind {
        return Err(SonyFieldError::FieldType(actual_kind));
    }
    if actual_count != count {
        return Err(SonyFieldError::Count(actual_count));
    }
    let len = usize::try_from(count).ok().and_then(|n| n.checked_mul(2)).ok_or(SonyFieldError::OutsideData)?;
    dir.values(entry, len).ok_or(SonyFieldError::OutsideData)
}

fn setting(dir: &Directory<'_>, entry: &[u8]) -> Read<[u16; 2]> {
    match words(dir, entry, SHORT, 2) {
        Ok(bytes) => Read::Valid(std::array::from_fn(|i| dir.order.read_u16(bytes, i * 2).unwrap_or(0))),
        Err(error) => Read::Rejected(error),
    }
}

fn table(dir: &Directory<'_>, entry: &[u8]) -> Read<[i16; 19]> {
    let bytes = match words(dir, entry, SSHORT, 20) {
        Ok(bytes) => bytes,
        Err(error) => return Read::Rejected(error),
    };
    let prefix = dir.order.read_u16(bytes, 0).unwrap_or(0) as i16;
    if prefix != 19 {
        return Read::Rejected(SonyFieldError::CountPrefix(prefix));
    }
    Read::Valid(std::array::from_fn(|i| dir.order.read_u16(bytes, (i + 1) * 2).unwrap_or(0) as i16))
}

pub(super) fn read(dir: Option<&Directory<'_>>) -> Option<SonyExtendedDistortion> {
    // No raw-IFD counterpart was observed. Reusing field diagnostics records the actual SR2
    // source, malformed data and repeated tags without inventing a copy-agreement claim.
    let absent = || Found { read: Read::Absent, repeated: false };
    let setting = combine(absent(), sr2_copy(dir, 0x7983, setting));
    let table = combine(Found { read: Read::Absent, repeated: false }, sr2_copy(dir, 0x7984, table));
    (SonyField::is_present(&setting) || SonyField::is_present(&table)).then_some(SonyExtendedDistortion {
        setting,
        table,
        status: SonyLensStatus::NotValidatedComponent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lens_metadata::{SonyCopy, SonySource};
    use craft_tiff::ByteOrder;

    fn block(order: ByteOrder, entries: &[(u16, u16, Vec<u16>)]) -> Vec<u8> {
        let refs: Vec<_> = entries.iter().map(|(tag, kind, values)| (*tag, *kind, values.as_slice())).collect();
        let mut bytes = crate::sr2::tests::block(1024, &refs, order);
        crate::sr2::tests::encrypt(&mut bytes);
        bytes
    }

    fn decode(bytes: &[u8], order: ByteOrder) -> SonyExtendedDistortion {
        read(Some(&Directory { entries: bytes, data: bytes, base: 1024, order, big: false })).unwrap()
    }

    fn frame() -> SonyLensGeometry {
        let crop = crate::Rect::new(12, 8, 7008, 4672);
        SonyLensGeometry {
            storage_width: 7040,
            storage_height: 4688,
            compression: 1,
            linear_ycbcr: false,
            native_3_2: true,
            image_crop: crop,
            reference: crop,
            sony_crop: Some(crop),
            default_crop: Some(crop),
        }
    }

    fn record(order: ByteOrder, switch: u16, count: u16, table: [i16; 19]) -> SonyExtendedDistortion {
        let values = std::iter::once(19).chain(table.map(|v| v as u16)).collect();
        decode(&block(order, &[(0x7983, SHORT, vec![switch, count]), (0x7984, SSHORT, values)]), order)
    }

    /// A barrel curve as a body with the record writes it.
    const BARREL: [i16; 19] = [0, -5, -24, -57, -105, -168, -246, -341, -451, -576, -713, -864, -1028, -1205, -1397, -1603, -1826, -2063, -2315];

    #[test]
    fn a_usable_record_owns_distortion_whatever_the_camera_did() {
        for order in [ByteOrder::Little, ByteOrder::Big] {
            for (switch, status) in
                [(1, SonyLensStatus::NotValidatedModel), (17, SonyLensStatus::NotValidatedModel), (0, SonyLensStatus::CameraCorrectionDisabled)]
            {
                let mut r = record(order, switch, 16, BARREL);
                assert!(!settle(&mut r));
                assert_eq!(r.status, status);
                // the same lens warp either way: the switch only says what the camera did to its own JPEG
                let mut on = record(order, 1, 16, BARREL);
                settle(&mut on);
                assert_eq!(general_warp(&r, &frame()), general_warp(&on, &frame()));
                assert!(general_warp(&r, &frame()).is_some());
            }
        }
    }

    #[test]
    fn neutral_broken_and_unknown_records_give_no_warp_and_keep_the_older_table_out() {
        let order = ByteOrder::Little;
        let mut neutral = record(order, 1, 16, [0; 19]);
        assert!(!settle(&mut neutral));
        assert_eq!(neutral.status, SonyLensStatus::Neutral);
        assert!(general_warp(&neutral, &frame()).is_none());
        for (switch, count) in [(2, 16), (1, 12), (1, 10), (1234, 5678)] {
            let mut r = record(order, switch, count, BARREL);
            assert!(!settle(&mut r), "{switch} {count}");
            assert_eq!(r.status, SonyLensStatus::Rejected);
            assert!(general_warp(&r, &frame()).is_none());
        }
        // half-present: a setting without its table
        let mut half = decode(&block(order, &[(0x7983, SHORT, vec![1, 16])]), order);
        assert!(!settle(&mut half));
        assert_eq!(half.status, SonyLensStatus::Rejected);
    }

    #[test]
    fn a_record_that_says_unavailable_leaves_the_older_table_in_charge() {
        let order = ByteOrder::Little;
        let mut r = decode(&block(order, &[(0x7983, SHORT, vec![u16::MAX; 2]), (0x7984, SSHORT, vec![0; 20])]), order);
        assert!(settle(&mut r));
        assert_eq!(r.status, SonyLensStatus::NoTable);
    }

    #[test]
    fn the_map_refuses_frames_and_curves_it_does_not_cover() {
        let mut r = record(ByteOrder::Little, 1, 16, BARREL);
        settle(&mut r);
        // an in-camera crop mode: about two thirds of the sensor inside the stored frame
        let mut crop_mode = frame();
        crop_mode.image_crop = crate::Rect::new(1200, 800, 4608, 3072);
        crop_mode.reference = crop_mode.image_crop;
        assert!(general_warp(&r, &crop_mode).is_none());
        let mut strong = BARREL;
        strong[18] = -8193;
        let mut r = record(ByteOrder::Little, 1, 16, strong);
        settle(&mut r);
        assert!(general_warp(&r, &frame()).is_none());
        let mut offset = BARREL;
        offset[0] = 1;
        let mut r = record(ByteOrder::Little, 1, 16, offset);
        settle(&mut r);
        assert!(general_warp(&r, &frame()).is_none());
    }

    #[test]
    fn extended_distortion_preserves_signed_knots_and_unknown_settings_in_both_byte_orders() {
        for order in [ByteOrder::Little, ByteOrder::Big] {
            let samples: [i16; 19] = std::array::from_fn(|i| i as i16 * 133 - 1200);
            let values = std::iter::once(19).chain(samples.map(|v| v as u16)).collect();
            let bytes = block(order, &[(0x7983, SHORT, vec![1234, 5678]), (0x7984, SSHORT, values)]);
            let record = decode(&bytes, order);
            assert_eq!(record.setting.value, Some([1234, 5678]));
            assert_eq!(record.table.value, Some(samples));
            assert_eq!(record.table.source, Some(SonySource::Sr2SubIfd));
            assert_eq!(record.table.raw_ifd, SonyCopy::Absent);
            assert_eq!(record.table.sr2, SonyCopy::Valid);
            assert_eq!(record.table.copies_agree, None);
            assert!(!record.table.sr2_repeated);
        }
    }

    #[test]
    fn extended_distortion_rejects_wrong_types_counts_and_prefixes() {
        for order in [ByteOrder::Little, ByteOrder::Big] {
            for (kind, values, error) in [
                (SHORT, vec![19; 20], SonyFieldError::FieldType(SHORT)),
                (SSHORT, vec![19; 19], SonyFieldError::Count(19)),
                (SSHORT, vec![19; 21], SonyFieldError::Count(21)),
                (SSHORT, vec![0; 20], SonyFieldError::CountPrefix(0)),
                (SSHORT, vec![16; 20], SonyFieldError::CountPrefix(16)),
                (SSHORT, vec![11; 20], SonyFieldError::CountPrefix(11)),
                (SSHORT, vec![u16::MAX; 20], SonyFieldError::CountPrefix(-1)),
            ] {
                let record = decode(&block(order, &[(0x7984, kind, values)]), order);
                assert_eq!(record.table.sr2, SonyCopy::Rejected(error));
                assert_eq!(record.table.value, None);
            }
            for (kind, values, error) in [
                (SSHORT, vec![0, 16], SonyFieldError::FieldType(SSHORT)),
                (SHORT, vec![0], SonyFieldError::Count(1)),
                (SHORT, vec![0, 16, 0], SonyFieldError::Count(3)),
            ] {
                let record = decode(&block(order, &[(0x7983, kind, values)]), order);
                assert_eq!(record.setting.sr2, SonyCopy::Rejected(error));
                assert_eq!(record.setting.value, None);
            }
        }
    }

    #[test]
    fn extended_distortion_repeated_or_outside_fields_do_not_replace_the_first() {
        for order in [ByteOrder::Little, ByteOrder::Big] {
            let mut entries = vec![(0x7984, SSHORT, vec![0; 20]), (0x7984, SSHORT, vec![19; 20])];
            let record = decode(&block(order, &entries), order);
            assert!(record.table.sr2_repeated);
            assert_eq!(record.table.value, None);
            assert_eq!(record.table.sr2, SonyCopy::Rejected(SonyFieldError::CountPrefix(0)));
            entries.truncate(1);
            let mut bytes = block(order, &entries);
            // First entry's out-of-line pointer is outside the decrypted SR2 block.
            let mut pointer = Vec::new();
            order.put_u32(&mut pointer, u32::MAX);
            bytes[10..14].copy_from_slice(&pointer);
            let record = decode(&bytes, order);
            assert_eq!(record.table.sr2, SonyCopy::Rejected(SonyFieldError::OutsideData));
        }
        assert!(read(None).is_none());
    }
}
