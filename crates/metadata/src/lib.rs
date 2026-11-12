#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Photo metadata: EXIF, IPTC-IIM, XMP packets and GPX track logs, read into one [`Metadata`] model.
//!
//! One module per format:
//! - EXIF: [`read_exif`] reads a TIFF-structured Exif block, [`from_tiff`] an already parsed stream,
//!   [`write_exif`] writes one. Maker notes supply the lens and, in [`focus`], where the camera focused.
//!   [`tags`] lists every tag by name.
//! - IPTC-IIM: [`parse_iptc`] reads record 2 datasets.
//! - XMP: [`parse_xmp`] and [`write_xmp`] read and write packets; [`merge_xmp`] updates a packet
//!   another program wrote without losing what it does not understand.
//! - GPX: [`parse_gpx`] reads track logs for geotagging by capture time.
//! - Containers: [`embedded`], [`jpeg_segments`], [`png_chunks`] and [`webp_chunks`] locate the Exif,
//!   XMP, ICC and IPTC blocks of JPEG, PNG, WebP, TIFF-based, Fujifilm RAF, Canon CR3 ([`cr3`]) and
//!   JPEG XL ([`jxl`]) files.
//!   [`extract`] does all of the above for a whole file. [`mpf`] reads and writes the Multi-Picture
//!   index of a JPEG that carries further images, such as a gain map.
//! - [`edit`]: byte-preserving Exif and XMP edits for export (Orientation, resolution, thumbnail).
//!
//! HEIF, PSD and MRW files are read by their own crates. Put the blocks they return into an
//! [`Embedded`] and call [`from_embedded`].

mod container;
pub mod cr3;
mod datetime;
pub mod edit;
mod exif;
pub mod focus;
mod focus_read;
mod fuji;
mod geom;
mod gpx;
mod iptc;
pub mod jxl;
pub mod mpf;
pub mod tags;
mod xmp;
mod xmp_merge;

pub use container::{Embedded, embedded, jpeg_segments, jpeg_segments_with_all_xmp, png_chunks, webp_chunks};
pub use datetime::{DatePrecision, DateTime};
pub use exif::{from_tiff, read_exif, strip_exif_header, try_read_exif, try_read_exif_with, write_exif};
pub use focus::FocusPoint;
pub use fuji::{FujiRecipe, FujiRow, might_hold_fuji_recipe, read_recipe_file, recipe_from_bytes, recipe_from_exif};
pub use geom::{Orientation, Point, Rect};
pub use gpx::{GpxError, Match, TrackPoint, Tracklog, parse_gpx};
pub use iptc::parse_iptc;
pub use tags::{TagRow, file_tag_rows, tag_rows};
pub use xmp::{
    CRS_NS, XmpData, XmpError, XmpNamespace, XmpValue, XmpWrite, parse_gps_position, parse_xmp, parse_xmp_with, write_xmp, write_xmp_with,
};
pub use xmp_merge::{MergeRules, merge_xmp};

use serde::{Deserialize, Serialize};

/// GPS position (WGS 84). Latitude/longitude in signed decimal degrees (north / east positive).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Gps {
    pub latitude: f64,
    pub longitude: f64,
    /// Metres above sea level (negative below).
    pub altitude: Option<f64>,
}

/// Exif `Flash` value decoded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flash {
    pub fired: bool,
    /// The raw Exif `Flash` bit field.
    pub raw: u16,
}

/// A named region of interest on a photo (MWG Region Guidelines, `mwg-rs:Regions`): most often a face,
/// drawn by Lightroom, digiKam, Picasa or similar tools. Read only: [`write_xmp`] does not write regions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Region {
    /// Normalized to the image's full, oriented frame (`Metadata::width`/`height`), y-down, 0..1 on
    /// both axes — MWG's own convention, and what `AppliedToDimensions` areas are converted to.
    pub rect: Rect,
    pub kind: RegionKind,
    /// `mwg-rs:Name` (e.g. the person's name).
    pub name: Option<String>,
    pub description: Option<String>,
}

/// `mwg-rs:Type`. Unrecognised values are kept verbatim rather than dropped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegionKind {
    Face,
    Pet,
    Focus,
    BarCode,
    /// `mwg-rs:Type` was missing, empty, or a value we don't recognise.
    Other(String),
}

/// What a photo catalogue shows or searches about a photo. All fields are optional; unknown = `None`/empty.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Metadata {
    // camera
    pub make: Option<String>,
    pub model: Option<String>,
    pub serial_number: Option<String>,
    pub software: Option<String>,
    // lens
    pub lens_make: Option<String>,
    pub lens_model: Option<String>,
    pub lens_serial_number: Option<String>,
    /// Min/max focal length (mm) and min/max f-number at those, as in Exif `LensSpecification`.
    pub lens_spec: Option<[f64; 4]>,
    // capture
    pub capture_time: Option<DateTime>,
    /// Seconds.
    pub exposure_time: Option<f64>,
    pub f_number: Option<f64>,
    pub iso: Option<u32>,
    /// Millimetres.
    pub focal_length: Option<f64>,
    pub focal_length_35mm: Option<f64>,
    pub flash: Option<Flash>,
    /// EV.
    pub exposure_bias: Option<f64>,
    /// Exif `ExposureProgram` code.
    pub exposure_program: Option<u16>,
    /// Exif `MeteringMode` code.
    pub metering_mode: Option<u16>,
    /// Exif `WhiteBalance` (0 auto, 1 manual).
    pub white_balance: Option<u16>,
    pub orientation: Option<Orientation>,
    pub gps: Option<Gps>,
    /// Pixel dimensions as recorded (not oriented).
    pub width: Option<u32>,
    pub height: Option<u32>,
    // descriptive / user
    pub artist: Option<String>,
    pub copyright: Option<String>,
    /// Copyright status (`xmpRights:Marked`): `true` = copyrighted, `false` = public domain, `None` = unknown.
    pub copyright_marked: Option<bool>,
    /// Rights usage terms (`xmpRights:UsageTerms`).
    pub usage_terms: Option<String>,
    /// Copyright info URL (`xmpRights:WebStatement`).
    pub copyright_url: Option<String>,
    pub title: Option<String>,
    pub caption: Option<String>,
    /// Accessibility text (`Iptc4xmpCore:AltTextAccessibility`).
    pub alt_text: Option<String>,
    /// Long accessibility description (`Iptc4xmpCore:ExtDescrAccessibility`).
    pub extended_description: Option<String>,
    /// Place within the city (`Iptc4xmpCore:Location`).
    pub sublocation: Option<String>,
    /// `photoshop:City`, `photoshop:State`, `photoshop:Country`.
    pub city: Option<String>,
    pub state: Option<String>,
    pub country: Option<String>,
    pub keywords: Vec<String>,
    /// Hierarchical keywords, `|`-separated paths (`lr:hierarchicalSubject` convention).
    pub hierarchical_keywords: Vec<String>,
    /// -1 = rejected, 0 = unrated, 1..=5 stars.
    pub rating: Option<i8>,
    /// Colour label name (e.g. "Red").
    pub label: Option<String>,
    /// Face/pet/focus/barcode regions (`mwg-rs:Regions`), read from XMP only.
    pub regions: Vec<Region>,
    /// Where the camera focused, from its maker notes ([`focus`]; Sony only for now), on the upright photo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<FocusPoint>,
    /// Fujifilm in-camera film recipe from the MakerNote. Not written back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fuji_recipe: Option<FujiRecipe>,
}

