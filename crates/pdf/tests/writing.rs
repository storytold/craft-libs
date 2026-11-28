//! Integration tests of craft-pdf: writing.

#![cfg(feature = "write")]
#![allow(clippy::unwrap_used, clippy::panic)]

use backend::configure::PdfVersion;
use backend::geom::{PathBuilder, Rect, Size};
use backend::page::PageSettings;
use backend::paint::Fill;
use craft_pdf::write::{self, Metadata, WriteError, Writer, backend};

fn document(compress: bool) -> Vec<u8> {
    let metadata = Metadata {
        creator: "Craft writer contract".into(),
        producer: "Craft writer contract".into(),
        title: Some("Synthetic export".into()),
        authors: vec!["Test author".into()],
        description: Some("Original synthetic artwork".into()),
        keywords: vec!["craft".into(), "pdf".into()],
        created: Some(backend::metadata::DateTime::new(2026).month(10).day(9)),
        modified: Some(backend::metadata::DateTime::new(2026).month(10).day(12)),
    };
    let settings = backend::SerializeSettings {
        compress_content_streams: compress,
        configuration: write::configuration(PdfVersion::Pdf16, None).unwrap(),
        ..Default::default()
    };
    let mut writer = Writer::new(settings, metadata);
    for (width, height) in [(80.0, 40.0), (30.0, 60.0)] {
        let mut page = writer.document_mut().start_page_with(PageSettings::new(Size::from_wh(width, height).unwrap()));
        let mut surface = page.surface();
        let mut path = PathBuilder::new();
        path.push_rect(Rect::from_xywh(2.0, 3.0, 15.0, 12.0).unwrap());
        surface.set_fill(Some(Fill { paint: backend::color::rgb::Color::new(20, 80, 160).into(), ..Default::default() }));
        surface.draw_path(&path.finish().unwrap());
        surface.finish();
        page.finish();
    }
    writer.finish().unwrap()
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn fixed_input_does_not_read_a_clock_or_change_pdf_bytes() {
    assert_eq!(document(false), document(false));
    assert_eq!(document(true), document(true));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn retaining_a_writer_after_page_unwind_cannot_serialize_missing_pages() {
    for mode in ["page", "surface", "explicit finish"] {
        let settings = backend::SerializeSettings { enable_tagging: false, ..Default::default() };
        let mut writer = Writer::new(settings, Metadata::default());
        writer.document_mut().start_page_with(PageSettings::from_wh(40.0, 20.0).unwrap()).finish();
        let result: Result<(), WriteError> = if mode == "page" {
            write::guard(|| {
                let _page = writer.document_mut().start_page_with(PageSettings::from_wh(40.0, 20.0).unwrap());
                panic!("synthetic failed middle page");
            })
        } else {
            // Keep the page outside the narrow guard: successful later page cleanup
            // must not clear poison from an unwound or explicitly failed surface.
            let mut page = writer.document_mut().start_page_with(PageSettings::from_wh(40.0, 20.0).unwrap());
            let result = write::guard(|| {
                let mut surface = page.surface();
                surface.push_transform(&backend::geom::Transform::from_translate(2.0, 3.0));
                if mode == "explicit finish" {
                    surface.finish();
                    return Ok(());
                }
                panic!("synthetic failed middle surface");
            });
            page.finish();
            result
        };
        assert!(matches!(result, Err(WriteError::BackendPanic(_))));
        writer.document_mut().start_page_with(PageSettings::from_wh(40.0, 20.0).unwrap()).finish();
        assert!(
            matches!(writer.finish(), Err(WriteError::Serialization(error)) if matches!(*error, backend::error::KrillaError::Poisoned)),
            "a retained failed writer must reject serialization after {mode}"
        );
    }
}

// Original pixel recipes from issue #561 (OPUZZLES); no app, font or image assets.
#[cfg(feature = "filters")]
mod row_prediction {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::Arc;

    use backend::color::device_n::{Colorant, DeviceNSpace, Process};
    use backend::image::{BitsPerComponent, CustomImage, Image, ImageColorspace};
    use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
    use hayro_syntax::object::{Array, Dict, Name, Object, Stream};

    // Original spec-built64x32 RGBA16 PNG: the custom RGB16/alpha recipe below,
    // big-endian samples; PNG color type6, no interlace, Up rows (tag2).
    // Generated only with Python stdlib struct/zlib (including chunk CRCs); no image codec/assets.
    const PNG16: &[u8] = b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0d\x49\x48\x44\x52\x00\x00\x00\x40\x00\x00\x00\x20\
        \x10\x06\x00\x00\x00\xf2\x0d\xa2\xc7\x00\x00\x01\x7d\x49\x44\x41\x54\x78\xda\xed\xd2\x21\x48\x43\
        \x51\x00\x85\xe1\xbb\x87\xc2\x34\xe8\x40\xc3\x0c\xca\xd0\xe0\x82\xc2\x34\xe8\x40\xc3\x2c\x32\xb0\
        \xac\x08\xc3\xe2\x82\x61\x16\x19\x58\x5e\x11\x86\xc5\x05\xc3\x2c\x32\xb0\xcc\x30\x50\x8b\x0a\x06\
        \x57\x44\xb1\xcc\x20\xa8\x45\x05\x83\x2b\xa2\x58\x66\x10\xd4\xf2\xe4\xf1\x07\x45\xb3\x96\x7b\x4e\
        \xb8\x61\xef\x83\xc1\xbd\xbf\x63\xbc\x80\xf1\x8c\x31\xfe\xe9\x79\xc1\x66\xff\x34\x26\xd4\xea\x9f\
        \xc6\x0b\xb7\xfb\x67\xc0\x44\x3a\x71\xd1\x2e\x5c\xac\x07\x17\xef\xc3\x25\xa2\xb8\xe4\x20\x2e\x35\
        \x8c\x4b\x8f\xe2\x32\xe3\xb8\xec\x04\x2e\x37\x89\x73\xa7\x70\xf9\x14\xae\x30\x8d\x2b\xce\xe0\x4a\
        \xb3\xb8\xf2\x1c\x6e\x6b\x1e\xb7\xb7\x80\x3b\x5c\xc4\x1d\xbb\xb8\xda\x12\xee\x62\x19\x77\xb3\x82\
        \xbb\x5f\xc5\x3d\xae\xe1\x1a\xeb\xb8\xb7\x0d\x9c\x29\xe3\x82\x15\x5c\x68\x07\x17\xde\xc5\x45\x0e\
        \x70\xd1\x2a\x2e\x76\x84\x8b\x9f\xe2\x12\x35\x5c\xf2\x1c\x97\xba\xc2\xa5\xaf\x71\x99\x3b\x5c\xb6\
        \x8e\xcb\x3d\xe0\xdc\x67\x5c\xbe\x81\x2b\xbc\xe2\x8a\x1f\xb8\x92\x87\xdb\x6c\xc2\x6d\xb7\xe0\xf6\
        \xdb\x70\xd5\x0e\xdc\x49\x18\x77\xd6\x8d\xbb\xec\xc5\xdd\xf6\xe3\xea\x03\xb8\xa7\x21\xdc\xcb\x08\
        \xee\x7d\xcc\x71\x02\x5f\x73\x7e\x9c\xce\xaf\x5f\xe5\xfe\xd7\x39\x7f\xfe\xbf\xdf\xbf\x6b\x16\x4e\
        \x01\x28\x00\x4d\x01\x68\x0a\x40\xb3\xf3\x59\x14\x80\x52\xd3\x14\x80\xa6\x00\x34\x05\xa0\x59\xf8\
        \x1c\x0a\x40\xc9\x69\x0a\x40\x53\x00\x9a\x02\xd0\x2c\x7c\x06\x05\xa0\xf4\x34\x05\xa0\x29\x00\x4d\
        \x01\x68\x16\x5e\xbf\x02\x50\x82\x9a\x02\xd0\x14\x80\xa6\x00\x34\x0b\xaf\x5d\x01\x28\x45\x4d\x01\
        \x68\x0a\x40\x53\x00\x9a\x85\xd7\xfd\x09\x61\x07\x13\x9b\x10\xb0\x38\x81\x00\x00\x00\x00\x49\x45\
        \x4e\x44\xae\x42\x60\x82";

    #[derive(Clone, Hash)]
    struct Samples {
        color: Vec<u8>,
        alpha: Option<Vec<u8>>,
        size: (u32, u32),
        bits: BitsPerComponent,
        space: ImageColorspace,
    }

    impl CustomImage for Samples {
        fn color_channel(&self) -> &[u8] {
            &self.color
        }
        fn alpha_channel(&self) -> Option<&[u8]> {
            self.alpha.as_deref()
        }
        fn bits_per_component(&self) -> BitsPerComponent {
            self.bits
        }
        fn size(&self) -> (u32, u32) {
            self.size
        }
        fn icc_profile(&self) -> Option<&[u8]> {
            None
        }
        fn color_space(&self) -> ImageColorspace {
            self.space
        }
        fn device_n_space(&self) -> Option<DeviceNSpace> {
            (self.space == ImageColorspace::DeviceN)
                .then(|| DeviceNSpace::new([Colorant::Process(Process::Magenta), Colorant::Process(Process::Cyan)]).unwrap())
        }
    }

    struct Row {
        name: &'static str,
        samples: Samples,
        rgba: bool,
        png: bool,
        must_predict: bool,
        must_keep_raw: bool,
    }

    struct Benefit {
        name: &'static str,
        ascii: bool,
        predictor: Option<i32>,
        selected: usize,
        unpredicted: usize,
    }

    fn components(space: ImageColorspace) -> usize {
        match space {
            ImageColorspace::Luma => 1,
            ImageColorspace::Rgb => 3,
            ImageColorspace::Cmyk => 4,
            ImageColorspace::DeviceN => 2,
        }
    }

    fn bpc(bits: BitsPerComponent) -> i32 {
        match bits {
            BitsPerComponent::Eight => 8,
            BitsPerComponent::Sixteen => 16,
        }
    }

    fn raw_deflate(data: &[u8]) -> Vec<u8> {
        // The existing writer's raw control: the same zlib level, with no row transform.
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(6));
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    /// Serialize complete candidate filter-stream objects, including Length and DecodeParms.
    /// The common image dictionary entries cancel; this control uses the shared framing writer.
    fn complete_stream_size(data: &[u8], samples: &Samples, predicted: bool, ascii: bool) -> usize {
        let mut encoded = Vec::new();
        if ascii {
            for (i, byte) in data.iter().enumerate() {
                write!(&mut encoded, "{byte:02X}").unwrap();
                if (i + 1) % 35 == 0 {
                    encoded.push(b'\n');
                }
            }
        } else {
            encoded.extend_from_slice(data);
        }
        let filter = if ascii { "[/ASCIIHexDecode /FlateDecode]" } else { "/FlateDecode" };
        let parms = if predicted {
            let dict = format!(
                "<< /Predictor 12 /Columns {} /Colors {} /BitsPerComponent {} >>",
                samples.size.0,
                components(samples.space),
                bpc(samples.bits)
            );
            if ascii { format!(" /DecodeParms [null {dict}]") } else { format!(" /DecodeParms {dict}") }
        } else {
            String::new()
        };
        let mut writer = craft_pdf::classic::Writer::with_capacity(0).unwrap();
        writer.object(1, "<< /Type /Catalog /Pages 2 0 R >>").unwrap();
        writer.object(2, "<< /Type /Pages /Kids [] /Count 0 >>").unwrap();
        writer.stream(3, &format!("/Filter {filter}{parms}"), &encoded).unwrap();
        let pdf = writer.finish(1, None).unwrap();
        // The one control stream is the last object, immediately before the classic xref table.
        let start = pdf.windows(b"3 0 obj\n".len()).position(|bytes| bytes == b"3 0 obj\n").unwrap();
        let end = pdf.windows(b"\nxref\n".len()).rposition(|bytes| bytes == b"\nxref\n").unwrap();
        end - start
    }

    fn image(row: &Row) -> Image {
        if row.png {
            Image::from_png(PNG16.to_vec().into(), false).unwrap()
        } else if row.rgba {
            let rgba = row.samples.color.as_chunks::<3>().0.iter().flat_map(|rgb| [rgb[0], rgb[1], rgb[2], 255]).collect();
            Image::from_rgba8(rgba, row.samples.size.0, row.samples.size.1)
        } else {
            Image::from_custom(row.samples.clone(), false).unwrap()
        }
    }

    fn export(row: &Row, ascii: bool) -> Vec<u8> {
        let settings = backend::SerializeSettings {
            ascii_compatible: ascii,
            no_device_cs: false,
            configuration: write::configuration(PdfVersion::Pdf17, None).unwrap(),
            ..Default::default()
        };
        let mut writer = Writer::new(settings, Metadata::default());
        let mut page = writer.document_mut().start_page_with(PageSettings::from_wh(100.0, 80.0).unwrap());
        let mut surface = page.surface();
        surface.draw_image(image(row), Size::from_wh(100.0, 80.0).unwrap());
        surface.finish();
        page.finish();
        writer.finish().unwrap()
    }

    fn check_plane(stream: &Stream<'_>, expected: &[u8], samples: &Samples, colors: usize, ascii: bool) -> Option<i32> {
        let dict = stream.dict();
        assert_eq!(dict.get::<Name<'_>>(b"Subtype").unwrap().as_str(), "Image");
        assert_eq!(dict.get::<i32>(b"Width"), Some(samples.size.0 as i32));
        assert_eq!(dict.get::<i32>(b"Height"), Some(samples.size.1 as i32));
        assert_eq!(dict.get::<i32>(b"BitsPerComponent"), Some(bpc(samples.bits)));
        assert_eq!(stream.decoded().unwrap().as_ref(), expected, "all samples, including 16-bit byte rollover");

        let has_params = dict.contains_key(b"DecodeParms");
        let params = if ascii {
            let filters: Vec<_> = dict.get::<Array<'_>>(b"Filter").unwrap().iter::<Name<'_>>().map(|n| n.as_str().to_owned()).collect();
            assert_eq!(filters, ["ASCIIHexDecode", "FlateDecode"]);
            has_params.then(|| {
                let array = dict.get::<Array<'_>>(b"DecodeParms").unwrap();
                let slots: Vec<_> = array.iter::<Object<'_>>().collect();
                assert_eq!(slots.len(), 2, "one DecodeParms slot per filter");
                assert!(matches!(slots[0], Object::Null(_)), "ASCIIHex has no parameters");
                slots[1].clone().into_dict().unwrap()
            })
        } else {
            assert_eq!(dict.get::<Name<'_>>(b"Filter").unwrap().as_str(), "FlateDecode");
            has_params.then(|| dict.get::<Dict<'_>>(b"DecodeParms").unwrap())
        };
        params.map(|params| {
            let predictor = params.get::<i32>(b"Predictor").unwrap();
            assert_eq!(predictor, 12);
            assert_eq!(params.get::<i32>(b"Colors"), Some(colors as i32));
            assert_eq!(params.get::<i32>(b"Columns"), Some(samples.size.0 as i32));
            assert_eq!(params.get::<i32>(b"BitsPerComponent"), Some(bpc(samples.bits)));
            if !ascii {
                let encoded = stream.raw_data();
                let mut tagged_rows = Vec::new();
                ZlibDecoder::new(encoded.as_ref()).read_to_end(&mut tagged_rows).unwrap();
                let stride = samples.size.0 as usize * colors * (bpc(samples.bits) as usize / 8) + 1;
                assert_eq!(tagged_rows.len(), stride * samples.size.1 as usize);
                assert!(tagged_rows.chunks_exact(stride).all(|row| row[0] == 2), "PNG Up tag on every row");
            }
            predictor
        })
    }

    fn check_row(row: &Row, ascii: bool) -> Option<Benefit> {
        let pdf = hayro_syntax::Pdf::new(Arc::new(export(row, ascii))).unwrap();
        let page = &pdf.pages()[0];
        assert_eq!(page.render_dimensions(), (100.0, 80.0));
        let resources = &page.resources().x_objects;
        let keys: Vec<_> = resources.keys().collect();
        assert_eq!(keys.len(), 1, "{}: one drawn image", row.name);
        let stream = resources.get::<Stream<'_>>(keys[0].as_ref()).unwrap();
        let predictor = check_plane(&stream, &row.samples.color, &row.samples, components(row.samples.space), ascii);
        let dict = stream.dict();
        if row.samples.space == ImageColorspace::DeviceN {
            let cs: Vec<_> = dict.get::<Array<'_>>(b"ColorSpace").unwrap().iter::<Object<'_>>().collect();
            assert_eq!(cs[0].clone().into_name().unwrap().as_str(), "DeviceN");
            let names: Vec<_> = cs[1].clone().into_array().unwrap().iter::<Name<'_>>().map(|n| n.as_str().to_owned()).collect();
            assert_eq!(names, ["Magenta", "Cyan"], "preserve colorant/sample order");
        } else {
            let name = match row.samples.space {
                ImageColorspace::Luma => Some("DeviceGray"),
                ImageColorspace::Rgb => Some("DeviceRGB"),
                ImageColorspace::Cmyk => Some("DeviceCMYK"),
                ImageColorspace::DeviceN => None,
            };
            assert!(name.is_some(), "{}: DeviceN fixture needs the colorant ColorSpace check", row.name);
            assert_eq!(dict.get::<Name<'_>>(b"ColorSpace").unwrap().as_str(), name.unwrap());
        }
        if let Some(alpha) = &row.samples.alpha {
            let mask = dict.get::<Stream<'_>>(b"SMask").unwrap();
            assert_eq!(mask.dict().get::<Name<'_>>(b"ColorSpace").unwrap().as_str(), "DeviceGray");
            let mask_predictor = check_plane(&mask, alpha, &row.samples, 1, ascii);
            if row.samples.space == ImageColorspace::Luma && row.samples.bits == BitsPerComponent::Sixteen {
                // This multi-row rollover mask is compressible; other masks may legitimately keep raw Flate.
                assert_eq!(mask_predictor, Some(12), "{} (ASCII={ascii}): predict the compressible soft mask", row.name);
            }
        } else {
            assert!(dict.get::<Object<'_>>(b"SMask").is_none(), "{}: no extra alpha plane", row.name);
        }
        if row.must_keep_raw {
            assert_eq!(predictor, None, "{}: prediction must not enlarge tiny/noisy images", row.name);
            let raw = raw_deflate(&row.samples.color);
            let expected_length = if ascii { raw.len() * 2 + raw.len() / 35 } else { raw.len() };
            assert_eq!(stream.raw_data().len(), expected_length, "{}: retain raw deflation", row.name);
        }
        row.must_predict.then(|| {
            let raw = raw_deflate(&row.samples.color);
            Benefit {
                name: row.name,
                ascii,
                predictor,
                // Compare like encoded payloads. The separate overhead control below compares
                // complete streams; tiny/noisy rows also retain their raw fallback checks.
                selected: stream.raw_data().len(),
                unpredicted: if ascii { raw.len() * 2 + raw.len() / 35 } else { raw.len() },
            }
        })
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn preserves_samples_and_compresses_redundant_rows() {
        let mut rows = Vec::new();
        for pattern in [false, true] {
            let mut color = Vec::with_capacity(1000 * 800 * 3);
            for y in 0..800u32 {
                for x in 0..1000u32 {
                    let rgb = if pattern {
                        [((x * 31 + y * 17) % 256) as u8, ((x * 7 + y * 53) % 256) as u8, ((x * 11 + y * 23) % 256) as u8]
                    } else {
                        [(x * 255 / 999) as u8, (y * 255 / 799) as u8, ((x + y) * 127 / 899) as u8]
                    };
                    color.extend_from_slice(&rgb);
                }
            }
            rows.push(Row {
                name: if pattern { "repeating RGB" } else { "gradient RGB" },
                samples: Samples { color, alpha: None, size: (1000, 800), bits: BitsPerComponent::Eight, space: ImageColorspace::Rgb },
                rgba: true,
                png: false,
                must_predict: true,
                must_keep_raw: false,
            });
        }
        for space in [ImageColorspace::Luma, ImageColorspace::Rgb, ImageColorspace::Cmyk, ImageColorspace::DeviceN] {
            for bits in [BitsPerComponent::Eight, BitsPerComponent::Sixteen] {
                let mut color = Vec::new();
                let mut alpha = Vec::new();
                for y in 0..32u16 {
                    for x in 0..64u16 {
                        for c in 0..components(space) {
                            // Carries cross both low- and high-byte boundaries between rows.
                            let sample = [0x00ffu16, 0x0100, 0xff00, 0xffff, 0][(x as usize + c) % 5].wrapping_add(y * 257);
                            if bits == BitsPerComponent::Sixteen {
                                color.extend_from_slice(&sample.to_be_bytes());
                            } else {
                                color.push(sample as u8);
                            }
                        }
                        let sample = x.wrapping_mul(2053).wrapping_add(y * 257);
                        if bits == BitsPerComponent::Sixteen {
                            alpha.extend_from_slice(&sample.to_be_bytes());
                        } else {
                            alpha.push(sample as u8);
                        }
                    }
                }
                rows.push(Row {
                    name: if bits == BitsPerComponent::Sixteen && space == ImageColorspace::Luma {
                        "16-bit gray rollover"
                    } else {
                        "custom color and alpha"
                    },
                    samples: Samples { color, alpha: Some(alpha), size: (64, 32), bits, space },
                    rgba: false,
                    png: false,
                    must_predict: bits == BitsPerComponent::Sixteen && space == ImageColorspace::Luma,
                    must_keep_raw: false,
                });
            }
        }
        let mut random = 0x561u32;
        let noisy = (0..37 * 29 * 3)
            .map(|_| {
                random ^= random << 13;
                random ^= random >> 17;
                random ^= random << 5;
                random as u8
            })
            .collect();
        for (name, color, size) in [("tiny RGB", vec![7, 129, 253], (1, 1)), ("noisy RGB", noisy, (37, 29))] {
            rows.push(Row {
                name,
                samples: Samples { color, alpha: None, size, bits: BitsPerComponent::Eight, space: ImageColorspace::Rgb },
                rgba: true,
                png: false,
                must_predict: false,
                must_keep_raw: true,
            });
        }

        // Prediction saves payload bytes here, but cannot pay for its serialized parameters in
        // either output mode. A payload-only selection would wrongly predict this multi-row image.
        let overhead = Samples {
            color: (0..3u8).flat_map(|y| (0..4u8).flat_map(move |x| (0..3u8).map(move |c| x * 19 + y * 7 + c * 31))).collect(),
            alpha: None,
            size: (4, 3),
            bits: BitsPerComponent::Eight,
            space: ImageColorspace::Rgb,
        };
        let mut up = Vec::new();
        for (y, row) in overhead.color.as_chunks::<12>().0.iter().enumerate() {
            up.push(2);
            for (x, byte) in row.iter().enumerate() {
                let previous = if y == 0 { 0 } else { overhead.color[(y - 1) * 12 + x] };
                up.push(byte.wrapping_sub(previous));
            }
        }
        let raw = raw_deflate(&overhead.color);
        let predicted = raw_deflate(&up);
        assert!(predicted.len() < raw.len(), "the overhead control must save compressed payload bytes");
        for ascii in [false, true] {
            assert!(
                complete_stream_size(&predicted, &overhead, true, ascii) > complete_stream_size(&raw, &overhead, false, ascii),
                "ASCII={ascii}: serialized parameters must outweigh the payload saving"
            );
        }
        rows.push(Row { name: "serialized overhead RGB", samples: overhead, rgba: true, png: false, must_predict: false, must_keep_raw: true });

        let png_samples = rows
            .iter()
            .find(|row| row.samples.space == ImageColorspace::Rgb && row.samples.bits == BitsPerComponent::Sixteen)
            .unwrap()
            .samples
            .clone();
        rows.push(Row {
            name: "PNG16 RGB and alpha rollover",
            samples: png_samples,
            rgba: false,
            png: true,
            must_predict: true,
            must_keep_raw: false,
        });

        let mut benefits = Vec::new();
        for row in &rows {
            for ascii in [false, true] {
                if let Some(benefit) = check_row(row, ascii) {
                    benefits.push(benefit);
                }
            }
        }
        // All rows' decoded samples, metadata, masks, ASCII chains and fallback checks run
        // before main's expected failure: it has no row predictor on the gradient.
        for benefit in benefits {
            assert_eq!(benefit.predictor, Some(12), "{} (ASCII={}): sampled rows need PNG Up prediction", benefit.name, benefit.ascii);
            assert!(
                benefit.selected < benefit.unpredicted,
                "{} (ASCII={}): predicted payload must beat raw-deflated payload",
                benefit.name,
                benefit.ascii
            );
        }
    }
}
