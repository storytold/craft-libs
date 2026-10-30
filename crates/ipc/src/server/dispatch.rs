//! Methods answered on the connection thread, plus the sink forward.

use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use super::events::{self, Subscription};
use super::{Activation, Assets, Hub, lock};
use crate::error::LinkError;
use crate::moniker::Moniker;
use crate::payload::OpenPayload;
use crate::wire::{self, Request};

pub(super) enum Outcome {
    Continue(Value),
    Close(Option<Value>),
    Bulk { reply: Value, ticket: String },
}

pub(super) fn dispatch(hub: &Hub, conn_id: u64, sub: &Subscription, first: bool, req: &Request) -> Outcome {
    match req.method.as_str() {
        "link.hello" => continue_result(&req.id, hello(hub, &req.params)),
        "events.subscribe" => continue_result(&req.id, subscribe(sub, &req.params)),
        "events.unsubscribe" => {
            events::unsubscribe(sub);
            Outcome::Continue(wire::reply_ok(&req.id, json!({})))
        }
        "link.bye" => {
            events::unsubscribe(sub);
            release_tickets(hub, conn_id);
            Outcome::Close(Some(wire::reply_ok(&req.id, json!({}))))
        }
        "app.activate" => continue_result(&req.id, activate(hub, &req.params)),
        "link.resolve" if hub.assets.is_some() => continue_result(&req.id, with_assets(hub, |assets| resolve(assets, &req.params))),
        "asset.describe" if hub.assets.is_some() => continue_result(&req.id, with_assets(hub, |assets| describe(assets, &req.params))),
        "link.refresh" if hub.assets.is_some() => continue_result(&req.id, with_assets(hub, |assets| refresh(assets, &req.params))),
        "link.open" => match validate_open(&req.params) {
            Ok(()) => forward(hub, &req.method, &req.id, req.params.clone()),
            Err(err) => Outcome::Continue(wire::reply_err(&req.id, &err)),
        },
        "frames.open" => continue_result(&req.id, frames_open(hub, conn_id, &req.params)),
        "bulk.attach" => bulk_attach(hub, first, req),
        method => forward(hub, method, &req.id, req.params.clone()),
    }
}

pub(super) fn release_tickets(hub: &Hub, conn_id: u64) {
    let owned = {
        let mut tickets = lock(&hub.tickets);
        let owned: Vec<String> = tickets.iter().filter(|(_, id)| **id == conn_id).map(|(ticket, _)| ticket.clone()).collect();
        for ticket in &owned {
            tickets.remove(ticket);
        }
        owned
    };
    for ticket in owned {
        hub.frames.close_ticket(&ticket);
    }
}

fn continue_result(id: &Value, result: Result<Value, LinkError>) -> Outcome {
    match result {
        Ok(result) => Outcome::Continue(wire::reply_ok(id, result)),
        Err(err) => Outcome::Continue(wire::reply_err(id, &err)),
    }
}

fn hello(hub: &Hub, params: &Value) -> Result<Value, LinkError> {
    let obj = wire::params_object(params)?;
    let protocol = obj.get("protocol").and_then(Value::as_object).ok_or_else(|| LinkError::BadParams("protocol required".into()))?;
    let min = required_u32(protocol.get("min"), "protocol.min")?;
    let max = required_u32(protocol.get("max"), "protocol.max")?;
    if !wire::protocols_overlap(min, max) {
        return Err(LinkError::Protocol { server_min: wire::PROTOCOL_MIN, server_max: wire::PROTOCOL_MAX });
    }
    let record = &hub.record;
    let mut caps: Vec<String> = ["events", "activate", "link.receive", "frames"].iter().map(|c| (*c).to_string()).collect();
    for cap in record.caps() {
        if !caps.contains(&cap) {
            caps.push(cap);
        }
    }
    Ok(json!({
        "app": record.app,
        "app_version": record.app_version,
        "pid": record.pid,
        "instance": record.instance,
        "protocol": wire::PROTOCOL_MAX,
        "caps": caps,
    }))
}

