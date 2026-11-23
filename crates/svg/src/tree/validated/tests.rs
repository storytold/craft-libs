#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
use super::*;
use std::io::Write;
const PIXELS: u64 = 24_000_000;
const SVG_BYTES: usize = 16 << 20;

fn test_limits(pixels: u64) -> Limits {
    Limits {
        bytes: 1 << 30,
        depth: 513,
        tree_depth: 513,
        embedded_bytes: SVG_BYTES,
        embedded_depth: 32,
        embedded_raster_pixels: pixels,
        ..Default::default()
    }
}
fn wrapped_options(pixels: u64) -> (usvg::Options<'static>, Arc<Mutex<State>>, Arc<ValidatedImageResolver>) {
    let resolver = Arc::new(ValidatedImageResolver::new(test_limits(pixels)));
    let options = usvg::Options { image_href_resolver: resolver.resolver(), ..Default::default() };
    (options, Arc::clone(&resolver.state), resolver)
}
fn on_test_stack<T: Send>(run: impl FnOnce() -> Result<T, Error> + Send) -> Result<T, Error> {
    std::thread::scope(|scope| std::thread::Builder::new().stack_size(16 << 20).spawn_scoped(scope, run).unwrap().join().unwrap())
}
fn png() -> Vec<u8> {
    let image = craft_image_codecs::Image::from_u8(2, 2, craft_image_codecs::ChannelLayout::Rgb, [17, 83, 149].repeat(4)).unwrap();
    craft_image_codecs::encode(&image, craft_image_codecs::Format::Png, &Default::default()).unwrap()
}
fn data_url(mime: &str, bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut result = format!("data:{mime},");
    for byte in bytes {
        write!(&mut result, "%{byte:02X}").unwrap();
    }
    result
}

#[test]
fn original_nested_xml_bytes_are_not_refunded_when_malformed_hrefs_shrink() {
    on_test_stack(|| {
        let (options, state, _resolver) = wrapped_options(PIXELS);
        let prefix = "<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'><image width='1' height='1' href='data:image/png;base64,";
        let suffix = "!'/></svg>";
        let original_size = 3 << 20;
        let source = Arc::new([prefix.as_bytes(), &vec![b'A'; original_size - prefix.len() - suffix.len()], suffix.as_bytes()].concat());
        assert_eq!(source.len(), original_size);
        for accepted in 1..=5 {
            let kind = (options.image_href_resolver.resolve_data)("image/svg+xml", Arc::clone(&source), &options);
            assert!(matches!(kind, Some(usvg::ImageKind::SVG(_))), "source {accepted} fits the original-text allowance");
            drop(kind);
            let state = state.lock().unwrap_or_else(PoisonError::into_inner);
            assert_eq!(state.svg_bytes_left, SVG_BYTES - accepted * original_size);
            assert!(!state.svg_failure_exhausted);
            assert_eq!(state.classification_bytes_left, SVG_BYTES);
            let usage = state.usage();
            assert_eq!(usage.embedded_svg_bytes, accepted * original_size, "the short replacement cannot refund original XML work");
        }
        assert!((options.image_href_resolver.resolve_data)("image/svg+xml", Arc::clone(&source), &options).is_none());
        let state = state.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(state.svg_bytes_left, 0);
        assert_eq!((state.svg_limits, state.svg_resource_limits), (1, 1));
        assert!(!state.svg_failure_exhausted, "ordinary byte exhaustion cannot enter compatibility classification");
        assert_eq!(state.classification_bytes_left, SVG_BYTES);
        assert_eq!(state.usage().svg_resource_limit_images, 1);
        Ok(())
    })
    .unwrap();
}

#[test]
fn invalid_utf8_gzip_consumes_remaining_svg_work_before_the_next_child() {
    on_test_stack(|| {
        let (options, state, _resolver) = wrapped_options(PIXELS);
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&[0xff; 4096]).unwrap();
        let invalid = Arc::new(encoder.finish().unwrap());
        assert!((options.image_href_resolver.resolve_data)("image/svg+xml", invalid, &options).is_none());
        {
            let state = state.lock().unwrap_or_else(PoisonError::into_inner);
            assert_eq!(state.svg_bytes_left, 0, "a failed decode cannot retry the full work allowance");
            assert_eq!(state.invalid_svg, 1);
            assert!(state.svg_failure_exhausted);
            assert_eq!(state.svg_resource_limits, 0);
            assert_eq!(state.classification_bytes_left, SVG_BYTES);
        }
        let valid = Arc::new(b"<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'><rect width='1' height='1'/></svg>".to_vec());
        let valid_size = valid.len();
        assert!((options.image_href_resolver.resolve_data)("image/svg+xml", valid, &options).is_none());
        let state = state.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!((state.invalid_svg, state.svg_limits, state.svg_resource_limits), (1, 1, 0));
        assert_eq!(state.classification_bytes_left, SVG_BYTES, "plain input does not spend retry inflation work");
        let usage = state.usage();
        assert_eq!(usage.embedded_svg_bytes, SVG_BYTES);
        assert_eq!(usage.svg_resource_limit_images, 0, "a later small child is omitted, not a resource error");
        let charged = 2 * SVG_BYTES - state.svg_bytes_left - state.classification_bytes_left;
        assert_eq!(charged, SVG_BYTES);
        assert!(valid_size < SVG_BYTES);
        Ok(())
    })
    .unwrap();
}

