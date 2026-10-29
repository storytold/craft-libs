//! Push job status for the WS-Print backend: a WS-Eventing client plus a small inbound listener.
//! **Opt-in**: nothing in this module runs unless the app calls [`WsdBackend::with_events`](super::WsdBackend::with_events). See `docs/events.md` for the design and the exposure rules.
//!
//! The one idea: **an event is only a wake-up.** A verified event makes the watch of the job it is
//! about poll *now*; the state of the job always comes from `job_status` (`GetJobElements`). Nothing
//! in an event's content is ever turned into a status, so a lost, forged or duplicated event cannot
//! produce a wrong result.
//!
//! * [`NotifyConfig`]: where to listen (`bind`) and which URL the printers are told (`advertise`).
//!   No defaults except [`NotifyConfig::loopback`], which is for tests and local simulators.
//!   `bind` is one concrete local interface address: a wildcard (`0.0.0.0`, `::`) is refused before
//!   any socket is opened.
//! * [`Events`]: the listener thread (the only background thread, joined on shutdown) and the
//!   subscription manager. Everything that *sends* (`Subscribe`, `Renew`, `Unsubscribe`) is
//!   caller-driven: [`Events::maintain`], on the injected clock.
//! * [`Wake`]: what a verified event turns into.

mod http;

use std::collections::VecDeque;
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, TryLockError};
use std::thread::JoinHandle;
use std::time::Duration;

use super::adapter::{EndpointRef, Expiry, Filter, ReferenceParameter, SharedTransport, SubscribeRequest, Subscription, WSE, new_uuid, wse};
use super::client::Client;
use super::error::WsdError;
use super::ns::Event;
use super::policy::{NetworkLimits, PrinterUrl};
use crate::{Clock, ErrorKind, PrintError, WakeReceiver, WakeSender, wake_channel};

/// The events a subscription asks for: job news and printer news, not capability changes.
pub const SUBSCRIBED: [Event; 5] =
    [Event::JobStatus, Event::JobEndState, Event::PrinterStatusSummary, Event::PrinterStatusCondition, Event::PrinterStatusConditionCleared];

/// The longest a single connection may take to deliver its request, whatever
/// [`EventTuning::connection_deadline`] says (5 minutes). The listener serves one connection at a
/// time, so a time without a bound would let one silent peer hold it, and the thread that stops it,
/// for good.
pub const MAX_CONNECTION_DEADLINE: Duration = Duration::from_secs(300);

/// The numbers the listener and the manager run on. The defaults are what the design note says;
/// tests (and apps with a reason) change them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventTuning {
    /// The lease asked for in `Subscribe` and `Renew`. Default 10 minutes.
    pub lease: Duration,
    /// Renew when this share of the granted lease (percent) has passed. Default 50.
    pub renew_percent: u32,
    /// A subscription is trusted only if a verified event arrived within this long. Default 2 minutes.
    pub quiet_window: Duration,
    /// The first wait after a failed `Subscribe`, doubling to `backoff_max`. Default 1 minute.
    pub backoff_min: Duration,
    /// The longest wait between `Subscribe` attempts. Default 10 minutes.
    pub backoff_max: Duration,
    /// Token bucket for incoming connections: burst size. Default 50.
    pub burst: u32,
    /// Token bucket: refilled per second. Default 20.
    pub per_second: u32,
    /// The whole time one connection may take to deliver its request. Default 3 seconds; a longer
    /// time is cut to [`MAX_CONNECTION_DEADLINE`] (5 minutes), so `Duration::MAX` means "the longest
    /// allowed", not "wait forever".
    pub connection_deadline: Duration,
    /// The largest request body accepted, in bytes. Default 64 KiB.
    pub max_body: usize,
    /// The most printers subscribed at once. Default 16.
    pub max_subscriptions: usize,
}

impl Default for EventTuning {
    fn default() -> Self {
        Self {
            lease: Duration::from_secs(600),
            renew_percent: 50,
            quiet_window: Duration::from_secs(120),
            backoff_min: Duration::from_secs(60),
            backoff_max: Duration::from_secs(600),
            burst: 50,
            per_second: 20,
            connection_deadline: Duration::from_secs(3),
            max_body: 64 * 1024,
            max_subscriptions: 16,
        }
    }
}

