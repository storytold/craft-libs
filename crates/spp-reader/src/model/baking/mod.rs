//! [`ProjectBakeSettings`]: the reference app's baking settings from `/baking/baking.ini`, read with the generic object walker
//! ([`super::RawTree`]; the file is the reference app's object serialisation, not an `.ini`). A faithful mirror; the mapping
//! onto our bake settings is `translate::bake_settings`'s job.
//!
//! Layout (observed on a corpus project, format 20): root `BakingMaterials` with `materials[]` (`BakingMaterial`:
//! `materialUID` = the `DataMaterial` uid in `document.bin`, `enabled`, `materialParameters` with
//! `commonParameters`/`detailParameters`, `definitions[]` = one `BakingDefinition { usage, enabled,
//! parameters }` per baker), where parameters are references into `tweakLists[]` (`BakingTweakList { bakerId,
//! tweaks[] }`, each tweak `DataTweak* { identifier, value }`). `materialTemplate` holds the same inline.
//! Older files (format 12, version 2020) differ; the `reader` module reads both.

mod reader;

use super::RawTree;
use super::raw::RawObject;
use crate::decode::DecodeError;
use crate::decode::document::Scalar;
use reader::read;

/// Where the settings live inside a project file.
pub const BAKING_INI: &str = "/baking/baking.ini";

#[derive(Debug, Clone, PartialEq)]
pub enum TweakValue {
    Bool(bool),
    Int(i64),
    Int2([i32; 2]),
    Float(f32),
    Text(String),
    Other(String),
}

impl TweakValue {
    fn from_scalar(s: &Scalar) -> TweakValue {
        match s {
            Scalar::Bool(b) => TweakValue::Bool(*b),
            Scalar::Int(v) => TweakValue::Int(i64::from(*v)),
            Scalar::UInt(v) => TweakValue::Int(i64::from(*v)),
            Scalar::IVec2(v) => TweakValue::Int2(*v),
            Scalar::Float(v) => TweakValue::Float(*v),
            Scalar::String(v) => TweakValue::Text(v.clone()),
            other => TweakValue::Other(other.to_string()),
        }
    }

    pub fn as_f32(&self) -> Option<f32> {
        match self {
            TweakValue::Float(v) => Some(*v),
            TweakValue::Int(v) => Some(*v as f32),
            _ => None,
        }
    }
}

impl std::fmt::Display for TweakValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TweakValue::Bool(v) => write!(f, "{v}"),
            TweakValue::Int(v) => write!(f, "{v}"),
            TweakValue::Int2(v) => write!(f, "{}x{}", v[0], v[1]),
            TweakValue::Float(v) => write!(f, "{v}"),
            TweakValue::Text(v) => write!(f, "{v:?}"),
            TweakValue::Other(v) => f.write_str(v),
        }
    }
}

/// One `BakingTweakList`: the parameters of one baker (or the common/detail parameters).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TweakList {
    pub baker: String,
    pub tweaks: Vec<(String, TweakValue)>,
}

impl TweakList {
    /// A `BakingTweakList` object.
    pub(super) fn from_object(o: &RawObject) -> TweakList {
        TweakList::from_tweaks(o.string("bakerId").unwrap_or_default(), o.objects("tweaks"))
    }

    /// `DataTweak*` objects of the baker `baker`.
    pub(super) fn from_tweaks<'a>(baker: &str, tweaks: impl Iterator<Item = &'a RawObject>) -> TweakList {
        TweakList {
            baker: baker.to_string(),
            tweaks: tweaks.filter_map(|t| Some((t.string("identifier")?.to_string(), TweakValue::from_scalar(t.scalar("value")?)))).collect(),
        }
    }

    pub fn get(&self, id: &str) -> Option<&TweakValue> {
        self.tweaks.iter().find(|(k, _)| k == id).map(|(_, v)| v)
    }

    pub fn f32(&self, id: &str) -> Option<f32> {
        self.get(id)?.as_f32()
    }

    pub fn bool(&self, id: &str) -> Option<bool> {
        match self.get(id)? {
            TweakValue::Bool(b) => Some(*b),
            TweakValue::Int(v) => Some(*v != 0),
            _ => None,
        }
    }

    pub fn text(&self, id: &str) -> Option<&str> {
        match self.get(id)? {
            TweakValue::Text(t) => Some(t),
            _ => None,
        }
    }
}

/// One baker of a material: its map usage (the reference app's `additionalMaps` usage; 6 = tangent normal).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BakerSettings {
    pub usage: Option<u64>,
    pub enabled: bool,
    pub params: TweakList,
}

/// The settings the reference app baked one texture set with.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MaterialBaking {
    /// `DataMaterial` uid (`None` for the template).
    pub material_uid: Option<u64>,
    pub enabled: bool,
    pub common: TweakList,
    pub detail: TweakList,
    pub bakers: Vec<BakerSettings>,
}

impl MaterialBaking {
    pub fn baker(&self, usage: u64) -> Option<&BakerSettings> {
        self.bakers.iter().find(|b| b.usage == Some(usage))
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectBakeSettings {
    pub materials: Vec<MaterialBaking>,
    pub template: Option<MaterialBaking>,
}

impl ProjectBakeSettings {
    pub fn parse(data: &[u8]) -> Result<ProjectBakeSettings, DecodeError> {
        let tree = RawTree::parse(data)?;
        Ok(tree.root.as_ref().map(read).unwrap_or_default())
    }

    /// The material's own settings, else the template's.
    pub fn for_material(&self, uid: Option<u64>) -> Option<&MaterialBaking> {
        uid.and_then(|u| self.materials.iter().find(|m| m.material_uid == Some(u))).or(self.template.as_ref())
    }
}