fn subscribe(sub: &Subscription, params: &Value) -> Result<Value, LinkError> {
    let obj = wire::params_object(params)?;
    let list = obj.get("events").and_then(Value::as_array).ok_or_else(|| LinkError::BadParams("events must be an array".into()))?;
    let mut names = Vec::with_capacity(list.len());
    for item in list {
        let Some(name) = item.as_str() else {
            return Err(LinkError::BadParams("event name must be a string".into()));
        };
        names.push(name.to_string());
    }
    events::subscribe(sub, names);
    Ok(json!({"seq": 0}))
}

fn activate(hub: &Hub, params: &Value) -> Result<Value, LinkError> {
    let obj = wire::params_object(params)?;
    let token = optional_string(obj, "token")?;
    let from_pid = optional_u32(obj.get("from_pid"), "from_pid")?;
    let open = match obj.get("open") {
        None => None,
        Some(Value::Object(open)) => {
            if let Some(asset) = open.get("asset") {
                let text = asset.as_str().ok_or_else(|| LinkError::BadParams("open.asset must be a moniker string".into()))?;
                Moniker::parse(text).map_err(|err| LinkError::BadParams(format!("open.asset: {err}")))?;
            }
            Some(Value::Object(open.clone()))
        }
        Some(_) => return Err(LinkError::BadParams("open must be an object".into())),
    };
    let activated = match hub.activate.activate(token, from_pid, open) {
        Activation::Yes => Value::Bool(true),
        Activation::No => Value::Bool(false),
        Activation::Requested => Value::String("requested".into()),
    };
    Ok(json!({"activated": activated}))
}

fn with_assets(hub: &Hub, run: impl FnOnce(&dyn Assets) -> Result<Value, LinkError>) -> Result<Value, LinkError> {
    match &hub.assets {
        Some(assets) => run(assets.as_ref()),
        None => Err(LinkError::NotReady("this app has no asset provider".into())),
    }
}

fn asset_param(params: &Value) -> Result<Moniker, LinkError> {
    let obj = wire::params_object(params)?;
    let text = obj.get("asset").and_then(Value::as_str).ok_or_else(|| LinkError::BadParams("asset must be a moniker string".into()))?;
    Moniker::parse(text).map_err(|err| LinkError::BadParams(format!("asset: {err}")))
}

fn resolve(assets: &dyn Assets, params: &Value) -> Result<Value, LinkError> {
    let asset = asset_param(params)?;
    Ok(serde_json::to_value(assets.resolve(&asset)?)?)
}

fn describe(assets: &dyn Assets, params: &Value) -> Result<Value, LinkError> {
    let asset = asset_param(params)?;
    Ok(serde_json::to_value(assets.describe(&asset)?)?)
}