macro_rules! fill {
    ($self:ident, $o:ident, $($f:ident),*) => { $( if $self.$f.is_none() { $self.$f = $o.$f.clone(); } )* };
}
macro_rules! overlay {
    ($self:ident, $o:ident, $($f:ident),*) => { $( if $o.$f.is_some() { $self.$f = $o.$f.clone(); } )* };
}

/// How far (relative) a duration may be from an exact `1/n` second and still be shown as `1/n`:
/// absorbs decimal EXIF rationals such as `3333/10000` for 1/3 s without turning 0.3 s or
/// 1/2.8 s into a reciprocal they aren't.
pub const RECIPROCAL_TOLERANCE: f64 = 1e-3;

/// The one shutter-speed formatter for display (catalog `shutter`, Info panel, All Metadata):
/// a near-exact `1/n` second as `1/n` (`1/3`, `1/250`), anything else rounded to four
/// significant digits with trailing zeros dropped (`0.3`, `0.3571`, `2.5`, `30`). Display only:
/// export writes the exact duration. `None` for a time that isn't positive and finite.
pub fn format_exposure_time(t: f64) -> Option<String> {
    if !t.is_finite() || t <= 0.0 {
        return None;
    }
    let n = (1.0 / t).round();
    if n >= 2.0 && n.is_finite() && (1.0 / (n * t) - 1.0).abs() <= RECIPROCAL_TOLERANCE {
        return Some(format!("1/{n}"));
    }
    // four significant digits; whole seconds never lose digits
    let decimals = (3 - t.log10().floor() as i32).max(0) as usize;
    let text = format!("{t:.decimals$}");
    Some(if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.').to_string() } else { text })
}

impl Metadata {
    /// Fill every empty field of `self` from `other`.
    pub fn fill_missing(&mut self, other: &Metadata) {
        fill!(
            self,
            other,
            make,
            model,
            serial_number,
            software,
            lens_make,
            lens_model,
            lens_serial_number,
            lens_spec,
            capture_time,
            exposure_time,
            f_number,
            iso,
            focal_length,
            focal_length_35mm,
            flash,
            exposure_bias,
            exposure_program,
            metering_mode,
            white_balance,
            orientation,
            gps,
            width,
            height
        );
        self.fill_missing_descriptive(other);
    }

    /// Fill descriptive metadata separately from the camera and image properties.
    fn fill_missing_descriptive(&mut self, other: &Metadata) {
        fill!(
            self,
            other,
            artist,
            copyright,
            copyright_marked,
            usage_terms,
            copyright_url,
            title,
            caption,
            alt_text,
            extended_description,
            sublocation,
            city,
            state,
            country,
            rating,
            label
        );
        if self.keywords.is_empty() {
            self.keywords = other.keywords.clone();
        }
        if self.hierarchical_keywords.is_empty() {
            self.hierarchical_keywords = other.hierarchical_keywords.clone();
        }
        if self.regions.is_empty() {
            self.regions = other.regions.clone();
        }
        if self.focus.is_none() {
            self.focus.clone_from(&other.focus);
        }
        if self.fuji_recipe.is_none() {
            self.fuji_recipe.clone_from(&other.fuji_recipe);
        }
    }

    /// Replace the user-editable fields (rating, label, title, caption, artist, copyright, keywords, GPS) with
    /// `other`'s where `other` has them — the XMP-over-EXIF precedence rule.
    pub fn overlay_user_fields(&mut self, other: &Metadata) {
        overlay!(self, other, rating, label, title, caption, artist, copyright, copyright_marked, usage_terms, copyright_url, gps);
        if !other.keywords.is_empty() {
            self.keywords = other.keywords.clone();
        }
        if !other.hierarchical_keywords.is_empty() {
            self.hierarchical_keywords = other.hierarchical_keywords.clone();
        }
        if !other.regions.is_empty() {
            self.regions = other.regions.clone();
        }
    }

    /// Exposure time formatted like cameras show it: `1/250`, `0.3571`, `2.5″` (see [`format_exposure_time`]).
    pub fn exposure_display(&self) -> Option<String> {
        let t = self.exposure_time?;
        format_exposure_time(t).map(|text| if t >= 1.0 { format!("{text}″") } else { text })
    }

    /// Human name of the metering mode.
    pub fn metering_name(&self) -> Option<&'static str> {
        Some(match self.metering_mode? {
            1 => "Average",
            2 => "Center-weighted average",
            3 => "Spot",
            4 => "Multi-spot",
            5 => "Pattern",
            6 => "Partial",
            255 => "Other",
            _ => "Unknown",
        })
    }

    /// Human name of the exposure program.
    pub fn exposure_program_name(&self) -> Option<&'static str> {
        Some(match self.exposure_program? {
            1 => "Manual",
            2 => "Program AE",
            3 => "Aperture-priority AE",
            4 => "Shutter speed priority AE",
            5 => "Creative (slow speed)",
            6 => "Action (high speed)",
            7 => "Portrait",
            8 => "Landscape",
            9 => "Bulb",
            _ => "Not defined",
        })
    }
}

/// Read everything understood from a whole file (JPEG, PNG, WebP, TIFF-based incl. DNG and most raws, RAF, CR3, JPEG XL).
/// Precedence: EXIF, then IPTC fills gaps, then XMP overrides user fields and fills the rest.
pub fn extract(bytes: &[u8]) -> Metadata {
    from_embedded(&embedded(bytes))
}

/// [`extract`] for blocks that are already located, for example by a HEIF or PSD reader. Best effort:
/// a block that cannot be read counts as absent. A caller that must tell "no location" from
/// "unreadable" uses [`try_read_exif`] and [`parse_xmp`] on the blocks first.
pub fn from_embedded(e: &Embedded) -> Metadata {
    let mut m = e.exif.as_deref().map(read_exif).unwrap_or_default();
    let xmp = embedded_xmp(e).unwrap_or_else(|error| error.standard.map(|x| *x));
    // the size the Exif states: a size only the XMP states may be the upright one, so it can't tell how the photo is stored
    let stored = m.width.zip(m.height);
    // and the orientation the Exif states: the photo is displayed by that alone, so an orientation only the XMP
    // states ([`Metadata::fill_missing`] reports it below) can't tell how the photo is stored either
    let exif_orientation = e.exif.as_deref().and_then(exif::ifd0_orientation);
    if let Some(i) = e.iptc.as_deref() {
        m.fill_missing(&parse_iptc(i));
    }
    if let Some(xmp) = xmp {
        m.overlay_user_fields(&xmp.metadata);
        m.fill_missing(&xmp.metadata);
        // boxes another app drew on the photo as stored go where the upright photo has them
        if let (Some(drawn_on), Some(orientation), Some(stored)) = (xmp.regions_drawn_on(), exif_orientation, stored) {
            upright_regions(&mut m.regions, drawn_on, orientation, stored);
        }
    }
    if let Some(exif) = e.exif.as_deref() {
        m.fuji_recipe = fuji::recipe_from_exif(exif);
    }
    m
}

