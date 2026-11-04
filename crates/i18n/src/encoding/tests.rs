use std::borrow::Cow;

use super::{
    DecodeError, DecodeMode, DecodeOptions, DetectionResult, EncodingEvidence, EncodingMetadata, LegacyEncoding, VietnameseEncoding, VniDecoder,
    decode, decode_tcvn3, decode_text, decode_vni, decode_with_mode, detect_encoding, detect_encoding_from_str, detect_legacy_encoding,
    detect_vietnamese_encoding, font_hint, is_already_unicode_vietnamese, is_legacy_text, is_serif_legacy_font, is_vietnamese_font, normalize_nfc,
    transcode, transcode_explicit, transcode_summary, transcode_tcvn3, transcode_to_unicode, transcode_vietnamese, transcode_vni,
    transcode_with_summary, validate_and_normalize,
};

#[test]
fn test_transcoded_output_is_nfc_composed() {
    let samples = [
        ("B¶n vÏ chi tiÕt", ".VnTime"),
        ("MÆT B»NG KÕT CÊU", ".VnTime"),
        ("TI£U CHUÈN", ".VnTime"),
        ("B¶n vÏ", ".VnArialH"),
        ("BAÛN VEÕ CHI TIEÁT", "VNI-Times"),
        ("Maët baèng keát caáu", "VNI-Helve"),
        ("Tieâu chuaån kyõ thuaät", "VNI-Aptima"),
        ("NHOÂM 3003 DAØY 2MM", "VNI-Times"),
    ];

    for (input, hint) in samples {
        let out = transcode_vietnamese(input, Some(hint));
        assert_ne!(out, input, "Must be transcoded");
        for c in out.chars() {
            assert!(
                !matches!(c, '\u{0300}'..='\u{036F}' | '\u{1DC0}'..='\u{1DFF}'),
                "Transcoded output for {:?} contains combining mark {:?}",
                input,
                c
            );
        }
    }
}

#[test]
fn test_vni_nhom_day() {
    assert_eq!(transcode_vni("NHOÂM 3003 DAØY 2MM"), "NHÔM 3003 DÀY 2MM");
}

#[test]
fn test_vni_tong_khoi_luong() {
    assert_eq!(transcode_vni("TOÅNG KHOÁI LÖÔÏNG"), "TỔNG KHỐI LƯỢNG");
}

#[test]
fn test_vni_quy_cach() {
    assert_eq!(transcode_vni("QUY CAÙCH"), "QUY CÁCH");
}

#[test]
fn test_vni_dang_hinh_dung() {
    assert_eq!(transcode_vni("DAÙNG"), "DÁNG");
    assert_eq!(transcode_vni("ÑÖÙNG"), "ĐỨNG");
    assert_eq!(transcode_vni("HIØNH ÑÖÙNG"), "HÌNH ĐỨNG");
}

#[test]
fn test_vni_composites() {
    assert_eq!(transcode_vni("OÂ"), "Ô");
    assert_eq!(transcode_vni("AÙ"), "Á");
    assert_eq!(transcode_vni("OÅ"), "Ổ");
    assert_eq!(transcode_vni("OÃ"), "Ỗ");
    assert_eq!(transcode_vni("EÂ"), "Ê");
    assert_eq!(transcode_vni("ÖÔÏ"), "ƯỢ");
    assert_eq!(transcode_vni("oâ"), "ô");
    assert_eq!(transcode_vni("aù"), "á");
    assert_eq!(transcode_vni("öôï"), "ượ");
}

#[test]
fn test_already_unicode_never_transcoded() {
    for unicode in ["HÌNH CHIẾU ĐỨNG", "HÌNH DÁNG", "TÒA NHÀ", "CHÚNG TÔI", "BÁO CÁO"] {
        assert!(is_already_unicode_vietnamese(unicode), "failed on {unicode}");
        assert_eq!(transcode_vietnamese(unicode, Some("VNI-Helve")), unicode);
        assert_eq!(transcode_vietnamese(unicode, Some(".VnTime")), unicode);
        assert_eq!(transcode_vietnamese(unicode, None), unicode);
    }
}

#[test]
fn test_false_positives_never_transcoded() {
    for word in ["Hòa Bình", "Chúng tôi", "Báo cáo công tác quý II", "Bê tông mác 200, cát vàng", "Piñón", "Café"] {
        assert_eq!(detect_vietnamese_encoding(word), None, "false positive on {word}");
        assert_eq!(transcode_vietnamese(word, None), word);
        assert_eq!(transcode_vietnamese(word, Some("Standard")), word);
    }
}

