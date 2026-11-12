//! Camera colour profiles of our own, pooled from many photos of one camera model.
//!
//! A raw file without colour matrices (Sony ARW, Nikon NEF, Fujifilm RAF, Canon CRW) gets its look fitted to its own embedded camera
//! JPEG (`camera_preview`), but one photo shows too little of some colours: a lime shirt covering a
//! few dozen proxy pixels next to a hillside of foliage at the same hue. `lightcraft-cli calibrate`
//! pools the colour pairs of many photos per model and fits one matrix and hue/saturation/value
//! table; photos of that model then only fit their tone and chroma curves.
//!
//! Profiles are JSON ([`CameraProfile::from_json`]). The ones built in ([`BUNDLED`], from `assets/camera-profiles/`)
//! are used unless the app [`register`]s its own for the model (LightCraft reads a folder of them; this crate does
//! no file I/O). They hold aggregate colour statistics only, never image content. Fitting a new one from many
//! photos (`lightcraft-cli calibrate`, LightCraft's `Pool`) stays in LightCraft.
use crate::color::Mat3;
use crate::sensor::profile::HsvTable;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// Format version of the profile files.
const VERSION: u32 = 1;
/// Largest profile JSON read (a 5 × 72 × 5 table is ~60 KB).
const MAX_JSON: usize = 4 << 20;

