//! `contract/motion`: animation, timed repaints, clocks and reduced-motion reads in UI crates.

use crate::report::Finding;

use super::Cx;
use super::sites::Kind;

pub const ID: &str = "contract/motion";

/// Permission to read a clock: the runtime's motion module.
const CLOCK: &str = "contract/motion:clock";

pub(super) fn check(cx: &Cx<'_>, out: &mut Vec<Finding>) {
    let why = "drives egui animation or repaint timing; go through `runtime::egui`";
    cx.flag_methods(out, ID, ID, why, |name, _| {
        name.starts_with("animate_") || matches!(name, "request_repaint_after" | "animation_time" | "prefers_reduced_motion" | "reduced_motion")
    });
    for site in &cx.sites {
        if let Kind::Path { path, call: true, .. } = &site.kind
            && path.len() >= 2
            && matches!(path[path.len() - 2].as_str(), "Instant" | "SystemTime")
            && path[path.len() - 1] == "now"
        {
            cx.flag(out, ID, CLOCK, site, format!("`{}` reads a clock; read the runtime's motion clock", path.join("::")));
        }
    }
}
