//! [`InfoPanel`]: a read-only readout grid of the colour, position and size under the pointer.

use craft_behavior::ColumnWidth;
use craft_cms::convert::CmykConverter;
use craft_theme::Srgba;
use craft_theme::variant::Tone;
use craft_ui::contract::{Gap, Outcome as _, Pattern, Props as _, Show as _, ShowContainer as _};
use craft_ui::display::{Divider, Text};
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout::{CellAlign, GridFit};
use craft_ui::layout_components::{Grid, Stack};
use craft_ui::notices::{Notice, NoticeLayout};
use craft_ui::runtime::env::{Env, Label};
use egui::{Response, Ui};

use super::SamplerReadout;

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// What a missing value shows.
const DASH: &str = "–";

/// A block's two halves, equal in width.
const HALVES: [ColumnWidth; 2] = [ColumnWidth::fill(1.0), ColumnWidth::fill(1.0)];

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
        /// The Ruler tool's angle.
        angle: "info.angle" = "A";
        /// A plain ruler's length.
        length: "info.length" = "L";
        /// A protractor's first arm.
        length1: "info.length1" = "L1";
        /// A protractor's second arm.
        length2: "info.length2" = "L2";
        /// The document size template.
        document: template "info.document" = "Doc: {width} x {height}";
        no_document: "info.noDocument" = "No document";
    }
}

/// A colour in 8-bit sRGB and CMYK percent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct InfoColor {
    /// The colour in 8-bit sRGB.
    pub rgb: [u8; 3],
    /// The colour in CMYK percent.
    pub cmyk: [u8; 4],
}

impl InfoColor {
    /// The colour with the given 8-bit sRGB and CMYK percent values, as an app already holds them.
    pub const fn from_parts(rgb: [u8; 3], cmyk: [u8; 4]) -> Self {
        InfoColor { rgb, cmyk }
    }

    /// `color` through the document's CMYK converter: the one sRGB-and-CMYK conversion, which `SamplerReadout::new` builds
    /// its `rgb` and `cmyk` through. A channel that is not finite reads as 0 and the rest is clamped.
    pub fn new(color: Srgba, cmyk: &CmykConverter) -> Self {
        let unit = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        let rgb = [color.r, color.g, color.b].map(unit);
        InfoColor { rgb: rgb.map(|v| (v * 255.0).round() as u8), cmyk: cmyk.from_srgb(rgb).map(|v| (unit(v) * 100.0).round() as u8) }
    }
}

/// The Ruler tool's measure, its values as the app formats them in the ruler's unit.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum InfoMeasure {
    /// A plain ruler: its angle and length.
    #[non_exhaustive]
    Line {
        /// The angle.
        angle: Label,
        /// The length.
        length: Label,
    },
    /// A protractor: the angle between its arms and the arms' lengths.
    #[non_exhaustive]
    Protractor {
        /// The angle between the arms.
        angle: Label,
        /// The first arm's length.
        first: Label,
        /// The second arm's length.
        second: Label,
    },
}

impl InfoMeasure {
    /// A plain ruler's measure.
    pub fn line(angle: impl Into<Label>, length: impl Into<Label>) -> Self {
        InfoMeasure::Line { angle: angle.into(), length: length.into() }
    }

