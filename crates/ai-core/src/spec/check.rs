//! The checker: every rule a spec must keep, applied once when it is loaded.
//!
//! An error says where (`flow.submit.call.path`) and why, in words a person can act on.

use std::collections::BTreeSet;

use super::template::{self, Slot};
use super::{
    Call, Extract, ExtractAs, Flow, MAX_MODELS, MAX_PATH_BYTES, MAX_POINTER_DEPTH, MAX_SPEC_BYTES, MAX_STATES, MAX_TEMPLATE_BYTES, Method,
    SPEC_VERSION, Spec, SpecAuth, SpecModel, StateWord,
};
use crate::capability::CapabilityId;
use crate::connection::check_key_header;
use crate::endpoint::Endpoint;
use crate::ids::{ConnectionId, ModelId};
use crate::model::InputRole;
use crate::payload::validate_media_type;

/// Why a spec was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpecError {
    /// Where in the spec, such as `flow.submit.call.path`.
    pub at: String,
    pub why: String,
}

impl std::fmt::Display for SpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.at, self.why)
    }
}

impl std::error::Error for SpecError {}

fn err(at: impl Into<String>, why: impl Into<String>) -> SpecError {
    SpecError { at: at.into(), why: why.into() }
}

const SEND: &[Slot] = &[Slot::Prompt, Slot::Model];
const FOLLOW: &[Slot] = &[Slot::Model, Slot::JobId];
const NOTHING: &[Slot] = &[];

pub(super) fn parse(text: &str) -> Result<Spec, SpecError> {
    if text.len() > MAX_SPEC_BYTES {
        return Err(err("spec", format!("this spec is too large (the limit is {MAX_SPEC_BYTES} bytes)")));
    }
    let spec: Spec = serde_json::from_str(text).map_err(|e| err("spec", format!("not a spec this version can read: {e}")))?;
    validate(&spec)?;
    Ok(spec)
}

fn validate(spec: &Spec) -> Result<(), SpecError> {
    if spec.spec_version != SPEC_VERSION {
        return Err(err("spec_version", "only version 1 is read"));
    }
    ConnectionId::new(&spec.id).map_err(|e| err("id", e.to_string()))?;
    check_label("label", &spec.label)?;
    check_base(&spec.base)?;
    if let SpecAuth::Header { name } = &spec.auth {
        check_key_header(name).map_err(|why| err("auth.name", why))?;
    }
    if let Some(probe) = &spec.probe {
        if probe.method != Method::Get {
            return Err(err("probe.method", "the probe must be a GET"));
        }
        check_call("probe", probe, NOTHING, false)?;
    }
    check_models(&spec.models)?;
    let sent = check_flow(&spec.flow)?;
    if !sent.contains(&Slot::Prompt) {
        return Err(err("flow", "the prompt is never sent: use {prompt} in the request"));
    }
    if let Some(i) = spec.models.iter().position(|m| !m.requires.iter().any(|r| r == "prompt")) {
        return Err(err(format!("models[{i}].requires"), "a spec that sends {prompt} needs every model to require a prompt"));
    }
    check_limits(spec)
}

fn check_label(at: &str, label: &str) -> Result<(), SpecError> {
    if label.trim().is_empty() || label.chars().count() > 120 || label.chars().any(char::is_control) {
        return Err(err(at, "a label is 1 to 120 plain characters"));
    }
    Ok(())
}

fn check_base(base: &str) -> Result<(), SpecError> {
    Endpoint::parse(base).map_err(|e| err("base", e.to_string()))?;
    if base.contains(['?', '#']) {
        return Err(err("base", "a base has no query or fragment"));
    }
    let after_scheme = base.split_once("://").map_or("", |(_, rest)| rest);
    if let Some(i) = after_scheme.find('/') {
        let prefix = after_scheme[i..].trim_end_matches('/');
        if !prefix.is_empty() {
            check_path(prefix).map_err(|why| err("base", why))?;
        }
    }
    Ok(())
}

