//! `craft run --headless`: load a plugin into the real plugin host and call one contribution.
//!
//! "Headless" means a host with no document and no app commands. Everything else is the code an
//! app runs: the policy gate, the wasmi engine, the permission prompt (answered by flags), the
//! command registry, budgets, and the call lifecycle. `host.log` lines, notices and faults print to
//! stderr. The contribution's JSON answer prints to stdout.
//!
//! The headless sessions of the apps (their `<app>-cli` programs) are not wired in. A plugin
//! that calls `cmd.execute` or `doc.query` gets what an app with no commands would give it.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use craft_command::{CommandEntry, Owner, Registry};
use craft_plugin_abi::manifest::{Contribution, Kind, Manifest};
use craft_plugin_abi::permissions::DocScope;
use craft_plugin_abi::{HostPolicy, Permission};
use craft_plugin_host::budget::BudgetConfig;
use craft_plugin_host::contrib::command::command_id;
use craft_plugin_host::dispatch::DispatchConfig;
use craft_plugin_host::engine::wasmi::WasmiEngine;
use craft_plugin_host::engine::{Clock, SystemClock, Trust};
use craft_plugin_host::invoke::CallError;
use craft_plugin_host::lifecycle::{Decision, GrantRequest, InstallOutcome, LifecycleError, ManagerConfig, PluginManager};
use craft_plugin_host::services::{
    CallOutcome, CallScope, CommandFailure, CommandInfo, HostInfo, LinkApp, LinkFailure, LogLevel, QueryFailure, Services, UserNotice,
};
use craft_plugin_host::store::{MemoryStore, Source};
use serde_json::{Value, json};

use crate::error::{CliError, Result};
use crate::gate;

/// The app session type of the headless host. It holds nothing, because there is no document.
pub struct Session;

/// The app error type of the headless host: a plugin call error.
#[derive(Debug)]
pub struct AppError(pub CallError);

impl From<CallError> for AppError {
    fn from(e: CallError) -> Self {
        AppError(e)
    }
}

static NO_BUILTINS: [craft_command::CommandSpec<Session, AppError>; 0] = [];

/// One thing the host told the user or the app.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Log { plugin: String, level: &'static str, message: String },
    Progress { fraction: f64, message: Option<String> },
    Notice { plugin: String, message: String },
    CallBegin { contribution: String },
    CallEnd { outcome: String },
}

impl Event {
    fn line(&self) -> String {
        match self {
            Event::Log { plugin, level, message } => format!("[{plugin} {level}] {message}"),
            Event::Progress { fraction, message } => format!("[progress {:.0}%] {}", fraction * 100.0, message.as_deref().unwrap_or("")),
            Event::Notice { plugin, message } => format!("[notice {plugin}] {message}"),
            Event::CallBegin { contribution } => format!("[call begin] {contribution}"),
            Event::CallEnd { outcome } => format!("[call end] {outcome}"),
        }
    }
}

/// `Services` for a host with no app behind it. Every answer is real for that situation: no
/// commands, no document, no linked apps. Events are recorded and, when `echo` is set, printed.
pub struct HeadlessServices {
    app: String,
    echo: bool,
    events: Mutex<Vec<Event>>,
    next_scope: AtomicU64,
}

impl HeadlessServices {
    fn new(app: &str, echo: bool) -> HeadlessServices {
        HeadlessServices { app: app.to_owned(), echo, events: Mutex::new(Vec::new()), next_scope: AtomicU64::new(1) }
    }

    fn record(&self, event: Event) {
        if self.echo {
            eprintln!("{}", event.line());
        }
        self.events.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(event);
    }

    pub fn events(&self) -> Vec<Event> {
        self.events.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }
}

fn level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
    }
}

impl Services for HeadlessServices {
    fn host_info(&self) -> HostInfo {
        HostInfo { app: self.app.clone(), version: format!("craft-cli {}", env!("CARGO_PKG_VERSION")) }
    }

