use craft_ai_core::{CapabilityId, InputRole, Role, RoleId, StoreFile};

fn tool(id: &str) -> Role {
    Role {
        id: RoleId::new(id).unwrap(),
        label: "Describe photograph".into(),
        capability: CapabilityId::parse("image.describe").unwrap(),
        inputs: vec![InputRole::Source],
    }
}

#[test]
fn duplicate_user_tool_ids_are_refused_before_the_store_is_written() {
    let store = StoreFile { custom_tools: vec![tool("describe-photo"), tool("describe-photo")], ..StoreFile::default() };
    assert!(store.validate().is_err());
}

#[test]
fn old_store_loads_and_user_tool_round_trips() {
    let old = r#"{"schema_version":1,"connections":[],"defaults":[],"apps":{}}"#;
    let mut store = StoreFile::parse(old).unwrap();
    assert!(store.custom_tools.is_empty());
    store.custom_tools.push(tool("describe-photo"));
    let again = StoreFile::parse(&store.to_json().unwrap()).unwrap();
    assert_eq!(again.custom_tools, store.custom_tools);
    assert_eq!(again.custom_tools[0].capability.as_str(), "image.describe");
}

#[test]
fn hostile_tool_declarations_are_refused_on_load() {
    let mut s = StoreFile { custom_tools: vec![tool("describe-photo"); 65], ..StoreFile::default() };
    assert!(s.validate().is_err());
    s.custom_tools.truncate(1);
    s.custom_tools[0].inputs.push(InputRole::Source);
    assert!(StoreFile::parse(&s.to_json().unwrap()).is_err());
    s.custom_tools[0].inputs = vec![InputRole::Source];
    s.custom_tools[0].label = " \t ".into();
    assert!(StoreFile::parse(&s.to_json().unwrap()).is_err());
    s.custom_tools[0].label = "x".repeat(121);
    assert!(StoreFile::parse(&s.to_json().unwrap()).is_err());
    s.custom_tools[0].label = "Describe photograph".into();
    s.custom_tools[0].inputs.clear();
    assert!(StoreFile::parse(&s.to_json().unwrap()).is_err());
}

#[test]
fn an_app_cannot_reinterpret_the_inputs_of_a_shared_custom_tool() {
    use craft_ai_core::role::ResolveError;
    use craft_ai_core::{AppId, Binding, CapabilitySpec, Connection, ConnectionId, ModelId, ModelInfo};
    let shared = tool("describe-photo");
    let model = ModelInfo::new(ModelId::new("m").unwrap(), "M").with(CapabilitySpec::new(shared.capability.clone()).accepting(InputRole::Source, 1));
    let connection: Connection = serde_json::from_value(serde_json::json!({
        "id":"service", "label":"Service", "provider":"test",
        "endpoint":"http://127.0.0.1:9", "locality":"local", "auth":{"kind":"none"},
        "billing":"free", "models":[model]
    }))
    .unwrap();
    let store = StoreFile {
        connections: vec![connection],
        defaults: vec![Binding { role: shared.id.clone(), connection: ConnectionId::new("service").unwrap(), model: ModelId::new("m").unwrap() }],
        custom_tools: vec![shared.clone()],
        ..StoreFile::default()
    };
    let app = AppId::new("photocraft").unwrap();
    assert!(store.resolve(&app, &shared).is_ok());
    let mut redefined = shared;
    redefined.inputs.clear();
    assert!(matches!(store.resolve(&app, &redefined), Err(ResolveError::ToolMismatch(_))));
}
