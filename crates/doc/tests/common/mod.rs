//! Synthetic documents built from scratch for the tests: hand-written JSON, a small literal PDF,
//! PNG files encoded here, ZIP archives and gzip streams assembled here.
#![allow(dead_code)]

use std::io::Write;

/// A one-page PDF with an empty page.
pub const PDF: &[u8] = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 72] >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n";

pub const BLOB_MAGIC: &[u8] = b"\n\0VCBLOBS\0";

pub fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(bytes).unwrap();
    gz.finish().unwrap()
}

pub fn deflate(bytes: &[u8]) -> Vec<u8> {
    let mut d = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    d.write_all(bytes).unwrap();
    d.finish().unwrap()
}

pub fn crc32(bytes: &[u8]) -> u32 {
    let mut c = flate2::Crc::new();
    c.update(bytes);
    c.sum()
}

pub fn base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        out.push(A[(n >> 18) as usize & 63] as char);
        out.push(A[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { A[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { A[n as usize & 63] as char } else { '=' });
    }
    out
}

/// A gray `w` × `h` PNG with every pixel `value`.
pub fn png(w: u32, h: u32, value: u8) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend((data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend(data);
        out.extend(&body);
        out.extend(crc32(&body).to_be_bytes());
    }
    let mut ihdr = w.to_be_bytes().to_vec();
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 0, 0, 0, 0]);
    let mut raw = vec![];
    for _ in 0..h {
        raw.push(0);
        raw.extend(std::iter::repeat_n(value, w as usize));
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&raw).unwrap();
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z.finish().unwrap());
    chunk(&mut out, b"IEND", &[]);
    out
}

/// One ZIP entry: its name, its bytes, and whether it is DEFLATE-compressed.
pub struct Entry<'a> {
    pub name: &'a str,
    pub data: &'a [u8],
    pub deflate: bool,
}

pub fn store<'a>(name: &'a str, data: &'a [u8]) -> Entry<'a> {
    Entry { name, data, deflate: false }
}

pub fn deflated<'a>(name: &'a str, data: &'a [u8]) -> Entry<'a> {
    Entry { name, data, deflate: true }
}

/// A ZIP archive of `entries`, in order, with a central directory and an end record.
pub fn zip(entries: &[Entry]) -> Vec<u8> {
    let mut out = vec![];
    let mut central = vec![];
    for e in entries {
        let payload = if e.deflate { deflate(e.data) } else { e.data.to_vec() };
        let crc = crc32(e.data);
        let method: u16 = if e.deflate { 8 } else { 0 };
        let offset = out.len() as u32;
        let fields = |out: &mut Vec<u8>| {
            out.extend(20u16.to_le_bytes());
            out.extend(0x0800u16.to_le_bytes());
            out.extend(method.to_le_bytes());
            out.extend(0u16.to_le_bytes());
            out.extend(0x21u16.to_le_bytes());
            out.extend(crc.to_le_bytes());
            out.extend((payload.len() as u32).to_le_bytes());
            out.extend((e.data.len() as u32).to_le_bytes());
            out.extend((e.name.len() as u16).to_le_bytes());
            out.extend(0u16.to_le_bytes());
        };
        out.extend(0x0403_4b50u32.to_le_bytes());
        fields(&mut out);
        out.extend(e.name.as_bytes());
        out.extend(&payload);
        central.extend(0x0201_4b50u32.to_le_bytes());
        central.extend(20u16.to_le_bytes());
        fields(&mut central);
        // Comment length, disk, internal and external attributes.
        central.extend([0u8; 10]);
        central.extend(offset.to_le_bytes());
        central.extend(e.name.as_bytes());
    }
    let dir_offset = out.len() as u32;
    out.extend(&central);
    out.extend(0x0605_4b50u32.to_le_bytes());
    out.extend([0u8; 4]);
    out.extend((entries.len() as u16).to_le_bytes());
    out.extend((entries.len() as u16).to_le_bytes());
    out.extend((central.len() as u32).to_le_bytes());
    out.extend(dir_offset.to_le_bytes());
    out.extend(0u16.to_le_bytes());
    out
}

