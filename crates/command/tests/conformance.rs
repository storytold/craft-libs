#![cfg(feature = "testing")]

//! A host that stands in for an app. It is not FilmCraft. It records the
//! same facts the seam contract asks T1 through T6 to check.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use craft_command::conformance::{ChangeCause, CommandHost, OutcomeClass, run_all, run_with};
use craft_command::schema::from_doc;

struct Host {
    commands: Vec<String>,
    ui: Vec<String>,
    doc: Value,
    revision: u64,
    veto: bool,
    record_before: bool,
    before: Vec<String>,
    after: Vec<OutcomeClass>,
    schemas: BTreeMap<String, Value>,
    menu: Vec<String>,
    shortcuts: BTreeSet<String>,
    dirty: bool,
    bad_flush: bool,
}

impl Host {
    fn demo() -> Self {
        let specs = [
            ("edit.undo", "{}"),
            ("layer.select", "{layers: [id|name|#n], add?, toggle?}"),
            ("comp.note", r#"{"type":"object","properties":{"note":{"type":"string"}},"required":["note"]}"#),
            ("mystery.do", "free text, not a key list"),
            ("plugin.demo.flip", r#"{"name":str}"#),
        ];
        let mut schemas = BTreeMap::new();
        let mut commands = Vec::new();
        for (id, params) in specs {
            commands.push(id.to_string());
            schemas.insert(id.to_string(), from_doc(params).value);
        }
        let plugin = "plugin.demo.flip".to_string();
        Self {
            commands,
            ui: vec!["ui.transport.play".to_string(), "ui.viewer.zoom".to_string()],
            doc: json!({"n": 0}),
            revision: 3,
            veto: false,
            record_before: true,
            before: Vec::new(),
            after: Vec::new(),
            schemas,
            menu: vec![plugin.clone()],
            shortcuts: BTreeSet::from([plugin]),
            dirty: false,
            bad_flush: false,
        }
    }

    fn classify(id: &str) -> OutcomeClass {
        if id.contains("mystery") {
            OutcomeClass::Failure
        } else if id.contains("layer") {
            OutcomeClass::Disabled
        } else {
            OutcomeClass::Success
        }
    }
}

impl CommandHost for Host {
    fn fresh() -> Self {
        Self::demo()
    }

    fn command_ids(&self) -> Vec<String> {
        self.commands.clone()
    }

    fn ui_command_ids(&self) -> Vec<String> {
        self.ui.clone()
    }

    fn execute(&mut self, id: &str, _params: &Value) -> OutcomeClass {
        if !self.commands.iter().any(|got| got == id) {
            return OutcomeClass::Failure;
        }
        if self.record_before {
            self.before.push(id.to_string());
        }
        let class = if self.veto { OutcomeClass::Vetoed } else { Self::classify(id) };
        self.after.push(class);
        class
    }

    fn invoke_ui(&mut self, id: &str, _params: &Value) -> OutcomeClass {
        if self.record_before {
            self.before.push(id.to_string());
        }
        let class = if self.veto { OutcomeClass::Vetoed } else { OutcomeClass::Success };
        self.after.push(class);
        class
    }

    fn fingerprint(&self) -> u64 {
        let mut hash = 0xcbf29ce484222325u64;
        for byte in self.doc.to_string().bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn set_veto_all(&mut self, enabled: bool) {
        self.veto = enabled;
    }

    fn take_before_ids(&mut self) -> Vec<String> {
        std::mem::take(&mut self.before)
    }

    fn take_after(&mut self) -> Vec<OutcomeClass> {
        std::mem::take(&mut self.after)
    }

    fn schema(&self, id: &str) -> Option<Value> {
        self.schemas.get(id).cloned()
    }

    fn script_len(&self) -> usize {
        4
    }

    fn run_script_step(&mut self, index: usize) -> u64 {
        match index {
            0 => {
                self.doc["n"] = json!(self.revision.saturating_add(1));
                self.revision = self.revision.saturating_add(1);
                1
            }
            1 => 0,
            2 => {
                // Revision moves. The document bytes do not, so the fingerprint stays.
                self.revision = self.revision.saturating_add(1);
                1
            }
            3 => {
                self.doc["n"] = json!(7);
                self.revision = self.revision.saturating_add(1);
                self.doc["n"] = json!(8);
                self.revision = self.revision.saturating_add(1);
                2
            }
            _ => 0,
        }
    }

    fn write_without_bump(&mut self) {
        self.doc["untracked"] = json!(true);
        self.dirty = true;
    }

    fn flush(&mut self) -> Vec<ChangeCause> {
        if !self.dirty {
            return Vec::new();
        }
        self.dirty = false;
        if self.bad_flush { vec![ChangeCause::Untracked, ChangeCause::Other] } else { vec![ChangeCause::Untracked] }
    }

    fn prepare_unregister(&mut self) -> (String, String) {
        let id = "plugin.demo.flip".to_string();
        if !self.commands.iter().any(|got| got == &id) {
            self.commands.push(id.clone());
        }
        if !self.menu.iter().any(|got| got == &id) {
            self.menu.push(id.clone());
        }
        self.shortcuts.insert(id.clone());
        ("demo".to_string(), id)
    }

    fn unregister_owner(&mut self, owner: &str) {
        if owner != "demo" {
            return;
        }
        let id = "plugin.demo.flip";
        self.commands.retain(|got| got != id);
        self.menu.retain(|got| got != id);
    }

    fn resolves(&self, id: &str) -> bool {
        self.commands.iter().any(|got| got == id)
    }

    fn menu_lists(&self, id: &str) -> bool {
        self.menu.iter().any(|got| got == id)
    }

    fn user_shortcut_kept(&self, id: &str) -> bool {
        self.shortcuts.contains(id)
    }
}

#[test]
fn t1_through_t6_pass_on_a_host_that_wires_the_seam() {
    run_all::<Host>().expect("conformance");
}

#[test]
fn t1_fails_when_the_gate_does_not_see_the_call() {
    let mut host = Host::demo();
    host.record_before = false;
    let err = run_with(&mut host).expect_err("missed gate");
    assert_eq!(err.test, "T1");
    assert!(err.detail.contains("veto gate saw"), "{}", err.detail);
}

#[test]
fn t4_fails_when_flush_does_not_emit_exactly_one_untracked() {
    let mut host = Host::demo();
    host.bad_flush = true;
    let err = run_with(&mut host).expect_err("bad flush");
    assert_eq!(err.test, "T4");
    assert!(err.detail.contains("Untracked"), "{}", err.detail);
}