/// Where the listener listens and what the printers are told. There is no default: the app decides
/// to open a port and which address a printer can reach.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct NotifyConfig {
    /// The socket to listen on: **one concrete local interface address** (loopback, link-local or
    /// private, the policy of a printer URL). A wildcard (`0.0.0.0`, `::`, `::ffff:0.0.0.0`) is
    /// refused. Port 0 picks a port; the advertised port then follows it.
    pub bind: SocketAddr,
    /// The URL (scheme, address and port; the path is ignored) the printers are told to call. It
    /// must pass the numeric local-address policy of a printer URL, and its address must be the
    /// `bind` address, or a private address when the app advertises another one (NAT, a forwarded
    /// port).
    pub advertise: PrinterUrl,
    pub tuning: EventTuning,
    follow_port: bool,
}

impl NotifyConfig {
    /// Listen on `bind` and tell printers `advertise` (`http://<numeric local address>:<port>`).
    /// With port 0 in `bind` the advertised port is replaced by the one the system picked.
    ///
    /// Refused with [`ErrorKind::InvalidRequest`], before any socket is opened: a wildcard `bind`
    /// (`0.0.0.0`, `::`, or the v4-mapped `::ffff:0.0.0.0`: name the interface a printer reaches
    /// you on), a `bind` that is not a loopback, link-local or private address, an `advertise` that
    /// breaks the same numeric local-address policy, and an `advertise` address that is neither the
    /// `bind` address nor a private address (a NAT address the app names on purpose).
    pub fn new(bind: SocketAddr, advertise: &str) -> Result<Self, PrintError> {
        let advertise = PrinterUrl::parse(advertise).map_err(PrintError::from)?;
        check_addresses(bind, &advertise)?;
        Ok(Self { follow_port: bind.port() == 0, bind, advertise, tuning: EventTuning::default() })
    }

    /// [`Self::new`] from text (`"192.168.1.5:9000"`, `"http://192.168.1.5:9000"`), for callers that
    /// hold strings (a settings file, a language binding).
    pub fn parse(bind: &str, advertise: &str) -> Result<Self, PrintError> {
        let bind = bind
            .trim()
            .parse::<SocketAddr>()
            .map_err(|e| PrintError::new(ErrorKind::InvalidRequest, format!("the event listener address {bind:?} is not an ip:port: {e}")))?;
        Self::new(bind, advertise)
    }

    /// `127.0.0.1` on a system-picked port, advertised as such: for tests and for simulators on the
    /// same machine. A real printer cannot call it.
    pub fn loopback() -> Result<Self, PrintError> {
        Self::new(SocketAddr::from(([127, 0, 0, 1], 0)), "http://127.0.0.1:1/")
    }

    /// Use `tuning` for leases, renewals, back-off and the listener's limits.
    #[must_use]
    pub fn with_tuning(mut self, tuning: EventTuning) -> Self {
        self.tuning = tuning;
        self
    }
}

/// A configuration refusal: invalid request, prepare phase, safe to retry once fixed.
fn refused(why: String) -> PrintError {
    PrintError::new(ErrorKind::InvalidRequest, why).with_phase(crate::Phase::Prepare).with_safety(crate::RetrySafety::Safe)
}

