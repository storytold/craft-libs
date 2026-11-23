//! Tests on schemas written in the compact text form and messages built in code.
#![allow(clippy::unwrap_used, clippy::panic)]

use super::{DEFAULT_MAX_VALUES, Def, Field, Kind, Kv, Reader, Schema, Type, encode};

const SCHEMA: &str = "
enum Kind NONE BOX TEXT
struct Id uint session, uint local
struct Point float x, float y
message Blob byte[] bytes
message Node Id id, Kind kind, string name, bool visible, Point size, Node[] children, float[] dashes, int64 stamp
message Message Node[] nodes, Blob[] blobs
";

fn schema() -> Schema {
    Schema::parse(SCHEMA).unwrap()
}

fn o(fields: Vec<(&'static str, Kv<'static>)>) -> Kv<'static> {
    Kv::Object(fields)
}

fn node(local: u64, name: &str, children: Vec<Kv<'static>>) -> Kv<'static> {
    o(vec![
        ("id", o(vec![("session", Kv::Uint(1)), ("local", Kv::Uint(local))])),
        ("kind", Kv::Enum("BOX")),
        ("name", Kv::Str(name.into())),
        ("visible", Kv::Bool(true)),
        ("size", o(vec![("x", Kv::Float(10.5)), ("y", Kv::Float(-2.0))])),
        ("children", Kv::Array(children)),
        ("dashes", Kv::Array(vec![Kv::Float(1.0), Kv::Float(0.0)])),
        ("stamp", Kv::Int(-5_000_000_000)),
    ])
}

fn message() -> Kv<'static> {
    o(vec![
        ("nodes", Kv::Array(vec![node(1, "a", vec![node(2, "héllo", vec![])]), node(3, "", vec![])])),
        ("blobs", Kv::Array(vec![o(vec![("bytes", Kv::Bytes(vec![0, 1, 2, 255]))])])),
    ])
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

#[test]
fn overflowing_u32_varint_is_rejected() {
    let mut boundary = Reader { data: &[0xff, 0xff, 0xff, 0xff, 0x0f], pos: 0 };
    assert_eq!(boundary.var_uint(), Ok(u32::MAX));

    let mut overflowing = Reader { data: &[0xff, 0xff, 0xff, 0xff, 0x7f], pos: 0 };
    assert!(overflowing.var_uint().is_err(), "overflowing u32 varint was accepted");
}

#[test]
fn the_wire_form_is_the_published_one() {
    // message M { uint a = 1; string s = 2; float f = 3; int i = 4; }: field id, value, …, 0.
    let sc = Schema::parse("message M uint a, string s, float f, int i").unwrap();
    let v = o(vec![("a", Kv::Uint(300)), ("s", Kv::Str("a".into())), ("f", Kv::Float(0.0)), ("i", Kv::Int(-1))]);
    // 300 as a varint is AC 02; a string ends in NUL; a zero float is one zero byte; -1 zigzags to 1.
    let bytes = encode::message(&sc, 0, &v).unwrap();
    assert_eq!(bytes, [1, 0xAC, 0x02, 2, b'a', 0, 3, 0, 4, 1, 0]);
    assert_eq!(sc.decode_message(0, &bytes).unwrap(), v);
    // 1.0f32 (0x3F800000): the exponent byte comes first.
    let one = encode::message(&sc, 0, &o(vec![("f", Kv::Float(1.0))])).unwrap();
    assert_eq!(one, [3, 0x7F, 0, 0, 0, 0]);
}

#[test]
fn duplicate_explicit_field_ids_are_ambiguous() {
    assert!(Schema::parse("message M uint a=1, uint b=1").is_err());
}

#[test]
fn schema_and_message_round_trip() {
    let sc = schema();
    let back = Schema::decode(&encode::schema(&sc)).unwrap();
    assert_eq!(back.defs.len(), sc.defs.len());
    for (a, b) in sc.defs.iter().zip(&back.defs) {
        assert_eq!((a.name.as_str(), a.kind), (b.name.as_str(), b.kind));
        let fields = |d: &Def| d.fields.iter().map(|f| (f.name.clone(), f.ty, f.array, f.value)).collect::<Vec<_>>();
        assert_eq!(fields(a), fields(b));
    }
    let msg = message();
    let root = sc.find("Message").unwrap();
    assert_eq!(back.decode_message(root, &encode::message(&sc, root, &msg).unwrap()).unwrap(), msg);
}

#[test]
fn scalars_round_trip() {
    let names = ["f", "i", "u", "i64", "u64", "s", "b"];
    let types = [Type::Float, Type::Int, Type::Uint, Type::Int64, Type::Uint64, Type::Str, Type::Bool];
    let fields = names
        .iter()
        .zip(types)
        .enumerate()
        .map(|(i, (n, t))| Field { name: n.to_string(), ty: Some(t), array: false, value: i as u32 + 1 })
        .collect();
    let sc = Schema { defs: vec![Def { name: "M".into(), kind: Kind::Message, fields }] };
    let cases = [
        (0.0f32, 0i32, 0u32, 0i64, 0u64),
        (-1.5, -7, 300, i64::MIN, u64::MAX),
        (3.25e7, i32::MAX, u32::MAX, i64::MAX, 1 << 60),
        (1e-40, i32::MIN, 1, -1, 127),
    ];
    for (fv, iv, uv, i64v, u64v) in cases {
        let mut w = encode::Writer::default();
        w.var_uint(1);
        w.var_float(fv);
        w.var_uint(2);
        w.var_int(iv);
        w.var_uint(3);
        w.var_uint(uv);
        w.var_uint(4);
        w.var_int64(i64v);
        w.var_uint(5);
        w.var_uint64(u64v);
        w.var_uint(6);
        w.string("héllo");
        w.var_uint(7);
        w.byte(1);
        w.var_uint(0);
        let got = sc.decode_message(0, &w.0).unwrap();
        // Denormals are stored as zero.
        let want_f = if fv.abs() < f32::MIN_POSITIVE { 0.0 } else { fv };
        assert_eq!(got.get("f"), Some(&Kv::Float(want_f)));
        assert_eq!(got.get("i"), Some(&Kv::Int(i64::from(iv))));
        assert_eq!(got.get("u"), Some(&Kv::Uint(u64::from(uv))));
        assert_eq!(got.get("i64"), Some(&Kv::Int(i64v)));
        assert_eq!(got.get("u64"), Some(&Kv::Uint(u64v)));
        assert_eq!(got.get("s"), Some(&Kv::Str("héllo".into())));
        assert_eq!(got.get("b"), Some(&Kv::Bool(true)));
    }
}

#[test]
fn the_encoder_refuses_what_does_not_fit_the_schema() {
    let sc = schema();
    let root = sc.find("Message").unwrap();
    let bad = [
        o(vec![("missing", Kv::Uint(1))]),
        o(vec![("nodes", Kv::Uint(1))]),
        o(vec![("nodes", Kv::Array(vec![o(vec![("kind", Kv::Enum("NOPE"))])]))]),
        o(vec![("nodes", Kv::Array(vec![o(vec![("id", o(vec![("session", Kv::Uint(1))]))])]))]),
        o(vec![("nodes", Kv::Array(vec![o(vec![("size", o(vec![("x", Kv::Float(f32::NAN)), ("y", Kv::Float(0.0))]))])]))]),
        o(vec![("nodes", Kv::Array(vec![o(vec![("id", o(vec![("session", Kv::Int(-1)), ("local", Kv::Uint(0))]))])]))]),
    ];
    for v in bad {
        assert!(encode::message(&sc, root, &v).is_err(), "{v:?}");
    }
    // Infinity is a value; a message nested past the depth cap is not written.
    let inf = o(vec![("nodes", Kv::Array(vec![o(vec![("dashes", Kv::Array(vec![Kv::Float(f32::INFINITY)]))])]))]);
    assert!(encode::message(&sc, root, &inf).is_ok());
    let mut deep = o(vec![]);
    for _ in 0..super::MAX_DEPTH + 2 {
        deep = o(vec![("children", Kv::Array(vec![deep]))]);
    }
    assert!(encode::message(&sc, root, &o(vec![("nodes", Kv::Array(vec![deep]))])).is_err());
}

#[test]
fn mangled_input_never_panics() {
    let sc = schema();
    let root = sc.find("Message").unwrap();
    let schema_bytes = encode::schema(&sc);
    let msg = encode::message(&sc, root, &message()).unwrap();
    let mut r = Lcg(11);
    for cut in 0..schema_bytes.len() {
        let _ = Schema::decode(&schema_bytes[..cut]);
    }
    for cut in 0..msg.len() {
        let _ = sc.decode_message(root, &msg[..cut]);
    }
    for _ in 0..2000 {
        let mut m = msg.clone();
        for _ in 0..1 + r.next() % 4 {
            let i = (r.next() as usize) % m.len();
            m[i] = r.next() as u8;
        }
        let _ = sc.decode_message(root, &m);
        let mut sbytes = schema_bytes.clone();
        let i = (r.next() as usize) % sbytes.len();
        sbytes[i] = r.next() as u8;
        if let Ok(s2) = Schema::decode(&sbytes)
            && let Some(rt) = s2.find("Message")
        {
            let _ = s2.decode_message(rt, &msg);
        }
    }
    for _ in 0..500 {
        let junk: Vec<u8> = (0..(r.next() % 200)).map(|_| r.next() as u8).collect();
        let _ = Schema::decode(&junk);
        let _ = sc.decode_message(root, &junk);
        let _ = Schema::parse(&String::from_utf8_lossy(&junk));
    }
}

#[test]
fn scalar_fields_count_against_the_value_budget() {
    // A message of 1000 scalar fields (id 1, `true`) is 1001 values: a budget of 10 refuses it.
    let fields = vec![Field { name: "b".into(), ty: Some(Type::Bool), array: false, value: 1 }];
    let sc = Schema { defs: vec![Def { name: "Message".into(), kind: Kind::Message, fields }] };
    let mut data = [1u8, 1].repeat(1000);
    data.push(0);
    let e = sc.decode_message_within(0, &data, 10).unwrap_err();
    assert_eq!(e.0, "message: too many values");
    assert!(sc.decode_message_within(0, &data, 1001).is_ok());
    assert!(sc.decode_message_within(0, &data, 1000).is_err());
    assert!(DEFAULT_MAX_VALUES >= 1001 && sc.decode_message(0, &data).is_ok());
}

#[test]
fn empty_arrays_count_against_the_value_budget() {
    // 1000 empty `uint[]` fields: each field is a value even with no elements, so a small budget
    // can't be outgrown by piling up empty containers.
    let fields = vec![Field { name: "a".into(), ty: Some(Type::Uint), array: true, value: 1 }];
    let sc = Schema { defs: vec![Def { name: "Message".into(), kind: Kind::Message, fields }] };
    let mut data = [1u8, 0].repeat(1000);
    data.push(0);
    let e = sc.decode_message_within(0, &data, 10).unwrap_err();
    assert_eq!(e.0, "message: too many values");
    assert!(sc.decode_message_within(0, &data, 1001).is_ok());
}

#[test]
fn message_tags_find_their_field_by_id_in_a_wide_definition() {
    // Every tag is looked up by id, and the first field with an id wins.
    let mut fields: Vec<Field> = (1..=5000).map(|i| Field { name: format!("f{i}"), ty: Some(Type::Bool), array: false, value: i }).collect();
    fields.push(Field { name: "shadow".into(), ty: Some(Type::Bool), array: false, value: 7 });
    let sc = Schema { defs: vec![Def { name: "Message".into(), kind: Kind::Message, fields }] };
    let mut w = encode::Writer::default();
    for _ in 0..20_000 {
        w.var_uint(5000);
        w.byte(1);
    }
    w.var_uint(7);
    w.byte(0);
    w.var_uint(0);
    let Kv::Object(out) = sc.decode_message(0, &w.0).unwrap() else { panic!() };
    assert_eq!(out.len(), 20_001);
    assert!(out.iter().take(20_000).all(|(n, v)| *n == "f5000" && *v == Kv::Bool(true)));
    assert_eq!(out.last().map(|(n, _)| *n), Some("f7"));
}

#[test]
fn a_schema_is_refused_past_its_limits() {
    // 25 definitions of 10 000 fields each: every one is within the per-definition cap, but
    // together they pass the schema's total.
    let mut w = encode::Writer::default();
    w.var_uint(25);
    for d in 0..25 {
        w.string(&format!("D{d}"));
        w.byte(0);
        w.var_uint(10_000);
        for i in 0..10_000u32 {
            w.string("");
            w.var_int(0);
            w.byte(0);
            w.var_uint(i);
        }
    }
    assert_eq!(Schema::decode(&w.0).unwrap_err().0, "schema: too many fields");
    // More definitions, or more fields in one, than the caps (and than the data could hold).
    let mut w = encode::Writer::default();
    w.var_uint(super::MAX_DEFS as u32 + 1);
    w.0.resize(20_000, 0);
    assert_eq!(Schema::decode(&w.0).unwrap_err().0, "schema: too many definitions");
    let mut w = encode::Writer::default();
    w.var_uint(1);
    w.string("D");
    w.byte(0);
    w.var_uint(super::MAX_FIELDS as u32 + 1);
    w.0.resize(20_000, 0);
    assert_eq!(Schema::decode(&w.0).unwrap_err().0, "schema: too many fields");
}

#[test]
fn hostile_input_is_refused() {
    // A message that contains itself, nested past the depth cap.
    let field = |ty, array| vec![Field { name: "m".into(), ty: Some(ty), array, value: 1 }];
    let nested = Schema { defs: vec![Def { name: "Message".into(), kind: Kind::Message, fields: field(Type::Def(0), false) }] };
    assert_eq!(nested.decode_message(0, &[1u8; 10_000]).unwrap_err().0, "message: nested too deeply");
    // An array count larger than the data.
    let array = Schema { defs: vec![Def { name: "Message".into(), kind: Kind::Message, fields: field(Type::Uint, true) }] };
    assert!(array.decode_message(0, &[1, 0xff, 0xff, 0xff, 0xff, 0x0f]).is_err());
    // A field type naming a missing definition.
    let mut w = encode::Writer::default();
    w.var_uint(1);
    w.string("Message");
    w.byte(2);
    w.var_uint(1);
    w.string("x");
    w.var_int(5);
    w.byte(0);
    w.var_uint(1);
    assert!(Schema::decode(&w.0).is_err());
    // A message field id the definition doesn't have.
    assert!(array.decode_message(0, &[9, 0]).is_err());
}

#[test]
fn a_wide_schema_decodes_without_a_scan_per_field() {
    // 20 messages of 10 000 distinct field ids each: within every cap. A scan of the earlier
    // fields for each new one made this a billion comparisons.
    let mut w = encode::Writer::default();
    w.var_uint(20);
    for d in 0..20 {
        w.string(&format!("D{d}"));
        w.byte(2);
        w.var_uint(10_000);
        for i in 1..=10_000u32 {
            w.string("f");
            w.var_int(-1);
            w.byte(0);
            w.var_uint(i);
        }
    }
    let started = std::time::Instant::now();
    let sc = Schema::decode(&w.0).unwrap();
    assert_eq!(sc.defs.iter().map(|d| d.fields.len()).sum::<usize>(), 200_000);
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
    // A repeated id is still refused.
    let mut w = encode::Writer::default();
    w.var_uint(1);
    w.string("M");
    w.byte(2);
    w.var_uint(2);
    for _ in 0..2 {
        w.string("f");
        w.var_int(-1);
        w.byte(0);
        w.var_uint(3);
    }
    assert_eq!(Schema::decode(&w.0).unwrap_err().0, "schema: duplicate field id 3 in M");
}

#[test]
fn the_text_form_keeps_the_limits_and_refuses_ambiguous_definitions() {
    // Definitions without fields are definitions.
    let sc = Schema::parse("message Empty\nstruct Nothing\nenum None\nmessage M Empty e, Nothing n").unwrap();
    assert_eq!(sc.defs.iter().map(|d| d.fields.len()).collect::<Vec<_>>(), [0, 0, 0, 2]);
    assert_eq!(Schema::decode(&encode::schema(&sc)).unwrap().defs.len(), 4);
    // Two fields of one name, two values of one name, a message field with id 0 or a repeated id.
    for bad in ["message M uint x, uint x", "struct S uint x, float x", "enum E A A", "message M uint x=0", "message M uint a=2, uint b=2"] {
        assert!(Schema::parse(bad).is_err(), "{bad}");
    }
    // A struct's fields have no ids on the wire: their numbers are free.
    assert!(Schema::parse("struct S uint a=0, uint b=0").is_ok());
    // The caps of the binary form.
    let wide = |n: usize| format!("message M {}", (0..n).map(|i| format!("bool f{i}")).collect::<Vec<_>>().join(", "));
    assert!(Schema::parse(&wide(super::MAX_FIELDS)).is_ok());
    assert_eq!(Schema::parse(&wide(super::MAX_FIELDS + 1)).unwrap_err().0, "schema text: too many fields");
    let many = (0..=super::MAX_DEFS).map(|i| format!("enum E{i}")).collect::<Vec<_>>().join("\n");
    assert_eq!(Schema::parse(&many).unwrap_err().0, "schema text: too many definitions");
    let line = (0..super::MAX_FIELDS).map(|i| format!("bool f{i}")).collect::<Vec<_>>().join(", ");
    let total = (0..=super::MAX_SCHEMA_FIELDS / super::MAX_FIELDS).map(|i| format!("struct S{i} {line}")).collect::<Vec<_>>().join("\n");
    assert_eq!(Schema::parse(&total).unwrap_err().0, "schema text: too many fields");
}