#[test]
fn test_diameter_symbols_never_transcoded() {
    for sym in ["Ø20", "OØ20", "oØ20", "2OØ25"] {
        assert_eq!(detect_vietnamese_encoding(sym), None, "diameter false positive on {sym}");
        assert_eq!(transcode_vietnamese(sym, None), sym);
        assert_eq!(transcode_vietnamese(sym, Some("Standard")), sym);
    }
}

#[test]
fn test_symbols_and_uppercase_european_never_transcoded() {
    let cases = ["© 2026 ACME ®", "1½ x ¾", "§ 5 ¶ 2", "NIÑO", "ÉCOLE", "100° ± 0.5°", "AREA: 50 m² ± 2 m²"];
    for s in cases {
        assert_eq!(detect_vietnamese_encoding(s), None, "failed detection on {s}");
        assert_eq!(transcode_vietnamese(s, None), s, "failed unhinted transcoding on {s}");
        assert_eq!(transcode_vietnamese(s, Some("Standard")), s, "failed Standard font transcoding on {s}");
    }
}

#[test]
fn test_tcvn3h_ascii_and_legacy() {
    assert_eq!(transcode_vietnamese("ban ve chi tiet", Some(".VnTimeH")), "BAN VE CHI TIET");
    assert_eq!(transcode_vietnamese("B¶n vÏ chi tiÕt", Some(".VnTimeH")), "BẢN VẼ CHI TIẾT");
    match transcode_vietnamese("BAN VE", Some(".VnTimeH")) {
        Cow::Borrowed(b) => assert_eq!(b, "BAN VE"),
        Cow::Owned(_) => panic!("expected borrowed string for already-uppercase ASCII"),
    }
}

#[test]
fn test_tcvn3_all_caps() {
    assert_eq!(transcode_tcvn3("MÆT B»NG KÕT CÊU"), "MẶT BẰNG KẾT CẤU");
    assert_eq!(transcode_tcvn3("B¶N VÏ CHI TIÕT"), "BẢN VẼ CHI TIẾT");
}

#[test]
fn test_hinh_dang_unicode() {
    assert_eq!(transcode_vietnamese("HÌNH DÁNG", None), "HÌNH DÁNG");
}

#[test]
fn test_generic_font_transcodes_when_vietnamese_detected() {
    assert_eq!(transcode_vietnamese("THOÂNG SOÁ", Some("Standard")), "THÔNG SỐ");
    assert_eq!(transcode_vietnamese("NHOÂM 3003 DAØY 2MM", Some("simplex.shx")), "NHÔM 3003 DÀY 2MM");
    assert_eq!(transcode_vietnamese("TOÅNG KHOÁI LÖÔÏNG", Some("Arial")), "TỔNG KHỐI LƯỢNG");
    assert_eq!(transcode_vietnamese("QUY CAÙCH", None), "QUY CÁCH");
}

#[test]
fn test_tcvn3_ban_ve_chi_tiet() {
    assert_eq!(transcode_tcvn3("B¶n vÏ chi tiÕt"), "Bản vẽ chi tiết");
}

#[test]
fn test_tcvn3_mat_bang_ket_cau() {
    assert_eq!(transcode_tcvn3("MÆt b»ng kÕt cÊu"), "Mặt bằng kết cấu");
}

#[test]
fn test_is_vietnamese_font() {
    assert_eq!(is_vietnamese_font(".VnTime"), Some(LegacyEncoding::Tcvn3));
    assert_eq!(is_vietnamese_font(".VnArialH"), Some(LegacyEncoding::Tcvn3H));
    assert_eq!(is_vietnamese_font("vnsimli.shx"), Some(LegacyEncoding::Tcvn3));
    assert_eq!(is_vietnamese_font("VNI-Helve"), Some(LegacyEncoding::Vni));
    assert_eq!(is_vietnamese_font("VNI-Times.ttf"), Some(LegacyEncoding::Vni));
    assert_eq!(is_vietnamese_font("arial.ttf"), None);
    assert_eq!(is_vietnamese_font("vnext.ttf"), None);
    assert_eq!(is_vietnamese_font("vnorth"), None);
    assert_eq!(is_vietnamese_font(".VnTech"), Some(LegacyEncoding::Tcvn3));
}

#[test]
fn test_transcode_vietnamese_with_font_hint() {
    assert_eq!(transcode_vietnamese("NHOÂM 3003 DAØY 2MM", Some("VNI-Helve")), "NHÔM 3003 DÀY 2MM");
    assert_eq!(transcode_vietnamese("B¶n vÏ chi tiÕt", Some(".VnTime")), "Bản vẽ chi tiết");
    let ascii = "SIMPLE TEXT 123";
    match transcode_vietnamese(ascii, Some(".VnTime")) {
        Cow::Borrowed(b) => assert_eq!(b, ascii),
        Cow::Owned(_) => panic!("expected borrowed string for ascii"),
    }
}