/// The rules of the listener's addresses; run when a [`NotifyConfig`] is made and again by
/// [`Events::start`] (its fields are public), always before a socket is opened.
///
/// * `bind` is not a wildcard (`0.0.0.0`, `::`, `::ffff:0.0.0.0`) and passes the numeric
///   local-address policy of a printer URL (loopback, link-local or private).
/// * `advertise` (already checked by the same policy when parsed) is the `bind` address, or a
///   private address (`10/8`, `172.16/12`, `192.168/16`, `fc00::/7`): the app's explicit NAT or
///   port-forward address. A loopback or link-local `advertise` that is not the `bind` address could
///   not be the listener, so it is refused.
fn check_addresses(bind: SocketAddr, advertise: &PrinterUrl) -> Result<(), PrintError> {
    let ip = bind.ip().to_canonical();
    if ip.is_unspecified() {
        return Err(refused(format!(
            "the event listener cannot bind the wildcard address {}: name the one local interface address (loopback, link-local or private) that the printers reach you on",
            bind.ip()
        )));
    }
    if let Err(e) = PrinterUrl::parse(&format!("http://{}/", SocketAddr::new(ip, bind.port().max(1)))) {
        return Err(refused(format!("the event listener address {} is not a loopback, link-local or private address: {e}", bind.ip())));
    }
    let advertised = advertise.host().to_canonical();
    let private = match advertised {
        IpAddr::V4(v4) => v4.is_private(),
        IpAddr::V6(v6) => v6.is_unique_local(),
    };
    if advertised != ip && !private {
        return Err(refused(format!(
            "the advertised address {advertised} is neither the listener's address {ip} nor a private address: a printer cannot reach a loopback or link-local address that is not the listener's own"
        )));
    }
    Ok(())
}

/// One printer's subscription, as plain data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubscriptionInfo {
    /// The printer's native id (its canonical URL).
    pub printer: String,
    /// `Subscribe` succeeded and the lease has not run out.
    pub subscribed: bool,
    /// Subscribed and trusted: a verified event arrived within the quiet window (see `docs/events.md`).
    pub healthy: bool,
    /// Watches that hold the subscription.
    pub watchers: usize,
    /// Consecutive failed `Subscribe` attempts or ends, driving the back-off.
    pub failures: u32,
    /// Whole seconds since the last verified event, if one ever arrived.
    pub seconds_since_event: Option<u64>,
}

/// The listener and its subscriptions as plain data (for a UI, a log or a language binding).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventsStatus {
    /// Where the listener is bound.
    pub listen: String,
    /// The URL printers are told.
    pub advertise: String,
    pub subscriptions: Vec<SubscriptionInfo>,
    pub stats: EventStats,
}

/// What a verified event turns into: whose watch to bring forward. It carries no status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wake {
    /// The printer's native id (its canonical URL).
    pub printer: String,
    /// The job number for a job event; `None` for a printer-level event (every job on the printer).
    pub job: Option<u32>,
}

/// Counters of what the listener did, for diagnostics and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct EventStats {
    /// Connections accepted from a known printer address.
    pub connections: u64,
    /// Verified events (each one produced a [`Wake`]).
    pub wakes: u64,
    /// Verified events seen before (same message id): acknowledged, not acted on.
    pub duplicates: u64,
    /// Connections from an address that is not a printer we subscribed with: closed unread.
    pub rejected_source: u64,
    /// Connections refused by the token bucket.
    pub rejected_rate: u64,
    /// Not a well-formed request or notification (or over a size or time cap).
    pub malformed: u64,
    /// Wrong token, wrong source for the token, or wrong identifier.
    pub forged: u64,
    /// A well-formed notification of a type (or namespace) we did not subscribe to.
    pub unsubscribed_type: u64,
    /// `SubscriptionEnd` messages accepted.
    pub ended: u64,
}

#[derive(Default)]
struct Counters {
    connections: AtomicU64,
    wakes: AtomicU64,
    duplicates: AtomicU64,
    rejected_source: AtomicU64,
    rejected_rate: AtomicU64,
    malformed: AtomicU64,
    forged: AtomicU64,
    unsubscribed_type: AtomicU64,
    ended: AtomicU64,
}

