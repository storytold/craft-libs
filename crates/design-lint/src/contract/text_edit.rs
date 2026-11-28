//! `contract/text-edit`: `runtime::egui::text_edit`, the builder of egui's `TextEdit`, outside the text adapter.

use crate::report::Finding;

use super::Cx;
use super::sites::Kind;

pub const ID: &str = "contract/text-edit";

pub(super) fn check(cx: &Cx<'_>, out: &mut Vec<Finding>) {
    for site in &cx.sites {
        if let Kind::Path { path, .. } = &site.kind
            && path.windows(3).any(|w| w[0] == "runtime" && w[1] == "egui" && w[2] == "text_edit")
        {
            cx.flag(out, ID, ID, site, format!("`{}` builds egui's `TextEdit`; only `primitives::text_adapter` calls it", path.join("::")));
        }
    }
}
