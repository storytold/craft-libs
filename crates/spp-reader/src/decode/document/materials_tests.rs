//! Tests of [`super::materials`] on synthetic documents.

use super::materials::*;
use super::test_writer::DocWriter;

fn string(w: &mut DocWriter, name: &str, value: &str) {
    w.field(name).raw(&[0x10]).raw(&(value.len() as u32).to_le_bytes()).raw(value.as_bytes());
}

fn enum8(w: &mut DocWriter, name: &str, tag: u8, value: u64) {
    w.field(name).raw(&[tag]).raw(&value.to_le_bytes());
}

/// A document with one material: name, uid, resolution, one baked map, one channel and two layers.
fn document() -> Vec<u8> {
    let mut w = DocWriter::new(78);
    w.object(0x12, "DataDocument", 1);
    w.field("materials").list(0x13, 1);
    w.object(0x12, "DataMaterial", 6);
    w.field("additionalMaps").list(0x11, 1);
    w.object(0x14, "DataAdditionalMap", 2);
    string(&mut w, "urlToBitmapRes", "/AO?version=ab");
    enum8(&mut w, "usage", 0x09, 1);
    w.close().close();
    w.field("resolutionLog2").raw(&[0x06, 11, 0, 0, 0, 10, 0, 0, 0]);
    string(&mut w, "sceneMaterialName", "Body");
    enum8(&mut w, "sceneMaterialUID", 0x0c, 42);
    w.field("normalMapBitmap").object(0x12, "DataBitmap", 1);
    string(&mut w, "urlToBitmapRes", "/N?version=cd");
    w.close();
    w.field("stacks").list(0x13, 1);
    w.object(0x12, "DataMaterialStack", 2);
    w.field("channels").list(0x13, 1);
    w.object(0x12, "DataChannel", 2);
    enum8(&mut w, "format", 0x09, 5);
    enum8(&mut w, "type", 0x15, 2);
    w.close().close();
    w.field("stack").object(0x14, "DataStackLayers", 1);
    w.field("items").list(0x13, 2);
    w.object(0x12, "DataLayerColor", 0).close();
    w.object(0x12, "DataLayerGroup", 1);
    w.field("x").object(0x14, "DataActionFill", 0).close();
    w.close();
    w.close().close().close().close();
    w.close().close().close();
    w.bytes
}

#[test]
fn reads_material_records() {
    let records = DocumentMaterials::scan(&document()).unwrap();
    assert_eq!(records.len(), 1);
    let m = &records[0];
    assert_eq!((m.name.as_str(), m.scene_uid, m.resolution_log2), ("Body", Some(42), Some([11, 10])));
    assert_eq!(m.channels, [ChannelRecord { type_bit: 2, format: 5, user_name: String::new() }]);
    assert_eq!(m.baked.len(), 2);
    assert_eq!((m.baked[0].url.as_str(), m.baked[0].usage), ("/AO?version=ab", Some(1)));
    assert_eq!((m.baked[1].url.as_str(), m.baked[1].usage), ("/N?version=cd", None));
    assert_eq!((m.layer_count, m.action_count), (2, 1));
}

#[test]
fn truncated_documents_fail_without_panicking() {
    let data = document();
    for len in 0..data.len() {
        assert!(DocumentMaterials::scan(&data[..len]).is_err(), "length {len}");
    }
}