#[test]
fn test_transcode_to_unicode_result() {
    let r = transcode_to_unicode("B¶n vÏ chi tiÕt", Some(".VnTime"));
    assert_eq!(r.text, "Bản vẽ chi tiết");
    assert_eq!(r.encoding, Some(LegacyEncoding::Tcvn3));
    assert!(r.was_transcoded);

    let ascii_r = transcode_to_unicode("hello world", Some(".VnTime"));
    assert_eq!(ascii_r.text, "hello world");
    assert_eq!(ascii_r.encoding, Some(LegacyEncoding::Tcvn3));
    assert!(!ascii_r.was_transcoded);
}

#[test]
fn test_normalize_nfc() {
    // NFD decomposed 'e' + combining acute (U+0065, U+0301) -> NFC precomposed 'é' (U+00E9)
    let decomposed = "e\u{0301}";
    let normalized = normalize_nfc(decomposed);
    assert_eq!(normalized, "é");
    assert!(matches!(normalized, Cow::Owned(_)));

    // Already NFC stays borrowed
    let precomposed = "é";
    let stays = normalize_nfc(precomposed);
    assert_eq!(stays, "é");
    assert!(matches!(stays, Cow::Borrowed(_)));
}

#[test]
fn test_adversarial_unicode_latin1_standalone_never_corrupted() {
    let words = ["TÒA NHÀ", "TÒA ÁN", "TÔI ĐI", "CÔ GÁI", "BÁO CÁO", "CHÚ Ý", "LÝ DO", "ĐỒNG HỒ", "GIA ĐÌNH", "TÓM TẮT"];
    for w in words {
        assert_eq!(transcode_vietnamese(w, Some("VNI-Helve")), w, "Failed on {w}");
        assert_eq!(transcode_vietnamese(w, None), w, "Failed on {w}");
    }
}

#[test]
fn test_adversarial_unicode_diphthongs_never_corrupted() {
    let words = ["DỰ TOÁN", "THANH TOÁN", "HOÀN CÔNG", "BẢN VẼ HOÀN CÔNG", "AN TOÀN LAO ĐỘNG", "ÁO KHOÁC", "THOÁT HIỂM", "HOÀ BÌNH", "TRÌ HOÃN"];
    for w in words {
        assert_eq!(transcode_vietnamese(w, Some("VNI-Helve")), w, "Failed on {w}");
        assert_eq!(transcode_vietnamese(w, None), w, "Failed on {w}");
    }
}

#[test]
fn test_adversarial_genuine_vni_still_transcodes() {
    let tests = [
        ("SOÁ LIEÄU", "SỐ LIỆU"),
        ("PHOÁ COÅ", "PHỐ CỔ"),
        ("MOÃI NGAØY", "MỖI NGÀY"),
        ("NHOÂM 6063", "NHÔM 6063"),
        ("TOÅNG COÄNG", "TỔNG CỘNG"),
        ("QUY CAÙCH", "QUY CÁCH"),
    ];
    for (input, expected) in tests {
        assert_eq!(transcode_vietnamese(input, Some("VNI-Helve")), expected, "Failed on {input}");
        assert_eq!(transcode_vietnamese(input, None), expected, "Failed on {input}");
    }
}

#[test]
fn test_adversarial_multilingual_european_languages_never_transcoded_without_hint() {
    let french = ["FAÇADE", "BÉTON", "où", "maïs", "côté", "hôtel", "être", "bâtiment", "aéronef"];
    let spanish = ["CIMENTACIÓN", "AÑO", "DISEÑO", "Uña", "señal", "niño", "SEÑOR", "plano"];
    let german = ["GRÖSSE", "ÜBERGANG", "MÜLLER", "Straße", "Fläche", "Höhe"];
    let portuguese = ["SÃO", "CONEXÃO", "CÔNCAVO", "ORÇAMENTO", "seção", "construção"];
    let other_european = [
        "ăsta", "România", "clădire", "mój", "błąd", "część", "rysunek", "výkres", "čára", "půdorys", "méret", "terv", "épület", "kâğıt", "sağlam",
        "MÆRSK", "Jökull",
    ];

    let all_cases = french.iter().chain(spanish.iter()).chain(german.iter()).chain(portuguese.iter()).chain(other_european.iter());

    for &text in all_cases {
        assert_eq!(detect_vietnamese_encoding(text), None, "False positive detection on European text: {text}");
        assert_eq!(transcode_vietnamese(text, None), text, "Failed to preserve unhinted European text: {text}");
        assert_eq!(transcode_vietnamese(text, Some("Standard")), text, "Failed to preserve Standard font text: {text}");
        assert_eq!(transcode_vietnamese(text, Some("Arial")), text, "Failed to preserve Arial font text: {text}");
    }
}

