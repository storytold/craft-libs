//! The dialog's state machine, with no server and no window.
//!
//! What matters here is what the dialog does with answers: that it offers
//! nothing until the service says sharing exists, that an answer the user has
//! typed past is thrown away rather than painted, that a share reaching four
//! of five people says which one it missed, and that a picture nobody can
//! fetch is asked for once.

use std::sync::{Arc, Mutex};

use super::*;

/// A backend that records what was asked and answers nothing, so a test
/// decides when each answer arrives and in which order.
#[derive(Clone, Default)]
struct Silent(Arc<Mutex<Vec<Request>>>);

impl Silent {
    fn seen(&self) -> Vec<Request> {
        self.0.lock().map(|seen| seen.clone()).unwrap_or_default()
    }
}

impl ShareBackend for Silent {
    fn submit(&self, request: Request, _reply: Replier) {
        if let Ok(mut seen) = self.0.lock() {
            seen.push(request);
        }
    }
}

fn can(capabilities: Vec<Capability>) -> Answer {
    Answer::Capabilities { capabilities, linked: true }
}

fn everything() -> Answer {
    can(vec![Capability::Sharing, Capability::Contacts, Capability::Mail])
}

/// A connected dialog that has been told sharing works, open on one document.
fn ready() -> (Context, ShareDialog, Silent) {
    let ctx = Context::default();
    let backend = Silent::default();
    let mut dialog = ShareDialog::new();
    dialog.connect(backend.clone());
    dialog.collect(&ctx);
    dialog.receive(&ctx, 1, Ok(everything()));
    dialog.open("files/budget.xlsx", "Budget");
    (ctx, dialog, backend)
}

fn person(id: &str, name: &str) -> Contact {
    Contact {
        id: id.into(),
        target: ShareTarget::User(id.into()),
        name: name.into(),
        email: Some(format!("{id}@example.com")),
        avatar: None,
        is_group: false,
    }
}

fn share(id: &str, target: ShareTarget) -> Share {
    Share { id: id.into(), target, display: None, permission: Permission::View, url: None, note: None, created: None }
}

#[test]
fn a_share_is_labelled_with_the_name_the_provider_gave() {
    let labels = ShareLabels::default();
    let mut one = share("s1", ShareTarget::User("adunn".to_owned()));
    assert_eq!(super::share_label(&one, &labels), "adunn", "the handle is all there is");
    one.display = Some("Ada Lovelace".to_owned());
    assert_eq!(super::share_label(&one, &labels), "Ada Lovelace");
    // A provider that answers with an empty string has said nothing.
    one.display = Some("   ".to_owned());
    assert_eq!(super::share_label(&one, &labels), "adunn");

    let mut team = share("s2", ShareTarget::Group("eng".to_owned()));
    team.display = Some("Engineering".to_owned());
    assert_eq!(super::share_label(&team, &labels), format!("Engineering ({})", labels.group));

    // A link is not a person and never carries one.
    assert_eq!(super::share_label(&share("s3", ShareTarget::Link), &labels), labels.anyone_with_link);
}

#[test]
fn nothing_is_offered_until_the_service_says_sharing_exists() {
    let ctx = Context::default();
    let backend = Silent::default();
    let mut dialog = ShareDialog::new();
    assert!(!dialog.available(), "an unconnected dialog cannot share");

    dialog.connect(backend.clone());
    assert_eq!(backend.seen(), Vec::new(), "connecting alone asks nothing");
    dialog.collect(&ctx);
    assert_eq!(backend.seen(), vec![Request::Capabilities]);

    // Still unanswered: the question is outstanding, so the answer is no.
    assert!(!dialog.available());
    dialog.open("files/budget.xlsx", "Budget");
    assert!(!dialog.is_open(), "the dialog must not open before the answer");

    // A provider with files but no sharing hides the feature rather than
    // offering a button that fails.
    dialog.receive(&ctx, 1, Ok(can(vec![Capability::Files, Capability::Contacts])));
    assert!(!dialog.available());
    dialog.open("files/budget.xlsx", "Budget");
    assert!(!dialog.is_open());

    dialog.receive(&ctx, 1, Ok(everything()));
    assert!(dialog.available());
    dialog.open("files/budget.xlsx", "Budget");
    assert!(dialog.is_open());
}

