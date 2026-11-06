//! The host side of `craft_host.call`.
//!
//! One [`HostDispatcher`] serves one plugin instance. Every call passes four checks in order:
//! the op is in the manifest `requires.ops`, the plugin holds the permission the op needs, the
//! request parses, and the response fits the guest's buffer. An op that this release lists but
//! does not serve (`doc.apply`, `event.poll`, `fs.*`, `net.fetch`) returns `E_UNSUPPORTED`
//! after the first two checks, so a plugin learns it is missing a grant before it learns the host
//! lacks the op.
//!
//! Retries. A guest that gets `E_TOO_SMALL` calls again with a larger buffer. A call with a side
//! effect (`host.log`, `host.progress`, `cmd.execute`, `link.call`, `state.put`) must not run twice for that.
//! The dispatcher keeps the response of the call that did not fit and replays it for an identical
//! retry, so the effect happens once.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use craft_command::Owner;
use craft_plugin_abi::permissions::{DocScope, LINK_CALL_METHODS, LINK_EXEC_METHOD, is_link_call_method};
use craft_plugin_abi::{Op, Permission, Status};
use serde_json::{Value, json};

use crate::engine::{Clock, EngineId, HostDispatch, HostError, HostResult};
use crate::services::{CommandFailure, LinkFailure, LogLevel, QueryFailure, Services, UserNotice};
use crate::store::{DEFAULT_STATE_QUOTA_BYTES, StateStore, StoreError};

/// Limits the dispatcher enforces. The defaults are the ABI numbers.
#[derive(Clone, Debug)]
pub struct DispatchConfig {
    /// `state.*` quota per plugin.
    pub state_quota_bytes: usize,
    /// `host.log` burst size.
    pub log_burst: u32,
    /// `host.log` refill, lines per second.
    pub log_per_second: u32,
    /// A longer log message is cut at this many bytes, on a character boundary.
    pub max_log_bytes: usize,
    /// Largest `random.bytes` request.
    pub max_random_bytes: usize,
    /// Longest `state.*` key.
    pub max_key_bytes: usize,
}

impl Default for DispatchConfig {
    fn default() -> Self {
        DispatchConfig {
            state_quota_bytes: DEFAULT_STATE_QUOTA_BYTES,
            log_burst: 32,
            log_per_second: 64,
            max_log_bytes: 4096,
            max_random_bytes: 4096,
            max_key_bytes: 256,
        }
    }
}

/// Who the dispatcher serves and what that plugin may do.
#[derive(Clone, Debug)]
pub struct PluginContext {
    pub owner: Owner,
    /// `requires.ops` of the manifest. An op outside this set is `E_DENIED`.
    pub declared_ops: HashSet<Op>,
    /// Permissions the user granted.
    pub grants: Vec<Permission>,
    pub engine: EngineId,
    /// Host chosen seed for the `random.bytes` stream a plugin gets without the `random` grant.
    pub random_seed: u64,
}

struct Pending {
    op: Op,
    input: Vec<u8>,
    reply: Vec<u8>,
}

struct Mutable {
    log_tokens: f64,
    log_last: Duration,
    log_dropped: u32,
    pending: Option<Pending>,
    /// SplitMix64 counter for the deterministic stream.
    stream: u64,
}

