//! Original source-role fixtures framed from the public XCF record layouts.
use super::*;
use crate::tests::{TestImage, TestLayer};

fn saved_channel(marker: Option<&[u8]>, width: u32, code: u32, data: &[u8]) -> Vec<u8> {
    let mut w = W::new(26);
    for word in [2, 1, 1, code] {
        w.u32(word);
    }
    w.prop(17, &[0]);
    w.end();
    w.ptr(0);
    let slot = w.slot();
    w.ptr(0);
    w.ptr(0);
    let at = w.out.len();
    w.patch(slot, at);
    w.u32(width);
    w.u32(1);
    w.string("saved");
    if let Some(payload) = marker {
        w.prop(4, payload);
    }
    w.end();
    let pointer = w.slot();
    w.ptr(0);
    let sample_bytes = Precision::from_header(26, code).unwrap().bytes;
    let hierarchy = hierarchy(&mut w, width, 1, sample_bytes, data, Compression::None);
    w.patch(pointer, hierarchy);
    w.out
}

#[test]
fn only_empty_saved_selection_markers_establish_selection_transfer_and_canvas_extent() {
    for code in [100, 150, 650] {
        let data = if code == 650 { [0.25f32.to_be_bytes(), 0.75f32.to_be_bytes()].concat() } else { vec![64, 192] };
        for marker in [None, Some(b"".as_slice()), Some(b"bad".as_slice())] {
            let bytes = saved_channel(marker, 2, code, &data);
            let reader = Xcf::parse(&bytes).unwrap();
            let mut budget = data.len();
            let metadata = reader.meta_budget.get();
            let mut calls = 0;
            let result = reader.visit_channel_tiles(0, SampleOptions::default(), &mut budget, GO, |tile| {
                calls += 1;
                assert_eq!(tile.data, data);
                let selected = marker == Some(b"".as_slice());
                assert_eq!(tile.drawable.precision.transfer, if selected || code != 150 { Transfer::Linear } else { Transfer::Nonlinear });
                assert_eq!(tile.drawable.transfer_is_assumed, !selected || code == 150);
                Ok(())
            });
            if marker == Some(b"bad".as_slice()) {
                assert!(matches!(result, Err(Error::Invalid(ref message)) if message == "selection marker payload must be empty"));
                assert_eq!(calls, 0);
                assert_eq!(budget, data.len());
                assert_eq!(reader.meta_budget.get(), metadata);
            } else {
                let (channel, info) = result.unwrap();
                assert_eq!(channel.selection, marker.is_some());
                assert_eq!(info.transfer_is_assumed, marker.is_none() || code == 150);
                assert_eq!(calls, 1);
                assert_eq!(budget, 0);
            }
        }
    }
    let bytes = saved_channel(Some(&[]), 1, 150, &[64]);
    let reader = Xcf::parse(&bytes).unwrap();
    let mut budget = 1;
    assert!(matches!(reader.visit_channel_tiles(0, SampleOptions::default(), &mut budget, GO, |_| panic!("mismatched selection reached callback")),
        Err(Error::Invalid(message)) if message == "selection dimensions do not match the canvas"));
    assert_eq!(budget, 1);
}

#[test]
fn stray_selection_markers_do_not_change_layer_mask_role_or_owner_dimensions() {
    for marker in [b"".as_slice(), b"bad".as_slice()] {
        let mut image = TestImage::rgb8(4, 4);
        image.version = 26;
        let mut layer = TestLayer::rgba("owner", 2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]);
        layer.mask = Some(vec![64, 192]);
        image.layers.push(layer);
        let mut bytes = image.write();
        let (slot, hierarchy) = {
            let reader = Xcf::parse(&bytes).unwrap();
            let mut owner = reader.c.at(reader.layer_sources[0]);
            owner.take(12).unwrap();
            owner.string().unwrap();
            props(&mut owner).unwrap();
            owner.ptr().unwrap();
            let slot = owner.pos;
            let mut mask = reader.c.at(reader.image.layers[0].mask);
            mask.take(8).unwrap();
            mask.string().unwrap();
            props(&mut mask).unwrap();
            (slot, mask.ptr().unwrap())
        };
        let mut record = W::new(26);
        record.out.clear();
        record.u32(2);
        record.u32(1);
        record.string("owner mask");
        record.prop(4, marker);
        record.end();
        record.ptr(hierarchy);
        record.ptr(0);
        let target = bytes.len();
        bytes.extend(record.out);
        bytes[slot..slot + 8].copy_from_slice(&(target as u64).to_be_bytes());
        let reader = Xcf::parse(&bytes).unwrap();
        assert!(reader.layer_mask(0, &mut 1024, GO).unwrap().is_some(), "legacy owner mask remains decodable");
        let mut budget = 2;
        let (channel, info) = reader
            .visit_layer_mask_tiles(0, SampleOptions::default(), &mut budget, GO, |tile| {
                assert_eq!(tile.data, [64, 192]);
                assert_eq!(tile.drawable.precision.transfer, Transfer::Linear);
                assert!(!tile.drawable.transfer_is_assumed);
                Ok(())
            })
            .unwrap()
            .unwrap();
        assert!(!channel.selection);
        assert_eq!((info.width, info.height), (2, 1));
        assert_eq!(budget, 0);
        let mut mismatched = bytes;
        mismatched[target..target + 4].copy_from_slice(&1u32.to_be_bytes());
        let reader = Xcf::parse(&mismatched).unwrap();
        let mut budget = 2;
        assert!(matches!(reader.visit_layer_mask_tiles(0, SampleOptions::default(), &mut budget, GO, |_| panic!("owner mismatch reached callback")),
            Err(Error::Invalid(message)) if message == "mask dimensions do not match their owner"));
        assert_eq!(budget, 2);
    }
}

#[test]
fn shared_saved_channel_fixtures_have_the_versioned_effect_tail() {
    for version in [25, 26] {
        let mut image = TestImage::rgb8(2, 1);
        image.version = version;
        image.channels.push(("saved", vec![13, 71], false));
        image.channels.push(("selection", vec![29, 193], true));
        let bytes = image.write();
        let reader = Xcf::parse(&bytes).unwrap();
        for (index, expected) in [[13, 71], [29, 193]].into_iter().enumerate() {
            let (channel, pixels) = reader.channel(index, &mut 1024, GO).unwrap();
            assert_eq!(channel.selection, index == 1);
            assert_eq!((pixels.width, pixels.height), (2, 1));
            let (channel, _) = reader
                .visit_channel_tiles(index, SampleOptions::default(), &mut 1024, GO, |tile| {
                    assert_eq!(tile.data, expected);
                    Ok(())
                })
                .unwrap();
            assert_eq!(channel.selection, index == 1);
        }
        if version == 25 {
            // Decode through the hierarchy immediately following the record. If
            // the writer inserts v26 framing here, this points at it and fails.
            let mut cursor = reader.c.at(reader.channels[1]);
            cursor.take(8).unwrap();
            cursor.string().unwrap();
            props(&mut cursor).unwrap();
            let hierarchy_slot = cursor.pos;
            cursor.ptr().unwrap();
            let record_end = cursor.pos;
            let mut file = W { out: bytes, wide: true };
            file.patch(hierarchy_slot, record_end);
            let reader = Xcf::parse(&file.out).unwrap();
            reader
                .visit_channel_tiles(1, SampleOptions::default(), &mut 2, GO, |tile| {
                    assert_eq!(tile.data, [29, 193]);
                    Ok(())
                })
                .unwrap();
        }
    }
}
