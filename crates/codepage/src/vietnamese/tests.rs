use std::borrow::Cow;

use super::{
    DecodeError, DecodeMode, DecodeOptions, DetectionResult, EncodingEvidence, EncodingMetadata, LegacyEncoding, VietnameseEncoding, VniDecoder,
    decode, decode_tcvn3, decode_text, decode_vni, decode_windows1258, decode_with_mode, detect_encoding, detect_encoding_from_str,
    detect_legacy_encoding, detect_vietnamese_encoding, font_hint, is_already_unicode_vietnamese, is_legacy_text, is_serif_legacy_font,
    is_vietnamese_font, normalize_cp1258_nfc, normalize_nfc, transcode, transcode_explicit, transcode_summary, transcode_tcvn3, transcode_to_unicode,
    transcode_vietnamese, transcode_vni, transcode_with_summary, validate_and_normalize,
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
    // -------------------------------------------------------------------------
    // WHY: Modern documents already encoded in standard UTF-8 must pass through
    // the pipeline completely untouched with borrowed fast-path guarantee (Cow::Borrowed).
    // DANGER: False-positive transcoding of genuine UTF-8 text corrupts modern data.
    // CONTRACT: Tier 1 - Fast-Path Guard Invariant (is_already_unicode_vietnamese).
    // -------------------------------------------------------------------------
    for unicode in ["HÌNH CHIẾU ĐỨNG", "HÌNH DÁNG", "TÒA NHÀ", "CHÚNG TÔI", "BÁO CÁO"] {
        assert!(is_already_unicode_vietnamese(unicode), "failed on {unicode}");
        let res_vni = transcode_vietnamese(unicode, Some("VNI-Helve"));
        assert_eq!(res_vni, unicode);
        assert!(matches!(res_vni, Cow::Borrowed(_)), "Must satisfy Borrowed fast-path guarantee for {unicode}");

        let res_tcvn = transcode_vietnamese(unicode, Some(".VnTime"));
        assert_eq!(res_tcvn, unicode);
        assert!(matches!(res_tcvn, Cow::Borrowed(_)), "Must satisfy Borrowed fast-path guarantee for {unicode}");

        let res_none = transcode_vietnamese(unicode, None);
        assert_eq!(res_none, unicode);
        assert!(matches!(res_none, Cow::Borrowed(_)), "Must satisfy Borrowed fast-path guarantee for {unicode}");
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
    assert_eq!(transcode_vietnamese("THOÂNG SOÁ KYÕ THUAÄT", Some("Standard")), "THÔNG SỐ KỸ THUẬT");
    assert_eq!(transcode_vietnamese("NHOÂM 3003 DAØY 2MM", Some("simplex.shx")), "NHÔM 3003 DÀY 2MM");
    assert_eq!(transcode_vietnamese("TOÅNG KHOÁI LÖÔÏNG", Some("Arial")), "TỔNG KHỐI LƯỢNG");
    assert_eq!(transcode_vietnamese("QUY CAÙCH TOÅNG THEÅ", None), "QUY CÁCH TỔNG THỂ");
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
fn composed_cp1258_text_is_borrowed() {
    // Why: every precomposed character in the repertoire is already NFC (NFC_QC=Yes), so only
    // an explicit combining mark can change the result. Decomposing a lone `É` just to recompose
    // it would allocate and hand back the same borrow.
    for text in ["\u{00C9}", "\u{1EC7}", "Ti\u{1EBF}ng Vi\u{1EC7}t"] {
        assert!(matches!(normalize_cp1258_nfc(text), Cow::Borrowed(_)), "{text:?} must stay borrowed");
    }
    // A mark that needs reordering still takes the full decompose-order-recompose pipeline.
    assert_eq!(normalize_cp1258_nfc("\u{00C9}\u{0323}"), "\u{1EB8}\u{0301}");
}

#[test]
fn long_mark_runs_are_ordered_without_quadratic_work() {
    // Why: `decode_text` runs this ordering on attacker-controlled text, so a long mark run must
    // stay O(n log n). 50,000 `U+0301 U+0323` pairs are about 1.25 billion shifts for an insertion
    // sort. `#` takes no composition, so every mark survives and its class stays observable.
    let mut text = String::from("#");
    for _ in 0..50_000 {
        text.push('\u{0301}');
        text.push('\u{0323}');
    }
    let normalized = normalize_cp1258_nfc(&text);
    let classes: Vec<u8> = normalized.chars().skip(1).map(super::combining_class).collect();
    assert_eq!(classes.len(), 100_000);
    assert_eq!(classes[0], 220);
    assert_eq!(classes[99_999], 230);
    assert!(classes.windows(2).all(|w| w[0] <= w[1]), "marks must be in canonical order");
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
    // 1. Short strings (< 16 bytes) transcode cleanly when font hint is provided
    let short_hinted = [
        ("SOÁ LIEÄU", "SỐ LIỆU"),
        ("PHOÁ COÅ", "PHỐ CỔ"),
        ("MOÃI NGAØY", "MỖI NGÀY"),
        ("NHOÂM 6063", "NHÔM 6063"),
        ("TOÅNG COÄNG", "TỔNG CỘNG"),
        ("QUY CAÙCH", "QUY CÁCH"),
    ];
    for (input, expected) in short_hinted {
        assert_eq!(transcode_vietnamese(input, Some("VNI-Helve")), expected, "Failed on hinted {input}");
    }

    // 2. Unhinted text must meet the Short-String Invariant (>= 16 bytes) to activate heuristics
    let unhinted_long = [
        ("SOÁ LIEÄU TOÅNG HOÏP", "SỐ LIỆU TỔNG HỌP"),
        ("PHOÁ COÅ HAØ NOÄI", "PHỐ CỔ HÀ NỘI"),
        ("MOÃI NGAØY MOÄT NIEÀM VUI", "MỖI NGÀY MỘT NIỀM VUI"),
        ("NHOÂM 6063 DAØY 2MM", "NHÔM 6063 DÀY 2MM"),
        ("TOÅNG COÄNG KHOÁI LÖÔÏNG", "TỔNG CỘNG KHỐI LƯỢNG"),
        ("QUY CAÙCH TOÅNG THEÅ", "QUY CÁCH TỔNG THỂ"),
    ];
    for (input, expected) in unhinted_long {
        assert_eq!(transcode_vietnamese(input, None), expected, "Failed on unhinted long {input}");
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
    // -------------------------------------------------------------------------
    // WHY: Windows-1258 uses decomposed combining diacritical marks (U+0300..=U+0323).
    // In Unicode NFC, combining marks are only composed if a canonical precomposed character exists.
    // Vietnamese consonants never take tone marks, so a sequence like `k` + acute (0xEC)
    // has no precomposed representation in Latin Extended-A/B and remains `k\u{0301}`.
    // DANGER: If a naive decoder assumes all combining marks disappear into precomposed NFC,
    // it will panic or silently discard unpairable diacritics.
    // CONTRACT: Tier 1 - Contract 1 (Canonical Normalization Limits).
    // -------------------------------------------------------------------------
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

    let raw_tcvn3: Vec<u8> = "B¶n vÏ chi tiÕt mÆt b»ng".chars().map(|c| c as u8).collect();
    let res_heuristic = detect_encoding(&raw_tcvn3, None);
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
    // -------------------------------------------------------------------------
    // WHY: Standard TCVN3 (.VnTime) only had 8-bit space for lowercase accented vowels.
    // To write all-caps titles (e.g. "ỦY BAN NHÂN DÂN"), editors switched fonts to .VnTimeH.
    // In raw bytes, the identical byte 0xD2 is lowercase 'ề' under .VnTime, but renders as uppercase 'Ề' under .VnTimeH.
    // Decoding .VnTimeH MUST map all high bytes to their uppercase Unicode counterparts.
    // DANGER: If TCVN3H is decoded using the standard TCVN3 table, headlines and drawing sheet
    // titles in CAD/WordCraft will display as lowercase or mixed-case gibberish.
    // CONTRACT: Tier 1 - Font Hint Overrides (TCVN3H).
    // -------------------------------------------------------------------------
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
    // -------------------------------------------------------------------------
    // WHY: 1995-2005 Vietnamese office documents heavily borrowed the 0x80..=0x9F range from Windows-1252
    // for smart typography (curly quotes “ ”, em-dash —, and ellipsis …).
    // Strict decoders reject these bytes as unmapped, destroying legal quotes in decrees and contracts.
    // Practical mode must preserve these typographic marks while maintaining strict Vietnamese vowel mappings.
    // DANGER: Rejecting or corrupting CP1252 bytes renders real-world contracts and AutoCAD title blocks unreadable.
    // CONTRACT: Tier 1 - Operating Modes (Strict vs. Practical).
    // -------------------------------------------------------------------------
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
fn test_vni_streaming_chunk_partition_invariance() {
    // -------------------------------------------------------------------------
    // WHY: Streaming decoders must guarantee invariance across chunk boundaries:
    // D(B) = D(B_1 || B_2 || ... || B_n), exhaustively tested over all two-way split points
    // and selected multi-chunk partitions of the curated corpus.
    // DANGER: If chunk splits desynchronize lookahead buffers (e.g. pending base vowels,
    // multi-byte CapsLock sequences like `I` + 0xCF + `T`), the output diverges based on network
    // packet or file buffer size, corrupting text non-deterministically.
    // CONTRACT: Tier 1 - Contract 2 (Streaming State Machine Chunk Invariance).
    // -------------------------------------------------------------------------
    let corpora: &[&[u8]] = &[
        b"CONG HOA XA HOI CHU NGHIA VI\xCFT NAM",
        b"VI\xCFT NAM DONG NAI BA RIA VUNG TAU",
        b"Ban ve so 1a\xf9 va chi tiet gia cong k\xf8 thong so ky thuat",
        b"TO\xc5NG KHO\xc1I L\xd6\xd4\xcfNG",
        b"NHO\xc2M 3003 DA\xd8Y 2MM",
        b"QUY CA\xd9CH",
        b"VI\xCF",   // Truncated IET at EOF -> VỊ
        b"VI\xCF A", // IET followed by non-T -> VỊ A
        b"a\xf8 a\xf9 a\xfa a\xf5 a\xef",
    ];

    for &payload in corpora {
        let expected = decode_vni(payload, DecodeMode::Practical).expect("one-shot decode");

        // Test 1: Byte-by-byte chunking (worst-case fragmentation: chunk size = 1)
        let mut dec_1 = VniDecoder::new(DecodeMode::Practical);
        let mut out_1 = String::new();
        for &byte in payload {
            dec_1.decode_chunk(&[byte], &mut out_1).expect("1-byte chunk");
        }
        dec_1.finish(&mut out_1).expect("finish 1-byte");
        assert_eq!(out_1, expected.as_ref(), "1-byte chunking must match one-shot for {:?}", payload);

        // Test 2: Every single split point partition (B_1 = [0..split], B_2 = [split..len])
        for split in 0..=payload.len() {
            let mut dec_split = VniDecoder::new(DecodeMode::Practical);
            let mut out_split = String::new();
            dec_split.decode_chunk(&payload[..split], &mut out_split).expect("chunk 1");
            dec_split.decode_chunk(&payload[split..], &mut out_split).expect("chunk 2");
            dec_split.finish(&mut out_split).expect("finish split");
            assert_eq!(out_split, expected.as_ref(), "Split at {split} must match one-shot for {:?}", payload);
        }

        // Test 3: Multiple chunk sizes (2, 3, 5, 7 bytes)
        for chunk_size in [2, 3, 5, 7] {
            let mut dec_chunk = VniDecoder::new(DecodeMode::Practical);
            let mut out_chunk = String::new();
            for window in payload.chunks(chunk_size) {
                dec_chunk.decode_chunk(window, &mut out_chunk).expect("window chunk");
            }
            dec_chunk.finish(&mut out_chunk).expect("finish window");
            assert_eq!(out_chunk, expected.as_ref(), "{chunk_size}-byte window must match one-shot for {:?}", payload);
        }
    }
}

#[test]
fn test_vni_streaming_chunk_boundaries() {
    // -------------------------------------------------------------------------
    // WHY: VNI is a composite encoding where diacritics follow base vowels (e.g., 'a' + 0xF9 -> 'á').
    // When reading from streams or I/O chunk buffers, the base vowel may arrive at the end of Chunk 1
    // and the accent mark at the start of Chunk 2.
    // DANGER: If the decoder is stateless or flushes immediately at chunk end, 'a' and 0xF9 are
    // split into separate orphan characters ("a´" instead of "á").
    // CONTRACT: Tier 1 - Contract 2 (Streaming State Machine across Chunk Boundaries).
    // -------------------------------------------------------------------------
    // Simulate buffer split between base 'a' (chunk 1) and diacritic mark 0xF9 (chunk 2)
    let mut decoder = VniDecoder::new(DecodeMode::Practical);
    let mut out = String::new();

    decoder.decode_chunk(b"Ban ve so 1a", &mut out).expect("Chunk 1");
    // 'a' is held as pending_base
    assert_eq!(out, "Ban ve so 1");

    decoder.decode_chunk(b"\xf9 va chi tiet", &mut out).expect("Chunk 2");
    decoder.finish(&mut out).expect("finish");

    // 'a' and '\xf9' seamlessly composed into 'á'
    assert_eq!(out, "Ban ve so 1á va chi tiet");
}

#[test]
fn test_vni_streaming_error_stream_offset() {
    // -------------------------------------------------------------------------
    // WHY: Error diagnostics (e.g. UnmappedByte, MalformedSequence) must report
    // the absolute byte offset within the entire multi-chunk stream, not a chunk-local index.
    // DANGER: In multi-gigabyte or streaming CAD/DWG imports, reporting a local chunk index
    // misdirects operators to the wrong file location when diagnosing corruption.
    // CONTRACT: Tier 1 - Contract 4 (Explicit Error Model & Absolute Stream Offsets).
    // -------------------------------------------------------------------------
    // Test 1: Error in Chunk 2 at local index 3, following 50 bytes in Chunk 1
    let mut dec = VniDecoder::new(DecodeMode::Strict);
    let mut out = String::new();
    let chunk1 = [b'A'; 50];
    dec.decode_chunk(&chunk1, &mut out).expect("chunk 1 must succeed");
    assert_eq!(dec.stream_offset(), 50);

    // Chunk 2 contains invalid byte 0x81 at local index 3 (absolute offset: 50 + 3 = 53)
    let chunk2 = [b'B', b'C', b'D', 0x81, b'E'];
    let err = dec.decode_chunk(&chunk2, &mut out).expect_err("chunk 2 must fail on unmapped byte");
    assert_eq!(err, DecodeError::UnmappedByte { byte: 0x81, offset: 53, encoding: LegacyEncoding::Vni });

    // Test 2: Error across 3 chunks: Chunk 1 (40 bytes), Chunk 2 (30 bytes), Chunk 3 with error at index 5 (offset: 75)
    let mut dec3 = VniDecoder::new(DecodeMode::Strict);
    let mut out3 = String::new();
    dec3.decode_chunk(&[b'X'; 40], &mut out3).expect("chunk 1");
    dec3.decode_chunk(&[b'Y'; 30], &mut out3).expect("chunk 2");
    assert_eq!(dec3.stream_offset(), 70);

    let chunk3 = [b'Z', b'Z', b'Z', b'Z', b'Z', 0xC0]; // 0xC0 is unmapped in Strict
    let err3 = dec3.decode_chunk(&chunk3, &mut out3).expect_err("chunk 3 must fail");
    assert_eq!(err3, DecodeError::UnmappedByte { byte: 0xC0, offset: 75, encoding: LegacyEncoding::Vni });

    // Test 3: Error on pending base byte deferred from Chunk 1 and failing in Chunk 2
    let mut dec_base = VniDecoder::new(DecodeMode::Strict);
    let mut out_base = String::new();
    // 40 bytes ASCII, plus 0xCA (unmapped in Strict if not combined) at index 40
    let mut chunk_base1 = vec![b'X'; 40];
    chunk_base1.push(0xCA);
    dec_base.decode_chunk(&chunk_base1, &mut out_base).expect("chunk 1 defers base byte");
    assert_eq!(dec_base.stream_offset(), 41);

    // Chunk 2 starts with space ' ' which does not combine with 0xCA
    let chunk_base2 = b" ";
    let err_base = dec_base.decode_chunk(chunk_base2, &mut out_base).expect_err("unmapped standalone 0xCA");
    assert_eq!(err_base, DecodeError::UnmappedByte { byte: 0xCA, offset: 40, encoding: LegacyEncoding::Vni });
}

#[test]
fn test_vni_streaming_eof_flush_pending_states() {
    // -------------------------------------------------------------------------
    // WHY: State machines holding lookahead buffers (pending_base_byte, pending_iet)
    // must deterministically resolve remaining state when finish() is invoked at EOF.
    // DANGER: Silent truncation drops characters at end-of-file; over-strict rejection
    // breaks valid unaccented terminal vowels.
    // CONTRACT: Tier 1 - Contract 2 (Pending-at-EOF Policy & Composite State Machine).
    // -------------------------------------------------------------------------
    // 1. Pending base vowel at EOF:
    // Practical mode: flushes base character
    let mut dec = VniDecoder::new(DecodeMode::Practical);
    let mut out = String::new();
    dec.decode_chunk(b"Ban ve so 1a", &mut out).expect("decode chunk");
    assert_eq!(out, "Ban ve so 1"); // 'a' is held in pending_base_byte
    dec.finish(&mut out).expect("finish must succeed");
    assert_eq!(out, "Ban ve so 1a");

    // Strict mode: flushes valid base character
    let mut dec_s = VniDecoder::new(DecodeMode::Strict);
    let mut out_s = String::new();
    dec_s.decode_chunk(b"Ban ve so 1a", &mut out_s).expect("decode chunk");
    dec_s.finish(&mut out_s).expect("finish must succeed");
    assert_eq!(out_s, "Ban ve so 1a");

    // Strict mode with unmapped candidate byte at EOF: returns UnmappedByte with absolute offset
    let mut dec_s_err = VniDecoder::new(DecodeMode::Strict);
    let mut out_s_err = String::new();
    let mut payload = vec![b'B'; 20];
    payload.push(0xCA); // 0xCA is unmapped in Strict
    dec_s_err.decode_chunk(&payload, &mut out_s_err).expect("chunk defers base candidate");
    let err = dec_s_err.finish(&mut out_s_err).expect_err("finish must fail on unmapped byte");
    assert_eq!(err, DecodeError::UnmappedByte { byte: 0xCA, offset: 20, encoding: LegacyEncoding::Vni });

    // 2. Pending IET lookahead at EOF:
    // Practical mode: flushes fallback Ị
    let mut dec_iet_p = VniDecoder::new(DecodeMode::Practical);
    let mut out_iet_p = String::new();
    dec_iet_p.decode_chunk(b"VI\xCF", &mut out_iet_p).expect("chunk 1");
    dec_iet_p.finish(&mut out_iet_p).expect("practical finish flushes Ị");
    assert_eq!(out_iet_p, "VỊ");

    // Strict mode: truncated lookahead at EOF returns MalformedSequence with exact offset
    let mut dec_iet_s = VniDecoder::new(DecodeMode::Strict);
    let mut out_iet_s = String::new();
    let mut payload_iet = vec![b'X'; 30];
    payload_iet.extend_from_slice(b"VI\xCF");
    dec_iet_s.decode_chunk(&payload_iet, &mut out_iet_s).expect("chunk 1");
    let err_iet = dec_iet_s.finish(&mut out_iet_s).expect_err("strict finish must fail on truncated lookahead");
    assert_eq!(err_iet, DecodeError::MalformedSequence { offset: 31, encoding: LegacyEncoding::Vni });
}

#[test]
fn test_vni_independent_golden_fixtures() {
    // -------------------------------------------------------------------------
    // WHY: Partition invariance tests must be grounded in independent golden oracles
    // (exact Unicode literals verified against VNI specifications), not circular assertions
    // against decode_vni().
    // DANGER: Circular test oracles hide systemic bugs shared between one-shot and streaming decoders.
    // CONTRACT: Tier 1 - Contract 2 & Verification Fixtures.
    // -------------------------------------------------------------------------
    let golden_cases: &[(&[u8], &str)] = &[
        // All-caps CapsLock sequences (I + 0xCF + T -> IỆT)
        (b"VI\xCFT", "VIỆT"),
        (b"CONG HOA XA HOI CHU NGHIA VI\xCFT NAM", "CONG HOA XA HOI CHU NGHIA VIỆT NAM"),
        // Mixed-case permutations: base determines vowel pair case; follower consonant retains its case
        (b"vi\xeft", "việt"),
        (b"VI\xcft", "VIỆt"),
        (b"vi\xefT", "việT"),
        // Non-T follower flushes Ị / ị
        (b"VI\xCF A", "VỊ A"),
        (b"vi\xef a", "vị a"),
        (b"VI\xCF 123", "VỊ 123"),
        // Full VNI technical drawing label
        (b"TO\xc5NG KHO\xc1I L\xd6\xd4\xcfNG", "TỔNG KHỐI LƯỢNG"),
        (b"QUY CA\xd9CH", "QUY CÁCH"),
    ];

    for &(bytes, expected_unicode) in golden_cases {
        // 1. One-shot verification
        let one_shot = decode_vni(bytes, DecodeMode::Practical).expect("one-shot decode");
        assert_eq!(one_shot.as_ref(), expected_unicode, "One-shot failed on {:?}", bytes);

        // 2. Worst-case fragmentation: 1-byte chunks
        let mut dec_1 = VniDecoder::new(DecodeMode::Practical);
        let mut out_1 = String::new();
        for &b in bytes {
            dec_1.decode_chunk(&[b], &mut out_1).expect("1-byte chunk");
        }
        dec_1.finish(&mut out_1).expect("finish 1-byte");
        assert_eq!(out_1, expected_unicode, "1-byte partition failed on {:?}", bytes);

        // 3. Exhaustive two-way split points
        for split in 0..=bytes.len() {
            let mut dec_split = VniDecoder::new(DecodeMode::Practical);
            let mut out_split = String::new();
            dec_split.decode_chunk(&bytes[..split], &mut out_split).expect("chunk 1");
            dec_split.decode_chunk(&bytes[split..], &mut out_split).expect("chunk 2");
            dec_split.finish(&mut out_split).expect("finish split");
            assert_eq!(out_split, expected_unicode, "Split at {split} failed on {:?}", bytes);
        }
    }
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
    streaming_decoder.finish(&mut streamed_out).expect("finish");
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
fn test_short_string_heuristic_inhibition_invariant() {
    // -------------------------------------------------------------------------
    // WHY: Short labels are common in CAD (.dwg) and spreadsheet (.xlsx) documents (e.g. "TẦNG 1", "CỘT C1", "KHO").
    // Running statistical n-gram heuristics on such short byte slices produces high rates of false-positive detections.
    // Short strings MUST inhibit heuristics and rely strictly on explicit metadata or FontHints (.VnTime, VNI-).
    // DANGER: Without this length gate, short French, German, or English words get erroneously transcoded.
    // CONTRACT: Tier 1 - Contract 3 (Short-String Invariant < 16 bytes).
    // -------------------------------------------------------------------------
    // "TẦNG 1" in TCVN3 (6 bytes < 16 bytes)
    let short_cad_label = b"T\xC7NG 1";

    // 1. Unhinted: Heuristics are inhibited on strings < 16 bytes -> Must be Unknown
    let no_hint_result = detect_encoding(short_cad_label, None);
    assert_eq!(no_hint_result, DetectionResult::Unknown, "Short strings (< 16 bytes) must inhibit heuristics and return Unknown");

    // decode_text on unhinted non-UTF8 short string returns InvalidUtf8
    let decode_unhinted = decode_text(short_cad_label, &DecodeOptions::default());
    assert!(decode_unhinted.is_err(), "Unhinted short non-UTF8 text must not transcode speculatively");

    // 1b. Exact 15-byte vs 16-byte boundary with 2 valid VNI composites
    let vni_15 = b"a\xF8 a\xF9          ";
    assert_eq!(vni_15.len(), 15);
    assert_eq!(detect_encoding(vni_15, None), DetectionResult::Unknown);
    assert!(decode_text(vni_15, &DecodeOptions::default()).is_err());

    let vni_16 = b"a\xF8 a\xF9           ";
    assert_eq!(vni_16.len(), 16);
    assert_eq!(
        detect_encoding(vni_16, None),
        DetectionResult::HighConfidence { encoding: LegacyEncoding::Vni, evidence: EncodingEvidence::Heuristic }
    );
    assert_eq!(decode_text(vni_16, &DecodeOptions::default()).as_deref(), Ok("à á           "));

    // 2. With FontHint: Overcomes length threshold via Evidence Hierarchy
    let meta = EncodingMetadata { font_name: Some(".VnTime"), codepage_hint: None };
    let with_hint_result = detect_encoding(short_cad_label, Some(&meta));
    assert_eq!(
        with_hint_result,
        DetectionResult::HighConfidence { encoding: LegacyEncoding::Tcvn3, evidence: EncodingEvidence::FontHint },
        "Short strings with valid FontHint must decode successfully"
    );

    let decode_hinted = decode_text(short_cad_label, &DecodeOptions { metadata: Some(&meta), ..Default::default() });
    assert_eq!(decode_hinted.as_deref(), Ok("TẦNG 1"));

    // 3. Short valid UTF-8 string (< 16 bytes) is preserved as UTF-8
    let short_utf8 = "TẦNG 1".as_bytes();
    let decode_utf8 = decode_text(short_utf8, &DecodeOptions::default());
    assert_eq!(decode_utf8.as_deref(), Ok("TẦNG 1"));
    assert!(matches!(decode_utf8, Ok(Cow::Borrowed(_))));
}

#[test]
fn test_font_hint_case_insensitive_and_stems() {
    let dummy_payload = b"Sample text payload for font hint matching";

    // TCVN3 standard font name variants (mixed case and dotless stems)
    for font in &[".VnTime", ".vntime", ".VNTIME", "vntime"] {
        let meta = EncodingMetadata { font_name: Some(font), codepage_hint: None };
        assert_eq!(
            detect_encoding(dummy_payload, Some(&meta)),
            DetectionResult::HighConfidence { encoding: LegacyEncoding::Tcvn3, evidence: EncodingEvidence::FontHint },
            "Font hint '{font}' must match Tcvn3"
        );
    }

    // TCVN3H all-caps display font variants
    for font in &[".VnTimeH", ".vntimeh", ".VNTIMEH"] {
        let meta = EncodingMetadata { font_name: Some(font), codepage_hint: None };
        assert_eq!(
            detect_encoding(dummy_payload, Some(&meta)),
            DetectionResult::HighConfidence { encoding: LegacyEncoding::Tcvn3H, evidence: EncodingEvidence::FontHint },
            "Font hint '{font}' must match Tcvn3H"
        );
    }

    // VNI-Windows font variants
    for font in &["VNI-Times", "vni-times", "VNI-Aptima", "vni-telex"] {
        let meta = EncodingMetadata { font_name: Some(font), codepage_hint: None };
        assert_eq!(
            detect_encoding(dummy_payload, Some(&meta)),
            DetectionResult::HighConfidence { encoding: LegacyEncoding::Vni, evidence: EncodingEvidence::FontHint },
            "Font hint '{font}' must match Vni"
        );
    }
}

#[test]
fn test_coderabbit_review_invariants() {
    // -------------------------------------------------------------------------
    // WHY: In VNI-Windows, typing all-caps "VIỆT" with CapsLock produces `VI\xCF` + `T`.
    // The byte 0xCF represents both standalone 'Ị' and the decomposed mark for 'Ê' + dot-below.
    // Without multi-chunk lookahead, splitting this sequence across chunk boundaries (e.g., [b"VI\xCF", b"T"])
    // causes naive decoders to prematurely emit "VỊ" and then "T", corrupting "VIỆT" into "VỊT".
    // DANGER: All-caps company and national names ("CỘNG HÒA XÃ HỘI CHỦ NGHĨA VIỆT NAM")
    // become corrupted into "VỊT NAM" in archival documents.
    // CONTRACT: Tier 1 - Contract 2 (Multi-chunk Lookahead Pair).
    // -------------------------------------------------------------------------
    // 1. VNI multi-chunk streaming for "VIỆT"
    // Case A: 2 chunks split right at the mark byte: [b"VI\xCF", b"T"]
    let mut dec_2 = VniDecoder::new(DecodeMode::Practical);
    let mut out_2 = String::new();
    dec_2.decode_chunk(b"VI\xCF", &mut out_2).expect("chunk 1");
    dec_2.decode_chunk(b"T", &mut out_2).expect("chunk 2");
    dec_2.finish(&mut out_2).expect("finish");
    assert_eq!(out_2, "VIỆT", "2-chunk split at mark byte must yield VIỆT");

    // Case B: 3 chunks split across base, mark, and follower: [b"VI", b"\xCF", b"T"]
    let mut dec_3 = VniDecoder::new(DecodeMode::Practical);
    let mut out_3 = String::new();
    dec_3.decode_chunk(b"VI", &mut out_3).expect("chunk 1");
    dec_3.decode_chunk(b"\xCF", &mut out_3).expect("chunk 2");
    dec_3.decode_chunk(b"T", &mut out_3).expect("chunk 3");
    dec_3.finish(&mut out_3).expect("finish");
    assert_eq!(out_3, "VIỆT", "3-chunk split across base, mark, and follower must yield VIỆT");

    // Case C: EOF without 'T' flushes 'Ị'
    let mut dec_eof = VniDecoder::new(DecodeMode::Practical);
    let mut out_eof = String::new();
    dec_eof.decode_chunk(b"VI\xCF", &mut out_eof).expect("chunk 1");
    dec_eof.finish(&mut out_eof).expect("finish");
    assert_eq!(out_eof, "VỊ", "EOF without trailing T must flush standalone Ị");

    // 2. Genuine UTF-8 text with .VnTime font hint must never be corrupted
    let cat_utf8 = "cát vàng".as_bytes();
    let meta_tcvn = EncodingMetadata { font_name: Some(".VnTime"), codepage_hint: None };
    let cat_res =
        decode_text(cat_utf8, &DecodeOptions { metadata: Some(&meta_tcvn), ..Default::default() }).expect("genuine UTF-8 must decode cleanly");
    assert_eq!(cat_res, "cát vàng", "Valid UTF-8 must never be corrupted by font hint");

    // 3. Explicit legacy Vietnamese font hint overrides container codepage metadata (FontHint > Codepage),
    // because DXF/RTF files using .VnTime or VNI-Times default their header codepage to 1252/1258.
    let conflicting_meta = EncodingMetadata { font_name: Some(".VnTime"), codepage_hint: Some(1258) };
    let det_res = detect_encoding(b"Any payload >= 16 bytes", Some(&conflicting_meta));
    assert_eq!(
        det_res,
        DetectionResult::HighConfidence { encoding: LegacyEncoding::Tcvn3, evidence: EncodingEvidence::FontHint },
        "Explicit legacy font hint (.VnTime) must override container codepage 1258"
    );

    // Verify decode_text uses TCVN3 when .VnTime font is paired with codepage 1258
    let tcvn3_non_utf8 = b"B\xB6n v\xCF"; // "Bản vẽ" in TCVN3
    let options = DecodeOptions { force_encoding: None, metadata: Some(&conflicting_meta), mode: DecodeMode::Practical, normalize_nfc: true };
    let decoded = decode_text(tcvn3_non_utf8, &options).expect(".VnTime overrides container codepage 1258");
    assert_eq!(decoded.as_ref(), "Bản vẽ");

    // Without a legacy Vietnamese font hint, codepage 1258 metadata selects Windows-1258 and composes NFC
    let cp1258_only = EncodingMetadata { font_name: Some("Arial"), codepage_hint: Some(1258) };
    let cp1258_non_utf8 = [b'T', b'i', 0xEA, 0xEC, b't']; // "Tiê" + sắc (0xEC) + "t" in CP1258
    let cp_opts = DecodeOptions { force_encoding: None, metadata: Some(&cp1258_only), mode: DecodeMode::Practical, normalize_nfc: true };
    let cp_decoded = decode_text(&cp1258_non_utf8, &cp_opts).expect("CP1258 decodes and composes NFC");
    assert_eq!(cp_decoded.as_ref(), "Tiết");

    // When FontHint (.VnTime) conflicts with strong VNI byte signatures, detect_encoding returns Ambiguous
    let vni_bytes = b"a\xF8 a\xF9 e\xF8 e\xF9 e\xF5 u\xF8"; // >= 16 bytes of pure VNI composites
    let tcvn_font_only = EncodingMetadata { font_name: Some(".VnTime"), codepage_hint: None };
    assert_eq!(
        detect_encoding(vni_bytes, Some(&tcvn_font_only)),
        DetectionResult::Ambiguous(&[LegacyEncoding::Tcvn3, LegacyEncoding::Vni]),
        "FontHint conflicting with byte-stream signatures must return Ambiguous"
    );

    // 4. transcode_explicit bypasses is_already_unicode_vietnamese guard
    // Byte 0xC3 in CP1258 is 'Ă' (U+0102). In Latin-1 it is 'Ã' (U+00C3).
    let latin1_c3 = "\u{00C3}";
    let explicit_res = transcode_explicit(latin1_c3, LegacyEncoding::Windows1258);
    assert_eq!(explicit_res, "Ă", "transcode_explicit must bypass guard and decode 0xC3 to Ă");
}

#[test]
fn test_coderabbit_0f38107_findings() {
    // Finding 1: VNI single-byte 0xCC, 0xCD, 0xEC, 0xED -> Ì, Í, ì, í ("tím")
    assert_eq!(decode_vni(b"t\xEDm", DecodeMode::Practical).expect("Practical VNI tím"), "tím");
    assert_eq!(decode_vni(b"t\xEDm", DecodeMode::Strict).expect("Strict VNI tím"), "tím");
    assert_eq!(decode_vni(b"\xCC\xCD\xEC\xED", DecodeMode::Strict).expect("VNI ÌÍìí"), "ÌÍìí");

    // Finding 5: Valid UTF-8 >= 16 bytes must never trigger VNI/TCVN3 heuristic in detect_encoding
    assert_eq!(
        detect_encoding("aøaøaøaøaøaøaøaø".as_bytes(), None),
        DetectionResult::Unknown,
        "Valid UTF-8 must return Unknown without running heuristic"
    );

    // Finding 4: Scores within 30% margin must yield DetectionResult::Ambiguous
    // 5 VNI composites (score 10) vs 4 TCVN3 composites (score 8) => 20% diff < 30%
    let close_scores = "aø aù eø eù eõ Aµ A¸ A¹ A¶";
    assert_eq!(
        detect_encoding_from_str(close_scores),
        DetectionResult::Ambiguous(&[LegacyEncoding::Vni, LegacyEncoding::Tcvn3]),
        "Candidates differing by < 30% must be rejected as Ambiguous"
    );
}

#[test]
fn test_zero_dep_vietnamese_nfc_composer() {
    // CP1258 combining marks (base + U+0300/U+0301/U+0309/U+0303/U+0323)
    assert_eq!(normalize_nfc("Tie\u{0302}\u{0301}ng Vie\u{0323}\u{0302}t").as_ref(), "Tiếng Việt");
    assert_eq!(normalize_nfc("a\u{0306}\u{0300} o\u{031B}\u{0323} u\u{031B}\u{0303}").as_ref(), "ằ ợ ữ");
    // Already-NFC and pure ASCII return Cow::Borrowed (zero alloc)
    assert!(matches!(normalize_nfc("Tiếng Việt"), Cow::Borrowed(_)));
    assert!(matches!(normalize_nfc("Clean ASCII"), Cow::Borrowed(_)));
    // Non-precomposable sequence (k + acute) stays decomposed and borrowed
    let non_precomposed = "k\u{0301}";
    let res = normalize_nfc(non_precomposed);
    assert!(matches!(res, Cow::Borrowed(_)));
    assert_eq!(res.as_ref(), non_precomposed);
}

#[test]
fn test_minor_edge_cases_and_tcvn3_cad_escapes() {
    // 1. AutoCAD %%c escape must be ignored by byte-path all-caps detection
    assert_eq!(decode(b"%%c B\xB6N", LegacyEncoding::Tcvn3).expect("CAD %%c + BẢN"), "%%c BẢN");

    // 2. CP1252 fallback bytes (0x80..=0x9F, e.g. 0x9C 'œ') must keep original case in all-caps / Tcvn3H
    assert_eq!(decode_tcvn3(b"B\x9CN", false, DecodeMode::Practical).expect("TCVN3 all-caps preserves œ"), "BœN");
    assert_eq!(super::transcode_tcvn3_h("b\u{009C}n").as_ref(), "BœN");
    assert!(matches!(super::transcode_tcvn3_h("BAN VE"), Cow::Borrowed(_)));

    // 3. Invalid-threshold candidate must not tie out a valid candidate
    // "aø aÐ 1234567890" has VNI=2 (valid) and TCVN3=2 (below threshold 4 => invalid)
    assert_eq!(
        detect_encoding_from_str("aø aÐ 1234567890"),
        DetectionResult::HighConfidence { encoding: LegacyEncoding::Vni, evidence: EncodingEvidence::Heuristic }
    );

    // 4. Tcvn3H font hint conflicting with VNI bytes preserves Tcvn3H in Ambiguous
    let vni_bytes = b"a\xF8 a\xF9 e\xF8 e\xF9 e\xF5 u\xF8";
    let tcvn3h_meta = EncodingMetadata { font_name: Some(".VnTimeH"), codepage_hint: None };
    assert_eq!(detect_encoding(vni_bytes, Some(&tcvn3h_meta)), DetectionResult::Ambiguous(&[LegacyEncoding::Tcvn3H, LegacyEncoding::Vni]));
}

#[test]
fn test_tcvn3_latin1_widened_overlap_and_final_coderabbit_cases() {
    // 41 unambiguous TCVN3 markers bypass the Unicode guard even next to ambiguous vowels like Ý.
    assert_eq!(transcode_vietnamese("kh«ng Ýt h¬n", Some(".VnTime")), "không ít hơn");
    assert_eq!(transcode_vietnamese("KH«NG ÝT H¬N", Some(".VnTimeH")), "KHÔNG ÍT HƠN");

    // Token-level phonotactic rules decode false-Unicode TCVN3 words; genuine Unicode stays intact.
    assert_eq!(transcode_vietnamese("Ých", Some(".VnTime")), "ích");
    assert_eq!(transcode_vietnamese("bÕn Ýt", Some(".VnTime")), "bến ít");
    assert_eq!(transcode_vietnamese("gâ Ýt", Some(".VnTime")), "gõ ít");
    assert_eq!(transcode_vietnamese("Ít khi", Some(".VnTime")), "Ít khi");
    assert_eq!(transcode_vietnamese("Ý kiến", Some(".VnTime")), "Ý kiến");

    // An undecodable VNI coincidence (0xA7 invalid in VNI) must not veto a valid `.VnTimeH` hint.
    let meta = EncodingMetadata { font_name: Some(".VnTimeH"), codepage_hint: None };
    let decoded = decode_text(b"Y\xD5N \xA7\xB6O 12345678", &DecodeOptions { metadata: Some(&meta), ..Default::default() })
        .expect("undecodable VNI coincidence must not override .VnTimeH");
    assert_eq!(decoded, "YẾN ĐẢO 12345678");

    // AutoCAD `%%%` literal percent + `%%d` keeps all-caps inference; unknown `%%x` leaves `x` visible.
    assert_eq!(decode_tcvn3(b"A%%%%%d \xD2", false, DecodeMode::Practical).unwrap(), "A%%%%%d Ề");
    assert_eq!(decode_tcvn3(b"A%%x \xD2", false, DecodeMode::Practical).unwrap(), "A%%x ề");

    // Non-Latin-1 input under `.VnTimeH` stays borrowed.
    let emoji = "Plan 🙂";
    let out = super::transcode_tcvn3_h(emoji);
    assert!(matches!(out, Cow::Borrowed(_)));
    assert_eq!(out, emoji);
}

#[test]
fn test_regression_invariants_for_vietnamese_transcoding_and_nfc() {
    // 1. Why: Standalone copyright (`©` = 0xA9) and middle-dot (`·` = 0xB7) symbols
    // in genuine Unicode Vietnamese text must NOT bypass the Unicode guard under
    // `.VnTime` or `.VnTimeH`, whereas markers embedded in an alphabetic token
    // (`"kh«ng Ýt h¬n"`) still bypass it and decode as TCVN3.
    assert_eq!(transcode_vietnamese("TÒA ÁN © 2026", Some(".VnTimeH")), "TÒA ÁN © 2026");
    assert_eq!(transcode_vietnamese("TÒA NHÀ · A", Some(".VnTime")), "TÒA NHÀ · A");
    assert_eq!(transcode_vietnamese("TÒA NHÀ · A", Some(".VnTimeH")), "TÒA NHÀ · A");
    assert_eq!(transcode_vietnamese("kh«ng Ýt h¬n", Some(".VnTime")), "không ít hơn");

    // 2. Why: A mixed-case token that already reads as valid Unicode Vietnamese
    // (`"tÔi"`) must not be rewritten by the mid-word capital rule (`"tễi"`), and a
    // checked syllable carrying the allowed acute tone (`"Ép tôn"`, `É` = 0xC9 before
    // `-p`) must stay intact, while genuinely impossible Unicode words (`"Ých"`) and
    // mid-word TCVN3 capitals (`"bÕn"`) still decode.
    assert_eq!(transcode_vietnamese("tÔi", Some(".VnTime")), "tÔi");
    assert_eq!(transcode_vietnamese("Ép tôn", Some(".VnTime")), "Ép tôn");
    assert_eq!(transcode_vietnamese("Ých", Some(".VnTime")), "ích");
    assert_eq!(transcode_vietnamese("bÕn", Some(".VnTime")), "bến");

    // 3. Why: Windows-1258 NFC normalization must compose every canonical pair in the
    // CP1258 repertoire, including `ü + U+0301` -> `ǘ` (`0xFC 0xEC`), while
    // `decode_windows1258` stays a raw, unnormalized decoding primitive.
    assert_eq!(decode_windows1258(&[0xFC, 0xEC]).expect("strict CP1258 ü + acute"), "ü\u{0301}");
    assert_eq!(
        decode_text(&[0xFC, 0xEC], &DecodeOptions { force_encoding: Some(LegacyEncoding::Windows1258), normalize_nfc: true, ..Default::default() })
            .expect("CP1258 ǘ"),
        "ǘ"
    );

    // 4. Why: Canonical Combining Class order is part of NFC: dot below `U+0323`
    // (ccc 220) must precede top accents (ccc 230), so `0xEC 0xF2 0x41`
    // (`"\u{0301}\u{0323}A"`) normalizes to `"\u{0323}\u{0301}A"`.
    assert_eq!(normalize_cp1258_nfc("\u{0301}\u{0323}A"), "\u{0323}\u{0301}A");
    assert_eq!(
        decode_text(
            &[0xEC, 0xF2, 0x41],
            &DecodeOptions { force_encoding: Some(LegacyEncoding::Windows1258), normalize_nfc: true, ..Default::default() }
        )
        .expect("CP1258 reordered marks"),
        "\u{0323}\u{0301}A"
    );

    // 5. Why: An explicit legacy font hint (`.VnTimeH`) overrides container codepage
    // metadata (1258), and an undecodable VNI coincidence (`0xA7` is invalid in VNI)
    // must never veto the hint.
    let meta = EncodingMetadata { font_name: Some(".VnTimeH"), codepage_hint: Some(1258) };
    let decoded = decode_text(b"Y\xD5N \xA7\xB6O 12345678", &DecodeOptions { metadata: Some(&meta), ..Default::default() })
        .expect("undecodable VNI coincidence must not override .VnTimeH");
    assert_eq!(decoded, "YẾN ĐẢO 12345678");

    // 6. Why: AutoCAD `%%%` literal percent escapes and valid inline codes (`%%d`)
    // keep all-caps inference, while CP1252 fallback characters (`0x9C` -> `œ`)
    // preserve their original case under `.VnTimeH`.
    assert_eq!(decode_tcvn3(b"A%%%%%d \xD2", false, DecodeMode::Practical).unwrap(), "A%%%%%d Ề");
    assert_eq!(super::transcode_tcvn3_h("b\u{009C}n").as_ref(), "BœN");
    // 7. Why (UAX #15 starter retention & precomposed decomposition in CP1258):
    // - Bytes `43 EC F2` (`C` + `\u{0301}` + `\u{0323}`) reorder to `C` + `\u{0323}` +
    //   `\u{0301}`; the normalizer must keep `C` as starter across the uncomposed
    //   dot-below so `\u{0301}` still composes with `C` into `Ć` + `\u{0323}`.
    // - Bytes `C9 F2` (`É` + `\u{0323}`) must canonically decompose `É` into
    //   `E + \u{0301}` before CCC ordering, so it equals `45 F2 EC` -> `Ẹ\u{0301}`.
    assert_eq!(
        decode_text(
            &[0x43, 0xEC, 0xF2],
            &DecodeOptions { force_encoding: Some(LegacyEncoding::Windows1258), normalize_nfc: true, ..Default::default() }
        )
        .expect("CP1258 Ć + dot below"),
        "Ć\u{0323}"
    );
    assert_eq!(
        decode_text(&[0xC9, 0xF2], &DecodeOptions { force_encoding: Some(LegacyEncoding::Windows1258), normalize_nfc: true, ..Default::default() })
            .expect("CP1258 Ẹ + acute"),
        "Ẹ\u{0301}"
    );

    // 8. Why (all-high-byte TCVN3 words & consonantal frames):
    // - `"®·"` (`0xAE 0xB7`) is TCVN3 `"đã"` and contains no ASCII letter, yet the
    //   whole line must still decode to `"Á CHÂU ĐÃ CÓ"` under `.VnTimeH`.
    // - `"nÒn"` (`6E D2 6E`) has `Ò` flanked by ASCII consonants, the TCVN3
    //   consonant-vowel-consonant frame, so it decodes to `"nền"`; `"tÔi"` instead
    //   opens a valid Unicode vowel cluster (`Ôi`) and must stay untouched.
    assert_eq!(transcode_vietnamese("Á CHÂU ®· Cã", Some(".VnTimeH")), "Á CHÂU ĐÃ CÓ");
    assert_eq!(transcode_vietnamese("nÒn", Some(".VnTime")), "nền");
    assert_eq!(transcode_vietnamese("tÔi", Some(".VnTime")), "tÔi");

    // 9. Why: every Windows-1258 entry point must agree on NFC. `transcode_explicit`
    // receives the Latin-1 widened form of bytes `C9 F2` and `43 EC F2`, so it must run
    // the same UAX #15 pipeline as `decode_text` and return `"Ẹ\u{0301}"` and
    // `"Ć\u{0323}"` instead of a partially composed `É\u{0323}` / `C\u{0323}\u{0301}`.
    assert_eq!(transcode_explicit("\u{00C9}\u{00F2}", LegacyEncoding::Windows1258), "Ẹ\u{0301}");
    assert_eq!(transcode_explicit("\u{0043}\u{00EC}\u{00F2}", LegacyEncoding::Windows1258), "Ć\u{0323}");
    assert_eq!(
        transcode_explicit("\u{00C9}\u{00F2}", LegacyEncoding::Windows1258),
        decode_text(&[0xC9, 0xF2], &DecodeOptions { force_encoding: Some(LegacyEncoding::Windows1258), normalize_nfc: true, ..Default::default() })
            .expect("CP1258 Ẹ + acute")
    );
    assert_eq!(
        transcode_explicit("\u{0043}\u{00EC}\u{00F2}", LegacyEncoding::Windows1258),
        decode_text(
            &[0x43, 0xEC, 0xF2],
            &DecodeOptions { force_encoding: Some(LegacyEncoding::Windows1258), normalize_nfc: true, ..Default::default() }
        )
        .expect("CP1258 Ć + dot below")
    );
}

#[test]
fn cp1258_nfc_matches_unicode_oracle_exhaustively() {
    use unicode_normalization::UnicodeNormalization;

    // Why: Windows-1258 is a closed 256-byte repertoire with only 5 combining marks, so
    // its canonical normalization can be proven rather than sampled: every byte alone,
    // every byte with one mark, and every byte with every mark pair must agree with the
    // independent Unicode NFC oracle. This is what catches boundary cases such as
    // `43 EC F2`, `C9 F2` and `C4 F2` before review does.
    const MARKS: [u8; 5] = [0xCC, 0xEC, 0xD2, 0xDE, 0xF2];
    let mut checked = 0_u32;

    for first in 0_u8..=0xFF {
        let single = [first];
        if let Ok(decoded) = decode_windows1258(&single) {
            let ours = normalize_cp1258_nfc(&decoded);
            let oracle: String = decoded.nfc().collect();
            assert_eq!(ours.as_ref(), oracle, "CP1258 bytes {single:02X?} decoded {decoded:?}");
            checked += 1;
        }

        for second in MARKS {
            let pair = [first, second];
            let Ok(decoded) = decode_windows1258(&pair) else { continue };
            let ours = normalize_cp1258_nfc(&decoded);
            let oracle: String = decoded.nfc().collect();
            assert_eq!(ours.as_ref(), oracle, "CP1258 bytes {pair:02X?} decoded {decoded:?}");
            checked += 1;

            for third in MARKS {
                let triple = [first, second, third];
                let decoded = decode_windows1258(&triple).expect("two valid CP1258 marks");
                let ours = normalize_cp1258_nfc(&decoded);
                let oracle: String = decoded.nfc().collect();
                assert_eq!(ours.as_ref(), oracle, "CP1258 bytes {triple:02X?} decoded {decoded:?}");
                checked += 1;
            }
        }
    }

    // 247 defined bytes x (1 single + 5 single-mark + 25 mark-pair) sequences.
    assert!(checked > 7_600, "expected the full CP1258 x mark x mark domain, checked {checked}");
}

#[test]
fn test_linear_normalization_and_scanning_invariants() {
    // 1. Why: A hostile run of 50,000 interleaved combining marks (`U+0301` ccc=230,
    // `U+0323` ccc=220) after `'C'` must normalize in strictly O(n) time using the
    // 4-bucket streaming UAX #15 pipeline without any quadratic sorting or shifts.
    let mut hostile = String::with_capacity(1 + 25_000 * 4);
    hostile.push('C');
    for _ in 0..25_000 {
        hostile.push_str("\u{0301}\u{0323}");
    }
    let normalized = normalize_cp1258_nfc(&hostile);
    let mut expected = String::with_capacity(hostile.len());
    expected.push('Ć');
    for _ in 0..25_000 {
        expected.push('\u{0323}');
    }
    for _ in 0..24_999 {
        expected.push('\u{0301}');
    }
    assert_eq!(normalized.as_ref(), expected);

    // 2. Why: Lazy starter decomposition (only when followed by combining marks)
    // must preserve exact UAX #15 equivalence on mixed precomposed/decomposed runs.
    assert_eq!(normalize_cp1258_nfc("Éa\u{0301}C\u{0301}\u{0323}Ç\u{0301}Ä\u{0301}").as_ref(), "ÉáĆ\u{0323}ḈÄ\u{0301}");

    // 3. Why: Long multi-token TCVN3 input must scan in a single linear pass under
    // `.VnTime` while still decoding both Region 1 (`kh«ng Ýt h¬n`) and Region 2 (`bÕn`).
    let mut long_tcvn3 = "kh«ng Ýt h¬n ".repeat(3_000);
    long_tcvn3.push_str("bÕn");
    let mut expected_tcvn3 = "không ít hơn ".repeat(3_000);
    expected_tcvn3.push_str("bến");
    assert_eq!(transcode_vietnamese(&long_tcvn3, Some(".VnTime")), expected_tcvn3);

    // 4. Why: Symmetric VNI font-hint guard — `0xCD` ('Í' in VNI) is unmapped in TCVN3,
    // so an undecodable TCVN3 inversion coincidence (`a\xD0 a\xD0`) must not veto `VNI-Times`.
    let vni_payload = b"t\xCDn a\xD0 a\xD0 12345678";
    let vni_meta = EncodingMetadata { font_name: Some("VNI-Times"), codepage_hint: None };
    assert_eq!(
        detect_encoding(vni_payload, Some(&vni_meta)),
        DetectionResult::HighConfidence { encoding: LegacyEncoding::Vni, evidence: EncodingEvidence::FontHint }
    );

    // 5. Why: Non-Latin-1 pass-through on `transcode_vni` must return `Cow::Borrowed`
    // without allocating a temporary `Vec<u8>`.
    let emoji = "Plan 🙂";
    let out = transcode_vni(emoji);
    assert!(matches!(out, Cow::Borrowed(_)));
    assert_eq!(out, emoji);
}
