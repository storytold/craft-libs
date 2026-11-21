//! [`SampledEase`]: an ease kept as a table of values.

use crate::Vec2;
use crate::custom::{CustomEase, EasePoint, MAX_EASE_Y};

/// Most samples a sampled ease keeps (longer tables are thinned evenly).
pub const MAX_EASE_SAMPLES: usize = 1025;
/// Points of the custom curve a sampled ease becomes when edited (of [`crate::MAX_EASE_POINTS`]).
pub const POLYLINE_POINTS: usize = 48;
/// Longest source or ease id a sampled ease keeps.
const MAX_EASE_ID: usize = 64;
/// Longest name a sampled ease keeps.
const MAX_EASE_NAME: usize = 64;

/// An ease as a table: `values[i]` is the eased progress at `t = i / (n - 1)`, read between
/// samples by linear interpolation, so evaluation is a constant-time function of `t`. It keeps
/// a name and where it came from (a source id, such as a plug-in's, and the ease's id there).
///
/// Always valid, however it was made: 2 to [`MAX_EASE_SAMPLES`] finite values within
/// ±[`MAX_EASE_Y`], starting at exactly 0 and ending at exactly 1.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(from = "SampledEaseFile"))]
pub struct SampledEase {
    name: String,
    plugin: String,
    ease: String,
    values: Vec<f64>,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct SampledEaseFile {
    #[serde(default, deserialize_with = "read_name")]
    name: String,
    #[serde(default, deserialize_with = "read_id")]
    plugin: String,
    #[serde(default, deserialize_with = "read_id")]
    ease: String,
    #[serde(default, deserialize_with = "read_values")]
    values: Vec<f64>,
}

/// Most values read from a file's list; the rest are skipped.
#[cfg(feature = "serde")]
const MAX_SAMPLE_INPUT: usize = 65_536;

#[cfg(feature = "serde")]
fn read_name<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    crate::read_cleaned(d, |s| ease_name(s).unwrap_or_default())
}

#[cfg(feature = "serde")]
fn read_id<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    crate::read_cleaned(d, ease_id)
}

#[cfg(feature = "serde")]
fn read_values<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<f64>, D::Error> {
    crate::read_first(d, MAX_SAMPLE_INPUT)
}

#[cfg(feature = "serde")]
impl From<SampledEaseFile> for SampledEase {
    fn from(f: SampledEaseFile) -> SampledEase {
        SampledEase::new(&f.name, &f.plugin, &f.ease, f.values)
    }
}

/// An id as a sampled ease keeps it: `[A-Za-z0-9._-]`, at most [`MAX_EASE_ID`] characters.
fn ease_id(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')).take(MAX_EASE_ID).collect()
}

/// A name as kept: control characters, bidirectional overrides and invisible format characters
/// (zero-width spaces and joiners, word joiners, the byte-order mark, soft hyphens) removed,
/// trimmed, at most [`MAX_EASE_NAME`] characters; `None` when nothing is left.
fn ease_name(name: &str) -> Option<String> {
    let n: String = name
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(
                    c,
                    '\u{ad}' | '\u{180e}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}'
                )
        })
        .skip_while(|c| c.is_whitespace())
        .take(MAX_EASE_NAME)
        .collect();
    let n = n.trim().to_string();
    (!n.is_empty()).then_some(n)
}

/// The index of the `k`th of [`MAX_EASE_SAMPLES`] samples spread evenly over `n` values,
/// computed in 128 bits so a long table cannot overflow a 32-bit `usize`.
pub(crate) fn thinned(k: usize, n: usize) -> usize {
    let i = (k as u128) * (n.saturating_sub(1) as u128) / ((MAX_EASE_SAMPLES - 1) as u128);
    usize::try_from(i).unwrap_or(usize::MAX)
}

