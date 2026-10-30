//! `format` contributions: the host keeps them in a [`FormatBook`] and an app opens or
//! saves a file with an unknown extension through it. The happy path runs `tests/fixtures/raw-rgb.wat`,
//! a raw RGB reader and writer. Error paths run the `echo` WAT fixture, whose entries trap, answer
//! `{"error"}`, answer text that is not JSON, or echo the request.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::echo_manifest;
use common::services::Harness;
use craft_plugin_abi::Permission;
use craft_plugin_abi::format::ImageDoc;
use craft_plugin_host::budget::BudgetConfig;
use craft_plugin_host::contrib::format::{FormatBook, FormatError};
use craft_plugin_host::engine::wasmi::WasmiEngine;
use craft_plugin_host::invoke::CallError;
use craft_plugin_host::lifecycle::{Decision, InstallOutcome};
use craft_plugin_host::store::{MemoryStore, Source};
use serde_json::{Value, json};

const RAW: &str = "org.example.raw-rgb";
const T: &str = "org.example.fmt";

/// The production slice size. The harness default (20 000 fuel) is too small for one `memory.grow`
/// of a guest that allocates megabytes, and a decoder does.
fn harness(step: Duration) -> Harness {
    let clock = common::StepClock::new(step);
    let engine = Arc::new(WasmiEngine::new(clock.clone()));
    let store = Arc::new(MemoryStore::new());
    Harness::build(engine, clock, BudgetConfig::default(), store.clone(), store)
}

fn rig() -> (Harness, Arc<FormatBook>) {
    let h = harness(Duration::ZERO);
    let book = FormatBook::new();
    h.manager.attach_surfaces(book.clone());
    (h, book)
}

/// The raw RGB plugin as a package. It asks for `fs.picked` and nothing else.
fn raw_package() -> Vec<u8> {
    let manifest = json!({
        "schema": 1, "id": RAW, "name": "Raw RGB image", "version": "1.0.0",
        "abi": { "craft": 1 },
        "permissions": { "required": ["fs.picked"] },
        "limits": { "memory_mb": 64 },
        "contributions": [{ "kind": "format", "id": "rgbraw", "entry": 0, "extensions": ["rgbraw"] }]
    });
    common::package_from(&common::fixture_text("raw-rgb"), &manifest)
}

fn install_raw(h: &Harness) {
    match h.manager.install(&raw_package(), Source::File).unwrap() {
        InstallOutcome::NeedsGrant(req) => {
            assert_eq!(req.required, vec![Permission::FsPicked], "the raw RGB plugin asks for fs.picked and nothing else");
            h.manager.grant(RAW, Decision { accept_required: true, optional: vec![] }).unwrap();
        }
        other => panic!("{other:?}"),
    }
}

/// The echo fixture as a format plugin. `fs.picked` is optional here so a test can decline it.
fn install_echo(h: &Harness, id: &str, contributions: Value, allow: bool) -> InstallOutcome {
    let mut m = echo_manifest(id, "1.0.0", contributions);
    m["permissions"]["optional"] = json!(["fs.picked"]);
    let bytes = common::package_from(&common::fixture_text("echo"), &m);
    let outcome = h.manager.install(&bytes, Source::File).unwrap();
    assert!(matches!(outcome, InstallOutcome::NeedsGrant(_)), "{outcome:?}");
    h.manager.grant(id, Decision { accept_required: true, optional: if allow { vec![Permission::FsPicked] } else { vec![] } }).unwrap()
}

fn fmt(id: &str, entry: u32, ext: &str) -> Value {
    json!({ "kind": "format", "id": id, "entry": entry, "extensions": [ext] })
}

fn rgb_file(w: u32, h: u32, rgb: &[u8]) -> Vec<u8> {
    let mut f = b"RGB8".to_vec();
    f.extend_from_slice(&w.to_le_bytes());
    f.extend_from_slice(&h.to_le_bytes());
    f.extend_from_slice(rgb);
    f
}