#[test]
fn an_unlinked_account_cannot_share_even_where_the_provider_can() {
    let ctx = Context::default();
    let mut dialog = ShareDialog::new();
    dialog.connect(Silent::default());
    dialog.receive(&ctx, 1, Ok(Answer::Capabilities { capabilities: vec![Capability::Sharing], linked: false }));
    assert!(!dialog.available());
}

#[test]
fn losing_sharing_closes_a_dialog_that_was_already_up() {
    let (ctx, mut dialog, _) = ready();
    assert!(dialog.is_open());
    dialog.receive(&ctx, 1, Ok(can(vec![Capability::Files])));
    assert!(!dialog.is_open(), "a dialog that cannot do anything must not stay up");
}

#[test]
fn opening_asks_who_already_has_access() {
    let (_ctx, dialog, backend) = ready();
    assert_eq!(backend.seen(), vec![Request::Capabilities, Request::Access { id: "files/budget.xlsx".into() }]);
    assert!(dialog.is_open());
}

#[test]
fn an_unsaved_document_asks_where_it_could_go_and_never_about_a_handle() {
    let ctx = Context::default();
    let backend = Silent::default();
    let mut dialog = ShareDialog::new();
    dialog.connect(backend.clone());
    dialog.collect(&ctx);
    dialog.receive(&ctx, 1, Ok(everything()));
    dialog.open("", "Book1");
    assert!(dialog.is_open());
    // Sharing an unsaved document means saving it first. The dialog therefore
    // needs somewhere to offer. It asks nothing about a handle. There is none.
    assert_eq!(backend.seen(), vec![Request::Capabilities, Request::Destinations]);
    assert!(!backend.seen().iter().any(|r| matches!(r, Request::Access { .. })), "there is no handle to ask about");
}

#[test]
fn an_answer_the_user_has_typed_past_is_dropped() {
    let (ctx, mut dialog, _) = ready();
    let early = dialog.issue(Request::Lookup { query: "an".into(), limit: MATCHES });
    let late = dialog.issue(Request::Lookup { query: "ann".into(), limit: MATCHES });
    if let Some(sheet) = &mut dialog.open {
        sheet.lookup = Some(late);
        sheet.searching = true;
    }

    // The first query answers second, which is what a service under load
    // does. Painting it would show matches for "an" under the text "ann".
    dialog.receive(&ctx, late, Ok(Answer::Lookup(vec![person("ann", "Ann Reed")])));
    dialog.receive(&ctx, early, Ok(Answer::Lookup(vec![person("andy", "Andy Shaw"), person("anna", "Anna Beck")])));

    let sheet = dialog.open.as_ref().expect("still open");
    assert_eq!(sheet.matches.len(), 1);
    assert_eq!(sheet.matches.first().map(|c| c.name.as_str()), Some("Ann Reed"));
    assert!(!sheet.searching);
}

#[test]
fn several_recipients_can_be_picked_and_each_only_once() {
    let (_ctx, mut dialog, _) = ready();
    let sheet = dialog.open.as_mut().expect("open");
    for contact in [person("ann", "Ann Reed"), person("bob", "Bob Vine"), person("ann", "Ann Reed")] {
        choose(sheet, &contact);
    }
    choose(
        sheet,
        &Contact {
            id: "finance".into(),
            target: ShareTarget::Group("finance".into()),
            name: "Finance".into(),
            email: None,
            avatar: None,
            is_group: true,
        },
    );

    let picked: Vec<&ShareTarget> = sheet.chosen.iter().map(|c| &c.target).collect();
    assert_eq!(
        picked,
        vec![&ShareTarget::User("ann".into()), &ShareTarget::User("bob".into()), &ShareTarget::Group("finance".into())],
        "a group shares as a group, and picking the same person twice adds one chip"
    );
    assert!(sheet.query.is_empty(), "picking clears what was typed");
}

