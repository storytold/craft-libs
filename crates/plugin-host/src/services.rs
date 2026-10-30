//! What the host app gives a plugin host. The plugin host owns the rules (permissions, quotas,
//! budgets). The app owns the document, the command layer, the log, and the user interface.
//!
//! Every method is a real seam. There are no default bodies, so an app cannot ship an
//! implementation that quietly ignores a call.

use std::any::Any;
use std::sync::atomic::AtomicBool;

use craft_command::Owner;
use craft_plugin_abi::permissions::DocScope;
use serde_json::Value;

use crate::engine::MaybeSendSync;

/// App name and version for `host.info`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostInfo {
    pub app: String,
    pub version: String,
}

/// `host.log` severity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn parse(name: &str) -> Option<LogLevel> {
        match name {
            "debug" => Some(LogLevel::Debug),
            "info" => Some(LogLevel::Info),
            "warn" => Some(LogLevel::Warn),
            "error" => Some(LogLevel::Error),
            _ => None,
        }
    }
}

/// One row for `cmd.list`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandInfo {
    pub id: String,
    pub label: String,
}

/// Why `cmd.execute` did not produce a result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandFailure {
    /// The id is not in the registry. The guest sees `E_BAD_ARG`.
    Unknown,
    /// The command ran and failed, or a gate vetoed it. The guest sees `{"error": message}`.
    Failed(String),
}

/// Why `doc.query` did not produce a result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueryFailure {
    /// The query is malformed. The guest sees `E_BAD_ARG`.
    BadQuery(String),
    /// The app has no document or does not serve this scope. The guest sees `E_UNSUPPORTED`.
    Unsupported,
}

/// One running app a plugin may reach through Craft Link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkApp {
    /// App name, as in the instance record (`filmcraft`, `effectcraft`).
    pub app: String,
    pub app_version: String,
}

/// Why `link.call` or `link.apps` did not produce a result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkFailure {
    /// No instance of the app is running. The guest sees `{"error": message}`.
    NotRunning(String),
    /// Discovery, the socket, the handshake, or the peer failed. The guest sees `{"error": message}`.
    Failed(String),
    /// The user cancelled while the call was in flight. The guest sees `E_CANCELLED`.
    Cancelled,
}

/// A message the user must see, such as a plugin fault or a tripped circuit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserNotice {
    pub plugin_id: String,
    pub contribution: Option<String>,
    pub message: String,
}

impl UserNotice {
    /// A notice whose `message` is plugin influenced text. The message becomes one safe line of at
    /// most [`craft_plugin_abi::text::MAX_NOTICE_CHARS`] characters, so a plugin cannot
    /// queue a huge toast, forge a second line, or hide text with bidi controls. Build every notice
    /// with this, never with a struct literal.
    pub fn new(plugin_id: impl Into<String>, contribution: Option<String>, message: &str) -> UserNotice {
        UserNotice { plugin_id: plugin_id.into(), contribution, message: craft_plugin_abi::text::clean_notice(message) }
    }
}

/// Opaque id the app returns from `begin_call` and gets back in `end_call`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CallScope(pub u64);

/// How a guest call ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallOutcome {
    /// The guest returned a response.
    Completed,
    /// The guest trapped, broke a budget, or returned a malformed response. The app must restore
    /// the document to the state `begin_call` saw.
    Faulted(String),
    /// The user cancelled. The app decides whether to keep partial work.
    Cancelled,
}

/// The app side of every host call and of the call lifecycle.
pub trait Services: MaybeSendSync {
    fn host_info(&self) -> HostInfo;
    /// Receive a `host.log` line. The plugin host already rate limited it.
    fn log(&self, owner: &Owner, level: LogLevel, message: &str);
    /// Receive `host.progress`. `fraction` is in `0.0..=1.0`.
    fn progress(&self, owner: &Owner, fraction: f64, message: Option<&str>);
    /// Every command the app has. The plugin host filters the list by the plugin's grants.
    fn list_commands(&self) -> Vec<CommandInfo>;
    /// Run one command as one undo step with `Origin::Plugin`. The plugin host has already checked
    /// `commands.run`. The call arrives inside [`Services::lend_session`], so the app can reach its
    /// session. That holds in a command body and in a `before` gate.
    fn execute_command(&self, owner: &Owner, id: &str, params: &Value) -> Result<Value, CommandFailure>;
    /// Answer a snapshot query. The plugin host has already checked `doc.read` for `scope`.
    fn doc_query(&self, owner: &Owner, scope: DocScope, query: &Value) -> Result<Value, QueryFailure>;
    /// Wall clock, milliseconds since the Unix epoch. For `time.now` with the `clock` permission
    /// and for grant time stamps.
    fn unix_millis(&self) -> u64;
    /// Fill `buf` with entropy. For `random.bytes` with the `random` permission.
    fn fill_random(&self, buf: &mut [u8]);
    /// Tell the user something.
    fn notify(&self, notice: UserNotice);
    /// Every app with a live Craft Link instance. The plugin host filters the list by the plugin's
    /// `link.call:<app>` grants. `crate::link::LinkClient` (feature `link`) is a ready
    /// implementation.
    ///
    /// An app with no Craft Link returns `Ok(Vec::new())`: no app is running, so a plugin sees an
    /// empty list.
    fn link_apps(&self, owner: &Owner) -> Result<Vec<LinkApp>, LinkFailure>;
    /// Call `method` on the running `app` and wait for the reply. The plugin host has already
    /// checked `link.call:<app>`. The call blocks the calling thread, so run link calling
    /// contributions with `budget: batch` off the UI thread. Poll `cancel` and return
    /// [`LinkFailure::Cancelled`] when it is set.
    ///
    /// An app with no Craft Link returns [`LinkFailure::NotRunning`] with a message. The guest
    /// sees `{"error": message}`.
    fn link_call(&self, owner: &Owner, app: &str, method: &str, params: &Value, cancel: &AtomicBool) -> Result<Value, LinkFailure>;
    /// Make `session` reachable for host calls while `run` executes.
    ///
    /// The plugin host calls this around every guest call that runs inside a command body or a
    /// `before` gate. Both hold the app session as `&mut`, and the guest's host calls come back
    /// through `&self` methods of this trait. An app whose host calls need the session moves it
    /// somewhere they can reach, runs `run`, and puts it back, also when `run` unwinds.
    ///
    /// `session` is the app's own session type. An app that cannot use the value (a type it does
    /// not know) still runs `run`; its host calls then fail with a busy error.
    fn lend_session(&self, session: &mut dyn Any, run: &mut dyn FnMut());
    /// A guest call starts. The app opens an undo checkpoint.
    fn begin_call(&self, owner: &Owner, contribution: &str) -> CallScope;
    /// The guest call ends. On `Faulted` the app restores the checkpoint.
    fn end_call(&self, scope: CallScope, outcome: &CallOutcome);
}

/// Run `body` inside [`Services::lend_session`] and return what it returns. `None` means the app
/// did not run the body, which breaks the trait contract. The guest did not run then.
pub fn lend<R>(services: &dyn Services, session: &mut dyn Any, body: impl FnOnce() -> R) -> Option<R> {
    let mut body = Some(body);
    let mut out = None;
    services.lend_session(session, &mut || {
        if let Some(body) = body.take() {
            out = Some(body());
        }
    });
    out
}