#[test]
fn test_display_name_and_facade_helpers() {
    assert_eq!(LegacyEncoding::Tcvn3.display_name(), "TCVN3 (ABC)");
    assert_eq!(LegacyEncoding::Tcvn3H.display_name(), "TCVN3 All-Caps");
    assert_eq!(LegacyEncoding::Vni.display_name(), "VNI-Windows");

    assert!(is_legacy_text("B¶n vÏ chi tiÕt", Some(".VnTime")));
    assert!(is_legacy_text("NHOÂM 3003", Some("VNI-Helve")));
    assert!(!is_legacy_text("hello world", None));

    assert_eq!(detect_legacy_encoding("B¶n vÏ", Some(".VnTime")), Some(LegacyEncoding::Tcvn3));
    assert_eq!(detect_legacy_encoding("NHOÂM", Some("VNI-Helve")), Some(LegacyEncoding::Vni));

    let (text, enc) = transcode_with_summary("B¶n vÏ chi tiÕt", Some(".VnTime"));
    assert_eq!(text, "Bản vẽ chi tiết");
    assert_eq!(enc, Some(LegacyEncoding::Tcvn3));

    // CADCraft exact compatibility checks:
    assert_eq!(transcode("B¶n vÏ chi tiÕt", Some(".VnTime")), "Bản vẽ chi tiết");
    assert_eq!(font_hint(&["Standard", ".VnTime"]), Some(".VnTime"));
    assert_eq!(font_hint(&["Standard", "Arial"]), None);
    assert_eq!(transcode_summary(Some(".VnTime")), "TCVN3");
    assert_eq!(transcode_summary(Some("VNI-Helve")), "VNI Windows");
    assert_eq!(transcode_summary(None), "legacy 8-bit");
    let _: VietnameseEncoding = LegacyEncoding::Tcvn3;

    assert!(is_serif_legacy_font(".VnTime"));
    assert!(is_serif_legacy_font("VNI-Times"));
    assert!(!is_serif_legacy_font("VNI-Helve"));
    assert!(!is_serif_legacy_font(".VnArial"));
}

#[test]
fn test_transcode_explicit_and_validate_normalize() {
    let explicit_tcvn = transcode_explicit("B¶n vÏ chi tiÕt", LegacyEncoding::Tcvn3);
    assert_eq!(explicit_tcvn, "Bản vẽ chi tiết");

    let explicit_vni = transcode_explicit("NHOÂM 3003", LegacyEncoding::Vni);
    assert_eq!(explicit_vni, "NHÔM 3003");

    let clean = validate_and_normalize("Bản vẽ kỹ thuật");
    assert_eq!(clean, Ok(Cow::Borrowed("Bản vẽ kỹ thuật")));

    let legacy = validate_and_normalize("B¶n vÏ chi tiÕt");
    assert_eq!(legacy, Err("TCVN3 (ABC)"));

    let legacy_vni = validate_and_normalize("NHOÂM 3003 DAØY 2MM");
    assert_eq!(legacy_vni, Err("VNI-Windows"));
}

#[test]
fn test_windows1258_decode_and_transcode_explicit() {
    assert_eq!(LegacyEncoding::Windows1258.display_name(), "Windows-1258");

    // Pure byte decode
    let bytes = b"Ti\xEA\xECng Vi\xEA\xF2t 100.000\xFE";
    let pure_decoded = decode(bytes, LegacyEncoding::Windows1258);
    assert!(pure_decoded.is_ok());
    if let Ok(text) = pure_decoded {
        assert_eq!(text.as_ref(), "Tiê\u{0301}ng Viê\u{0323}t 100.000\u{20AB}");
    }

    // Explicit transcode with NFC normalization
    let latin1_repr = "Ti\u{00EA}\u{00EC}ng Vi\u{00EA}\u{00F2}t 100.000\u{00FE}";
    let transcoded = transcode_explicit(latin1_repr, LegacyEncoding::Windows1258);
    assert_eq!(transcoded, "Tiếng Việt 100.000₫");

    // ASCII fast path in decode
    let ascii_bytes = b"Standard ASCII Text 123";
    let ascii_decoded = decode(ascii_bytes, LegacyEncoding::Windows1258);
    assert!(matches!(ascii_decoded, Ok(Cow::Borrowed(_))));

    // Error formatting
    let err = DecodeError::UnmappedByte { byte: 0x81, offset: 5, encoding: LegacyEncoding::Windows1258 };
    assert_eq!(format!("{err}"), "unmapped byte 0x81 encountered at byte offset 5 for encoding Windows1258");
}

