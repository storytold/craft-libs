#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::{MAX_DEPTH, MAX_REFERENCE, MAX_XMP_SCAN, decode_references, pdfx_claim, pdfx_output_intents};
use craft_pdf::syntax::Pdf;

/// 1 catalog with `catalog_extra`, 2 pages, 3 page, 4 the information dictionary with `info`,
/// 5 an XMP packet when `xmp` is given.
fn pdf(catalog_extra: &str, info: &str, xmp: Option<&str>) -> Pdf {
    let metadata = if xmp.is_some() { "/Metadata 5 0 R" } else { "" };
    let mut objects = vec![
        format!("<< /Type /Catalog /Pages 2 0 R {metadata} {catalog_extra} >>").into_bytes(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 9 9] >>".to_vec(),
        format!("<< {info} >>").into_bytes(),
    ];
    if let Some(x) = xmp {
        objects
            .push([format!("<< /Type /Metadata /Subtype /XML /Length {} >>\nstream\n", x.len()).as_bytes(), x.as_bytes(), b"\nendstream"].concat());
    }
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R /Info 4 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    Pdf::new(out).unwrap()
}

fn packet(description: &str) -> String {
    format!(
        "<?xpacket begin=\"\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?><x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF \
         xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">{description}</rdf:RDF></x:xmpmeta><?xpacket end=\"w\"?>"
    )
}

const INTENT: &str = "/OutputIntents [<< /Type /OutputIntent /S /GTS_PDFX /OutputConditionIdentifier (FOGRA39) >> << /S /GTS_PDFA1 >>]";

#[test]
fn an_x4_claim_in_xmp_element_form_with_its_output_intent() {
    let xmp = packet(
        "<rdf:Description rdf:about=\"\" xmlns:pdfxid=\"http://www.npes.org/pdfx/ns/id/\">\
         <pdfxid:GTS_PDFXVersion>PDF/X-4</pdfxid:GTS_PDFXVersion></rdf:Description>",
    );
    let c = pdfx_claim(&pdf(INTENT, "", Some(&xmp))).unwrap();
    assert_eq!(c.level, "PDF/X-4");
    assert_eq!(c.output_intents, vec!["FOGRA39".to_owned()]);
    assert!(c.is_recognised() && c.carriers_agree() && c.has_output_intent());
    assert_eq!(c.info_version, None);
}

#[test]
fn the_attribute_form_under_any_prefix_bound_to_the_namespace() {
    let xmp = packet("<rdf:Description rdf:about=\"\" xmlns:q='http://www.npes.org/pdfx/ns/id/' q:GTS_PDFXVersion='PDF/X-5pg'/>");
    assert_eq!(pdfx_claim(&pdf("", "", Some(&xmp))).unwrap().level, "PDF/X-5pg");
}

#[test]
fn another_namespace_with_the_same_property_name_is_not_a_claim() {
    let xmp = packet(
        "<rdf:Description rdf:about=\"\" xmlns:pdfx=\"http://ns.adobe.com/pdfx/1.3/\">\
         <pdfx:GTS_PDFXVersion>PDF/X-4</pdfx:GTS_PDFXVersion></rdf:Description>",
    );
    assert_eq!(pdfx_claim(&pdf(INTENT, "", Some(&xmp))), None);
}

#[test]
fn an_output_intent_alone_is_not_a_claim() {
    let file = pdf(INTENT, "", None);
    assert_eq!(pdfx_claim(&file), None);
    assert_eq!(pdfx_output_intents(&file), vec!["FOGRA39".to_owned()]);
    assert!(pdfx_output_intents(&pdf("", "", None)).is_empty());
}

#[test]
fn the_2001_conformance_key_splits_x1_from_x1a() {
    let both = "/GTS_PDFXVersion (PDF/X-1:2001) /GTS_PDFXConformance (PDF/X-1a:2001)";
    let c = pdfx_claim(&pdf("", both, None)).unwrap();
    assert_eq!(c.level, "PDF/X-1a:2001");
    let x1 = pdfx_claim(&pdf("", "/GTS_PDFXVersion (PDF/X-1:2001)", None)).unwrap();
    assert_eq!(x1.level, "PDF/X-1:2001");
    assert!(!x1.has_output_intent());
}

