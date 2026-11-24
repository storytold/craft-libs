//! Optional content (layers): `Document::add_optional_content_group`,
//! `Surface::start_optional_content` and `Surface::end_optional_content`, read back with the
//! shared reader.
#![cfg(feature = "write")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Arc;

use backend::SerializeSettings;
use backend::configure::{Archival, PdfVersion, ValidationError};
use backend::error::KrillaError;
use backend::geom::{PathBuilder, Rect, Size};
use backend::optional_content::OcgId;
use backend::page::PageSettings;
use backend::paint::Fill;
use backend::surface::Surface;
use backend::tagging::ContentTag;
use craft_pdf::write::{self, Metadata, WriteError, Writer, backend};
use hayro_syntax::object::{Array, Dict, Name, Stream, String as PdfString};

const NOTES: &str = "注記";

fn rect(surface: &mut Surface<'_>, x: f32) {
    let mut path = PathBuilder::new();
    path.push_rect(Rect::from_xywh(x, 10.0, 20.0, 20.0).unwrap());
    surface.set_fill(Some(Fill::default()));
    surface.draw_path(&path.finish().unwrap());
}

fn page_settings() -> PageSettings {
    PageSettings::new(Size::from_wh(200.0, 60.0).unwrap())
}

/// Two pages and three groups: "Walls" (shown), the hidden notes layer and "Unused" (shown,
/// on no page). Page 1 marks one rect per layer, plus a mark inside an isolated group. Page 2
/// nests the notes inside the walls and a tagged section inside the notes.
fn layered(settings: SerializeSettings, page_mode: bool) -> Result<Vec<u8>, WriteError> {
    let mut writer = Writer::new(settings, Metadata::default());
    let document = writer.document_mut();
    let walls = document.add_optional_content_group("Walls", true);
    let notes = document.add_optional_content_group(NOTES, false);
    document.add_optional_content_group("Unused", true);
    document.set_optional_content_page_mode(page_mode);
    {
        let mut page = document.start_page_with(page_settings());
        let mut surface = page.surface();
        surface.start_optional_content(walls).unwrap();
        rect(&mut surface, 0.0);
        surface.end_optional_content().unwrap();
        surface.start_optional_content(notes).unwrap();
        rect(&mut surface, 30.0);
        surface.end_optional_content().unwrap();
        surface.push_isolated();
        surface.start_optional_content(notes).unwrap();
        rect(&mut surface, 60.0);
        surface.end_optional_content().unwrap();
        surface.pop();
        surface.finish();
        page.finish();
    }
    {
        let mut page = document.start_page_with(page_settings());
        let mut surface = page.surface();
        surface.start_optional_content(walls).unwrap();
        rect(&mut surface, 0.0);
        surface.start_optional_content(notes).unwrap();
        surface.start_tagged(ContentTag::Other);
        rect(&mut surface, 30.0);
        surface.end_tagged();
        surface.end_optional_content().unwrap();
        surface.end_optional_content().unwrap();
        surface.finish();
        page.finish();
    }
    writer.finish()
}

fn text(string: &PdfString<'_>) -> String {
    let bytes = string.as_bytes();
    match bytes.strip_prefix(&[0xFE, 0xFF]) {
        Some(utf16) => String::from_utf16(&utf16.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect::<Vec<_>>()).unwrap(),
        None => String::from_utf8(bytes.to_vec()).unwrap(),
    }
}

fn ocg_name(ocg: &Dict<'_>) -> String {
    assert_eq!(ocg.get::<Name<'_>>(b"Type".as_slice()).unwrap().as_str(), "OCG");
    text(&ocg.get::<PdfString<'_>>(b"Name".as_slice()).unwrap())
}

fn names(array: &Array<'_>) -> Vec<String> {
    array.iter::<Dict<'_>>().map(|ocg| ocg_name(&ocg)).collect()
}

fn open(bytes: &[u8]) -> hayro_syntax::Pdf {
    hayro_syntax::Pdf::new(Arc::new(bytes.to_vec())).unwrap()
}

fn root(pdf: &hayro_syntax::Pdf) -> Dict<'_> {
    pdf.xref().get::<Dict<'_>>(pdf.xref().root_id()).unwrap()
}

/// The `/OC /name BDC` names in a content stream, after checking that every `BDC` has its
/// `EMC` and that nothing ends before it starts.
fn oc_marks(content: &[u8]) -> Vec<String> {
    // pdf-writer omits the space before a name and around dictionaries.
    let content = String::from_utf8_lossy(content).replace('/', " /").replace("<<", " << ").replace(">>", " >> ");
    let tokens: Vec<&str> = content.split_whitespace().collect();
    let mut depth = 0usize;
    let mut marks = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        match *token {
            "BDC" => {
                depth += 1;
                if i >= 2 && tokens[i - 2] == "/OC" {
                    marks.push(tokens[i - 1].trim_start_matches('/').to_owned());
                }
            }
            "EMC" => depth = depth.checked_sub(1).expect("EMC without BDC"),
            _ => {}
        }
    }
    assert_eq!(depth, 0, "unbalanced marked content");
    marks
}

