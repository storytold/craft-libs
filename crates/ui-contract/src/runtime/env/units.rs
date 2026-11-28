//! `Units`: what typed number input and unit readouts need from the env (core 2.1): the locale's separators and the
//! unit spellings of a number's kind in the user's language, and the file sizes and durations written with them.

use super::{EnvValue, Label};
use craft_behavior::NumberCodec;
use craft_behavior::quantity::{Reader, is_unit_spelling};
use craft_i18n::{FileSize, Format, WordSet as _, Words};
use serde::{Deserialize, Serialize};
use std::ops::RangeInclusive;
use std::sync::{Arc, OnceLock};

craft_ui::words! {
    /// The unit words: `{n}` is where the number goes. The `_name` words name a unit in a list of units (a unit
    /// select), without a number.
    pub struct UnitWords {
        pixels: unit "units.pixels" = "{n} px";
        percent: unit "units.percent" = "{n}%";
        degrees: unit "units.degrees" = "{n}°";
        points: unit "units.points" = "{n} pt";
        times: unit "units.times" = "{n}×";
        inches: unit "units.inches" = "{n} in";
        centimetres: unit "units.centimetres" = "{n} cm";
        millimetres: unit "units.millimetres" = "{n} mm";
        picas: unit "units.picas" = "{n} pica";
        seconds: unit "units.seconds" = "{n} s";
        pixels_per_inch: unit "units.pixelsPerInch" = "{n} ppi";
        pixels_per_centimetre: unit "units.pixelsPerCentimetre" = "{n} ppcm";
        lines_per_inch: unit "units.linesPerInch" = "{n} lpi";
        lines_per_centimetre: unit "units.linesPerCentimetre" = "{n} lpcm";
        feet: unit "units.feet" = "{n} ft";
        yards: unit "units.yards" = "{n} yd";
        metres: unit "units.metres" = "{n} m";
        q: unit "units.q" = "{n} Q";
        ha: unit "units.ha" = "{n} H";
        letters: unit "units.letters" = "{n} letters";
        hyphens: unit "units.hyphens" = "{n} hyphens";
        minutes: unit "units.minutes" = "{n} min";
        columns: unit "units.columns" = "{n} col";
        per_inch: unit "units.perInch" = "{n}/in";
        milliseconds: unit "units.milliseconds" = "{n} ms";
        pixels_name: "units.pixelsName" = "Pixels";
        percent_name: "units.percentName" = "Percent";
        degrees_name: "units.degreesName" = "Degrees";
        points_name: "units.pointsName" = "Points";
        times_name: "units.timesName" = "Times";
        inches_name: "units.inchesName" = "Inches";
        centimetres_name: "units.centimetresName" = "Centimetres";
        millimetres_name: "units.millimetresName" = "Millimetres";
        picas_name: "units.picasName" = "Picas";
        pixels_per_inch_name: "units.pixelsPerInchName" = "Pixels/Inch";
        pixels_per_centimetre_name: "units.pixelsPerCentimetreName" = "Pixels/Centimetre";
        minutes_name: "units.minutesName" = "Minutes";
        columns_name: "units.columnsName" = "Columns";
        lines_per_inch_name: "units.linesPerInchName" = "Lines/Inch";
        lines_per_centimetre_name: "units.linesPerCentimetreName" = "Lines/Centimetre";
        feet_name: "units.feetName" = "Feet";
        yards_name: "units.yardsName" = "Yards";
        metres_name: "units.metresName" = "Metres";
        q_name: "units.qName" = "Q";
        ha_name: "units.haName" = "Ha";
        letters_name: "units.lettersName" = "Letters";
        hyphens_name: "units.hyphensName" = "Hyphens";
        per_inch_name: "units.perInchName" = "Per Inch";
        milliseconds_name: "units.millisecondsName" = "Milliseconds";
    }
}

/// How many percent make the whole of an extent.
const PERCENT: f64 = 100.0;

