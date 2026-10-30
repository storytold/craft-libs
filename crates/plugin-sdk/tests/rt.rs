//! The safe core of the exports: status codes for bad entries and bad JSON, and band checks.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_plugin_abi::ProcessDescriptor;
use craft_plugin_abi::descriptor::{Alpha, Kind, Sample};
use craft_plugin_abi::ops::pack_invoke_return;
use craft_plugin_sdk::rt::{self, process_status};
use craft_plugin_sdk::{Band, ControlFn, DataFn, Value, json, testing};

fn echo_json(req: &Value, blob: &[u8]) -> Result<Option<Value>, String> {
    Ok(Some(json!({ "req": req, "blob": blob.len() })))
}

fn nothing(_: &Value, _: &[u8]) -> Result<Option<Value>, String> {
    Ok(None)
}

fn malformed(_: &Value, _: &[u8]) -> Result<Option<Value>, String> {
    Err("not an object".into())
}

const CONTROL: [Option<ControlFn>; 4] = [Some(echo_json), Some(nothing), Some(malformed), None];

fn quiet_host<R>(f: impl FnOnce() -> R) -> R {
    testing::with_host(|_, _, out| testing::reply(out, &json!({})), f)
}

#[test]
fn invoke_bytes_serializes_the_response() {
    let out = rt::invoke_bytes(&CONTROL, 0, br#"{"a":1}"#, b"xyz").unwrap().unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&out).unwrap(), json!({ "req": { "a": 1 }, "blob": 3 }));
}

#[test]
fn an_empty_response_is_none() {
    assert_eq!(rt::invoke_bytes(&CONTROL, 1, b"{}", b""), Ok(None));
}

#[test]
fn an_empty_request_reads_as_null() {
    let out = rt::invoke_bytes(&CONTROL, 0, b"", b"").unwrap().unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&out).unwrap()["req"], Value::Null);
}

#[test]
fn bad_entry_and_bad_json_give_the_abi_statuses() {
    quiet_host(|| {
        assert_eq!(rt::invoke_bytes(&CONTROL, 3, b"{}", b""), Err(-3), "a slot with no handler is E_UNSUPPORTED");
        assert_eq!(rt::invoke_bytes(&CONTROL, 9, b"{}", b""), Err(-3), "an entry past the table");
        assert_eq!(rt::invoke_bytes(&CONTROL, -1, b"{}", b""), Err(-3), "a negative entry");
        assert_eq!(rt::invoke_bytes(&CONTROL, 0, b"{not json", b""), Err(-4), "unparseable request is E_BAD_ARG");
        assert_eq!(rt::invoke_bytes(&CONTROL, 2, b"{}", b""), Err(-4), "an adapter error is E_BAD_ARG");
    });
}

#[test]
fn a_bad_entry_tells_the_host_why() {
    let logged = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let sink = logged.clone();
    testing::with_host(
        move |op, input, out| {
            assert_eq!(op, 0x0001);
            sink.borrow_mut().push(serde_json::from_slice::<Value>(input).unwrap()["msg"].as_str().unwrap().to_owned());
            testing::reply(out, &json!({}))
        },
        || {
            let _ = rt::invoke_bytes(&CONTROL, 9, b"{}", b"");
        },
    );
    assert_eq!(logged.borrow().len(), 1);
    assert!(logged.borrow()[0].contains("entry 9"));
}

#[test]
fn pack_limits_match_the_abi() {
    // len must fit in 31 bits, because bit 31 reads back as a negative status.
    assert!(pack_invoke_return(8, 12).is_some());
    assert!(pack_invoke_return(8, 0x8000_0000).is_none());
}

fn desc(kind: Kind) -> ProcessDescriptor {
    ProcessDescriptor {
        kind,
        entry: 0,
        w: 2,
        h: 2,
        stride_bytes: 3,
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
        sample_rate: 48_000,
        frames: 4,
        in_ptr: 0,
        out_ptr: 0,
        params_ptr: 0,
        params_len: 0,
    }
}

fn copy_rows(band: &mut Band) -> Result<(), String> {
    for (src, dst) in band.rows() {
        dst.copy_from_slice(src);
    }
    Ok(())
}

fn refuse(_: &mut Band) -> Result<(), String> {
    Err("no".into())
}

const DATA: [Option<DataFn>; 3] = [Some(copy_rows), Some(refuse), None];

#[test]
fn band_len_is_stride_times_height_for_images_and_planar_f32_for_audio() {
    assert_eq!(rt::band_len(&desc(Kind::Image)), Some(6));
    let mut a = desc(Kind::Audio);
    a.channels = 2;
    a.frames = 5;
    assert_eq!(rt::band_len(&a), Some(40));
    let mut huge = desc(Kind::Image);
    huge.stride_bytes = u32::MAX;
    huge.h = u32::MAX;
    if usize::BITS == 32 {
        assert_eq!(rt::band_len(&huge), None);
    }
}

#[test]
fn process_slices_runs_the_entry_and_maps_results_to_codes() {
    quiet_host(|| {
        let input = [1u8, 2, 3, 4, 5, 6];
        let mut output = [0u8; 6];
        assert_eq!(rt::process_slices(&DATA, &desc(Kind::Image), &input, &mut output, b""), process_status::OK);
        assert_eq!(output, input, "rows are stride bytes, so all six bytes pass through");

        let mut d = desc(Kind::Image);
        d.entry = 1;
        assert_eq!(rt::process_slices(&DATA, &d, &input, &mut output, b""), process_status::HANDLER_ERROR);
        d.entry = 2;
        assert_eq!(rt::process_slices(&DATA, &d, &input, &mut output, b""), process_status::NO_SUCH_ENTRY);
        d.entry = 77;
        assert_eq!(rt::process_slices(&DATA, &d, &input, &mut output, b""), process_status::NO_SUCH_ENTRY);
        d.entry = 0;
        assert_eq!(rt::process_slices(&DATA, &d, &input, &mut output, b"{bad"), process_status::BAD_DESCRIPTOR);
    });
}

#[test]
fn process_vec_checks_the_input_length() {
    assert_eq!(rt::process_vec(&DATA, 0, &desc(Kind::Image), &[0; 6], json!({})).unwrap(), vec![0; 6]);
    assert!(rt::process_vec(&DATA, 0, &desc(Kind::Image), &[0; 5], json!({})).is_err());
}