#[test]
fn test_detection_result_and_evidence_hierarchy() {
    // 1. Font name hint (advisory)
    let meta_tcvn = EncodingMetadata { font_name: Some(".VnTime"), codepage_hint: None };
    let res = detect_encoding(b"Unknown text", Some(&meta_tcvn));
    assert_eq!(res, DetectionResult::HighConfidence { encoding: LegacyEncoding::Tcvn3, evidence: EncodingEvidence::FontHint });
    assert_eq!(res.encoding(), Some(LegacyEncoding::Tcvn3));
    assert_eq!(res.evidence(), Some(EncodingEvidence::FontHint));

    let meta_vni = EncodingMetadata { font_name: Some("VNI-Times"), codepage_hint: None };
    let res_vni = detect_encoding(b"Unknown text", Some(&meta_vni));
    assert_eq!(res_vni, DetectionResult::HighConfidence { encoding: LegacyEncoding::Vni, evidence: EncodingEvidence::FontHint });

    // 2. Codepage container metadata (Windows-1258)
    let meta_cp1258 = EncodingMetadata { font_name: None, codepage_hint: Some(1258) };
    let res_cp = detect_encoding(b"Plain text", Some(&meta_cp1258));
    assert_eq!(res_cp, DetectionResult::HighConfidence { encoding: LegacyEncoding::Windows1258, evidence: EncodingEvidence::Metadata });

    // 3. Content heuristic on legacy text
    assert_eq!(
        detect_encoding_from_str("B¶n vÏ chi tiÕt"),
        DetectionResult::HighConfidence { encoding: LegacyEncoding::Tcvn3, evidence: EncodingEvidence::Heuristic }
    );

    let res_heuristic = detect_encoding("B¶n vÏ chi tiÕt".as_bytes(), None);
    assert_eq!(res_heuristic, DetectionResult::HighConfidence { encoding: LegacyEncoding::Tcvn3, evidence: EncodingEvidence::Heuristic });

    // 4. Unicode safety: genuine Unicode text never identified as legacy
    let res_unicode = detect_encoding("Bản vẽ chi tiết".as_bytes(), None);
    assert_eq!(res_unicode, DetectionResult::Unknown);
    assert_eq!(res_unicode.encoding(), None);

    // 5. Pure ASCII is Unknown (no legacy mutation)
    let res_ascii = detect_encoding(b"Clean ASCII Text 123", None);
    assert_eq!(res_ascii, DetectionResult::Unknown);
}

#[test]
fn test_vni_capslock() {
    let res1 = decode(b"VI\xcfT NAM", LegacyEncoding::Vni).expect("VNI CapsLock with standalone Ï");
    assert_eq!(res1.as_ref(), "VIỆT NAM");

    let res2 = decode(b"VI\xca\xcfT NAM", LegacyEncoding::Vni).expect("VNI CapsLock with ÊÏ");
    assert_eq!(res2.as_ref(), "VIỆT NAM");

    let to_hop = decode(b"TO\xc5NG", LegacyEncoding::Vni).expect("VNI CapsLock O mark");
    assert_eq!(to_hop.as_ref(), "TỔNG");
}

#[test]
fn test_tcvn3h_uppercase_font() {
    // In .VnTime (lower), 0xD2 is 'ề'
    let lower_res = decode_tcvn3(b"\xd2", false, DecodeMode::Practical).expect("TCVN3 lower byte");
    assert_eq!(lower_res.as_ref(), "ề");

    // In .VnTimeH (upper), 0xD2 is 'Ề'
    let upper_res = decode_tcvn3(b"\xd2", true, DecodeMode::Practical).expect("TCVN3 upper byte");
    assert_eq!(upper_res.as_ref(), "Ề");

    // Via facade decode with LegacyEncoding::Tcvn3H
    let facade_res = decode(b"\xd2", LegacyEncoding::Tcvn3H).expect("TCVN3H decode");
    assert_eq!(facade_res.as_ref(), "Ề");
}

