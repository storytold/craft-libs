//! Tests for the permission prompt model and its egui view.

use craft_plugin_abi::permissions::{DocScope, PermissionError};
use craft_plugin_ui::{
    Decision, GrantAction, GrantError, GrantOutcome, GrantPrompt, GrantsReview, Permission, PromptAction, PromptLevel, describe, parse_permission,
    show_prompt, show_review,
};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn s(list: &[&str]) -> Vec<String> {
    list.iter().map(|x| (*x).to_owned()).collect()
}

fn perm(x: &str) -> Permission {
    parse_permission(x).unwrap_or_else(|e| panic!("{x} should parse: {e}"))
}

#[test]
fn levels_match_abi_section_5() {
    let table = [
        ("commands.run", PromptLevel::Normal),
        ("commands.run:blur.*", PromptLevel::Normal),
        ("commands.observe", PromptLevel::Normal),
        ("commands.intercept", PromptLevel::High),
        ("doc.read", PromptLevel::Normal),
        ("doc.read:metadata", PromptLevel::Normal),
        ("doc.read:pixels", PromptLevel::Normal),
        ("doc.write", PromptLevel::High),
        ("ui.contribute", PromptLevel::Low),
        ("state.local", PromptLevel::Low),
        ("link.call:filmcraft", PromptLevel::High),
        ("link.exec:filmcraft:effect.*", PromptLevel::High),
        ("fs.picked", PromptLevel::OnFirstUse),
        ("net.fetch:https://example.com", PromptLevel::High),
        ("clock", PromptLevel::Low),
        ("random", PromptLevel::Low),
    ];
    for (name, level) in table {
        let p = perm(name);
        assert_eq!(p.prompt_level(), level, "{name}");
        assert_eq!(p.to_string(), name, "wire form round trips");
    }
}

#[test]
fn parse_refuses_unknown_reserved_and_malformed() {
    let invalid = |input: &str| matches!(parse_permission(input), Err(GrantError::Invalid(PermissionError { input: i, .. })) if i == input);
    assert!(invalid("root.everything"));
    assert_eq!(parse_permission("frames.read"), Err(GrantError::Reserved("frames.read".into())));
    for bad in [
        "doc.read:everything",
        "link.call:Film",
        "link.call",
        "link.exec",
        "link.exec:filmcraft",
        "link.exec:filmcraft:",
        "link.exec:Film:x.*",
        "link.exec:filmcraft:a b",
        "net.fetch:http://x.test",
        "net.fetch:https://a b",
        "commands.run:",
        "clock:now",
        "doc.write:x",
    ] {
        assert!(invalid(bad), "{bad}");
    }
}

#[test]
fn rows_sort_high_first_and_start_in_the_right_state() {
    let p = GrantPrompt::new(&s(&["clock", "doc.write", "commands.observe"]), &s(&["fs.picked", "link.call:filmcraft"])).expect("valid");
    let order: Vec<String> = p.rows().iter().map(|r| r.permission.to_string()).collect();
    assert_eq!(order, ["doc.write", "link.call:filmcraft", "commands.observe", "fs.picked", "clock"]);
    let state: Vec<(Decision, bool)> = p.rows().iter().map(|r| (r.decision, r.required)).collect();
    assert_eq!(
        state,
        [(Decision::Pending, true), (Decision::Pending, false), (Decision::Pending, true), (Decision::Allowed, false), (Decision::Allowed, true)]
    );
}

#[test]
fn listed_twice_is_required_once() {
    let p = GrantPrompt::new(&s(&["doc.write"]), &s(&["doc.write"])).expect("valid");
    assert_eq!(p.rows().len(), 1);
    assert!(p.rows()[0].required);
}

#[test]
fn bad_permission_fails_the_whole_prompt() {
    assert!(matches!(GrantPrompt::new(&s(&["clock"]), &s(&["nope"])), Err(GrantError::Invalid(e)) if e.input == "nope"));
}

#[test]
fn outcome_follows_decisions() {
    let mut p = GrantPrompt::new(&s(&["doc.write", "clock"]), &s(&["commands.observe"])).expect("valid");
    assert_eq!(p.outcome(), GrantOutcome::Pending(2));
    assert!(p.set(&perm("doc.write"), Decision::Allowed));
    assert_eq!(p.outcome(), GrantOutcome::Pending(1));
    assert!(p.set(&perm("commands.observe"), Decision::Declined));
    assert_eq!(p.outcome(), GrantOutcome::Granted { granted: vec![perm("doc.write"), perm("clock")], declined: vec![perm("commands.observe")] });
    assert!(p.set(&perm("doc.write"), Decision::Declined));
    assert_eq!(p.outcome(), GrantOutcome::Blocked(vec![perm("doc.write")]));
    assert!(!p.set(&perm("random"), Decision::Allowed), "unknown row is reported");
    p.allow_all();
    assert!(matches!(p.outcome(), GrantOutcome::Granted { declined, .. } if declined.is_empty()));
}

