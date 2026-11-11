//! `contract/runtime-egui`: component families and patterns (layers 5 and 6) naming `runtime::egui`.

use crate::report::Finding;

use super::Cx;
use super::sites::Kind;

pub const ID: &str = "contract/runtime-egui";

/// Layers whose modules are written against the contract: component families and patterns.
const FAMILY_LAYERS: std::ops::RangeInclusive<u8> = 5..=6;

pub(super) fn check(cx: &Cx<'_>, out: &mut Vec<Finding>) {
    if !cx.scope.layer(cx.path).is_some_and(|l| FAMILY_LAYERS.contains(&l)) {
        return;
    }
    for site in &cx.sites {
        if let Kind::Path { path, .. } = &site.kind
            && path.windows(2).any(|w| w[0] == "runtime" && w[1] == "egui")
        {
            cx.flag(
                out,
                ID,
                ID,
                site,
                format!("`{}` is the egui adapter; a family or pattern module goes through the contract and `primitives`", path.join("::")),
            );
        }
    }
}
