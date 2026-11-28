//! Missing fonts: resolution and the dialog.

use std::sync::Arc;

use craft_font_catalog::{Limits, Os};
use craft_font_manager::missing::{DocumentFont, FontStatus, MissingFontsWords, document_fonts, missing_font_warnings};
use craft_font_manager::{Catalog, FontEnv, Source};
use craft_ui::testing::trial;

/// Hack, the face egui ships.
fn catalog() -> Arc<Catalog> {
    let catalog = Arc::new(Catalog::new(FontEnv { os: Os::None, ..FontEnv::default() }).with_font_service(false));
    let _ = catalog.add_bytes(epaint_default_fonts::HACK_REGULAR.to_vec(), Source::Bundled, &Limits::default());
    catalog
}

fn font(family: &str, style: &str, postscript_name: Option<&str>) -> DocumentFont {
    DocumentFont::new(family, style, postscript_name)
}

/// A PostScript name that resolves counts as Installed even when the family name differs.
#[trial]
fn postscript_name_resolves_before_family_and_style() {
    let catalog = catalog();
    let fonts = [
        font("Hack Renamed", "Regular", Some("Hack-Regular")),
        font("Hack Renamed", "Regular", Some("Hack-Regular")),
        font("Hack", "Bold Oblique", None),
        font("Nonesuch", "Regular", Some("Nonesuch-Regular")),
    ];
    let rows = document_fonts(&catalog, &fonts);
    assert_eq!(rows.len(), 3, "the duplicate is merged");
    assert_eq!(rows[0].status, FontStatus::Installed);
    assert!(matches!(rows[1].status, FontStatus::OtherStyle(_)));
    assert_eq!(rows[2].status, FontStatus::Missing);
    assert_eq!(
        missing_font_warnings(&rows, &MissingFontsWords::default()),
        ["font 'Nonesuch' is not installed; text may render using a fallback face"]
    );
}