/// The size of a point, the Point/Pica Size of the Units preference. An app sets it once on its root env with
/// `RootEnv::values`; `Units` reads it for typed Points and Picas and for their conversion to other lengths.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, crate::contract::MemoProps)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum PointSize {
    /// 72 points to the inch.
    #[default]
    PostScript,
    /// 72.27 points to the inch.
    Traditional,
}

impl PointSize {
    /// How many points make an inch.
    pub const fn points_per_inch(self) -> f64 {
        match self {
            PointSize::PostScript => UnitKind::POINTS_PER_INCH,
            PointSize::Traditional => 72.27,
        }
    }
}

impl EnvValue for PointSize {
    fn root() -> Self {
        PointSize::default()
    }
}

/// How many decimals a zoom shows. An app sets it once on its root env with `RootEnv::values`; [`Units::zoom_codec`]
/// follows it, so every zoom display (a zoom field, the status bar's, a print preview's row) reads alike.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, crate::contract::MemoProps)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum ZoomPrecision {
    /// Up to two decimals, for the ladder's rungs such as 66.67 %.
    #[default]
    Fractional,
    /// A whole percent ("94 %"), as a word processor shows its zoom. The zoom itself keeps its fraction, and a typed
    /// fraction still applies; only the shown number is whole.
    Whole,
}

impl EnvValue for ZoomPrecision {
    fn root() -> Self {
        ZoomPrecision::default()
    }
}

impl EnvValue for craft_behavior::ZoomScheme {
    fn root() -> Self {
        craft_behavior::ZoomScheme::PHOTOSHOP
    }
}

/// What a number counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, craft_ui_macros::PlainData)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum UnitKind {
    /// `{n} px`.
    Pixels,
    /// `{n}%`.
    Percent,
    /// `{n}°`.
    Degrees,
    /// `{n} pt`.
    Points,
    /// `{n}×`.
    Times,
    /// `{n} in`.
    Inches,
    /// `{n} cm`.
    Centimetres,
    /// `{n} mm`.
    Millimetres,
    /// `{n} pica`.
    Picas,
    /// `{n} ppi`.
    PixelsPerInch,
    /// `{n} ppcm`.
    PixelsPerCentimetre,
    /// `{n} min`.
    Minutes,
    /// `{n} col`: a width in columns of a column size (`ColumnSize`), an affine rule no unit factor expresses.
    Columns,
    /// `{n} lpi`: a halftone screen's frequency.
    LinesPerInch,
    /// `{n} lpcm`: a halftone screen's frequency.
    LinesPerCentimetre,
    /// `{n} ft`.
    Feet,
    /// `{n} yd`.
    Yards,
    /// `{n} m`.
    Metres,
    /// `{n} Q`.
    Q,
    /// `{n} H`.
    Ha,
    /// `{n} letters`: a hyphenation setting counted in letters.
    Letters,
    /// `{n} hyphens`: a hyphenation setting counted in hyphens.
    Hyphens,
    /// `{n}/in`: how many of something fall in an inch of length, such as the points a distortion adds along a path.
    /// It is a count, not a length: [`per_inch`](Self::per_inch) answers `None` for it.
    PerInch,
    /// `{n} ms`: a duration or delay in milliseconds.
    Milliseconds,
}

impl UnitKind {
    /// The lengths a unit choice offers. [`per_inch`](Self::per_inch) answers for more kinds (feet, yards, metres, Q,
    /// Ha), which a typed number still reads.
    pub const LENGTHS: [UnitKind; 5] = [UnitKind::Inches, UnitKind::Centimetres, UnitKind::Millimetres, UnitKind::Points, UnitKind::Picas];

    /// The kind a [`Reader`](craft_behavior::Reader) tag names: the tags `reader_over` registers are indexes of `ALL`.
    #[doc(hidden)]
    pub fn from_tag(tag: u32) -> Option<UnitKind> {
        Self::ALL.get(tag as usize).copied()
    }

