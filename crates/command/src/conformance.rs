//! Conformance checks for an app's command seam.
//!
//! These tests do not prove that a command behaves correctly with valid
//! params. T3's job events may use fake job results. The host wires the real
//! seam. This module only checks the counts and the outcome classes.
//!
//! `run_all` returns `Err` on failure. It does not panic: this crate forbids
//! `panic` in library code. The app test decides how to surface the error.

use serde_json::Value;

/// Outcome class of one command call. The host maps its own result onto this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutcomeClass {
    /// A gate vetoed the call. `after` still runs.
    Vetoed,
    /// The command resolved and was disabled.
    Disabled,
    /// The command returned success.
    Success,
    /// The command failed, or the id did not resolve.
    Failure,
}

/// Why a document change was emitted. T4 only requires `Untracked`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeCause {
    /// A write skipped `bump`, and `flush` reported it.
    Untracked,
    /// Any other cause. A T4 flush that reports this fails.
    Other,
}

/// One conformance failure. `test` is `T1` through `T6`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConformanceError {
    /// Which check failed.
    pub test: &'static str,
    /// What differed.
    pub detail: String,
}

impl std::fmt::Display for ConformanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.test, self.detail)
    }
}

impl std::error::Error for ConformanceError {}

fn fail(test: &'static str, detail: impl Into<String>) -> ConformanceError {
    ConformanceError { test, detail: detail.into() }
}

/// What an adopting app supplies. The types are strings and JSON so the app
/// does not have to expose its session type here.
pub trait CommandHost {
    /// A new host with a demo document and the app's command tables.
    fn fresh() -> Self;

    /// Ids `for_each` visits, built-in and dynamic, in visit order.
    fn command_ids(&self) -> Vec<String>;

    /// UI-only ids. These may be absent from `command_ids`.
    fn ui_command_ids(&self) -> Vec<String>;

    /// Run one registry command. Record `before` and `after` on the host.
    fn execute(&mut self, id: &str, params: &Value) -> OutcomeClass;

    /// Run one UI-only command through the same gate and observer.
    fn invoke_ui(&mut self, id: &str, params: &Value) -> OutcomeClass;

    /// Hash of the document. A vetoed call must leave this unchanged.
    fn fingerprint(&self) -> u64;

    /// Document revision. A vetoed call must leave this unchanged.
    fn revision(&self) -> u64;

    /// Install or remove a gate that records every id and vetoes the call.
    fn set_veto_all(&mut self, enabled: bool);

    /// Ids passed to `before` since the last take, in call order.
    fn take_before_ids(&mut self) -> Vec<String>;

    /// Outcome classes passed to `after` since the last take, in call order.
    fn take_after(&mut self) -> Vec<OutcomeClass>;

    /// Schema for a registry id. `None` when the id is unknown.
    fn schema(&self, id: &str) -> Option<Value>;

    /// How many scripted steps T3 should run. Must be at least 1.
    fn script_len(&self) -> usize;

    /// Run script step `index`. Return how many change events it emitted.
    fn run_script_step(&mut self, index: usize) -> u64;

    /// Write the document without `bump`.
    fn write_without_bump(&mut self);

    /// Flush pending writes. A dirty unbumped write emits one `Untracked`.
    fn flush(&mut self) -> Vec<ChangeCause>;

    /// Register one dynamic command, its gate, and a user shortcut.
    /// Returns `(owner, command_id)`.
    fn prepare_unregister(&mut self) -> (String, String);

    /// Drop every command, menu entry, and gate owned by `owner`.
    /// Keep the user shortcut binding.
    fn unregister_owner(&mut self, owner: &str);

    /// The command still resolves through the registry.
    fn resolves(&self, id: &str) -> bool;

    /// The command is still listed in a menu.
    fn menu_lists(&self, id: &str) -> bool;

    /// The user shortcut binding is still present.
    fn user_shortcut_kept(&self, id: &str) -> bool;
}

/// Run T1 through T6 on a fresh host.
pub fn run_all<H: CommandHost>() -> Result<(), ConformanceError> {
    run_with(&mut H::fresh())
}

/// Run T1 through T6 on `host`. Stops at the first failure.
pub fn run_with<H: CommandHost>(host: &mut H) -> Result<(), ConformanceError> {
    t1(host)?;
    t2(host)?;
    t3(host)?;
    t4(host)?;
    t5(host)?;
    t6(host)?;
    Ok(())
}

fn t1<H: CommandHost>(host: &mut H) -> Result<(), ConformanceError> {
    let ids = host.command_ids();
    if ids.is_empty() {
        return Err(fail("T1", "command_ids is empty, so the gate check would pass without seeing a command"));
    }
    let ui = host.ui_command_ids();
    let fingerprint = host.fingerprint();
    let revision = host.revision();
    host.set_veto_all(true);
    let _ = host.take_before_ids();

    for id in &ids {
        let class = host.execute(id, &Value::Null);
        if class != OutcomeClass::Vetoed {
            return Err(fail("T1", format!("command {id} returned {class:?}, expected Vetoed")));
        }
        if host.fingerprint() != fingerprint || host.revision() != revision {
            return Err(fail("T1", format!("command {id} changed the fingerprint or the revision under a veto")));
        }
    }
    let seen = host.take_before_ids();
    if seen != ids {
        return Err(fail("T1", format!("veto gate saw {seen:?}, command ids are {ids:?}")));
    }
    println!("observed {}/{} commands", seen.len(), ids.len());

    let _ = host.take_before_ids();
    for id in &ui {
        let class = host.invoke_ui(id, &Value::Null);
        if class != OutcomeClass::Vetoed {
            return Err(fail("T1", format!("ui command {id} returned {class:?}, expected Vetoed")));
        }
        if host.fingerprint() != fingerprint || host.revision() != revision {
            return Err(fail("T1", format!("ui command {id} changed the fingerprint or the revision under a veto")));
        }
    }
    let seen_ui = host.take_before_ids();
    if seen_ui != ui {
        return Err(fail("T1", format!("veto gate saw ui ids {seen_ui:?}, ui ids are {ui:?}")));
    }
    println!("ui commands {}/{}", seen_ui.len(), ui.len());
    host.set_veto_all(false);
    Ok(())
}

