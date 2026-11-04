use craft_link_core::{Id, Manifest, affected, validate_edge};

#[test]
fn graph_rejects_cycles_without_runtime_or_storage() {
    let (a, b, c) = (Id::from_u128(1), Id::from_u128(2), Id::from_u128(3));
    let edges = [(a, b), (b, c)];
    assert_eq!(affected(&edges, a), [b, c].into());
    assert!(validate_edge(&edges, c, a).is_err());
    assert!(validate_edge(&edges, a, a).is_err());
}

#[test]
fn unknown_manifest_schema_and_properties_fail_explicitly() {
    let mut m:Manifest=serde_json::from_str(r#"{"schema":1,"application":"example.app","version":"1","object_types":{"rectangle":["name"]},"representations":["application/json"],"read":true,"write":true,"history":"external","transactions":"external","lifecycle":"external","authentication":"caller","filesystem":"caller","fidelity":"name only"}"#).unwrap();
    m.negotiate("rectangle", &["name".into()], true).unwrap();
    assert!(m.negotiate("rectangle", &["mask".into()], true).is_err());
    m.schema = 2;
    assert!(m.validate().is_err());
}
