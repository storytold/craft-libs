//! Independently chosen precision codes, component widths and byte orders.
use super::*;
use crate::Compression;
use crate::tests::{W, hierarchy};

fn f32_source_tile(data: &[u8], order: SampleOrder) -> SourceTile<'_> {
    SourceTile {
        x: 0,
        y: 0,
        width: 2,
        height: 1,
        drawable: SourceDrawable {
            width: 2,
            height: 1,
            channels: 4,
            indexed: false,
            precision: SourcePrecision { storage: Precision { bytes: 4, float: true, linear: true }, transfer: Transfer::Linear },
            sample_order: order,
        },
        data,
    }
}

#[test]
fn bulk_f32_delivery_preserves_every_source_bit_in_either_order() {
    // Includes signed zero, signed HDR, infinities, a subnormal and a NaN payload.
    let bits = [0x0000_0000u32, 0x8000_0000, 0xc020_0000, 0x4080_0000, 0x7f80_0000, 0xff80_0000, 0x0000_0001, 0x7fc1_2345];
    for order in [SampleOrder::BigEndian, SampleOrder::LittleEndian] {
        let bytes: Vec<_> =
            bits.iter().flat_map(|word| if order == SampleOrder::LittleEndian { word.to_le_bytes() } else { word.to_be_bytes() }).collect();
        let tile = f32_source_tile(&bytes, order);
        let mut samples = tile.f32_samples().unwrap().unwrap();
        assert_eq!(samples.len(), bits.len());
        assert_eq!(samples.next().unwrap().to_bits(), bits[0]);
        assert_eq!(samples.len(), bits.len() - 1);
        assert_eq!(samples.map(f32::to_bits).collect::<Vec<_>>(), bits[1..]);
        for (index, expected) in bits.into_iter().enumerate() {
            let SourceSample::F32(scalar) = tile.sample(index).unwrap() else { panic!("F32") };
            assert_eq!(scalar.to_bits(), expected, "bulk and scalar retain source bits");
        }
    }
}

#[test]
fn bulk_f32_delivery_requires_exact_checked_layout_and_does_not_reinterpret_other_storage() {
    for size in [0, 1, 28, 31, 33, 36] {
        let bytes = vec![0; size];
        assert!(f32_source_tile(&bytes, SampleOrder::BigEndian).f32_samples().is_err(), "size {size}");
    }
    let bytes = [0; 32];
    assert_eq!(f32_source_tile(&bytes, SampleOrder::BigEndian).f32_samples().unwrap().unwrap().len(), 8);
    let mut overflow = f32_source_tile(&[], SampleOrder::BigEndian);
    overflow.width = u32::MAX;
    overflow.height = u32::MAX;
    overflow.drawable.channels = usize::MAX;
    assert!(overflow.f32_samples().is_err());
    for storage in [
        Precision::GAMMA8,
        Precision { bytes: 4, float: false, linear: false },
        Precision { bytes: 2, float: true, linear: false },
        Precision { bytes: 8, float: true, linear: false },
    ] {
        let mut tile = f32_source_tile(&[], SampleOrder::LittleEndian);
        tile.drawable.precision.storage = storage;
        assert!(tile.f32_samples().unwrap().is_none());
    }
}

struct Source {
    version: u32,
    code: u32,
    bytes: usize,
    width: u32,
    height: u32,
    base: u32,
    kind: u32,
    channels: usize,
    compression: Compression,
    data: Vec<u8>,
    floating: bool,
    mask: Option<Vec<u8>>,
    channel: Option<Vec<u8>>,
}
impl Source {
    fn gray(code: u32, bytes: usize, data: Vec<u8>) -> Self {
        Self {
            version: 26,
            code,
            bytes,
            width: 1,
            height: 1,
            base: 1,
            kind: 2,
            channels: 1,
            compression: Compression::None,
            data,
            floating: false,
            mask: None,
            channel: None,
        }
    }
    fn write(&self) -> Vec<u8> {
        let mut w = W::new(self.version);
        for word in [self.width, self.height, self.base] {
            w.u32(word);
        }
        if self.version >= 4 {
            w.u32(self.code);
        }
        let compression = match self.compression {
            Compression::None => 0,
            Compression::Rle => 1,
            Compression::Zlib => 2,
        };
        w.prop(17, &[compression]);
        w.end();
        let layer = w.slot();
        w.ptr(0);
        let channel = self.channel.as_ref().map(|_| w.slot());
        w.ptr(0);
        if self.version >= 18 {
            w.ptr(0);
        }
        let at = w.out.len();
        w.patch(layer, at);
        for word in [self.width, self.height, self.kind] {
            w.u32(word);
        }
        w.string("Source");
        if self.floating {
            w.prop(5, &[0; 8]);
        }
        w.end();
        let pixels = w.slot();
        let mask = w.slot();
        if self.version >= 20 {
            w.ptr(0);
        }
        let at = hierarchy(&mut w, self.width, self.height, self.channels * self.bytes, &self.data, self.compression);
        w.patch(pixels, at);
        for (slot, name, data, selection) in self
            .mask
            .as_ref()
            .map(|data| (mask, "Mask", data, false))
            .into_iter()
            .chain(channel.zip(self.channel.as_ref()).map(|(slot, data)| (slot, "Selection", data, true)))
        {
            let at = w.out.len();
            w.patch(slot, at);
            w.u32(self.width);
            w.u32(self.height);
            w.string(name);
            if selection {
                w.prop(4, &[]);
            }
            w.end();
            let pixels = w.slot();
            let at = hierarchy(&mut w, self.width, self.height, self.bytes, data, self.compression);
            w.patch(pixels, at);
        }
        w.out
    }
}
const GO: &dyn Fn() -> bool = &|| true;
fn options(order: SampleOrder) -> SampleOptions {
    SampleOptions { historical_order: HistoricalOrder::Explicit(order) }
}