    /// A protractor's measure.
    pub fn protractor(angle: impl Into<Label>, first: impl Into<Label>, second: impl Into<Label>) -> Self {
        InfoMeasure::Protractor { angle: angle.into(), first: first.into(), second: second.into() }
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
    /// The Ruler tool's measure, while it has a ruler.
    pub measure: Option<InfoMeasure>,
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

    /// Sets the Ruler tool's measure.
    #[must_use]
    pub fn measure(mut self, measure: Option<InfoMeasure>) -> Self {
        self.measure = measure;
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

/// A read-only panel of blocks, a rule between them, each block two equal halves of labelled readouts: RGB beside CMYK
/// (in percent), the Ruler tool's measure (the angle A beside the length L, or a protractor's arms L1 and L2 stacked
/// beside it), X/Y beside W/H, the document size, then a position block and a colour block per sampler; a dash for a
/// missing value; `document: None` shows "No document" as a centred notice in place of the blocks.
#[must_use = "show the panel with `show`"]
#[derive(Pattern)]
pub struct InfoPanel<'a> {
    label: Label,
    readings: &'a InfoReadings<'a>,
}

/// A name and its value, or `None` for a dash.
type Cell = (Label, Option<Label>);

/// One block: the readouts of its left and right halves.
type Block = [Vec<Cell>; 2];

/// The value of a number, or `None` when it is missing.
fn number(value: Option<impl ToString>) -> Option<Label> {
    value.map(|value| Label::from(value.to_string()))
}

/// The value of a CMYK percent, or `None` when it is missing.
fn percent(value: Option<u8>) -> Option<Label> {
    value.map(|value| Label::from(format!("{value}%")))
}

/// A half of a block: its readouts as the fields of one form group, so the names carry their colons and share a
/// column and the values line up.
fn half(ui: &mut Ui, cells: &[Cell]) {
    let _ = FieldGroup::new(Label::from("")).gap(Gap::Tight).show(ui, |ui, _| {
        for (name, value) in cells {
            let value = value.clone().unwrap_or_else(|| Label::from(DASH));
            let _ = Field::new(name.clone()).value(value.as_str()).show(ui, |ui, _| {
                Text::new(value.clone()).show(ui);
            });
        }
    });
}

/// A block: two equal halves, which stay beside each other in a narrow panel.
fn block(ui: &mut Ui, [left, right]: &Block) {
    let _ = Grid::new(&HALVES).fit(GridFit::Fixed).cells(CellAlign::Top).show(ui, |ui, _| {
        half(ui, left);
        half(ui, right);
    });
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
                let color = readings.color;
                let rgb = rgb_names.iter().enumerate().map(|(at, name)| (name.clone(), number(color.map(|color| color.rgb[at])))).collect();
                let cmyk = cmyk_names.iter().enumerate().map(|(at, name)| (name.clone(), percent(color.map(|color| color.cmyk[at])))).collect();
                let mut blocks = vec![[rgb, cmyk]];
                match &readings.measure {
                    Some(InfoMeasure::Line { angle, length }) => {
                        blocks.push([vec![(name(&words.angle), Some(angle.clone()))], vec![(name(&words.length), Some(length.clone()))]]);
                    }
                    Some(InfoMeasure::Protractor { angle, first, second }) => {
                        blocks.push([
                            vec![(name(&words.angle), Some(angle.clone()))],
                            vec![(name(&words.length1), Some(first.clone())), (name(&words.length2), Some(second.clone()))],
                        ]);
                    }
                    None => {}
                }
                let (position, extent) = (readings.position.as_ref(), readings.extent.as_ref());
                let axes = [(name(&words.x), name(&words.w)), (name(&words.pos_y), name(&words.h))];
                let (mut at, mut size) = (Vec::new(), Vec::new());
                for (axis, (position_name, extent_name)) in axes.into_iter().enumerate() {
                    at.push((position_name, position.map(|at| at[axis].clone())));
                    size.push((extent_name, extent.map(|size| size[axis].clone())));
                }
                blocks.push([at, size]);
                for (at, readings) in blocks.iter().enumerate() {
                    if at > 0 {
                        Divider::horizontal().show(ui);
                    }
                    block(ui, readings);
                }
                let size = words.document.fill(&[("width", &width.to_string()), ("height", &height.to_string())]);
                Divider::horizontal().show(ui);
                Text::new(Label::from(size)).show(ui);
                for (readout, position) in readings.samplers {
                    Divider::horizontal().show(ui);
                    Text::new(Label::from(format!("#{}", readout.number))).show(ui);
                    let place = [vec![(name(&words.x), Some(position[0].clone()))], vec![(name(&words.pos_y), Some(position[1].clone()))]];
                    let rgb = rgb_names.iter().zip(readout.rgb).map(|(name, value)| (name.clone(), number(Some(value)))).collect();
                    let cmyk = cmyk_names.iter().zip(readout.cmyk).map(|(name, value)| (name.clone(), percent(Some(value)))).collect();
                    block(ui, &place);
                    block(ui, &[rgb, cmyk]);
                }
            })
            .output
    }
}