#[test]
fn a_share_reaching_four_of_five_names_the_fifth() {
    let (ctx, mut dialog, _) = ready();
    let sheet = dialog.open.as_mut().expect("open");
    for who in ["ann", "bob", "cal", "dee"] {
        choose(sheet, &person(who, who));
    }
    add(sheet, ShareTarget::Email("nobody@@".into()), "nobody@@".into(), None, None);
    let granting = dialog.issue(Request::Grant(Grant {
        id: "files/budget.xlsx".into(),
        password: None,
        targets: Vec::new(),
        permission: Permission::Edit,
        note: None,
        notify: true,
    }));
    if let Some(sheet) = &mut dialog.open {
        sheet.granting = Some(granting);
    }

    let shared: Vec<Share> =
        ["ann", "bob", "cal", "dee"].iter().enumerate().map(|(i, who)| share(&format!("s{i}"), ShareTarget::User((*who).into()))).collect();
    let failed =
        vec![Failure { target: ShareTarget::Email("nobody@@".into()), code: "invalid_address".into(), detail: "nobody@@ is not an address".into() }];
    dialog.receive(&ctx, granting, Ok(Answer::Granted { shared, failed }));

    let (shared, failed) = dialog.outcome();
    assert_eq!(shared.len(), 4);
    assert_eq!(failed.len(), 1, "a wrong address is named, not folded into one error");
    assert_eq!(failed.first().map(|f| f.detail.as_str()), Some("nobody@@ is not an address"));

    // The four that went through leave the picker; the one that did not stays,
    // so correcting it is one edit rather than a retyped list.
    let left = dialog.open.as_ref().expect("open");
    assert_eq!(left.chosen.len(), 1);
    assert_eq!(left.chosen.first().map(|c| &c.target), Some(&ShareTarget::Email("nobody@@".into())));
}

#[test]
fn a_refusal_that_stopped_the_whole_call_is_not_a_failed_recipient() {
    let (ctx, mut dialog, _) = ready();
    let granting = dialog.issue(Request::Grant(Grant {
        id: "files/budget.xlsx".into(),
        password: None,
        targets: vec![ShareTarget::User("ann".into())],
        permission: Permission::View,
        note: None,
        notify: false,
    }));
    if let Some(sheet) = &mut dialog.open {
        sheet.granting = Some(granting);
    }
    dialog.receive(&ctx, granting, Err("sign in again".into()));

    let sheet = dialog.open.as_ref().expect("open");
    assert_eq!(sheet.error.as_deref(), Some("sign in again"));
    assert!(sheet.failed.is_empty());
    assert!(sheet.granting.is_none(), "the button must come back");
}

#[test]
fn a_successful_share_reloads_who_has_access() {
    let (ctx, mut dialog, backend) = ready();
    let granting = dialog.issue(Request::Grant(Grant {
        id: "files/budget.xlsx".into(),
        password: None,
        targets: vec![ShareTarget::User("ann".into())],
        permission: Permission::View,
        note: None,
        notify: false,
    }));
    if let Some(sheet) = &mut dialog.open {
        sheet.granting = Some(granting);
    }
    dialog.receive(&ctx, granting, Ok(Answer::Granted { shared: vec![share("s1", ShareTarget::User("ann".into()))], failed: Vec::new() }));
    let asked = backend.seen().iter().filter(|r| **r == Request::Access { id: "files/budget.xlsx".into() }).count();
    assert_eq!(asked, 2, "the list is stale the moment a share succeeds");
}

