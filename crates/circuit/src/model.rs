//! The data an editor hands over: parts, sheets and what is drawn on them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A position in sheet coordinates, in the drawing's units.
pub type Point = [f64; 2];

/// The distance within which two points meet when a project does not say.
pub const DEFAULT_TOLERANCE: f64 = 1e-6;

fn default_tolerance() -> f64 {
    DEFAULT_TOLERANCE
}

fn yes() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

/// A schematic: its parts, and the sheets their pins are placed on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    /// Points closer than this meet, in the drawing's units.
    #[serde(default = "default_tolerance")]
    pub tolerance: f64,
    #[serde(default)]
    pub parts: Vec<Part>,
    #[serde(default)]
    pub sheets: Vec<Sheet>,
}

impl Default for Project {
    fn default() -> Self {
        Self { tolerance: DEFAULT_TOLERANCE, parts: Vec::new(), sheets: Vec::new() }
    }
}

impl Project {
    /// Where an item sits: a pin, junction or label at its point, a wire at its first point.
    pub fn item_point(&self, item: ItemRef) -> Option<Point> {
        let sheet = self.sheets.get(item.sheet)?;
        match item.kind {
            ItemKind::Pin => sheet.pins.get(item.index).map(|p| p.at),
            ItemKind::Wire => sheet.wires.get(item.index).and_then(|w| w.points.first().copied()),
            ItemKind::Junction => sheet.junctions.get(item.index).copied(),
            ItemKind::Label => sheet.labels.get(item.index).map(|l| l.at),
        }
    }

    /// The pin an item reference names, when it names a pin.
    pub fn pin(&self, item: ItemRef) -> Option<&Pin> {
        match item.kind {
            ItemKind::Pin => self.sheets.get(item.sheet)?.pins.get(item.index),
            _ => None,
        }
    }
}

/// One component: a resistor, a relay, a connector. Its pins are placed on the sheets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Part {
    /// The reference designator (`R1`, `-K3`). Empty, or ending in `?`, means not numbered yet.
    pub designator: String,
    pub value: String,
    pub footprint: String,
    /// Further fields: manufacturer, order number, rating.
    pub fields: BTreeMap<String, String>,
    /// Whether the part belongs in the parts list.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub in_bom: bool,
}

impl Default for Part {
    fn default() -> Self {
        Self { designator: String::new(), value: String::new(), footprint: String::new(), fields: BTreeMap::new(), in_bom: true }
    }
}

impl Part {
    /// Whether the part has been given its final designator.
    pub fn is_numbered(&self) -> bool {
        !self.designator.is_empty() && !self.designator.ends_with('?')
    }
}

/// One sheet of the schematic.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sheet {
    pub name: String,
    pub pins: Vec<Pin>,
    pub wires: Vec<Wire>,
    pub junctions: Vec<Point>,
    pub labels: Vec<Label>,
}

/// A pin of a part, placed on a sheet. A part with several units has its pins on several
/// sheets, all naming the same part.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pin {
    /// Index into [`Project::parts`].
    pub part: usize,
    /// The pin number on the part (`1`, `A2`, `13`).
    pub number: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: PinKind,
    pub at: Point,
}

/// What a pin does electrically, for the connection checks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinKind {
    Input,
    Output,
    Bidirectional,
    /// An output that can also let go of the line (three-state).
    TriState,
    /// A resistor or capacitor lead, a switch contact: no direction.
    #[default]
    Passive,
    /// Takes supply.
    PowerIn,
    /// Gives supply.
    PowerOut,
    /// Pulls the line one way only and needs a pull-up or pull-down.
    OpenCollector,
    /// Must stay unconnected.
    NotConnected,
    /// The symbol does not say.
    Unspecified,
}

/// A wire drawn as a polyline. All its points are connected.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Wire {
    pub points: Vec<Point>,
}

/// A name placed on a wire or pin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Label {
    pub name: String,
    #[serde(default)]
    pub scope: LabelScope,
    pub at: Point,
}

/// How far a label's name reaches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelScope {
    /// Joins labels of the same name on the same sheet.
    #[default]
    Sheet,
    /// Joins labels of the same name on every sheet.
    Global,
    /// A supply symbol (`GND`, `+24V`): global, and counts as a source of supply.
    Power,
}

/// Which kind of drawn item an [`ItemRef`] names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Pin,
    Wire,
    Junction,
    Label,
}

/// One drawn item: its sheet, its kind and its index in that sheet's list of that kind.
/// An app keeps its own handles in the same order to map results back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ItemRef {
    pub sheet: usize,
    pub kind: ItemKind,
    pub index: usize,
}