#[test]
fn a_utf16_info_value_decodes() {
    let c = pdfx_claim(&pdf("", "/GTS_PDFXVersion <FEFF005000440046002F0058002D0033003A0032003000300033>", None));
    assert_eq!(c.unwrap().level, "PDF/X-3:2003");
}

#[test]
fn xmp_wins_and_a_mismatch_is_reported() {
    let xmp = packet("<rdf:Description xmlns:pdfxid=\"http://www.npes.org/pdfx/ns/id/\" pdfxid:GTS_PDFXVersion=\"PDF/X-4\"/>");
    let c = pdfx_claim(&pdf("", "/GTS_PDFXVersion (PDF/X-3:2003)", Some(&xmp))).unwrap();
    assert_eq!(c.level, "PDF/X-4");
    assert_eq!(c.info_version.as_deref(), Some("PDF/X-3:2003"));
    assert!(!c.carriers_agree());
}

#[test]
fn an_unregistered_value_passes_through_verbatim() {
    let c = pdfx_claim(&pdf("", "/GTS_PDFXVersion (PDF/X-6:2027)", None)).unwrap();
    assert_eq!(c.level, "PDF/X-6:2027");
    assert!(!c.is_recognised());
}

#[test]
fn a_prefix_that_ends_another_name_does_not_match() {
    // `xid:` is bound to the namespace, but the property is under `pdfxid:`, bound elsewhere.
    let xmp = packet(
        "<rdf:Description xmlns:xid=\"http://www.npes.org/pdfx/ns/id/\" xmlns:pdfxid=\"urn:other\">\
         <pdfxid:GTS_PDFXVersion>PDF/X-4</pdfxid:GTS_PDFXVersion></rdf:Description>",
    );
    assert_eq!(pdfx_claim(&pdf("", "", Some(&xmp))), None);
}

#[test]
fn a_default_namespace_element_is_read() {
    let xmp = packet("<rdf:Description><GTS_PDFXVersion xmlns=\"http://www.npes.org/pdfx/ns/id/\">PDF/X-6</GTS_PDFXVersion></rdf:Description>");
    assert_eq!(pdfx_claim(&pdf("", "", Some(&xmp))).unwrap().level, "PDF/X-6");
}

#[test]
fn junk_values_are_ignored() {
    let long = format!("/GTS_PDFXVersion ({})", "X".repeat(65));
    assert_eq!(pdfx_claim(&pdf("", &long, None)), None);
    assert_eq!(pdfx_claim(&pdf("", "/GTS_PDFXVersion /PDFX4", None)), None);
}

#[test]
fn the_xmp_scan_stops_at_its_cap() {
    let claim_at = |pad: usize| {
        let xmp = packet(&format!(
            "<rdf:Description xmlns:pdfxid=\"http://www.npes.org/pdfx/ns/id/\"><!--{}-->\
             <pdfxid:GTS_PDFXVersion>PDF/X-4</pdfxid:GTS_PDFXVersion></rdf:Description>",
            " ".repeat(pad)
        ));
        pdfx_claim(&pdf("", "", Some(&xmp)))
    };
    assert!(claim_at(16).is_some());
    assert_eq!(claim_at(MAX_XMP_SCAN), None);
}

#[test]
fn a_truncated_packet_states_nothing_and_does_not_panic() {
    let full = packet(
        "<rdf:Description xmlns:pdfxid=\"http://www.npes.org/pdfx/ns/id/\"><pdfxid:GTS_PDFXVersion>PDF/X-4</pdfxid:GTS_PDFXVersion></rdf:Description>",
    );
    let cut = full.find(">PDF/X-4").unwrap();
    for end in 0..=cut {
        assert_eq!(pdfx_claim(&pdf("", "", Some(full.get(..end).unwrap()))), None, "cut at {end}");
    }
}

#[test]
fn an_identifier_is_any_text_not_a_claim_value() {
    let identifier =
        |text: &str| pdfx_output_intents(&pdf(&format!("/OutputIntents [<< /S /GTS_PDFX /OutputConditionIdentifier {text} >>]"), "", None));
    // UTF-16BE "Fogra Ü".
    assert_eq!(identifier("<FEFF0046006F0067007200610020 00DC>"), vec!["Fogra \u{dc}".to_owned()]);
    let long = "C".repeat(80);
    assert_eq!(identifier(&format!("({long})")), vec![long]);
}

