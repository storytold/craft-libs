use std::borrow::Cow;

use super::{
    DecodeError, LegacyEncoding, VietnameseEncoding, decode, detect_legacy_encoding, detect_vietnamese_encoding, font_hint,
    is_already_unicode_vietnamese, is_legacy_text, is_serif_legacy_font, is_vietnamese_font, normalize_nfc, transcode, transcode_explicit,
    transcode_summary, transcode_tcvn3, transcode_to_unicode, transcode_vietnamese, transcode_vni, transcode_with_summary, validate_and_normalize,
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
