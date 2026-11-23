//! Bounds on what is read and how much work connection building may do.

use crate::model::Project;

/// Bounds on a project. [`Limits::default`] is generous for real schematics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Bytes of JSON [`Project::from_json`] reads.
    pub input_bytes: usize,
    pub sheets: usize,
    pub parts: usize,
    /// Pins, wire points, junctions and labels together, over all sheets.
    pub items: usize,
    /// Point and segment comparisons connection building may make.
    pub work: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { input_bytes: 64 << 20, sheets: 10_000, parts: 1_000_000, items: 4_000_000, work: 400_000_000 }
    }
}

/// The largest coordinate accepted, so that grid arithmetic stays exact enough.
const MAX_COORD: f64 = 1e12;
/// The largest tolerance accepted.
const MAX_TOLERANCE: f64 = 1e6;

/// Why a project was refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("not a circuit project: {0}")]
    Json(String),
    #[error("{what} exceed the limit of {limit}")]
    TooMany { what: &'static str, limit: usize },
    #[error("sheet {sheet}, pin {pin}: part {part} does not exist")]
    NoSuchPart { sheet: usize, pin: usize, part: usize },
    #[error("sheet {sheet} has a coordinate that is not a number or is too large")]
    OutOfRange { sheet: usize },
    #[error("the tolerance must be a positive number no larger than {MAX_TOLERANCE}")]
    BadTolerance,
    #[error("the schematic needs more connection tests than the limit of {limit} allows")]
    TooComplex { limit: u64 },
}

impl Project {
    /// Reads a project from JSON within [`Limits::default`].
    pub fn from_json(bytes: &[u8]) -> Result<Self, Error> {
        Self::from_json_with(bytes, &Limits::default())
    }

    /// Reads a project from JSON within `limits`, and checks it with [`Project::validate`].
    pub fn from_json_with(bytes: &[u8], limits: &Limits) -> Result<Self, Error> {
        if bytes.len() > limits.input_bytes {
            return Err(Error::TooMany { what: "input bytes", limit: limits.input_bytes });
        }
        let project: Project = serde_json::from_slice(bytes).map_err(|e| Error::Json(e.to_string()))?;
        project.validate(limits)?;
        Ok(project)
    }

    /// The project as JSON.
    pub fn to_json(&self) -> Result<String, Error> {
        serde_json::to_string_pretty(self).map_err(|e| Error::Json(e.to_string()))
    }

    /// Checks counts, part references, coordinates and the tolerance.
    pub fn validate(&self, limits: &Limits) -> Result<(), Error> {
        if !(self.tolerance > 0.0 && self.tolerance <= MAX_TOLERANCE) {
            return Err(Error::BadTolerance);
        }
        if self.sheets.len() > limits.sheets {
            return Err(Error::TooMany { what: "sheets", limit: limits.sheets });
        }
        if self.parts.len() > limits.parts {
            return Err(Error::TooMany { what: "parts", limit: limits.parts });
        }
        let mut items = 0usize;
        for (s, sheet) in self.sheets.iter().enumerate() {
            let points = sheet.wires.iter().map(|w| w.points.len()).fold(0usize, usize::saturating_add);
            items = items
                .saturating_add(sheet.pins.len())
                .saturating_add(sheet.junctions.len())
                .saturating_add(sheet.labels.len())
                .saturating_add(points);
            if items > limits.items {
                return Err(Error::TooMany { what: "items", limit: limits.items });
            }
            self.validate_sheet(s)?;
        }
        Ok(())
    }

    fn validate_sheet(&self, s: usize) -> Result<(), Error> {
        let sheet = &self.sheets[s];
        for (i, pin) in sheet.pins.iter().enumerate() {
            if pin.part >= self.parts.len() {
                return Err(Error::NoSuchPart { sheet: s, pin: i, part: pin.part });
            }
        }
        let fine = |p: &[f64; 2]| p.iter().all(|c| c.is_finite() && c.abs() <= MAX_COORD);
        let ok = sheet.pins.iter().all(|p| fine(&p.at))
            && sheet.junctions.iter().all(fine)
            && sheet.labels.iter().all(|l| fine(&l.at))
            && sheet.wires.iter().all(|w| w.points.iter().all(fine));
        if ok { Ok(()) } else { Err(Error::OutOfRange { sheet: s }) }
    }
}