    const ALL: [UnitKind; 24] = [
        UnitKind::Pixels,
        UnitKind::Percent,
        UnitKind::Degrees,
        UnitKind::Points,
        UnitKind::Times,
        UnitKind::Inches,
        UnitKind::Centimetres,
        UnitKind::Millimetres,
        UnitKind::Picas,
        UnitKind::PixelsPerInch,
        UnitKind::PixelsPerCentimetre,
        UnitKind::Minutes,
        UnitKind::Columns,
        UnitKind::LinesPerInch,
        UnitKind::LinesPerCentimetre,
        UnitKind::Feet,
        UnitKind::Yards,
        UnitKind::Metres,
        UnitKind::Q,
        UnitKind::Ha,
        UnitKind::Letters,
        UnitKind::Hyphens,
        UnitKind::PerInch,
        UnitKind::Milliseconds,
    ];

    /// The digits after the decimal mark a number of this kind shows by default, at most.
    pub const fn decimals(self) -> RangeInclusive<usize> {
        match self {
            UnitKind::Pixels | UnitKind::Minutes | UnitKind::Letters | UnitKind::Hyphens | UnitKind::Milliseconds => 0..=0,
            UnitKind::Percent | UnitKind::Degrees | UnitKind::Points => 0..=1,
            UnitKind::Q | UnitKind::Ha => 0..=2,
            UnitKind::Feet | UnitKind::Yards | UnitKind::Metres => 0..=4,
            UnitKind::Times
            | UnitKind::Centimetres
            | UnitKind::Millimetres
            | UnitKind::Picas
            | UnitKind::PixelsPerInch
            | UnitKind::PixelsPerCentimetre
            | UnitKind::LinesPerInch
            | UnitKind::LinesPerCentimetre
            | UnitKind::PerInch => 0..=2,
            UnitKind::Inches | UnitKind::Columns => 0..=3,
        }
    }

    /// What one arrow press, wheel notch or scrub point changes a number of this kind by, where the control that shows
    /// the kind steps by the kind's rule.
    pub const fn step(self) -> f64 {
        match self {
            UnitKind::Times | UnitKind::Inches | UnitKind::Centimetres | UnitKind::Feet | UnitKind::Yards | UnitKind::Metres => 0.1,
            _ => 1.0,
        }
    }

    /// The points in an inch of PostScript points, the default of Points and Picas.
    pub const POINTS_PER_INCH: f64 = 72.0;

    /// How many units of this kind make an inch, for the kinds that measure length; `points_per_inch` sets the size
    /// of Points and Picas (72, or 72.27 for traditional points). `None` for a kind that is not a length.
    pub fn per_inch(self, points_per_inch: f64) -> Option<f64> {
        match self {
            UnitKind::Inches => Some(1.0),
            UnitKind::Centimetres => Some(2.54),
            UnitKind::Millimetres => Some(25.4),
            UnitKind::Points => Some(points_per_inch),
            UnitKind::Picas => Some(points_per_inch / 12.0),
            UnitKind::Feet => Some(1.0 / 12.0),
            UnitKind::Yards => Some(1.0 / 36.0),
            UnitKind::Metres => Some(0.0254),
            UnitKind::Q | UnitKind::Ha => Some(101.6),
            _ => None,
        }
    }
}

impl UnitKind {
    /// The document numbers every conversion uses, so `from_pixels` and `to_pixels` stay exact inverses: the extent
    /// and the pixels per inch are at least 1.
    fn document_numbers(extent: f64, pixels_per_inch: f64) -> (f64, f64) {
        (extent.max(1.0), pixels_per_inch.max(1.0))
    }

    /// `pixels` in this unit: Pixels as they are, Percent of `extent` pixels, a length through `pixels_per_inch`
    /// (`extent` and `pixels_per_inch` clamped to at least 1; `points_per_inch` as in [`per_inch`](Self::per_inch)).
    /// `None` for a kind that is not a pixel or length unit.
    pub fn from_pixels(self, pixels: f64, extent: f64, pixels_per_inch: f64, points_per_inch: f64) -> Option<f64> {
        let (extent, resolution) = Self::document_numbers(extent, pixels_per_inch);
        Some(match self {
            UnitKind::Pixels => pixels,
            UnitKind::Percent => pixels / extent * PERCENT,
            _ => pixels / resolution * self.per_inch(points_per_inch)?,
        })
    }