fn refresh(assets: &dyn Assets, params: &Value) -> Result<Value, LinkError> {
    let obj = wire::params_object(params)?;
    let link_id = obj
        .get("link_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| LinkError::BadParams("link_id must be a non-empty string".into()))?;
    Ok(serde_json::to_value(assets.refresh(link_id)?)?)
}

/// `link.open` carries a `payload` whose shape depends on `format`. A bad one
/// never reaches the app. Params that are not an object stay `bad_params`.
fn validate_open(params: &Value) -> Result<(), LinkError> {
    let obj = wire::params_object(params)?;
    let payload = obj.get("payload").ok_or_else(|| LinkError::PayloadInvalid("payload is required".into()))?;
    OpenPayload::from_value(payload)?;
    match obj.get("result_path") {
        None | Some(Value::Null) => Ok(()),
        Some(Value::String(path)) => crate::payload::check_peer_path(path, "result_path"),
        Some(_) => Err(LinkError::PayloadInvalid("result_path must be a string".into())),
    }
}

fn frames_open(hub: &Hub, conn_id: u64, params: &Value) -> Result<Value, LinkError> {
    let obj = wire::params_object(params)?;
    let opened = hub.frames.open(obj)?;
    if opened.ticket.len() != 32 || !crate::auth::is_lower_hex(&opened.ticket) {
        hub.frames.close_ticket(&opened.ticket);
        return Err(LinkError::BadParams("ticket must be 32 lowercase hex characters".into()));
    }
    let mut tickets = lock(&hub.tickets);
    if tickets.contains_key(&opened.ticket) {
        return Err(LinkError::BadParams("ticket already registered".into()));
    }
    tickets.insert(opened.ticket.clone(), conn_id);
    drop(tickets);
    Ok(json!({
        "ticket": opened.ticket,
        "w": opened.w,
        "h": opened.h,
        "stride": opened.stride,
        "len": opened.len,
    }))
}

fn bulk_attach(hub: &Hub, first: bool, req: &Request) -> Outcome {
    if !first {
        let err = LinkError::BadParams("bulk.attach must be the first method".into());
        return Outcome::Close(Some(wire::reply_err(&req.id, &err)));
    }
    let ticket = match ticket_param(&req.params) {
        Ok(ticket) => ticket,
        Err(err) => return Outcome::Close(Some(wire::reply_err(&req.id, &err))),
    };
    let known = lock(&hub.tickets).contains_key(&ticket);
    if !known {
        let err = LinkError::BadParams("unknown ticket".into());
        return Outcome::Close(Some(wire::reply_err(&req.id, &err)));
    }
    Outcome::Bulk { reply: wire::reply_ok(&req.id, json!({})), ticket }
}

fn ticket_param(params: &Value) -> Result<String, LinkError> {
    let obj = wire::params_object(params)?;
    let Some(ticket) = obj.get("ticket").and_then(Value::as_str) else {
        return Err(LinkError::BadParams("ticket required".into()));
    };
    Ok(ticket.to_string())
}

fn forward(hub: &Hub, method: &str, id: &Value, params: Value) -> Outcome {
    let Some(rx) = hub.sink.submit(method.to_string(), params) else {
        return Outcome::Close(None);
    };
    let deadline = Instant::now() + Duration::from_secs(wire::REPLY_DEADLINE_SECS);
    loop {
        if hub.stop.load(std::sync::atomic::Ordering::SeqCst) {
            return Outcome::Close(None);
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Outcome::Continue(timeout_reply(method, id));
        }
        let slice = left.min(Duration::from_millis(200));
        match rx.recv_timeout(slice) {
            Ok(result) => return Outcome::Continue(adopt_sink_reply(id, result)),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Outcome::Close(None),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

/// FilmCraft's UI sends a finished reply object (`ok` plus `result` or `error`).
/// A value with `ok` is that object. Insert `id` and do not wrap it again.
/// Any other value is a result payload and gets [`wire::reply_ok`].
fn adopt_sink_reply(id: &Value, value: Value) -> Value {
    let Some(obj) = value.as_object() else {
        return wire::reply_ok(id, value);
    };
    if !obj.contains_key("ok") {
        return wire::reply_ok(id, value);
    }
    let mut reply = value;
    if let Some(map) = reply.as_object_mut()
        && !map.contains_key("id")
    {
        map.insert("id".to_string(), id.clone());
    }
    reply
}

fn timeout_reply(method: &str, id: &Value) -> Value {
    if is_new_method(method) { wire::reply_err(id, &LinkError::Timeout) } else { wire::reply_err_text(id, "timeout") }
}

/// New methods use an error object. Legacy names keep the string `"timeout"`.
fn is_new_method(method: &str) -> bool {
    matches!(
        method,
        "link.open"
            | "link.attach"
            | "link.focus"
            | "link.close"
            | "link.hello"
            | "events.subscribe"
            | "events.unsubscribe"
            | "link.bye"
            | "app.activate"
            | "frames.open"
            | "bulk.attach"
            | "link.resolve"
            | "asset.describe"
    )
}

fn optional_string(obj: &Map<String, Value>, key: &str) -> Result<Option<String>, LinkError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(LinkError::BadParams(format!("{key} must be a string or null"))),
    }
}

fn optional_u32(value: Option<&Value>, key: &str) -> Result<Option<u32>, LinkError> {
    match value {
        None => Ok(None),
        Some(value) => required_u32(Some(value), key).map(Some),
    }
}

fn required_u32(value: Option<&Value>, key: &str) -> Result<u32, LinkError> {
    let Some(value) = value else {
        return Err(LinkError::BadParams(format!("{key} is required")));
    };
    let Some(number) = value.as_u64() else {
        return Err(LinkError::BadParams(format!("{key} must be a number")));
    };
    u32::try_from(number).map_err(|_| LinkError::BadParams(format!("{key} is out of range")))
}
