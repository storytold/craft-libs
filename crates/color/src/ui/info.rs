//! [`InfoPanel`]: a read-only readout grid of the colour, position and size under the pointer.

use craft_behavior::ColumnWidth;
use craft_cms::convert::CmykConverter;
use craft_theme::Srgba;
use craft_theme::variant::Tone;
use craft_ui::contract::{Outcome as _, Pattern, Props as _, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::layout_components::{Grid, Stack};
use craft_ui::notices::{Notice, NoticeLayout};
use craft_ui::runtime::env::{Env, Label};
use egui::{Response, Ui};

use super::SamplerReadout;

#[cfg(feature = "screen-testing")]
mod stories;

/// What a missing value shows.
const DASH: &str = "–";

/// The grid's columns: a name and its value, twice, so that the two readouts of a line stand side by side and each
/// column lines up down the grid.
static COLUMNS: [ColumnWidth; 4] = [ColumnWidth::Content, ColumnWidth::Content, ColumnWidth::Content, ColumnWidth::Content];

craft_ui::words! {
    /// The words of the Info panel, in the installed language unless set.
    pub struct InfoWords {
        r: "info.r" = "R";
        g: "info.g" = "G";
        b: "info.b" = "B";
        c: "info.c" = "C";
        m: "info.m" = "M";
        y: "info.y" = "Y";
        k: "info.k" = "K";
        x: "info.x" = "X";
        pos_y: "info.posY" = "Y";
        w: "info.w" = "W";
        h: "info.h" = "H";
        /// The document size template.
        document: template "info.document" = "Doc: {width} x {height}";
        no_document: "info.noDocument" = "No document";
    }
}

/// A colour in 8-bit sRGB and CMYK percent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InfoColor {
    /// The colour in 8-bit sRGB.
    pub rgb: [u8; 3],
    /// The colour in CMYK percent.
    pub cmyk: [u8; 4],
}

impl InfoColor {
    /// `color` through the document's CMYK converter: the one sRGB-and-CMYK conversion, which `SamplerReadout::new` builds
    /// its `rgb` and `cmyk` through. A channel that is not finite reads as 0 and the rest is clamped.
    pub fn new(color: Srgba, cmyk: &CmykConverter) -> Self {
        let unit = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        let rgb = [color.r, color.g, color.b].map(unit);
        InfoColor { rgb: rgb.map(|v| (v * 255.0).round() as u8), cmyk: cmyk.from_srgb(rgb).map(|v| (unit(v) * 100.0).round() as u8) }
    }
}

/// What the Info panel shows.
#[non_exhaustive]
#[derive(Clone, Debug, Default)]
pub struct InfoReadings<'a> {
    /// The colour under the pointer.
    pub color: Option<InfoColor>,
    /// The pointer's position.
    pub position: Option<[Label; 2]>,
    /// The selection's extent.
    pub extent: Option<[Label; 2]>,
    /// The document's size; `None` is the no-document state.
    pub document: Option<[u32; 2]>,
    /// The colour samplers' readouts with their position labels.
    pub samplers: &'a [(SamplerReadout, [Label; 2])],
}

impl<'a> InfoReadings<'a> {
    /// Readings with nothing in them.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the colour.
    #[must_use]
    pub fn color(mut self, color: Option<InfoColor>) -> Self {
        self.color = color;
        self
    }

    /// Sets the position.
    #[must_use]
    pub fn position(mut self, position: Option<[Label; 2]>) -> Self {
        self.position = position;
        self
    }

    /// Sets the extent.
    #[must_use]
    pub fn extent(mut self, extent: Option<[Label; 2]>) -> Self {
        self.extent = extent;
        self
    }

    /// Sets the document size.
    #[must_use]
    pub fn document(mut self, document: Option<[u32; 2]>) -> Self {
        self.document = document;
        self
    }

    /// Sets the samplers.
    #[must_use]
    pub fn samplers(mut self, samplers: &'a [(SamplerReadout, [Label; 2])]) -> Self {
        self.samplers = samplers;
        self
    }
}