#[test]
fn historical_sample_choice_is_independent_of_structural_pointer_width() {
    for (version, code) in [(4, 1), (5, 250), (6, 250), (7, 250), (10, 250), (11, 250)] {
        for order in [SampleOrder::BigEndian, SampleOrder::LittleEndian] {
            let expected = [0x1234u16, 0xabcd];
            let data =
                expected.into_iter().flat_map(|v| if order == SampleOrder::LittleEndian { v.to_le_bytes() } else { v.to_be_bytes() }).collect();
            let mut source = Source::gray(code, 2, data);
            source.width = 2;
            source.version = version;
            let bytes = source.write();
            let reader = Xcf::parse(&bytes).unwrap();
            assert_eq!(reader.historical_sample_order_request().unwrap().version, version);
            let mut budget = 4;
            let mut visited = false;
            let missing = SampleOptions { historical_order: HistoricalOrder::RequireExplicit };
            assert!(
                reader
                    .visit_layer_tiles(0, missing, &mut budget, GO, |_| {
                        visited = true;
                        Ok(())
                    })
                    .is_err()
            );
            assert!(!visited);
            assert_eq!(budget, 4);
            let mut actual = Vec::new();
            reader
                .visit_layer_tiles(0, options(order), &mut budget, GO, |tile| {
                    for i in 0..2 {
                        actual.push(tile.sample(i)?);
                    }
                    Ok(())
                })
                .unwrap();
            assert_eq!(actual, expected.map(SourceSample::U16));
            assert_eq!(budget, 0);
            // Additive defaults retain the old BE-normalized f32 API exactly.
            let mut legacy_budget = usize::MAX;
            let legacy = reader.layer_pixels(0, &mut legacy_budget, GO).unwrap().unwrap();
            let mut default = Vec::new();
            let mut default_budget = usize::MAX;
            reader
                .visit_layer_tiles(0, SampleOptions::default(), &mut default_budget, GO, |tile| {
                    for i in 0..2 {
                        default.push(tile.sample(i)?.normalized_f64() as f32);
                    }
                    Ok(())
                })
                .unwrap();
            assert_eq!(default, legacy.data);
        }
    }
}

#[test]
fn modern_samples_ignore_historical_overrides_and_keep_perceptual_transfer() {
    for code in [250, 275] {
        let mut source = Source::gray(code, 2, 0x1234u16.to_be_bytes().to_vec());
        source.version = 12;
        let bytes = source.write();
        let reader = Xcf::parse(&bytes).unwrap();
        assert!(reader.historical_sample_order_request().is_none());
        let mut budget = 2;
        let info = reader
            .visit_layer_tiles(0, options(SampleOrder::LittleEndian), &mut budget, GO, |tile| {
                assert_eq!(tile.drawable.sample_order, SampleOrder::BigEndian);
                assert_eq!(tile.sample(0)?, SourceSample::U16(0x1234));
                Ok(())
            })
            .unwrap()
            .unwrap();
        assert_eq!(info.precision.transfer, if code == 275 { Transfer::Perceptual } else { Transfer::Nonlinear });
    }
}