#[test]
fn withdrawing_takes_it_off_the_list_and_only_then() {
    let (ctx, mut dialog, backend) = ready();
    let access = dialog.open.as_ref().and_then(|s| s.access_seq).expect("asked");
    dialog.receive(
        &ctx,
        access,
        Ok(Answer::Access(vec![share("s1", ShareTarget::User("ann".into())), share("s2", ShareTarget::Group("finance".into()))])),
    );
    dialog.apply(&ctx, vec![Act::Withdraw("s1".into())]);

    let waiting = dialog.open.as_ref().expect("open");
    assert_eq!(waiting.access.as_ref().map(Vec::len), Some(2), "it stays until the server agrees");
    assert_eq!(waiting.withdrawing.len(), 1);
    assert!(backend.seen().contains(&Request::Withdraw { share_id: "s1".into() }));

    let seq = waiting.withdrawing.first().map(|(seq, _)| *seq).expect("in flight");
    dialog.receive(&ctx, seq, Ok(Answer::Withdrawn { share_id: "s1".into() }));
    let after = dialog.open.as_ref().expect("open");
    assert_eq!(after.access.as_ref().map(|a| a.iter().map(|s| s.id.as_str()).collect::<Vec<_>>()), Some(vec!["s2"]));
    assert!(after.withdrawing.is_empty());
}

#[test]
fn a_refused_withdrawal_leaves_the_share_and_says_why() {
    let (ctx, mut dialog, _) = ready();
    let access = dialog.open.as_ref().and_then(|s| s.access_seq).expect("asked");
    dialog.receive(&ctx, access, Ok(Answer::Access(vec![share("s1", ShareTarget::User("ann".into()))])));
    dialog.apply(&ctx, vec![Act::Withdraw("s1".into())]);
    let seq = dialog.open.as_ref().and_then(|s| s.withdrawing.first().map(|(seq, _)| *seq)).expect("in flight");
    dialog.receive(&ctx, seq, Err("you do not own this share".into()));

    let sheet = dialog.open.as_ref().expect("open");
    assert_eq!(sheet.access.as_ref().map(Vec::len), Some(1));
    assert!(sheet.withdrawing.is_empty(), "the button must come back");
    assert_eq!(sheet.error.as_deref(), Some("you do not own this share"));
}

#[test]
fn a_picture_is_asked_for_once_and_a_refusal_is_remembered() {
    let (ctx, mut dialog, backend) = ready();
    let url = "https://cloud.example.com/avatar/ann/64";
    dialog.apply(&ctx, vec![Act::Avatar(url.into()), Act::Avatar(url.into())]);
    let asked = backend.seen().iter().filter(|r| **r == Request::Avatar { url: url.into() }).count();
    assert_eq!(asked, 1, "one row painted twice is one picture");

    let seq = match dialog.avatars.get(url) {
        Some(Avatar::Pending(seq)) => *seq,
        other => panic!("expected a pending picture, got {other:?}"),
    };
    dialog.receive(&ctx, seq, Err("this build does not fetch contact pictures".into()));
    assert_eq!(dialog.avatars.get(url), Some(&Avatar::Missing));

    // Refused once is refused. Painting the row again must not ask again.
    dialog.apply(&ctx, vec![Act::Avatar(url.into())]);
    let asked = backend.seen().iter().filter(|r| **r == Request::Avatar { url: url.into() }).count();
    assert_eq!(asked, 1);

    // And a refused picture is the dialog's business, not the user's.
    assert_eq!(dialog.open.as_ref().and_then(|s| s.error.clone()), None);
}

#[test]
fn an_unanswered_capability_question_is_asked_again_later() {
    let ctx = Context::default();
    let backend = Silent::default();
    let mut dialog = ShareDialog::new();
    dialog.connect(backend.clone());
    dialog.collect(&ctx);
    dialog.collect(&ctx);
    assert_eq!(backend.seen().len(), 1, "a question already outstanding is not asked every frame");

    dialog.asked = Some(-RETRY);
    dialog.collect(&ctx);
    assert_eq!(backend.seen().len(), 2, "one unreachable moment must not hide Share for the session");

    dialog.receive(&ctx, 2, Ok(everything()));
    dialog.asked = Some(-RETRY);
    dialog.collect(&ctx);
    assert_eq!(backend.seen().len(), 2, "an answered question is not asked again");
}

