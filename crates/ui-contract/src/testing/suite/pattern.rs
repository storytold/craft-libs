//! The pattern rows of the 24.4 table. A pattern has no pipeline, so the suite shows each story of the pattern
//! and reads what the shown components left in the AccessKit tree: every interactive node has a name and the
//! runtime's role rules reported nothing.

use crate::testing::access::violations;
use crate::testing::fixture::Fixture;
use crate::testing::story::Story;

/// The trial body for the pattern `name`: it needs a story, and each story shows without a violation.
pub(super) fn run(name: &str, stories: &[Story]) -> Result<(), String> {
    if stories.is_empty() {
        return Err(format!("`{name}` is a registered pattern with no story"));
    }
    let mut failures = Vec::new();
    for story in stories {
        let mut fixture = Fixture::story(story);
        fixture.settle();
        failures.extend(violations(fixture.ctx(), fixture.tree()).into_iter().map(|found| format!("{}: {found}", story.name)));
    }
    if failures.is_empty() { Ok(()) } else { Err(failures.join("\n")) }
}