#[test]
fn post_failure_classification_is_cumulative_and_never_admits_later_children() {
    on_test_stack(|| {
        let (options, state, _resolver) = wrapped_options(PIXELS);
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&[0xff; 256]).unwrap();
        assert!((options.image_href_resolver.resolve_data)("image/svg+xml", Arc::new(encoder.finish().unwrap()), &options).is_none());
        let text = b"<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'/>";
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(text).unwrap();
        let valid = Arc::new(encoder.finish().unwrap());
        // Exercise the cumulative inflated-byte boundary with a small injected
        // second pool; public tests cover the full 16 MiB allowance.
        let classification_limit = text.len() * 2 - 1;
        state.lock().unwrap_or_else(PoisonError::into_inner).classification_bytes_left = classification_limit;
        assert!((options.image_href_resolver.resolve_data)("image/svg+xml", Arc::clone(&valid), &options).is_none());
        {
            let state = state.lock().unwrap_or_else(PoisonError::into_inner);
            assert_eq!(state.classification_bytes_left, text.len() - 1);
            assert_eq!((state.svg_limits, state.svg_resource_limits), (1, 0));
        }
        assert!((options.image_href_resolver.resolve_data)("image/svg+xml", Arc::clone(&valid), &options).is_none());
        let state = state.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(state.classification_bytes_left, 0);
        assert_eq!((state.svg_limits, state.svg_resource_limits), (2, 1));
        assert_eq!(state.svg_bytes_left, 0, "classification never refunds original failed work");
        assert!(state.svg_failure_exhausted);
        let charged = 2 * SVG_BYTES - state.svg_bytes_left - state.classification_bytes_left;
        assert_eq!(charged, 2 * SVG_BYTES, "original content plus aggregate retry inflation allowances remain bounded");
        assert_eq!(state.usage().embedded_svg_count, 0, "classified children must never reach the shared parser");
        Ok(())
    })
    .unwrap();
}

