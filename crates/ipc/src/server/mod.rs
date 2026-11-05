//! Craft Link server.
//!
//! One accept thread, then a read thread and a write thread per connection.
//! There is no tokio runtime here.

mod dispatch;
mod events;
mod listener;
mod tcp;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::Value;

use crate::asset::{Described, Refreshed, Resolved};
use crate::discovery::{Discovery, InstanceRecord, RuntimeDir};
use crate::error::LinkError;
use crate::moniker::Moniker;

/// App request channel.
///
/// `submit` returns `None` when the app is gone. The connection then closes.
/// The `Receiver` yields either a result payload or a finished reply object.
/// A payload is wrapped with [`crate::wire::reply_ok`]. An object that already
/// contains `ok` is passed through, and `id` is inserted when it is missing.
/// This signature cannot carry a [`LinkError`].
/// A sink that never replies hits [`crate::wire::REPLY_DEADLINE_SECS`].
pub trait ControlSink: Send + Sync {
    /// None means the app is gone. The connection must close.
    fn submit(&self, method: String, params: Value) -> Option<std::sync::mpsc::Receiver<Value>>;
}

/// What [`Activate::activate`] decided. The server serializes this as JSON
/// `true`, `false`, or the string `"requested"`. It does not rewrite the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activation {
    /// The window has focus. JSON `true`.
    Yes,
    /// Focus was not taken. The app asked for user attention instead. JSON `false`.
    No,
    /// The app asked the OS to focus the window. The OS decides later and
    /// nobody confirmed it. JSON `"requested"`.
    Requested,
}

/// Raises or focuses the app. There is no default success.
pub trait Activate: Send + Sync {
    fn activate(&self, token: Option<String>, from_pid: Option<u32>, open: Option<Value>) -> Activation;
}

/// One bulk ticket from [`Frames::open`]. `ticket` is 32 lowercase hex characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameOpen {
    pub ticket: String,
    pub w: u32,
    pub h: u32,
    pub stride: u32,
    pub len: u64,
}

/// Frame tickets and rendered payloads. The server registers the ticket on the
/// control connection that called `frames.open`.
pub trait Frames: Send + Sync {
    fn open(&self, params: &serde_json::Map<String, Value>) -> Result<FrameOpen, LinkError>;
    fn render(&self, ticket: &str, header: &Value) -> Result<(Value, Vec<u8>), LinkError>;
    fn close_ticket(&self, ticket: &str);
}

/// Answers asset questions on the connection thread, with no UI frame
/// (`link.resolve`, `asset.describe`, `link.refresh`). Pass one in
/// [`Extensions::assets`]. Without it these three methods go to the
/// [`ControlSink`] like any other link method.
///
/// Every method runs on a connection thread, so an implementation must be
/// thread safe and must not wait on the UI thread.
pub trait Assets: Send + Sync {
    /// Where the last-known result of `asset` is. `Err(LinkError::Unknown)` when
    /// the asset or its result file does not exist.
    fn resolve(&self, asset: &Moniker) -> Result<Resolved, LinkError>;
    /// Name, kind and thumbnail of `asset`.
    fn describe(&self, asset: &Moniker) -> Result<Described, LinkError>;
    /// Re-check the file behind `link_id` and report its state.
    fn refresh(&self, link_id: &str) -> Result<Refreshed, LinkError>;
}

/// Optional parts for [`Server::start_with`]. The default adds nothing.
#[derive(Default)]
pub struct Extensions {
    /// Answers asset methods on the connection thread. Adds the `assets` cap.
    pub assets: Option<Arc<dyn Assets>>,
    /// Extra caps for the instance record and `link.hello`, such as `hub`.
    pub caps: Vec<String>,
}

/// What [`Server::start`] needs. `tcp_port: None` does not bind a TCP socket.
pub struct ServerConfig {
    pub app: String,
    pub app_version: String,
    pub instance: Option<String>,
    pub runtime: Option<RuntimeDir>,
    pub tcp_port: Option<u16>,
    pub sink: Arc<dyn ControlSink>,
    pub activate: Arc<dyn Activate>,
    pub frames: Arc<dyn Frames>,
}