impl SampledEase {
    /// A valid table from any values (see the type's docs): a longer table is thinned evenly, a
    /// value that is not finite becomes the straight line's, values are clamped, the ends pinned;
    /// fewer than two values make the straight line.
    pub fn new(name: &str, plugin: &str, ease: &str, mut values: Vec<f64>) -> SampledEase {
        if values.len() > MAX_EASE_SAMPLES {
            let n = values.len();
            values = (0..MAX_EASE_SAMPLES).filter_map(|k| values.get(thinned(k, n)).copied()).collect();
        }
        if values.len() < 2 {
            values = vec![0.0, 1.0];
        }
        let last = (values.len() - 1) as f64;
        for (i, v) in values.iter_mut().enumerate() {
            *v = if v.is_finite() { v.clamp(-MAX_EASE_Y, MAX_EASE_Y) } else { i as f64 / last };
        }
        if let Some(v) = values.first_mut() {
            *v = 0.0;
        }
        if let Some(v) = values.last_mut() {
            *v = 1.0;
        }
        SampledEase { name: ease_name(name).unwrap_or_else(|| "Sampled ease".to_string()), plugin: ease_id(plugin), ease: ease_id(ease), values }
    }

    /// The name an ease menu shows.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// The id of the source it came from.
    pub fn plugin(&self) -> &str {
        &self.plugin
    }
    /// Its id within that source.
    pub fn ease(&self) -> &str {
        &self.ease
    }
    pub fn values(&self) -> &[f64] {
        &self.values
    }

    /// A table a caller supplies, checked: an error for a value that is not finite or outside
    /// ±[`MAX_EASE_Y`], fewer than 2 values or more than [`MAX_EASE_SAMPLES`]. The ends are then
    /// pinned to 0 and 1 as [`SampledEase::new`] does.
    pub fn try_new(name: &str, plugin: &str, ease: &str, values: Vec<f64>) -> Result<SampledEase, crate::EaseError> {
        for v in &values {
            crate::within(*v, -MAX_EASE_Y, MAX_EASE_Y, "sample")?;
        }
        if values.len() < 2 {
            return Err(crate::EaseError::TooFew { what: "a sampled ease", need: 2 });
        }
        if values.len() > MAX_EASE_SAMPLES {
            return Err(crate::EaseError::Full(MAX_EASE_SAMPLES));
        }
        Ok(SampledEase::new(name, plugin, ease, values))
    }

    /// The eased value at time `t`, between the two nearest samples. `t` outside `0..=1` is
    /// clamped, and NaN reads as 0.
    pub fn apply(&self, t: f64) -> f64 {
        let t = if t.is_finite() {
            t.clamp(0.0, 1.0)
        } else if t > 0.0 {
            1.0
        } else {
            0.0
        };
        let last = self.values.len().saturating_sub(1);
        if last == 0 {
            return t;
        }
        let x = t * last as f64;
        let i = (x.floor() as usize).min(last - 1);
        let f = x - i as f64;
        let (a, b) = (self.values.get(i).copied().unwrap_or(0.0), self.values.get(i + 1).copied().unwrap_or(1.0));
        a + (b - a) * f
    }

    /// The custom curve of straight segments through [`POLYLINE_POINTS`] of its samples, for a
    /// graph editor: editing a sampled ease makes it a custom ease drawn like it, with room left
    /// to add points.
    pub fn to_polyline(&self) -> CustomEase {
        let n = self.values.len();
        let m = POLYLINE_POINTS.min(n).max(2);
        let at: Vec<Vec2> = (0..m)
            .map(|k| {
                let i = k * (n - 1) / (m - 1);
                Vec2::new(i as f64 / (n - 1).max(1) as f64, self.values.get(i).copied().unwrap_or(0.0))
            })
            .collect();
        let pts = at
            .iter()
            .enumerate()
            .map(|(k, p)| {
                let prev = if k == 0 { *p } else { at.get(k - 1).copied().unwrap_or(*p) };
                let next = at.get(k + 1).copied().unwrap_or(*p);
                EasePoint { at: *p, h_in: (prev - *p) * (1.0 / 3.0), out: (next - *p) * (1.0 / 3.0), smooth: false }
            })
            .collect();
        CustomEase::new(pts)
    }
}
