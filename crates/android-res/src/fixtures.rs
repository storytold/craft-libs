//! Test-only XML and resource table fixtures ported from Craft Anything.

pub const PACKAGE: &str = "com.example.notes";

fn u16le(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn u32le(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn align4(v: &mut Vec<u8>) {
    while !v.len().is_multiple_of(4) {
        v.push(0);
    }
}

/// A string pool chunk (UTF-16, or UTF-8 when `utf8`).
pub fn string_pool(strings: &[&str], utf8: bool) -> Vec<u8> {
    let mut data = Vec::new();
    let mut offs = Vec::new();
    for s in strings {
        offs.push(data.len() as u32);
        if utf8 {
            let n16 = s.encode_utf16().count();
            data.push(n16 as u8);
            data.push(s.len() as u8);
            data.extend_from_slice(s.as_bytes());
            data.push(0);
        } else {
            let units: Vec<u16> = s.encode_utf16().collect();
            u16le(&mut data, units.len() as u16);
            for u in units {
                u16le(&mut data, u);
            }
            u16le(&mut data, 0);
        }
    }
    align4(&mut data);
    let strings_start = 28 + 4 * strings.len();
    let mut v = Vec::new();
    u16le(&mut v, 0x0001);
    u16le(&mut v, 28);
    u32le(&mut v, (strings_start + data.len()) as u32);
    u32le(&mut v, strings.len() as u32);
    u32le(&mut v, 0);
    u32le(&mut v, if utf8 { 1 << 8 } else { 0 });
    u32le(&mut v, strings_start as u32);
    u32le(&mut v, 0);
    for o in offs {
        u32le(&mut v, o);
    }
    v.extend_from_slice(&data);
    v
}

/// An attribute value in a binary XML element.
#[derive(Debug, Clone, Copy)]
pub enum AxVal<'a> {
    Str(&'a str),
    Bool(bool),
    Int(i32),
    Hex(u32),
    Ref(u32),
}

#[derive(Debug, Clone)]
pub struct AxAttr<'a> {
    pub name: &'a str,
    /// Android framework attribute (namespaced, with its resource id).
    pub res_id: Option<u32>,
    pub val: AxVal<'a>,
}

#[derive(Debug, Clone)]
pub enum AxNode<'a> {
    Start(&'a str, Vec<AxAttr<'a>>),
    End(&'a str),
}

const NS: &str = "http://schemas.android.com/apk/res/android";

/// Shorthand for an `android:` attribute.
pub fn a<'a>(name: &'a str, res_id: u32, val: AxVal<'a>) -> AxAttr<'a> {
    AxAttr { name, res_id: Some(res_id), val }
}

/// Write a binary XML document with the android namespace declared on the root.
pub fn axml(nodes: &[AxNode<'_>]) -> Vec<u8> {
    let mut strings: Vec<String> = Vec::new();
    let mut ids: Vec<u32> = Vec::new();
    for n in nodes {
        if let AxNode::Start(_, attrs) = n {
            for at in attrs {
                if let Some(id) = at.res_id
                    && !strings.iter().any(|s| s == at.name)
                {
                    strings.push(at.name.to_string());
                    ids.push(id);
                }
            }
        }
    }
    let intern = |s: &str, strings: &mut Vec<String>| -> u32 {
        if let Some(i) = strings.iter().position(|x| x == s) {
            return i as u32;
        }
        strings.push(s.to_string());
        (strings.len() - 1) as u32
    };
    let prefix = intern("android", &mut strings);
    let uri = intern(NS, &mut strings);
    let mut body = Vec::new();
    let ns_chunk = |t: u16| {
        let mut c = Vec::new();
        u16le(&mut c, t);
        u16le(&mut c, 16);
        u32le(&mut c, 24);
        u32le(&mut c, 1);
        u32le(&mut c, u32::MAX);
        u32le(&mut c, prefix);
        u32le(&mut c, uri);
        c
    };
    body.extend(ns_chunk(0x0100));
    for (line, n) in nodes.iter().enumerate() {
        match n {
            AxNode::Start(name, attrs) => {
                let nidx = intern(name, &mut strings);
                let mut c = Vec::new();
                u16le(&mut c, 0x0102);
                u16le(&mut c, 16);
                u32le(&mut c, (36 + 20 * attrs.len()) as u32);
                u32le(&mut c, line as u32 + 2);
                u32le(&mut c, u32::MAX);
                u32le(&mut c, u32::MAX);
                u32le(&mut c, nidx);
                u16le(&mut c, 20);
                u16le(&mut c, 20);
                u16le(&mut c, attrs.len() as u16);
                u16le(&mut c, 0);
                u16le(&mut c, 0);
                u16le(&mut c, 0);
                for at in attrs {
                    let an = intern(at.name, &mut strings);
                    let (raw, ty, data) = match at.val {
                        AxVal::Str(s) => {
                            let i = intern(s, &mut strings);
                            (i, 0x03u8, i)
                        }
                        AxVal::Bool(b) => (u32::MAX, 0x12, if b { u32::MAX } else { 0 }),
                        AxVal::Int(i) => (u32::MAX, 0x10, i as u32),
                        AxVal::Hex(h) => (u32::MAX, 0x11, h),
                        AxVal::Ref(r) => (u32::MAX, 0x01, r),
                    };
                    u32le(&mut c, if at.res_id.is_some() { uri } else { u32::MAX });
                    u32le(&mut c, an);
                    u32le(&mut c, raw);
                    u16le(&mut c, 8);
                    c.push(0);
                    c.push(ty);
                    u32le(&mut c, data);
                }
                body.extend(c);
            }
            AxNode::End(name) => {
                let nidx = intern(name, &mut strings);
                let mut c = Vec::new();
                u16le(&mut c, 0x0103);
                u16le(&mut c, 16);
                u32le(&mut c, 24);
                u32le(&mut c, line as u32 + 2);
                u32le(&mut c, u32::MAX);
                u32le(&mut c, u32::MAX);
                u32le(&mut c, nidx);
                body.extend(c);
            }
        }
    }
    body.extend(ns_chunk(0x0101));
    let refs: Vec<&str> = strings.iter().map(String::as_str).collect();
    let pool = string_pool(&refs, false);
    let mut map = Vec::new();
    u16le(&mut map, 0x0180);
    u16le(&mut map, 8);
    u32le(&mut map, (8 + 4 * ids.len()) as u32);
    for id in &ids {
        u32le(&mut map, *id);
    }
    let mut out = Vec::new();
    u16le(&mut out, 0x0003);
    u16le(&mut out, 8);
    u32le(&mut out, (8 + pool.len() + map.len() + body.len()) as u32);
    out.extend(pool);
    out.extend(map);
    out.extend(body);
    out
}

/// The Notes app's `AndroidManifest.xml`.
pub fn manifest_axml() -> Vec<u8> {
    use AxNode::{End, Start};
    use AxVal::*;
    let name = |v| a("name", 0x0101_0003, Str(v));
    axml(&[
        Start(
            "manifest",
            vec![
                AxAttr { name: "package", res_id: None, val: Str(PACKAGE) },
                a("versionCode", 0x0101_021b, Int(42)),
                a("versionName", 0x0101_021c, Str("1.2.3")),
            ],
        ),
        Start("uses-sdk", vec![a("minSdkVersion", 0x0101_020c, Int(24)), a("targetSdkVersion", 0x0101_0270, Int(34))]),
        End("uses-sdk"),
        Start("uses-permission", vec![name("android.permission.INTERNET")]),
        End("uses-permission"),
        Start("uses-permission", vec![name("android.permission.CAMERA")]),
        End("uses-permission"),
        Start("permission", vec![name("com.example.notes.permission.SYNC"), a("protectionLevel", 0x0101_0009, Hex(2))]),
        End("permission"),
        Start(
            "application",
            vec![
                name(".NotesApp"),
                a("label", 0x0101_0001, Ref(0x7f01_0000)),
                a("debuggable", 0x0101_000f, Bool(true)),
                a("allowBackup", 0x0101_0280, Bool(false)),
            ],
        ),
        Start("activity", vec![name(".MainActivity"), a("exported", 0x0101_0010, Bool(true))]),
        Start("intent-filter", vec![]),
        Start("action", vec![name("android.intent.action.MAIN")]),
        End("action"),
        Start("category", vec![name("android.intent.category.LAUNCHER")]),
        End("category"),
        Start("data", vec![a("scheme", 0x0101_0027, Str("notes")), a("host", 0x0101_0028, Str("open"))]),
        End("data"),
        End("intent-filter"),
        End("activity"),
        Start("service", vec![name(".SyncService"), a("exported", 0x0101_0010, Bool(false))]),
        End("service"),
        Start(
            "provider",
            vec![
                name("androidx.core.content.FileProvider"),
                a("authorities", 0x0101_0018, Str("com.example.notes.files")),
                a("exported", 0x0101_0010, Bool(false)),
            ],
        ),
        End("provider"),
        End("application"),
        End("manifest"),
    ])
}

fn config(lang: &[u8; 2], density: u16, ui_mode: u8, sdk: u16) -> Vec<u8> {
    let mut c = vec![0u8; 64];
    c[0] = 64;
    if lang != b"\0\0" {
        c[8..10].copy_from_slice(lang);
    }
    c[14..16].copy_from_slice(&density.to_le_bytes());
    c[24..26].copy_from_slice(&sdk.to_le_bytes());
    c[29] = ui_mode;
    c
}

fn type_spec(id: u8, entries: u32) -> Vec<u8> {
    let mut c = Vec::new();
    u16le(&mut c, 0x0202);
    u16le(&mut c, 16);
    u32le(&mut c, 16 + 4 * entries);
    c.push(id);
    c.push(0);
    u16le(&mut c, 0);
    u32le(&mut c, entries);
    for _ in 0..entries {
        u32le(&mut c, 0);
    }
    c
}

/// A type chunk from already-encoded entries.
fn type_chunk(id: u8, cfg: &[u8], entries: &[Vec<u8>]) -> Vec<u8> {
    let header = 20 + cfg.len();
    let start = header + 4 * entries.len();
    let mut offs = Vec::new();
    let mut body = Vec::new();
    for e in entries {
        offs.push(body.len() as u32);
        body.extend_from_slice(e);
    }
    let mut c = Vec::new();
    u16le(&mut c, 0x0201);
    u16le(&mut c, header as u16);
    u32le(&mut c, (start + body.len()) as u32);
    c.push(id);
    c.push(0);
    u16le(&mut c, 0);
    u32le(&mut c, entries.len() as u32);
    u32le(&mut c, start as u32);
    c.extend_from_slice(cfg);
    for o in offs {
        u32le(&mut c, o);
    }
    c.extend(body);
    c
}

fn simple_entry(key: u32, ty: u8, data: u32) -> Vec<u8> {
    let mut e = Vec::new();
    u16le(&mut e, 8);
    u16le(&mut e, 0);
    u32le(&mut e, key);
    u16le(&mut e, 8);
    e.push(0);
    e.push(ty);
    u32le(&mut e, data);
    e
}

/// `resources.arsc`: `string/app_name` and `string/greeting` in the default locale and French,
/// and one `style/AppTheme` bag (night, xhdpi, v21) with a parent and a colour item.
pub fn resources_arsc() -> Vec<u8> {
    let global = string_pool(&["Notes", "Hello", "Notes FR", "Bonjour"], true);
    let types = string_pool(&["string", "style"], false);
    let keys = string_pool(&["app_name", "greeting", "AppTheme"], false);
    let mut bag = Vec::new();
    u16le(&mut bag, 16);
    u16le(&mut bag, 1);
    u32le(&mut bag, 2);
    u32le(&mut bag, 0x0103_0005);
    u32le(&mut bag, 1);
    u32le(&mut bag, 0x7f03_0000);
    u16le(&mut bag, 8);
    bag.push(0);
    bag.push(0x1c);
    u32le(&mut bag, 0xff33_6699);
    let mut chunks = Vec::new();
    chunks.extend(type_spec(1, 2));
    chunks.extend(type_chunk(1, &config(b"\0\0", 0, 0, 0), &[simple_entry(0, 3, 0), simple_entry(1, 3, 1)]));
    chunks.extend(type_chunk(1, &config(b"fr", 0, 0, 0), &[simple_entry(0, 3, 2), simple_entry(1, 3, 3)]));
    chunks.extend(type_spec(2, 1));
    chunks.extend(type_chunk(2, &config(b"\0\0", 320, 0x20, 21), &[bag]));
    let header = 288usize;
    let mut pkg = Vec::new();
    u16le(&mut pkg, 0x0200);
    u16le(&mut pkg, header as u16);
    u32le(&mut pkg, (header + types.len() + keys.len() + chunks.len()) as u32);
    u32le(&mut pkg, 0x7f);
    let mut name = [0u16; 128];
    for (i, u) in PACKAGE.encode_utf16().enumerate() {
        name[i] = u;
    }
    for u in name {
        u16le(&mut pkg, u);
    }
    u32le(&mut pkg, header as u32);
    u32le(&mut pkg, 2);
    u32le(&mut pkg, (header + types.len()) as u32);
    u32le(&mut pkg, 3);
    u32le(&mut pkg, 0);
    pkg.extend(types);
    pkg.extend(keys);
    pkg.extend(chunks);
    let mut out = Vec::new();
    u16le(&mut out, 0x0002);
    u16le(&mut out, 12);
    u32le(&mut out, (12 + global.len() + pkg.len()) as u32);
    u32le(&mut out, 1);
    out.extend(global);
    out.extend(pkg);
    out
}