/// Time and count limits on connections. `Limits::default()` is what the apps run.
/// Tests shorten the times through [`Server::start_with_limits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// A new connection must send a valid `auth` line within this time, or it is closed.
    pub auth_deadline: Duration,
    /// A request line, once started, must end within this time, or the connection is closed.
    pub partial_line: Duration,
    /// An authenticated control connection that sends nothing for this long is closed. A
    /// connection with an event subscription and a bulk connection are exempt.
    pub idle: Duration,
    /// Connections that have not authenticated yet. At most `MAX_CONNECTIONS` minus this many
    /// slots can be held by them.
    pub max_unauthenticated: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            auth_deadline: Duration::from_secs(2),
            partial_line: Duration::from_secs(10),
            idle: Duration::from_secs(60 * 60),
            max_unauthenticated: crate::wire::MAX_UNAUTHENTICATED,
        }
    }
}

pub(super) struct Hub {
    pub(super) limits: Limits,
    pub(super) pending: AtomicUsize,
    pub(super) record: InstanceRecord,
    pub(super) sink: Arc<dyn ControlSink>,
    pub(super) activate: Arc<dyn Activate>,
    pub(super) frames: Arc<dyn Frames>,
    pub(super) assets: Option<Arc<dyn Assets>>,
    pub(super) stop: AtomicBool,
    pub(super) open: AtomicUsize,
    pub(super) next_id: AtomicU64,
    pub(super) tickets: Mutex<HashMap<String, u64>>,
    pub(in crate::server) subs: Mutex<Vec<Arc<events::Subscription>>>,
    pub(super) threads: Mutex<Vec<JoinHandle<()>>>,
}

/// A running link endpoint. [`Server::shutdown`] removes the instance record.
/// Drop only joins the accept thread and the connection threads.
pub struct Server {
    discovery: Option<Discovery>,
    hub: Arc<Hub>,
    accept: Option<JoinHandle<()>>,
    tcp_port: Option<u16>,
    record: InstanceRecord,
}

impl Server {
    /// Bind, publish, then accept. Scan runs before the bind so a dead socket
    /// from an earlier process is unlinked. A lock that is still held is left
    /// alone. The instance record keeps the local socket as its endpoint. When `tcp_port` is set the
    /// record also carries the bound port (`record.tcp_port()`), and the TCP listener requires the
    /// same `auth` token line as the socket.
    pub fn start(config: ServerConfig) -> Result<Self, LinkError> {
        Self::start_inner(config, Extensions::default(), Limits::default())
    }

    /// Like [`Server::start`], plus the optional parts in [`Extensions`].
    pub fn start_with(config: ServerConfig, extensions: Extensions) -> Result<Self, LinkError> {
        Self::start_inner(config, extensions, Limits::default())
    }

    /// Like [`Server::start_with`], with connection limits other than the defaults.
    pub fn start_with_limits(config: ServerConfig, extensions: Extensions, limits: Limits) -> Result<Self, LinkError> {
        Self::start_inner(config, extensions, limits)
    }

