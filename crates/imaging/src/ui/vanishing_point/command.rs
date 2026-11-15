//! [`VanishingPointCommand`]: what OK answers, in the params shape of `filter.vanishingPoint`, and the readers of the
//! `planes` and `clone` values the automation surface sets, with the engine's rules.

use serde::Serialize;
use serde_json::Value;

use super::bar::PASTE_WIDTH;
use super::editor::{StampLimits, StampSettings, StampStroke, VanishingPointSession, VanishingPointSettings, clamp_in};
use super::planes::{MAX_PLANES, PlaneEdge, TornPlane, VanishingPlane};

/// A plane of the command: its corners, or how it was torn off an earlier plane.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum VanishingCommandPlane {
    /// A plane drawn by hand.
    #[non_exhaustive]
    Drawn {
        /// The corners in document pixels.
        corners: [[f64; 2]; 4],
    },
    /// A plane torn off an edge of an earlier plane.
    Torn(TornPlane),
}

/// The pasted image of the command: the plane it lands on, its top-left in plane coordinates and its width.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[non_exhaustive]
pub struct VanishingCommandPaste {
    /// The plane's index.
    pub plane: usize,
    /// The image's top-left in plane coordinates.
    pub at: [f64; 2],
    /// The image's width as a fraction of the plane's width.
    pub width: f64,
}

/// A clone stroke of the command; hardness and opacity are in percent.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[non_exhaustive]
pub struct VanishingCommandStroke {
    /// The clone source in document pixels.
    pub source: [f64; 2],
    /// The brush path in document pixels.
    pub points: Vec<[f64; 2]>,
    /// The brush diameter.
    pub size: f64,
    /// Hardness, 0..=100.
    pub hardness: f64,
    /// Opacity, 0..=100.
    pub opacity: f64,
}

/// The engine command a Vanishing Point OK answers (`filter.vanishingPoint`'s params, without the app's `layer`): the
/// planes, the paste and the clone strokes when there are any, and `newLayer` when it is on.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct VanishingPointCommand {
    /// Every plane, in index order.
    pub planes: Vec<VanishingCommandPlane>,
    /// The pasted image; at most one entry, only with Paste on and a clipboard image.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub paste: Vec<VanishingCommandPaste>,
    /// The clone strokes, in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub clone: Vec<VanishingCommandStroke>,
    /// Whether the result goes to a new layer.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub new_layer: bool,
}

impl VanishingPointCommand {
    /// The command for `session` under `settings`. The paste goes on the `selected` plane, else the first, and only
    /// when Paste is on and the source has an image (`has_paste`).
    pub(super) fn new(session: &VanishingPointSession, settings: &VanishingPointSettings, selected: Option<usize>, has_paste: bool) -> Self {
        let plane = selected.filter(|i| *i < session.planes.len()).or_else(|| (!session.planes.is_empty()).then_some(0));
        let paste = plane
            .filter(|_| settings.paste && has_paste)
            .map(|plane| VanishingCommandPaste { plane, at: session.paste.at, width: session.paste.width })
            .into_iter()
            .collect();
        Self { planes: command_planes(session), paste, clone: command_strokes(session), new_layer: settings.new_layer }
    }
}

/// The session's clone strokes as the command writes them, hardness and opacity in percent.
pub(super) fn command_strokes(session: &VanishingPointSession) -> Vec<VanishingCommandStroke> {
    session
        .strokes
        .iter()
        .map(|s| VanishingCommandStroke {
            source: s.source,
            points: s.points.clone(),
            size: s.size,
            hardness: s.hardness * 100.0,
            opacity: s.opacity * 100.0,
        })
        .collect()
}

/// The session's planes as the command writes them: `{corners}` for a drawn plane, `{from, edge, angle, depth}` for a
/// torn one.
pub(super) fn command_planes(session: &VanishingPointSession) -> Vec<VanishingCommandPlane> {
    session
        .planes
        .iter()
        .enumerate()
        .map(|(i, plane)| match session.torn.get(i).copied().flatten() {
            Some(torn) => VanishingCommandPlane::Torn(torn),
            None => VanishingCommandPlane::Drawn { corners: plane.corners },
        })
        .collect()
}

/// A point `[x, y]` of finite values under 1e7 in magnitude.
pub(super) fn point(value: &Value) -> Option<[f64; 2]> {
    let at = [value.get(0)?.as_f64()?, value.get(1)?.as_f64()?];
    at.iter().all(|v| v.is_finite() && v.abs() < 1e7).then_some(at)
}