pub struct HostDispatcher {
    ctx: PluginContext,
    services: Arc<dyn Services>,
    state: Arc<dyn StateStore>,
    clock: Arc<dyn Clock>,
    cancel: Arc<AtomicBool>,
    cfg: DispatchConfig,
    inner: Mutex<Mutable>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn status(s: Status) -> HostError {
    HostError::Status(s)
}

/// Ops whose second run would repeat a visible effect.
fn has_side_effect(op: Op) -> bool {
    matches!(op, Op::HostLog | Op::HostProgress | Op::CmdExecute | Op::LinkCall | Op::StatePut)
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// An app name as a permission writes it: `[a-z][a-z0-9-]*`, at most 32 characters.
fn is_link_name(app: &str) -> bool {
    let mut chars = app.chars();
    app.len() <= 32
        && matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A Craft Link method name such as `engine.execute`: `[a-z][a-z0-9_.]*`, at most 64 characters.
fn is_link_method(method: &str) -> bool {
    let mut chars = method.chars();
    method.len() <= 64
        && matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.'))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from(DIGITS[usize::from(b >> 4)]));
        s.push(char::from(DIGITS[usize::from(b & 15)]));
    }
    s
}

fn parse(input: &[u8]) -> Result<Value, HostError> {
    if input.is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    serde_json::from_slice(input).map_err(|_| status(Status::BadArg))
}

fn text_field<'a>(v: &'a Value, name: &str) -> Result<&'a str, HostError> {
    v.get(name).and_then(Value::as_str).ok_or_else(|| status(Status::BadArg))
}

impl HostDispatcher {
    pub fn new(
        ctx: PluginContext,
        services: Arc<dyn Services>,
        state: Arc<dyn StateStore>,
        clock: Arc<dyn Clock>,
        cancel: Arc<AtomicBool>,
        cfg: DispatchConfig,
    ) -> Arc<HostDispatcher> {
        let now = clock.now();
        let inner = Mutable { log_tokens: f64::from(cfg.log_burst), log_last: now, log_dropped: 0, pending: None, stream: ctx.random_seed };
        Arc::new(HostDispatcher { ctx, services, state, clock, cancel, cfg, inner: Mutex::new(inner) })
    }

    /// The handle the engine takes.
    pub fn handle(self: &Arc<Self>) -> HostDispatch {
        let me = Arc::clone(self);
        Arc::new(move |op: Op, input: &[u8], out: &mut [u8]| -> HostResult { me.dispatch(op, input, out) })
    }

    pub fn owner(&self) -> &Owner {
        &self.ctx.owner
    }

    /// Run one host call.
    pub fn dispatch(&self, op: Op, input: &[u8], out: &mut [u8]) -> HostResult {
        if !self.ctx.declared_ops.contains(&op) {
            return Err(status(Status::Denied));
        }
        if let Some(kind) = op.permission()
            && !op.degrades_without_permission()
            && !self.ctx.grants.iter().any(|g| g.kind() == kind)
        {
            return Err(status(Status::Denied));
        }
        let reply = self.reply_for(op, input)?;
        if reply.len() > out.len() {
            return Err(HostError::TooSmall { required: reply.len() });
        }
        let Some(dst) = out.get_mut(..reply.len()) else {
            return Err(HostError::TooSmall { required: reply.len() });
        };
        dst.copy_from_slice(&reply);
        lock(&self.inner).pending = None;
        Ok(reply.len())
    }

    /// The response bytes. A retry of a side effect call that did not fit replays the stored bytes.
    fn reply_for(&self, op: Op, input: &[u8]) -> Result<Vec<u8>, HostError> {
        {
            let guard = lock(&self.inner);
            if let Some(p) = &guard.pending
                && p.op == op
                && p.input == input
            {
                return Ok(p.reply.clone());
            }
        }
        lock(&self.inner).pending = None;
        let value = self.run(op, input)?;
        let bytes = serde_json::to_vec(&value).map_err(|_| status(Status::Unsupported))?;
        if has_side_effect(op) {
            lock(&self.inner).pending = Some(Pending { op, input: input.to_vec(), reply: bytes.clone() });
        }
        Ok(bytes)
    }