fn check_path(path: &str) -> Result<(), &'static str> {
    if path.contains("://") || path.starts_with("//") {
        return Err("a path cannot name another host");
    }
    if !path.starts_with('/') {
        return Err("a path starts with /");
    }
    if path.len() > MAX_PATH_BYTES {
        return Err("a path is too long (at most 256 bytes)");
    }
    if path.contains(['?', '#']) {
        return Err("no ? or # in a path (put query values in query)");
    }
    if path.contains(['%', '\\']) {
        return Err("no backslash or % escape in a path");
    }
    if !path.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("a path uses plain visible characters only");
    }
    if path.split('/').any(|s| s == "..") {
        return Err("no '..' in a path");
    }
    if path.contains("//") {
        return Err("no '//' in a path");
    }
    Ok(())
}

fn check_pointer(at: &str, pointer: &str) -> Result<(), SpecError> {
    if !pointer.starts_with('/') {
        return Err(err(at, "a pointer starts with /"));
    }
    if pointer.len() > 128 || pointer.chars().any(char::is_control) {
        return Err(err(at, "a pointer is at most 128 plain characters"));
    }
    if pointer.matches('/').count() > MAX_POINTER_DEPTH {
        return Err(err(at, "a pointer goes at most 8 levels deep"));
    }
    Ok(())
}

fn slot_name(slot: Slot) -> &'static str {
    match slot {
        Slot::Prompt => "{prompt}",
        Slot::Model => "{model}",
        Slot::JobId => "{job_id}",
    }
}

/// Checks one call and returns the slots it uses.
fn check_call(at: &str, call: &Call, allow: &[Slot], need_job: bool) -> Result<Vec<Slot>, SpecError> {
    let mut used = Vec::new();
    check_path(&call.path).map_err(|why| err(format!("{at}.path"), why))?;
    for slot in template::slots(&call.path).map_err(|why| err(format!("{at}.path"), why))? {
        if slot != Slot::JobId {
            return Err(err(format!("{at}.path"), format!("only {{job_id}} may be used in a path, not {}", slot_name(slot))));
        }
        used.push(slot);
    }
    if call.query.len() > 8 {
        return Err(err(format!("{at}.query"), "a call has at most 8 query values"));
    }
    for (key, value) in &call.query {
        let ok = !key.is_empty() && key.len() <= 64 && key.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
        if !ok || value.len() > MAX_PATH_BYTES {
            return Err(err(format!("{at}.query"), "query names are plain, values at most 256 bytes"));
        }
        used.extend(template::slots(value).map_err(|why| err(format!("{at}.query"), why))?);
    }
    if let Some(body) = &call.body {
        if call.method != Method::Post {
            return Err(err(format!("{at}.body"), "only a POST has a body"));
        }
        if body.to_string().len() > MAX_TEMPLATE_BYTES {
            return Err(err(format!("{at}.body"), "a body template is at most 4 KiB"));
        }
        used.extend(template::value_slots(body).map_err(|why| err(format!("{at}.body"), why))?);
    }
    if let Some(slot) = used.iter().find(|s| !allow.contains(s)) {
        return Err(err(at, format!("{} is not available in this call", slot_name(*slot))));
    }
    if need_job && !used.contains(&Slot::JobId) {
        return Err(err(at, "this call never uses {job_id}, so it cannot name the job"));
    }
    Ok(used)
}