#[test]
fn a_file_with_an_unknown_extension_opens_through_the_plugin_and_saves_back() {
    let (h, book) = rig();
    assert!(!book.handles("rgbraw"), "nothing is hosted before the install");
    install_raw(&h);
    assert!(book.handles("rgbraw") && book.handles("RGBRAW"), "the lookup ignores case");
    assert_eq!(book.extensions(), vec!["rgbraw".to_owned()]);

    let doc = book.read("/home/me/pic.RGBRAW", &rgb_file(2, 1, &[1, 2, 3, 4, 5, 6])).unwrap();
    assert_eq!(doc, ImageDoc::new(2, 1, vec![1, 2, 3, 255, 4, 5, 6, 255]).unwrap());

    let saved = book.write("pic.rgbraw", &ImageDoc::new(1, 1, vec![9, 8, 7, 6]).unwrap()).unwrap();
    assert_eq!(saved, rgb_file(1, 1, &[9, 8, 7]), "the plugin drops alpha");
    // The call went through the guarded path: the app's undo scope opened and closed once per call.
    h.services.recorded(|r| assert_eq!(r.begun.len(), 2));
}

#[test]
fn errors_are_typed() {
    let (h, book) = rig();
    install_raw(&h);
    assert_eq!(book.read("noext", b"x").unwrap_err(), FormatError::NoExtension("noext".into()));
    assert_eq!(book.read("a.zzz", b"x").unwrap_err(), FormatError::NoPlugin { ext: "zzz".into() });
    assert_eq!(book.write("a.zzz", &ImageDoc::new(1, 1, vec![0; 4]).unwrap()).unwrap_err(), FormatError::NoPlugin { ext: "zzz".into() });
    // The plugin's own refusal is a Plugin error. It does not count toward the circuit.
    let err = book.read("a.rgbraw", b"not a file").unwrap_err();
    assert!(matches!(&err, FormatError::Call(CallError::Plugin(m)) if m.contains("12 byte header")), "{err:?}");
    for _ in 0..5 {
        assert!(book.read("a.rgbraw", b"nope").is_err());
    }
    assert!(book.read("a.rgbraw", &rgb_file(1, 1, &[1, 2, 3])).is_ok(), "five plugin errors do not open the circuit");
    assert!(err.to_string().contains("12 byte header"));
}

#[test]
fn a_format_without_fs_picked_is_left_out_and_says_why() {
    let (h, book) = rig();
    install_echo(&h, T, json!([fmt("f", 0, "zzz")]), false);
    assert!(!book.handles("zzz"));
    let info = h.manager.info(T).unwrap();
    assert_eq!(info.surfaces.skipped.len(), 1);
    assert_eq!(info.surfaces.skipped[0].reason, "missing permission fs.picked");
    assert_eq!(book.read("a.zzz", b"x").unwrap_err(), FormatError::NoPlugin { ext: "zzz".into() });
}

#[test]
fn revoking_fs_picked_removes_the_format() {
    let (h, book) = rig();
    install_echo(&h, T, json!([fmt("f", 0, "zzz")]), true);
    assert!(book.handles("zzz"));
    h.manager.revoke(T, &Permission::FsPicked).unwrap();
    assert!(!book.handles("zzz"), "a revoked permission leaves no format behind");
    h.manager.remove(T).unwrap();
    assert!(book.extensions().is_empty());
}

#[test]
fn a_disabled_or_removed_plugin_leaves_the_book() {
    let (h, book) = rig();
    install_raw(&h);
    h.manager.disable(RAW).unwrap();
    assert!(!book.handles("rgbraw"));
    h.manager.remove(RAW).unwrap();
    assert!(book.extensions().is_empty());
}

