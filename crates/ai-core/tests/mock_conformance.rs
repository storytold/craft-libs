//! Drives the mock provider through the shared conformance checks, and shows the model working
//! end to end the way an app would use it: declare roles, resolve through the store, check
//! consent, submit, poll, cancel. Also proves one library serves different modalities.

use craft_ai_core::conformance::{run_cancel, run_lifecycle, run_rejects, run_unknown_job};
use craft_ai_core::consent::Disclosure;
use craft_ai_core::job::{Event, JobState, Output};
use craft_ai_core::mock::{MockProvider, Outcome, Script};
use craft_ai_core::role::{Binding, Role};
use craft_ai_core::*;

fn cap(s: &str) -> CapabilityId {
    CapabilityId::parse(s).unwrap()
}

fn model_id(s: &str) -> ModelId {
    ModelId::new(s).unwrap()
}

fn png() -> Payload {
    Payload::inline("image/png", vec![1, 2, 3]).unwrap()
}

fn image_model() -> ModelInfo {
    ModelInfo::new(model_id("img"), "Image model").with(
        CapabilitySpec::new(cap("image.inpaint"))
            .accepting(InputRole::Prompt, 1)
            .accepting(InputRole::Source, 1)
            .accepting(InputRole::Mask, 1)
            .requiring(InputRole::Source)
            .requiring(InputRole::Mask)
            .max_outputs(3),
    )
}

fn text_model() -> ModelInfo {
    ModelInfo::new(model_id("txt"), "Text model").with(
        CapabilitySpec::new(cap("text.complete")).accepting(InputRole::Prompt, 1).accepting(InputRole::System, 1).with_features(Features {
            streaming: true,
            tools: true,
            structured_output: true,
        }),
    )
}

fn inpaint_request() -> Request {
    Request::new(cap("image.inpaint")).with_input(InputRole::Source, png()).with_input(InputRole::Mask, png()).with_outputs(2)
}

fn two_outputs() -> Vec<Output> {
    (1..=2).map(|i| Output { payload: png(), label: Some(format!("variation {i}")) }).collect()
}

#[test]
fn image_job_passes_the_lifecycle_and_returns_variations() {
    let p = MockProvider::new(vec![image_model()], Script { outcome: Outcome::Succeed(two_outputs()), ..Script::default() });
    let done = run_lifecycle(&p, &model_id("img"), &inpaint_request()).unwrap();
    assert_eq!(done.state, JobState::Succeeded);
    assert_eq!(done.events.iter().filter(|e| matches!(e, Event::Output(_))).count(), 2);
}

#[test]
fn text_job_streams_deltas_in_order() {
    let script = Script { text_chunks: vec!["Hel".into(), "lo ".into(), "world".into()], ..Script::default() };
    let p = MockProvider::new(vec![text_model()], script);
    let req = Request::new(cap("text.complete")).with_input(InputRole::Prompt, Payload::text("say hello")).streaming();
    let done = run_lifecycle(&p, &model_id("txt"), &req).unwrap();
    let text: String = done.events.iter().filter_map(|e| if let Event::TextDelta { text } = e { Some(text.as_str()) } else { None }).collect();
    assert_eq!(text, "Hello world");
}

#[test]
fn failure_is_reported_with_its_kind() {
    let err = AiError::new(ErrorKind::Quota, "out of credits");
    let p = MockProvider::new(vec![image_model()], Script { outcome: Outcome::Fail(err.clone()), ..Script::default() });
    let done = run_lifecycle(&p, &model_id("img"), &inpaint_request()).unwrap();
    assert_eq!(done.state, JobState::Failed { error: err });
}

#[test]
fn cancel_mid_job_ends_cancelled() {
    let p = MockProvider::new(vec![image_model()], Script { running_polls: 50, ..Script::default() });
    run_cancel(&p, &model_id("img"), &inpaint_request()).unwrap();
}