/// Add one to a statistics counter.
fn bump(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

struct Rx {
    job: u32,
    tx: WakeSender,
}

struct Handle {
    subscription: Subscription,
}

/// One printer's subscription, shared by all the jobs watched on it.
struct Sub {
    printer: PrinterUrl,
    token: String,
    handle: Option<Handle>,
    receivers: Vec<Rx>,
    last_event: Option<Duration>,
    expires_at: Duration,
    renew_at: Duration,
    /// A renewal failed, or the printer ended the subscription: not trusted until a renewal or a
    /// new subscription succeeds.
    degraded: bool,
    next_attempt: Duration,
    failures: u32,
    seen: VecDeque<String>,
}

const MAX_SEEN: usize = 256;
const MAX_RECEIVERS: usize = 64;

impl Sub {
    /// Whether events can be trusted now: subscribed, not degraded, the lease not run out, and an event
    /// heard within `quiet`.
    fn healthy(&self, now: Duration, quiet: Duration) -> bool {
        self.handle.is_some() && !self.degraded && now < self.expires_at && self.last_event.is_some_and(|at| now.saturating_sub(at) <= quiet)
    }

    /// Tell every attached wake receiver whether the subscription is currently healthy.
    fn publish_health(&self, now: Duration, quiet: Duration) {
        let healthy = self.healthy(now, quiet);
        for rx in &self.receivers {
            rx.tx.set_healthy(healthy);
        }
    }

    /// Drop wake receivers whose watch has gone away.
    fn prune(&mut self) {
        self.receivers.retain(|rx| rx.tx.is_connected());
    }
}

#[derive(Default)]
struct State {
    subs: Vec<Sub>,
    bucket: http::Bucket,
}

struct Shared {
    clock: Arc<dyn Clock>,
    transport: SharedTransport,
    limits: NetworkLimits,
    tuning: EventTuning,
    advertise: String,
    stop: AtomicBool,
    state: Mutex<State>,
    counters: Counters,
    maintaining: Mutex<()>,
}

/// Lock `mutex`, taking the data even if a panicking thread poisoned it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Compare two secrets without stopping at the first difference.
fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= usize::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0));
    }
    diff == 0
}

/// A fresh 64-hex-digit secret from two random UUIDs, used in the sink URL and as the subscription identifier.
fn new_token() -> String {
    let mut token = String::with_capacity(64);
    for _ in 0..2 {
        token.extend(new_uuid().chars().filter(char::is_ascii_hexdigit));
    }
    token
}

/// The listener and the subscription manager of one backend. Cheap to share through the backend.
pub struct Events {
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
    local: SocketAddr,
}

impl std::fmt::Debug for Events {
    /// The listening address and the advertised base URL; subscriptions and secrets are left out.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Events").field("listening", &self.local).field("advertise", &self.shared.advertise).finish_non_exhaustive()
    }
}

/// What `maintain` decided to send for one subscription.
enum Action {
    Subscribe { printer: PrinterUrl, token: String },
    Renew { printer: PrinterUrl, token: String, subscription: Subscription },
    Unsubscribe { printer: PrinterUrl, subscription: Subscription },
}

impl Events {
    /// Bind the listener and start its thread. This is the only call that opens a socket.
    pub fn start(config: NotifyConfig, clock: Arc<dyn Clock>, transport: SharedTransport, limits: NetworkLimits) -> Result<Self, PrintError> {
        check_addresses(config.bind, &config.advertise)?;
        let listener = TcpListener::bind(config.bind)
            .map_err(|e| PrintError::new(ErrorKind::Unavailable, format!("cannot listen on {} for printer events: {e}", config.bind)))?;
        listener.set_nonblocking(true).map_err(|e| PrintError::new(ErrorKind::Failed, format!("cannot set up the event listener: {e}")))?;
        let local = listener.local_addr().map_err(|e| PrintError::new(ErrorKind::Failed, format!("cannot read the listener address: {e}")))?;
        let port = if config.follow_port { local.port() } else { config.advertise.port() };
        let advertise = SocketAddr::new(config.advertise.host(), port);
        let shared = Arc::new(Shared {
            clock,
            transport,
            limits,
            tuning: config.tuning,
            advertise: format!("http://{advertise}"),
            stop: AtomicBool::new(false),
            state: Mutex::new(State { subs: Vec::new(), bucket: http::Bucket::new(config.tuning.burst, config.tuning.per_second) }),
            counters: Counters::default(),
            maintaining: Mutex::new(()),
        });
        let worker = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("craft-print-events".into())
            .spawn(move || http::serve(&worker, &listener))
            .map_err(|e| PrintError::new(ErrorKind::Failed, format!("cannot start the event listener: {e}")))?;
        Ok(Self { shared, thread: Mutex::new(Some(thread)), local })
    }