#[test]
fn retry_inflation_room_and_saved_decoded_content_room_remain_independent() {
    on_test_stack(|| {
        let mut latin1 = b"<?xml version='1.0' encoding='ISO-8859-1'?><svg xmlns='http://www.w3.org/2000/svg'><!--".to_vec();
        latin1.extend([0xe9; 100]);
        latin1.extend(b"--></svg>");
        let decoded_len = crate::svgz::decode(&latin1).unwrap().len();
        assert!(latin1.len() < 256 && decoded_len > 256 && decoded_len < 512);
        let gzip = |bytes: &[u8]| {
            let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(bytes).unwrap();
            Arc::new(encoder.finish().unwrap())
        };
        for (content_room, expected_resource) in [(512usize, false), (256, true)] {
            let (options, state, _resolver) = wrapped_options(PIXELS);
            // Model preceding genuine content consumption before an unreadable
            // stream burns the rest. Classification must save this exact room.
            state.lock().unwrap_or_else(PoisonError::into_inner).svg_bytes_left = content_room;
            assert!((options.image_href_resolver.resolve_data)("image/svg+xml", gzip(&[0xff; 4]), &options).is_none());
            {
                let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
                assert_eq!(state.classification_content_left, content_room);
                assert_eq!(state.svg_bytes_left, 0);
                // Enough compressed retry work for the raw Latin-1 bytes, but
                // less than their decoded UTF-8 length, just like main's case.
                state.classification_bytes_left = latin1.len() + 1;
            }
            assert!((options.image_href_resolver.resolve_data)("image/svg+xml", gzip(&latin1), &options).is_none());
            let state = state.lock().unwrap_or_else(PoisonError::into_inner);
            assert_eq!(state.classification_bytes_left, 1, "retry work charges inflated bytes, not UTF-8 bytes");
            assert_eq!(state.svg_resource_limits, usize::from(expected_resource));
            assert_eq!(state.svg_bytes_left, 0, "classification never restores original content allowance");
            assert_eq!(state.usage().embedded_svg_count, 0, "no classified child is parsed or drawn");
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn failed_raster_callbacks_release_sources_and_keep_every_decode_charge() {
    let mut broken = png();
    let idat = broken.windows(4).position(|bytes| bytes == b"IDAT").unwrap();
    broken[idat + 4] ^= 1;
    assert_eq!(craft_image_codecs::probe_allocation_size(&broken).unwrap(), (2, 2));
    let (options, state, resolver) = wrapped_options(8);
    let encoded_before = state.lock().unwrap().encoded_left;
    for attempt in 1..=2 {
        // Match usvg's actual callback boundary: each href gets a new Arc.
        let encoded = Arc::new(broken.clone());
        let weak = Arc::downgrade(&encoded);
        assert!((options.image_href_resolver.resolve_data)("image/png", Arc::clone(&encoded), &options).is_none());
        let usage = resolver.usage();
        assert_eq!(usage.raster_pixels, attempt * 4);
        assert_eq!(state.lock().unwrap().encoded_left, encoded_before - attempt as usize * encoded.len());
        assert_eq!(usage.invalid_raster_images, attempt as usize);
        drop(encoded);
        assert!(weak.upgrade().is_none(), "validation must not retain failed callback buffers until parse completion");
    }
    assert!((options.image_href_resolver.resolve_data)("image/png", Arc::new(png()), &options).is_none());
    let usage = resolver.usage();
    assert_eq!((usage.invalid_raster_images, usage.raster_limit_images, usage.raster_pixels), (2, 1, 8));
}

#[test]
fn valid_raster_callbacks_spend_exact_pixel_and_encoded_allowances_per_call() {
    let encoded = Arc::new(png());
    let (options, state, resolver) = wrapped_options(8);
    let call = || (options.image_href_resolver.resolve_data)("image/png", Arc::clone(&encoded), &options);
    state.lock().unwrap().encoded_left = encoded.len() * 2;
    assert!(call().is_some());
    assert_eq!(resolver.usage().raster_pixels, 4);
    assert_eq!(state.lock().unwrap().encoded_left, encoded.len());
    assert!(call().is_some(), "even a directly reused Arc is a new actual validation call");
    assert_eq!(state.lock().unwrap().encoded_left, 0);
    assert!(call().is_none());
    let usage = resolver.usage();
    assert_eq!((usage.raster_pixels, usage.invalid_raster_images, usage.raster_limit_images), (8, 0, 1));
}

#[test]
fn raster_decode_happens_after_the_work_is_charged_and_mutex_released() {
    let (_, state, resolver) = wrapped_options(4);
    let image = png();
    assert!(resolver.raster_with(&image, |admitted| {
        assert_eq!(admitted, 4);
        assert!(state.try_lock().is_ok(), "a usage snapshot must not wait for the raster decode");
        assert_eq!(resolver.usage().raster_pixels, 4);
        Ok(())
    }));
}

#[test]
fn raster_callbacks_own_per_call_pixel_charges_and_keep_mime_aliases_separate() {
    let png = Arc::new(png());
    let (options, state, resolver) = wrapped_options(4);
    assert!((options.image_href_resolver.resolve_data)("image/png", Arc::clone(&png), &options).is_some());
    assert!((options.image_href_resolver.resolve_data)("image/png", Arc::clone(&png), &options).is_none());
    assert_eq!((resolver.usage().raster_pixels, resolver.usage().invalid_raster_images, resolver.usage().raster_limit_images), (4, 0, 1));
    assert!((options.image_href_resolver.resolve_data)("image/jpeg", png, &options).is_none());
    let usage = state.lock().unwrap().usage();
    assert_eq!((usage.raster_pixels, usage.invalid_raster_images), (4, 1));
}

#[test]
fn encoded_raster_refusal_is_per_image_and_does_not_consume_unattempted_work() {
    let bytes = png();
    let mut oversized = bytes.clone();
    oversized.extend([0; 32]);
    let (options, state, resolver) = wrapped_options(4);
    state.lock().unwrap().encoded_left = bytes.len();
    assert!((options.image_href_resolver.resolve_data)("image/png", Arc::new(oversized), &options).is_none());
    assert_eq!(state.lock().unwrap().encoded_left, bytes.len());
    assert_eq!(resolver.usage().raster_pixels, 0);
    assert!((options.image_href_resolver.resolve_data)("image/png", Arc::new(bytes), &options).is_some());
    assert_eq!(state.lock().unwrap().encoded_left, 0);
    let usage = resolver.usage();
    assert_eq!((usage.raster_pixels, usage.raster_limit_images, usage.invalid_raster_images), (4, 1, 0));
}

#[test]
fn encoded_raster_exhaustion_keeps_ordinary_drawing_and_reports_only_omitted_images() {
    let bytes = png();
    let href = data_url("image/png", &bytes);
    let text = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"><rect width="4" height="2" fill="#00ff00"/><image width="2" height="2" href="{href}"/><image x="2" width="2" height="2" href="{href}"/></svg>"##
    );
    let resolver = ValidatedImageResolver::new(test_limits(8));
    resolver.state.lock().unwrap().encoded_left = bytes.len();
    let tree = resolver.parse_preflight(preflight(&text, &resolver.limits()).unwrap(), usvg::Options::default()).unwrap();
    let usage = resolver.usage();
    assert_eq!((usage.raster_pixels, usage.raster_limit_images, usage.invalid_raster_images), (4, 1, 0));
    let output = super::super::draw::render(&tree, usvg::Transform::identity(), 4, 2, 0, 1 << 20).unwrap();
    let background = output.pixel(3, 1).unwrap().demultiply();
    assert_eq!((background.red(), background.green(), background.blue(), background.alpha()), (0, 255, 0, 255));
    let pixel = output.pixel(0, 0).unwrap().demultiply();
    assert_eq!((pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()), (17, 83, 149, 255));
    assert_eq!(resolver.state.lock().unwrap().encoded_left, 0);
}

// Preserve upstream's text_rechecked boundary using the integrated real resolver.
// Plain and gzip retries spend source bytes before decoding, independently of
// inflation work and the drawing's genuine decoded-content capacity.
#[test]
fn plain_legacy_encoding_rechecks_have_a_separate_bounded_work_allowance() {
    on_test_stack(|| {
        let xml = b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><svg><!--\xe9--></svg>";
        for compressed in [false, true] {
            let (options, state, _resolver) = wrapped_options(PIXELS);
            let encode = |bytes: &[u8]| {
                if compressed {
                    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                    encoder.write_all(bytes).unwrap();
                    encoder.finish().unwrap()
                } else {
                    bytes.to_vec()
                }
            };
            // A failed compressed decode exhausts the primary pool. The
            // callbacks under test may use plain or compressed encodings.
            let invalid = Arc::new(crate::svgz::compress(&[0xff]));
            assert!((options.image_href_resolver.resolve_data)("image/svg+xml", invalid, &options).is_none());
            state.lock().unwrap_or_else(PoisonError::into_inner).classification_text_bytes_left = xml.len();
            assert!((options.image_href_resolver.resolve_data)("image/svg+xml", Arc::new(encode(xml)), &options).is_none());
            {
                let state = state.lock().unwrap_or_else(PoisonError::into_inner);
                assert_eq!(state.classification_text_bytes_left, 0);
                assert_eq!(state.svg_resource_limits, 0, "exactly fitting text recheck is permitted");
                assert_eq!(state.classification_bytes_left, SVG_BYTES - usize::from(compressed) * xml.len());
            }
            assert!((options.image_href_resolver.resolve_data)("image/svg+xml", Arc::new(encode(xml)), &options).is_none());
            let state = state.lock().unwrap_or_else(PoisonError::into_inner);
            assert_eq!(state.classification_text_bytes_left, 0);
            assert_eq!(state.svg_resource_limits, 1, "the next source cannot repeat text-conversion work");
            assert_eq!(state.invalid_svg, 1, "the original encoding error stays invalid");
            assert_eq!(state.svg_limits, 2);
            assert_eq!(state.usage().embedded_svg_count, 0, "rechecked children are never admitted");
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn embedded_gzip_obeys_document_bytes_before_aggregate_inflation_room() {
    on_test_stack(|| {
        let mut text = b"<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'><!--".to_vec();
        text.resize(256 - b"--></svg>".len(), b' ');
        text.extend_from_slice(b"--></svg>");
        assert_eq!(text.len(), 256);
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&text).unwrap();
        let encoded = Arc::new(encoder.finish().unwrap());
        for bytes in [text.len() - 1, text.len()] {
            let limits = Limits { bytes, embedded_bytes: 1024, ..test_limits(4) };
            assert!(limits.embedded_bytes > text.len());
            let resolver = ValidatedImageResolver::new(limits);
            let options = usvg::Options { image_href_resolver: resolver.resolver(), ..Default::default() };
            let image = (options.image_href_resolver.resolve_data)("image/svg+xml", Arc::clone(&encoded), &options);
            let usage = resolver.usage();
            if bytes == text.len() {
                assert!(matches!(image, Some(usvg::ImageKind::SVG(_))), "exact document boundary remains admissible");
                assert_eq!((usage.embedded_svg_count, usage.embedded_svg_bytes), (1, text.len()));
                assert_eq!((usage.svg_resource_limit_images, usage.invalid_svg_images), (0, 0));
            } else {
                assert!(image.is_none(), "aggregate room must not lift one document's inflate limit");
                assert_eq!((usage.embedded_svg_count, usage.svg_resource_limit_images, usage.invalid_svg_images), (0, 1, 0));
                assert_eq!(usage.embedded_svg_bytes, 1024, "refused work retains its aggregate charge");
            }
        }
        Ok(())
    })
    .unwrap();
}