/// Each `/Properties` entry of a resource dictionary, as resource name to group name.
fn properties(resources: &Dict<'_>) -> BTreeMap<String, String> {
    resources.keys().map(|key| (key.as_str().to_owned(), ocg_name(&resources.get::<Dict<'_>>(key.as_ref()).unwrap()))).collect()
}

/// Per page: the layer of each optional content mark, in content order.
fn page_layers(bytes: &[u8]) -> Vec<Vec<String>> {
    let pdf = open(bytes);
    pdf.pages()
        .iter()
        .map(|page| {
            let map = properties(&page.resources().properties);
            oc_marks(page.page_stream().unwrap())
                .iter()
                .map(|mark| map.get(mark).unwrap_or_else(|| panic!("/{mark} is not in /Properties")).clone())
                .collect()
        })
        .collect()
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn groups_are_listed_in_the_catalog_with_hidden_ones_off() {
    let bytes = layered(SerializeSettings::default(), false).unwrap();
    let pdf = open(&bytes);
    let root = root(&pdf);
    let properties = root.get::<Dict<'_>>(b"OCProperties".as_slice()).unwrap();
    let all = ["Walls", NOTES, "Unused"].map(str::to_owned).to_vec();
    assert_eq!(names(&properties.get::<Array<'_>>(b"OCGs".as_slice()).unwrap()), all);
    let config = properties.get::<Dict<'_>>(b"D".as_slice()).unwrap();
    assert_eq!(text(&config.get::<PdfString<'_>>(b"Name".as_slice()).unwrap()), "Layers");
    assert_eq!(names(&config.get::<Array<'_>>(b"Order".as_slice()).unwrap()), all);
    assert_eq!(names(&config.get::<Array<'_>>(b"OFF".as_slice()).unwrap()), vec![NOTES.to_owned()]);
    assert!(root.get::<Name<'_>>(b"PageMode".as_slice()).is_none());
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn page_mode_use_oc_is_opt_in() {
    let bytes = layered(SerializeSettings::default(), true).unwrap();
    let pdf = open(&bytes);
    assert_eq!(root(&pdf).get::<Name<'_>>(b"PageMode".as_slice()).unwrap().as_str(), "UseOC");

    // Without groups there is nothing to show.
    let mut writer = Writer::new(SerializeSettings::default(), Metadata::default());
    writer.document_mut().set_optional_content_page_mode(true);
    let bytes = writer.finish().unwrap();
    let pdf = open(&bytes);
    assert!(root(&pdf).get::<Name<'_>>(b"PageMode".as_slice()).is_none());
    assert!(root(&pdf).get::<Dict<'_>>(b"OCProperties".as_slice()).is_none());
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn marks_name_groups_in_their_own_stream_resources() {
    let bytes = layered(SerializeSettings::default(), false).unwrap();
    assert_eq!(page_layers(&bytes), vec![vec!["Walls".to_owned(), NOTES.to_owned()], vec!["Walls".to_owned(), NOTES.to_owned()]]);

    // Page 2 holds a tagged section inside the nested marks: three BDCs, balanced.
    let pdf = open(&bytes);
    let page = &pdf.pages()[1];
    let content = String::from_utf8_lossy(page.page_stream().unwrap()).into_owned();
    assert_eq!(content.matches("BDC").count(), 3);

    // The isolated group on page 1 is a form XObject with its own `/Properties`.
    let page = &pdf.pages()[0];
    let x_objects = &page.resources().x_objects;
    let forms: Vec<Stream<'_>> = x_objects.keys().filter_map(|key| x_objects.get::<Stream<'_>>(key.as_ref())).collect();
    assert_eq!(forms.len(), 1);
    let resources = forms[0].dict().get::<Dict<'_>>(b"Resources".as_slice()).unwrap();
    let map = properties(&resources.get::<Dict<'_>>(b"Properties".as_slice()).unwrap());
    let decoded = forms[0].decoded().unwrap();
    let layers: Vec<String> = oc_marks(&decoded).iter().map(|mark| map[mark].clone()).collect();
    assert_eq!(layers, vec![NOTES.to_owned()]);
}

fn validation_errors(version: PdfVersion, archival: Option<Archival>) -> Vec<ValidationError> {
    let settings = SerializeSettings { configuration: write::configuration(version, archival).unwrap(), ..Default::default() };
    match layered(settings, false) {
        Ok(_) => Vec::new(),
        Err(WriteError::Serialization(error)) => match *error {
            KrillaError::Validation(errors) => errors.into_iter().map(|(error, _)| error).collect(),
            other => panic!("expected validation errors, got {other:?}"),
        },
        Err(other) => panic!("expected validation errors, got {other:?}"),
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn optional_content_is_a_pdf_a1_error() {
    let is_oc = |error: &ValidationError| matches!(error, ValidationError::OptionalContent(_));
    // The metadata is missing, so every archival export fails; only PDF/A-1 names layers.
    assert!(validation_errors(PdfVersion::Pdf14, Some(Archival::A1_B)).iter().any(is_oc));
    assert!(!validation_errors(PdfVersion::Pdf17, Some(Archival::A2_B)).iter().any(is_oc));
    assert!(validation_errors(PdfVersion::Pdf14, None).is_empty());
}

fn one_page(draw: impl FnOnce(&mut Surface<'_>, OcgId)) -> Result<Vec<u8>, WriteError> {
    let mut writer = Writer::new(SerializeSettings::default(), Metadata::default());
    let layer = writer.document_mut().add_optional_content_group("Layer", true);
    {
        let mut page = writer.document_mut().start_page_with(page_settings());
        let mut surface = page.surface();
        draw(&mut surface, layer);
        surface.finish();
        page.finish();
    }
    writer.finish()
}

fn serialization_error(result: Result<Vec<u8>, WriteError>) -> KrillaError {
    match result {
        Err(WriteError::Serialization(error)) => *error,
        other => panic!("expected a serialization error, got {:?}", other.map(|bytes| bytes.len())),
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn misuse_returns_errors_instead_of_panicking() {
    // An end without a start, and an end that would close a tagged section started inside
    // the mark, are refused and write nothing; the document stays valid.
    let bytes = one_page(|surface, layer| {
        assert!(matches!(surface.end_optional_content(), Err(KrillaError::UnbalancedOptionalContent(_))));
        surface.start_optional_content(layer).unwrap();
        surface.start_tagged(ContentTag::Other);
        assert!(matches!(surface.end_optional_content(), Err(KrillaError::UnbalancedOptionalContent(_))));
        surface.end_tagged();
        surface.end_optional_content().unwrap();
    })
    .unwrap();
    assert_eq!(page_layers(&bytes), vec![vec!["Layer".to_owned()]]);

    // A group of another document.
    let mut other = backend::Document::new();
    let foreign = (0..4).map(|i| other.add_optional_content_group(format!("{i}"), true)).last().unwrap();
    one_page(|surface, _| {
        assert!(matches!(surface.start_optional_content(foreign), Err(KrillaError::UnknownOptionalContentGroup(_))));
    })
    .unwrap();

    // Marks left open when the page or a group finishes, and a tagged section that ends
    // inside a mark started within it, fail the document.
    let unbalanced = |draw: fn(&mut Surface<'_>, OcgId)| {
        assert!(matches!(serialization_error(one_page(draw)), KrillaError::UnbalancedOptionalContent(_)));
    };
    unbalanced(|surface, layer| surface.start_optional_content(layer).unwrap());
    unbalanced(|surface, layer| {
        surface.push_isolated();
        surface.start_optional_content(layer).unwrap();
        surface.pop();
    });
    unbalanced(|surface, layer| {
        surface.start_tagged(ContentTag::Other);
        surface.start_optional_content(layer).unwrap();
        surface.end_tagged();
        assert!(surface.end_optional_content().is_err());
    });
}