fn t2<H: CommandHost>(host: &mut H) -> Result<(), ConformanceError> {
    host.set_veto_all(false);
    let _ = host.take_after();
    let ids = host.command_ids();
    let mut results = Vec::with_capacity(ids.len());
    for id in &ids {
        results.push(host.execute(id, &Value::Null));
    }
    let after = host.take_after();
    if after.len() != results.len() {
        return Err(fail("T2", format!("after ran {} times for {} command calls", after.len(), results.len())));
    }
    for (index, (got, want)) in after.iter().zip(results.iter()).enumerate() {
        if got != want {
            return Err(fail("T2", format!("command call {index} after class {got:?} does not match result {want:?}")));
        }
    }

    let _ = host.take_after();
    let ui = host.ui_command_ids();
    let mut ui_results = Vec::with_capacity(ui.len());
    for id in &ui {
        ui_results.push(host.invoke_ui(id, &Value::Null));
    }
    let ui_after = host.take_after();
    if ui_after.len() != ui_results.len() {
        return Err(fail("T2", format!("after ran {} times for {} ui calls", ui_after.len(), ui_results.len())));
    }
    for (index, (got, want)) in ui_after.iter().zip(ui_results.iter()).enumerate() {
        if got != want {
            return Err(fail("T2", format!("ui call {index} after class {got:?} does not match result {want:?}")));
        }
    }
    Ok(())
}

fn t3<H: CommandHost>(host: &mut H) -> Result<(), ConformanceError> {
    let len = host.script_len();
    if len == 0 {
        return Err(fail("T3", "script_len is 0, so the revision check would pass without a step"));
    }
    for index in 0..len {
        let revision_before = host.revision();
        let fingerprint_before = host.fingerprint();
        let changes = host.run_script_step(index);
        let revision_after = host.revision();
        let Some(delta) = revision_after.checked_sub(revision_before) else {
            return Err(fail("T3", format!("step {index} moved the revision backwards")));
        };
        if changes != delta {
            return Err(fail("T3", format!("step {index} emitted {changes} changes but the revision moved by {delta}")));
        }
        if host.fingerprint() != fingerprint_before && changes == 0 {
            return Err(fail("T3", format!("step {index} changed the fingerprint without a change event")));
        }
    }
    Ok(())
}

fn t4<H: CommandHost>(host: &mut H) -> Result<(), ConformanceError> {
    host.write_without_bump();
    let causes = host.flush();
    if causes != [ChangeCause::Untracked] {
        return Err(fail("T4", format!("flush emitted {causes:?}, expected exactly one Untracked")));
    }
    Ok(())
}

fn t5<H: CommandHost>(host: &mut H) -> Result<(), ConformanceError> {
    let mut doc = 0u64;
    let mut derived = 0u64;
    let mut opaque = 0u64;
    for id in host.command_ids() {
        let Some(schema) = host.schema(&id) else {
            return Err(fail("T5", format!("schema is missing for {id}")));
        };
        let tag = schema.get("x-craft-source").and_then(Value::as_str);
        match tag {
            Some("doc") => doc += 1,
            Some("derived") => derived += 1,
            Some("opaque") => opaque += 1,
            Some(other) => return Err(fail("T5", format!("schema for {id} has x-craft-source {other}"))),
            None => return Err(fail("T5", format!("schema for {id} has no x-craft-source"))),
        }
    }
    println!("schema x-craft-source doc={doc} derived={derived} opaque={opaque}");
    Ok(())
}

fn t6<H: CommandHost>(host: &mut H) -> Result<(), ConformanceError> {
    let (owner, id) = host.prepare_unregister();
    if !host.resolves(&id) {
        return Err(fail("T6", format!("{id} did not resolve before unregister")));
    }
    if !host.menu_lists(&id) {
        return Err(fail("T6", format!("{id} was not in the menu before unregister")));
    }
    if !host.user_shortcut_kept(&id) {
        return Err(fail("T6", format!("{id} had no user shortcut before unregister")));
    }
    host.set_veto_all(true);
    let _ = host.take_before_ids();
    let class = host.execute(&id, &Value::Null);
    if class != OutcomeClass::Vetoed {
        return Err(fail("T6", format!("gate did not veto {id} before unregister (got {class:?})")));
    }
    let seen = host.take_before_ids();
    if !seen.iter().any(|got| got == &id) {
        return Err(fail("T6", format!("gate was not called for {id} before unregister")));
    }

    host.unregister_owner(&owner);
    if host.resolves(&id) {
        return Err(fail("T6", format!("{id} still resolves after unregister")));
    }
    if host.menu_lists(&id) {
        return Err(fail("T6", format!("{id} is still in the menu after unregister")));
    }
    if !host.user_shortcut_kept(&id) {
        return Err(fail("T6", format!("user shortcut for {id} was removed with the plugin")));
    }
    let _ = host.take_before_ids();
    let class = host.execute(&id, &Value::Null);
    if class == OutcomeClass::Vetoed {
        return Err(fail("T6", format!("gate still vetoed {id} after unregister")));
    }
    let seen = host.take_before_ids();
    if seen.iter().any(|got| got == &id) {
        return Err(fail("T6", format!("gate was called for {id} after unregister")));
    }
    host.set_veto_all(false);
    Ok(())
}