#[test]
fn typed_samples_keep_uint32_endpoints_f64_precision_and_signed_hdr() {
    let cases = [
        (350, 4, [0u32, u32::MAX].into_iter().flat_map(u32::to_be_bytes).collect::<Vec<_>>()),
        (550, 2, [0x8000u16, 0x4200].into_iter().flat_map(u16::to_be_bytes).collect()),
        (650, 4, [-2.5f32, 4.25].into_iter().flat_map(f32::to_be_bytes).collect()),
        (750, 8, [-0.0f64, 1.0 + f64::EPSILON].into_iter().flat_map(f64::to_be_bytes).collect()),
    ];
    for (code, width, data) in cases {
        let mut source = Source::gray(code, width, data.clone());
        source.width = 2;
        let bytes = source.write();
        let reader = Xcf::parse(&bytes).unwrap();
        let mut budget = data.len();
        let mut actual = Vec::new();
        reader
            .visit_layer_tiles(0, SampleOptions::default(), &mut budget, GO, |tile| {
                assert_eq!(tile.data, data);
                for i in 0..2 {
                    actual.push(tile.sample(i)?);
                }
                assert!(tile.sample(2).is_err());
                assert!(tile.sample(usize::MAX).is_err());
                Ok(())
            })
            .unwrap();
        match code {
            350 => assert_eq!(actual, [SourceSample::U32(0), SourceSample::U32(u32::MAX)]),
            550 => assert_eq!(actual, [SourceSample::F16(0x8000), SourceSample::F16(0x4200)]),
            650 => assert_eq!(actual, [SourceSample::F32(-2.5), SourceSample::F32(4.25)]),
            750 => {
                assert_eq!(actual[0].normalized_f64().to_bits(), (-0.0f64).to_bits());
                assert_eq!(actual[1], SourceSample::F64(1.0 + f64::EPSILON));
            }
            _ => panic!("unexpected precision test case"),
        }
        assert_eq!(budget, 0);
    }
}

#[test]
fn layer_masks_and_channels_share_order_and_exact_source_budget() {
    let mut source = Source::gray(650, 4, (-2.5f32).to_be_bytes().to_vec());
    source.mask = Some(1.25f32.to_be_bytes().to_vec());
    source.channel = Some((-0.25f32).to_be_bytes().to_vec());
    let bytes = source.write();
    let reader = Xcf::parse(&bytes).unwrap();
    let mut budget = 12;
    reader
        .visit_layer_tiles(0, SampleOptions::default(), &mut budget, GO, |tile| {
            assert_eq!(tile.sample(0)?, SourceSample::F32(-2.5));
            Ok(())
        })
        .unwrap();
    let (mask, _) = reader
        .visit_layer_mask_tiles(0, SampleOptions::default(), &mut budget, GO, |tile| {
            assert_eq!(tile.sample(0)?, SourceSample::F32(1.25));
            Ok(())
        })
        .unwrap()
        .unwrap();
    assert_eq!(mask.name, "Mask");
    let (channel, _) = reader
        .visit_channel_tiles(0, SampleOptions::default(), &mut budget, GO, |tile| {
            assert_eq!(tile.sample(0)?, SourceSample::F32(-0.25));
            Ok(())
        })
        .unwrap();
    assert!(channel.selection);
    assert_eq!(budget, 0);
}

#[test]
fn edge_tiles_have_bounded_scratch_and_budget_refusal_precedes_callbacks() {
    let mut source = Source::gray(150, 1, (0..65 * 2).map(|i| (i % 251) as u8).collect());
    source.width = 65;
    source.height = 2;
    for compression in [Compression::None, Compression::Rle, Compression::Zlib] {
        source.compression = compression;
        let bytes = source.write();
        let reader = Xcf::parse(&bytes).unwrap();
        let mut seen = Vec::new();
        let mut budget = 129;
        assert!(
            reader.visit_layer_tiles(0, SampleOptions::default(), &mut budget, GO, |_| { panic!("budget refusal precedes publication") }).is_err()
        );
        assert_eq!(budget, 129);
        budget = 130;
        reader
            .visit_layer_tiles(0, SampleOptions::default(), &mut budget, GO, |tile| {
                seen.push((tile.x, tile.y, tile.width, tile.height, tile.data.len()));
                assert!(tile.data.len() <= 64 * 64);
                Ok(())
            })
            .unwrap();
        assert_eq!(seen, [(0, 0, 64, 2, 128), (64, 0, 1, 2, 2)]);
        assert_eq!(budget, 0);
        let mut callbacks = 0;
        let mut budget = 130;
        assert!(matches!(
            reader.visit_layer_tiles(0, SampleOptions::default(), &mut budget, &|| false, |_| {
                callbacks += 1;
                Ok(())
            }),
            Err(Error::Cancelled)
        ));
        assert_eq!(callbacks, 0);
        assert_eq!(budget, 130);
        let go = std::cell::Cell::new(true);
        let mut callbacks = 0;
        let mut budget = 130;
        assert!(matches!(
            reader.visit_layer_tiles(0, SampleOptions::default(), &mut budget, &|| go.get(), |_| {
                callbacks += 1;
                go.set(false);
                Ok(())
            }),
            Err(Error::Cancelled)
        ));
        assert_eq!(callbacks, 1, "cancellation between tiles stops later publication");
        assert_eq!(budget, 0, "admitted work stays spent after cancellation");
        let mut budget = 130;
        assert!(matches!(reader.visit_layer_tiles(0, SampleOptions::default(), &mut budget, GO, |_| Err(Error::Cancelled)), Err(Error::Cancelled)));
        assert_eq!(budget, 0, "already admitted work is not refunded");
    }
}

