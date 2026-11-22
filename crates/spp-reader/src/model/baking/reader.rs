//! Reads [`ProjectBakeSettings`] from the generic object tree of `baking.ini`, in both layouts seen in the corpus:
//!
//! - format 20 (version 7+, observed on a corpus project): per-material `materialParameters { commonParameters,
//!   detailParameters }` and `definitions[].parameters` are references into the root's `tweakLists[]`;
//!   each definition has its own `enabled`.
//! - format 12 (version 2020, observed on a corpus project): one root `parameters { commonParameters,
//!   detailParameters }` for all materials, each a plain list of tweaks; definitions hold `bakerId` and their
//!   tweaks inline; which bakers are on is in the root's `usageBakerTable.usageBakerAssoc[]`.

use super::{BakerSettings, MaterialBaking, ProjectBakeSettings, TweakList};
use crate::model::raw::{RawObject, RawValue};
use std::collections::BTreeMap;

pub fn read(root: &RawObject) -> ProjectBakeSettings {
    let reader = Reader {
        lists: root.objects("tweakLists").filter_map(|o| Some((o.uid()?, TweakList::from_object(o)))).collect(),
        shared: root.object("parameters"),
        enabled: root
            .object("usageBakerTable")
            .into_iter()
            .flat_map(|t| t.objects("usageBakerAssoc"))
            .filter_map(|a| Some((a.u64("usage")?, a.bool("enabled").unwrap_or(true))))
            .collect(),
    };
    ProjectBakeSettings {
        materials: root.objects("materials").map(|m| reader.material(m)).collect(),
        template: root.object("materialTemplate").map(|m| reader.material(m)),
    }
}

struct Reader<'a> {
    /// `tweakLists[]` by uid (format 20).
    lists: BTreeMap<u64, TweakList>,
    /// The root's shared common/detail parameters (format 12).
    shared: Option<&'a RawObject>,
    /// Baker on/off by usage (format 12).
    enabled: BTreeMap<u64, bool>,
}

impl Reader<'_> {
    fn material(&self, m: &RawObject) -> MaterialBaking {
        let params = m.object("materialParameters").or(self.shared).unwrap_or(m);
        MaterialBaking {
            material_uid: m.u64("materialUID"),
            enabled: m.bool("enabled").unwrap_or(true),
            common: self.list("common", params.field("commonParameters")),
            detail: self.list("detail", params.field("detailParameters")),
            bakers: m.objects("definitions").map(|d| self.baker(d)).collect(),
        }
    }

    fn baker(&self, d: &RawObject) -> BakerSettings {
        let usage = d.u64("usage");
        let mut params = self.list(d.string("bakerId").unwrap_or_default(), d.field("parameters"));
        if params.baker.is_empty() {
            params.baker = d.string("bakerId").unwrap_or_default().to_string();
        }
        BakerSettings { usage, enabled: d.bool("enabled").or_else(|| usage.and_then(|u| self.enabled.get(&u).copied())).unwrap_or(true), params }
    }

    /// An inline `BakingTweakList`, a reference to one, or a plain list of tweaks named `baker`.
    fn list(&self, baker: &str, value: Option<&RawValue>) -> TweakList {
        match value {
            Some(RawValue::Object(o)) => TweakList::from_object(o),
            Some(RawValue::Scalar(s)) => s.as_u64().and_then(|u| self.lists.get(&u)).cloned().unwrap_or_default(),
            Some(RawValue::List(items)) => TweakList::from_tweaks(
                baker,
                items.iter().filter_map(|v| match v {
                    RawValue::Object(o) => Some(o.as_ref()),
                    _ => None,
                }),
            ),
            _ => TweakList::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::document::Scalar;

    fn obj(class: &str, fields: Vec<(&str, RawValue)>) -> RawObject {
        RawObject { class: class.into(), fields: fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect() }
    }

    fn s(v: Scalar) -> RawValue {
        RawValue::Scalar(v)
    }

    fn tweak(id: &str, v: Scalar) -> RawValue {
        RawValue::Object(Box::new(obj("DataTweak", vec![("identifier", s(Scalar::String(id.into()))), ("value", s(v))])))
    }

    #[test]
    fn reads_the_old_layout_with_shared_parameters() {
        let shared = obj("BakingCommonParameters", vec![("detailParameters", RawValue::List(vec![tweak("Detail.LowAsHigh", Scalar::Bool(true))]))]);
        let definition = obj(
            "BakingDefinition",
            vec![
                ("bakerId", s(Scalar::String("GL.AOFromDetail".into()))),
                ("parameters", RawValue::List(vec![tweak("Detail.SecondaryCommon.NbSecondary", Scalar::Int(16))])),
                ("usage", s(Scalar::Enum(1))),
            ],
        );
        let material = obj(
            "BakingMaterial",
            vec![("definitions", RawValue::List(vec![RawValue::Object(Box::new(definition))])), ("materialUID", s(Scalar::Uid(5)))],
        );
        let assoc = obj("BakingUsageBakerAssoc", vec![("enabled", s(Scalar::Bool(false))), ("usage", s(Scalar::Enum(1)))]);
        let table = obj("BakingUsageBakerTable", vec![("usageBakerAssoc", RawValue::List(vec![RawValue::Object(Box::new(assoc))]))]);
        let root = obj(
            "BakingMaterials",
            vec![
                ("materials", RawValue::List(vec![RawValue::Object(Box::new(material))])),
                ("parameters", RawValue::Object(Box::new(shared))),
                ("usageBakerTable", RawValue::Object(Box::new(table))),
            ],
        );
        let baking = read(&root);
        let m = baking.for_material(Some(5)).unwrap();
        assert_eq!(m.detail.bool("Detail.LowAsHigh"), Some(true));
        let ao = m.baker(1).unwrap();
        assert_eq!(ao.params.baker, "GL.AOFromDetail");
        assert_eq!(ao.params.f32("Detail.SecondaryCommon.NbSecondary"), Some(16.0));
        assert!(!ao.enabled);
        assert!(baking.for_material(Some(6)).is_none());
    }
}
