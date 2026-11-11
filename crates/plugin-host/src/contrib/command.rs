//! `command` and `menu` contributions become `DynCommand`s in the app's registry.
//!
//! Id: `plugin.<plugin_id>.<contribution_id>`. Owner: (plugin id, generation). Enablement is the
//! manifest's declarative value, so a menu never calls plugin code to decide.
//! The run closure routes through the [`Router`], so a command registered under generation N
//! calls generation N+1 after a reload without a second registration.

use std::sync::Arc;

use craft_command::{DynCommand, Enablement, Owner, RegisterError, Registry, Visibility};
use craft_plugin_abi::manifest::{Contribution, Enabled, Kind, Manifest, ParamSpec, ParamType};
use serde_json::{Map, Value, json};

use crate::invoke::{CallError, Router};
use crate::services::{Services, lend};

/// The registry id of a plugin command.
pub fn command_id(plugin_id: &str, contribution_id: &str) -> String {
    format!("plugin.{plugin_id}.{contribution_id}")
}

/// True for the kinds this module registers.
pub fn is_command_kind(kind: &Kind) -> bool {
    matches!(kind, Kind::Command | Kind::Menu)
}

fn param_schema(spec: &ParamSpec) -> Value {
    let mut m = Map::new();
    match spec.ty {
        ParamType::Number | ParamType::Angle => {
            m.insert("type".into(), json!("number"));
        }
        ParamType::Int => {
            m.insert("type".into(), json!("integer"));
        }
        ParamType::Bool => {
            m.insert("type".into(), json!("boolean"));
        }
        ParamType::Choice => {
            m.insert("type".into(), json!("string"));
            if let Some(options) = &spec.options {
                m.insert("enum".into(), json!(options));
            }
        }
        ParamType::Text | ParamType::Layer | ParamType::File => {
            m.insert("type".into(), json!("string"));
        }
        ParamType::Color => {
            m.insert("type".into(), json!("array"));
            m.insert("minItems".into(), json!(4));
            m.insert("maxItems".into(), json!(4));
            m.insert("items".into(), json!({ "type": "number" }));
        }
        ParamType::Point2 | ParamType::Point3 => {
            let n = if spec.ty == ParamType::Point2 { 2 } else { 3 };
            m.insert("type".into(), json!("array"));
            m.insert("minItems".into(), json!(n));
            m.insert("maxItems".into(), json!(n));
            m.insert("items".into(), json!({ "type": "number" }));
        }
        ParamType::Json => {}
    }
    if let Some(min) = spec.min {
        m.insert("minimum".into(), json!(min));
    }
    if let Some(max) = spec.max {
        m.insert("maximum".into(), json!(max));
    }
    if let Some(default) = &spec.default {
        m.insert("default".into(), default.clone());
    }
    if let Some(label) = &spec.label {
        m.insert("description".into(), json!(label));
    }
    Value::Object(m)
}

/// The command's params doc: a JSON Schema object, which the command layer keeps as is
/// (`schema::from_doc` source `doc`). Params with a `default` are not required.
pub fn params_doc(c: &Contribution) -> String {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for (name, spec) in c.params.iter().flatten() {
        properties.insert(name.clone(), param_schema(spec));
        if spec.default.is_none() {
            required.push(json!(name));
        }
    }
    json!({ "type": "object", "properties": properties, "required": required }).to_string()
}

fn enablement(c: &Contribution) -> Enablement {
    match c.enabled() {
        Enabled::Always => Enablement::Always,
        Enabled::HasDocument => Enablement::HasDocument,
        Enabled::HasSelection => Enablement::HasSelection,
        // The plugin reports the state through `Registry::set_enabled`. Until then the command is on.
        Enabled::Cached => Enablement::Cached(Ok(())),
    }
}

/// Build the `DynCommand` for one `command` or `menu` contribution. `None` for any other kind, and
/// for a `menu` contribution when the plugin lacks `ui.contribute`, because a menu entry is all it is.
/// Without `ui.contribute` a `command` stays in the palette and the registry but gets no menu path
/// and no shortcut (default deny).
pub fn dyn_command<S: 'static, E: From<CallError> + 'static>(
    owner: &Owner,
    c: &Arc<Contribution>,
    router: &Arc<Router>,
    services: &Arc<dyn Services>,
    ui_contribute: bool,
) -> Option<DynCommand<S, E>> {
    if !is_command_kind(&c.kind) || (c.kind == Kind::Menu && !ui_contribute) {
        return None;
    }
    let menu_path: Vec<Arc<str>> = if ui_contribute { c.menu.iter().flatten().map(|s| Arc::from(s.as_str())).collect() } else { Vec::new() };
    let in_menu = ui_contribute && (c.kind == Kind::Menu || !menu_path.is_empty());
    let shortcut = c.shortcut.as_deref().filter(|_| ui_contribute);
    let plugin = owner.plugin_id.to_string();
    let contribution = Arc::clone(c);
    let router = Arc::clone(router);
    let services = Arc::clone(services);
    let run = move |session: &mut S, params: &Value| -> Result<Value, E> {
        let generation = router.current(&plugin).ok_or(CallError::NotLoaded)?;
        let request = json!({ "params": params, "origin": "unknown" });
        // The body keeps the real session. The app lends it to host calls only while the guest runs.
        let response = lend(&*services, session, || generation.call(&contribution, &request, &[], 0))
            .ok_or_else(|| CallError::NotCallable("the app did not run the plugin call".into()))??;
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    };
    Some(DynCommand {
        id: Arc::from(command_id(&owner.plugin_id, &c.id)),
        label: Arc::from(c.label.as_deref().unwrap_or(&c.id)),
        params: Arc::from(params_doc(c)),
        shortcut: shortcut.map(Arc::from),
        menu: Arc::from(menu_path),
        owner: owner.clone(),
        enablement: enablement(c),
        visibility: Visibility { menu: in_menu, palette: c.kind == Kind::Command, shortcut: shortcut.is_some() },
        journal: true,
        run: Arc::new(run),
    })
}

/// Why `register_all` stopped. The registry holds none of the plugin's commands afterward.
#[derive(Debug, PartialEq, Eq)]
pub struct RegisterFailure {
    pub command_id: String,
    pub error: RegisterError,
}

impl std::fmt::Display for RegisterFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.command_id, self.error)
    }
}

impl std::error::Error for RegisterFailure {}

/// Register every command and menu contribution of `manifest` under `owner`. All or none.
/// Returns the registered ids.
pub fn register_all<S: 'static, E: From<CallError> + 'static>(
    registry: &Registry<S, E>,
    owner: &Owner,
    manifest: &Manifest,
    router: &Arc<Router>,
    services: &Arc<dyn Services>,
    ui_contribute: bool,
) -> Result<Vec<String>, RegisterFailure> {
    let mut ids = Vec::new();
    for c in manifest.contributions.iter().map(|c| Arc::new(c.clone())) {
        let Some(cmd) = dyn_command::<S, E>(owner, &c, router, services, ui_contribute) else {
            continue;
        };
        let id = cmd.id.to_string();
        if let Err(error) = registry.register(cmd) {
            registry.unregister_owner(owner);
            return Err(RegisterFailure { command_id: id, error });
        }
        ids.push(id);
    }
    Ok(ids)
}