    fn log(&self, owner: &Owner, level: LogLevel, message: &str) {
        self.record(Event::Log { plugin: owner.plugin_id.to_string(), level: level_name(level), message: message.to_owned() });
    }

    fn progress(&self, _owner: &Owner, fraction: f64, message: Option<&str>) {
        self.record(Event::Progress { fraction, message: message.map(str::to_owned) });
    }

    fn list_commands(&self) -> Vec<CommandInfo> {
        Vec::new()
    }

    fn execute_command(&self, _owner: &Owner, _id: &str, _params: &Value) -> std::result::Result<Value, CommandFailure> {
        // No app commands exist here, so every id is unknown. The guest sees E_BAD_ARG.
        Err(CommandFailure::Unknown)
    }

    fn doc_query(&self, _owner: &Owner, _scope: DocScope, _query: &Value) -> std::result::Result<Value, QueryFailure> {
        Err(QueryFailure::Unsupported)
    }

    fn unix_millis(&self) -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }

    fn fill_random(&self, buf: &mut [u8]) {
        if let Err(e) = getrandom::getrandom(buf) {
            // The OS has no entropy source. Say so, and fall back to the std hasher's per-process random keys.
            eprintln!("craft: getrandom failed ({e}); random.bytes uses std RandomState instead");
            use std::hash::{BuildHasher, Hasher};
            for (i, byte) in buf.iter_mut().enumerate() {
                let mut h = std::collections::hash_map::RandomState::new().build_hasher();
                h.write_usize(i);
                *byte = h.finish().to_le_bytes()[0];
            }
        }
    }

    fn notify(&self, notice: UserNotice) {
        self.record(Event::Notice { plugin: notice.plugin_id, message: notice.message });
    }

    fn link_apps(&self, _owner: &Owner) -> std::result::Result<Vec<LinkApp>, LinkFailure> {
        Ok(Vec::new())
    }

    fn link_call(
        &self,
        _owner: &Owner,
        app: &str,
        _method: &str,
        _params: &Value,
        _cancel: &std::sync::atomic::AtomicBool,
    ) -> std::result::Result<Value, LinkFailure> {
        Err(LinkFailure::NotRunning(format!("headless mode has no running app {app:?}")))
    }

    fn lend_session(&self, _session: &mut dyn std::any::Any, run: &mut dyn FnMut()) {
        // Headless mode has no document, so no host call needs the session.
        run();
    }

    fn begin_call(&self, _owner: &Owner, contribution: &str) -> CallScope {
        self.record(Event::CallBegin { contribution: contribution.to_owned() });
        CallScope(self.next_scope.fetch_add(1, Ordering::Relaxed))
    }

    fn end_call(&self, _scope: CallScope, outcome: &CallOutcome) {
        let text = match outcome {
            CallOutcome::Completed => "completed".to_owned(),
            // A real app restores its undo checkpoint here. This host has no document to restore.
            CallOutcome::Faulted(reason) => format!("faulted ({reason}); an app would undo the call, headless has nothing to undo"),
            CallOutcome::Cancelled => "cancelled".to_owned(),
        };
        self.record(Event::CallEnd { outcome: text });
    }
}

/// What `install` did.
#[derive(Debug, PartialEq, Eq)]
pub struct Installed {
    pub plugin_id: String,
    pub generation: u64,
    pub replaced: Option<u64>,
    /// Permissions granted without a prompt, as written in the manifest.
    pub granted: Vec<String>,
}

/// A plugin host with no app.
pub struct Headless {
    pub manager: Arc<PluginManager<Session, AppError>>,
    pub registry: Arc<Registry<Session, AppError>>,
    pub services: Arc<HeadlessServices>,
    manifests: Mutex<BTreeMap<String, Manifest>>,
}

fn lifecycle(e: LifecycleError) -> CliError {
    CliError::new(e.to_string())
}