    /// A value in this unit as pixels; the inverse of [`from_pixels`](Self::from_pixels).
    pub fn to_pixels(self, value: f64, extent: f64, pixels_per_inch: f64, points_per_inch: f64) -> Option<f64> {
        let (extent, resolution) = Self::document_numbers(extent, pixels_per_inch);
        Some(match self {
            UnitKind::Pixels => value,
            UnitKind::Percent => value * extent / PERCENT,
            _ => value / self.per_inch(points_per_inch)? * resolution,
        })
    }
}

/// The column size of a document: `n` columns are `n·(width + gutter) − gutter` points.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ColumnSize {
    /// The width of one column, in points; a value that is not a finite number above 0 counts as 180.
    pub width: f64,
    /// The gutter between columns, in points; a value that is not finite counts as 12, a negative one as 0.
    pub gutter: f64,
}

impl ColumnSize {
    /// The column size with columns `width` points wide `gutter` points apart.
    pub const fn new(width: f64, gutter: f64) -> Self {
        Self { width, gutter }
    }

    /// The column width used when the given one is not a finite number above 0.
    const FALLBACK_WIDTH: f64 = 180.0;
    /// The gutter used when the given one is not finite.
    const FALLBACK_GUTTER: f64 = 12.0;

    /// The width and gutter the conversion uses: the fallbacks the field docs name, so a degenerate preference
    /// never divides by zero.
    fn sane(self) -> (f64, f64) {
        let width = if self.width.is_finite() && self.width > 0.0 { self.width } else { Self::FALLBACK_WIDTH };
        let gutter = if self.gutter.is_finite() { self.gutter.max(0.0) } else { Self::FALLBACK_GUTTER };
        (width, gutter)
    }

    /// `columns` columns as points; a `relative` change converts the span of `|columns|` with its sign, and 0 stays 0.
    pub fn points(self, columns: f64, relative: bool) -> f64 {
        let (width, gutter) = self.sane();
        let span = |n: f64| n * (width + gutter) - gutter;
        if !relative {
            return span(columns);
        }
        if columns == 0.0 {
            return 0.0;
        }
        columns.signum() * span(columns.abs()).max(0.0)
    }

    /// `points` as columns; the inverse of [`points`](Self::points).
    pub fn columns(self, points: f64, relative: bool) -> f64 {
        let (width, gutter) = self.sane();
        let count = |p: f64| (p + gutter) / (width + gutter);
        if !relative {
            return count(points);
        }
        if points == 0.0 {
            return 0.0;
        }
        points.signum() * count(points.abs())
    }
}

/// A length kept in one unit and shown in another: what a length field reads and writes (`stored`, the document's
/// unit) and what it shows and, unless `bare` names another unit, parses bare numbers in (`shown`, the user's unit). Both are `UnitKind::Pixels` or any
/// kind [`UnitKind::per_inch`] answers; the document's resolution sizes pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct LengthUnits {
    /// The unit of the value the field's binding holds.
    pub stored: UnitKind,
    /// The unit the field shows.
    pub shown: UnitKind,
    /// Pixels per inch of the document, for `Pixels` on either side.
    pub pixels_per_inch: f64,
    /// The unit a number typed without a unit is in; `None` for `shown` ([`LengthUnits::bare_in`]).
    pub bare: Option<UnitKind>,
}

impl LengthUnits {
    /// A length kept in `stored` and shown in `shown`, with `pixels_per_inch` sizing pixels.
    pub const fn new(stored: UnitKind, shown: UnitKind, pixels_per_inch: f64) -> Self {
        LengthUnits { stored, shown, pixels_per_inch, bare: None }
    }