/// The XMP of `e` read as one packet: the standard packet, with the Extended XMP it links to
/// (`xmpNote:HasExtendedXMP`) merged in (see [`xmp::parse_xmp_pair`]). An error when the standard
/// packet is not XMP, when its link is malformed, when the linked extension is missing or
/// unreadable ([`Embedded::xmp_extended_error`]), or when there is an extension nothing links to:
/// a caller that must see all of a file's metadata (to prove it carries no location, say) refuses
/// the file then. [`from_embedded`] falls back to [`EmbeddedXmpError::standard`].
pub fn embedded_xmp(e: &Embedded) -> Result<Option<XmpData>, EmbeddedXmpError> {
    let fail = |standard: Option<XmpData>, message: String| EmbeddedXmpError { standard: standard.map(Box::new), message };
    let Some(packet) = e.xmp.as_deref() else {
        return match (&e.xmp_extended, &e.xmp_extended_error) {
            (None, None) => Ok(None),
            _ => Err(fail(None, "Extended XMP is present without a standard packet".into())),
        };
    };
    let parsed = parse_xmp(packet).map_err(|error| fail(None, format!("embedded XMP is invalid: {error}")))?;
    let mut standard = parsed.clone();
    standard.properties.remove("xmpNote:HasExtendedXMP");
    standard.values.remove("xmpNote:HasExtendedXMP");
    let reference = xmp::extended_xmp_reference(&parsed).map_err(|error| fail(Some(standard.clone()), error))?;
    match (reference, e.xmp_extended.as_deref()) {
        (Some(guid), Some(extended)) if e.xmp_extended_guid.as_deref() == Some(guid.as_str()) => xmp::parse_xmp_pair(packet, extended, &[])
            .map(Some)
            .map_err(|error| fail(Some(standard), format!("the linked Extended XMP is invalid: {error}"))),
        (Some(_), _) => {
            let why = e.xmp_extended_error.clone().unwrap_or_else(|| "the linked Extended XMP is missing".into());
            Err(fail(Some(standard), why))
        }
        (None, Some(_)) => Err(fail(Some(standard), "Extended XMP is present but the standard packet does not link it".into())),
        (None, None) => Ok(Some(standard)),
    }
}

/// Why [`embedded_xmp`] could not read a file's XMP whole.
#[derive(Clone, Debug, PartialEq)]
pub struct EmbeddedXmpError {
    /// The standard packet, when it could be read: what a best-effort reader uses.
    pub standard: Option<Box<XmpData>>,
    pub message: String,
}

impl std::fmt::Display for EmbeddedXmpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for EmbeddedXmpError {}

/// Turn regions drawn on a photo as it is stored onto the upright photo. MWG gives each box as a fraction of
/// `AppliedToDimensions` (`drawn_on`); for a photo whose Exif `orientation` turns it a quarter turn, some apps give the
/// stored, sideways size there and boxes on the stored pixels to match (Lightroom says so per box with
/// `mwg-rs:Rotation` instead, which [`parse_xmp`] reads). When `drawn_on` has the shape of the photo as stored
/// (`stored`, before `orientation`) rather than the upright one, every box is turned with `orientation`. Returns
/// whether they were. A square photo or `drawn_on` looks the same both ways, and a half turn or a mirror doesn't
/// change the shape: their boxes are left as they are.
pub fn upright_regions(regions: &mut [Region], drawn_on: (f64, f64), orientation: Orientation, stored: (u32, u32)) -> bool {
    let ((dw, dh), (sw, sh)) = (drawn_on, stored);
    if regions.is_empty() || !orientation.swaps_axes() || sw == sh || dw == dh || (dw > dh) != (sw > sh) {
        return false;
    }
    for r in regions.iter_mut() {
        r.rect = orientation.map_norm_rect(r.rect);
    }
    true
}

/// A lens name from its focal and aperture range (Exif LensSpecification, or a Nikon maker note's Lens), written the
/// way Lightroom shows a lens it has no name for: `50.0 mm f/1.8`, `18.0-135.0 mm f/3.5-5.6`.
pub fn lens_spec_name(spec: [f64; 4]) -> Option<String> {
    let [short, long, wide, narrow] = spec;
    if ![short, long, wide, narrow].iter().all(|v| v.is_finite() && *v > 0.0) || long < short {
        return None;
    }
    let focal = if (long - short).abs() < 0.05 { format!("{short:.1}") } else { format!("{short:.1}-{long:.1}") };
    let aperture = if (narrow - wide).abs() < 0.05 { format!("{wide:.1}") } else { format!("{wide:.1}-{narrow:.1}") };
    Some(format!("{focal} mm f/{aperture}"))
}

/// Whether `s` has the `<focal> mm f/<aperture>` shape written by [`lens_spec_name`].
pub fn is_lens_spec_name(s: &str) -> bool {
    let Some((focal, aperture)) = s.trim().split_once(" mm f/") else { return false };
    let is_num_or_range = |part: &str| {
        let mut pieces = part.split('-');
        let Some(first) = pieces.next() else { return false };
        let second = pieces.next();
        pieces.next().is_none()
            && first.parse::<f64>().is_ok_and(|v| v.is_finite() && v > 0.0)
            && second.is_none_or(|b| b.parse::<f64>().is_ok_and(|v| v.is_finite() && v > 0.0))
    };
    is_num_or_range(focal) && is_num_or_range(aperture)
}

