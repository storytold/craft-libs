//! `contract/egui-import`: egui's behavioural APIs outside `runtime::egui`.

use crate::report::Finding;

use super::Cx;
use super::sites::{Kind, is_egui};

pub const ID: &str = "contract/egui-import";

/// The layer that may name the `accesskit` data types (spec Overview, layer 1: `AccessRequest` carries an action).
const DATA_LAYER: u8 = 1;

/// Path segments naming egui's behavioural types and modules: memory, layers, AccessKit, options, input and pass control.
const SEGMENTS: [&str; 13] = [
    "Memory",
    "memory",
    "IdTypeMap",
    "LayerId",
    "Order",
    "accesskit",
    "Options",
    "InputOptions",
    "RawInput",
    "FullOutput",
    "Plugin",
    "Focus",
    "FocusDirection",
];

/// Methods taking a closure over memory, options or raw input.
const CLOSURE_METHODS: [&str; 5] = ["memory", "memory_mut", "options", "options_mut", "input_mut"];

/// Methods of pass control, plugins and focus movement.
const METHODS: [&str; 6] = ["begin_pass", "end_pass", "on_begin_pass", "on_end_pass", "add_plugin", "move_focus"];

pub(super) fn check(cx: &Cx<'_>, out: &mut Vec<Finding>) {
    for site in &cx.sites {
        match &site.kind {
            Kind::Path { path, .. } if is_egui(path) => {
                let data_types = cx.scope.layer(cx.path) == Some(DATA_LAYER);
                let accesskit_root = path.first().filter(|f| *f == "accesskit" && !data_types);
                if let Some(seg) = path.iter().skip(1).find(|s| SEGMENTS.contains(&s.as_str())).or(accesskit_root) {
                    cx.flag(
                        out,
                        ID,
                        ID,
                        site,
                        format!("`{}` is an egui behavioural API (`{seg}`); reach it through `runtime::egui`", path.join("::")),
                    );
                }
            }
            Kind::Method { name, args, .. } if METHODS.contains(&name.as_str()) || (args.has_closure && CLOSURE_METHODS.contains(&name.as_str())) => {
                cx.flag(out, ID, ID, site, format!("{} is an egui behavioural API; reach it through `runtime::egui`", super::call(name)));
            }
            _ => {}
        }
    }
}
