//! Sony's camera-menu compensation settings, independently of raw/SR2 correction tables.
//! Public layout: https://exiftool.org/TagNames/Sony.html, tags 0x2011..0x2013 (int32u).
//! This parser does not apply corrections or infer which processing stage used the settings.

use crate::lens_metadata::{SonyCameraSetting, SonyCameraSettings, SonyFieldError};
use craft_tiff::{Ifd, Value};

fn setting(note: &Ifd, tag: u16, model: &str) -> SonyCameraSetting {
    let Some(entry) = note.get(tag) else { return SonyCameraSetting::Absent };
    let Value::Long(values) = &entry.value else {
        return SonyCameraSetting::Rejected(SonyFieldError::FieldType(entry.field_type() as u16));
    };
    let &[value] = values.as_slice() else {
        return SonyCameraSetting::Rejected(SonyFieldError::Count(entry.count() as u64));
    };
    match value {
        0 => SonyCameraSetting::Off,
        2 => SonyCameraSetting::Auto,
        // Only this body/component has a capture-backed Low interpretation. Never turn an unknown
        // value on another body, or a CA/distortion value, into an asserted shading mode.
        3 if tag == 0x2011 && model == "ILCE-1M2" => SonyCameraSetting::Low,
        u32::MAX => SonyCameraSetting::Unavailable,
        other => SonyCameraSetting::Unknown(other),
    }
}