#[test]
fn what_counts_as_an_address() {
    for typed in ["ann@example.com", "  ann.reed+tag@mail.example.co.uk  "] {
        assert!(address(typed).is_some(), "{typed} is an address");
    }
    for typed in ["ann", "ann@", "@example.com", "ann@example", "ann reed@example.com", "ann@.com", "a@b@c.com", ""] {
        assert!(address(typed).is_none(), "{typed} is not an address");
    }
    assert_eq!(address("  ann@example.com "), Some("ann@example.com".to_owned()), "the trim is kept");
}

#[test]
fn initials_come_from_the_name_and_never_from_an_index() {
    assert_eq!(initials("Ann Reed"), "AR");
    assert_eq!(initials("ann"), "A");
    assert_eq!(initials("Ann Marie Reed"), "AM");
    assert_eq!(initials(""), "");
    assert_eq!(initials("   "), "");
    assert_eq!(initials("Ærø Ødegård"), "ÆØ");
    assert_eq!(initials("张 伟"), "张伟");
}

#[test]
fn every_kind_of_recipient_reads_as_something() {
    let labels = ShareLabels::default();
    assert_eq!(target_label(&ShareTarget::User("ann".into()), &labels), "ann");
    assert_eq!(target_label(&ShareTarget::Group("finance".into()), &labels), "finance (group)");
    assert_eq!(target_label(&ShareTarget::Email("ann@example.com".into()), &labels), "ann@example.com");
    assert_eq!(target_label(&ShareTarget::Link, &labels), "Anyone with the link");
}

#[test]
fn a_failure_with_no_detail_still_says_something() {
    let bare = Failure { target: ShareTarget::Link, code: "forbidden".into(), detail: "   ".into() };
    assert_eq!(describe(&bare), "forbidden");
    let told = Failure { target: ShareTarget::Link, code: "forbidden".into(), detail: "link shares are off".into() };
    assert_eq!(describe(&told), "link shares are off");
}

#[test]
fn two_contacts_never_share_one_picture_cache_entry() {
    let one = avatar_uri("https://cloud.example.com/avatar/ann/64");
    let two = avatar_uri("https://cloud.example.com/avatar/anna/64");
    assert_ne!(one, two);
    assert_eq!(one, avatar_uri("https://cloud.example.com/avatar/ann/64"), "the same URL is the same entry");
    assert!(one.starts_with("bytes://"), "egui's own loaders have to recognize it");
}

#[test]
fn a_closed_dialog_still_collects_and_reports_nothing() {
    let (ctx, mut dialog, _) = ready();
    dialog.close();
    assert!(!dialog.is_open());
    dialog.receive(&ctx, 99, Ok(Answer::Lookup(vec![person("ann", "Ann Reed")])));
    dialog.receive(&ctx, 99, Err("gone".into()));
    let (shared, failed) = dialog.outcome();
    assert!(shared.is_empty() && failed.is_empty());
}

#[test]
fn pressing_return_takes_the_first_match_or_an_address() {
    let (_ctx, mut dialog, _) = ready();
    let sheet = dialog.open.as_mut().expect("open");

    // With matches on screen, Return takes the one at the top.
    sheet.query = "an".into();
    sheet.matches = vec![person("ann", "Ann Reed"), person("andy", "Andy Shaw")];
    commit(sheet);
    assert_eq!(sheet.chosen.first().map(|c| &c.target), Some(&ShareTarget::User("ann".into())));

    // With none, an address is taken as itself. Somebody with no account on
    // this server is reached by address or not at all.
    sheet.query = "outside@example.com".into();
    commit(sheet);
    assert_eq!(sheet.chosen.len(), 2);
    assert_eq!(sheet.chosen.last().map(|c| &c.target), Some(&ShareTarget::Email("outside@example.com".into())));

    // A name that matched nothing is not a recipient.
    sheet.query = "nobody".into();
    commit(sheet);
    assert_eq!(sheet.chosen.len(), 2);
    assert_eq!(sheet.query, "nobody", "and what was typed is left alone to be corrected");
}