/// Reads the `planes` list: per plane its corners, or how it is torn off an earlier plane (`from` below its own
/// index, `edge` defaulting to top, `angle` to 90 in 0..=360, `depth` to 1 in (0, 1000]). A torn plane's corners are
/// zero until the scene resolves them.
pub(super) fn read_planes(value: &Value) -> Result<(Vec<VanishingPlane>, Vec<Option<TornPlane>>), String> {
    let list = value.as_array().ok_or("planes: a list")?;
    if list.len() > MAX_PLANES {
        return Err(format!("at most {MAX_PLANES} planes"));
    }
    let (mut planes, mut torn) = (Vec::new(), Vec::new());
    for (i, entry) in list.iter().enumerate() {
        if let Some(corners) = entry.get("corners") {
            let corners = corners.as_array().filter(|c| c.len() == 4).ok_or_else(|| format!("plane {i}: four corners"))?;
            let [Some(a), Some(b), Some(c), Some(d)] = [0, 1, 2, 3].map(|k| corners.get(k).and_then(point)) else {
                return Err(format!("plane {i}: corners are [x, y]"));
            };
            planes.push(VanishingPlane::new([a, b, c, d]));
            torn.push(None);
            continue;
        }
        let from = entry
            .get("from")
            .and_then(Value::as_u64)
            .and_then(|from| usize::try_from(from).ok())
            .filter(|from| *from < i)
            .ok_or_else(|| format!("plane {i}: `corners` or `from` (an earlier plane)"))?;
        let edge = match entry.get("edge").and_then(Value::as_str).unwrap_or("top") {
            "top" => PlaneEdge::Top,
            "right" => PlaneEdge::Right,
            "bottom" => PlaneEdge::Bottom,
            "left" => PlaneEdge::Left,
            other => return Err(format!("plane {i}: unknown edge `{other}`")),
        };
        let angle = entry.get("angle").and_then(Value::as_f64).unwrap_or(90.0);
        let depth = entry.get("depth").and_then(Value::as_f64).unwrap_or(1.0);
        if !((0.0..=360.0).contains(&angle) && depth > 0.0 && depth <= 1000.0) {
            return Err(format!("plane {i}: angle 0..360, depth above 0 and at most 1000"));
        }
        planes.push(VanishingPlane::new([[0.0; 2]; 4]));
        torn.push(Some(TornPlane::new(from, edge, angle, depth)));
    }
    Ok((planes, torn))
}

/// Reads the `clone` list under the engine's limits: a missing or null value is no strokes; each stroke has a finite
/// `source` and `points` (possibly none), `size` (default 40, 1..=2000), `hardness` (default 50) and `opacity`
/// (default 100), both 0..=100 and kept as shares of 1.
pub(super) fn read_strokes(value: &Value, limits: StampLimits) -> Result<Vec<StampStroke>, String> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    let list = value.as_array().ok_or("`clone` is a list of strokes")?;
    if list.len() > limits.strokes {
        return Err(format!("at most {} clone strokes", limits.strokes));
    }
    let number = |stroke: &Value, key: &str, default: f64, range: &std::ops::RangeInclusive<f64>| -> Result<f64, String> {
        match stroke.get(key) {
            None | Some(Value::Null) => Ok(default),
            Some(v) => v.as_f64().filter(|x| x.is_finite()).map(|x| clamp_in(range, x)).ok_or_else(|| format!("clone `{key}` must be a number")),
        }
    };
    let defaults = StampSettings::default();
    let mut dabs = 0_usize;
    let mut strokes = Vec::with_capacity(list.len());
    for (k, stroke) in list.iter().enumerate() {
        let source = stroke.get("source").and_then(point).ok_or_else(|| format!("clone {k}: `source` [x,y]"))?;
        let raw = stroke.get("points").and_then(Value::as_array).ok_or_else(|| format!("clone {k}: `points` [[x,y], …]"))?;
        dabs = dabs.saturating_add(raw.len());
        if dabs > limits.dabs {
            return Err(format!("at most {} clone points in all", limits.dabs));
        }
        let points = raw.iter().map(point).collect::<Option<Vec<_>>>().ok_or_else(|| format!("clone {k}: every point is [x,y]"))?;
        strokes.push(StampStroke::new(
            source,
            points,
            number(stroke, "size", defaults.diameter, &StampSettings::DIAMETER)?,
            number(stroke, "hardness", defaults.hardness, &StampSettings::PERCENT)? / 100.0,
            number(stroke, "opacity", defaults.opacity, &StampSettings::PERCENT)? / 100.0,
        ));
    }
    Ok(strokes)
}

/// The paste's top-left the control channel sets: each axis limited to -1..=2 (a non-finite value falls back to the
/// default placement). `None` when the value is not `[x, y]`.
pub(super) fn read_paste_at(value: &Value) -> Option<[f64; 2]> {
    let at = [value.get(0)?.as_f64()?, value.get(1)?.as_f64()?];
    let default = VanishingPointSession::default().paste.at;
    Some([0, 1].map(|k| if at[k].is_finite() { at[k].clamp(-1.0, 2.0) } else { default[k] }))
}

/// The paste's width, as a fraction of the plane's width, the control channel sets: limited to the bar's range.
/// `None` when the value is not a finite number.
pub(super) fn read_paste_width(value: &Value) -> Option<f64> {
    let width = value.as_f64().filter(|w| w.is_finite())?;
    Some(clamp_in(&PASTE_WIDTH, width * 100.0) / 100.0)
}