pub(super) fn read(note: &Ifd, model: &str) -> Option<SonyCameraSettings> {
    let settings = SonyCameraSettings {
        shading: setting(note, 0x2011, model),
        chromatic_aberration: setting(note, 0x2012, model),
        distortion: setting(note, 0x2013, model),
    };
    (settings != SonyCameraSettings::default()).then_some(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_tiff::Entry;

    fn note(value: Value) -> Ifd {
        Ifd { entries: vec![Entry { tag: 0x2011, value, offset: 0 }], ..Default::default() }
    }

    #[test]
    fn unknown_and_malformed_flags_never_become_off() {
        for (value, expected) in [
            (Value::Long(vec![0]), SonyCameraSetting::Off),
            (Value::Long(vec![2]), SonyCameraSetting::Auto),
            (Value::Long(vec![u32::MAX]), SonyCameraSetting::Unavailable),
            (Value::Long(vec![1]), SonyCameraSetting::Unknown(1)),
            (Value::Long(vec![65536]), SonyCameraSetting::Unknown(65536)),
            (Value::Short(vec![0]), SonyCameraSetting::Rejected(SonyFieldError::FieldType(3))),
            (Value::SLong(vec![-1]), SonyCameraSetting::Rejected(SonyFieldError::FieldType(9))),
            (Value::Long(vec![]), SonyCameraSetting::Rejected(SonyFieldError::Count(0))),
            (Value::Long(vec![0, 2]), SonyCameraSetting::Rejected(SonyFieldError::Count(2))),
        ] {
            let settings = read(&note(value), "ILCE-1M2").unwrap();
            assert_eq!(settings.shading, expected);
            assert_eq!(settings.chromatic_aberration, SonyCameraSetting::Absent);
            assert_eq!(settings.distortion, SonyCameraSetting::Absent);
        }
        assert_eq!(read(&Ifd::default(), "ILCE-1M2"), None);
    }

    #[test]
    fn low_is_only_identified_for_the_measured_body_and_component() {
        let shading = note(Value::Long(vec![3]));
        assert_eq!(read(&shading, "ILCE-1M2").unwrap().shading, SonyCameraSetting::Low);
        for model in ["", "ILCE-1", "ILCE-7RM5"] {
            assert_eq!(read(&shading, model).unwrap().shading, SonyCameraSetting::Unknown(3));
        }
        let image = crate::probe_info(&fixture(craft_tiff::ByteOrder::Little, false, [3, 3, 3], true)).unwrap();
        assert_eq!(
            image.lens_metadata.sony_camera_settings,
            Some(SonyCameraSettings {
                shading: SonyCameraSetting::Low,
                chromatic_aberration: SonyCameraSetting::Unknown(3),
                distortion: SonyCameraSetting::Unknown(3),
            })
        );
        assert!(image.opcodes.list3.is_empty());
    }

    fn fixture(order: craft_tiff::ByteOrder, header: bool, flags: [u32; 3], table_setting: bool) -> Vec<u8> {
        use craft_tiff::{IfdBuilder, ImageData, TiffWriter, tags as t};
        let mut maker = if header { b"SONY DSC \0\0\0".to_vec() } else { Vec::new() };
        order.put_u16(&mut maker, 3);
        for (tag, value) in [0x2011, 0x2012, 0x2013].into_iter().zip(flags) {
            order.put_u16(&mut maker, tag);
            order.put_u16(&mut maker, 4);
            order.put_u32(&mut maker, 1);
            order.put_u32(&mut maker, value);
        }
        order.put_u32(&mut maker, 0);
        let mut raw = IfdBuilder::new();
        raw.set(t::MAKE, Value::Ascii("SONY".into()));
        raw.set(t::MODEL, Value::Ascii("ILCE-1M2".into()));
        raw.set(t::IMAGE_WIDTH, Value::Long(vec![32]));
        raw.set(t::IMAGE_LENGTH, Value::Long(vec![24]));
        raw.set(t::BITS_PER_SAMPLE, Value::Short(vec![16]));
        raw.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![1]));
        raw.set(t::PHOTOMETRIC, Value::Short(vec![t::photometric::CFA]));
        raw.set(t::COMPRESSION, Value::Short(vec![1]));
        if table_setting {
            raw.set(0x7031, Value::Short(vec![272]));
        }
        raw.set_child(t::EXIF_IFD, IfdBuilder::new().with(t::MAKER_NOTE, Value::Undefined(maker)));
        raw.set_image(ImageData::Strips { rows_per_strip: 24, strips: vec![vec![0; 32 * 24 * 2]] });
        TiffWriter::new(order, false).write(&[raw]).unwrap()
    }

    #[test]
    fn capture_flags_survive_header_and_full_decode_without_applying_corrections() {
        use crate::lens_metadata::SonySettingMeaning;
        use craft_tiff::ByteOrder::{Big, Little};
        for order in [Little, Big] {
            for header in [false, true] {
                let off = fixture(order, header, [0, 0, 0], true);
                let on = fixture(order, header, [2, 2, 2], true);
                let decoded_off = crate::decode(&off).unwrap();
                let decoded_on = crate::decode(&on).unwrap();
                for (file, image, flag) in [(&off, &decoded_off, SonyCameraSetting::Off), (&on, &decoded_on, SonyCameraSetting::Auto)] {
                    let info = crate::probe_info(file).unwrap();
                    assert_eq!(info.lens_metadata, image.lens_metadata);
                    assert_eq!(image.lens_metadata.sony.as_ref().unwrap().shading.setting.value.unwrap().meaning, SonySettingMeaning::Auto);
                    assert_eq!(
                        image.lens_metadata.sony_camera_settings,
                        Some(SonyCameraSettings { shading: flag, chromatic_aberration: flag, distortion: flag })
                    );
                    assert!(image.opcodes.list1.is_empty() && image.opcodes.list2.is_empty() && image.opcodes.list3.is_empty());
                }
                assert_eq!(decoded_off.data, decoded_on.data);
                let without_tables = crate::probe_info(&fixture(order, header, [0, 2, u32::MAX], false)).unwrap();
                assert!(without_tables.lens_metadata.sony.is_none());
                assert_eq!(
                    without_tables.lens_metadata.sony_camera_settings,
                    Some(SonyCameraSettings {
                        shading: SonyCameraSetting::Off,
                        chromatic_aberration: SonyCameraSetting::Auto,
                        distortion: SonyCameraSetting::Unavailable,
                    })
                );
            }
        }
    }
}
