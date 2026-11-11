//! `contract/egui-widget`: egui's widget constructors and `Ui` shortcuts outside `runtime::egui`.

use crate::report::Finding;

use super::Cx;
use super::sites::Kind;

pub const ID: &str = "contract/egui-widget";

/// `Ui` widget shortcuts. Only calls on a receiver named `ui` count: models and builders have their own `radio`, `link` and `button`.
const SHORTCUTS: [&str; 12] = [
    "button",
    "small_button",
    "checkbox",
    "selectable_label",
    "selectable_value",
    "radio",
    "radio_value",
    "hyperlink",
    "hyperlink_to",
    "link",
    "separator",
    "image",
];

/// `Ui` text shortcuts. Only calls on a receiver named `ui` count: components and builders have their own `label`.
const UI_TEXT: [&str; 8] = ["label", "heading", "monospace", "colored_label", "code", "small", "text_edit_singleline", "text_edit_multiline"];

/// Egui text types the library crates do not build outside `runtime::egui`.
const TEXT_TYPES: [&str; 2] = ["RichText", "Label"];

const WIDGETS: [&str; 5] = ["Button", "TextEdit", "DragValue", "Slider", "ComboBox"];

/// An argument that reads a design token: it mentions `tokens`, `Tokens` or `token`.
fn is_token(idents: &[String]) -> bool {
    idents.iter().any(|i| matches!(i.as_str(), "tokens" | "Tokens" | "token"))
}

pub(super) fn check(cx: &Cx<'_>, out: &mut Vec<Finding>) {
    for site in &cx.sites {
        if let Kind::Method { name, receiver, .. } = &site.kind
            && SHORTCUTS.contains(&name.as_str())
            && receiver.last().is_some_and(|r| r == "ui")
        {
            cx.flag(out, ID, ID, site, format!("{} is an egui widget shortcut; the contract's parts draw widgets", super::call(name)));
        }
    }
    // Tools (crates with no layer-map entry) draw with egui directly; only library modules are judged here.
    let library = cx.scope.layer(cx.path).is_some();
    for site in cx.sites.iter().filter(|_| library) {
        if let Kind::Path { .. } = &site.kind
            && let Some(path) = site.egui_path()
            && path.iter().any(|s| TEXT_TYPES.contains(&s.as_str()))
        {
            cx.flag(out, ID, ID, site, format!("`{}` builds egui text; the contract's parts draw text", path.join("::")));
        }
        if let Kind::Method { name, receiver, .. } = &site.kind
            && UI_TEXT.contains(&name.as_str())
            && receiver.last().is_some_and(|r| r == "ui")
        {
            cx.flag(out, ID, ID, site, format!("{} is an egui text shortcut; the contract's parts draw text", super::call(name)));
        }
    }
    // Literal arguments are `literal-size/add-space`.
    cx.flag_methods(out, ID, ID, "takes a space that is not a token", |name, args| {
        name == "add_space" && !args.has_literal && !is_token(&args.idents)
    });
    for site in &cx.sites {
        if let Kind::Path { call: true, .. } = &site.kind
            && let Some(path) = site.egui_path()
            && path.iter().any(|s| WIDGETS.contains(&s.as_str()))
        {
            cx.flag(out, ID, ID, site, format!("`{}` constructs an egui widget; the contract's parts draw widgets", path.join("::")));
        }
    }
}