#[test]
fn unsupported_requests_are_refused_before_sending() {
    let p = MockProvider::new(vec![image_model(), text_model()], Script::default());
    let with_reference = inpaint_request().with_input(InputRole::Reference, png());
    run_rejects(&p, &model_id("img"), &with_reference, ErrorKind::Unsupported).unwrap();
    let too_many = inpaint_request().with_outputs(9);
    run_rejects(&p, &model_id("img"), &too_many, ErrorKind::Unsupported).unwrap();
    let missing_mask = Request::new(cap("image.inpaint")).with_input(InputRole::Source, png());
    run_rejects(&p, &model_id("img"), &missing_mask, ErrorKind::InvalidRequest).unwrap();
    let wrong_model = Request::new(cap("text.complete")).with_input(InputRole::Prompt, Payload::text("x"));
    run_rejects(&p, &model_id("img"), &wrong_model, ErrorKind::Unsupported).unwrap();
}

#[test]
fn unknown_jobs_are_errors() {
    run_unknown_job(&MockProvider::new(vec![image_model()], Script::default())).unwrap();
}

#[test]
fn an_app_flow_from_store_to_finished_job() {
    // The launcher's store: one remote connection offering both models, defaults for two apps' roles.
    let conn = Connection {
        id: ConnectionId::new("conn1").unwrap(),
        label: "My provider".into(),
        provider: "http".into(),
        endpoint: Some("https://api.example.com".into()),
        locality: Locality::Remote,
        auth: Auth::ApiKey { secret: SecretRef::new("secret-conn1").unwrap() },
        billing: Billing::Metered,
        models: vec![image_model(), text_model()],
        enabled: true,
        key_header: None,
    };
    let fill = Role {
        id: RoleId::new("generative-fill").unwrap(),
        label: "Generative Fill".into(),
        capability: cap("image.inpaint"),
        inputs: vec![InputRole::Source, InputRole::Mask],
    };
    let rewrite =
        Role { id: RoleId::new("rewrite").unwrap(), label: "Rewrite".into(), capability: cap("text.complete"), inputs: vec![InputRole::Prompt] };
    let bind = |role: &Role, model: &str| Binding { role: role.id.clone(), connection: conn.id.clone(), model: model_id(model) };
    let store = StoreFile { connections: vec![conn.clone()], defaults: vec![bind(&fill, "img"), bind(&rewrite, "txt")], ..StoreFile::default() };
    let store = StoreFile::parse(&store.to_json().unwrap()).unwrap();

    // Two different apps resolve their own roles from the same store.
    let photocraft = AppId::new("photocraft").unwrap();
    let wordcraft = AppId::new("wordcraft").unwrap();
    let resolved = store.resolve(&photocraft, &fill).unwrap();
    assert_eq!(resolved.model.id.as_str(), "img");
    assert_eq!(store.resolve(&wordcraft, &rewrite).unwrap().model.id.as_str(), "txt");

    // Consent is required for a remote connection, shows kinds and sizes, and is remembered.
    let request = inpaint_request().with_input(InputRole::Prompt, Payload::text("remove the lamp"));
    let disclosure = Disclosure::for_request(&photocraft, resolved.connection, &request);
    let mut ledger = ConsentLedger::default();
    assert!(ledger.needs_consent(&disclosure, 0));
    ledger.grant(&disclosure);
    assert!(!ledger.needs_consent(&disclosure, 0));

    // Run it through a provider built from the same model list.
    let provider = MockProvider::new(resolved.connection.models.clone(), Script { outcome: Outcome::Succeed(two_outputs()), ..Script::default() });
    resolved.spec.validate_request(&request).unwrap();
    let done = run_lifecycle(&provider, &resolved.model.id, &request).unwrap();
    assert_eq!(done.state, JobState::Succeeded);
}

trait ValidateRequest {
    fn validate_request(&self, r: &Request) -> Result<(), AiError>;
}

impl ValidateRequest for CapabilitySpec {
    fn validate_request(&self, r: &Request) -> Result<(), AiError> {
        r.validate(self, &Limits::default())
    }
}