#[test]
fn a_binding_on_another_element_does_not_reach_the_property() {
    let xmp = packet(
        "<rdf:Description xmlns:p=\"http://www.npes.org/pdfx/ns/id/\"/>\
         <rdf:Description xmlns:p=\"urn:other\" p:GTS_PDFXVersion=\"PDF/X-4\"/>",
    );
    assert_eq!(pdfx_claim(&pdf("", "", Some(&xmp))), None);
    let nested = packet(
        "<rdf:Description xmlns:p=\"http://www.npes.org/pdfx/ns/id/\"><x:a xmlns:x=\"urn:x\"/>\
         <p:GTS_PDFXVersion>PDF/X-4</p:GTS_PDFXVersion></rdf:Description>",
    );
    assert_eq!(pdfx_claim(&pdf("", "", Some(&nested))).unwrap().level, "PDF/X-4");
}

#[test]
fn a_property_inside_a_comment_is_not_read() {
    let xmp = packet(
        "<rdf:Description xmlns:p=\"http://www.npes.org/pdfx/ns/id/\">\
         <!-- <p:GTS_PDFXVersion>PDF/X-4</p:GTS_PDFXVersion> --></rdf:Description>",
    );
    assert_eq!(pdfx_claim(&pdf("", "", Some(&xmp))), None);
}

#[test]
fn character_references_in_a_value_are_decoded() {
    let attribute = packet("<rdf:Description xmlns:p=\"http://www.npes.org/pdfx/ns/id/\" p:GTS_PDFXVersion=\"PDF/X&#x2D;4\"/>");
    assert!(pdfx_claim(&pdf("", "", Some(&attribute))).unwrap().is_recognised());
    let element =
        packet("<rdf:Description xmlns:p=\"http://www.npes.org/pdfx/ns/id/\"><p:GTS_PDFXVersion>PDF&#47;X-6p</p:GTS_PDFXVersion></rdf:Description>");
    assert_eq!(pdfx_claim(&pdf("", "", Some(&element))).unwrap().level, "PDF/X-6p");
    let unknown = packet("<rdf:Description xmlns:p=\"http://www.npes.org/pdfx/ns/id/\" p:GTS_PDFXVersion=\"PDF/X&bogus;4\"/>");
    assert_eq!(pdfx_claim(&pdf("", "", Some(&unknown))).unwrap().level, "PDF/X&bogus;4");
}

#[test]
fn a_reference_longer_than_the_cap_is_kept_as_written() {
    let long = format!("PDF/X&#{}45;4", "0".repeat(MAX_REFERENCE));
    let xmp = packet(&format!("<rdf:Description xmlns:p=\"http://www.npes.org/pdfx/ns/id/\" p:GTS_PDFXVersion=\"{long}\"/>"));
    assert_eq!(pdfx_claim(&pdf("", "", Some(&xmp))).unwrap().level, long);
}

#[test]
fn the_reference_window_is_counted_in_bytes() {
    // A two-byte character straddles the cap and the only `;` is far past it.
    let name = "a".repeat(MAX_REFERENCE - 2);
    let far = "b".repeat(64);
    let text = format!("&{name}\u{e9}{far};&amp;");
    assert_eq!(decode_references(&text), format!("&{name}\u{e9}{far};&"));
}

#[test]
fn a_property_nested_past_the_depth_cap_states_nothing() {
    let nested = |depth: usize| {
        packet(&format!(
            "<rdf:Description xmlns:p=\"http://www.npes.org/pdfx/ns/id/\">{}<p:GTS_PDFXVersion>PDF/X-4</p:GTS_PDFXVersion>{}</rdf:Description>",
            "<a>".repeat(depth),
            "</a>".repeat(depth)
        ))
    };
    assert!(pdfx_claim(&pdf("", "", Some(&nested(8)))).is_some());
    assert_eq!(pdfx_claim(&pdf("", "", Some(&nested(MAX_DEPTH)))), None);
}
