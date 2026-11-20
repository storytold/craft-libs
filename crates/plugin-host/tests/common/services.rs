//! A recording `Services` implementation and a harness that wires a `PluginManager` to it.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use craft_command::{Owner, Registry};
use craft_plugin_abi::HostPolicy;
use craft_plugin_abi::permissions::DocScope;
use craft_plugin_host::budget::BudgetConfig;
use craft_plugin_host::dispatch::DispatchConfig;
use craft_plugin_host::engine::Trust;
use craft_plugin_host::engine::wasmi::WasmiEngine;
use craft_plugin_host::invoke::CallError;
use craft_plugin_host::lifecycle::{ManagerConfig, PluginManager};
use craft_plugin_host::services::*;
use craft_plugin_host::store::{MemoryStore, PackageStore};
use serde_json::{Value, json};

use super::StepClock;

#[derive(Default)]
pub struct Recorded {
    pub logs: Vec<(String, LogLevel, String)>,
    pub progress: Vec<(f64, Option<String>)>,
    pub executed: Vec<(String, u64, String, Value)>,
    pub begun: Vec<(CallScope, String)>,
    pub ended: Vec<(CallScope, CallOutcome)>,
    pub notices: Vec<UserNotice>,
    pub doc_queries: Vec<(DocScope, Value)>,
    pub link_calls: Vec<(String, String, String, Value)>,
}

/// A log call blocks until the test releases it, so a test can hold a guest call open.
pub struct LogGate {
    pub entered: Sender<()>,
    pub release: Receiver<()>,
}

pub struct TestServices {
    pub rec: Mutex<Recorded>,
    pub commands: Mutex<Vec<CommandInfo>>,
    pub exec_result: Mutex<Result<Value, CommandFailure>>,
    pub doc_result: Mutex<Result<Value, QueryFailure>>,
    pub log_gate: Mutex<Option<LogGate>>,
    pub link_apps_result: Mutex<Result<Vec<LinkApp>, LinkFailure>>,
    /// When set, the link methods go to this client instead of the scripted results.
    #[cfg(feature = "link")]
    pub link_client: Mutex<Option<craft_plugin_host::link::LinkClient>>,
    pub link_result: Mutex<Result<Value, LinkFailure>>,
    next_scope: AtomicU64,
    pub now_ms: AtomicU64,
    /// When set, `execute_command` and `doc_query` fail with the busy error unless the calling
    /// thread is inside [`Services::lend_session`], as in an app whose session must be lent.
    pub require_lend: AtomicBool,
    /// How many times the plugin host lent the session.
    pub lends: AtomicU64,
    /// When set, `fill_random` reports that the system has no entropy.
    pub entropy_fails: AtomicBool,
}