#[test]
fn doc_read_scope_parses() {
    assert_eq!(perm("doc.read:pixels"), Permission::DocRead(Some(DocScope::Pixels)));
}

struct PromptApp {
    prompt: GrantPrompt,
    actions: Vec<PromptAction>,
}

fn prompt_harness(required: &[&str], optional: &[&str]) -> Harness<'static, PromptApp> {
    let prompt = GrantPrompt::new(&s(required), &s(optional)).expect("valid");
    let mut h = Harness::new_ui_state(
        |ui, app: &mut PromptApp| {
            if let Some(a) = show_prompt(ui, &mut app.prompt) {
                app.actions.push(a);
            }
        },
        PromptApp { prompt, actions: Vec::new() },
    );
    h.run();
    h
}

#[test]
fn prompt_view_install_is_disabled_until_all_rows_are_answered() {
    let mut h = prompt_harness(&["doc.write"], &["commands.observe"]);
    h.get_by_label("Install").click();
    h.run();
    assert!(h.state().actions.is_empty(), "Install must not fire while rows are pending");
    assert!(h.query_by_label_contains("still need an answer").is_some());
}

#[test]
fn prompt_view_answers_flow_into_the_model_and_confirm_fires() {
    let mut h = prompt_harness(&["doc.write"], &["commands.observe"]);
    // Rows sort high first, so Allow buttons appear in the order doc.write, commands.observe.
    let allow = h.get_all_by_label("Allow").collect::<Vec<_>>();
    assert_eq!(allow.len(), 2);
    allow[0].click();
    h.run();
    let decline = h.get_all_by_label("Decline").collect::<Vec<_>>();
    decline[1].click();
    h.run();
    assert_eq!(h.state().prompt.outcome(), GrantOutcome::Granted { granted: vec![perm("doc.write")], declined: vec![perm("commands.observe")] });
    h.get_by_label("Install").click();
    h.run();
    assert_eq!(h.state().actions, vec![PromptAction::Confirm]);
}

#[test]
fn prompt_view_declining_a_required_row_blocks_install() {
    let mut h = prompt_harness(&["doc.write"], &[]);
    h.get_by_label("Decline").click();
    h.run();
    assert!(h.query_by_label_contains("Install is blocked").is_some());
    h.get_by_label("Install").click();
    h.run();
    assert!(h.state().actions.is_empty());
    h.get_by_label("Cancel").click();
    h.run();
    assert_eq!(h.state().actions, vec![PromptAction::Cancel]);
}

#[test]
fn review_lists_grants_and_revoke_returns_action() {
    struct App {
        review: GrantsReview,
        actions: Vec<GrantAction>,
    }
    let review = GrantsReview::new("blur-pack", &s(&["clock", "doc.write", "clock"])).expect("valid");
    assert_eq!(review.granted().len(), 2, "duplicates collapse");
    let mut h = Harness::new_ui_state(
        |ui, app: &mut App| {
            let a = show_review(ui, &mut app.review);
            app.actions.extend(a);
        },
        App { review, actions: Vec::new() },
    );
    h.run();
    let revoke = h.get_all_by_label("Revoke").collect::<Vec<_>>();
    assert_eq!(revoke.len(), 2);
    revoke[0].click(); // doc.write sorts first (high level)
    h.run();
    assert_eq!(h.state().actions, vec![GrantAction::Revoke { plugin_id: "blur-pack".into(), permission: perm("doc.write") }]);
    assert_eq!(h.state().review.granted(), [perm("clock")]);
    assert!(h.query_by_label("doc.write").is_none());
}

#[test]
fn review_of_no_grants_says_so() {
    let mut review = GrantsReview::new("p", &[]).expect("valid");
    let mut h = Harness::new_ui(move |ui| {
        let _ = show_review(ui, &mut review);
    });
    h.run();
    assert!(h.query_by_label("This plugin holds no permissions.").is_some());
}

#[test]
fn the_consent_text_says_what_link_call_and_link_exec_allow() {
    let call = describe(&perm("link.call:filmcraft"));
    assert!(call.contains("filmcraft"), "{call}");
    assert!(call.contains("cannot run commands"), "link.call must say it is read-only: {call}");
    let exec = describe(&perm("link.exec:filmcraft:effect.*"));
    assert!(exec.contains("filmcraft") && exec.contains("effect.*"), "{exec}");
    assert!(exec.contains("Run commands") && exec.contains("own rights"), "{exec}");
}
