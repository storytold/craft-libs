//! PDF presets: named [`PdfSettings`]. The built-in ones are generated here (the app default,
//! which keeps the document editable, plus presets for print, press, small files and the PDF/X
//! standards); the user's own live in the app's preferences.

use super::{
    BleedSettings, ColorConversion, Compatibility, CompressionSettings, DOCUMENT_CMYK, Downsample, ImageCodec, ImageCompression, JpegQuality,
    MonoCodec, MonoCompression, OutputSettings, PdfSettings, Standard,
};

/// A named set of PDF settings: a built-in preset or one the user saved.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfPreset {
    #[allow(missing_docs)]
    pub name: String,
    /// What the preset is for.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default)]
    #[allow(missing_docs)]
    pub settings: PdfSettings,
}

/// The name of the built-in preset every PDF export starts from, in the app called `app`.
pub fn default_preset_name(app: &str) -> String {
    format!("{app} Default")
}

/// Colour and greyscale images resampled (bicubic) to `ppi` above `above` ppi, compressed with
/// `codec`.
fn images(ppi: f64, above: f64, codec: ImageCodec, quality: JpegQuality) -> ImageCompression {
    ImageCompression { downsample: Downsample::Bicubic, ppi, above_ppi: above, compression: codec, quality }
}

/// Compression with the same settings for colour and greyscale images, and CCITT Group 4
/// monochrome images resampled to `mono_ppi` above 1.5 × that.
fn compression(img: ImageCompression, mono_ppi: f64) -> CompressionSettings {
    let mono = MonoCompression { downsample: Downsample::Bicubic, ppi: mono_ppi, above_ppi: mono_ppi * 1.5, compression: MonoCodec::CcittG4 };
    CompressionSettings { color: img.clone(), gray: img, mono, compress_text: true }
}

/// Settings for print: PDF 1.6, images kept at 300 ppi, the document's bleed when `bleed`.
fn print(codec: ImageCodec, bleed: bool) -> PdfSettings {
    PdfSettings {
        compatibility: Compatibility::Pdf16,
        compression: compression(images(300.0, 450.0, codec, JpegQuality::Maximum), 1200.0),
        bleed: BleedSettings { use_document: bleed, ..Default::default() },
        ..Default::default()
    }
}

/// A PDF/X preset: print settings for `standard` at PDF version `compatibility`.
fn pdf_x(standard: Standard, compatibility: Compatibility) -> PdfSettings {
    PdfSettings { standard, compatibility, ..print(ImageCodec::Auto, true) }
}

/// Every built-in preset, the app default (named by [`default_preset_name`]) first.
pub fn builtin_presets(app: &str) -> Vec<PdfPreset> {
    let preset = |name: &str, description: &str, settings| PdfPreset { name: name.into(), description: description.into(), settings };
    vec![
        preset(
            &default_preset_name(app),
            &format!("Keeps the document editable: {app} reopens the PDF with nothing lost. For files you'll edit again."),
            PdfSettings { preserve_editing: true, ..Default::default() },
        ),
        preset("High Quality Print", "For desktop printers and proofs: images stay at print resolution.", print(ImageCodec::Auto, false)),
        // Converted to Document CMYK, the numbers of colours already in it kept: RGB colours and
        // images come out CMYK, in an RGB document too, as a press needs. A printer's published
        // settings for the [Press Quality] preset list the same Output: Convert to Destination
        // (Preserve Numbers), Document CMYK, no profiles included
        // (https://postmediasolutions.com/hubfs/Website/Client%20Resources/PostmediaPrintPDFSpecSheet.pdf).
        preset(
            "Press Quality",
            "For commercial printing: high-resolution images compressed losslessly, the document's bleed, and colours converted to the CMYK profile in effect (Document CMYK), keeping the numbers of colours already in it.",
            PdfSettings {
                output: OutputSettings { conversion: ColorConversion::PreserveNumbers, destination: DOCUMENT_CMYK.into(), ..Default::default() },
                ..print(ImageCodec::Zip, true)
            },
        ),
        preset(
            "Smallest File Size",
            "For screens, email and the web: images resampled to 100 ppi and compressed.",
            PdfSettings {
                compatibility: Compatibility::Pdf16,
                fast_web_view: true,
                compression: compression(images(100.0, 150.0, ImageCodec::Jpeg, JpegQuality::Medium), 300.0),
                ..Default::default()
            },
        ),
        preset(
            "PDF/X-1a:2001",
            "Print exchange with CMYK and spot colours only: transparency flattened, fonts embedded.",
            pdf_x(Standard::PdfX1a, Compatibility::Pdf13),
        ),
        preset(
            "PDF/X-3:2002",
            "Print exchange allowing colour-managed colours: transparency flattened, fonts embedded.",
            pdf_x(Standard::PdfX3, Compatibility::Pdf13),
        ),
        preset(
            "PDF/X-4:2010",
            "Print exchange keeping live transparency and layers, colour-managed colours and embedded fonts.",
            pdf_x(Standard::PdfX4, Compatibility::Pdf16),
        ),
    ]
}

/// The built-in preset `name` names (any case; `default` is the app default).
pub fn builtin_preset(name: &str, app: &str) -> Option<PdfPreset> {
    let name = name.trim();
    let name = if name.eq_ignore_ascii_case("default") { default_preset_name(app) } else { name.to_string() };
    builtin_presets(app).into_iter().find(|p| p.name.eq_ignore_ascii_case(&name))
}
