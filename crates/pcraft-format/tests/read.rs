use craft_pcraft_format::{
    COMPOSITE, Error, FORMAT_VERSION, FULL_COMPOSITE, Limits, Manifest, Step, THUMB, TileRef, read_manifest, read_manifest_with,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Doc {
    name: String,
    #[serde(default)]
    tiles: Vec<TileRef>,
}

const NONE: &[Step] = &[];

fn read(json: &[u8]) -> Result<Manifest<Doc>, Error> {
    read_manifest(json, &Limits::default(), NONE, FORMAT_VERSION)
}

fn nested_document(depth: usize) -> Vec<u8> {
    let mut json = br#"{"format_version":1,"generator":"g","document":{"name":"a","deep":"#.to_vec();
    json.extend("[".repeat(depth).bytes());
    json.extend("]".repeat(depth).bytes());
    json.extend(b"}}");
    json
}

#[test]
fn a_manifest_reads_with_its_previews_and_document() {
    let json = br#"{"format_version":1,"generator":"photocraft-format 0.1.0","document":{"name":"a","tiles":[{"tx":0,"ty":-1,"hash":"x"}]},"thumbnail":"thumb.png","composite":null,"later":[1]}"#;
    let m = read(json).unwrap();
    assert_eq!(m.format_version, 1);
    assert_eq!(m.generator, "photocraft-format 0.1.0");
    assert_eq!(m.document.name, "a");
    assert_eq!(m.document.tiles, vec![TileRef::new(0, -1, "x".into())]);
    assert_eq!(m.thumbnail.as_deref(), Some("thumb.png"));
    assert_eq!(m.composite, None);
}

#[test]
fn a_full_composite_has_one_fixed_path_and_may_be_absent() {
    for full in [Value::Null, json!(FULL_COMPOSITE)] {
        let json = json!({"format_version": 1, "generator": "g", "document": {"name": "a"}, "full_composite": full});
        assert_eq!(read(json.to_string().as_bytes()).unwrap().full_composite.as_deref(), full.as_str());
    }
    for path in ["../full.png", "composite/preview.png", "/composite/full.png", "composite/FULL.png"] {
        let json = json!({"format_version": 1, "generator": "g", "document": {"name": "a"}, "full_composite": path});
        assert!(matches!(read(json.to_string().as_bytes()), Err(Error::Corrupt(_))), "{path}");
    }
    for full in [json!(1), json!([]), json!({})] {
        let json = json!({"format_version": 1, "generator": "g", "document": {"name": "a"}, "full_composite": full});
        assert!(matches!(read(json.to_string().as_bytes()), Err(Error::Json(_))));
    }
}

#[test]
fn a_newer_manifest_is_too_new_and_names_its_writer() {
    let json = br#"{"format_version":2,"generator":"photocraft-format 9.0.0","document":{"name":"a"}}"#;
    let Err(Error::TooNew { found, supported, generator, .. }) = read(json) else { panic!("not TooNew") };
    assert_eq!((found, supported), (2, 1));
    assert_eq!(generator.as_deref(), Some("photocraft-format 9.0.0"));
    let message = read(json).err().map(|e| e.to_string());
    assert_eq!(message.as_deref(), Some("bundle written by a newer version (format 2, this build reads up to 1)"));
}

#[test]
fn a_newer_manifest_without_a_string_generator_is_too_new_without_one() {
    for json in [&br#"{"format_version":3,"document":{}}"#[..], br#"{"format_version":3,"generator":7}"#] {
        assert!(matches!(read(json), Err(Error::TooNew { found: 3, generator: None, .. })));
    }
}

#[test]
fn a_manifest_past_the_limits_is_refused_before_it_is_read() {
    let limits = Limits::default();
    let deep = nested_document(limits.max_depth);
    assert!(matches!(read(&deep), Err(Error::LimitExceeded(m)) if m == "manifest JSON nested deeper than 367 levels"));
    let within = nested_document(limits.max_depth - 2);
    let m: Manifest<Value> = read_manifest(&within, &limits, NONE, FORMAT_VERSION).unwrap();
    assert_eq!(m.document["name"], "a");

    let mut small = Limits::default();
    small.max_manifest_bytes = 10;
    let json = br#"{"format_version":1,"generator":"g","document":{"name":"a"}}"#;
    let large: Result<Manifest<Doc>, Error> = read_manifest(json, &small, NONE, FORMAT_VERSION);
    assert!(matches!(large, Err(Error::LimitExceeded(m)) if m == "`manifest.json` exceeds 10 bytes"));

    let mut few = Limits::default();
    few.max_items = 2;
    let many: Result<Manifest<Doc>, Error> = read_manifest(br#"{"format_version":1,"generator":"g","document":{"name":"a"}}"#, &few, NONE, 1);
    assert!(matches!(many, Err(Error::LimitExceeded(_))));
}

#[test]
fn the_steps_run_before_the_document_is_read() {
    fn rename(v: &mut Value) -> Result<(), Error> {
        v["document"]["name"] = v["document"]["title"].take();
        Ok(())
    }
    let steps: &[Step] = &[rename];
    let json = br#"{"format_version":1,"generator":"g","document":{"title":"old"}}"#;
    let m: Manifest<Doc> = read_manifest(json, &Limits::default(), steps, 2).unwrap();
    assert_eq!(m.format_version, 2);
    assert_eq!(m.document.name, "old");
}

#[test]
fn a_missing_or_invalid_envelope_member_is_a_json_error() {
    for (json, message) in [
        (&br#"{"format_version":1,"generator":"g"}"#[..], "missing field `document`"),
        (br#"{"format_version":1,"document":{"name":"a"}}"#, "missing field `generator`"),
        (br#"{"format_version":1,"generator":"g","document":{"name":"a"},"thumbnail":5}"#, "invalid type: integer `5`, expected a string"),
        (br#"{"format_version":1,"generator":"g","document":{"title":"a"}}"#, "missing field `name`"),
    ] {
        let e = read(json).err();
        assert!(matches!(&e, Some(Error::Json(_))), "{e:?}");
        assert_eq!(e.map(|e| e.to_string()).as_deref(), Some(format!("manifest JSON: {message}").as_str()));
    }
}

/// The manifest as a struct with a derived `Deserialize`, for comparing errors.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Derived {
    format_version: u32,
    generator: String,
    document: Doc,
    #[serde(default)]
    thumbnail: Option<String>,
    #[serde(default)]
    composite: Option<String>,
}

#[test]
fn a_manifest_with_several_faults_reports_the_one_a_derived_struct_reports() {
    for json in [
        json!({"format_version": 1, "composite": 1, "document": {"title": "a"}}),
        json!({"format_version": 1, "generator": 2, "document": {"title": "a"}}),
        json!({"format_version": 1, "document": {"title": "a"}, "thumbnail": 3}),
        json!({"format_version": 1, "document": {"name": "a"}, "thumbnail": 3}),
        json!({"format_version": 1, "thumbnail": 3}),
        json!({"format_version": 1}),
        json!({"format_version": 1, "generator": "g"}),
        json!({"format_version": 1, "composite": [], "generator": {}}),
    ] {
        let want = serde_json::from_value::<Derived>(json.clone()).err().map(|e| format!("manifest JSON: {e}"));
        let got = read(json.to_string().as_bytes()).err().map(|e| e.to_string());
        assert!(want.is_some());
        assert_eq!(got, want, "{json}");
    }
}

#[test]
fn a_missing_or_invalid_version_is_corrupt() {
    assert!(matches!(read(br#"{"generator":"g","document":{"name":"a"}}"#), Err(Error::Corrupt(_))));
    assert!(matches!(read(br#"{"format_version":"1","generator":"g","document":{"name":"a"}}"#), Err(Error::Corrupt(_))));
    assert!(matches!(read(br#"{"format_version":1.5,"generator":"g","document":{"name":"a"}}"#), Err(Error::Corrupt(_))));
    assert!(matches!(read(b"[1]"), Err(Error::Corrupt(_))));
}

#[test]
fn a_preview_other_than_its_entry_is_corrupt() {
    for (member, entry) in [
        ("thumbnail", "../x"),
        ("thumbnail", "/etc/passwd"),
        ("composite", "../x"),
        ("composite", "C:\\x.png"),
        ("thumbnail", "composite/preview.png"),
        ("composite", "thumb.png"),
    ] {
        let json = json!({"format_version": 1, "generator": "g", "document": {"name": "a"}, member: entry});
        let e = read(json.to_string().as_bytes()).err();
        assert!(matches!(&e, Some(Error::Corrupt(_))), "{json}: {e:?}");
    }
    let json =
        json!({"format_version": 1, "generator": "g", "document": {"name": "a"}, "thumbnail": "thumb.png", "composite": "composite/preview.png"});
    let m = read(json.to_string().as_bytes()).unwrap();
    assert_eq!((m.thumbnail.as_deref(), m.composite.as_deref()), (Some(THUMB), Some(COMPOSITE)));
}

#[derive(Debug)]
enum AppError {
    Format(Error),
    Document(String),
}

impl From<Error> for AppError {
    fn from(e: Error) -> Self {
        AppError::Format(e)
    }
}

#[test]
fn read_manifest_with_reads_the_document_with_the_caller_s_function() {
    let json = br#"{"format_version":1,"generator":"g","document":{"name":"a","layers":[1,2,3]}}"#;
    let none: &[Step<AppError>] = &[];
    let m = read_manifest_with(json, &Limits::default(), none, 1, |mut document: Value| {
        let layers = document.get_mut("layers").map(Value::take).unwrap_or_default();
        Ok::<_, AppError>((document, layers))
    })
    .unwrap();
    assert_eq!(m.document.0, json!({"name": "a", "layers": null}));
    assert_eq!(m.document.1, json!([1, 2, 3]));

    let refused = read_manifest_with(json, &Limits::default(), none, 1, |_| Err::<(), _>(AppError::Document("no".into())));
    assert!(matches!(refused, Err(AppError::Document(m)) if m == "no"));
    let newer = read_manifest_with(br#"{"format_version":2}"#, &Limits::default(), none, 1, |_| Ok::<(), AppError>(()));
    assert!(matches!(newer, Err(AppError::Format(Error::TooNew { found: 2, .. }))));
}