    /// A number typed without a unit is in `unit` rather than in the shown unit (a preference such as "numbers
    /// without units are points").
    #[must_use]
    pub const fn bare_in(mut self, unit: UnitKind) -> Self {
        self.bare = Some(unit);
        self
    }
}

/// The English unit words, built once for the process.
fn english_words() -> Arc<UnitWords> {
    static ENGLISH: OnceLock<Arc<UnitWords>> = OnceLock::new();
    ENGLISH.get_or_init(|| Arc::new(UnitWords::english())).clone()
}

/// Reads and writes numbers with their unit in the user's language and digits. Build it with `Env::units` or
/// `EnvView::units`.
#[derive(Clone, Debug)]
pub struct Units {
    format: Arc<Format>,
    words: Arc<UnitWords>,
    /// The scope's words, for the rows craft-i18n owns (file sizes).
    scope_words: Arc<Words>,
    /// The readers built so far, one slot per `reader` argument (see `reader_slot`); shared by clones.
    readers: Arc<[OnceLock<Reader>; UnitKind::ALL.len() + 1]>,
    /// The scope's point size: what `reader` and `points_per_inch` use for Points and Picas.
    point_size: PointSize,
    /// The decimals a zoom shows (`zoom_codec`).
    zoom: ZoomPrecision,
}

/// What every number it reads and writes depends on: the format, the unit words and the scope's words (each by its
/// generation) and the point size.
impl crate::contract::MemoProps for Units {
    fn key(&self, key: &mut crate::contract::MemoKey) {
        let Units { format, words, scope_words, readers: _, point_size, zoom } = self;
        crate::contract::MemoProps::key(&(format, words, scope_words, point_size, zoom), key);
    }
}

impl Units {
    /// The codec of a zoom field in percent: the percent reader, up to two decimals (the zoom ladder has rungs such as
    /// 66.67 %), with zoom typing (`a/b` as a ratio, a second separator cutting the text), so every zoom field reads
    /// typed text by the same rules. Under [`ZoomPrecision::Whole`] it writes whole percents.
    pub fn zoom_codec(&self) -> NumberCodec {
        NumberCodec::new(self.reader(Some(UnitKind::Percent)), self.zoom_decimals()).zoom_typing()
    }
}

impl Units {
    /// The most decimals a zoom shows: the env's [`ZoomPrecision`], shared by `zoom_codec` and `zoom_percent`.
    fn zoom_decimals(&self) -> usize {
        match self.zoom {
            ZoomPrecision::Fractional => ZOOM_DECIMALS,
            ZoomPrecision::Whole => 0,
        }
    }
}

/// The most decimals a zoom field shows.
const ZOOM_DECIMALS: usize = 2;

impl Units {
    #[doc(hidden)]
    pub fn new(format: Arc<Format>, words: Arc<UnitWords>, scope_words: Arc<Words>, point_size: PointSize) -> Units {
        Units { format, words, scope_words, readers: Arc::default(), point_size, zoom: ZoomPrecision::default() }
    }

    /// The same units with `zoom` as the decimals a zoom shows: the env's [`ZoomPrecision`].
    #[must_use]
    pub(crate) fn with_zoom_precision(mut self, zoom: ZoomPrecision) -> Self {
        self.zoom = zoom;
        self
    }

    /// How many points make an inch in the scope: the app's point-size preference.
    pub fn points_per_inch(&self) -> f64 {
        self.point_size.points_per_inch()
    }