    /// The address the listener is bound to.
    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    /// The URL printers are told (scheme, address and port).
    pub fn advertise_url(&self) -> &str {
        &self.shared.advertise
    }

    /// What the listener has done so far.
    pub fn stats(&self) -> EventStats {
        let c = &self.shared.counters;
        let get = |a: &AtomicU64| a.load(Ordering::Relaxed);
        EventStats {
            connections: get(&c.connections),
            wakes: get(&c.wakes),
            duplicates: get(&c.duplicates),
            rejected_source: get(&c.rejected_source),
            rejected_rate: get(&c.rejected_rate),
            malformed: get(&c.malformed),
            forged: get(&c.forged),
            unsubscribed_type: get(&c.unsubscribed_type),
            ended: get(&c.ended),
        }
    }

    /// The listener, its subscriptions and counters as plain data. Takes the clock's `now`, sends nothing.
    pub fn status(&self) -> EventsStatus {
        let now = self.shared.clock.now();
        let quiet = self.shared.tuning.quiet_window;
        let subscriptions = lock(&self.shared.state)
            .subs
            .iter()
            .map(|s| SubscriptionInfo {
                printer: s.printer.as_url(),
                subscribed: s.handle.is_some() && now < s.expires_at,
                healthy: s.healthy(now, quiet),
                watchers: s.receivers.iter().filter(|r| r.tx.is_connected()).count(),
                failures: s.failures,
                seconds_since_event: s.last_event.map(|at| now.saturating_sub(at).as_secs()),
            })
            .collect();
        EventsStatus { listen: self.local.to_string(), advertise: self.shared.advertise.clone(), subscriptions, stats: self.stats() }
    }

    /// Whether the subscription for this printer is established and trusted right now.
    pub fn is_healthy(&self, printer: &PrinterUrl) -> bool {
        let now = self.shared.clock.now();
        let quiet = self.shared.tuning.quiet_window;
        lock(&self.shared.state).subs.iter().any(|s| s.printer == *printer && s.healthy(now, quiet))
    }

    /// Whether any subscription is trusted right now.
    pub fn any_healthy(&self) -> bool {
        let now = self.shared.clock.now();
        let quiet = self.shared.tuning.quiet_window;
        lock(&self.shared.state).subs.iter().any(|s| s.healthy(now, quiet))
    }

    /// The path token of the printer's subscription (what a test client puts in the URL).
    pub fn token_for(&self, printer: &PrinterUrl) -> Option<String> {
        lock(&self.shared.state).subs.iter().find(|s| s.printer == *printer).map(|s| s.token.clone())
    }

    /// Say that a watch wants news about job `job` of `printer`. Sends nothing: the subscription is
    /// made by the next [`Self::maintain`]. The receiver starts unhealthy and turns healthy only
    /// when the subscription has earned trust; dropping it releases the interest. `None` when the
    /// listener is stopping or the subscription limit is reached.
    pub fn register(&self, printer: &PrinterUrl, job: u32) -> Option<WakeReceiver> {
        if self.shared.stop.load(Ordering::Acquire) {
            return None;
        }
        let (tx, rx) = wake_channel();
        let now = self.shared.clock.now();
        let tuning = self.shared.tuning;
        let mut state = lock(&self.shared.state);
        state.subs.iter_mut().for_each(Sub::prune);
        let index = match state.subs.iter().position(|s| s.printer == *printer) {
            Some(i) => i,
            None => {
                if state.subs.len() >= tuning.max_subscriptions {
                    return None;
                }
                state.subs.push(Sub {
                    printer: printer.clone(),
                    token: new_token(),
                    handle: None,
                    receivers: Vec::new(),
                    last_event: None,
                    expires_at: now,
                    renew_at: now,
                    degraded: false,
                    next_attempt: now,
                    failures: 0,
                    seen: VecDeque::new(),
                });
                state.subs.len() - 1
            }
        };
        let sub = state.subs.get_mut(index)?;
        if sub.receivers.len() >= MAX_RECEIVERS {
            return None;
        }
        tx.set_healthy(sub.healthy(now, tuning.quiet_window));
        sub.receivers.push(Rx { job, tx });
        Some(rx)
    }