impl Headless {
    /// `app` is the name `host.info` reports. `echo` prints events to stderr as they happen.
    pub fn new(app: &str, echo: bool) -> Headless {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let engine = Arc::new(WasmiEngine::new(Arc::clone(&clock)));
        let store = Arc::new(MemoryStore::new());
        let services = Arc::new(HeadlessServices::new(app, echo));
        let registry = Arc::new(Registry::new(&NO_BUILTINS));
        // The developer runs this next to cargo, often on an unoptimised host build. A 250 ms wall default
        // would fault a correct plugin when the machine is busy, so an interactive call gets the full
        // 2 s cap of the budget table. The cap itself, fuel, memory and the circuit breaker stay as in the apps.
        let budgets = BudgetConfig::default();
        let budgets = BudgetConfig { interactive_default: budgets.interactive_cap, ..budgets };
        let cfg = ManagerConfig { policy: HostPolicy::native(), trust: Trust::Untrusted, budgets, dispatch: DispatchConfig::default() };
        let manager = PluginManager::new(engine, cfg, Arc::clone(&registry), store.clone(), store, services.clone(), clock);
        Headless { manager, registry, services, manifests: Mutex::new(BTreeMap::new()) }
    }

    /// The prompt, answered. Required permissions are granted because the developer asked to run
    /// this plugin. Optional ones are granted only if named in `optional`.
    fn answer(&self, request: &GrantRequest, optional: &[Permission]) -> Result<Decision> {
        if let Some(p) = optional.iter().find(|p| !request.optional.contains(p)) {
            return Err(CliError::new(format!(
                "--grant {p}: the manifest does not list it as optional; it lists {:?}",
                request.optional.iter().map(ToString::to_string).collect::<Vec<_>>()
            )));
        }
        Ok(Decision { accept_required: true, optional: optional.to_vec() })
    }

    /// Install or replace a plugin. A first install of a plugin with permissions is granted
    /// headlessly: required ones always, optional ones from `optional`.
    pub fn install(&self, bytes: &[u8], source: Source, optional: &[Permission]) -> Result<Installed> {
        let manifest = gate::check(bytes).manifest;
        let outcome = self.manager.install(bytes, source).map_err(lifecycle)?;
        let (outcome, granted) = match outcome {
            InstallOutcome::NeedsGrant(request) => {
                let decision = self.answer(&request, optional)?;
                let mut granted: Vec<String> = request.required.iter().map(ToString::to_string).collect();
                granted.extend(decision.optional.iter().map(ToString::to_string));
                (self.manager.grant(&request.plugin_id, decision).map_err(lifecycle)?, granted)
            }
            other => (other, Vec::new()),
        };
        let manifest = manifest.ok_or_else(|| CliError::new("the package has no manifest"))?;
        let plugin_id = manifest.id.clone();
        match outcome {
            InstallOutcome::Loaded { generation, replaced } => {
                self.manifests.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(plugin_id.clone(), manifest);
                Ok(Installed { plugin_id, generation, replaced, granted })
            }
            InstallOutcome::StoredDisabled => Err(CliError::new(format!("{plugin_id} was stored disabled, not loaded"))),
            InstallOutcome::UpdateDeclined => Err(CliError::new(format!("{plugin_id}: the update was declined"))),
            InstallOutcome::NeedsGrant(_) => Err(CliError::new(format!("{plugin_id} still waits for a permission answer"))),
        }
    }

    /// The manifest of an installed plugin.
    pub fn manifest(&self, plugin_id: &str) -> Option<Manifest> {
        self.manifests.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(plugin_id).cloned()
    }

