//! Drives the mock provider through the shared conformance checks.

use craft_ai_core::conformance::run_lifecycle;
use craft_ai_core::job::{Event, JobState, Output};
use craft_ai_core::mock::{MockProvider, Outcome, Script};
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