#[test]
fn opaque_rgb_and_indexed_source_components_are_not_synthesized_or_expanded() {
    let mut source = Source::gray(650, 4, [-2.0f32, 3.0, 0.5].into_iter().flat_map(f32::to_be_bytes).collect());
    source.base = 0;
    source.kind = 0;
    source.channels = 3;
    let bytes = source.write();
    let reader = Xcf::parse(&bytes).unwrap();
    let mut budget = 12;
    reader
        .visit_layer_tiles(0, SampleOptions::default(), &mut budget, GO, |tile| {
            assert_eq!(tile.drawable.channels, 3);
            assert_eq!(tile.sample(0)?, SourceSample::F32(-2.0));
            assert_eq!(tile.sample(1)?, SourceSample::F32(3.0));
            Ok(())
        })
        .unwrap();
    let mut source = Source::gray(150, 1, vec![7, 128]);
    source.base = 2;
    source.kind = 5;
    source.channels = 2;
    let bytes = source.write();
    let reader = Xcf::parse(&bytes).unwrap();
    let mut budget = 2;
    reader
        .visit_layer_tiles(0, SampleOptions::default(), &mut budget, GO, |tile| {
            assert!(tile.drawable.indexed);
            assert_eq!(tile.drawable.channels, 2);
            assert_eq!(tile.data, [7, 128]);
            Ok(())
        })
        .unwrap();
    let mut legacy_budget = 16;
    let legacy = reader.layer_pixels(0, &mut legacy_budget, GO).unwrap().unwrap();
    assert_eq!(legacy.channels, 4);
    assert_eq!(legacy.data, [7. / 255., 7. / 255., 7. / 255., 128. / 255.]);
}

#[test]
fn zero_first_tile_delivers_bounded_zero_source_tiles_without_following_later_bytes() {
    let mut source = Source::gray(150, 1, vec![7; 65]);
    source.width = 65;
    let mut bytes = source.write();
    let parsed = Xcf::parse(&bytes).unwrap();
    let mut hierarchy = parsed.c.at(parsed.image.layers[0].hierarchy);
    hierarchy.take(12).unwrap();
    let level = hierarchy.ptr().unwrap();
    let first = level + 8;
    bytes[first..first + 8].fill(0);
    let reader = Xcf::parse(&bytes).unwrap();
    let mut budget = 65;
    let mut seen = Vec::new();
    reader
        .visit_layer_tiles(0, SampleOptions::default(), &mut budget, GO, |tile| {
            assert!(tile.data.iter().all(|v| *v == 0));
            seen.push((tile.x, tile.width, tile.data.len()));
            Ok(())
        })
        .unwrap();
    assert_eq!(seen, [(0, 64, 64), (64, 1, 1)]);
    assert_eq!(budget, 0);
    let mut legacy_budget = 260;
    assert!(reader.layer_pixels(0, &mut legacy_budget, GO).is_err(), "legacy compatibility keeps its previous zero-first rejection");
}