fn check_models(models: &[SpecModel]) -> Result<(), SpecError> {
    if models.is_empty() {
        return Err(err("models", "a spec offers at least one model"));
    }
    if models.len() > MAX_MODELS {
        return Err(err("models", "a spec offers at most 16 models"));
    }
    let mut seen = BTreeSet::new();
    for (i, m) in models.iter().enumerate() {
        let at = |field: &str| format!("models[{i}].{field}");
        ModelId::new(&m.id).map_err(|e| err(at("id"), e.to_string()))?;
        if !seen.insert(&m.id) {
            return Err(err(at("id"), "this model id is used twice"));
        }
        check_label(&at("label"), &m.label)?;
        CapabilityId::parse(&m.capability).map_err(|e| err(at("capability"), e.to_string()))?;
        if m.accepts.is_empty() {
            return Err(err(at("accepts"), "a model accepts at least one input"));
        }
        for (role, count) in &m.accepts {
            let parsed = InputRole::parse(role).map_err(|e| err(at("accepts"), e.to_string()))?;
            if parsed != InputRole::Prompt {
                return Err(err(at("accepts"), format!("unsupported: input '{role}' (version 1 sends a prompt only)")));
            }
            if *count != 1 {
                return Err(err(at("accepts"), "the prompt count must be 1 (one prompt is sent)"));
            }
        }
        if let Some(r) = m.requires.iter().find(|r| !m.accepts.contains_key(*r)) {
            return Err(err(at("requires"), format!("'{r}' is required, so it must also be accepted")));
        }
        if !(1..=4).contains(&m.max_outputs) {
            return Err(err(at("max_outputs"), "outputs are 1 to 4"));
        }
    }
    Ok(())
}

fn check_extract(at: &str, ex: &Extract) -> Result<(), SpecError> {
    check_pointer(&format!("{at}.at"), &ex.at)?;
    match (ex.kind, &ex.media_type) {
        (ExtractAs::Text, Some(_)) => Err(err(at, "a text result carries no media_type")),
        (ExtractAs::Url | ExtractAs::Base64, None) => Err(err(at, "a media_type is needed for a link or base64 result")),
        (_, Some(mt)) => validate_media_type(mt).map_err(|_| err(at, format!("media_type {mt:?} is not a media type"))),
        (ExtractAs::Text, None) => Ok(()),
    }
}

/// Checks the flow and returns the slots its first request sends.
fn check_flow(flow: &Flow) -> Result<Vec<Slot>, SpecError> {
    match flow {
        Flow::RequestResponse { call, result } => {
            let sent = check_call("flow.call", call, SEND, false)?;
            check_extract("flow.result", result)?;
            Ok(sent)
        }
        Flow::Queue { submit, poll, cancel } => {
            let sent = check_call("flow.submit.call", &submit.call, SEND, false)?;
            check_pointer("flow.submit.job_id_at", &submit.job_id_at)?;
            check_call("flow.poll.call", &poll.call, FOLLOW, true)?;
            check_pointer("flow.poll.status_at", &poll.status_at)?;
            check_states("flow.poll.states", &poll.states)?;
            check_extract("flow.poll.result", &poll.result)?;
            if let Some(cancel) = cancel {
                check_call("flow.cancel", cancel, FOLLOW, true)?;
            }
            Ok(sent)
        }
    }
}

fn check_states(at: &str, states: &std::collections::BTreeMap<String, StateWord>) -> Result<(), SpecError> {
    if states.len() > MAX_STATES {
        return Err(err(at, "a status table has at most 16 words"));
    }
    if states.keys().any(|w| w.is_empty() || w.len() > 64 || w.chars().any(char::is_control)) {
        return Err(err(at, "a status word is 1 to 64 plain characters"));
    }
    for (state, name) in [(StateWord::Succeeded, "succeeded"), (StateWord::Failed, "failed")] {
        if !states.values().any(|s| *s == state) {
            return Err(err(at, format!("the table needs a word that means {name}")));
        }
    }
    Ok(())
}

fn check_limits(spec: &Spec) -> Result<(), SpecError> {
    if !(1..=10_000).contains(&spec.limits.max_polls) {
        return Err(err("limits.max_polls", "status checks are 1 to 10000"));
    }
    if !(1..=3600).contains(&spec.limits.max_seconds) {
        return Err(err("limits.max_seconds", "seconds are 1 to 3600"));
    }
    Ok(())
}