    fn run(&self, op: Op, input: &[u8]) -> Result<Value, HostError> {
        match op {
            Op::HostLog => self.host_log(input),
            Op::HostInfo => Ok(self.host_info()),
            Op::HostCancelled => Ok(json!({ "cancelled": self.cancel.load(Ordering::Relaxed) })),
            Op::HostProgress => self.host_progress(input),
            Op::CmdList => Ok(self.cmd_list()),
            Op::CmdExecute => self.cmd_execute(input),
            Op::DocQuery => self.doc_query(input),
            Op::StateGet => self.state_get(input),
            Op::StatePut => self.state_put(input),
            Op::TimeNow => Ok(self.time_now()),
            Op::RandomBytes => self.random_bytes(input),
            Op::LinkCall => self.link_call(input),
            Op::LinkApps => self.link_apps(),
            // Named in the ABI, not served by this release. The gate already knows the names.
            Op::DocApply | Op::EventPoll | Op::FsOpenPicked | Op::FsRead | Op::FsWrite | Op::NetFetch => Err(status(Status::Unsupported)),
        }
    }

    fn host_log(&self, input: &[u8]) -> Result<Value, HostError> {
        let req = parse(input)?;
        let level = match req.get("level") {
            None => LogLevel::Info,
            Some(v) => v.as_str().and_then(LogLevel::parse).ok_or_else(|| status(Status::BadArg))?,
        };
        let msg = text_field(&req, "msg")?;
        let dropped = {
            let mut m = lock(&self.inner);
            let now = self.clock.now();
            let elapsed = now.saturating_sub(m.log_last).as_secs_f64();
            m.log_last = now;
            m.log_tokens = (m.log_tokens + elapsed * f64::from(self.cfg.log_per_second)).min(f64::from(self.cfg.log_burst));
            if m.log_tokens < 1.0 {
                m.log_dropped = m.log_dropped.saturating_add(1);
                return Err(status(Status::Limit));
            }
            m.log_tokens -= 1.0;
            std::mem::take(&mut m.log_dropped)
        };
        if dropped > 0 {
            self.services.log(&self.ctx.owner, LogLevel::Warn, &format!("{dropped} log lines were dropped by the rate limit"));
        }
        let mut end = msg.len().min(self.cfg.max_log_bytes);
        while !msg.is_char_boundary(end) {
            end -= 1;
        }
        // No newline or escape byte reaches the log, so a plugin cannot forge a line.
        self.services.log(&self.ctx.owner, level, &craft_plugin_abi::text::clean_log(msg.get(..end).unwrap_or("")));
        Ok(json!({}))
    }

    fn host_info(&self) -> Value {
        let info = self.services.host_info();
        let mut ops: Vec<&str> = self.ctx.declared_ops.iter().map(|o| o.name()).collect();
        ops.sort_unstable();
        json!({
            "abi": 1,
            "app": info.app,
            "version": info.version,
            "engine": format!("{:?}", self.ctx.engine).to_lowercase(),
            "plugin": { "id": &*self.ctx.owner.plugin_id, "generation": self.ctx.owner.generation },
            "ops": ops,
        })
    }