#[test]
fn test_cp1252_punctuation_preservation() {
    // b"\x93\x91" are left double quote “ and left single quote ‘
    let punct_vni = decode(b"\x93\x91", LegacyEncoding::Vni).expect("CP1252 quotes in VNI");
    assert_eq!(punct_vni.as_ref(), "“‘");

    let punct_tcvn = decode(b"\x93\x91", LegacyEncoding::Tcvn3).expect("CP1252 quotes in TCVN3");
    assert_eq!(punct_tcvn.as_ref(), "“‘");

    // Real sentence: “trích dẫn” in TCVN3
    // “ (0x93), t, r, í (0xDD), c, h, ' ', đ (0xAE), ẫ (0xC9), n, ” (0x94)
    let sentence = b"\x93tr\xddch d\xc9n\x94";
    let transcoded = decode(sentence, LegacyEncoding::Tcvn3).expect("TCVN3 with quotes");
    assert_eq!(transcoded.as_ref(), "“trích dẫn”");

    // Strict mode rejects CP1252 punctuation
    let strict_res = decode_with_mode(b"\x93", LegacyEncoding::Tcvn3, DecodeMode::Strict);
    assert_eq!(strict_res, Err(DecodeError::UnmappedByte { byte: 0x93, offset: 0, encoding: LegacyEncoding::Tcvn3 }));
}

#[test]
fn test_vni_orphan_diacritic_fallback() {
    // Standalone acute: b"f\xf9" -> "f´"
    let orphan = decode_vni(b"f\xf9", DecodeMode::Practical).expect("VNI orphan acute mark");
    assert_eq!(orphan.as_ref(), "f\u{00B4}");

    // Standalone grave: b"123 \xf8" -> "123 `"
    let grave = decode(b"123 \xf8", LegacyEncoding::Vni).expect("VNI orphan grave mark");
    assert_eq!(grave.as_ref(), "123 `");

    // Strict mode returns UnmappedByte for orphan mark
    let strict_orphan = decode_with_mode(b"f\xf9", LegacyEncoding::Vni, DecodeMode::Strict);
    assert_eq!(strict_orphan, Err(DecodeError::UnmappedByte { byte: 0xF9, offset: 1, encoding: LegacyEncoding::Vni }));
}

#[test]
fn test_vni_streaming_chunk_boundaries() {
    // Simulate buffer split between base 'a' (chunk 1) and diacritic mark 0xF9 (chunk 2)
    let mut decoder = VniDecoder::new(DecodeMode::Practical);
    let mut out = String::new();

    decoder.decode_chunk(b"Ban ve so 1a", &mut out).expect("Chunk 1");
    // 'a' is held as pending_base
    assert_eq!(out, "Ban ve so 1");

    decoder.decode_chunk(b"\xf9 va chi tiet", &mut out).expect("Chunk 2");
    decoder.finish(&mut out);

    // 'a' and '\xf9' seamlessly composed into 'á'
    assert_eq!(out, "Ban ve so 1á va chi tiet");
}

#[test]
fn test_fixture_clean_utf8_payload() {
    let clean_bytes = include_bytes!("fixtures/clean_utf8_payload.txt");
    let clean_res = decode_text(clean_bytes, &DecodeOptions::default()).expect("Clean UTF-8 decode");
    assert!(matches!(clean_res, Cow::Borrowed(_)), "Clean UTF-8 text must take fast path and remain Borrowed with zero allocation");
    assert!(clean_res.contains("Cộng hòa Xã hội Chủ nghĩa Việt Nam"));
    assert!(clean_res.contains("Độc lập - Tự do - Hạnh phúc"));
    assert!(clean_res.contains("tiêu chuẩn TCVN"));
    assert_eq!(normalize_nfc(clean_res.as_ref()), clean_res.as_ref());
}