    fn template(&self, kind: UnitKind) -> &str {
        let words = &*self.words;
        match kind {
            UnitKind::Pixels => words.pixels.as_str(),
            UnitKind::Percent => words.percent.as_str(),
            UnitKind::Degrees => words.degrees.as_str(),
            UnitKind::Points => words.points.as_str(),
            UnitKind::Times => words.times.as_str(),
            UnitKind::Inches => words.inches.as_str(),
            UnitKind::Centimetres => words.centimetres.as_str(),
            UnitKind::Millimetres => words.millimetres.as_str(),
            UnitKind::Picas => words.picas.as_str(),
            UnitKind::PixelsPerInch => words.pixels_per_inch.as_str(),
            UnitKind::PixelsPerCentimetre => words.pixels_per_centimetre.as_str(),
            UnitKind::Minutes => words.minutes.as_str(),
            UnitKind::Columns => words.columns.as_str(),
            UnitKind::LinesPerInch => words.lines_per_inch.as_str(),
            UnitKind::LinesPerCentimetre => words.lines_per_centimetre.as_str(),
            UnitKind::Feet => words.feet.as_str(),
            UnitKind::Yards => words.yards.as_str(),
            UnitKind::Metres => words.metres.as_str(),
            UnitKind::Q => words.q.as_str(),
            UnitKind::Ha => words.ha.as_str(),
            UnitKind::Letters => words.letters.as_str(),
            UnitKind::Hyphens => words.hyphens.as_str(),
            UnitKind::PerInch => words.per_inch.as_str(),
            UnitKind::Milliseconds => words.milliseconds.as_str(),
        }
    }

    /// The unit's name in the user's language, as a list of units shows it: Pixels, Pixels/Inch.
    pub fn name(&self, kind: UnitKind) -> Label {
        let words = &*self.words;
        let name = match kind {
            UnitKind::Pixels => &words.pixels_name,
            UnitKind::Percent => &words.percent_name,
            UnitKind::Degrees => &words.degrees_name,
            UnitKind::Points => &words.points_name,
            UnitKind::Times => &words.times_name,
            UnitKind::Inches => &words.inches_name,
            UnitKind::Centimetres => &words.centimetres_name,
            UnitKind::Millimetres => &words.millimetres_name,
            UnitKind::Picas => &words.picas_name,
            UnitKind::PixelsPerInch => &words.pixels_per_inch_name,
            UnitKind::PixelsPerCentimetre => &words.pixels_per_centimetre_name,
            UnitKind::Minutes => &words.minutes_name,
            UnitKind::Columns => &words.columns_name,
            UnitKind::LinesPerInch => &words.lines_per_inch_name,
            UnitKind::LinesPerCentimetre => &words.lines_per_centimetre_name,
            UnitKind::Feet => &words.feet_name,
            UnitKind::Yards => &words.yards_name,
            UnitKind::Metres => &words.metres_name,
            UnitKind::Q => &words.q_name,
            UnitKind::Ha => &words.ha_name,
            UnitKind::Letters => &words.letters_name,
            UnitKind::Hyphens => &words.hyphens_name,
            UnitKind::PerInch => &words.per_inch_name,
            UnitKind::Milliseconds => &words.milliseconds_name,
        };
        name.clone()
    }

    /// The unit's spelling in the user's language without the number: `mm`, `px`.
    pub fn symbol(&self, kind: UnitKind) -> String {
        self.template(kind).replace("{n}", "").trim().to_owned()
    }

    /// The text before and after the number in the user's language, spacing included: `("", " mm")`, `("", "%")`.
    pub fn affixes(&self, kind: UnitKind) -> (String, String) {
        let template = self.template(kind);
        template.split_once("{n}").map_or_else(|| (String::new(), template.to_owned()), |(before, after)| (before.to_owned(), after.to_owned()))
    }

    /// The digits after the decimal mark `n` needs within `decimals`: the fewest that show it as it rounds at the most.
    fn digits(n: f64, decimals: &RangeInclusive<usize>) -> usize {
        let (least, most) = (*decimals.start(), *decimals.end());
        let scale = 10f64.powi(i32::try_from(most).unwrap_or(0));
        let target = (n * scale).round();
        (least..=most)
            .find(|d| {
                let at = 10f64.powi(i32::try_from(*d).unwrap_or(0));
                ((n * at).round() / at * scale).round() == target
            })
            .unwrap_or(most)
    }