    /// Whatever is due, on the injected clock: update trust, release subscriptions nobody watches
    /// (`Unsubscribe`), `Subscribe` where a watch is waiting, `Renew` at the renewal point. One
    /// bounded request per subscription at most; returns at once when nothing is due or another
    /// call is already doing this. Never fails: a problem is a subscription that is not healthy.
    pub fn maintain(&self) {
        let shared = &self.shared;
        let _guard = match shared.maintaining.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
            Err(TryLockError::WouldBlock) => return,
        };
        if shared.stop.load(Ordering::Acquire) {
            return;
        }
        let now = shared.clock.now();
        let tuning = shared.tuning;
        let mut actions = Vec::new();
        {
            let mut state = lock(&shared.state);
            let mut index = 0;
            while let Some(sub) = state.subs.get_mut(index) {
                sub.prune();
                if sub.receivers.is_empty() {
                    let sub = state.subs.remove(index);
                    if let Some(handle) = sub.handle {
                        actions.push(Action::Unsubscribe { printer: sub.printer, subscription: handle.subscription });
                    }
                    continue;
                }
                if sub.handle.is_some() && now >= sub.expires_at {
                    // The lease ran out on our clock: the printer has forgotten us.
                    sub.handle = None;
                    sub.degraded = true;
                    sub.next_attempt = now;
                }
                if let Some(handle) = &sub.handle {
                    if now >= sub.renew_at {
                        actions.push(Action::Renew {
                            printer: sub.printer.clone(),
                            token: sub.token.clone(),
                            subscription: handle.subscription.clone(),
                        });
                    }
                } else if now >= sub.next_attempt {
                    actions.push(Action::Subscribe { printer: sub.printer.clone(), token: sub.token.clone() });
                }
                sub.publish_health(now, tuning.quiet_window);
                index += 1;
            }
        }
        for action in actions {
            self.perform(action);
        }
    }

    /// A WSD client for `printer` with the shared transport and limits.
    fn client(&self, printer: &PrinterUrl) -> Client<SharedTransport> {
        Client::new(Arc::clone(&self.shared.transport), printer.clone()).with_limits(self.shared.limits)
    }

    /// Run one network action outside the state lock and record its result. A new subscription gets its
    /// lease and renewal time, or a back-off after a failure; a failed renewal either starts over (the
    /// printer faulted) or retries soon until the lease runs out. Unsubscribe is best effort.
    fn perform(&self, action: Action) {
        let shared = &self.shared;
        let tuning = shared.tuning;
        match action {
            Action::Unsubscribe { printer, subscription } => {
                // Best effort: the lease ends by itself if this fails.
                let _ = wse::unsubscribe(self.client(&printer).soap(), &subscription);
            }
            Action::Subscribe { printer, token } => {
                let result = self.subscribe(&printer, &token);
                let now = shared.clock.now();
                let mut state = lock(&shared.state);
                let Some(sub) = state.subs.iter_mut().find(|s| s.token == token) else {
                    // Released while the request was in flight.
                    if let Ok(subscription) = result {
                        drop(state);
                        let _ = wse::unsubscribe(self.client(&printer).soap(), &subscription);
                    }
                    return;
                };
                match result {
                    Ok(subscription) => {
                        let granted = granted_lease(subscription.expires, tuning.lease);
                        sub.handle = Some(Handle { subscription });
                        sub.expires_at = now.saturating_add(granted);
                        sub.renew_at = now.saturating_add(renew_after(granted, tuning.renew_percent));
                        sub.degraded = false;
                        sub.failures = 0;
                        sub.last_event = None;
                    }
                    Err(_) => {
                        sub.failures = sub.failures.saturating_add(1);
                        sub.next_attempt = now.saturating_add(backoff(sub.failures, tuning));
                    }
                }
                sub.publish_health(now, tuning.quiet_window);
            }
            Action::Renew { printer, token, subscription } => {
                let result = wse::renew(self.client(&printer).soap(), &subscription, Some(Expiry::After(tuning.lease))).map_err(WsdError::from);
                let now = shared.clock.now();
                let mut state = lock(&shared.state);
                let Some(sub) = state.subs.iter_mut().find(|s| s.token == token) else { return };
                match result {
                    Ok(expires) => {
                        let granted = granted_lease(expires, tuning.lease);
                        sub.expires_at = now.saturating_add(granted);
                        sub.renew_at = now.saturating_add(renew_after(granted, tuning.renew_percent));
                        sub.degraded = false;
                    }
                    Err(WsdError::Fault(_)) => {
                        // The printer does not know the subscription any more (it restarted, or
                        // dropped us): start over.
                        sub.handle = None;
                        sub.degraded = true;
                        sub.next_attempt = now;
                    }
                    Err(_) => {
                        // Could not reach the printer: try again soon, until the lease runs out.
                        sub.degraded = true;
                        let left = sub.expires_at.saturating_sub(now);
                        sub.renew_at = now.saturating_add((left / 2).clamp(Duration::from_secs(1), Duration::from_secs(15)));
                    }
                }
                sub.publish_health(now, tuning.quiet_window);
            }
        }
    }

    /// Subscribe at `printer` to the job events this backend follows, delivered to this listener's
    /// sink URLs for `token` (`/n/` for events, `/e/` for the end notice), for one lease.
    fn subscribe(&self, printer: &PrinterUrl, token: &str) -> Result<Subscription, WsdError> {
        let base = &self.shared.advertise;
        let sink = |kind: &str| EndpointRef::new(format!("{base}/{kind}/{token}")).with_parameter(ReferenceParameter::new(WSE, "Identifier", token));
        let uris: Vec<String> = SUBSCRIBED.iter().map(|e| e.uri()).collect();
        let refs: Vec<&str> = uris.iter().map(String::as_str).collect();
        let mut request = SubscribeRequest::new(sink("n"));
        request.end_to = Some(sink("e"));
        request.expires = Some(Expiry::After(self.shared.tuning.lease));
        request.filter = Some(Filter::actions(&refs)?);
        Ok(wse::subscribe(self.client(printer).soap(), &request)?)
    }

    /// Send `Unsubscribe` for every subscription, stop the listener and join its thread. Safe to
    /// call again; also done when the last clone of the backend is dropped.
    pub fn shutdown(&self) {
        if self.shared.stop.swap(true, Ordering::AcqRel) {
            return;
        }
        let handle = lock(&self.thread).take();
        if let Some(handle) = handle {
            let _ = handle.join();
        }
        let subs = std::mem::take(&mut lock(&self.shared.state).subs);
        for sub in subs {
            for rx in &sub.receivers {
                rx.tx.set_healthy(false);
            }
            if let Some(handle) = sub.handle {
                let _ = wse::unsubscribe(self.client(&sub.printer).soap(), &handle.subscription);
            }
        }
    }
}