thread_local! {
    static LENT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Marks this thread as inside `lend_session` until it drops, also on unwind.
struct LentGuard;

impl LentGuard {
    fn enter() -> LentGuard {
        LENT.with(|c| c.set(c.get() + 1));
        LentGuard
    }
}

impl Drop for LentGuard {
    fn drop(&mut self) {
        LENT.with(|c| c.set(c.get() - 1));
    }
}

fn lent_here() -> bool {
    LENT.with(|c| c.get() > 0)
}

impl TestServices {
    pub fn new() -> Arc<TestServices> {
        Arc::new(TestServices {
            rec: Mutex::new(Recorded::default()),
            commands: Mutex::new(vec![
                CommandInfo { id: "app.set".into(), label: "Set".into() },
                CommandInfo { id: "app.delete".into(), label: "Delete".into() },
                CommandInfo { id: "other.thing".into(), label: "Thing".into() },
            ]),
            exec_result: Mutex::new(Ok(json!({ "done": true }))),
            doc_result: Mutex::new(Ok(json!({ "layers": 3 }))),
            log_gate: Mutex::new(None),
            link_apps_result: Mutex::new(Ok(Vec::new())),
            #[cfg(feature = "link")]
            link_client: Mutex::new(None),
            link_result: Mutex::new(Ok(json!({ "linked": true }))),
            next_scope: AtomicU64::new(1),
            now_ms: AtomicU64::new(1_700_000_000_000),
            require_lend: AtomicBool::new(false),
            lends: AtomicU64::new(0),
            entropy_fails: AtomicBool::new(false),
        })
    }

    pub fn recorded<R>(&self, f: impl FnOnce(&Recorded) -> R) -> R {
        f(&self.rec.lock().unwrap())
    }
}

impl Services for TestServices {
    fn host_info(&self) -> HostInfo {
        HostInfo { app: "testapp".into(), version: "9.9".into() }
    }

    fn log(&self, owner: &Owner, level: LogLevel, message: &str) {
        self.rec.lock().unwrap().logs.push((owner.plugin_id.to_string(), level, message.to_owned()));
        let gate = self.log_gate.lock().unwrap().take();
        if let Some(g) = gate {
            g.entered.send(()).unwrap();
            g.release.recv().unwrap();
        }
    }

    fn progress(&self, _owner: &Owner, fraction: f64, message: Option<&str>) {
        self.rec.lock().unwrap().progress.push((fraction, message.map(str::to_owned)));
    }

    fn list_commands(&self) -> Vec<CommandInfo> {
        self.commands.lock().unwrap().clone()
    }

    fn execute_command(&self, owner: &Owner, id: &str, params: &Value) -> Result<Value, CommandFailure> {
        if self.require_lend.load(Ordering::Relaxed) && !lent_here() {
            return Err(CommandFailure::Failed("the document is busy".into()));
        }
        self.rec.lock().unwrap().executed.push((owner.plugin_id.to_string(), owner.generation, id.to_owned(), params.clone()));
        self.exec_result.lock().unwrap().clone()
    }

    fn doc_query(&self, _owner: &Owner, scope: DocScope, query: &Value) -> Result<Value, QueryFailure> {
        if self.require_lend.load(Ordering::Relaxed) && !lent_here() {
            return Err(QueryFailure::Unsupported);
        }
        self.rec.lock().unwrap().doc_queries.push((scope, query.clone()));
        self.doc_result.lock().unwrap().clone()
    }

    fn unix_millis(&self) -> u64 {
        self.now_ms.load(Ordering::Relaxed)
    }

    fn fill_random(&self, buf: &mut [u8]) -> Result<(), EntropyUnavailable> {
        if self.entropy_fails.load(Ordering::Relaxed) {
            return Err(EntropyUnavailable("the test has no entropy".into()));
        }
        buf.fill(0xAB);
        Ok(())
    }

    fn notify(&self, notice: UserNotice) {
        self.rec.lock().unwrap().notices.push(notice);
    }

    fn link_apps(&self, _owner: &Owner) -> Result<Vec<LinkApp>, LinkFailure> {
        #[cfg(feature = "link")]
        if let Some(client) = self.link_client.lock().unwrap().clone() {
            return client.apps();
        }
        self.link_apps_result.lock().unwrap().clone()
    }

    fn link_call(&self, owner: &Owner, app: &str, method: &str, params: &Value, cancel: &AtomicBool) -> Result<Value, LinkFailure> {
        self.rec.lock().unwrap().link_calls.push((owner.plugin_id.to_string(), app.to_owned(), method.to_owned(), params.clone()));
        #[cfg(feature = "link")]
        if let Some(client) = self.link_client.lock().unwrap().clone() {
            return client.call(app, method, params, cancel);
        }
        #[cfg(not(feature = "link"))]
        let _ = cancel;
        self.link_result.lock().unwrap().clone()
    }

    fn lend_session(&self, session: &mut dyn std::any::Any, run: &mut dyn FnMut()) {
        assert!(session.is::<Sess>(), "the plugin host lends the app's own session type");
        self.lends.fetch_add(1, Ordering::Relaxed);
        let _lent = LentGuard::enter();
        run();
    }

    fn begin_call(&self, _owner: &Owner, contribution: &str) -> CallScope {
        let scope = CallScope(self.next_scope.fetch_add(1, Ordering::Relaxed));
        self.rec.lock().unwrap().begun.push((scope, contribution.to_owned()));
        scope
    }

    fn end_call(&self, scope: CallScope, outcome: &CallOutcome) {
        self.rec.lock().unwrap().ended.push((scope, outcome.clone()));
    }
}

/// The app session type and error type of the harness.
pub struct Sess;

#[derive(Debug)]
pub struct AppError(pub CallError);

impl From<CallError> for AppError {
    fn from(e: CallError) -> Self {
        AppError(e)
    }
}

pub static NO_BUILTINS: [craft_command::CommandSpec<Sess, AppError>; 0] = [];

pub struct Harness {
    pub manager: Arc<PluginManager<Sess, AppError>>,
    pub registry: Arc<Registry<Sess, AppError>>,
    pub services: Arc<TestServices>,
    pub store: Arc<MemoryStore>,
    pub clock: Arc<StepClock>,
}

impl Harness {
    /// A manager on wasmi with a clock that does not move.
    pub fn new() -> Harness {
        Harness::with_clock(Duration::ZERO, BudgetConfig::default())
    }

    pub fn with_clock(step: Duration, budgets: BudgetConfig) -> Harness {
        let clock = StepClock::new(step);
        let engine = Arc::new(WasmiEngine::new(clock.clone()).with_slice_fuel(20_000));
        let store = Arc::new(MemoryStore::new());
        Harness::build(engine, clock, budgets, store.clone(), store)
    }

    pub fn build(
        engine: Arc<WasmiEngine>,
        clock: Arc<StepClock>,
        budgets: BudgetConfig,
        packages: Arc<dyn PackageStore>,
        store: Arc<MemoryStore>,
    ) -> Harness {
        Harness::build_with(engine, clock, budgets, packages, store, |s| s)
    }

    /// Like `build`, with `wrap` between the manager and the recording services. A test wraps them
    /// to change what a few methods do.
    pub fn build_with(
        engine: Arc<WasmiEngine>,
        clock: Arc<StepClock>,
        budgets: BudgetConfig,
        packages: Arc<dyn PackageStore>,
        store: Arc<MemoryStore>,
        wrap: impl FnOnce(Arc<TestServices>) -> Arc<dyn Services>,
    ) -> Harness {
        let services = TestServices::new();
        let registry = Arc::new(Registry::new(&NO_BUILTINS));
        let cfg = ManagerConfig { policy: HostPolicy::native(), trust: Trust::Untrusted, budgets, dispatch: DispatchConfig::default() };
        let manager = PluginManager::new(engine, cfg, registry.clone(), packages, store.clone(), wrap(services.clone()), clock.clone());
        Harness { manager, registry, services, store, clock }
    }

    /// Run a registered command as the app would.
    pub fn run(&self, id: &str, params: &Value) -> Result<Value, AppError> {
        match self.registry.resolve(id) {
            Some(craft_command::CommandEntry::Dynamic(cmd)) => (cmd.run)(&mut Sess, params),
            other => panic!("{id} did not resolve to a dynamic command: {other:?}"),
        }
    }
}