#[test]
fn historical_float_layers_masks_and_channels_use_one_explicit_order() {
    for (version, code, sample_bytes) in [(4, 3, 2), (5, 400, 2), (11, 550, 2), (4, 4, 4), (5, 500, 4), (11, 650, 4)] {
        for order in [SampleOrder::BigEndian, SampleOrder::LittleEndian] {
            let little = order == SampleOrder::LittleEndian;
            let encode = |half: u16, single: f32| {
                if sample_bytes == 2 {
                    if little { half.to_le_bytes().to_vec() } else { half.to_be_bytes().to_vec() }
                } else if little {
                    single.to_le_bytes().to_vec()
                } else {
                    single.to_be_bytes().to_vec()
                }
            };
            let mut source = Source::gray(code, sample_bytes, encode(0xc000, -2.0));
            source.version = version;
            source.mask = Some(encode(0x3d00, 1.25));
            source.channel = Some(encode(0xb400, -0.25));
            let bytes = source.write();
            let reader = Xcf::parse(&bytes).unwrap();
            let mut budget = sample_bytes * 3;
            reader
                .visit_layer_tiles(0, options(order), &mut budget, GO, |tile| {
                    assert_eq!(tile.sample(0)?.normalized_f64(), -2.0);
                    Ok(())
                })
                .unwrap();
            reader
                .visit_layer_mask_tiles(0, options(order), &mut budget, GO, |tile| {
                    assert_eq!(tile.sample(0)?.normalized_f64(), 1.25);
                    Ok(())
                })
                .unwrap();
            reader
                .visit_channel_tiles(0, options(order), &mut budget, GO, |tile| {
                    assert_eq!(tile.sample(0)?.normalized_f64(), -0.25);
                    Ok(())
                })
                .unwrap();
            assert_eq!(budget, 0);
        }
    }
}

#[test]
fn oversized_caller_budget_cannot_lift_the_source_drawable_limit() {
    let source = Source::gray(150, 1, vec![1]);
    let mut bytes = source.write();
    let hierarchy = Xcf::parse(&bytes).unwrap().image.layers[0].hierarchy;
    bytes[hierarchy..hierarchy + 4].copy_from_slice(&crate::MAX_SIDE.to_be_bytes());
    bytes[hierarchy + 4..hierarchy + 8].copy_from_slice(&crate::MAX_SIDE.to_be_bytes());
    let mut reader = Xcf::parse(&bytes).unwrap();
    reader.image.layers[0].width = crate::MAX_SIDE;
    reader.image.layers[0].height = crate::MAX_SIDE;
    let mut budget = usize::MAX;
    let error = reader.visit_layer_tiles(0, SampleOptions::default(), &mut budget, GO, |_| panic!("refuse before following the level")).unwrap_err();
    assert!(error.to_string().contains("source pixel limit"));
    assert_eq!(budget, usize::MAX);
}

#[test]
fn indexed_floating_gray_alpha_samples_are_not_palette_indices() {
    for floating in [false, true] {
        let mut source = Source::gray(150, 1, vec![64, 192]);
        source.base = 2;
        source.kind = 3;
        source.channels = 2;
        source.floating = floating;
        let bytes = source.write();
        let xcf = Xcf::parse(&bytes).unwrap();
        let mut visited = Vec::new();
        let mut budget = 2;
        let result = xcf.visit_layer_tiles(0, SampleOptions::default(), &mut budget, &|| true, |tile| {
            assert!(!tile.drawable.indexed);
            visited.extend_from_slice(tile.data);
            Ok(())
        });
        if floating {
            let drawable = result.unwrap().unwrap();
            assert_eq!(drawable.channels, 2);
            assert!(!drawable.indexed);
            assert_eq!(visited, [64, 192]);
            assert_eq!(budget, 0);
        } else {
            assert!(result.is_err());
            assert!(visited.is_empty());
            assert_eq!(budget, 2);
        }
    }
}

#[test]
fn source_tiles_before_the_level_table_cannot_consume_its_dimensions() {
    for complete in [false, true] {
        let source = Source::gray(150, 1, vec![0]);
        let mut file = W { out: source.write(), wide: true };
        let hierarchy = file.out.len();
        for value in [2, 1, 1] {
            file.u32(value);
        }
        let level_slot = file.slot();
        let pixels_at = file.out.len();
        file.out.push(22);
        if complete {
            file.out.push(44);
        }
        let level = file.out.len();
        file.patch(level_slot, level);
        file.u32(2);
        file.u32(1);
        file.ptr(pixels_at);
        file.ptr(0);
        let xcf = Xcf::parse(&file.out).unwrap();
        let mut visited = Vec::new();
        let result = xcf.visit_hierarchy(
            VisitTarget { pointer: hierarchy, channels: 1, indexed: false, dimensions: Some((2, 1)) },
            SampleOptions::default(),
            &mut 2,
            &|| true,
            |tile| {
                visited.extend_from_slice(tile.data);
                Ok(())
            },
        );
        if complete {
            result.unwrap();
            assert_eq!(visited, [22, 44]);
        } else {
            assert!(result.is_err());
            assert!(visited.is_empty());
        }
    }
}
