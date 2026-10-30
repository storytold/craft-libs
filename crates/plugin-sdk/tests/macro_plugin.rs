//! A plugin built with `#[craft_plugin]` from a fixture manifest, driven through the generated
//! `craft_testing` module: the same adapters and dispatch tables `craft_invoke` and
//! `craft_process` use on wasm32.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_plugin_sdk::abi::ProcessDescriptor;
use craft_plugin_sdk::abi::descriptor::{Alpha, Kind, Sample};
use craft_plugin_sdk::abi::manifest::Manifest;
use craft_plugin_sdk::prelude::*;
use craft_plugin_sdk::{craft_plugin, testing};

#[craft_plugin(manifest = "tests/fixtures/plugin.toml")]
mod plugin {
    use craft_plugin_sdk::prelude::*;

    fn echo(call: CommandCall) -> Result<Value, String> {
        Ok(json!({ "params": call.params, "origin": call.origin }))
    }

    fn fail(_call: CommandCall) -> Result<Value, String> {
        Err("it broke".to_owned())
    }

    fn gate(hook: HookCall) -> HookVerdict {
        match hook.call.as_ref().map(|c| c.id.as_str()) {
            Some("clip.delete") => HookVerdict::Veto("no deleting".to_owned()),
            Some("clip.move") => HookVerdict::Rewrite(json!({ "x": 0 })),
            _ => HookVerdict::Pass,
        }
    }

    fn watcher(hook: HookCall) -> HookVerdict {
        host::info(&format!("outcome {:?}", hook.outcome));
        HookVerdict::Pass
    }

    fn side(event: PanelEvent) -> Option<Widget> {
        (event.event != "close").then(|| ui::column("root").child(ui::label("e", &event.event)))
    }

    fn halve(band: &mut Band) -> Result<(), String> {
        let amount = band.param_f64("amount", 0.5);
        for (src, dst) in band.input.iter().zip(band.output.iter_mut()) {
            *dst = (f64::from(*src) * amount) as u8;
        }
        Ok(())
    }
}

use plugin::craft_testing as t;

fn desc(entry: u32, w: u32, h: u32) -> ProcessDescriptor {
    ProcessDescriptor {
        kind: Kind::Image,
        entry,
        w,
        h,
        stride_bytes: w,
        channels: 1,
        sample: Sample::U8,
        alpha: Alpha::None,
        color_mode: 0,
        depth: 8,
        origin_x: 0,
        origin_y: 0,
        overlap: 0,
        scale: 1.0,
        time: 0.0,
        sample_rate: 0,
        frames: 0,
        in_ptr: 0,
        out_ptr: 0,
        params_ptr: 0,
        params_len: 0,
    }
}

#[test]
fn manifest_entries_are_the_contribution_indexes_and_handlers_are_stripped() {
    let m = Manifest::parse(t::MANIFEST_JSON.as_bytes()).expect("the embedded manifest passes the gate's own parser");
    assert_eq!(m.id, "org.example.fixture");
    assert_eq!(m.contributions.len(), 8);
    for (i, c) in m.contributions.iter().enumerate() {
        assert_eq!(c.entry as usize, i, "{}", c.id);
        assert_eq!(c.id, t::CONTRIBUTIONS[i].0);
        assert_eq!(c.kind.name(), t::CONTRIBUTIONS[i].1);
    }
    assert!(!t::MANIFEST_JSON.contains("handler"), "handler names are build-time only");
    assert_eq!(m.unsupported_contributions().count(), 1, "the gizmo kind stays in the manifest");
}

#[test]
fn command_returns_result_and_passes_origin() {
    let out = t::invoke("echo", json!({ "params": { "a": 1 }, "origin": "script" })).unwrap();
    assert_eq!(out, Some(json!({ "result": { "params": { "a": 1 }, "origin": "script" } })));
    let out = t::invoke("echo", json!({ "params": null })).unwrap();
    assert_eq!(out, Some(json!({ "result": { "params": null, "origin": "unknown" } })), "a missing origin reads as unknown");
}

#[test]
fn command_error_becomes_an_error_response_not_a_status() {
    assert_eq!(t::invoke("fail", json!({ "params": {} })).unwrap(), Some(json!({ "error": "it broke" })));
}

#[test]
fn menu_contributions_run_the_command_shape() {
    let out = t::invoke("menu-item", json!({ "params": 7 })).unwrap().unwrap();
    assert_eq!(out["result"]["params"], json!(7));
}

#[test]
fn before_hook_answers_pass_rewrite_and_veto_in_wire_shapes() {
    let call = |id: &str| json!({ "call": { "id": id, "params": {}, "depth": 0, "origin": "ui", "seq": 4 } });
    assert_eq!(t::invoke("gate", call("clip.add")).unwrap(), Some(json!({})));
    assert_eq!(t::invoke("gate", call("clip.move")).unwrap(), Some(json!({ "rewrite": { "x": 0 } })));
    assert_eq!(t::invoke("gate", call("clip.delete")).unwrap(), Some(json!({ "veto": "no deleting" })));
}

#[test]
fn after_hook_sees_the_outcome() {
    let logged = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    let sink = logged.clone();
    let host = move |op: u32, input: &[u8], out: &mut [u8]| {
        assert_eq!(op, 0x0001, "the hook logs through host.log");
        *sink.borrow_mut() = serde_json::from_slice::<Value>(input).unwrap()["msg"].as_str().unwrap().to_owned();
        testing::reply(out, &json!({}))
    };
    testing::with_host(host, || t::invoke("watcher", json!({ "call": { "id": "x" }, "outcome": "vetoed" })).unwrap());
    assert_eq!(*logged.borrow(), "outcome Some(\"vetoed\")");
}

#[test]
fn panel_returns_a_tree_or_an_empty_object() {
    let open = t::invoke("side", json!({ "event": "open" })).unwrap().unwrap();
    assert_eq!(open["tree"]["type"], "column");
    assert_eq!(open["tree"]["children"][0]["text"], "open");
    assert_eq!(t::invoke("side", json!({ "event": "close" })).unwrap(), Some(json!({})));
    assert!(t::invoke("side", json!({ "widget": "w" })).is_err(), "a panel request with no event is malformed");
}

#[test]
fn effect_runs_on_a_band_with_its_params() {
    let out = t::process("halve", &desc(0, 4, 1), &[200, 100, 50, 10], json!({ "amount": 0.5 })).unwrap();
    assert_eq!(out, vec![100, 50, 25, 5]);
    let out = t::process("halve", &desc(0, 2, 1), &[200, 100], json!({})).unwrap();
    assert_eq!(out, vec![100, 50], "the default amount applies when params omit it");
}

#[test]
fn the_wrong_plane_and_unknown_ids_are_errors() {
    assert!(t::invoke("halve", json!({})).is_err(), "an effect is not a craft_invoke entry");
    assert!(t::process("echo", &desc(0, 1, 1), &[0], json!({})).is_err(), "a command is not a craft_process entry");
    assert!(t::invoke("from-the-future", json!({})).is_err(), "an unknown kind has no handler");
    assert!(t::invoke("nope", json!({})).is_err());
    assert!(t::process("halve", &desc(0, 4, 1), &[1, 2, 3], json!({})).is_err(), "input shorter than stride * h");
}