#[test]
fn test_fixture_tcvn3_cad_dwg() {
    let tcvn3_bytes = include_bytes!("fixtures/tcvn3_cad_dwg.txt");

    // Practical mode: preserves CP1252 quotes “...” and decodes standard TCVN3 body
    let tcvn3_practical =
        decode_text(tcvn3_bytes, &DecodeOptions { force_encoding: Some(LegacyEncoding::Tcvn3), mode: DecodeMode::Practical, ..Default::default() })
            .expect("Practical TCVN3 CAD DWG decoding");

    assert!(tcvn3_practical.contains("“Tiêu chuẩn thiết kế kết cấu bê tông cốt thép”"));
    assert!(tcvn3_practical.contains("Ghi chú: Kiểm tra kích thước trước khi gia công."));
    assert!(tcvn3_practical.contains("Cao độ: +3.500m; Vật liệu: Thép CT3."));

    for c in tcvn3_practical.chars() {
        assert!(!matches!(c, '\u{0300}'..='\u{036F}'), "TCVN3 output must not contain combining diacritics: {c:?}");
    }

    // Line 1 in .VnTimeH (all-caps CAD font entity mapping)
    let tcvn3_lines: Vec<&[u8]> = tcvn3_bytes.split(|&b| b == b'\n').collect();
    let header_line = tcvn3_lines[0].strip_suffix(b"\r").unwrap_or(tcvn3_lines[0]);
    let header =
        decode_text(header_line, &DecodeOptions { force_encoding: Some(LegacyEncoding::Tcvn3H), mode: DecodeMode::Practical, ..Default::default() })
            .expect("Decode TCVN3H header");
    assert_eq!(header.as_ref(), "BẢN VẼ CHI TIẾT CÔNG TRÌNH");

    // Strict mode: rejects CP1252 left quote 0x93 with exact byte offset
    let expected_offset_93 = tcvn3_bytes.iter().position(|&b| b == 0x93).unwrap_or(27);
    let tcvn3_strict_err =
        decode_text(tcvn3_bytes, &DecodeOptions { force_encoding: Some(LegacyEncoding::Tcvn3), mode: DecodeMode::Strict, ..Default::default() });
    assert_eq!(tcvn3_strict_err, Err(DecodeError::UnmappedByte { byte: 0x93, offset: expected_offset_93, encoding: LegacyEncoding::Tcvn3 }));
}

#[test]
fn test_fixture_vni_contract_1998() {
    let vni_bytes = include_bytes!("fixtures/vni_contract_1998.txt");

    // Practical mode: decodes CapsLock diacritics and gracefully converts orphan mark to standalone acute
    let vni_practical =
        decode_text(vni_bytes, &DecodeOptions { force_encoding: Some(LegacyEncoding::Vni), mode: DecodeMode::Practical, ..Default::default() })
            .expect("Practical VNI contract decoding");

    assert!(vni_practical.contains("CỘNG HÒA XÃ HỘI CHỦ NGHĨA VIỆT NAM"));
    assert!(vni_practical.contains("ĐỘC LẬP - TỰ DO - HẠNH PHÚC"));
    assert!(vni_practical.contains("HỢP ĐỒNG KINH TẾ SỐ 198/HĐKT"));
    assert!(vni_practical.contains("Điều 1: Giá cả và phương thức thanh toán"));
    assert!(vni_practical.contains("Số tiền: 500.000.000 f´"));

    for c in vni_practical.chars() {
        assert!(!matches!(c, '\u{0300}'..='\u{036F}'), "VNI output must not contain combining diacritics: {c:?}");
    }

    // Strict mode: rejects orphan diacritic byte 0xF9 with exact offset
    let expected_offset_f9 = vni_bytes.iter().rposition(|&b| b == 0xF9).unwrap_or(179);
    let vni_strict_err =
        decode_text(vni_bytes, &DecodeOptions { force_encoding: Some(LegacyEncoding::Vni), mode: DecodeMode::Strict, ..Default::default() });
    assert_eq!(vni_strict_err, Err(DecodeError::UnmappedByte { byte: 0xF9, offset: expected_offset_f9, encoding: LegacyEncoding::Vni }));

    // Streaming state machine: stream vni_bytes in small 7-byte chunks
    let mut streaming_decoder = VniDecoder::new(DecodeMode::Practical);
    let mut streamed_out = String::new();
    for chunk in vni_bytes.chunks(7) {
        streaming_decoder.decode_chunk(chunk, &mut streamed_out).expect("Streaming VNI chunk");
    }
    streaming_decoder.finish(&mut streamed_out);
    assert_eq!(streamed_out, vni_practical.as_ref(), "Streaming output must match whole-file decoding");
}