    /// `n` of `kind` with between the least and most digits of `decimals` after the decimal mark, in the user's
    /// language and digits.
    pub fn text(&self, kind: UnitKind, n: f64, decimals: RangeInclusive<usize>) -> String {
        let number = self.format.number_ungrouped(n, Self::digits(n, &decimals));
        craft_i18n::fmt(self.template(kind), &[("n", &number)])
    }

    /// `n` of `kind` with its default digits ([`UnitKind::decimals`]).
    pub fn show(&self, kind: UnitKind, n: f64) -> String {
        self.text(kind, n, kind.decimals())
    }

    /// `n` pixels: `12 px`.
    pub fn pixels(&self, n: f64) -> String {
        self.show(UnitKind::Pixels, n)
    }

    /// `n` percent: `12%`.
    pub fn percent(&self, n: f64) -> String {
        self.show(UnitKind::Percent, n)
    }

    /// A zoom of `n` percent: `100%`, `12.5%`, `66.67%`. The fewest digits that show it to two decimals, so the
    /// ladder's thirds read as Photoshop shows them. Under [`ZoomPrecision::Whole`] a whole percent.
    pub fn zoom_percent(&self, n: f64) -> String {
        self.text(UnitKind::Percent, n, 0..=self.zoom_decimals())
    }

    /// `n` degrees: `12°`.
    pub fn degrees(&self, n: f64) -> String {
        self.show(UnitKind::Degrees, n)
    }

    /// `n` times: `12×`.
    pub fn times(&self, n: f64) -> String {
        self.show(UnitKind::Times, n)
    }

    /// `seconds` as a duration: `0.35 s`, `12 s`, with at most two digits after the decimal mark.
    pub fn duration(&self, seconds: f64) -> String {
        let number = self.format.number_ungrouped(seconds, Self::digits(seconds, &(0..=2)));
        craft_i18n::fmt(self.words.seconds.as_str(), &[("n", &number)])
    }

    /// `bytes` as a file size in steps of 1024 with one decimal (`1.5 KB`, `12.0 MB`, `512 bytes`), the unit
    /// names and digits of the user's language.
    pub fn file_size(&self, bytes: u64) -> String {
        self.file_size_as(bytes, FileSize::BINARY)
    }

    /// `bytes` as a file size written the way `style` says.
    pub fn file_size_as(&self, bytes: u64, style: FileSize) -> String {
        style.text(bytes, &self.format, &self.scope_words)
    }

    /// `n` with digit groups and between the least and most digits of `decimals` after the decimal mark, in the
    /// user's language and digits, with no unit.
    pub fn number(&self, n: f64, decimals: RangeInclusive<usize>) -> String {
        self.format.number(n, Self::digits(n, &decimals))
    }

    /// `n` with between the least and most digits of `decimals` after the decimal mark, in the user's language and
    /// digits, with no digit groups and no unit.
    pub fn number_ungrouped(&self, n: f64, decimals: RangeInclusive<usize>) -> String {
        self.format.number_ungrouped(n, Self::digits(n, &decimals))
    }

    /// A reader for numbers of `kind` (plain numbers when `None`): the locale's decimal and group separators and
    /// the unit spellings that convert to `kind`, in the user's language and in English.
    pub fn reader(&self, kind: Option<UnitKind>) -> Reader {
        self.readers[Self::reader_slot(kind)].get_or_init(|| self.build_reader(kind, self.points_per_inch())).clone()
    }

    /// Like [`reader`](Units::reader), with Points and Picas of `points_per_inch` to the inch, so a typed `12pt` is the
    /// length a dialog's own point size converts.
    pub fn reader_with_points(&self, kind: Option<UnitKind>, points_per_inch: f64) -> Reader {
        if points_per_inch == self.points_per_inch() { self.reader(kind) } else { self.build_reader(kind, points_per_inch) }
    }

    /// How many units of `kind` make an inch, for Pixels (at `pixels_per_inch`) and the length kinds.
    fn length_per_inch(&self, kind: UnitKind, pixels_per_inch: f64) -> Option<f64> {
        let per = match kind {
            UnitKind::Pixels => Some(pixels_per_inch),
            kind => kind.per_inch(self.points_per_inch()),
        };
        per.filter(|per| per.is_finite() && *per > 0.0)
    }