/// Profiles built in (`assets/camera-profiles/`, see `ATTRIBUTION.md`):
/// `(model, JSON)`.
pub const BUNDLED: &[(&str, &str)] = &[
    ("Canon EOS D60", include_str!("../../assets/camera-profiles/Canon_EOS_D60.json")),
    ("ILCA-99M2", include_str!("../../assets/camera-profiles/ILCA-99M2.json")),
    ("ILCE-7CR", include_str!("../../assets/camera-profiles/ILCE-7CR.json")),
    ("ILCE-7M4", include_str!("../../assets/camera-profiles/ILCE-7M4.json")),
    ("ILCE-7RM5", include_str!("../../assets/camera-profiles/ILCE-7RM5.json")),
    ("NIKON D5300", include_str!("../../assets/camera-profiles/NIKON_D5300.json")),
    ("NIKON Z 6_2", include_str!("../../assets/camera-profiles/NIKON_Z_6_2.json")),
    ("NIKON Z 8", include_str!("../../assets/camera-profiles/NIKON_Z_8.json")),
    ("X-H2S", include_str!("../../assets/camera-profiles/X-H2S.json")),
    ("X-T4", include_str!("../../assets/camera-profiles/X-T4.json")),
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraProfile {
    pub version: u32,
    /// The camera model as the files report it (Exif `Model`), e.g. `ILCE-7M4`.
    pub model: String,
    /// Photos and colour pairs the profile was fitted on.
    pub files: usize,
    pub samples: usize,
    /// White-balanced camera RGB (with the baseline exposure) → linear Rec.2020, rows.
    matrix: [[f64; 3]; 3],
    /// Hue/saturation/value correction after `matrix` (linear ProPhoto RGB).
    pub hue_sat: Option<HsvTable>,
}

impl CameraProfile {
    pub fn new(model: &str, files: usize, samples: usize, matrix: Mat3, hue_sat: Option<HsvTable>) -> CameraProfile {
        CameraProfile { version: VERSION, model: model.to_owned(), files, samples, matrix: matrix.0, hue_sat }
    }

    pub fn matrix(&self) -> Mat3 {
        Mat3(self.matrix)
    }

    /// Whether the data is usable (bounded matrix, table shape matching its data).
    fn valid(&self) -> bool {
        let matrix = self.matrix.iter().flatten().all(|v| v.is_finite() && v.abs() < 8.0) && Mat3(self.matrix).inverse().is_some();
        let table = self.hue_sat.as_ref().is_none_or(|t| {
            let dims = (1..=4096).contains(&t.hue_divisions) && (2..=4096).contains(&t.sat_divisions) && (1..=4096).contains(&t.val_divisions);
            let len = t.hue_divisions.checked_mul(t.sat_divisions).and_then(|n| n.checked_mul(t.val_divisions));
            dims && len == Some(t.data.len()) && t.data.iter().flatten().all(|v| v.is_finite()) && t.data.iter().all(|e| e[1] >= 0.0 && e[2] >= 0.0)
        });
        self.version == VERSION && !self.model.is_empty() && matrix && table
    }

    /// A profile from its JSON, validated (bounded matrix, table shape matching its data).
    pub fn from_json(json: &[u8]) -> Result<CameraProfile, String> {
        parse(json, "camera profile")
    }

    /// The profile as JSON, as [`Self::from_json`] reads it.
    pub fn to_json(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec_pretty(self).map_err(|e| e.to_string())
    }
}

/// Parse and validate a profile's JSON (`origin` names it in errors).
fn parse(json: &[u8], origin: &str) -> Result<CameraProfile, String> {
    if json.len() > MAX_JSON {
        return Err(format!("{origin}: {} bytes is too large for a camera profile", json.len()));
    }
    let profile: CameraProfile = serde_json::from_slice(json).map_err(|e| format!("{origin}: {e}"))?;
    if !profile.valid() {
        return Err(format!("{origin}: not a usable camera profile (version {}, model {:?})", profile.version, profile.model));
    }
    Ok(profile)
}

/// The built-in profile for `model`, if any.
fn bundled(model: &str) -> Option<CameraProfile> {
    let (_, json) = BUNDLED.iter().find(|(m, _)| *m == model)?;
    parse(json.as_bytes(), &format!("built-in profile {model}")).ok()
}

fn registry() -> &'static Mutex<HashMap<String, Arc<CameraProfile>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<CameraProfile>>>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

fn cache() -> &'static Mutex<HashMap<String, Option<Arc<CameraProfile>>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<Arc<CameraProfile>>>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Use `profile` for its model from now on, in place of a built-in one (process-wide, like LightCraft's profile
/// folder). Sources developed before keep the look they were fitted with: the caller forgets them.
pub fn register(profile: CameraProfile) {
    let model = profile.model.trim().to_owned();
    registry().lock().unwrap_or_else(PoisonError::into_inner).insert(model.clone(), Arc::new(profile));
    cache().lock().unwrap_or_else(PoisonError::into_inner).remove(&model);
}

/// The profile for camera `model`: a [`register`]ed one, else built in.
pub fn get(model: &str) -> Option<Arc<CameraProfile>> {
    let model = model.trim();
    if model.is_empty() {
        return None;
    }
    if let Some(p) = registry().lock().unwrap_or_else(PoisonError::into_inner).get(model) {
        return Some(p.clone());
    }
    let mut cache = cache().lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(hit) = cache.get(model) {
        return hit.clone();
    }
    let profile = bundled(model).map(Arc::new);
    cache.insert(model.to_owned(), profile.clone());
    profile
}

/// Changes when the registered profiles change: part of render cache keys, so a render made before a profile
/// was registered is not reused after.
pub fn cache_key() -> u64 {
    // FNV-1a over the registered profiles' JSON, by model
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut eat = |b: &[u8]| {
        for &x in b {
            h = (h ^ u64::from(x)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    let reg = registry().lock().unwrap_or_else(PoisonError::into_inner);
    let mut models: Vec<&String> = reg.keys().collect();
    models.sort();
    for m in models {
        eat(m.as_bytes());
        if let Some(p) = reg.get(m) {
            eat(&serde_json::to_vec(&**p).unwrap_or_default());
        }
    }
    for (model, json) in BUNDLED {
        eat(model.as_bytes());
        eat(json.as_bytes());
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> CameraProfile {
        let table = HsvTable { hue_divisions: 4, sat_divisions: 2, val_divisions: 1, data: vec![[5.0, 1.2, 1.0]; 8], srgb_value: false };
        CameraProfile::new("ILCE-7M4", 12, 3456, Mat3([[1.6, -0.5, -0.1], [-0.2, 1.3, -0.1], [-0.1, -0.2, 1.3]]), Some(table))
    }

    #[test]
    fn profiles_round_trip_and_reject_bad_files() {
        assert_eq!(CameraProfile::from_json(&profile().to_json().unwrap()).unwrap(), profile());
        let bad = |p: CameraProfile| CameraProfile::from_json(&serde_json::to_vec(&p).unwrap()).is_err();
        let mut p = profile();
        p.matrix[0][0] = f64::NAN;
        assert!(bad(p), "non-finite matrix");
        let mut p = profile();
        p.matrix = [[0.0; 3]; 3];
        assert!(bad(p), "singular matrix");
        let mut p = profile();
        if let Some(t) = p.hue_sat.as_mut() {
            t.data.truncate(3);
        }
        assert!(bad(p), "table data doesn't match its shape");
        let mut p = profile();
        p.version = 99;
        assert!(bad(p), "unknown version");
        assert!(CameraProfile::from_json(b"{not json").is_err());
    }

    #[test]
    fn bundled_profiles_are_valid_and_named_after_their_model() {
        assert!(!BUNDLED.is_empty());
        for (model, json) in BUNDLED {
            let p = parse(json.as_bytes(), model).unwrap();
            assert_eq!(p.model, *model);
            assert!(p.files >= 5 && p.hue_sat.is_some(), "{model}: {} photos", p.files);
            assert_eq!(bundled(model), Some(p));
        }
        assert!(bundled("No Such Camera").is_none());
        assert!(bundled("Canon EOS D60").is_some_and(|p| p.files > 1000));
    }

    #[test]
    fn a_registered_profile_replaces_the_built_in_one_and_changes_the_key() {
        let before = cache_key();
        let mut p = profile();
        p.model = "Test Camera Registered".into();
        assert!(get("Test Camera Registered").is_none());
        register(p.clone());
        assert_eq!(get("Test Camera Registered").as_deref(), Some(&p));
        assert_ne!(cache_key(), before);
    }
}