#[test]
fn what_the_guest_does_wrong_is_a_typed_error() {
    let (h, book) = rig();
    install_echo(&h, T, json!([fmt("echo", 0, "echo"), fmt("trap", 1, "trap"), fmt("err", 4, "err"), fmt("junk", 8, "junk")]), true);
    // The echo entry answers {"result": <request>}: valid JSON, but no `doc`.
    assert!(matches!(book.read("a.echo", b"x").unwrap_err(), FormatError::Doc(_)));
    assert!(matches!(book.write("a.echo", &ImageDoc::new(1, 1, vec![0; 4]).unwrap()).unwrap_err(), FormatError::Doc(_)));
    assert!(matches!(book.read("a.trap", b"x").unwrap_err(), FormatError::Call(CallError::Trap(_))));
    assert!(matches!(book.read("a.err", b"x").unwrap_err(), FormatError::Call(CallError::Plugin(m)) if m == "nope"));
    assert!(matches!(book.read("a.junk", b"x").unwrap_err(), FormatError::Call(CallError::BadResponse(_))));
}

#[test]
fn two_plugins_for_one_extension_resolve_the_same_way_every_time() {
    let (h, book) = rig();
    install_echo(&h, "org.example.b", json!([fmt("f", 4, "dup")]), true);
    install_echo(&h, "org.example.a", json!([fmt("f", 1, "dup")]), true);
    // "org.example.a" sorts first, so its entry (trap) answers, whatever the install order was.
    for _ in 0..2 {
        assert!(matches!(book.read("x.dup", b"x").unwrap_err(), FormatError::Call(CallError::Trap(_))));
        assert_eq!(book.owners("dup"), vec!["org.example.a".to_owned(), "org.example.b".to_owned()]);
    }
}

#[test]
fn a_reload_swaps_the_generation_without_a_second_registration() {
    let (h, book) = rig();
    install_raw(&h);
    let before = book.read("a.rgbraw", &rgb_file(1, 1, &[1, 2, 3])).unwrap();
    install_raw_again(&h);
    assert_eq!(book.extensions().len(), 1, "one entry after the reload");
    assert_eq!(book.read("a.rgbraw", &rgb_file(1, 1, &[1, 2, 3])).unwrap(), before);
}

fn install_raw_again(h: &Harness) {
    let out = h.manager.install(&raw_package(), Source::File).unwrap();
    assert!(matches!(out, InstallOutcome::Loaded { replaced: Some(_), .. }), "{out:?}");
}

#[test]
fn a_megapixel_image_round_trips_through_the_guest() {
    let (h, book) = rig();
    install_raw(&h);
    let (w, hgt) = (1024u32, 256u32);
    let rgba: Vec<u8> = (0..w * hgt * 4).map(|i| if i % 4 == 3 { 255 } else { (i * 31 % 251) as u8 }).collect();
    let doc = ImageDoc::new(w, hgt, rgba).unwrap();
    let file = book.write("big.rgbraw", &doc).unwrap();
    assert_eq!(file.len(), 12 + (w * hgt * 3) as usize);
    assert_eq!(book.read("big.rgbraw", &file).unwrap(), doc);
}

#[test]
#[ignore = "about 3 minutes in a debug build: run with --ignored"]
fn an_answer_over_the_response_cap_is_a_bad_response_not_a_crash() {
    let (h, book) = rig();
    install_raw(&h);
    // 2048 x 2048 RGBA is 16 MiB of pixels, 22 MiB as base64: over MAX_RESPONSE_BYTES.
    let file = rgb_file(2048, 2048, &vec![7u8; 2048 * 2048 * 3]);
    let err = book.read("huge.rgbraw", &file).unwrap_err();
    assert!(matches!(err, FormatError::Call(CallError::BadResponse(_) | CallError::Trap(_))), "{err:?}");
    assert!(book.read("a.rgbraw", &rgb_file(1, 1, &[1, 2, 3])).is_ok(), "the plugin still works after");
}

#[test]
fn a_plugin_that_never_returns_is_stopped_at_the_books_deadline() {
    let h = harness(Duration::from_millis(1));
    let book = FormatBook::with_deadline(Duration::from_millis(50));
    h.manager.attach_surfaces(book.clone());
    install_echo(&h, T, json!([fmt("spin", 2, "spin")]), true);
    let err = book.read("a.spin", b"x").unwrap_err();
    assert!(matches!(err, FormatError::Call(CallError::Trap(craft_plugin_host::engine::Trap::Deadline))), "{err:?}");
}