/// A VectorCraft document member holding `bytes` as base64.
pub fn inline(mime: &str, bytes: &[u8]) -> String {
    format!(r#"{{"mime":"{mime}","data":"{}"}}"#, base64(bytes))
}

/// A VectorCraft document member pointing at the data after the JSON.
pub fn blob(mime: &str, offset: usize, len: usize) -> String {
    format!(r#"{{"mime":"{mime}","blob":[{offset},{len}]}}"#)
}

/// The JSON of a VectorCraft file: `members` go between `generator` and `document`.
pub fn vectorcraft_json(version: u32, members: &str) -> String {
    let document = r#"{"version":1,"title":"","units":"Points","color_mode":"Rgb",
        "artboards":[{"id":1,"name":"Front","rect":{"x0":0.0,"y0":0.0,"x1":612.0,"y1":792.0},"show_center_mark":false,"show_cross_hairs":false},
                     {"id":2,"name":"Back","rect":{"x0":700.0,"y0":800.0,"x1":650.0,"y1":700.0},"show_center_mark":false,"show_cross_hairs":false}],
        "layers":[{"id":1,"name":"Art","kind":{"type":"layer","color":{"Preset":0},"template":false,"printable":true,
                     "children":[{"id":3,"kind":{"type":"group","children":[{"id":4,"kind":{"type":"path","path":{"subpaths":[]}}}]}}]}},
                  {"id":2,"name":"Notes","visible":false,"opacity":0.5,"blend":"Multiply","kind":{"type":"layer","color":{"Preset":1},"template":false,"printable":false,"children":[]}}],
        "next_id":5}"#;
    let sep = if members.is_empty() { "" } else { "," };
    format!(r#"{{"format":"vectorcraft","version":{version},"generator":"VectorCraft 0.1.0"{sep}{members},"document":{document},"images":{{}}}}"#)
}

/// A PhotoCraft layer as the manifest writes it.
pub fn pcraft_layer(name: &str, visible: bool, blend: &str, opacity: f32, content: &str) -> String {
    format!(
        r#"{{"id":1,"name":"{name}","visible":{visible},"locks":{{}},"blend":"{blend}","opacity":{opacity},"fill_opacity":1.0,"clipped":false,"mask":null,
            "effects":{{"enabled":true,"items":[],"psd_raw":null}},"label":"None","content":{content},"psd_blocks":[],"psd_id":null,"fill_cache":null}}"#
    )
}

pub fn raster() -> &'static str {
    r#"{"kind":"raster","surface":{"format":{"mode":"Rgb","sample":"U8","alpha":true},"default":"00000000","tiles":[]}}"#
}

/// A PhotoCraft manifest with `layers` (a JSON array) and the preview paths given.
pub fn pcraft_manifest(version: u32, layers: &str, previews: &str) -> String {
    format!(
        r#"{{"format_version":{version},"generator":"photocraft-format 0.1.0",
            "document":{{"id":1,"name":"Untitled","size":{{"width":640,"height":480}},"resolution_dpi":300.0,"mode":"Rgb","depth":"U16",
              "icc_profile":null,"layers":{layers},"channels":[],"guides":{{}},"selection":null,"metadata":{{}},"global_light":{{}}}}{previews}}}"#
    )
}

/// A `.pcraft` bundle: a tile, the previews, and the manifest last.
pub fn pcraft(thumb: &[u8], composite: &[u8]) -> Vec<u8> {
    let layers = format!(
        "[{},{}]",
        pcraft_layer("Background", true, "Normal", 1.0, raster()),
        pcraft_layer(
            "Group",
            false,
            "Screen",
            0.25,
            &format!(r#"{{"kind":"group","expanded":true,"children":[{}]}}"#, pcraft_layer("Inner", true, "Multiply", 0.5, raster()))
        )
    );
    let manifest = pcraft_manifest(1, &layers, r#","thumbnail":"thumb.png","composite":"composite/preview.png""#);
    zip(&[
        store("tiles/0123.zst", b"not really zstd"),
        store("thumb.png", thumb),
        deflated("composite/preview.png", composite),
        deflated("manifest.json", manifest.as_bytes()),
    ])
}

/// An EffectCraft project with a folder, a footage item and two compositions.
pub fn ecproj(schema: u32) -> String {
    format!(
        r#"{{
  "savedBy": "0.12.0",
  "activeComp": 10,
  "schema": {schema},
  "settings": {{}},
  "items": {{
    "1": {{"id": 1, "name": "Footage", "label": 0, "kind": {{"type": "Folder"}}}},
    "2": {{"id": 2, "name": "clip.mov", "label": 0, "parent": 1,
          "kind": {{"type": "Footage", "path": "clip.mov", "kind": "Video", "width": 640, "height": 360, "pixel_aspect": 1.0,
                   "frame_rate": {{"num": 30, "den": 1}}, "duration": 2540160000000}}}},
    "10": {{"id": 10, "name": "Main", "label": 0,
           "kind": {{"type": "Comp", "width": 1920, "height": 1080, "pixel_aspect": 1.0, "frame_rate": {{"num": 24000, "den": 1001}},
                    "duration": 1270080000000, "display_start": 0, "background": [0.0, 0.0, 0.0], "work_area": [0, 1270080000000],
                    "layers": [
                      {{"id": 1, "name": "Title", "source": {{"type": "Text"}}, "switches": {{"video": true, "audio": false}}, "blend_mode": "Add"}},
                      {{"id": 2, "name": "Plate", "source": {{"type": "Footage", "item": 2}}, "switches": {{"video": false, "audio": true}}, "blend_mode": "Normal"}}
                    ]}}}},
    "3": {{"id": 3, "name": "Intro", "label": 0,
          "kind": {{"type": "Comp", "width": 640, "height": 360, "pixel_aspect": 1.0, "frame_rate": {{"num": 30, "den": 1}},
                   "duration": 254016000000, "layers": []}}}}
  }},
  "next_id": 11
}}"#
    )
}
