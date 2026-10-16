//! Independent runtime contract for the shipping `craft-pdf/read` dependency graph.
//! The nested workspace excludes the facade's interpreter-bearing dev-dependencies.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use craft_pdf::syntax::{
        DecodeLimits,
        object::{ObjectIdentifier, Stream},
    };
    use craft_pdf::{OpenError, OpenOptions, open, open_slice, open_slice_with_options, open_with_options};
    use std::sync::Arc;

    // zlib-compressed b"q Q\n" repeated 128 times: 17 encoded bytes, 512 decoded bytes.
    // This fixture exercises the pure-Rust Flate path without any compression dependency.
    const FLATE_CONTENT: &[u8] = &[120, 156, 43, 84, 8, 228, 42, 28, 197, 35, 22, 3, 0, 132, 234, 118, 1];

    fn flate_pdf() -> Vec<u8> {
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = vec![0];
        let mut content = format!("<< /Length {} /Filter /FlateDecode >>\nstream\n", FLATE_CONTENT.len()).into_bytes();
        content.extend_from_slice(FLATE_CONTENT);
        content.extend_from_slice(b"\nendstream");
        let objects: [&[u8]; 4] = [
            b"<< /Type /Catalog /Pages 2 0 R >>",
            b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>",
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 120 80] /Contents 4 0 R >>",
            &content,
        ];
        for (index, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            bytes.extend_from_slice(object);
            bytes.extend_from_slice(b"\nendobj\n");
        }
        let xref = bytes.len();
        bytes.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        for offset in offsets.iter().skip(1) {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(format!("trailer\n<< /Root 1 0 R /Size 5 >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
        bytes
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn read_only_opens_and_decodes_flate_without_image_features() {
        let bytes = flate_pdf();
        let pdf = open_slice(&bytes, None).unwrap();
        assert_eq!(pdf.pages().len(), 1);
        let page = &pdf.pages()[0];
        let dimensions = page.media_box();
        assert_eq!((dimensions.width(), dimensions.height()), (120.0, 80.0));
        let expected = b"q Q\n".repeat(128);
        assert_eq!(page.page_stream().unwrap(), expected.as_slice());

        // Also use the shipping incremental path and drain once beyond the expected end so
        // completion is actually observed, rather than inferred from a valid prefix.
        let mut reader = page.content_reader();
        let mut decoded = Vec::new();
        while reader.read_into(&mut decoded, 19) != 0 {
            assert!(decoded.len() <= expected.len());
        }
        assert_eq!(decoded, expected);
        assert!(!reader.truncated());
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn incremental_flate_keeps_its_separate_content_budget() {
        let bytes = flate_pdf();
        let expected = b"q Q\n".repeat(128);
        for limit in [0, 32] {
            let options = OpenOptions { decode_limits: DecodeLimits::default().with_max_decoded_stream(limit), ..Default::default() };
            let pdf = open_slice_with_options(&bytes, None, &options).unwrap();
            let page = &pdf.pages()[0];
            assert_eq!(page.page_stream().unwrap().len(), limit);
            let mut reader = page.content_reader();
            let mut decoded = Vec::new();
            while reader.read_into(&mut decoded, 19) != 0 {
                assert!(decoded.len() <= expected.len());
            }
            assert_eq!(decoded, expected);
            assert!(!reader.truncated(), "the separate streaming budget was not exhausted");
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn document_stream_limits_are_lowered_without_affecting_another_open() {
        let bytes = Arc::new(flate_pdf());
        let limits = DecodeLimits::default().with_max_decoded_stream(32);
        let mut options = OpenOptions { decode_limits: limits, ..Default::default() };
        let low = open_with_options(bytes.clone(), None, &options).unwrap();
        let normal = open(bytes.clone(), None).unwrap();
        // The options value is copied into the document, not consulted later.
        options.decode_limits = limits.with_max_decoded_stream(0);
        let zero = open_slice_with_options(bytes.as_slice(), None, &options).unwrap();
        let expected = b"q Q\n".repeat(128);
        let low_stream = low.xref().get::<Stream<'_>>(ObjectIdentifier::new(4, 0)).unwrap();
        assert!(low_stream.raw_data().len() < limits.max_decoded_stream());
        assert_eq!(low_stream.decoded().unwrap().as_ref(), &expected[..32]);
        assert_eq!(low_stream.decoded_within(7).unwrap().as_ref(), &expected[..7]);
        assert_eq!(low.pages()[0].page_stream().unwrap(), &expected[..32]);
        assert_eq!(normal.pages()[0].page_stream().unwrap(), expected.as_slice());
        assert_eq!(zero.pages()[0].page_stream().unwrap(), b"");
        assert_eq!(low.decode_limits(), limits);
        assert_eq!(normal.decode_limits(), DecodeLimits::default());
        assert_eq!(limits.with_max_decoded_stream(usize::MAX), limits);
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn caller_input_limit_applies_to_borrowed_and_shared_open() {
        let bytes = flate_pdf();
        let limit = bytes.len() - 1;
        let options = OpenOptions { max_input_bytes: limit, ..Default::default() };
        assert!(matches!(open_slice_with_options(&bytes, None, &options), Err(OpenError::InputLimit { limit: actual }) if actual == limit));
        assert!(matches!(open_with_options(Arc::new(bytes), None, &options), Err(OpenError::InputLimit { limit: actual }) if actual == limit));
    }
}