impl Drop for Events {
    /// Shut down: unsubscribe everything and stop the listener.
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The share of `granted` (percent, at most 100) after which a lease is renewed. A lease so long
/// that the multiplication overflows (an app can set a huge `lease`) is cut by the percentage first.
fn renew_after(granted: Duration, percent: u32) -> Duration {
    let percent = percent.min(100);
    granted.checked_mul(percent).map_or_else(|| (granted / 100).saturating_mul(percent), |d| d / 100)
}

/// The lease the printer granted, or what we asked for when it said nothing we can use. An
/// absolute time is read against the system's wall clock.
fn granted_lease(granted: Option<Expiry>, asked: Duration) -> Duration {
    let lease = match granted {
        Some(Expiry::After(d)) => d,
        Some(Expiry::At(secs)) => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|now| u64::try_from(secs).ok().map(|at| Duration::from_secs(at).saturating_sub(now)))
            .unwrap_or(asked),
        None => asked,
    };
    lease.max(Duration::from_secs(2))
}

/// 1 minute doubling to the cap (by default).
fn backoff(failures: u32, tuning: EventTuning) -> Duration {
    let doublings = failures.saturating_sub(1).min(16);
    tuning.backoff_min.saturating_mul(1u32 << doublings).min(tuning.backoff_max)
}

#[cfg(test)]
mod tests;