#[test]
fn test_fixture_cp1258_accounting() {
    let cp1258_bytes = include_bytes!("fixtures/cp1258_accounting.txt");

    // Pure decoding primitive (without implicit NFC normalization)
    let cp1258_pure =
        decode_text(cp1258_bytes, &DecodeOptions { force_encoding: Some(LegacyEncoding::Windows1258), normalize_nfc: false, ..Default::default() })
            .expect("Pure CP1258 decoding");

    assert!(cp1258_pure.contains('₫'));
    assert!(
        cp1258_pure.chars().any(|c| matches!(c, '\u{0300}'..='\u{036F}')),
        "Pure CP1258 decoding must preserve combining diacritics without hidden NFC"
    );

    // Canonical NFC decoding: all combining marks composed into precomposed NFC
    let cp1258_nfc =
        decode_text(cp1258_bytes, &DecodeOptions { force_encoding: Some(LegacyEncoding::Windows1258), normalize_nfc: true, ..Default::default() })
            .expect("NFC CP1258 decoding");

    assert!(cp1258_nfc.contains('₫'));
    assert!(cp1258_nfc.contains("MÃ VT\tTÊN VẬT TƯ\tSỐ LƯỢNG\tĐƠN GIÁ\tTHÀNH TIỀN"));
    assert!(cp1258_nfc.contains("Xi măng PCB40\t100\t85.000 ₫\t8.500.000 ₫"));
    assert!(cp1258_nfc.contains("Thép cuộn phi 6\t500\t18.500 ₫\t9.250.000 ₫"));
    assert!(cp1258_nfc.contains("TỔNG CỘNG:\t\t\t\t17.750.000 ₫"));

    assert!(!cp1258_nfc.chars().any(|c| matches!(c, '\u{0300}'..='\u{036F}')), "NFC CP1258 decoding must not contain any leftover combining marks");

    // Undefined byte rejection: 0x81 is strictly undefined in CP1258.TXT
    let mut invalid_cp = cp1258_bytes.to_vec();
    invalid_cp[5] = 0x81;
    let cp_err = decode_text(&invalid_cp, &DecodeOptions { force_encoding: Some(LegacyEncoding::Windows1258), ..Default::default() });
    assert_eq!(cp_err, Err(DecodeError::UnmappedByte { byte: 0x81, offset: 5, encoding: LegacyEncoding::Windows1258 }));
}

#[test]
fn test_real_world_corpora_fixtures() {
    test_fixture_clean_utf8_payload();
    test_fixture_tcvn3_cad_dwg();
    test_fixture_vni_contract_1998();
    test_fixture_cp1258_accounting();
}

#[test]
fn test_short_string_invariant() {
    // 1. Short legacy string (< 16 bytes) without FontHint disables heuristics -> Unknown
    let short_legacy = b"T\xC7NG 1"; // 6 bytes: legacy TCVN3 for "TẦNG 1"
    let unhinted_res = detect_encoding(short_legacy, None);
    assert_eq!(unhinted_res, DetectionResult::Unknown);

    // decode_text on unhinted non-UTF8 short string returns InvalidUtf8
    let decode_unhinted = decode_text(short_legacy, &DecodeOptions::default());
    assert!(decode_unhinted.is_err());

    // 2. Short legacy string WITH FontHint -> HighConfidence via FontHint
    let meta_tcvn = EncodingMetadata { font_name: Some(".VnTime"), codepage_hint: None };
    let hinted_res = detect_encoding(short_legacy, Some(&meta_tcvn));
    assert_eq!(hinted_res, DetectionResult::HighConfidence { encoding: LegacyEncoding::Tcvn3, evidence: EncodingEvidence::FontHint });

    let decode_hinted = decode_text(short_legacy, &DecodeOptions { metadata: Some(&meta_tcvn), ..Default::default() });
    assert_eq!(decode_hinted.as_deref(), Ok("TẦNG 1"));

    // 3. Short valid UTF-8 string (< 16 bytes) is preserved as UTF-8
    let short_utf8 = "TẦNG 1".as_bytes();
    let decode_utf8 = decode_text(short_utf8, &DecodeOptions::default());
    assert_eq!(decode_utf8.as_deref(), Ok("TẦNG 1"));
    assert!(matches!(decode_utf8, Ok(Cow::Borrowed(_))));
}

#[test]
fn test_font_hint_case_insensitive_normalization() {
    let variants = [
        (".vntime", LegacyEncoding::Tcvn3),
        (".VNTIME", LegacyEncoding::Tcvn3),
        (".VnTime.ttf", LegacyEncoding::Tcvn3),
        (".vntimeh", LegacyEncoding::Tcvn3H),
        (".VNTIMEH.SHX", LegacyEncoding::Tcvn3H),
        ("vni-times", LegacyEncoding::Vni),
        ("VNI-TIMES", LegacyEncoding::Vni),
        ("vni-helve.otf", LegacyEncoding::Vni),
        ("vtime", LegacyEncoding::Vni),
        ("vhelv", LegacyEncoding::Vni),
    ];
    for (font, expected) in variants {
        assert_eq!(is_vietnamese_font(font), Some(expected), "failed for font: {font}");
        let meta = EncodingMetadata { font_name: Some(font), codepage_hint: None };
        let res = detect_encoding(b"dummy text", Some(&meta));
        assert_eq!(
            res,
            DetectionResult::HighConfidence { encoding: expected, evidence: EncodingEvidence::FontHint },
            "failed detection for font: {font}"
        );
    }
}