    fn start_inner(config: ServerConfig, extensions: Extensions, limits: Limits) -> Result<Self, LinkError> {
        let runtime = match config.runtime {
            Some(dir) => dir,
            None => RuntimeDir::resolve()?,
        };
        let _existing = Discovery::scan(&runtime)?;
        let pid = std::process::id();
        #[cfg(unix)]
        let socket_path = runtime.socket_file(&config.app, pid);
        let listener = listener::bind(&runtime, &config.app, pid)?;
        #[cfg(unix)]
        if let Err(err) = crate::discovery::enforce_mode(&socket_path, 0o600) {
            drop(listener);
            let _ = std::fs::remove_file(&socket_path);
            return Err(err);
        }
        let endpoint = listener::endpoint(&runtime, &config.app, pid)?;
        let mut record = match InstanceRecord::new(&config.app, &config.app_version, pid, endpoint) {
            Ok(record) => record,
            Err(err) => {
                drop(listener);
                #[cfg(unix)]
                let _ = std::fs::remove_file(&socket_path);
                return Err(err);
            }
        };
        let Extensions { assets, caps } = extensions;
        let mut wanted = caps;
        if assets.is_some() {
            wanted.push("assets".to_string());
        }
        for cap in &wanted {
            if let Err(err) = record.add_cap(cap) {
                drop(listener);
                #[cfg(unix)]
                let _ = std::fs::remove_file(&socket_path);
                return Err(err);
            }
        }
        if let Some(instance) = config.instance {
            record.instance = instance;
            if let Err(err) = record.validate() {
                drop(listener);
                #[cfg(unix)]
                let _ = std::fs::remove_file(&socket_path);
                return Err(err);
            }
        }
        // Bind TCP before publishing so the record carries the real port (port 0 picks one).
        let tcp = match config.tcp_port {
            Some(port) => match tcp::bind_loopback(port) {
                Ok(bound) => Some(bound),
                Err(err) => {
                    drop(listener);
                    #[cfg(unix)]
                    let _ = std::fs::remove_file(&socket_path);
                    return Err(err);
                }
            },
            None => None,
        };
        let tcp_port = tcp.as_ref().map(|bound| bound.port);
        if let Some(port) = tcp_port {
            record.set_tcp_port(port);
        }
        let discovery = match Discovery::publish(&runtime, record.clone()) {
            Ok(discovery) => discovery,
            Err(err) => {
                drop(listener);
                return Err(err);
            }
        };
        let hub = Arc::new(Hub {
            limits,
            pending: AtomicUsize::new(0),
            record: record.clone(),
            sink: config.sink,
            activate: config.activate,
            frames: config.frames,
            assets,
            stop: AtomicBool::new(false),
            open: AtomicUsize::new(0),
            next_id: AtomicU64::new(1),
            tickets: Mutex::new(HashMap::new()),
            subs: Mutex::new(Vec::new()),
            threads: Mutex::new(Vec::new()),
        });
        let accept_hub = Arc::clone(&hub);
        let accept = match std::thread::Builder::new().name("craft-ipc-accept".into()).spawn(move || {
            listener::accept_loop(listener, tcp, accept_hub);
        }) {
            Ok(handle) => handle,
            Err(err) => {
                let _ = discovery.remove();
                return Err(err.into());
            }
        };
        Ok(Self { discovery: Some(discovery), hub, accept: Some(accept), tcp_port, record })
    }

    pub fn record(&self) -> &InstanceRecord {
        &self.record
    }

    /// Queue `event` on subscribed connections. This returns after the enqueue,
    /// not after the socket write. A full per-connection queue records one
    /// `events.overflow` and then drops until that overflow line is written.
    pub fn emit(&self, event: &str, data: Value) -> Result<(), LinkError> {
        let subs = lock(&self.hub.subs);
        for sub in subs.iter() {
            events::enqueue(sub, event, &data);
        }
        Ok(())
    }

    /// Stop accept, join connection threads, and delete the instance record.
    pub fn shutdown(mut self) -> Result<(), LinkError> {
        self.stop_threads();
        match self.discovery.take() {
            Some(discovery) => discovery.remove(),
            None => Ok(()),
        }
    }

    /// Bound loopback port, or `None` when TCP was not requested.
    pub fn tcp_port(&self) -> Option<u16> {
        self.tcp_port
    }

    fn stop_threads(&mut self) {
        self.hub.stop.store(true, Ordering::SeqCst);
        events::wake_all(&self.hub.subs);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
        let handles = std::mem::take(&mut *lock(&self.hub.threads));
        for handle in handles {
            let _ = handle.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop_threads();
    }
}

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(err) => err.into_inner(),
    }
}