    /// How many `lengths.shown` make one `lengths.stored`; `None` when either is not a pixel or length unit or the
    /// resolution is not a positive number.
    pub fn length_scale(&self, lengths: LengthUnits) -> Option<f64> {
        Some(self.length_per_inch(lengths.shown, lengths.pixels_per_inch)? / self.length_per_inch(lengths.stored, lengths.pixels_per_inch)?)
    }

    /// A reader for lengths shown in `lengths.shown`: the spellings of every length unit and of pixels convert to it,
    /// and a bare number is in it, or in `lengths.bare` when set. Plain numbers when `lengths` is not a pixel or length
    /// pair.
    pub fn length_reader(&self, lengths: LengthUnits) -> Reader {
        let Some(shown) = self.length_per_inch(lengths.shown, lengths.pixels_per_inch) else { return self.reader(None) };
        let factor = |other: UnitKind| Some(shown / self.length_per_inch(other, lengths.pixels_per_inch)?);
        let reader = self.reader_over(factor);
        match lengths.bare.and_then(factor) {
            Some(bare) => reader.bare(bare),
            None => reader,
        }
    }

    /// The index of the reader for `kind` in `readers`: 0 for plain numbers, then one per unit kind.
    fn reader_slot(kind: Option<UnitKind>) -> usize {
        kind.map_or(0, |kind| 1 + UnitKind::ALL.iter().position(|other| *other == kind).unwrap_or(0))
    }

    fn build_reader(&self, kind: Option<UnitKind>, points_per_inch: f64) -> Reader {
        let Some(kind) = kind else { return self.reader_over(|_| None) };
        self.reader_over(|other| match (kind.per_inch(points_per_inch), other.per_inch(points_per_inch)) {
            (Some(of), Some(per)) => Some(of / per),
            _ if other == kind => Some(1.0),
            _ => None,
        })
    }

    /// A reader for the numbers of a document measured in `ruler`: the unit spellings of every pixel and length unit
    /// (px, %, in, cm, mm, pt, pica) convert to `ruler` through the document's numbers, percent of `extent` pixels
    /// and lengths at `pixels_per_inch`, with points of `points_per_inch` to the inch (`UnitKind::to_pixels` clamps
    /// them). A bare number is in `ruler`.
    pub fn document_reader(&self, ruler: UnitKind, extent: f64, pixels_per_inch: f64, points_per_inch: f64) -> Reader {
        let pixels = |kind: UnitKind| kind.to_pixels(1.0, extent, pixels_per_inch, points_per_inch);
        let one_ruler = pixels(ruler).filter(|one| *one != 0.0);
        self.reader_over(|other| Some(pixels(other)? / one_ruler?))
    }

    /// The locale's separators and, for every unit kind `factor` gives a factor to, its spellings (in the user's
    /// language and in English) reading as that many of the reader's own unit.
    fn reader_over(&self, factor: impl Fn(UnitKind) -> Option<f64>) -> Reader {
        let (decimal, group) = self.format.separators();
        let mut reader = Reader::new().decimal(decimal);
        if let Some(group) = group {
            reader = reader.group(group);
        }
        let english = Units::new(self.format.clone(), english_words(), Arc::new(Words::english()), self.point_size);
        let points = factor(UnitKind::Points);
        for (tag, other) in UnitKind::ALL.into_iter().enumerate() {
            let Some(factor) = factor(other) else { continue };
            for spelling in [self.symbol(other), english.symbol(other)] {
                if is_unit_spelling(&spelling) {
                    reader = reader.unit_tagged(&spelling, factor, tag as u32);
                }
            }
            if other == UnitKind::Picas
                && let Some(points) = points
            {
                // Picas and points as print apps read them: `2p6` is two picas and six points.
                reader = reader.split_unit("p", factor, points, tag as u32);
            }
        }
        reader
    }
}
