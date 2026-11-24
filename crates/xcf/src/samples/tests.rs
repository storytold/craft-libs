//! Independently chosen precision codes, component widths and byte orders.
use super::*;
use crate::Compression;
use crate::props;
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
            transfer_is_assumed: false,
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
    channel_selection: bool,
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
            channel_selection: true,
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
        let floating_target = self.floating.then(|| {
            w.u32(5);
            w.u32(if w.wide { 8 } else { 4 });
            w.slot()
        });
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
            .chain(channel.zip(self.channel.as_ref()).map(|(slot, data)| (slot, "Selection", data, self.channel_selection)))
        {
            let at = w.out.len();
            w.patch(slot, at);
            if Some(slot) == channel
                && let Some(target) = floating_target
            {
                w.patch(target, at);
            }
            w.u32(self.width);
            w.u32(self.height);
            w.string(name);
            if selection {
                w.prop(4, &[]);
            }
            w.end();
            let pixels = w.slot();
            if self.version >= 26 {
                w.ptr(0); // Channel/mask effects-list terminator precedes the hierarchy.
            }
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
            assert_eq!(tile.drawable.precision.transfer, Transfer::Linear);
            assert!(tile.drawable.precision.storage.linear);
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
fn historical_mask_question_preserves_the_metadata_for_an_explicit_retry() {
    let mut source = Source::gray(250, 2, 0x1234u16.to_be_bytes().to_vec());
    source.version = 11;
    source.mask = Some(0xabcd_u16.to_be_bytes().to_vec());
    let bytes = source.write();
    let reader = Xcf::parse(&bytes).unwrap();
    reader.meta_budget.set("Mask".len());
    let mut budget = 2;
    let missing = SampleOptions { historical_order: HistoricalOrder::RequireExplicit };
    let error = reader.visit_layer_mask_tiles(0, missing, &mut budget, GO, |_| panic!("no samples before a choice")).unwrap_err();
    assert!(error.to_string().contains("explicit byte order"));
    assert_eq!(reader.meta_budget.get(), "Mask".len());
    assert_eq!(budget, 2);
    let (mask, _) = reader
        .visit_layer_mask_tiles(0, options(SampleOrder::BigEndian), &mut budget, GO, |tile| {
            assert_eq!(tile.sample(0)?, SourceSample::U16(0xabcd));
            Ok(())
        })
        .unwrap()
        .unwrap();
    assert_eq!(mask.name, "Mask");
    assert_eq!(reader.meta_budget.get(), 0);
    assert_eq!(budget, 0);
}

#[test]
fn saved_selection_must_match_canvas_before_any_callback() {
    let mut source = Source::gray(150, 1, vec![10]);
    source.channel = Some(vec![128]);
    let mut bytes = source.write();
    // Keep the channel record and its hierarchy mutually consistent at 1x1;
    // only the independently framed canvas changes to 2x2.
    let image_start = 14;
    bytes[image_start..image_start + 4].copy_from_slice(&2u32.to_be_bytes());
    bytes[image_start + 4..image_start + 8].copy_from_slice(&2u32.to_be_bytes());
    let reader = Xcf::parse(&bytes).unwrap();
    let mut budget = 1;
    let error =
        reader.visit_channel_tiles(0, SampleOptions::default(), &mut budget, GO, |_| panic!("invalid selection cannot publish samples")).unwrap_err();
    assert!(error.to_string().contains("selection dimensions"));
    assert_eq!(budget, 1);
    // An ordinary auxiliary channel need not cover the canvas.
    let ptr = reader.channels[0];
    drop(reader);
    let prop = ptr + 8 + 4 + "Selection".len() + 1;
    bytes[prop..prop + 4].copy_from_slice(&3u32.to_be_bytes()); // Active channel, not selection.
    let reader = Xcf::parse(&bytes).unwrap();
    let (channel, drawable) = reader
        .visit_channel_tiles(0, SampleOptions::default(), &mut budget, GO, |tile| {
            assert_eq!(tile.sample(0)?, SourceSample::U8(128));
            Ok(())
        })
        .unwrap();
    assert!(!channel.selection);
    assert_eq!((drawable.width, drawable.height), (1, 1));
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
                if tile.x == 64 {
                    assert_eq!(tile.data, [64, 129], "{compression:?}: right-edge samples");
                } else {
                    assert!(tile.data.iter().eq(source.data[..64].iter().chain(&source.data[65..129])), "{compression:?}: both full rows");
                }
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
    // Fits usize even on wasm32, but exceeds the 1 GiB source pixel limit.
    bytes[hierarchy..hierarchy + 4].copy_from_slice(&32_769u32.to_be_bytes());
    bytes[hierarchy + 4..hierarchy + 8].copy_from_slice(&32_768u32.to_be_bytes());
    let mut reader = Xcf::parse(&bytes).unwrap();
    reader.image.layers[0].width = 32_769;
    reader.image.layers[0].height = 32_768;
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
        file.ptr(0); // Mandatory hierarchy pointer-list terminator.
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
            VisitTarget {
                pointer: hierarchy,
                channels: 1,
                indexed: false,
                dimensions: Some((2, 1)),
                coverage: false,
                transfer_is_assumed: false,
                excluded_source: &[],
            },
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

#[test]
fn rgb_floating_gray_alpha_attached_to_a_channel_keeps_two_raw_components() {
    for floating in [false, true] {
        let mut source = Source::gray(650, 4, [0.25f32, 0.75].into_iter().flat_map(f32::to_be_bytes).collect());
        source.base = 0;
        source.kind = 3;
        source.channels = 2;
        source.floating = floating;
        source.channel = Some(0.5f32.to_be_bytes().to_vec());
        let bytes = source.write();
        if !floating {
            assert!(Xcf::parse(&bytes).is_err(), "ordinary GrayAlpha is still invalid on RGB");
            continue;
        }
        let xcf = Xcf::parse(&bytes).unwrap();
        assert!(xcf.image.layers[0].floating_selection);
        let mut visited = 0;
        let mut budget = 8;
        let info = xcf
            .visit_layer_tiles(0, SampleOptions::default(), &mut budget, GO, |tile| {
                assert_eq!(tile.drawable.channels, 2);
                assert!(!tile.drawable.indexed);
                assert_eq!(tile.data, source.data);
                assert_eq!(tile.sample(0)?, SourceSample::F32(0.25));
                assert_eq!(tile.sample(1)?, SourceSample::F32(0.75));
                visited += 1;
                Ok(())
            })
            .unwrap()
            .unwrap();
        assert_eq!(info.channels, 2);
        assert_eq!(visited, 1);
        assert_eq!(budget, 0);
    }
}

#[test]
fn saved_channels_and_selections_report_linear_high_precision_without_converting_samples() {
    for selection in [false, true] {
        for (code, width, data, stored_transfer) in [
            (175, 1, vec![128], Transfer::Linear),
            (650, 4, 0.5f32.to_be_bytes().to_vec(), Transfer::Linear),
            (600, 4, 0.5f32.to_be_bytes().to_vec(), Transfer::Linear),
            (250, 2, 0x8000u16.to_be_bytes().to_vec(), Transfer::Linear),
            (150, 1, vec![128], Transfer::Nonlinear),
            (100, 1, vec![128], Transfer::Linear),
        ] {
            let transfer = if selection { Transfer::Linear } else { stored_transfer };
            let mut source = Source::gray(code, width, data.clone());
            source.channel = Some(data.clone());
            source.channel_selection = selection;
            let bytes = source.write();
            let xcf = Xcf::parse(&bytes).unwrap();
            let mut budget = width;
            let (channel, info) = xcf
                .visit_channel_tiles(0, SampleOptions::default(), &mut budget, GO, |tile| {
                    assert_eq!(tile.data, data);
                    assert_eq!(tile.drawable.precision.transfer, transfer);
                    assert_eq!(tile.drawable.transfer_is_assumed, !selection || code == 150);
                    assert_eq!(tile.drawable.precision.storage.linear, transfer == Transfer::Linear);
                    if code == 650 || code == 600 {
                        assert_eq!(tile.sample(0)?, SourceSample::F32(0.5));
                    }
                    Ok(())
                })
                .unwrap();
            assert_eq!(channel.selection, selection);
            assert_eq!(info.precision.transfer, transfer);
            assert_eq!(info.transfer_is_assumed, !selection || code == 150);
            assert_eq!(budget, 0);
            // A color layer keeps the image transfer, independently of channel role.
            let color_transfer = if code == 175 {
                Transfer::Perceptual
            } else if code == 600 || code == 100 {
                Transfer::Linear
            } else {
                Transfer::Nonlinear
            };
            let mut color_budget = width;
            let layer = xcf
                .visit_layer_tiles(0, SampleOptions::default(), &mut color_budget, GO, |tile| {
                    assert_eq!(tile.data, data);
                    assert_eq!(tile.drawable.precision.transfer, color_transfer);
                    Ok(())
                })
                .unwrap()
                .unwrap();
            assert_eq!(layer.precision.transfer, color_transfer);
            assert!(!layer.transfer_is_assumed);
            assert_eq!(color_budget, 0);
        }
    }
}

#[test]
fn failed_channel_visits_release_only_their_name_reservation() {
    let mut source = Source::gray(150, 1, vec![1, 2]);
    source.width = 2;
    source.mask = Some(vec![3, 4]);
    source.channel = Some(vec![5, 6]);
    let bytes = source.write();
    let reader = Xcf::parse(&bytes).unwrap();
    reader.meta_budget.set("Mask".len());
    let mut short = 1;
    assert!(reader.visit_layer_mask_tiles(0, SampleOptions::default(), &mut short, GO, |_| panic!("unadmitted pixels")).is_err());
    assert_eq!(short, 1);
    assert_eq!(reader.meta_budget.get(), "Mask".len());
    let mut exact = 2;
    let (mask, _) = reader.visit_layer_mask_tiles(0, SampleOptions::default(), &mut exact, GO, |_| Ok(())).unwrap().unwrap();
    assert_eq!(mask.name, "Mask");
    assert_eq!(reader.meta_budget.get(), 0, "returned names remain retained");
    assert_eq!(exact, 0);

    reader.meta_budget.set("Mask".len() + "Selection".len());
    let mut outer_pixels = 2;
    let result = reader.visit_layer_mask_tiles(0, SampleOptions::default(), &mut outer_pixels, GO, |_| {
        let mut nested_pixels = 2;
        let (nested, _) = reader.visit_channel_tiles(0, SampleOptions::default(), &mut nested_pixels, GO, |_| Ok(()))?;
        assert_eq!(nested.name, "Selection");
        assert_eq!(nested_pixels, 0);
        Err(Error::Cancelled)
    });
    assert!(matches!(result, Err(Error::Cancelled)));
    assert_eq!(outer_pixels, 0, "decoded pixel work is never refunded");
    assert_eq!(reader.meta_budget.get(), "Mask".len(), "only the failed outer name is released");
}

#[test]
fn legacy_pixels_use_the_same_stored_layout_as_source_tiles() {
    use crate::tests::{TestImage, TestLayer};
    for base in [0, 2] {
        let mut image = TestImage::rgb8(1, 1);
        image.version = 22;
        image.base_type = base;
        let mut floating = TestLayer::rgba("Floating", 1, 1, vec![64, 192]);
        floating.kind = 3;
        floating.props.push((5, vec![0; 8]));
        image.layers.push(floating);
        let ordinary = if base == 0 { vec![20, 40, 60, 128] } else { vec![20, 128] };
        let mut layer = TestLayer::rgba("Ordinary", 1, 1, ordinary.clone());
        if base == 2 {
            layer.kind = 5;
        }
        image.layers.push(layer);
        image.channels.push(("Target", vec![128], false));
        let bytes = image.write();
        let xcf = Xcf::parse(&bytes).unwrap();
        let target = xcf.channels[0];
        let mut cursor = xcf.c.at(xcf.layer_sources[0]);
        cursor.take(12).unwrap();
        cursor.string().unwrap();
        let target_slot = loop {
            let kind = cursor.u32().unwrap();
            let size = cursor.u32().unwrap() as usize;
            assert_ne!(kind, 0);
            if kind == 5 {
                break cursor.pos;
            }
            cursor.take(size).unwrap();
        };
        let mut file = W { out: bytes, wide: true };
        file.patch(target_slot, target);
        let xcf = Xcf::parse(&file.out).unwrap();
        let mut budget = 24;
        let floating = xcf.layer_pixels(0, &mut budget, GO).unwrap().unwrap();
        assert_eq!(floating.channels, 2);
        assert_eq!(floating.data, [64u8, 192].map(|v| f32::from(v) / 255.0));
        let regular = xcf.layer_pixels(1, &mut budget, GO).unwrap().unwrap();
        assert_eq!(regular.channels, 4);
        let expected = if base == 0 { ordinary } else { vec![20, 20, 20, 128] };
        assert_eq!(regular.data, expected.into_iter().map(|v| f32::from(v) / 255.0).collect::<Vec<_>>());
        assert_eq!(budget, 0);
        let mut source_budget = 2;
        xcf.visit_layer_tiles(0, SampleOptions::default(), &mut source_budget, GO, |tile| {
            assert_eq!(tile.drawable.channels, 2);
            assert!(!tile.drawable.indexed);
            assert_eq!(tile.data, [64, 192]);
            Ok(())
        })
        .unwrap();
        assert_eq!(source_budget, 0);
    }
}

#[test]
fn hierarchy_headers_are_never_tile_payload_for_either_pixel_api() {
    for version in [10, 11] {
        for placement in ["before", "short-before", "inside", "between", "after"] {
            let mut source = Source::gray(150, 1, vec![0]);
            source.version = version;
            let mut file = W { out: source.write(), wide: version >= 11 };
            let before = file.out.len();
            file.out.push(22);
            if placement != "short-before" {
                file.out.push(44);
            }
            let hierarchy = file.out.len();
            for value in [2, 1, 1] {
                file.u32(value);
            }
            let level_slot = file.slot();
            file.ptr(0); // Mandatory hierarchy pointer-list terminator.
            let between = file.out.len();
            file.out.extend([22, 44]);
            let level = file.out.len();
            file.patch(level_slot, level);
            file.u32(2);
            file.u32(1);
            let tile_slot = file.slot();
            file.ptr(0);
            let after = file.out.len();
            file.out.extend([22, 44]);
            let tile = match placement {
                "before" | "short-before" => before,
                "inside" => hierarchy,
                "between" => between,
                _ => after,
            };
            file.patch(tile_slot, tile);
            let reader = Xcf::parse(&file.out).unwrap();
            let valid = !matches!(placement, "short-before" | "inside");
            let legacy = reader.hierarchy(hierarchy, 1, false, &mut 8, GO);
            let mut seen = Vec::new();
            let raw = reader.visit_hierarchy(
                VisitTarget {
                    pointer: hierarchy,
                    channels: 1,
                    indexed: false,
                    dimensions: Some((2, 1)),
                    coverage: false,
                    transfer_is_assumed: false,
                    excluded_source: &[],
                },
                SampleOptions::default(),
                &mut 2,
                GO,
                |tile| {
                    seen.extend_from_slice(tile.data);
                    Ok(())
                },
            );
            if valid {
                assert_eq!(legacy.unwrap().data, [22u8, 44].map(|v| f32::from(v) / 255.0), "v{version} {placement}");
                raw.unwrap();
                assert_eq!(seen, [22, 44]);
            } else {
                assert!(legacy.is_err(), "v{version} {placement}: legacy samples overlap a header");
                assert!(raw.is_err(), "v{version} {placement}: source samples overlap a header");
                assert!(seen.is_empty());
            }
        }
    }
}

#[test]
fn fixed_first_level_validation_accepts_zero_tiles_but_rejects_short_or_overlapping_headers() {
    for version in [10, 11] {
        let mut source = Source::gray(150, 1, vec![22]);
        source.version = version;
        let mut file = W { out: source.write(), wide: version >= 11 };
        let reader = Xcf::parse(&file.out).unwrap();
        let hierarchy = reader.image.layers[0].hierarchy;
        let head = reader.hierarchy_head(hierarchy, 1, None).unwrap();
        let level = head.level;
        let pointer_bytes = if version >= 11 { 8 } else { 4 };
        file.patch(level + 8, 0);
        let reader = Xcf::parse(&file.out).unwrap();
        let head = reader.hierarchy_head(hierarchy, 1, None).unwrap();
        reader.validate_first_level(&head).unwrap();
        let mut short = file.out.clone();
        short.truncate(level + 8 + pointer_bytes - 1);
        let reader = Xcf::parse(&short).unwrap();
        let head = reader.hierarchy_head(hierarchy, 1, None).unwrap();
        assert!(reader.validate_first_level(&head).is_err());
        let mut mismatch = file.out.clone();
        mismatch[level..level + 4].copy_from_slice(&2u32.to_be_bytes());
        let reader = Xcf::parse(&mismatch).unwrap();
        let head = reader.hierarchy_head(hierarchy, 1, None).unwrap();
        assert!(reader.validate_first_level(&head).is_err());
        file.patch(hierarchy + 12, hierarchy + 4);
        let reader = Xcf::parse(&file.out).unwrap();
        let head = reader.hierarchy_head(hierarchy, 1, None).unwrap();
        assert!(reader.validate_first_level(&head).is_err());
    }
}

#[test]
fn complete_hierarchy_pointer_lists_are_framing_not_pixel_payload() {
    for version in [10, 11] {
        for dummy_count in [0, 1, 31] {
            for location in ["terminator", "dummy", "payload"] {
                if dummy_count == 0 && location == "dummy" {
                    continue;
                }
                let mut source = Source::gray(150, 1, vec![0]);
                source.version = version;
                let mut file = W { out: source.write(), wide: version >= 11 };
                let hierarchy = file.out.len();
                for value in [2, 1, 1] {
                    file.u32(value);
                }
                let level_slot = file.slot();
                let dummy = file.out.len();
                for _ in 0..dummy_count {
                    // Unused offsets remain wire words, never dereferenced or
                    // narrowed to usize (even on a 32-bit caller).
                    if file.wide {
                        file.out.extend(u64::MAX.to_be_bytes());
                    } else {
                        file.u32(u32::MAX);
                    }
                }
                let terminator = file.out.len();
                file.ptr(0);
                let payload = file.out.len();
                file.out.extend([22, 44]);
                let level = file.out.len();
                file.patch(level_slot, level);
                file.u32(2);
                file.u32(1);
                file.ptr(match location {
                    "terminator" => terminator,
                    "dummy" => dummy,
                    _ => payload,
                });
                file.ptr(0);
                let reader = Xcf::parse(&file.out).unwrap();
                let legacy = reader.hierarchy(hierarchy, 1, false, &mut 8, GO);
                let mut seen = Vec::new();
                let source = reader.visit_hierarchy(
                    VisitTarget {
                        pointer: hierarchy,
                        channels: 1,
                        indexed: false,
                        dimensions: Some((2, 1)),
                        coverage: false,
                        transfer_is_assumed: false,
                        excluded_source: &[],
                    },
                    SampleOptions::default(),
                    &mut 2,
                    GO,
                    |tile| {
                        seen.extend_from_slice(tile.data);
                        Ok(())
                    },
                );
                if location == "payload" {
                    assert_eq!(legacy.unwrap().data, [22u8, 44].map(|v| f32::from(v) / 255.0));
                    source.unwrap();
                    assert_eq!(seen, [22, 44]);
                } else {
                    assert!(legacy.is_err(), "v{version}, {dummy_count} dummy levels, {location}");
                    assert!(source.is_err());
                    assert!(seen.is_empty());
                }
            }
        }
    }
}

#[test]
fn unterminated_or_excessive_hierarchy_lists_stop_before_pixel_admission() {
    for version in [10, 11] {
        for excessive in [false, true] {
            let mut source = Source::gray(150, 1, vec![22]);
            source.version = version;
            let mut file = W { out: source.write(), wide: version >= 11 };
            let hierarchy = file.out.len();
            for value in [1, 1, 1] {
                file.u32(value);
            }
            file.ptr(1);
            for _ in 0..if excessive { 32 } else { 1 } {
                file.ptr(1);
            }
            if excessive {
                file.ptr(0);
            }
            let reader = Xcf::parse(&file.out).unwrap();
            let mut budget = 1;
            assert!(
                reader
                    .visit_hierarchy(
                        VisitTarget {
                            pointer: hierarchy,
                            channels: 1,
                            indexed: false,
                            dimensions: Some((1, 1)),
                            coverage: false,
                            transfer_is_assumed: false,
                            excluded_source: &[],
                        },
                        SampleOptions::default(),
                        &mut budget,
                        GO,
                        |_| panic!("unadmitted hierarchy"),
                    )
                    .is_err()
            );
            assert_eq!(budget, 1);
        }
    }
}

#[test]
fn corroborated_dummy_level_headers_bound_tile_payloads() {
    for version in [10, 11] {
        for complete in [false, true] {
            let mut source = Source::gray(150, 1, vec![0]);
            source.version = version;
            let mut file = W { out: source.write(), wide: version >= 11 };
            let hierarchy = file.out.len();
            for value in [2, 2, 1] {
                file.u32(value);
            }
            let level_slot = file.slot();
            let dummy_slot = file.slot();
            file.ptr(0);
            let level = file.out.len();
            file.patch(level_slot, level);
            file.u32(2);
            file.u32(2);
            let tile_slot = file.slot();
            file.ptr(0);
            let tile = file.out.len();
            file.patch(tile_slot, tile);
            file.out.extend([11, 22, 33]);
            if complete {
                file.out.push(44);
            }
            let dummy = file.out.len();
            file.patch(dummy_slot, dummy);
            file.u32(1);
            file.u32(1);
            file.u32(0); // Dummy levels end with a WORD, even with wide hierarchy pointers.
            let reader = Xcf::parse(&file.out).unwrap();
            let legacy = reader.hierarchy(hierarchy, 1, false, &mut 16, GO);
            let mut seen = Vec::new();
            let raw = reader.visit_hierarchy(
                VisitTarget {
                    pointer: hierarchy,
                    channels: 1,
                    indexed: false,
                    dimensions: Some((2, 2)),
                    coverage: false,
                    transfer_is_assumed: false,
                    excluded_source: &[],
                },
                SampleOptions::default(),
                &mut 4,
                GO,
                |tile| {
                    seen.extend_from_slice(tile.data);
                    Ok(())
                },
            );
            if complete {
                assert_eq!(legacy.unwrap().data, [11u8, 22, 33, 44].map(|v| f32::from(v) / 255.0));
                raw.unwrap();
                assert_eq!(seen, [11, 22, 33, 44]);
            } else {
                assert!(legacy.is_err());
                assert!(raw.is_err());
                assert!(seen.is_empty());
            }
        }
    }
}

#[test]
fn a_matching_dummy_inside_a_complete_final_tile_cannot_hide_its_samples() {
    for short_by in [0, 1, 12] {
        let mut source = Source::gray(150, 1, vec![0]);
        source.version = 11;
        let mut file = W { out: source.write(), wide: true };
        let hierarchy = file.out.len();
        for word in [128, 64, 1] {
            file.u32(word);
        }
        let level_slot = file.slot();
        let dummy_slot = file.slot();
        file.ptr(0);
        let level = file.out.len();
        file.patch(level_slot, level);
        file.u32(128);
        file.u32(64);
        let first_slot = file.slot();
        let final_slot = file.slot();
        file.ptr(0);
        let first = file.out.len();
        file.patch(first_slot, first);
        let tile_bytes = 64 * 64;
        file.out.extend(std::iter::repeat_n(71, tile_bytes));
        let last = file.out.len();
        file.patch(final_slot, last);
        let mut final_pixels = vec![155; tile_bytes - short_by];
        let dummy = if short_by == 0 {
            let mut bytes = Vec::new();
            for word in [64u32, 32, 0] {
                bytes.extend(word.to_be_bytes());
            }
            final_pixels[16..16 + bytes.len()].copy_from_slice(&bytes);
            last + 16
        } else {
            last + final_pixels.len()
        };
        file.out.extend(&final_pixels);
        file.patch(dummy_slot, dummy);
        if short_by != 0 {
            for word in [64, 32, 0] {
                file.u32(word);
            }
        }

        let reader = Xcf::parse(&file.out).unwrap();
        let legacy = reader.hierarchy(hierarchy, 1, false, &mut (4 * 2 * tile_bytes), GO);
        let mut visited = Vec::new();
        let raw = reader.visit_hierarchy(
            VisitTarget {
                pointer: hierarchy,
                channels: 1,
                indexed: false,
                dimensions: Some((128, 64)),
                coverage: false,
                transfer_is_assumed: false,
                excluded_source: &[],
            },
            SampleOptions::default(),
            &mut (2 * tile_bytes),
            GO,
            |tile| {
                visited.push(tile.data.to_vec());
                Ok(())
            },
        );
        assert!(visited[0].iter().all(|&sample| sample == 71));
        if short_by == 0 {
            raw.unwrap();
            assert_eq!(visited[1], final_pixels);
            let image = legacy.unwrap();
            assert_eq!(image.data.len(), 2 * tile_bytes);
            for (row, final_row) in image.data.as_chunks::<128>().0.iter().zip(final_pixels.as_chunks::<64>().0) {
                assert!(row[..64].iter().all(|&sample| sample == 71. / 255.));
                assert!(row[64..].iter().zip(final_row).all(|(&decoded, &source)| decoded == f32::from(source) / 255.));
            }
        } else {
            assert!(matches!(raw, Err(Error::Invalid(ref message)) if message == "an uncompressed tile is cut short"));
            assert!(matches!(legacy, Err(Error::Invalid(ref message)) if message == "an uncompressed tile is cut short"));
            assert_eq!(visited.len(), 1, "the final tile was short by {short_by} bytes");
        }
    }
}

#[test]
fn unused_mip_pointer_into_primary_terminator_does_not_hide_complete_tiles() {
    let mut source = Source::gray(150, 1, vec![0]);
    source.version = 11;
    let mut file = W { out: source.write(), wide: true };
    let hierarchy = file.out.len();
    for word in [1, 65, 1] {
        file.u32(word);
    }
    let level_slot = file.slot();
    let dummy_slot = file.slot();
    file.ptr(0);
    let level = file.out.len();
    file.patch(level_slot, level);
    file.u32(1);
    file.u32(65);
    let first_slot = file.slot();
    let second_slot = file.slot();
    file.ptr(0);
    let first = file.out.len();
    // The terminator suffix and the first eight pixels resemble a 0x32 dummy.
    file.patch(dummy_slot, first - 4);
    file.patch(first_slot, first);
    let mut expected = [0; 65];
    expected[3] = 32;
    expected[64] = 74;
    file.patch(second_slot, first + 64);
    file.out.extend(expected);
    let reader = Xcf::parse(&file.out).unwrap();
    assert_eq!(reader.hierarchy(hierarchy, 1, false, &mut (65 * 4), GO).unwrap().data, expected.map(|value| f32::from(value) / 255.));
    let mut raw = Vec::new();
    reader
        .visit_hierarchy(
            VisitTarget {
                pointer: hierarchy,
                channels: 1,
                indexed: false,
                dimensions: Some((1, 65)),
                coverage: false,
                transfer_is_assumed: false,
                excluded_source: &[],
            },
            SampleOptions::default(),
            &mut 65,
            GO,
            |tile| {
                raw.extend_from_slice(tile.data);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(raw, expected);
}

#[test]
fn full_first_level_tables_must_not_overlap_the_hierarchy() {
    for version in [10, 11] {
        for overlap in [0, 1] {
            let mut source = Source::gray(150, 1, vec![0]);
            source.version = version;
            let mut file = W { out: source.write(), wide: version >= 11 };
            let level = file.out.len();
            file.u32(2);
            file.u32(1);
            let tile_slot = file.slot();
            file.ptr(0);
            file.out.truncate(file.out.len() - overlap);
            let hierarchy = file.out.len();
            for value in [2, 1, 1] {
                file.u32(value);
            }
            file.ptr(level);
            file.ptr(0);
            let tile = file.out.len();
            file.patch(tile_slot, tile);
            file.out.extend([22, 44]);
            let reader = Xcf::parse(&file.out).unwrap();
            let legacy = reader.hierarchy(hierarchy, 1, false, &mut 8, GO);
            let mut seen = Vec::new();
            let raw = reader.visit_hierarchy(
                VisitTarget {
                    pointer: hierarchy,
                    channels: 1,
                    indexed: false,
                    dimensions: Some((2, 1)),
                    coverage: false,
                    transfer_is_assumed: false,
                    excluded_source: &[],
                },
                SampleOptions::default(),
                &mut 2,
                GO,
                |tile| {
                    seen.extend_from_slice(tile.data);
                    Ok(())
                },
            );
            if overlap == 0 {
                assert_eq!(legacy.unwrap().data, [22u8, 44].map(|v| f32::from(v) / 255.0));
                raw.unwrap();
                assert_eq!(seen, [22, 44]);
            } else {
                assert!(legacy.is_err());
                assert!(raw.is_err());
                assert!(seen.is_empty());
            }
        }
    }
}

#[test]
fn zero_axis_dummy_levels_at_eof_still_bound_the_right_edge_tile() {
    for version in [10, 11] {
        for complete in [false, true] {
            let mut source = Source::gray(150, 1, vec![0]);
            source.version = version;
            let mut file = W { out: source.write(), wide: version >= 11 };
            let hierarchy = file.out.len();
            for value in [65, 1, 1] {
                file.u32(value);
            }
            let level_slot = file.slot();
            let dummy_slot = file.slot();
            file.ptr(0);
            let level = file.out.len();
            file.patch(level_slot, level);
            file.u32(65);
            file.u32(1);
            let first_slot = file.slot();
            let last_slot = file.slot();
            file.ptr(0);
            let first = file.out.len();
            file.patch(first_slot, first);
            file.out.extend([33; 64]);
            let last = file.out.len();
            file.patch(last_slot, last);
            if complete {
                file.out.push(77);
            }
            let dummy = file.out.len();
            file.patch(dummy_slot, dummy);
            for value in [32, 0, 0] {
                file.u32(value);
            }
            assert_eq!(file.out.len() - dummy, 12);
            let reader = Xcf::parse(&file.out).unwrap();
            let legacy = reader.hierarchy(hierarchy, 1, false, &mut (65 * 4), GO);
            let mut seen = Vec::new();
            let raw = reader.visit_hierarchy(
                VisitTarget {
                    pointer: hierarchy,
                    channels: 1,
                    indexed: false,
                    dimensions: Some((65, 1)),
                    coverage: false,
                    transfer_is_assumed: false,
                    excluded_source: &[],
                },
                SampleOptions::default(),
                &mut 65,
                GO,
                |tile| {
                    seen.extend_from_slice(tile.data);
                    Ok(())
                },
            );
            if complete {
                legacy.unwrap();
                raw.unwrap();
                assert_eq!(seen, [[33; 64].as_slice(), &[77]].concat());
            } else {
                assert!(legacy.is_err());
                assert!(raw.is_err());
                assert_eq!(seen, [33; 64], "only the earlier complete tile was published");
            }
        }
    }
}

#[test]
fn floating_gray_alpha_transfer_follows_known_targets_without_reinterpreting_samples() {
    use crate::tests::{TestImage, TestLayer};
    for code in [150, 650] {
        for role in ["mask", "layer", "channel", "unknown", "absent"] {
            let f32 = code == 650;
            let encode = |values: &[f32]| -> Vec<u8> {
                if f32 { values.iter().flat_map(|v| v.to_be_bytes()).collect() } else { values.iter().map(|v| (v * 255.0).round() as u8).collect() }
            };
            let mut image = TestImage::rgb8(1, 1);
            image.version = 22;
            image.precision = code;
            if role == "layer" {
                image.base_type = 1;
            }
            let samples = encode(&[0.25, 0.75]);
            let mut floating = TestLayer::rgba("Floating", 1, 1, samples.clone());
            floating.kind = 3;
            floating.props.push((5, vec![0; 8]));
            image.layers.push(floating);
            let mut owner = TestLayer::rgba("Owner", 1, 1, encode(if role == "layer" { &[0.4, 1.0] } else { &[0.2, 0.3, 0.4, 1.0] }));
            if role == "layer" {
                owner.kind = 3;
            }
            owner.mask = Some(encode(&[0.5]));
            image.layers.push(owner);
            image.channels.push(("Saved", encode(&[0.5]), false));
            let mut file = W { out: image.write(), wide: true };
            let reader = Xcf::parse(&file.out).unwrap();
            let target = match role {
                "mask" => reader.image.layers[1].mask as u64,
                "layer" => reader.layer_sources[1] as u64,
                "channel" => reader.channels[0] as u64,
                "unknown" => u64::MAX,
                _ => 0,
            };
            let mut cursor = reader.c.at(reader.layer_sources[0]);
            cursor.take(12).unwrap();
            cursor.string().unwrap();
            let slot = loop {
                let kind = cursor.u32().unwrap();
                let length = cursor.u32().unwrap() as usize;
                assert_ne!(kind, 0);
                if kind == 5 {
                    break cursor.pos;
                }
                cursor.take(length).unwrap();
            };
            file.out[slot..slot + 8].copy_from_slice(&target.to_be_bytes());
            let reader = Xcf::parse(&file.out).unwrap();
            let (linear, assumed) = match role {
                "mask" => (true, false),
                "layer" => (false, false),
                "channel" => (f32, true),
                _ => (false, true),
            };
            let mut budget = samples.len();
            let info = reader
                .visit_layer_tiles(0, SampleOptions::default(), &mut budget, GO, |tile| {
                    assert_eq!(tile.data, samples);
                    assert_eq!(tile.drawable.channels, 2);
                    assert_eq!(tile.drawable.precision.transfer, if linear { Transfer::Linear } else { Transfer::Nonlinear }, "{code} {role}");
                    assert_eq!(tile.drawable.transfer_is_assumed, assumed, "{code} {role}");
                    Ok(())
                })
                .unwrap()
                .unwrap();
            assert_eq!(info.transfer_is_assumed, assumed);
            assert_eq!(budget, 0);
        }
    }
}

#[test]
fn excluded_source_records_cannot_supply_tile_or_table_bytes() {
    let mut source = Source::gray(150, 1, vec![22, 44]);
    source.width = 2;
    let bytes = source.write();
    let reader = Xcf::parse(&bytes).unwrap();
    let hierarchy = reader.image.layers[0].hierarchy;
    let head = reader.hierarchy_head(hierarchy, 1, None).unwrap();
    let mut level = reader.c.at(head.level + 8);
    let tile = level.ptr().unwrap();
    for excluded in [tile..tile + 1, tile + 1..tile + 2, hierarchy..hierarchy + 1, head.level..head.level + 1, level.pos..level.pos + 1] {
        let mut called = false;
        let result = reader.visit_hierarchy(
            VisitTarget {
                pointer: hierarchy,
                channels: 1,
                indexed: false,
                dimensions: Some((2, 1)),
                coverage: false,
                transfer_is_assumed: false,
                excluded_source: std::slice::from_ref(&excluded),
            },
            SampleOptions::default(),
            &mut 2,
            GO,
            |_| {
                called = true;
                Ok(())
            },
        );
        assert!(result.is_err());
        assert!(!called);
    }
    let mut actual = Vec::new();
    reader
        .visit_hierarchy(
            VisitTarget {
                pointer: hierarchy,
                channels: 1,
                indexed: false,
                dimensions: Some((2, 1)),
                coverage: false,
                transfer_is_assumed: false,
                excluded_source: std::slice::from_ref(&(tile + 2..tile + 3)),
            },
            SampleOptions::default(),
            &mut 2,
            GO,
            |tile| {
                actual.extend_from_slice(tile.data);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(actual, [22, 44]);
}

#[test]
fn public_drawable_visitors_exclude_their_complete_source_records_and_preserve_adjacency() {
    for version in [10, 11, 26] {
        for role in ["layer", "mask", "channel"] {
            for placement in ["inside", "tail", "crossing", "adjacent"] {
                let mut source = Source::gray(150, 1, vec![22, 44]);
                source.width = 2;
                source.version = version;
                source.mask = Some(vec![22, 44]);
                source.channel = Some(vec![22, 44]);
                let mut file = W { out: source.write(), wide: version >= 11 };
                let reader = Xcf::parse(&file.out).unwrap();
                let mut master = reader.c.at(14 + 12 + if version >= 4 { 4 } else { 0 });
                props(&mut master).unwrap();
                let layer_slot = master.pos;
                master.ptr().unwrap();
                master.ptr().unwrap();
                let channel_slot = master.pos;
                let layer = &reader.image.layers[0];
                let mut layer_cursor = reader.c.at(reader.layer_sources[0]);
                layer_cursor.take(12).unwrap();
                layer_cursor.string().unwrap();
                props(&mut layer_cursor).unwrap();
                layer_cursor.ptr().unwrap();
                let mask_slot = layer_cursor.pos;
                let (start, slot) = match role {
                    "layer" => (reader.layer_sources[0], layer_slot),
                    "mask" => (layer.mask, mask_slot),
                    _ => (reader.channels[0], channel_slot),
                };
                let (end, hierarchy) = if role == "layer" {
                    (layer.source_record.end, layer.hierarchy)
                } else {
                    let mut cursor = reader.c.at(start);
                    cursor.take(8).unwrap();
                    cursor.string().unwrap();
                    props(&mut cursor).unwrap();
                    let hierarchy = cursor.ptr().unwrap();
                    reader.channel_effects_tail(&mut cursor, hierarchy, GO, None).unwrap();
                    (cursor.pos, hierarchy)
                };
                let record = file.out[start..end].to_vec();
                let head = reader.hierarchy_head(hierarchy, 1, None).unwrap();
                let level = head.level;
                // Relocate only the source record, retaining its already validated
                // hierarchy. This lets a real tile end exactly at its record.
                let payload = file.out.len();
                file.out.extend([22, 44]);
                let relocated = file.out.len();
                file.out.extend(record);
                file.patch(slot, relocated);
                file.patch(
                    level + 8,
                    match placement {
                        "inside" => relocated,
                        "tail" => file.out.len() - if file.wide { 8 } else { 4 },
                        "crossing" => relocated - 1,
                        _ => payload,
                    },
                );
                let reader = Xcf::parse(&file.out).unwrap();
                let mut seen = Vec::new();
                let mut budget = 2;
                let visit = |tile: SourceTile<'_>| {
                    seen.extend_from_slice(tile.data);
                    Ok(())
                };
                let raw = match role {
                    "layer" => reader.visit_layer_tiles(0, SampleOptions::default(), &mut budget, GO, visit).map(|_| ()),
                    "mask" => reader.visit_layer_mask_tiles(0, SampleOptions::default(), &mut budget, GO, visit).map(|_| ()),
                    _ => reader.visit_channel_tiles(0, SampleOptions::default(), &mut budget, GO, visit).map(|_| ()),
                };
                let legacy = match role {
                    "layer" => reader.layer_pixels(0, &mut 8, GO).map(|_| ()),
                    "mask" => reader.layer_mask(0, &mut 8, GO).map(|_| ()),
                    _ => reader.channel(0, &mut 8, GO).map(|_| ()),
                };
                if placement == "adjacent" {
                    raw.unwrap();
                    legacy.unwrap();
                    assert_eq!(seen, [22, 44]);
                } else {
                    assert!(raw.is_err(), "{version} {role} {placement}");
                    assert!(legacy.is_err(), "legacy {version} {role} {placement}");
                    assert!(seen.is_empty());
                }
                assert!(matches!(
                    reader.visit_channel_tiles(0, SampleOptions::default(), &mut 2, &|| false, |_| panic!("cancelled")),
                    Err(Error::Cancelled)
                ));
            }
        }
    }
}

#[test]
fn inert_dummy_pointer_is_ignored_only_inside_a_complete_contiguous_raw_tile() {
    for version in [10, 11] {
        let mut source = Source::gray(150, 1, vec![0]);
        source.version = version;
        let mut file = W { out: source.write(), wide: version >= 11 };
        let hierarchy = file.out.len();
        for word in [65, 1, 1] {
            file.u32(word);
        }
        let level_slot = file.slot();
        let dummy_slot = file.slot();
        file.ptr(0);
        let level = file.out.len();
        file.patch(level_slot, level);
        file.u32(65);
        file.u32(1);
        let first_slot = file.slot();
        let last_slot = file.slot();
        file.ptr(0);
        let first = file.out.len();
        file.patch(first_slot, first);
        let mut expected = vec![33; 64];
        expected[12..24].copy_from_slice(&[32u32.to_be_bytes(), 0u32.to_be_bytes(), 0u32.to_be_bytes()].concat());
        file.out.extend(&expected);
        file.patch(dummy_slot, first + 12);
        let last = file.out.len();
        file.patch(last_slot, last);
        file.out.push(77);
        expected.push(77);
        let reader = Xcf::parse(&file.out).unwrap();
        let legacy = reader.hierarchy(hierarchy, 1, false, &mut (65 * 4), GO).unwrap();
        assert_eq!(legacy.data, expected.iter().map(|v| f32::from(*v) / 255.0).collect::<Vec<_>>());
        let target = || VisitTarget {
            pointer: hierarchy,
            channels: 1,
            indexed: false,
            dimensions: Some((65, 1)),
            coverage: false,
            transfer_is_assumed: false,
            excluded_source: &[],
        };
        let mut seen = Vec::new();
        reader
            .visit_hierarchy(target(), SampleOptions::default(), &mut 65, GO, |tile| {
                seen.extend_from_slice(tile.data);
                Ok(())
            })
            .unwrap();
        assert_eq!(seen, expected);

        // A short tile followed by genuine dummy framing must fail before
        // delivery. The next primary tile starts after that header.
        file.out.truncate(first + 63);
        file.out[first + 12..first + 24].fill(33);
        let dummy = file.out.len();
        file.patch(dummy_slot, dummy);
        for word in [32, 0, 0] {
            file.u32(word);
        }
        let last = file.out.len();
        file.patch(last_slot, last);
        file.out.push(77);
        let reader = Xcf::parse(&file.out).unwrap();
        assert!(reader.hierarchy(hierarchy, 1, false, &mut (65 * 4), GO).is_err());
        let mut callbacks = 0;
        assert!(
            reader
                .visit_hierarchy(target(), SampleOptions::default(), &mut 65, GO, |_| {
                    callbacks += 1;
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(callbacks, 0);
    }
}

#[test]
fn floating_mask_transfer_requires_an_exact_target_and_an_available_matching_header() {
    use crate::tests::{TestImage, TestLayer};
    for version in [10, 11, 26] {
        for state in [
            "valid",
            "oversized",
            "short",
            "dangling",
            "mismatched",
            "short-name",
            "short-properties",
            "short-hierarchy",
            "short-tail",
            "bad-end",
            "bad-name-end",
        ] {
            if state == "short-tail" && version < 26 {
                continue;
            }
            let width = if version >= 11 { 8 } else { 4 };
            let mut image = TestImage::rgb8(1, 1);
            image.version = version;
            image.precision = 650;
            let mut floating = TestLayer::rgba("Floating", 1, 1, [0.25f32, 0.75].into_iter().flat_map(f32::to_be_bytes).collect());
            floating.kind = 3;
            floating.props.push((
                5,
                vec![
                    0;
                    if state == "oversized" {
                        width + 1
                    } else if state == "short" {
                        width - 1
                    } else {
                        width
                    }
                ],
            ));
            image.layers.push(floating);
            let mut owner = TestLayer::rgba("Owner", 1, 1, [0.2f32, 0.3, 0.4, 1.0].into_iter().flat_map(f32::to_be_bytes).collect());
            owner.mask = Some(0.5f32.to_be_bytes().to_vec());
            image.layers.push(owner);
            let mut file = W { out: image.write(), wide: version >= 11 };
            let reader = Xcf::parse(&file.out).unwrap();
            let mask = reader.image.layers[1].mask;
            let mut mask_cursor = reader.c.at(mask);
            mask_cursor.take(8).unwrap();
            mask_cursor.string_bytes().unwrap();
            let properties = mask_cursor.pos;
            props(&mut mask_cursor).unwrap();
            let hierarchy_slot = mask_cursor.pos;
            mask_cursor.ptr().unwrap();
            let tail = mask_cursor.pos;
            let mut cursor = reader.c.at(reader.layer_sources[0]);
            cursor.take(12).unwrap();
            cursor.string().unwrap();
            let (length, slot) = loop {
                let kind = cursor.u32().unwrap();
                let length = cursor.u32().unwrap() as usize;
                assert_ne!(kind, 0);
                if kind == 5 {
                    break (length, cursor.pos);
                }
                cursor.take(length).unwrap();
            };
            let target = if file.wide { (mask as u64).to_be_bytes().to_vec() } else { (mask as u32).to_be_bytes().to_vec() };
            let copied = length.min(target.len());
            file.out[slot..slot + copied].copy_from_slice(&target[..copied]);
            if state == "dangling" {
                file.out.truncate(mask);
            }
            if state == "mismatched" {
                file.out[mask..mask + 4].copy_from_slice(&2u32.to_be_bytes());
            }
            match state {
                "short-name" => file.out.truncate(mask + 12),
                "short-properties" => file.out.truncate(properties + 4),
                "short-hierarchy" => file.out.truncate(hierarchy_slot + width - 1),
                "short-tail" => file.out.truncate(tail + width - 1),
                "bad-end" => file.out[properties + 4..properties + 8].copy_from_slice(&4u32.to_be_bytes()),
                "bad-name-end" => file.out[properties - 1] = 1,
                _ => {}
            }
            let mut reader = Xcf::parse(&file.out).unwrap();
            if state == "valid" {
                let mask = reader
                    .visit_layer_mask_tiles(1, SampleOptions::default(), &mut 4, GO, |tile| {
                        assert_eq!(tile.sample(0)?, SourceSample::F32(0.5));
                        Ok(())
                    })
                    .unwrap()
                    .unwrap();
                assert_eq!(mask.1.precision.transfer, Transfer::Linear);
            }
            let metadata_before = reader.meta_budget.get();
            let info = reader
                .visit_layer_tiles(0, SampleOptions::default(), &mut 8, GO, |tile| {
                    assert_eq!(tile.sample(0)?, SourceSample::F32(0.25));
                    Ok(())
                })
                .unwrap()
                .unwrap();
            assert_eq!(reader.meta_budget.get(), metadata_before, "role checks retain no channel metadata");
            assert_eq!(info.transfer_is_assumed, state != "valid", "v{version} {state}");
            assert_eq!(info.precision.transfer, if state == "valid" { Transfer::Linear } else { Transfer::Nonlinear });
            if state == "valid" {
                // A complete channel frame cannot resolve a conflicting saved-channel role.
                reader.channels.push(mask);
                let aliased = reader.visit_layer_tiles(0, SampleOptions::default(), &mut 8, GO, |_| Ok(())).unwrap().unwrap();
                assert!(aliased.transfer_is_assumed, "v{version} saved channel and mask share the target");
            }
        }
    }
}

#[test]
fn channel_effect_tail_is_bounded_metered_and_cancellable_including_the_terminator() {
    let bytes = Source::gray(150, 1, vec![22]).write();
    let reader = Xcf::parse(&bytes).unwrap();
    let data = [1u64.to_be_bytes(), 0u64.to_be_bytes()].concat();
    let cursor = crate::Cursor { b: &data, pos: 0, wide: true };
    let mut work = crate::records::Budget::new(16, 0);
    reader.channel_effects_tail(&mut cursor.at(0), 0, GO, Some(&mut work)).unwrap();
    assert_eq!(work.work_remaining(), 0);
    assert!(matches!(
        reader.channel_effects_tail(&mut cursor.at(0), 0, GO, Some(&mut work)),
        Err(SampleHeaderError::Metadata(crate::records::StopReason::WorkLimit))
    ));
    assert!(matches!(
        reader.channel_effects_tail(&mut cursor.at(0), 0, &|| false, Some(&mut work)),
        Err(SampleHeaderError::Source(Error::Cancelled))
    ));
    let mut short = crate::records::Budget::new(15, 0);
    let mut position = cursor.at(0);
    assert!(matches!(
        reader.channel_effects_tail(&mut position, 0, GO, Some(&mut short)),
        Err(SampleHeaderError::Metadata(crate::records::StopReason::WorkLimit))
    ));
    assert_eq!(position.pos, 8, "the terminator is not read before admission");
    assert_eq!(short.work_remaining(), 7);
    assert!(reader.channel_effects_tail(&mut crate::Cursor { b: &data[..15], pos: 0, wide: true }, 0, GO, None).is_err());
}

#[test]
fn channel_effect_tail_count_limit_is_independent_of_eof() {
    let bytes = Source::gray(150, 1, vec![22]).write();
    let reader = Xcf::parse(&bytes).unwrap();
    let mut maximum = vec![1; 1024 * 8];
    maximum.extend([0; 8]);
    let mut cursor = crate::Cursor { b: &maximum, pos: 0, wide: true };
    reader.channel_effects_tail(&mut cursor, 0, GO, None).unwrap();
    assert_eq!(cursor.pos, maximum.len());

    let mut too_many = vec![1; 1025 * 8];
    too_many.extend([0; 8]);
    let mut cursor = crate::Cursor { b: &too_many, pos: 0, wide: true };
    assert!(matches!(
        reader.channel_effects_tail(&mut cursor, 0, GO, None),
        Err(SampleHeaderError::Source(Error::Invalid(ref problem))) if problem == "too many channel effects"
    ));
}

#[test]
fn failed_legacy_channel_tail_releases_its_name_for_another_channel_on_the_same_reader() {
    let mut source = Source::gray(150, 1, vec![1, 2]);
    source.width = 2;
    source.mask = Some(vec![3, 4]);
    source.channel = Some(vec![5, 6]);
    let mut file = W { out: source.write(), wide: true };
    let reader = Xcf::parse(&file.out).unwrap();
    let mut mask = reader.c.at(reader.image.layers[0].mask);
    mask.take(8).unwrap();
    mask.string().unwrap();
    props(&mut mask).unwrap();
    let hierarchy = mask.ptr().unwrap();
    let failed = file.out.len();
    file.u32(2);
    file.u32(1);
    file.string("Bad!");
    file.end();
    file.ptr(hierarchy);
    // No v26 tail follows. The complete pixels are elsewhere in the file.
    let mut reader = Xcf::parse(&file.out).unwrap();
    reader.channels.push(failed);
    reader.meta_budget.set(4);
    let mut pixels = 8;
    assert!(reader.channel(1, &mut pixels, GO).is_err());
    assert_eq!(pixels, 8, "tail refusal precedes pixel admission");
    assert_eq!(reader.meta_budget.get(), 4);
    let (channel, image) = reader.layer_mask(0, &mut pixels, GO).unwrap().unwrap();
    assert_eq!(channel.name, "Mask");
    assert_eq!(image.data, [3. / 255., 4. / 255.]);
    assert_eq!(pixels, 0);
    assert_eq!(reader.meta_budget.get(), 0, "successful name storage remains retained");
}

#[test]
fn failed_legacy_channel_refunds_only_name_storage_after_pixel_admission() {
    let mut source = Source::gray(150, 1, vec![1, 2]);
    source.width = 2;
    source.mask = Some(vec![3, 4]);
    let bytes = source.write();
    let reader = Xcf::parse(&bytes).unwrap();
    reader.meta_budget.set(4);
    let polls = std::cell::Cell::new(0);
    let mut pixels = 8;
    assert!(matches!(
        reader.layer_mask(0, &mut pixels, &|| {
            polls.set(polls.get() + 1);
            polls.get() < 3
        }),
        Err(Error::Cancelled)
    ));
    assert_eq!(pixels, 0, "admitted pixel work must not be refunded");
    assert_eq!(reader.meta_budget.get(), 4);
    reader.layer_mask(0, &mut 8, GO).unwrap().unwrap();
    assert_eq!(reader.meta_budget.get(), 0);
}

#[test]
fn parsed_layer_identity_wins_over_an_aliased_mask_pointer() {
    use crate::tests::{TestImage, TestLayer};
    for base in [0, 1] {
        let mut image = TestImage::rgb8(1, 1);
        image.version = 26;
        image.precision = 650;
        image.base_type = base;
        let mut floating = TestLayer::rgba("Floating", 1, 1, [0.25f32, 0.75].into_iter().flat_map(f32::to_be_bytes).collect());
        floating.kind = 3;
        floating.props.push((5, vec![0; 8]));
        image.layers.push(floating);
        let mut target = TestLayer::rgba(
            "Target",
            1,
            1,
            if base == 0 { [0.2f32, 0.3, 0.4, 1.0].into_iter().flat_map(f32::to_be_bytes).collect() } else { 0.5f32.to_be_bytes().to_vec() },
        );
        if base == 1 {
            target.kind = 2;
        }
        image.layers.push(target);
        let mut file = W { out: image.write(), wide: true };
        let reader = Xcf::parse(&file.out).unwrap();
        let target = reader.layer_sources[1];
        let mut cursor = reader.c.at(reader.layer_sources[0]);
        cursor.take(12).unwrap();
        cursor.string_bytes().unwrap();
        let floating_slot = loop {
            let kind = cursor.u32().unwrap();
            let len = cursor.u32().unwrap() as usize;
            assert_ne!(kind, 0);
            if kind == 5 {
                let slot = cursor.pos;
                cursor.take(len).unwrap();
                break slot;
            }
            cursor.take(len).unwrap();
        };
        props(&mut cursor).unwrap();
        cursor.ptr().unwrap();
        let mask_slot = cursor.pos;
        file.patch(floating_slot, target);
        file.patch(mask_slot, target);
        let reader = Xcf::parse(&file.out).unwrap();
        let info = reader
            .visit_layer_tiles(0, SampleOptions::default(), &mut 8, GO, |tile| {
                assert_eq!(tile.sample(0)?, SourceSample::F32(0.25));
                Ok(())
            })
            .unwrap()
            .unwrap();
        assert_eq!(info.precision.transfer, Transfer::Nonlinear);
        assert!(!info.transfer_is_assumed);
    }
}

#[test]
fn mask_role_framing_polls_cancellation_without_retaining_metadata() {
    let mut source = Source::gray(650, 4, 0.25f32.to_be_bytes().to_vec());
    source.mask = Some(0.5f32.to_be_bytes().to_vec());
    let bytes = source.write();
    let reader = Xcf::parse(&bytes).unwrap();
    let owner = &reader.image.layers[0];
    let metadata = reader.meta_budget.get();
    let calls = std::cell::Cell::new(0);
    let keep = || {
        calls.set(calls.get() + 1);
        calls.get() < 2
    };
    assert!(matches!(reader.confirm_mask_record(owner, &keep), Err(Error::Cancelled)));
    assert_eq!(reader.meta_budget.get(), metadata);
}

mod selection_roles;

#[test]
fn mask_transfer_requires_a_disjoint_addressable_hierarchy_and_a_complete_tail() {
    for placement in ["forward", "backward", "overlap", "eof", "no-tail"] {
        let mut source = Source::gray(650, 4, [0.25f32, 0.75].into_iter().flat_map(f32::to_be_bytes).collect());
        source.kind = 3;
        source.channels = 2;
        source.floating = true;
        let mut file = W { out: source.write(), wide: true };
        let reader = Xcf::parse(&file.out).unwrap();
        let layer = reader.layer_sources[0];
        let mut cursor = reader.c.at(layer);
        cursor.take(12).unwrap();
        cursor.string_bytes().unwrap();
        let target_slot = loop {
            let kind = cursor.u32().unwrap();
            let len = cursor.u32().unwrap() as usize;
            assert_ne!(kind, 0);
            if kind == 5 {
                let at = cursor.pos;
                cursor.take(len).unwrap();
                break at;
            }
            cursor.take(len).unwrap();
        };
        props(&mut cursor).unwrap();
        cursor.ptr().unwrap();
        let mask_slot = cursor.pos;
        let backward = if placement == "backward" {
            hierarchy(&mut file, 1, 1, 4, &0f32.to_be_bytes(), Compression::None)
        } else {
            // Reserve a prefix for a fixed header that would cross the mask start.
            file.u32(1);
            file.out.len() - 4
        };
        let mask = file.out.len();
        file.patch(target_slot, mask);
        file.patch(mask_slot, mask);
        file.u32(1);
        file.u32(1);
        file.u32(0);
        file.end();
        let hierarchy_slot = file.slot();
        if placement != "no-tail" {
            file.ptr(0);
        }
        let hierarchy = match placement {
            "backward" | "overlap" => backward,
            "eof" => file.out.len(),
            _ => hierarchy(&mut file, 1, 1, 4, &0f32.to_be_bytes(), Compression::None),
        };
        let confirmed = matches!(placement, "forward" | "backward");
        file.patch(hierarchy_slot, hierarchy);
        let reader = Xcf::parse(&file.out).unwrap();
        let info = reader
            .visit_layer_tiles(0, SampleOptions::default(), &mut 8, GO, |tile| {
                assert_eq!(tile.sample(0)?, SourceSample::F32(0.25));
                Ok(())
            })
            .unwrap()
            .unwrap();
        assert_eq!(info.transfer_is_assumed, !confirmed, "{placement}");
        assert_eq!(info.precision.transfer, if confirmed { Transfer::Linear } else { Transfer::Nonlinear });
        let visit = reader.visit_layer_mask_tiles(0, SampleOptions::default(), &mut 4, GO, |tile| {
            assert_eq!(tile.sample(0)?, SourceSample::F32(0.0));
            Ok(())
        });
        if confirmed {
            assert!(visit.unwrap().is_some());
        } else {
            assert!(visit.is_err(), "{placement}");
        }
    }
}

#[test]
fn channel_tail_forward_boundary_is_checked_before_work_and_keeps_typed_work_refusals() {
    let bytes = Source::gray(150, 1, vec![22]).write();
    let reader = Xcf::parse(&bytes).unwrap();
    let data = [1u64.to_be_bytes(), 0u64.to_be_bytes()].concat();
    for (hierarchy, expected_spent) in [(4, 0), (8, 8), (16, 16)] {
        let mut cursor = crate::Cursor { b: &data, pos: 0, wide: true };
        let mut budget = crate::records::Budget::new(16, 0);
        let result = reader.channel_effects_tail(&mut cursor, hierarchy, GO, Some(&mut budget));
        assert_eq!(16 - budget.work_remaining(), expected_spent);
        if hierarchy == 16 {
            result.unwrap();
        } else {
            assert!(matches!(result, Err(SampleHeaderError::Source(_))));
        }
        assert!(cursor.pos <= hierarchy);
    }
    let mut cursor = crate::Cursor { b: &data, pos: 0, wide: true };
    let mut budget = crate::records::Budget::new(7, 0);
    assert!(matches!(
        reader.channel_effects_tail(&mut cursor, 16, GO, Some(&mut budget)),
        Err(SampleHeaderError::Metadata(crate::records::StopReason::WorkLimit))
    ));
    assert_eq!(cursor.pos, 0);
}

#[test]
fn pre26_public_channel_visitors_follow_tiles_in_gaps_including_adjacent_zeros() {
    for version in [10, 11, 22] {
        for (gap, zero) in [(0, true), (1, true), (1, false), (2, true)] {
            for points_to_gap in [false, true] {
                if gap == 0 && points_to_gap {
                    continue;
                }
                let mut source = Source::gray(150, 1, vec![22, 44]);
                source.width = 2;
                source.version = version;
                source.channel = Some(vec![22, 44]);
                source.mask = Some(vec![22, 44]);
                for role in ["channel", "mask"] {
                    let mut file = W { out: source.write(), wide: version >= 11 };
                    let reader = Xcf::parse(&file.out).unwrap();
                    let pointer = if role == "channel" { reader.channels[0] } else { reader.image.layers[0].mask };
                    let mut cursor = reader.c.at(pointer);
                    cursor.take(8).unwrap();
                    cursor.string_bytes().unwrap();
                    props(&mut cursor).unwrap();
                    let hierarchy_slot = cursor.pos;
                    let hierarchy = cursor.ptr().unwrap();
                    let old_end = cursor.pos;
                    let width = if version >= 11 { 8 } else { 4 };
                    // Insert only this record's local gap and relocate its hierarchy
                    // pointers; other recorded source offsets all precede the insertion.
                    file.out.splice(old_end..old_end, std::iter::repeat_n(if zero { 0 } else { 77 }, gap * width));
                    let new_hierarchy = hierarchy + gap * width;
                    file.patch(hierarchy_slot, new_hierarchy);
                    let reader = Xcf::parse(&file.out).unwrap();
                    let mut head_cursor = reader.c.at(new_hierarchy + 12);
                    let old_level = head_cursor.ptr().unwrap();
                    let new_level = old_level + gap * width;
                    file.patch(new_hierarchy + 12, new_level);
                    let reader = Xcf::parse(&file.out).unwrap();
                    let mut tile_cursor = reader.c.at(new_level + 8);
                    let old_tile = tile_cursor.ptr().unwrap();
                    file.patch(new_level + 8, if points_to_gap { old_end } else { old_tile + gap * width });
                    let reader = Xcf::parse(&file.out).unwrap();
                    let mut seen = Vec::new();
                    let visit = |tile: SourceTile<'_>| {
                        seen.extend_from_slice(tile.data);
                        Ok(())
                    };
                    let result = if role == "channel" {
                        reader.visit_channel_tiles(0, SampleOptions::default(), &mut 2, GO, visit).map(|_| ())
                    } else {
                        reader.visit_layer_mask_tiles(0, SampleOptions::default(), &mut 2, GO, visit).map(|_| ())
                    };
                    let legacy = if role == "channel" {
                        reader.channel(0, &mut 8, GO).unwrap().1
                    } else {
                        reader.layer_mask(0, &mut 8, GO).unwrap().unwrap().1
                    };
                    result.unwrap();
                    let expected = if points_to_gap { [if zero { 0 } else { 77 }; 2] } else { [22, 44] };
                    assert_eq!(seen, expected, "v{version} {role}, {gap} gap words");
                    assert_eq!(legacy.data, expected.map(|value| f32::from(value) / 255.));
                }
            }
        }
    }
}