/// A read-only grid: RGB beside CMYK, X/Y beside W/H, the document size, then one row group per sampler; a dash for a
/// missing value; `document: None` shows "No document" as a centred notice in place of the grid.
#[must_use = "show the panel with `show`"]
#[derive(Pattern)]
pub struct InfoPanel<'a> {
    label: Label,
    readings: &'a InfoReadings<'a>,
}

/// A name and its value, or `None` for a dash.
type Cell = (Label, Option<Label>);

/// The value of a number, or `None` when it is missing.
fn number(value: Option<impl ToString>) -> Option<Label> {
    value.map(|value| Label::from(value.to_string()))
}

/// A name and its value in two grid cells, or two empty cells.
fn pair(ui: &mut Ui, cell: Option<Cell>) {
    match cell {
        Some((name, value)) => {
            Text::new(name).show(ui);
            Text::new(value.unwrap_or_else(|| Label::from(DASH))).show(ui);
        }
        None => {
            Text::new(Label::from("")).show(ui);
            Text::new(Label::from("")).show(ui);
        }
    }
}

/// A grid of lines, each two name and value pairs.
fn table(ui: &mut Ui, lines: Vec<[Option<Cell>; 2]>) {
    let _ = Grid::new(&COLUMNS).show(ui, |ui, _| {
        for [left, right] in lines.iter().cloned() {
            pair(ui, left);
            pair(ui, right);
        }
    });
}

/// `cells` two to a line.
fn in_pairs(cells: Vec<Cell>) -> Vec<[Option<Cell>; 2]> {
    let mut cells = cells.into_iter();
    let mut lines = Vec::new();
    while let Some(left) = cells.next() {
        lines.push([Some(left), cells.next()]);
    }
    lines
}

impl<'a> InfoPanel<'a> {
    /// A panel named `label` over `readings`.
    pub fn new(label: impl Into<Label>, readings: &'a InfoReadings<'a>) -> Self {
        InfoPanel { label: label.into(), readings }
    }

    /// Draws the grid.
    pub fn show(self, ui: &mut Ui) -> Response {
        let InfoPanel { label, readings } = self;
        let words = Env::of(ui).words::<InfoWords>();
        let Some([width, height]) = readings.document else {
            return Notice::new(Tone::Neutral, words.no_document.clone()).layout(NoticeLayout::Centred).show(ui).response().clone();
        };
        let name = |label: &Label| -> Label { label.clone() };
        let rgb_names = [name(&words.r), name(&words.g), name(&words.b)];
        let cmyk_names = [name(&words.c), name(&words.m), name(&words.y), name(&words.k)];
        Stack::new()
            .label(label)
            .show(ui, |ui, _| {
                let mut lines = Vec::new();
                for (at, cmyk_name) in cmyk_names.iter().enumerate() {
                    let rgb = rgb_names.get(at).map(|rgb_name| (rgb_name.clone(), number(readings.color.map(|color| color.rgb[at]))));
                    lines.push([rgb, Some((cmyk_name.clone(), number(readings.color.map(|color| color.cmyk[at]))))]);
                }
                let (position, extent) = (readings.position.as_ref(), readings.extent.as_ref());
                let axes = [(name(&words.x), name(&words.w)), (name(&words.pos_y), name(&words.h))];
                for (axis, (position_name, extent_name)) in axes.into_iter().enumerate() {
                    lines.push([
                        Some((position_name, position.map(|at| at[axis].clone()))),
                        Some((extent_name, extent.map(|size| size[axis].clone()))),
                    ]);
                }
                table(ui, lines);
                let size = words.document.fill(&[("width", &width.to_string()), ("height", &height.to_string())]);
                Text::new(Label::from(size)).show(ui);
                for (readout, position) in readings.samplers {
                    Text::new(Label::from(format!("#{}", readout.number))).show(ui);
                    let mut lines = vec![[Some((name(&words.x), Some(position[0].clone()))), Some((name(&words.pos_y), Some(position[1].clone())))]];
                    lines.extend(in_pairs(rgb_names.iter().zip(readout.rgb).map(|(name, value)| (name.clone(), number(Some(value)))).collect()));
                    lines.extend(in_pairs(cmyk_names.iter().zip(readout.cmyk).map(|(name, value)| (name.clone(), number(Some(value)))).collect()));
                    table(ui, lines);
                }
            })
            .output
    }
}