    /// Call one contribution.
    ///
    /// - `command` and `menu`: `arg` is the params. The call goes through the command registry, the
    ///   path an app's palette and scripts use.
    /// - `hook`, `panel`: `arg` is the whole request JSON the handler takes (`{"event":"open"}`),
    ///   sent straight to the plugin's active generation.
    /// - data plane kinds and unknown kinds: refused with the reason.
    pub fn invoke(&self, plugin_id: &str, target: &str, arg: &Value) -> Result<Value> {
        let manifest = self.manifest(plugin_id).ok_or_else(|| CliError::new(format!("{plugin_id} is not installed")))?;
        let c = resolve_contribution(&manifest, target)?;
        match &c.kind {
            Kind::Command | Kind::Menu => {
                let id = command_id(plugin_id, &c.id);
                match self.registry.resolve(&id) {
                    Some(CommandEntry::Dynamic(cmd)) => (cmd.run)(&mut Session, arg).map_err(|AppError(e)| CliError::new(e.to_string())),
                    Some(CommandEntry::Static(_)) | None => Err(CliError::new(format!(
                        "{id} is not in the command registry (a menu contribution registers only when the plugin holds ui.contribute)"
                    ))),
                }
            }
            Kind::Hook | Kind::Panel => {
                let generation =
                    self.manager.router().current(plugin_id).ok_or_else(|| CliError::new(format!("{plugin_id} has no active generation")))?;
                generation.call(c, arg, &[], 0).map_err(|e| CliError::new(e.to_string()))
            }
            kind if kind.is_data_plane() => Err(CliError::new(format!(
                "{} has kind {}. It runs through craft_process on pixel bands, and headless run has no image source",
                c.id,
                kind.name()
            ))),
            kind => Err(CliError::new(format!("{} has kind {:?}, which this build cannot run", c.id, kind.name()))),
        }
    }
}

/// Find the contribution `target` names.
///
/// `target` may be the contribution id (`say`), the plugin id and contribution id joined
/// (`dev.local.hello.say`), the registry id (`plugin.dev.local.hello.say`), or any dot-separated tail
/// of those, such as `hello.say`. A tail must match exactly one contribution.
pub fn resolve_contribution<'a>(manifest: &'a Manifest, target: &str) -> Result<&'a Contribution> {
    let full = |c: &Contribution| format!("{}.{}", manifest.id, c.id);
    let matches: Vec<&Contribution> = manifest
        .contributions
        .iter()
        .filter(|c| {
            let joined = full(c);
            c.id == target || joined == target || format!("plugin.{joined}") == target || joined.ends_with(&format!(".{target}"))
        })
        .collect();
    match matches.as_slice() {
        [one] => Ok(one),
        [] => {
            let ids: Vec<String> = manifest.contributions.iter().map(|c| format!("{} ({})", c.id, c.kind.name())).collect();
            Err(CliError::new(format!("no contribution matches {target:?}; {} has: {}", manifest.id, ids.join(", "))))
        }
        many => Err(CliError::new(format!(
            "{target:?} matches {} contributions: {}",
            many.len(),
            many.iter().map(|c| c.id.as_str()).collect::<Vec<_>>().join(", ")
        ))),
    }
}

/// Parse `--invoke ID [JSON]`.
pub fn parse_invoke(args: &[String]) -> Result<(String, Value)> {
    let Some(id) = args.first() else {
        return Err(CliError::new("--invoke needs a contribution id"));
    };
    let arg = match args.get(1) {
        Some(text) => serde_json::from_str(text).map_err(|e| CliError::new(format!("--invoke JSON argument: {e}")))?,
        None => json!({}),
    };
    Ok((id.clone(), arg))
}

/// Parse repeated `--grant` values.
pub fn parse_grants(list: &[String]) -> Result<Vec<Permission>> {
    list.iter().map(|s| s.parse::<Permission>().map_err(|e| CliError::new(format!("--grant {s:?}: {e}")))).collect()
}

/// `Headless` as the target of a developer folder watcher.
pub struct DevHost {
    pub host: Arc<Headless>,
    /// Optional permissions granted on every install.
    pub optional: Vec<Permission>,
}

impl crate::devfolder::DevTarget for DevHost {
    fn install(&self, bytes: &[u8]) -> Result<Installed> {
        self.host.install(bytes, Source::DevFolder, &self.optional)
    }

    fn remove(&self, plugin_id: &str) -> Result<()> {
        self.host.manager.remove(plugin_id).map_err(lifecycle)
    }
}