    fn host_progress(&self, input: &[u8]) -> Result<Value, HostError> {
        let req = parse(input)?;
        let fraction = req.get("fraction").and_then(Value::as_f64).filter(|f| (0.0..=1.0).contains(f)).ok_or_else(|| status(Status::BadArg))?;
        let message = match req.get("message") {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_str().ok_or_else(|| status(Status::BadArg))?),
        };
        self.services.progress(&self.ctx.owner, fraction, message);
        // A cancelled batch job learns it on the next progress report.
        if self.cancel.load(Ordering::Relaxed) {
            return Err(status(Status::Cancelled));
        }
        Ok(json!({}))
    }

    fn may_run(&self, id: &str) -> bool {
        self.ctx.grants.iter().any(|g| g.allows_command(id))
    }

    fn cmd_list(&self) -> Value {
        let rows: Vec<Value> =
            self.services.list_commands().into_iter().filter(|c| self.may_run(&c.id)).map(|c| json!({ "id": c.id, "label": c.label })).collect();
        json!({ "commands": rows })
    }

    fn cmd_execute(&self, input: &[u8]) -> Result<Value, HostError> {
        let req = parse(input)?;
        let id = text_field(&req, "id")?;
        if !self.may_run(id) {
            return Err(status(Status::Denied));
        }
        let params = req.get("params").cloned().unwrap_or(Value::Null);
        match self.services.execute_command(&self.ctx.owner, id, &params) {
            Ok(result) => Ok(json!({ "result": result })),
            Err(CommandFailure::Unknown) => Err(status(Status::BadArg)),
            Err(CommandFailure::Failed(message)) => Ok(json!({ "error": message })),
        }
    }

    /// `link.call`: `{"app", "method", "params"?}` gives `{"result"}` or `{"error"}`. The
    /// per-app grant is checked here. The missing permission is named in the error text, because a
    /// status code cannot carry it.
    ///
    /// Two routes, both closed:
    /// - `link.call:<app>` forwards only the methods of [`LINK_CALL_METHODS`].
    /// - `engine.execute` also needs `link.exec:<app>:<glob>` for the named command. The host
    ///   sends `{command, params}` and nothing else, and never a host command id.
    ///
    /// Every other method is refused here, before the service sees it.
    fn link_call(&self, input: &[u8]) -> Result<Value, HostError> {
        let req = parse(input)?;
        let app = text_field(&req, "app")?;
        let method = text_field(&req, "method")?;
        if !is_link_name(app) || !is_link_method(method) {
            return Err(status(Status::BadArg));
        }
        let params = match req.get("params") {
            None | Some(Value::Null) => Value::Object(Default::default()),
            Some(p @ Value::Object(_)) => p.clone(),
            Some(_) => return Err(status(Status::BadArg)),
        };
        if !self.has(|g| g.allows_link(app)) {
            return Ok(json!({ "error": format!("missing permission link.call:{app}") }));
        }
        let params = if method == LINK_EXEC_METHOD {
            match self.link_exec_params(app, &params)? {
                Ok(p) => p,
                Err(refusal) => return Ok(json!({ "error": refusal })),
            }
        } else if is_link_call_method(method) {
            params
        } else {
            return Ok(json!({
                "error": format!("link.call:{app} does not allow the method {method}; it forwards only {}", LINK_CALL_METHODS.join(", "))
            }));
        };
        match self.services.link_call(&self.ctx.owner, app, method, &params, &self.cancel) {
            Ok(result) => Ok(json!({ "result": result })),
            Err(LinkFailure::NotRunning(message) | LinkFailure::Failed(message)) => Ok(json!({ "error": message })),
            Err(LinkFailure::Cancelled) => Err(status(Status::Cancelled)),
        }
    }

    /// The params the host sends for `engine.execute`: `{"command", "params"}` rebuilt from the
    /// plugin's request, so a second spelling of the command key (`id`) or any extra key cannot
    /// reach the peer. `Ok(Err(text))` is a refusal the guest can read.
    fn link_exec_params(&self, app: &str, params: &Value) -> Result<Result<Value, String>, HostError> {
        let command = text_field(params, "command")?;
        if !self.has(|g| g.allows_link_exec(app, command)) {
            return Ok(Err(format!("missing permission link.exec:{app}:{}", command.escape_debug())));
        }
        let command_params = match params.get("params") {
            None | Some(Value::Null) => Value::Object(Default::default()),
            Some(p @ Value::Object(_)) => p.clone(),
            Some(_) => return Err(status(Status::BadArg)),
        };
        Ok(Ok(json!({ "command": command, "params": command_params })))
    }

    /// `link.apps`: the running apps this plugin holds `link.call:<app>` for.
    fn link_apps(&self) -> Result<Value, HostError> {
        match self.services.link_apps(&self.ctx.owner) {
            Ok(apps) => {
                let rows: Vec<Value> = apps
                    .into_iter()
                    .filter(|a| self.has(|g| g.allows_link(&a.app)))
                    .map(|a| json!({ "app": a.app, "app_version": a.app_version }))
                    .collect();
                Ok(json!({ "apps": rows }))
            }
            Err(LinkFailure::NotRunning(message) | LinkFailure::Failed(message)) => Ok(json!({ "error": message })),
            Err(LinkFailure::Cancelled) => Err(status(Status::Cancelled)),
        }
    }

    fn doc_query(&self, input: &[u8]) -> Result<Value, HostError> {
        let req = parse(input)?;
        let scope = match req.get("scope").and_then(Value::as_str) {
            None | Some("metadata") => DocScope::Metadata,
            Some("pixels") => DocScope::Pixels,
            Some(_) => return Err(status(Status::BadArg)),
        };
        if !self.ctx.grants.iter().any(|g| g.allows_doc_read(scope)) {
            return Err(status(Status::Denied));
        }
        match self.services.doc_query(&self.ctx.owner, scope, &req) {
            Ok(v) => Ok(v),
            Err(QueryFailure::BadQuery(_)) => Err(status(Status::BadArg)),
            Err(QueryFailure::Unsupported) => Err(status(Status::Unsupported)),
        }
    }

    fn key<'a>(&self, req: &'a Value) -> Result<&'a str, HostError> {
        let key = text_field(req, "key")?;
        if key.is_empty() || key.len() > self.cfg.max_key_bytes {
            return Err(status(Status::BadArg));
        }
        Ok(key)
    }

    fn store_failed(&self, error: &StoreError) -> HostError {
        match error {
            StoreError::QuotaExceeded { .. } => status(Status::Limit),
            other => {
                self.services.notify(UserNotice::new(
                    self.ctx.owner.plugin_id.to_string(),
                    None,
                    &format!("plugin state could not be used: {other}"),
                ));
                status(Status::Unsupported)
            }
        }
    }

    fn state_get(&self, input: &[u8]) -> Result<Value, HostError> {
        let req = parse(input)?;
        let key = self.key(&req)?;
        match self.state.get(&self.ctx.owner.plugin_id, key) {
            Ok(Some(value)) => Ok(json!({ "found": true, "value": value })),
            Ok(None) => Ok(json!({ "found": false })),
            Err(e) => Err(self.store_failed(&e)),
        }
    }

    fn state_put(&self, input: &[u8]) -> Result<Value, HostError> {
        let req = parse(input)?;
        let key = self.key(&req)?;
        let value = req.get("value").ok_or_else(|| status(Status::BadArg))?;
        match self.state.put(&self.ctx.owner.plugin_id, key, value, self.cfg.state_quota_bytes) {
            Ok(()) => Ok(json!({})),
            Err(e) => Err(self.store_failed(&e)),
        }
    }

    fn has(&self, grant: impl Fn(&Permission) -> bool) -> bool {
        self.ctx.grants.iter().any(grant)
    }

    fn time_now(&self) -> Value {
        let ms = if self.has(|g| matches!(g, Permission::Clock)) { self.services.unix_millis() } else { 0 };
        json!({ "unix_ms": ms })
    }

    fn random_bytes(&self, input: &[u8]) -> Result<Value, HostError> {
        let req = parse(input)?;
        let len = req.get("len").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).ok_or_else(|| status(Status::BadArg))?;
        if len > self.cfg.max_random_bytes {
            return Err(status(Status::Limit));
        }
        let mut buf = vec![0u8; len];
        if self.has(|g| matches!(g, Permission::Random)) {
            self.services.fill_random(&mut buf).map_err(|_| status(Status::Unsupported))?;
        } else {
            let mut m = lock(&self.inner);
            for chunk in buf.chunks_mut(8) {
                let word = splitmix64(&mut m.stream).to_le_bytes();
                let n = chunk.len();
                chunk.copy_from_slice(word.get(..n).unwrap_or(&[]));
            }
        }
        Ok(json!({ "hex": hex(&buf) }))
    }
}
