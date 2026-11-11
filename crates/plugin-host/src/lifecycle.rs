//! Install, grant, load, reload, disable, and remove.
//!
//! The manager keeps one rule above all: a step that fails leaves the active generation N
//! running and registered. Every fallible step of a reload therefore runs before the first step
//! that changes what the app sees, and the steps that change it have an undo.
//!
//! ```text
//! reload N -> N+1
//!   1 gate + compile           (nothing changed yet)
//!   2 craft_state_save on N    (250 ms; a failure aborts)
//!   3 craft_state_load on N+1  (runs before the switch, so a failure keeps N; ordering it
//!                               after the switch would lose N)
//!   4 persist N+1              (a failure aborts)
//!   5 unregister N, register N+1 (a failure re-registers N and restores the old record)
//!   6 route new calls to N+1   (calls already running on N finish on N, then N drops)
//! ```
//!
//! There is no observable "reloading" state. The manager holds its lock for the whole reload, and
//! command calls never take that lock, so they keep running on N until step 6.
//!
//! The manager needs an engine whose futures finish on the first poll (`Caps::SYNC_CALL`), because
//! command closures are synchronous. Native only, for the reason in [`crate::contrib`].

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex, Weak};

use craft_command::{Owner, Registry};
use craft_plugin_abi::manifest::Manifest;
use craft_plugin_abi::{HostPolicy, LegacyAbi, Permission, check_package};

use crate::budget::{BudgetConfig, FaultTracker};
use crate::contrib::command::{RegisterFailure, register_all};
use crate::contrib::hook::{HookRegistrar, HookRegistration};
use crate::contrib::surface::{SurfaceRegistration, SurfaceSink, register_surfaces};
use crate::dispatch::DispatchConfig;
use crate::engine::{CompileOptions, Engine, LoadError, Trust, load_package};
use crate::invoke::{Backend, CallError, Generation, Router, Shared, poll_ready};
use crate::services::Services;
use crate::store::{Decider, PackageStore, Source, StateStore, StoreError, StoredGrant, StoredPlugin};

/// Where an installed plugin stands. Rejected and Removed are not states: a rejected package is an
/// `Err` from `install`, and a removed plugin is absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginState {
    /// A new plugin waits for the user to accept its permissions.
    AwaitingGrant,
    /// Installed and switched off, or declined at the prompt.
    Disabled,
    /// Gate or compile or registration failed. The plugin stays installed.
    Faulted {
        reason: String,
    },
    Active {
        generation: u64,
    },
}

/// Why an update to an installed plugin needs a new answer from the user.
///
/// A package with an installed plugin's id replaces its code. The old grants were given to the old
/// code and to the old publisher, so they carry over only when the new package comes from the same
/// publisher and has a higher version. `author` is the only publisher name a package has. It is not
/// verified (the signature section is reserved), so this check stops accidents and lazy swaps, and
/// it does not stop an author who copies the name. Two packages with no author count as one publisher.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateRisk {
    /// A first install, an identical package, or a clean update. Grants carry over.
    None,
    /// The new package names another publisher, or names one where the installed code names none,
    /// or the reverse. `installed` is the publisher of the code that runs now.
    Publisher { installed: Option<String> },
    /// Same publisher, but the version does not go up. A downgrade, a re-release, or a version
    /// string that is not semver.
    NotNewer { installed: String },
}

impl UpdateRisk {
    pub fn is_risky(&self) -> bool {
        *self != UpdateRisk::None
    }
}

impl fmt::Display for UpdateRisk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpdateRisk::None => f.write_str("update"),
            UpdateRisk::Publisher { installed: Some(p) } => write!(f, "different publisher (installed code is from {p:?})"),
            UpdateRisk::Publisher { installed: None } => f.write_str("the installed code names no publisher and this package names one"),
            UpdateRisk::NotNewer { installed } => write!(f, "version is not higher than the installed {installed}"),
        }
    }
}

/// What the user must decide.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrantRequest {
    pub plugin_id: String,
    pub version: String,
    pub publisher: Option<String>,
    /// Every required permission of the new version.
    pub required: Vec<Permission>,
    /// The part of `required` that no earlier grant covers. Equal to `required` on a first install.
    pub new_required: Vec<Permission>,
    pub optional: Vec<Permission>,
    /// Not `None` when this package replaces an installed plugin and the old grants would not carry
    /// over. `new_required` then equals `required`, and an accepted answer grants only what the
    /// user accepts now.
    pub risk: UpdateRisk,
}

