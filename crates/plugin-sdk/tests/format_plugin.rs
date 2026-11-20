//! A `format` plugin built with `#[craft_plugin]`, driven through `craft_testing::invoke_with_blob`:
//! the adapter `craft_invoke` uses on wasm32. The format inverts RGB and keeps alpha.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_plugin_sdk::abi::format::{ImageDoc, parse_read_response, parse_write_response, read_request, write_request};
use craft_plugin_sdk::craft_plugin;
use craft_plugin_sdk::prelude::*;

#[craft_plugin(manifest = "tests/fixtures/format.toml")]
mod plugin {
    use craft_plugin_sdk::prelude::*;

    /// File: 4 bytes `INV1`, u16 width, u16 height (little endian), then RGBA with RGB inverted.
    fn invert(call: FormatCall) -> Result<FormatOutput, String> {
        match call.op {
            FormatOp::Read => {
                let (head, body) = call.data.split_at_checked(8).ok_or("file is shorter than its header")?;
                if &head[..4] != b"INV1" {
                    return Err("not an INV1 file".into());
                }
                let (w, h) = (u32::from(u16::from_le_bytes([head[4], head[5]])), u32::from(u16::from_le_bytes([head[6], head[7]])));
                let rgba = body.chunks(4).flat_map(|p| [255 - p[0], 255 - p[1], 255 - p[2], p[3]]).collect();
                Ok(FormatOutput::Image(craft_plugin_sdk::abi::format::ImageDoc::new(w, h, rgba).map_err(|e| e.to_string())?))
            }
            FormatOp::Write => {
                let doc = call.image()?;
                let mut out = b"INV1".to_vec();
                out.extend_from_slice(&(doc.width as u16).to_le_bytes());
                out.extend_from_slice(&(doc.height as u16).to_le_bytes());
                out.extend(doc.rgba8.chunks(4).flat_map(|p| [255 - p[0], 255 - p[1], 255 - p[2], p[3]]));
                Ok(FormatOutput::Bytes(out))
            }
        }
    }
}

use plugin::craft_testing as t;

fn file(w: u16, h: u16, rgba: &[u8]) -> Vec<u8> {
    let mut f = b"INV1".to_vec();
    f.extend_from_slice(&w.to_le_bytes());
    f.extend_from_slice(&h.to_le_bytes());
    f.extend_from_slice(rgba);
    f
}

#[test]
fn a_read_returns_the_decoded_image() {
    let res = t::invoke_with_blob("inv", read_request("a.inv", "inv"), &file(2, 1, &[0, 10, 255, 200, 1, 2, 3, 4])).unwrap().unwrap();
    let doc = parse_read_response(&res).unwrap();
    assert_eq!((doc.width, doc.height), (2, 1));
    assert_eq!(doc.rgba8, [255, 245, 0, 200, 254, 253, 252, 4]);
}

#[test]
fn a_write_encodes_the_pixels_and_a_read_gets_them_back() {
    let doc = ImageDoc::new(1, 2, vec![9, 8, 7, 6, 5, 4, 3, 2]).unwrap();
    let res = t::invoke_with_blob("inv", write_request("b.inv", "inv", 1, 2), &doc.rgba8).unwrap().unwrap();
    let bytes = parse_write_response(&res).unwrap();
    let back = t::invoke_with_blob("inv", read_request("b.inv", "inv"), &bytes).unwrap().unwrap();
    assert_eq!(parse_read_response(&back).unwrap(), doc);
}

#[test]
fn the_handlers_error_becomes_an_error_response_and_a_bad_op_is_a_bad_request() {
    let res = t::invoke_with_blob("inv", read_request("c.inv", "inv"), b"nope").unwrap().unwrap();
    assert_eq!(res["error"], "file is shorter than its header");
    let res = t::invoke_with_blob("inv", write_request("c.inv", "inv", 2, 2), &[0; 3]).unwrap().unwrap();
    assert!(res["error"].as_str().unwrap().contains("needs 16 bytes"), "{res}");
    let err = t::invoke_with_blob("inv", json!({"op": "delete"}), &[]).unwrap_err();
    assert!(err.contains("op"), "{err}");
}