/// Lenient number parsing used by XMP/IPTC: `"28/10"`, `"2.8"`, `"+0.7"`.
pub(crate) fn parse_number(s: &str) -> Option<f64> {
    let s = s.trim();
    if let Some((n, d)) = s.split_once('/') {
        let (n, d) = (n.trim().parse::<f64>().ok()?, d.trim().parse::<f64>().ok()?);
        return (d != 0.0).then_some(n / d).filter(|v| v.is_finite());
    }
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lens_names_from_a_spec_read_like_lightroom() {
        assert_eq!(lens_spec_name([50.0, 50.0, 1.8, 1.8]).as_deref(), Some("50.0 mm f/1.8"));
        assert_eq!(lens_spec_name([18.0, 135.0, 3.5, 5.6]).as_deref(), Some("18.0-135.0 mm f/3.5-5.6"));
        assert_eq!(lens_spec_name([24.0, 70.0, 2.8, 2.8]).as_deref(), Some("24.0-70.0 mm f/2.8"));
        assert_eq!(lens_spec_name([70.0, 210.0, 4.0, 5.6]).as_deref(), Some("70.0-210.0 mm f/4.0-5.6"));
        assert_eq!(lens_spec_name([0.0, 0.0, 0.0, 0.0]), None);
        assert_eq!(lens_spec_name([200.0, 80.0, 2.8, 2.8]), None);
        assert_eq!(lens_spec_name([f64::NAN, 50.0, 1.8, 1.8]), None);
    }

    #[test]
    fn lens_spec_names_are_recognised() {
        for spec in [[50.0, 50.0, 1.8, 1.8], [18.0, 135.0, 3.5, 5.6]] {
            assert!(is_lens_spec_name(&lens_spec_name(spec).unwrap()));
        }
        assert!(is_lens_spec_name(" 24.0-70.0 mm f/2.8 "));
        for not in ["EF 50mm f/1.8", "50 mm f/", "0 mm f/2.8", "50-60-70 mm f/2.8", "nan mm f/2", "50 mm f/1.8-"] {
            assert!(!is_lens_spec_name(not), "{not}");
        }
    }

    #[test]
    fn from_embedded_preserves_exif_iptc_xmp_precedence() {
        let exif = Metadata {
            make: Some("Example".into()),
            model: Some("Camera".into()),
            lens_model: Some("35mm F2".into()),
            artist: Some("EXIF artist".into()),
            orientation: Some(Orientation::from_exif(6)),
            exposure_time: Some(1.0 / 125.0),
            ..Default::default()
        };
        let packet = write_xmp(&Metadata {
            artist: Some("XMP artist".into()),
            caption: Some("XMP caption".into()),
            rating: Some(4),
            keywords: vec!["xmp-keyword".into()],
            ..Default::default()
        });
        let mut iptc = Vec::new();
        for (dataset, value) in [(5u8, "IPTC title"), (120u8, "IPTC caption")] {
            iptc.extend_from_slice(&[0x1c, 2, dataset]);
            iptc.extend_from_slice(&u16::try_from(value.len()).unwrap().to_be_bytes());
            iptc.extend_from_slice(value.as_bytes());
        }
        let blocks = Embedded { exif: Some(write_exif(&exif)), iptc: Some(iptc), xmp: Some(packet.clone()), ..Default::default() };
        let parsed = from_embedded(&blocks);
        assert_eq!(parsed.make, exif.make);
        assert_eq!(parsed.model, exif.model);
        assert_eq!(parsed.lens_model, exif.lens_model);
        assert_eq!(parsed.orientation, exif.orientation);
        assert_eq!(parsed.exposure_time, exif.exposure_time);
        assert_eq!(parsed.title.as_deref(), Some("IPTC title"));
        assert_eq!(parsed.caption.as_deref(), Some("XMP caption"));
        assert_eq!(parsed.artist.as_deref(), Some("XMP artist"));
        assert_eq!(parsed.rating, Some(4));
        assert_eq!(parsed.keywords, vec!["xmp-keyword"]);
        assert_eq!(blocks.xmp.as_deref(), Some(packet.as_str()));
    }

    #[test]
    fn from_embedded_ignores_damaged_blocks() {
        let blocks = Embedded { exif: Some(vec![1, 2, 3]), iptc: Some(vec![0x1c, 2]), xmp: Some("<broken".into()), ..Default::default() };
        assert_eq!(from_embedded(&blocks), Metadata::default());
    }

    #[test]
    fn numbers() {
        assert_eq!(parse_number("28/10"), Some(2.8));
        assert_eq!(parse_number(" +0.7 "), Some(0.7));
        assert_eq!(parse_number("1/0"), None);
        assert_eq!(parse_number("abc"), None);
        assert_eq!(parse_number("inf"), None);
    }

    #[test]
    fn exposure_time_formats_reciprocals_and_significant_digits() {
        for (t, want) in [
            (1.0 / 3.0, "1/3"),
            (1.0 / 250.0, "1/250"),
            (1.0 / 8000.0, "1/8000"),
            (0.5, "1/2"),
            (0.3, "0.3"),
            (0.4, "0.4"),
            (1.0 / 2.8, "0.3571"),
            (0.625, "0.625"),
            (0.003, "0.003"),
            (1.0, "1"),
            (1.3, "1.3"),
            (2.5, "2.5"),
            (30.0, "30"),
            (9.99996, "10"),
            (1800.0, "1800"),
        ] {
            assert_eq!(format_exposure_time(t).as_deref(), Some(want), "{t}");
        }
        for t in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(format_exposure_time(t), None, "{t}");
        }
    }

    #[test]
    fn exposure_time_reciprocal_tolerance() {
        // a camera's decimal rational for 1/3 s, and times just inside the tolerance either side
        assert_eq!(format_exposure_time(3333.0 / 10000.0).as_deref(), Some("1/3"));
        // a microsecond-level rational error still reads as 1/8000; above ~1/1000 s every time is
        // within the tolerance of some 1/n, which is how cameras show those speeds anyway
        assert_eq!(format_exposure_time(1.0 / 8000.0 * (1.0 + 1e-5)).as_deref(), Some("1/8000"));
        for n in [3.0, 10.0, 250.0] {
            let inside = 0.9 * RECIPROCAL_TOLERANCE;
            let outside = 1.1 * RECIPROCAL_TOLERANCE;
            let want = format!("1/{n}");
            assert_eq!(format_exposure_time(1.0 / n * (1.0 + inside)).as_deref(), Some(want.as_str()), "{n} +inside");
            assert_eq!(format_exposure_time(1.0 / n * (1.0 - inside)).as_deref(), Some(want.as_str()), "{n} -inside");
            assert_ne!(format_exposure_time(1.0 / n * (1.0 + outside)).as_deref(), Some(want.as_str()), "{n} +outside");
            assert_ne!(format_exposure_time(1.0 / n * (1.0 - outside)).as_deref(), Some(want.as_str()), "{n} -outside");
        }
    }

    #[test]
    fn exposure_display() {
        let mut m = Metadata { exposure_time: Some(1.0 / 250.0), ..Default::default() };
        assert_eq!(m.exposure_display().as_deref(), Some("1/250"));
        m.exposure_time = Some(0.5);
        assert_eq!(m.exposure_display().as_deref(), Some("1/2"));
        m.exposure_time = Some(0.3);
        assert_eq!(m.exposure_display().as_deref(), Some("0.3"));
        m.exposure_time = Some(2.0);
        assert_eq!(m.exposure_display().as_deref(), Some("2″"));
        m.exposure_time = Some(0.0);
        assert_eq!(m.exposure_display(), None);
    }

    #[test]
    fn merge_rules() {
        let mut a = Metadata { make: Some("A".into()), title: Some("Original".into()), rating: Some(1), ..Default::default() };
        let b = Metadata {
            make: Some("B".into()),
            model: Some("M".into()),
            title: Some("Replacement".into()),
            caption: Some("Caption".into()),
            rating: Some(4),
            keywords: vec!["k".into()],
            ..Default::default()
        };
        a.fill_missing(&b);
        assert_eq!(a.make.as_deref(), Some("A"));
        assert_eq!(a.model.as_deref(), Some("M"));
        assert_eq!(a.title.as_deref(), Some("Original"));
        assert_eq!(a.caption.as_deref(), Some("Caption"));
        assert_eq!(a.keywords, vec!["k".to_string()]);
        assert_eq!(a.rating, Some(1));
        a.overlay_user_fields(&b);
        assert_eq!(a.rating, Some(4));
        assert_eq!(a.make.as_deref(), Some("A"));
        assert_eq!(a.keywords, vec!["k".to_string()]);
    }

    #[test]
    fn names() {
        let m = Metadata { metering_mode: Some(5), exposure_program: Some(3), ..Default::default() };
        assert_eq!(m.metering_name(), Some("Pattern"));
        assert_eq!(m.exposure_program_name(), Some("Aperture-priority AE"));
    }

    /// An MWG regions packet with one face, its box centred at (0.25, 0.4) and 0.1 × 0.15 of `AppliedToDimensions`
    /// `w` × `h`; `extra` goes on the region (e.g. Lightroom's `mwg-rs:Rotation`).
    fn regions_packet(w: u32, h: u32, extra: &str) -> String {
        format!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description rdf:about="" xmlns:mwg-rs="http://www.metadataworkinggroup.com/schemas/regions/"
                xmlns:stArea="http://ns.adobe.com/xmp/sType/Area#" xmlns:stDim="http://ns.adobe.com/xap/1.0/sType/Dimensions#">
              <mwg-rs:Regions rdf:parseType="Resource">
                <mwg-rs:AppliedToDimensions rdf:parseType="Resource" stDim:w="{w}" stDim:h="{h}" stDim:unit="pixel"/>
                <mwg-rs:RegionList><rdf:Bag>
                  <rdf:li rdf:parseType="Resource" mwg-rs:Name="Hannah" mwg-rs:Type="Face"{extra}>
                    <mwg-rs:Area rdf:parseType="Resource" stArea:x="0.25" stArea:y="0.4" stArea:w="0.1" stArea:h="0.15" stArea:unit="normalized"/>
                  </rdf:li>
                </rdf:Bag></mwg-rs:RegionList>
              </mwg-rs:Regions>
            </rdf:Description></rdf:RDF></x:xmpmeta>"#
        )
    }

    fn near(r: Rect, b: [f64; 4]) -> bool {
        [r.x0 - b[0], r.y0 - b[1], r.x1 - b[2], r.y1 - b[3]].iter().all(|d| d.abs() < 1e-9)
    }

    /// The face's box as drawn: on the photo as stored, or on the upright one by apps that draw there.
    const DRAWN: [f64; 4] = [0.2, 0.325, 0.3, 0.475];
    /// The same box on the photo turned upright by Exif orientation 6 (a quarter turn clockwise).
    const TURNED_6: [f64; 4] = [0.525, 0.2, 0.675, 0.3];

    #[test]
    fn boxes_drawn_on_the_stored_photo_turn_with_its_orientation() {
        let region = || Region {
            rect: Rect { x0: DRAWN[0], y0: DRAWN[1], x1: DRAWN[2], y1: DRAWN[3] },
            kind: RegionKind::Face,
            name: None,
            description: None,
        };
        let turned = |o: Orientation, drawn_on: (f64, f64), stored: (u32, u32)| {
            let mut regions = [region()];
            let did = upright_regions(&mut regions, drawn_on, o, stored);
            (did, regions[0].rect)
        };
        // drawn on the 6000 × 4000 photo as stored: every quarter turn and its mirror image land elsewhere
        for (o, want) in [
            (Orientation::Rotate90, TURNED_6),
            (Orientation::Rotate270, [0.325, 0.7, 0.475, 0.8]),
            (Orientation::Transpose, [0.325, 0.2, 0.475, 0.3]),
            (Orientation::Transverse, [0.525, 0.7, 0.675, 0.8]),
        ] {
            let (did, r) = turned(o, (6000.0, 4000.0), (6000, 4000));
            assert!(did && near(r, want), "{o:?}: {r:?}");
            // a smaller copy of the photo as stored is the same frame
            assert!(near(turned(o, (1500.0, 1000.0), (6000, 4000)).1, want), "{o:?} on a smaller size");
        }
        // drawn on the upright photo (MWG's own frame), or where the shape can't tell the two apart: as they are
        for (o, drawn_on, stored) in [
            (Orientation::Rotate90, (4000.0, 6000.0), (6000, 4000)),
            (Orientation::Rotate90, (4000.0, 4000.0), (6000, 4000)),
            (Orientation::Rotate90, (4000.0, 4000.0), (4000, 4000)),
            (Orientation::Rotate180, (6000.0, 4000.0), (6000, 4000)),
            (Orientation::FlipH, (6000.0, 4000.0), (6000, 4000)),
            (Orientation::Normal, (6000.0, 4000.0), (6000, 4000)),
        ] {
            let (did, r) = turned(o, drawn_on, stored);
            assert!(!did && near(r, DRAWN), "{o:?} drawn on {drawn_on:?} of {stored:?}: {r:?}");
        }
    }

    /// An app that doesn't say how the photo was turned (no `mwg-rs:Rotation`) drew the faces of a photo
    /// with Exif orientation 6 on its stored pixels, with the stored 6000 × 4000 as `AppliedToDimensions`. The boxes
    /// land on the faces of the upright photo, not where the stored photo had them.
    #[test]
    fn regions_drawn_on_the_stored_photo_are_read_onto_the_upright_one() {
        let jpeg = |packet: &str| {
            let mut j = vec![0xff, 0xd8];
            let mut seg = |payload: Vec<u8>| {
                j.extend([0xff, 0xe1]);
                j.extend(u16::try_from(payload.len() + 2).unwrap().to_be_bytes());
                j.extend(payload);
            };
            // orientation 6, 6000 × 4000 as stored
            seg([b"Exif\0\0".as_slice(), &crate::exif::tests::sample_exif(craft_tiff::ByteOrder::Little)].concat());
            seg([b"http://ns.adobe.com/xap/1.0/\0".as_slice(), packet.as_bytes()].concat());
            j.extend([0xff, 0xd9]);
            j
        };
        let read = |packet: &str| {
            let m = extract(&jpeg(packet));
            assert_eq!((m.orientation, m.width, m.height), (Some(Orientation::Rotate90), Some(6000), Some(4000)));
            assert_eq!(from_embedded(&embedded(&jpeg(packet))).regions, m.regions);
            assert_eq!(m.regions.len(), 1);
            m.regions[0].rect
        };
        let stored = read(&regions_packet(6000, 4000, ""));
        assert!(near(stored, TURNED_6), "drawn on the photo as stored: {stored:?}");
        let upright = read(&regions_packet(4000, 6000, ""));
        assert!(near(upright, DRAWN), "drawn on the upright photo, MWG's way: {upright:?}");
        // Lightroom's boxes say how the photo was turned: turned once, by that
        let lightroom = read(&regions_packet(6000, 4000, r#" mwg-rs:Rotation="-1.57080""#));
        assert!(near(lightroom, TURNED_6), "Lightroom's −π/2: {lightroom:?}");
    }

    /// A size only the XMP states can't tell how the photo is stored: `tiff:ImageWidth` and `tiff:ImageLength` of the
    /// upright photo have the shape of boxes drawn on it upright, and those must not be turned.
    #[test]
    fn a_size_only_the_xmp_states_turns_no_regions() {
        // orientation 6, no pixel size
        let exif = write_exif(&Metadata { orientation: Some(Orientation::Rotate90), ..Default::default() });
        let exif = strip_exif_header(&exif);
        let packet = regions_packet(4000, 6000, "")
            .replace(r#"rdf:about="""#, r#"rdf:about="" xmlns:tiff="http://ns.adobe.com/tiff/1.0/" tiff:ImageWidth="4000" tiff:ImageLength="6000""#);
        let mut j = vec![0xff, 0xd8];
        let mut seg = |payload: Vec<u8>| {
            j.extend([0xff, 0xe1]);
            j.extend(u16::try_from(payload.len() + 2).unwrap().to_be_bytes());
            j.extend(payload);
        };
        seg([b"Exif\0\0".as_slice(), exif].concat());
        seg([b"http://ns.adobe.com/xap/1.0/\0".as_slice(), packet.as_bytes()].concat());
        j.extend([0xff, 0xd9]);
        let m = extract(&j);
        assert_eq!((m.orientation, m.width, m.height), (Some(Orientation::Rotate90), Some(4000), Some(6000)));
        assert_eq!(m.regions.len(), 1);
        assert!(near(m.regions[0].rect, DRAWN), "{:?}", m.regions[0].rect);
        assert_eq!(from_embedded(&embedded(&j)).regions, m.regions);
        // a TIFF-based file: the whole file is the Exif block, with the XMP in its first IFD
        let mut ifd0 = craft_tiff::IfdBuilder::new();
        ifd0.set(craft_tiff::tags::ORIENTATION, craft_tiff::Value::Short(vec![6]));
        ifd0.set(craft_tiff::tags::XMP, craft_tiff::Value::Byte(packet.into_bytes()));
        let m = extract(&craft_tiff::TiffWriter::default().write(&[ifd0]).unwrap());
        assert_eq!((m.orientation, m.width, m.height), (Some(Orientation::Rotate90), Some(4000), Some(6000)));
        assert!(near(m.regions[0].rect, DRAWN), "TIFF: {:?}", m.regions[0].rect);
    }

    /// A photo whose Exif has a size but no orientation, and whose XMP says `tiff:Orientation=6`: a viewer
    /// displays it by the Exif alone, so it stays as stored, and boxes drawn on that stored frame are right as
    /// written. Turning them by an orientation only the XMP states put them where the photo isn't.
    #[test]
    fn an_orientation_only_the_xmp_states_turns_no_regions() {
        // 6000 × 4000 in the Exif, and no orientation
        let size_ifd = || {
            let mut sub = craft_tiff::IfdBuilder::new();
            sub.set(craft_tiff::tags::PIXEL_X_DIMENSION, craft_tiff::Value::Long(vec![6000]));
            sub.set(craft_tiff::tags::PIXEL_Y_DIMENSION, craft_tiff::Value::Long(vec![4000]));
            sub
        };
        let mut exif_ifd0 = craft_tiff::IfdBuilder::new();
        exif_ifd0.set_child(craft_tiff::tags::EXIF_IFD, size_ifd());
        let exif = craft_tiff::TiffWriter::new(craft_tiff::ByteOrder::Little, false).write(&[exif_ifd0]).unwrap();
        let packet = regions_packet(6000, 4000, "")
            .replace(r#"rdf:about="""#, r#"rdf:about="" xmlns:tiff="http://ns.adobe.com/tiff/1.0/" tiff:Orientation="6""#);
        let mut j = vec![0xff, 0xd8];
        let mut seg = |payload: Vec<u8>| {
            j.extend([0xff, 0xe1]);
            j.extend(u16::try_from(payload.len() + 2).unwrap().to_be_bytes());
            j.extend(payload);
        };
        seg([b"Exif\0\0".as_slice(), &exif].concat());
        seg([b"http://ns.adobe.com/xap/1.0/\0".as_slice(), packet.as_bytes()].concat());
        j.extend([0xff, 0xd9]);
        let m = extract(&j);
        assert_eq!((m.width, m.height), (Some(6000), Some(4000)));
        assert_eq!(m.orientation, Some(Orientation::Rotate90), "the XMP's orientation is still reported");
        assert_eq!(m.regions.len(), 1);
        assert!(near(m.regions[0].rect, DRAWN), "turned by an orientation the Exif doesn't state: {:?}", m.regions[0].rect);
        assert_eq!(from_embedded(&embedded(&j)).regions, m.regions);
        // a TIFF-based file: the first IFD has no orientation, its XMP (tag 700) has
        let mut ifd0 = craft_tiff::IfdBuilder::new();
        ifd0.set(craft_tiff::tags::XMP, craft_tiff::Value::Byte(packet.into_bytes()));
        ifd0.set_child(craft_tiff::tags::EXIF_IFD, size_ifd());
        let m = extract(&craft_tiff::TiffWriter::default().write(&[ifd0]).unwrap());
        assert_eq!(m.orientation, Some(Orientation::Rotate90));
        assert!(near(m.regions[0].rect, DRAWN), "TIFF: {:?}", m.regions[0].rect);
    }

    const JPEG_XMP_SIG: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";

    const JPEG_XMP_EXT_SIG: &[u8] = b"http://ns.adobe.com/xmp/extension/\0";

    fn jpeg_segment(marker: u8, payload: &[u8]) -> Vec<u8> {
        let len = u16::try_from(payload.len() + 2).expect("test APP1 payload fits u16");
        let mut segment = vec![0xff, marker];
        segment.extend_from_slice(&len.to_be_bytes());
        segment.extend_from_slice(payload);
        segment
    }

    fn xmp_jpeg(standard: &str, guid: &str, extension: &str, chunks: &[(u32, &[u8])]) -> Vec<u8> {
        let mut jpeg = vec![0xff, 0xd8];
        let mut packet = JPEG_XMP_SIG.to_vec();
        packet.extend_from_slice(standard.as_bytes());
        jpeg.extend(jpeg_segment(0xe1, &packet));
        for (offset, data) in chunks {
            let mut payload = JPEG_XMP_EXT_SIG.to_vec();
            payload.extend_from_slice(guid.as_bytes());
            payload.extend_from_slice(&(extension.len() as u32).to_be_bytes());
            payload.extend_from_slice(&offset.to_be_bytes());
            payload.extend_from_slice(data);
            jpeg.extend(jpeg_segment(0xe1, &payload));
        }
        jpeg
    }

    fn append_extended_chunk(jpeg: &mut Vec<u8>, guid: &str, total_length: u32, offset: u32, data: &[u8]) {
        let mut payload = JPEG_XMP_EXT_SIG.to_vec();
        payload.extend_from_slice(guid.as_bytes());
        payload.extend_from_slice(&total_length.to_be_bytes());
        payload.extend_from_slice(&offset.to_be_bytes());
        payload.extend_from_slice(data);
        jpeg.extend(jpeg_segment(0xe1, &payload));
    }

    fn xmp_document(body: &str) -> String {
        format!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" {body}></rdf:Description></rdf:RDF></x:xmpmeta>"#
        )
    }

    fn referenced_standard(guid: &str, extra: &str) -> String {
        xmp_document(&format!(r#"xmlns:reference="http://ns.adobe.com/xmp/note/" reference:HasExtendedXMP="{guid}" {extra}"#))
    }

    fn split_chunks(packet: &str) -> [(u32, &[u8]); 2] {
        let split = packet.len() / 2;
        let (first, second) = packet.as_bytes().split_at(split);
        [(split as u32, second), (0, first)]
    }

    fn raf_with_jpeg(jpeg: &[u8]) -> Vec<u8> {
        let offset = 120u32;
        let length = u32::try_from(jpeg.len()).expect("test JPEG fits RAF length");
        let mut raf = b"FUJIFILMCCD-RAW 0201FF000000TEST".to_vec();
        raf.resize(offset as usize, 0);
        raf[84..88].copy_from_slice(&offset.to_be_bytes());
        raf[88..92].copy_from_slice(&length.to_be_bytes());
        raf.extend_from_slice(jpeg);
        raf
    }

    #[test]
    fn extended_xmp_gps_reaches_extractor() {
        let guid = "A1234567890123456789012345678901";
        let extension = xmp_document(r#"xmlns:exif="http://ns.adobe.com/exif/1.0/" exif:GPSLatitude="45,0N" exif:GPSLongitude="7,0E""#);
        let standard = referenced_standard(guid, "");
        let jpeg = xmp_jpeg(&standard, guid, &extension, &split_chunks(&extension));
        let gps = extract(&jpeg).gps.expect("the extension GPS reaches extraction");
        assert_eq!((gps.latitude, gps.longitude), (45.0, 7.0));
    }

    #[test]
    fn extended_xmp_caption_reaches_extractor() {
        let guid = "B1234567890123456789012345678901";
        let caption = "multi-chunk caption ".repeat(160);
        let extension = xmp_document(&format!(r#"xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:description>{caption}</dc:description>"#));
        let standard = referenced_standard(guid, "");
        let jpeg = xmp_jpeg(&standard, guid, &extension, &split_chunks(&extension));
        assert_eq!(extract(&jpeg).caption.as_deref(), Some(caption.trim()));
    }

    #[test]
    fn extended_xmp_split_gps_properties_are_combined() {
        let guid = "C1234567890123456789012345678901";
        let standard = xmp_document(&format!(
            r#"xmlns:note="http://ns.adobe.com/xmp/note/" xmlns:exif="http://ns.adobe.com/exif/1.0/" note:HasExtendedXMP="{guid}" exif:GPSLatitude="45,0N""#
        ));
        let extension = xmp_document(r#"xmlns:exif="http://ns.adobe.com/exif/1.0/" exif:GPSLongitude="7,0E""#);
        let jpeg = xmp_jpeg(&standard, guid, &extension, &split_chunks(&extension));
        let gps = extract(&jpeg).gps.expect("GPS properties across packets compose");
        assert_eq!((gps.latitude, gps.longitude), (45.0, 7.0));
    }

    #[test]
    fn extended_xmp_structured_values_are_composed() {
        let guid = "F1234567890123456789012345678901";
        let standard = xmp_document(&format!(
            r#"xmlns:note="http://ns.adobe.com/xmp/note/" xmlns:exif="http://ns.adobe.com/exif/1.0/" note:HasExtendedXMP="{guid}"><exif:Flash rdf:parseType="Resource"><exif:Fired>True</exif:Fired></exif:Flash>"#
        ));
        let extension =
            xmp_document(r#"xmlns:exif="http://ns.adobe.com/exif/1.0/"><exif:Flash rdf:parseType="Resource"><exif:Mode>2</exif:Mode></exif:Flash>"#);
        let jpeg = xmp_jpeg(&standard, guid, &extension, &split_chunks(&extension));
        assert_eq!(extract(&jpeg).flash.map(|flash| flash.raw), Some(17));
    }

    #[test]
    fn extended_xmp_ignores_malformed_chunks_for_unreferenced_guids() {
        let guid = "A1234567890123456789012345678901";
        let other_guid = "B1234567890123456789012345678901";
        let extension = xmp_document(r#"xmlns:dc="http://purl.org/dc/elements/1.1/" dc:title="Referenced title""#);
        let mut jpeg = xmp_jpeg(&referenced_standard(guid, ""), guid, &extension, &split_chunks(&extension));
        let mut malformed = JPEG_XMP_EXT_SIG.to_vec();
        malformed.extend_from_slice(other_guid.as_bytes());
        jpeg.extend(jpeg_segment(0xe1, &malformed));

        assert_eq!(extract(&jpeg).title.as_deref(), Some("Referenced title"));
        assert_eq!(embedded_xmp(&embedded(&jpeg)).unwrap().unwrap().metadata.title.as_deref(), Some("Referenced title"));
    }

    #[test]
    fn extended_xmp_ignores_chunk_limit_for_unreferenced_guids() {
        let guid = "C1234567890123456789012345678901";
        let other_guid = "D1234567890123456789012345678901";
        let extension = xmp_document(r#"xmlns:dc="http://purl.org/dc/elements/1.1/" dc:title="Referenced title""#);
        let mut jpeg = xmp_jpeg(&referenced_standard(guid, ""), guid, &extension, &split_chunks(&extension));
        for _ in 0..4097 {
            append_extended_chunk(&mut jpeg, other_guid, 1, 0, &[]);
        }

        assert_eq!(embedded_xmp(&embedded(&jpeg)).unwrap().unwrap().metadata.title.as_deref(), Some("Referenced title"));
    }

    #[test]
    fn extended_xmp_ignores_received_byte_limit_for_unreferenced_guids() {
        let guid = "E1234567890123456789012345678901";
        let other_guid = "F1234567890123456789012345678901";
        let extension = xmp_document(r#"xmlns:dc="http://purl.org/dc/elements/1.1/" dc:title="Referenced title""#);
        let mut jpeg = xmp_jpeg(&referenced_standard(guid, ""), guid, &extension, &split_chunks(&extension));
        let data = vec![0; 65_200];
        for _ in 0..258 {
            append_extended_chunk(&mut jpeg, other_guid, 65_200, 0, &data);
        }

        assert_eq!(embedded_xmp(&embedded(&jpeg)).unwrap().unwrap().metadata.title.as_deref(), Some("Referenced title"));
    }

    #[test]
    fn extended_xmp_rejects_mismatched_guid_and_bad_coverage() {
        let guid = "D1234567890123456789012345678901";
        let extension = xmp_document(r#"xmlns:dc="http://purl.org/dc/elements/1.1/" dc:title="Extension title""#);
        let standard = referenced_standard(guid, r#"xmlns:dc="http://purl.org/dc/elements/1.1/" dc:title="Standard title""#);
        let bytes = extension.as_bytes();
        let middle = bytes.len() / 2;
        let inconsistent_lengths = {
            let mut jpeg = vec![0xff, 0xd8];
            let mut standard_payload = JPEG_XMP_SIG.to_vec();
            standard_payload.extend_from_slice(standard.as_bytes());
            jpeg.extend(jpeg_segment(0xe1, &standard_payload));
            for (offset, declared, part) in
                [(0u32, bytes.len() as u32, &bytes[..middle]), (middle as u32, (bytes.len() + 1) as u32, &bytes[middle..])]
            {
                let mut payload = JPEG_XMP_EXT_SIG.to_vec();
                payload.extend_from_slice(guid.as_bytes());
                payload.extend_from_slice(&declared.to_be_bytes());
                payload.extend_from_slice(&offset.to_be_bytes());
                payload.extend_from_slice(part);
                jpeg.extend(jpeg_segment(0xe1, &payload));
            }
            jpeg
        };
        let oversized_length = {
            let mut jpeg = vec![0xff, 0xd8];
            let mut standard_payload = JPEG_XMP_SIG.to_vec();
            standard_payload.extend_from_slice(standard.as_bytes());
            jpeg.extend(jpeg_segment(0xe1, &standard_payload));
            let mut payload = JPEG_XMP_EXT_SIG.to_vec();
            payload.extend_from_slice(guid.as_bytes());
            payload.extend_from_slice(&(16u32 * 1024 * 1024 + 1).to_be_bytes());
            payload.extend_from_slice(&0u32.to_be_bytes());
            payload.extend_from_slice(bytes);
            jpeg.extend(jpeg_segment(0xe1, &payload));
            jpeg
        };
        let foreign_namespace_reference = {
            let foreign_standard = xmp_document(&format!(
                r#"xmlns:note="urn:example:foreign-note" note:HasExtendedXMP="{guid}" xmlns:dc="http://purl.org/dc/elements/1.1/" dc:title="Standard title""#
            ));
            xmp_jpeg(&foreign_standard, guid, &extension, &split_chunks(&extension))
        };
        let mut truncated_segment = xmp_jpeg(&standard, guid, &extension, &split_chunks(&extension));
        truncated_segment.pop();

        let cases = [
            xmp_jpeg(&standard, "E1234567890123456789012345678901", &extension, &split_chunks(&extension)),
            xmp_jpeg(&standard, guid, &extension, &[(0, &bytes[..middle]), ((middle + 1) as u32, &bytes[middle..])]),
            xmp_jpeg(&standard, guid, &extension, &[(0, &bytes[..middle]), ((middle - 1) as u32, &bytes[middle..])]),
            xmp_jpeg(&standard, guid, &extension, &[(0, bytes), (0, bytes)]),
            xmp_jpeg(&standard, guid, &extension, &[(0, &bytes[..middle])]),
            inconsistent_lengths,
            oversized_length,
            truncated_segment,
            foreign_namespace_reference,
        ];
        for (index, jpeg) in cases.iter().enumerate() {
            // the last case links nothing (a foreign HasExtendedXMP): its chunks stay visible as an unlinked
            // extension, as before, and embedded_xmp still refuses it below
            if index + 1 < cases.len() {
                assert!(embedded(jpeg).xmp_extended.is_none(), "malformed extension case {index} was exposed");
            }
            assert_eq!(extract(jpeg).title.as_deref(), Some("Standard title"), "standard XMP remains readable");
            assert!(embedded_xmp(&embedded(jpeg)).is_err(), "embedded_xmp must reject malformed extension case {index}");
            let error = embedded_xmp(&embedded(jpeg)).expect_err("invalid referenced extension is reported");
            assert_eq!(error.standard.as_deref().and_then(|xmp| xmp.metadata.title.as_deref()), Some("Standard title"));
        }
    }

    #[test]
    fn standard_only_jpeg_xmp_still_extracts() {
        let standard = xmp_document(r#"xmlns:dc="http://purl.org/dc/elements/1.1/" dc:title="Standard-only title""#);
        let mut jpeg = vec![0xff, 0xd8];
        let mut packet = JPEG_XMP_SIG.to_vec();
        packet.extend_from_slice(standard.as_bytes());
        jpeg.extend(jpeg_segment(0xe1, &packet));
        assert_eq!(extract(&jpeg).title.as_deref(), Some("Standard-only title"));
    }

    #[test]
    fn embedded_xmp_rejects_malformed_xmp_but_extract_remains_best_effort() {
        let mut jpeg = vec![0xff, 0xd8];
        let mut packet = JPEG_XMP_SIG.to_vec();
        packet.extend_from_slice(b"<x:xmpmeta><broken");
        jpeg.extend(jpeg_segment(0xe1, &packet));

        assert_eq!(extract(&jpeg), Metadata::default(), "best-effort extraction ignores invalid XMP");
        assert!(embedded_xmp(&embedded(&jpeg)).is_err());
    }

    #[test]
    fn raf_extended_xmp_errors_remain_fail_closed() {
        let guid = "A1234567890123456789012345678901";
        let extension = xmp_document(r#"xmlns:dc="http://purl.org/dc/elements/1.1/" dc:title="Extended title""#);
        let standard = xmp_document(r#"xmlns:dc="http://purl.org/dc/elements/1.1/" dc:title="Standard title""#);
        let unreferenced_extension = raf_with_jpeg(&xmp_jpeg(&standard, guid, &extension, &split_chunks(&extension)));
        let missing_extension = raf_with_jpeg(&xmp_jpeg(&referenced_standard(guid, ""), guid, &extension, &[]));

        assert_eq!(extract(&unreferenced_extension).title.as_deref(), Some("Standard title"), "best-effort reads retain standard XMP");
        for (name, raf) in [("unreferenced extension", unreferenced_extension), ("missing referenced extension", missing_extension)] {
            assert!(embedded_xmp(&embedded(&raf)).is_err(), "{name} must retain the JPEG validation error");
            assert!(embedded_xmp(&embedded(&raf)).is_err(), "{name} must fail closed");
        }

        let valid = raf_with_jpeg(&xmp_jpeg(&referenced_standard(guid, ""), guid, &extension, &split_chunks(&extension)));
        assert_eq!(extract(&valid).title.as_deref(), Some("Extended title"));
    }
}