/// The user's answer.
#[derive(Clone, Debug)]
pub struct Decision {
    /// False declines. A first install is then stored disabled. An update is dropped and the old version stays.
    pub accept_required: bool,
    /// Optional permissions the user accepts. Each must appear in the manifest.
    pub optional: Vec<Permission>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallOutcome {
    /// The plugin is active. `replaced` is the generation it took over from.
    Loaded { generation: u64, replaced: Option<u64> },
    /// The user must answer a prompt. Call `grant`.
    NeedsGrant(GrantRequest),
    /// Stored, not loaded: the plugin was disabled before, or the user declined.
    StoredDisabled,
    /// The user declined an update. The old version keeps running.
    UpdateDeclined,
}

#[derive(Debug)]
pub enum LifecycleError {
    /// The gate or the engine refused the package.
    Rejected(LoadError),
    Manifest(String),
    Store(StoreError),
    NoSuchPlugin(String),
    /// `grant` without a pending prompt.
    NothingPending(String),
    /// A decision named an optional permission the manifest does not list.
    NotRequested(String),
    /// `craft_state_save` failed on the old generation. The old generation keeps running.
    StateSave(CallError),
    /// `craft_state_load` failed on the new generation. The old generation keeps running.
    StateLoad(CallError),
    /// A contribution could not register. The old generation keeps running.
    Register(RegisterFailure),
    /// A contribution could not register and the old generation could not be put back.
    /// The plugin is `Faulted`.
    RollbackFailed {
        cause: RegisterFailure,
        restore: RegisterFailure,
    },
    Call(CallError),
    /// A required permission has no grant.
    MissingGrant(String),
    /// A v1 plugin (`ec_`, `pc_`, `vc_`) did not load through its shim.
    Legacy(String),
    /// The id equals an installed plugin's id except for ASCII case. Both would share one
    /// directory on a case-insensitive file system.
    IdCollision {
        id: String,
        existing: String,
    },
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LifecycleError::Rejected(e) => write!(f, "{e}"),
            LifecycleError::Manifest(m) => write!(f, "manifest: {m}"),
            LifecycleError::Store(e) => write!(f, "{e}"),
            LifecycleError::NoSuchPlugin(id) => write!(f, "no plugin {id}"),
            LifecycleError::NothingPending(id) => write!(f, "plugin {id} has no pending permission prompt"),
            LifecycleError::IdCollision { id, existing } => {
                write!(f, "plugin id {id:?} differs from the installed {existing:?} only by letter case; ids are case-insensitive")
            }
            LifecycleError::NotRequested(p) => write!(f, "permission {p} is not in the manifest"),
            LifecycleError::StateSave(e) => write!(f, "craft_state_save failed: {e}"),
            LifecycleError::StateLoad(e) => write!(f, "craft_state_load failed: {e}"),
            LifecycleError::Register(e) => write!(f, "cannot register {e}"),
            LifecycleError::RollbackFailed { cause, restore } => write!(f, "cannot register {cause}, and cannot restore the old version: {restore}"),
            LifecycleError::Call(e) => write!(f, "{e}"),
            LifecycleError::MissingGrant(p) => write!(f, "required permission {p} is not granted"),
            LifecycleError::Legacy(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for LifecycleError {}

/// What the plugin list says about a v1 plugin: which ABI it speaks and the limits it still runs
/// under. A v1 plugin has no permissions and no host ops, so its prompt is empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyInfo {
    pub abi: LegacyAbi,
    /// `(name, value)` pairs such as `("wall time per run", "60 s")`.
    pub limits: Vec<(String, String)>,
}

/// What the UI shows about one plugin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginInfo {
    pub id: String,
    pub version: String,
    pub state: PluginState,
    pub enabled: bool,
    /// Kinds in the manifest this release cannot run. Non-empty means "partly supported".
    pub unsupported_kinds: Vec<String>,
    /// Registry ids of the commands this plugin registered.
    pub commands: Vec<String>,
    /// Hooks that are live, and hooks the host left out with the reason. Empty without `attach_hooks`.
    pub hooks: HookRegistration,
    /// Panels and image or object contributions that are live, and those the host left out with the reason.
    pub surfaces: SurfaceRegistration,
    /// Set for a v1 plugin.
    pub legacy: Option<LegacyInfo>,
    /// A newer version waits for the user to accept new permissions.
    pub update_pending: bool,
    pub grants: Vec<String>,
}

struct Prepared {
    backend: Backend,
    legacy: Option<LegacyInfo>,
    manifest: Arc<Manifest>,
    bytes: Vec<u8>,
    source: Source,
    required: Vec<Permission>,
    optional: Vec<Permission>,
}

struct Entry {
    stored: StoredPlugin,
    manifest: Arc<Manifest>,
    state: PluginState,
    commands: Vec<String>,
    hooks: HookRegistration,
    surfaces: SurfaceRegistration,
    legacy: Option<LegacyInfo>,
    /// Old generations that may still run a call. Dead `Weak`s mean the generation drained.
    retired: Vec<Weak<Generation>>,
}

impl Entry {
    /// An entry with nothing registered.
    fn idle(stored: StoredPlugin, manifest: Arc<Manifest>, state: PluginState, legacy: Option<LegacyInfo>, retired: Vec<Weak<Generation>>) -> Entry {
        Entry {
            stored,
            manifest,
            state,
            commands: Vec::new(),
            hooks: HookRegistration::default(),
            surfaces: SurfaceRegistration::default(),
            legacy,
            retired,
        }
    }
}

#[derive(Default)]
struct Inner {
    plugins: BTreeMap<String, Entry>,
    pending: BTreeMap<String, Prepared>,
    generations: HashMap<String, u64>,
    /// Stored packages that `restore` could not load or even parse: id to reason.
    broken: BTreeMap<String, String>,
}

/// Host wide settings of the manager.
pub struct ManagerConfig {
    pub policy: HostPolicy,
    pub trust: Trust,
    pub budgets: BudgetConfig,
    pub dispatch: DispatchConfig,
}

pub struct PluginManager<S: 'static, E: 'static> {
    engine: Arc<dyn Engine>,
    cfg: ManagerConfig,
    registry: Arc<Registry<S, E>>,
    packages: Arc<dyn PackageStore>,
    shared: Arc<Shared>,
    router: Arc<Router>,
    hooks: Mutex<Option<Arc<dyn HookRegistrar>>>,
    surfaces: Mutex<Option<Arc<dyn SurfaceSink>>>,
    inner: Mutex<Inner>,
}

fn dedup_sorted(v: impl Iterator<Item = String>) -> Vec<String> {
    v.collect::<BTreeSet<_>>().into_iter().collect()
}

fn unsupported_kinds(m: &Manifest) -> Vec<String> {
    dedup_sorted(m.unsupported_contributions().map(|c| c.kind.name().to_owned()))
}

fn parse_grants(stored: &StoredPlugin) -> Vec<Permission> {
    // A grant string that no longer parses is dropped, which can only remove access.
    stored.grants.iter().filter_map(|g| g.permission.parse().ok()).collect()
}

impl<S: 'static, E: From<CallError> + 'static> PluginManager<S, E> {
    pub fn new(
        engine: Arc<dyn Engine>,
        cfg: ManagerConfig,
        registry: Arc<Registry<S, E>>,
        packages: Arc<dyn PackageStore>,
        state: Arc<dyn StateStore>,
        services: Arc<dyn Services>,
        clock: Arc<dyn crate::engine::Clock>,
    ) -> Arc<PluginManager<S, E>> {
        let shared = Arc::new(Shared {
            services,
            state,
            clock,
            faults: Arc::new(FaultTracker::new(cfg.budgets.circuit_threshold)),
            budgets: cfg.budgets.clone(),
            dispatch: cfg.dispatch.clone(),
            engine: engine.id(),
        });
        Arc::new(PluginManager {
            engine,
            cfg,
            registry,
            packages,
            shared,
            router: Router::new(),
            hooks: Mutex::new(None),
            surfaces: Mutex::new(None),
            inner: Mutex::new(Inner::default()),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn router(&self) -> &Arc<Router> {
        &self.router
    }

    pub fn faults(&self) -> &Arc<FaultTracker> {
        &self.shared.faults
    }

    /// Send the `hook` contributions of every plugin to `registrar` from now on: when a generation
    /// loads, reloads, or unloads. Plugins that are active now are registered at once. Call this
    /// before `restore` so no plugin starts without its hooks.
    pub fn attach_hooks(&self, registrar: Arc<dyn HookRegistrar>) {
        let mut inner = self.lock();
        *self.hook_slot() = Some(Arc::clone(&registrar));
        for (id, entry) in inner.plugins.iter_mut() {
            if let (PluginState::Active { .. }, Some(generation)) = (&entry.state, self.router.current(id)) {
                entry.hooks = registrar.register(&generation.owner, &generation.manifest, generation.grants());
            }
        }
    }

    fn hook_slot(&self) -> std::sync::MutexGuard<'_, Option<Arc<dyn HookRegistrar>>> {
        self.hooks.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn register_hooks(&self, generation: &Generation) -> HookRegistration {
        match &*self.hook_slot() {
            Some(r) => r.register(&generation.owner, &generation.manifest, generation.grants()),
            None => HookRegistration::default(),
        }
    }

    fn unregister_hooks(&self, owner: &craft_command::Owner) {
        if let Some(r) = &*self.hook_slot() {
            r.unregister(owner);
        }
    }

    /// Hand the panels and the image and object contributions of every plugin to `sink` from now
    /// on: when a generation loads, reloads, or unloads. Plugins that are active now are added at
    /// once. Call this before `restore` so no plugin starts without its surfaces.
    pub fn attach_surfaces(&self, sink: Arc<dyn SurfaceSink>) {
        let mut inner = self.lock();
        *self.surface_slot() = Some(Arc::clone(&sink));
        for (id, entry) in inner.plugins.iter_mut() {
            if let (PluginState::Active { .. }, Some(generation)) = (&entry.state, self.router.current(id)) {
                entry.surfaces = register_surfaces(&*sink, &self.router, &generation.owner, &generation.manifest, generation.grants());
            }
        }
    }

    fn surface_slot(&self) -> std::sync::MutexGuard<'_, Option<Arc<dyn SurfaceSink>>> {
        self.surfaces.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn register_surfaces(&self, generation: &Generation) -> SurfaceRegistration {
        match &*self.surface_slot() {
            Some(sink) => register_surfaces(&**sink, &self.router, &generation.owner, &generation.manifest, generation.grants()),
            None => SurfaceRegistration::default(),
        }
    }

    fn unregister_surfaces(&self, owner: &craft_command::Owner) {
        if let Some(sink) = &*self.surface_slot() {
            sink.remove(owner);
        }
    }

    // ---- reading --------------------------------------------------------------------------

    pub fn state(&self, id: &str) -> Option<PluginState> {
        let inner = self.lock();
        match inner.plugins.get(id) {
            Some(e) => Some(e.state.clone()),
            None => match inner.broken.get(id) {
                Some(reason) => Some(PluginState::Faulted { reason: reason.clone() }),
                None => inner.pending.contains_key(id).then_some(PluginState::AwaitingGrant),
            },
        }
    }

    pub fn info(&self, id: &str) -> Option<PluginInfo> {
        let inner = self.lock();
        let e = inner.plugins.get(id)?;
        Some(PluginInfo {
            id: id.to_owned(),
            version: e.stored.version.clone(),
            state: e.state.clone(),
            enabled: e.stored.enabled,
            unsupported_kinds: unsupported_kinds(&e.manifest),
            commands: e.commands.clone(),
            hooks: e.hooks.clone(),
            surfaces: e.surfaces.clone(),
            legacy: e.legacy.clone(),
            update_pending: inner.pending.contains_key(id),
            grants: e.stored.grants.iter().map(|g| g.permission.clone()).collect(),
        })
    }

    pub fn list(&self) -> Vec<String> {
        self.lock().plugins.keys().cloned().collect()
    }

    /// Retired generations of `id` that still have a call running.
    pub fn draining(&self, id: &str) -> usize {
        let mut inner = self.lock();
        let Some(e) = inner.plugins.get_mut(id) else {
            return 0;
        };
        e.retired.retain(|w| w.strong_count() > 0);
        e.retired.len()
    }

    /// Judge a document reference against the active plugins.
    pub fn ref_status(&self, r: &crate::missing::MissingPluginRef) -> crate::missing::RefStatus {
        let inner = self.lock();
        let active = inner.plugins.get(&r.plugin_id).filter(|e| matches!(e.state, PluginState::Active { .. }));
        let version = active.map(|e| e.stored.version.as_str());
        let contribution = active.and_then(|e| e.manifest.contributions.iter().find(|c| c.id == r.contribution_id)).map(|c| c.kind.is_known());
        r.status(version, contribution)
    }

    // ---- gate and compile -----------------------------------------------------------------

    fn prepare(&self, bytes: &[u8], source: Source) -> Result<Prepared, LifecycleError> {
        let report = check_package(bytes, &self.cfg.policy);
        #[cfg(feature = "legacy-v1")]
        if report.legacy.is_some() {
            return self.prepare_legacy(bytes, source);
        }
        let stack_bytes = report.manifest.as_ref().map_or(self.cfg.budgets.default_stack_kb * 1024, |m| self.cfg.budgets.limits_for(m).stack_bytes);
        let opts = CompileOptions { trust: self.cfg.trust, stack_bytes };
        let (module, report) = poll_ready(Box::pin(load_package(&*self.engine, bytes, &self.cfg.policy, opts)))
            .map_err(|e| LifecycleError::Rejected(LoadError::Compile(e.to_string())))?
            .map_err(LifecycleError::Rejected)?;
        let manifest = report.manifest.map(Arc::new).ok_or_else(|| LifecycleError::Manifest("accepted package has no manifest".into()))?;
        let (required, optional) = manifest.parsed_permissions().map_err(|e| LifecycleError::Manifest(e.to_string()))?;
        Ok(Prepared { backend: Backend::Craft(module), legacy: None, manifest, bytes: bytes.to_vec(), source, required, optional })
    }

    /// A v1 package skips the craft-abi 1 gate. Its shim loads it with the v1
    /// limits, and describes it as a craft-abi 1 manifest with no permissions, so the rest of the
    /// manager treats it like any other plugin.
    #[cfg(feature = "legacy-v1")]
    fn prepare_legacy(&self, bytes: &[u8], source: Source) -> Result<Prepared, LifecycleError> {
        let fail = |e: crate::shim::ShimError| LifecycleError::Legacy(e.to_string());
        let plugin = crate::shim::load_legacy(bytes, Arc::clone(&self.shared.clock)).map_err(fail)?;
        let manifest = Arc::new(plugin.craft_manifest().map_err(fail)?);
        let (required, optional) = manifest.parsed_permissions().map_err(|e| LifecycleError::Manifest(e.to_string()))?;
        let legacy = LegacyInfo { abi: plugin.abi(), limits: plugin.limit_lines().into_iter().map(|l| (l.name.to_owned(), l.value)).collect() };
        Ok(Prepared { backend: Backend::Legacy(Arc::new(plugin)), legacy: Some(legacy), manifest, bytes: bytes.to_vec(), source, required, optional })
    }

    fn generation_for(&self, p: &Prepared, grants: Vec<Permission>, number: u64) -> Arc<Generation> {
        let owner = Owner { plugin_id: Arc::from(p.manifest.id.as_str()), generation: number };
        let limits = self.cfg.budgets.limits_for(&p.manifest);
        let manifest = Arc::clone(&p.manifest);
        let shared = Arc::clone(&self.shared);
        Arc::new(match &p.backend {
            Backend::Craft(module) => Generation::new(owner, manifest, Arc::clone(module), grants, limits, shared),
            #[cfg(feature = "legacy-v1")]
            Backend::Legacy(plugin) => Generation::new_legacy(owner, manifest, Arc::clone(plugin), limits, shared),
        })
    }

    fn next_generation(inner: &mut Inner, id: &str) -> u64 {
        let n = inner.generations.entry(id.to_owned()).or_insert(0);
        *n += 1;
        *n
    }

    fn stored_grants(&self, p: &Prepared, grants: &[Permission]) -> Vec<StoredGrant> {
        let now = self.shared.services.unix_millis();
        grants
            .iter()
            .map(|g| StoredGrant {
                plugin_id: p.manifest.id.clone(),
                publisher: p.manifest.author.clone(),
                permission: g.to_string(),
                granted_at_ms: now,
                decided_by: Decider::User,
            })
            .collect()
    }

    // ---- install and grant ----------------------------------------------------------------

    /// Gate, compile, and either load, or return the prompt the user must answer.
    pub fn install(&self, bytes: &[u8], source: Source) -> Result<InstallOutcome, LifecycleError> {
        let prepared = self.prepare(bytes, source)?;
        let id = prepared.manifest.id.clone();
        let mut inner = self.lock();
        if let Some(existing) =
            inner.plugins.keys().chain(inner.pending.keys()).chain(inner.broken.keys()).find(|k| **k != id && k.eq_ignore_ascii_case(&id))
        {
            return Err(LifecycleError::IdCollision { id, existing: existing.clone() });
        }
        let risk = inner.plugins.get(&id).map_or(UpdateRisk::None, |e| update_risk(e, &prepared));
        // A risky update inherits nothing: the user answers for every required permission again.
        let previous: Option<HashSet<Permission>> =
            inner.plugins.get(&id).map(|e| if risk.is_risky() { HashSet::new() } else { parse_grants(&e.stored).into_iter().collect() });
        let new_required: Vec<Permission> = match &previous {
            Some(granted) => prepared.required.iter().filter(|p| !granted.contains(*p)).cloned().collect(),
            None => prepared.required.clone(),
        };
        let asks = risk.is_risky() || !new_required.is_empty() || (previous.is_none() && !prepared.optional.is_empty());
        if asks {
            let request = GrantRequest {
                plugin_id: id.clone(),
                version: prepared.manifest.version.clone(),
                publisher: prepared.manifest.author.clone(),
                required: prepared.required.clone(),
                new_required,
                optional: prepared.optional.clone(),
                risk,
            };
            inner.pending.insert(id, prepared);
            return Ok(InstallOutcome::NeedsGrant(request));
        }
        // No prompt. An update keeps the grants it still asks for.
        let wanted: HashSet<&Permission> = prepared.required.iter().chain(&prepared.optional).collect();
        let grants: Vec<Permission> = match &previous {
            Some(granted) => granted.iter().filter(|g| wanted.contains(g)).cloned().collect(),
            None => Vec::new(),
        };
        inner.pending.remove(&id);
        self.commit(&mut inner, prepared, grants)
    }

    /// Answer the prompt `install` returned.
    pub fn grant(&self, id: &str, decision: Decision) -> Result<InstallOutcome, LifecycleError> {
        let mut inner = self.lock();
        let prepared = inner.pending.remove(id).ok_or_else(|| LifecycleError::NothingPending(id.to_owned()))?;
        if !decision.accept_required {
            if inner.plugins.contains_key(id) {
                return Ok(InstallOutcome::UpdateDeclined);
            }
            let stored = StoredPlugin {
                id: id.to_owned(),
                version: prepared.manifest.version.clone(),
                bytes: prepared.bytes.clone(),
                enabled: false,
                installed_at_ms: self.shared.services.unix_millis(),
                source: prepared.source,
                grants: Vec::new(),
            };
            self.packages.save(&stored).map_err(LifecycleError::Store)?;
            let entry = Entry::idle(stored, prepared.manifest, PluginState::Disabled, prepared.legacy, Vec::new());
            inner.plugins.insert(id.to_owned(), entry);
            return Ok(InstallOutcome::StoredDisabled);
        }
        for p in &decision.optional {
            if !prepared.optional.contains(p) {
                inner.pending.insert(id.to_owned(), prepared);
                return Err(LifecycleError::NotRequested(p.to_string()));
            }
        }
        let mut grants = prepared.required.clone();
        grants.extend(decision.optional.iter().filter(|p| !grants.contains(p)).cloned().collect::<Vec<_>>());
        self.commit(&mut inner, prepared, grants)
    }

    /// Persist and activate `prepared` with `grants`: reload when a generation runs, load otherwise.
    fn commit(&self, inner: &mut Inner, prepared: Prepared, grants: Vec<Permission>) -> Result<InstallOutcome, LifecycleError> {
        let id = prepared.manifest.id.clone();
        let existing = inner.plugins.get(&id);
        let installed_at_ms = existing.map_or_else(|| self.shared.services.unix_millis(), |e| e.stored.installed_at_ms);
        let was_disabled = existing.is_some_and(|e| e.state == PluginState::Disabled);
        let stored = StoredPlugin {
            id: id.clone(),
            version: prepared.manifest.version.clone(),
            bytes: prepared.bytes.clone(),
            enabled: !was_disabled,
            installed_at_ms,
            source: prepared.source,
            grants: self.stored_grants(&prepared, &grants),
        };
        if was_disabled {
            self.packages.save(&stored).map_err(LifecycleError::Store)?;
            let old = inner.plugins.remove(&id);
            let retired = old.map(|e| e.retired).unwrap_or_default();
            inner.plugins.insert(id, Entry::idle(stored, prepared.manifest, PluginState::Disabled, prepared.legacy, retired));
            return Ok(InstallOutcome::StoredDisabled);
        }
        match self.router.current(&id) {
            Some(old) => self.reload(inner, &old, prepared, stored, grants),
            None => self.load_fresh(inner, prepared, stored, grants),
        }
    }

    fn ui_allowed(grants: &[Permission]) -> bool {
        grants.iter().any(|g| matches!(g, Permission::UiContribute))
    }

    fn check_required(p: &Prepared, grants: &[Permission]) -> Result<(), LifecycleError> {
        match p.required.iter().find(|r| !grants.contains(r)) {
            Some(missing) => Err(LifecycleError::MissingGrant(missing.to_string())),
            None => Ok(()),
        }
    }

    /// Load generation 1 of a plugin that has none running. A failure leaves the plugin installed and `Faulted`.
    fn load_fresh(
        &self,
        inner: &mut Inner,
        prepared: Prepared,
        stored: StoredPlugin,
        grants: Vec<Permission>,
    ) -> Result<InstallOutcome, LifecycleError> {
        let id = stored.id.clone();
        self.packages.save(&stored).map_err(LifecycleError::Store)?;
        let retired = inner.plugins.remove(&id).map(|e| e.retired).unwrap_or_default();
        let manifest = Arc::clone(&prepared.manifest);
        let legacy = prepared.legacy.clone();
        let fault = |inner: &mut Inner, stored: StoredPlugin, retired: Vec<Weak<Generation>>, e: LifecycleError| {
            let reason = e.to_string();
            inner.plugins.insert(id.clone(), Entry::idle(stored, Arc::clone(&manifest), PluginState::Faulted { reason }, legacy.clone(), retired));
            e
        };
        if let Err(e) = Self::check_required(&prepared, &grants) {
            return Err(fault(inner, stored, retired, e));
        }
        let number = Self::next_generation(inner, &id);
        let generation = self.generation_for(&prepared, grants.clone(), number);
        let commands = match register_all::<S, E>(
            &self.registry,
            &generation.owner,
            &prepared.manifest,
            &self.router,
            &self.shared.services,
            Self::ui_allowed(&grants),
        ) {
            Ok(c) => c,
            Err(f) => return Err(fault(inner, stored, retired, LifecycleError::Register(f))),
        };
        self.shared.faults.reset_plugin(&id);
        let hooks = self.register_hooks(&generation);
        let surfaces = self.register_surfaces(&generation);
        self.router.route(generation);
        inner.plugins.insert(
            id,
            Entry {
                commands,
                hooks,
                surfaces,
                ..Entry::idle(stored, prepared.manifest, PluginState::Active { generation: number }, prepared.legacy, retired)
            },
        );
        Ok(InstallOutcome::Loaded { generation: number, replaced: None })
    }

    /// The reload of the module comment. Every `?` before step 5 leaves N untouched.
    fn reload(
        &self,
        inner: &mut Inner,
        old: &Arc<Generation>,
        prepared: Prepared,
        stored: StoredPlugin,
        grants: Vec<Permission>,
    ) -> Result<InstallOutcome, LifecycleError> {
        let id = stored.id.clone();
        Self::check_required(&prepared, &grants)?;
        let number = Self::next_generation(inner, &id);
        let generation = self.generation_for(&prepared, grants.clone(), number);
        // 2 and 3: carry session state across.
        let old_version = old.manifest.version.clone();
        if let Some(data) = old.save_state().map_err(LifecycleError::StateSave)? {
            generation.load_state(&data, &old_version).map_err(LifecycleError::StateLoad)?;
        }
        // 4: persist, keeping the old record to restore.
        let old_stored = inner.plugins.get(&id).map(|e| e.stored.clone());
        self.packages.save(&stored).map_err(LifecycleError::Store)?;
        // 5: swap the registry. N goes first because both use the same ids.
        self.registry.unregister_owner(&old.owner);
        let ui = Self::ui_allowed(&grants);
        let commands = match register_all::<S, E>(&self.registry, &generation.owner, &prepared.manifest, &self.router, &self.shared.services, ui) {
            Ok(c) => c,
            Err(cause) => {
                if let Some(o) = &old_stored
                    && let Err(e) = self.packages.save(o)
                {
                    return Err(LifecycleError::Store(e));
                }
                let old_ui = Self::ui_allowed(old.grants());
                return match register_all::<S, E>(&self.registry, &old.owner, &old.manifest, &self.router, &self.shared.services, old_ui) {
                    Ok(_) => Err(LifecycleError::Register(cause)),
                    Err(restore) => {
                        self.unregister_hooks(&old.owner);
                        self.unregister_surfaces(&old.owner);
                        self.router.unroute(&id);
                        if let Some(e) = inner.plugins.get_mut(&id) {
                            e.state = PluginState::Faulted { reason: "a reload failed and the old version could not be restored".into() };
                            e.commands.clear();
                        }
                        Err(LifecycleError::RollbackFailed { cause, restore })
                    }
                };
            }
        };
        // 6: route. N finishes its running calls and drops when the last `Arc` goes.
        self.shared.faults.reset_plugin(&id);
        // Hooks cannot fail to register, so they move over after the one fallible step.
        self.unregister_hooks(&old.owner);
        self.unregister_surfaces(&old.owner);
        let hooks = self.register_hooks(&generation);
        let surfaces = self.register_surfaces(&generation);
        self.router.route(generation);
        let mut retired = inner.plugins.remove(&id).map(|e| e.retired).unwrap_or_default();
        retired.retain(|w| w.strong_count() > 0);
        retired.push(Arc::downgrade(old));
        inner.plugins.insert(
            id,
            Entry {
                commands,
                hooks,
                surfaces,
                ..Entry::idle(stored, prepared.manifest, PluginState::Active { generation: number }, prepared.legacy, retired)
            },
        );
        Ok(InstallOutcome::Loaded { generation: number, replaced: Some(old.owner.generation) })
    }

    // ---- disable, enable, revoke, remove --------------------------------------------------

    fn unload(&self, inner: &mut Inner, id: &str) {
        if let Some(old) = self.router.unroute(id) {
            self.registry.unregister_owner(&old.owner);
            self.unregister_hooks(&old.owner);
            self.unregister_surfaces(&old.owner);
            if let Some(e) = inner.plugins.get_mut(id) {
                e.retired.retain(|w| w.strong_count() > 0);
                e.retired.push(Arc::downgrade(&old));
                e.commands.clear();
                e.hooks = HookRegistration::default();
                e.surfaces = SurfaceRegistration::default();
            }
        }
    }

    /// Switch a plugin off. Its commands leave the registry. Running calls finish.
    pub fn disable(&self, id: &str) -> Result<(), LifecycleError> {
        let mut inner = self.lock();
        let entry = inner.plugins.get(id).ok_or_else(|| LifecycleError::NoSuchPlugin(id.to_owned()))?;
        let mut stored = entry.stored.clone();
        stored.enabled = false;
        self.packages.save(&stored).map_err(LifecycleError::Store)?;
        self.unload(&mut inner, id);
        if let Some(e) = inner.plugins.get_mut(id) {
            e.stored = stored;
            e.state = PluginState::Disabled;
        }
        Ok(())
    }

    /// Switch a disabled or faulted plugin on. The package goes through the gate again.
    pub fn enable(&self, id: &str) -> Result<InstallOutcome, LifecycleError> {
        let mut inner = self.lock();
        let entry = inner.plugins.get(id).ok_or_else(|| LifecycleError::NoSuchPlugin(id.to_owned()))?;
        if matches!(entry.state, PluginState::Active { .. }) {
            return Ok(InstallOutcome::Loaded { generation: entry.state_generation(), replaced: None });
        }
        let stored = StoredPlugin { enabled: true, ..entry.stored.clone() };
        let grants = parse_grants(&stored);
        let prepared = match self.prepare(&stored.bytes, stored.source) {
            Ok(p) => p,
            Err(e) => {
                if let Some(en) = inner.plugins.get_mut(id) {
                    en.state = PluginState::Faulted { reason: e.to_string() };
                }
                return Err(e);
            }
        };
        self.load_fresh(&mut inner, prepared, stored, grants)
    }

    /// Withdraw one granted permission. The plugin reloads without it, so the change takes effect
    /// now. Revoking a required permission disables the plugin.
    pub fn revoke(&self, id: &str, permission: &Permission) -> Result<(), LifecycleError> {
        let required = {
            let inner = self.lock();
            let entry = inner.plugins.get(id).ok_or_else(|| LifecycleError::NoSuchPlugin(id.to_owned()))?;
            entry.manifest.parsed_permissions().map_err(|e| LifecycleError::Manifest(e.to_string()))?.0.contains(permission)
        };
        if required {
            // Drop the grant, then switch off. Enable will ask again through a new install.
            let mut inner = self.lock();
            if let Some(e) = inner.plugins.get_mut(id) {
                e.stored.grants.retain(|g| g.permission != permission.to_string());
            }
            drop(inner);
            return self.disable(id);
        }
        let mut inner = self.lock();
        let entry = inner.plugins.get(id).ok_or_else(|| LifecycleError::NoSuchPlugin(id.to_owned()))?;
        let mut stored = entry.stored.clone();
        stored.grants.retain(|g| g.permission != permission.to_string());
        let grants = parse_grants(&stored);
        let prepared = self.prepare(&stored.bytes, stored.source)?;
        match self.router.current(id) {
            Some(old) => self.reload(&mut inner, &old, prepared, stored, grants).map(|_| ()),
            None => {
                self.packages.save(&stored).map_err(LifecycleError::Store)?;
                if let Some(e) = inner.plugins.get_mut(id) {
                    e.stored = stored;
                }
                Ok(())
            }
        }
    }

    /// Delete the plugin, its grants, and its `state.*` data. Documents keep their
    /// [`crate::missing::MissingPluginRef`] data and show "missing plugin".
    pub fn remove(&self, id: &str) -> Result<(), LifecycleError> {
        let mut inner = self.lock();
        let known = inner.plugins.contains_key(id) || inner.pending.contains_key(id) || inner.broken.contains_key(id);
        if !known {
            return Err(LifecycleError::NoSuchPlugin(id.to_owned()));
        }
        self.packages.remove(id).map_err(LifecycleError::Store)?;
        self.shared.state.clear(id).map_err(LifecycleError::Store)?;
        self.unload(&mut inner, id);
        inner.plugins.remove(id);
        inner.pending.remove(id);
        inner.broken.remove(id);
        self.shared.faults.reset_plugin(id);
        Ok(())
    }

    /// Load every stored plugin at app start. A plugin that fails stays installed and `Faulted`.
    /// Only a store that cannot list its entries at all returns `Err`.
    pub fn restore(&self) -> Result<Vec<(String, PluginState)>, LifecycleError> {
        let loaded = self.packages.load_all_lenient().map_err(LifecycleError::Store)?;
        let mut inner = self.lock();
        let mut out = Vec::new();
        // One bad entry never stops the rest. It is listed as `Faulted` under its
        // directory name, and `remove` can delete it.
        for skipped in loaded.skipped {
            inner.broken.insert(skipped.name.clone(), skipped.reason.clone());
            out.push((skipped.name, PluginState::Faulted { reason: skipped.reason }));
        }
        let mut seen: HashMap<String, String> = HashMap::new();
        for s in loaded.plugins {
            let id = s.id.clone();
            if let Some(first) = seen.insert(id.to_ascii_lowercase(), id.clone()) {
                let reason = format!("plugin id differs from {first:?} only by letter case; not loaded");
                inner.broken.insert(id.clone(), reason.clone());
                out.push((id, PluginState::Faulted { reason }));
                continue;
            }
            let grants = parse_grants(&s);
            if !s.enabled {
                let (manifest, legacy) = match self.prepare(&s.bytes, s.source) {
                    Ok(p) => (p.manifest, p.legacy),
                    Err(e) => {
                        // The package no longer gates, so it has no manifest to list. `state` reports
                        // it as faulted and `remove` can delete it.
                        let reason = e.to_string();
                        inner.broken.insert(id.clone(), reason.clone());
                        out.push((id, PluginState::Faulted { reason }));
                        continue;
                    }
                };
                inner.plugins.insert(id.clone(), Entry::idle(s, manifest, PluginState::Disabled, legacy, Vec::new()));
                out.push((id, PluginState::Disabled));
                continue;
            }
            let prepared = match self.prepare(&s.bytes, s.source) {
                Ok(p) => p,
                Err(e) => {
                    inner.broken.insert(id.clone(), e.to_string());
                    out.push((id, PluginState::Faulted { reason: e.to_string() }));
                    continue;
                }
            };
            // A failure here leaves a `Faulted` entry, which the returned state reports.
            let _ = self.load_fresh(&mut inner, prepared, s, grants);
            let state = inner.plugins.get(&id).map_or(PluginState::Disabled, |e| e.state.clone());
            out.push((id, state));
        }
        Ok(out)
    }
}

/// Judge a package that has the id of an installed plugin. See [`UpdateRisk`].
fn update_risk(installed: &Entry, new: &Prepared) -> UpdateRisk {
    if installed.stored.bytes == new.bytes {
        return UpdateRisk::None;
    }
    if installed.manifest.author != new.manifest.author {
        return UpdateRisk::Publisher { installed: installed.manifest.author.clone() };
    }
    if !crate::version::is_newer(&installed.manifest.version, &new.manifest.version) {
        return UpdateRisk::NotNewer { installed: installed.manifest.version.clone() };
    }
    UpdateRisk::None
}

impl Entry {
    fn state_generation(&self) -> u64 {
        match self.state {
            PluginState::Active { generation } => generation,
            _ => 0,
        }
    }
}
