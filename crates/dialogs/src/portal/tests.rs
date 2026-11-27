use super::*;
use craft_ui::testing::trial;

/// A portal answer falls back to another dialog only when the portal failed: a quick code 2 is a
/// dialog that never showed, a slow one is Esc, a queued cancel wins over a late answer, and a
/// failed or empty answer is unavailable.
#[trial]
fn a_portal_answer_falls_back_only_when_it_failed() {
    let nothing: HashMap<String, OwnedValue> = HashMap::new();
    let file = HashMap::from([("uris".to_owned(), Value::from(vec!["file:///s/a.txt".to_owned()]).try_to_owned().expect("an owned value"))]);
    let (quick, slow) = (Duration::ZERO, TOO_QUICK);
    assert_eq!(classify(2, quick, false, false, &nothing), Decision::Retry, "a dialog that closed at once never showed");
    assert_eq!(classify(2, slow, false, false, &nothing), Decision::Cancelled, "Esc in GNOME's dialog is a cancel");
    assert_eq!(classify(1, quick, false, false, &nothing), Decision::Cancelled, "Cancel is never asked again");
    assert!(matches!(classify(0, slow, false, false, &file), Decision::Saved(saved) if saved.path == *"/s/a.txt"), "a save names its file");
    assert_eq!(classify(0, slow, true, false, &file), Decision::Cancelled, "an answer after a queued cancel is a cancel");
    assert!(matches!(classify(0, slow, false, true, &nothing), Decision::Unavailable(_)), "the portal left the bus");
    assert!(matches!(classify(3, slow, false, false, &file), Decision::Unavailable(_)), "another code is a failure");
    assert!(matches!(classify(0, slow, false, false, &nothing), Decision::Unavailable(_)), "a save without a file");
}

#[trial]
fn file_uris_become_local_paths() {
    assert_eq!(file_uri_path("file:///home/me/Untitled-1.psd"), Some(PathBuf::from("/home/me/Untitled-1.psd")));
    assert_eq!(file_uri_path("file:///home/me/A%20b%C3%A9.png"), Some(PathBuf::from("/home/me/A bé.png")));
    assert_eq!(file_uri_path("file://localhost/tmp/x.png"), Some(PathBuf::from("/tmp/x.png")));
    assert_eq!(file_uri_path("sftp://host/x.png"), None);
    assert_eq!(file_uri_path("file://host/x.png"), None);
}

/// A `current_filter` of another shape than `(sa(us))` (a structure with fewer, other or more fields, a string)
/// gives the path alone; `uris` of another shape is an error.
#[trial]
fn an_answer_of_another_shape_is_not_read() {
    let answer_with = |uris: Value<'_>, filter: Option<Value<'_>>| {
        let mut r = HashMap::new();
        r.insert("uris".to_string(), OwnedValue::try_from(uris).unwrap());
        if let Some(filter) = filter {
            r.insert("current_filter".to_string(), OwnedValue::try_from(filter).unwrap());
        }
        saved(&r)
    };
    let card = || Value::from(vec!["file:///tmp/card".to_string()]);
    let malformed = [
        Value::from(("PNG".to_string(),)),
        Value::from(("PNG".to_string(), vec![(0u32,)])),
        Value::from(("PNG".to_string(), vec![("*.png".to_string(), 0u32)])),
        Value::from((1u32, vec![(0u32, "*.png".to_string())])),
        Value::from(("PNG".to_string(), vec![(0u32, "*.png".to_string())], 1u32)),
        Value::from("PNG"),
    ];
    for filter in malformed {
        let shape = filter.value_signature().to_string();
        assert_eq!(answer_with(card(), Some(filter)), Ok(Saved { path: "/tmp/card".into(), filter: None }), "{shape}");
    }
    // The shape the portal defines is still read, and so is the one `options` sends.
    let png = Value::from(("PNG".to_string(), vec![(0u32, "*.png".to_string())]));
    assert_eq!(answer_with(card(), Some(png)), Ok(Saved { path: "/tmp/card".into(), filter: Some("PNG".into()) }));
    let sent = Value::from(wire(&Filter::new("Photoshop", &["psd", "psb"])));
    assert_eq!(filter_label(&sent), Some("Photoshop".into()));
    let malformed = [
        Value::from(vec![1u32]),
        Value::from(vec![("file:///tmp/card".to_string(),)]),
        Value::from(Vec::<String>::new()),
        Value::from("file:///tmp/card"),
    ];
    for uris in malformed {
        let shape = uris.value_signature().to_string();
        assert!(answer_with(uris, None).is_err(), "{shape}");
    }
}
