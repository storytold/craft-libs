//! Gated on the `share` feature, which is off by default.
#![cfg(feature = "share")]

//! The share dialog driven through its real widgets, offscreen.
//!
//! Every assertion here reads the accessibility tree the dialog actually
//! produced, or the requests it actually sent, rather than its private state.
//! No window is opened, no GPU is touched and no server is contacted: the
//! backend is a fake that parks each request until a test answers it, which is
//! also how answers are made to arrive late or in the wrong order.

use std::sync::{Arc, Mutex};

use craft_cloud::{Capability, Contact, Permission, Share, ShareTarget};
use craft_ui::share::{Answer, Failure, Replier, Request, ShareBackend, ShareDialog};
use egui::{Context, Event, FullOutput, Modifiers, PointerButton, Pos2, RawInput, Rect, accesskit, pos2, vec2};

const SCREEN: [f32; 2] = [620.0, 900.0];
const DOCUMENT: &str = "files/Budget 2026.xlsx";

#[derive(Default)]
struct Log {
    seen: Vec<Request>,
    parked: Vec<(Request, Replier)>,
}

/// A backend that answers what the dialog needs to get going and parks the
/// rest, so a test decides when each answer arrives.
#[derive(Clone)]
struct Fake {
    log: Arc<Mutex<Log>>,
    capabilities: Vec<Capability>,
    access: Vec<Share>,
}

impl Fake {
    fn new(capabilities: Vec<Capability>, access: Vec<Share>) -> Fake {
        Fake { log: Arc::new(Mutex::new(Log::default())), capabilities, access }
    }

    fn seen(&self) -> Vec<Request> {
        self.log.lock().map(|log| log.seen.clone()).unwrap_or_default()
    }

    fn parked(&self) -> Vec<Request> {
        self.log.lock().map(|log| log.parked.iter().map(|(r, _)| r.clone()).collect()).unwrap_or_default()
    }

    /// Answer the request that has waited longest, which is the ordinary case.
    fn answer(&self, answer: Answer) {
        self.answer_nth(0, answer);
    }

    /// Answer the request parked at `index`, oldest first, so a test can
    /// deliver a late answer after a newer one.
    fn answer_nth(&self, index: usize, answer: Answer) {
        let Some((_, reply)) = self.take(index) else { panic!("nothing is waiting at {index}") };
        reply.answer(answer);
    }

    /// Take one parked request back, to refuse it or inspect its contents.
    fn take(&self, index: usize) -> Option<(Request, Replier)> {
        self.log.lock().ok().and_then(|mut log| if index < log.parked.len() { Some(log.parked.remove(index)) } else { None })
    }
}

impl ShareBackend for Fake {
    fn submit(&self, request: Request, reply: Replier) {
        let Ok(mut log) = self.log.lock() else { return };
        log.seen.push(request.clone());
        match &request {
            Request::Capabilities => {
                reply.answer(Answer::Capabilities { capabilities: self.capabilities.clone(), linked: true });
            }
            Request::Access { .. } => reply.answer(Answer::Access(self.access.clone())),
            _ => log.parked.push((request, reply)),
        }
    }
}

struct Harness {
    ctx: Context,
    dialog: ShareDialog,
    time: f64,
    typing: bool,
}

impl Harness {
    fn new(capabilities: Vec<Capability>, access: Vec<Share>) -> (Harness, Fake) {
        let ctx = Context::default();
        ctx.enable_accesskit();
        ctx.all_styles_mut(|style| {
            style.animation_time = 0.0;
            style.visuals.text_cursor.blink = false;
        });
        let fake = Fake::new(capabilities, access);
        let mut dialog = ShareDialog::new();
        dialog.connect(fake.clone());
        let mut harness = Harness { ctx, dialog, time: 0.0, typing: false };
        // One frame asks what the provider can do, the next reads the answer.
        harness.frame(Vec::new());
        harness.frame(Vec::new());
        (harness, fake)
    }

    /// An ordinary deployment: files, sharing, contacts and mail.
    fn everything() -> (Harness, Fake) {
        Harness::new(
            // Everything, which has to include ShareNotify. Notifying a
            // recipient of a share is a separate ability from sending
            // arbitrary mail, and the box is gated on the former.
            vec![Capability::Files, Capability::Sharing, Capability::Contacts, Capability::Mail, Capability::ShareNotify],
            Vec::new(),
        )
    }

    fn frame(&mut self, events: Vec<Event>) -> FullOutput {
        self.time += 1.0 / 60.0;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(SCREEN[0], SCREEN[1]))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let ctx = self.ctx.clone();
        if self.typing {
            // Keeping the recipient field focused is what a user holding the
            // caret there does; nothing else in the dialog wants focus.
            ctx.memory_mut(|m| m.request_focus(ShareDialog::query_field_id()));
        }
        let dialog = &mut self.dialog;
        let mut output = ctx.run_ui(raw, |ui| dialog.show_in(ui));
        output.textures_delta.clear();
        output
    }

    /// Let the clock run, for the debounce.
    fn wait(&mut self, seconds: f64) -> FullOutput {
        self.time += seconds;
        self.frame(Vec::new())
    }

    fn open(&mut self) {
        self.dialog.open(DOCUMENT, "Budget 2026.xlsx");
        assert!(self.dialog.is_open(), "the dialog should have opened");
        self.frame(Vec::new());
    }

    /// Type into the recipient field, then let the debounce elapse.
    fn type_and_settle(&mut self, text: &str) {
        self.typing = true;
        self.frame(vec![Event::Text(text.to_owned())]);
        self.wait(0.25);
    }

    fn click(&mut self, label: &str) {
        let output = self.frame(Vec::new());
        let Some(at) = control(&output, label) else {
            panic!("nothing to click labelled {label:?}; the controls on screen are {:?}", controls(&output))
        };
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![press(at, true)]);
        self.frame(vec![press(at, false)]);
        self.frame(Vec::new());
    }
}

fn press(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

fn key(key: egui::Key) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
}

/// One widget as the accessibility tree reported it.
struct Seen {
    role: accesskit::Role,
    label: String,
    bounds: Option<accesskit::Rect>,
    disabled: bool,
}

impl Seen {
    /// Something a pointer can act on, as opposed to a piece of text.
    fn interactive(&self) -> bool {
        matches!(self.role, accesskit::Role::Button | accesskit::Role::RadioButton | accesskit::Role::CheckBox)
    }

    fn at(&self) -> Option<Pos2> {
        self.bounds.map(|b| pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32))
    }
}

fn seen(output: &FullOutput) -> Vec<Seen> {
    let Some(tree) = output.platform_output.accesskit_update.as_ref() else { return Vec::new() };
    tree.nodes
        .iter()
        .filter_map(|(_, node)| {
            // egui puts a plain label's text in the accessibility value and a
            // control's text in the label, so both are text on screen.
            node.label().or_else(|| node.value()).map(|text| Seen {
                role: node.role(),
                label: text.to_owned(),
                bounds: node.bounds(),
                disabled: node.is_disabled(),
            })
        })
        .collect()
}

/// Where to click the control carrying this exact label.
fn control(output: &FullOutput, label: &str) -> Option<Pos2> {
    seen(output).iter().find(|s| s.interactive() && !s.disabled && s.label == label).and_then(Seen::at)
}

fn controls(output: &FullOutput) -> Vec<String> {
    seen(output).into_iter().filter(Seen::interactive).map(|s| s.label).collect()
}

/// The buttons alone, which is what a chip and a match row are.
fn buttons(output: &FullOutput) -> Vec<String> {
    seen(output).into_iter().filter(|s| s.role == accesskit::Role::Button).map(|s| s.label).collect()
}

/// Every piece of text on screen, for asserting what the user was told.
fn text(output: &FullOutput) -> Vec<String> {
    seen(output).into_iter().map(|s| s.label).collect()
}

/// The primary action, taken from the defaults the dialog actually draws.
/// A literal here drifts the moment the wording changes.
const SEND: &str = "Send";

/// The permission dropdown, chevron included. It is a menu, not a toggle.
const PERMISSION: &str = "View";

fn says(output: &FullOutput, fragment: &str) -> bool {
    text(output).iter().any(|line| line.contains(fragment))
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

fn group(id: &str, name: &str) -> Contact {
    Contact { id: id.into(), target: ShareTarget::Group(id.into()), name: name.into(), email: None, avatar: None, is_group: true }
}

fn existing(id: &str, target: ShareTarget, permission: Permission) -> Share {
    Share { id: id.into(), target, display: None, permission, url: None, note: None, created: None }
}

fn lookup(found: Vec<Contact>) -> Answer {
    Answer::Lookup(found)
}

// ---- the picker -----------------------------------------------------------

#[test]
fn typing_a_few_letters_offers_matches_and_picking_them_builds_chips() {
    let (mut h, fake) = Harness::everything();
    h.open();

    h.type_and_settle("an");
    h.frame(Vec::new());
    assert_eq!(
        fake.parked(),
        vec![Request::Lookup { query: "an".into(), limit: 8 }],
        "one lookup for the whole burst of typing, not one per keystroke"
    );

    fake.answer(lookup(vec![person("ann", "Ann Reed"), person("andy", "Andy Shaw"), group("finance", "Finance")]));
    let output = h.frame(Vec::new());
    for name in ["Ann Reed", "Andy Shaw", "Finance"] {
        assert!(buttons(&output).iter().any(|b| b == name), "{name} should be offered; saw {:?}", buttons(&output));
    }

    // Three recipients, picked one after another. Each lands as a chip whose
    // whole body is the control that takes it back again.
    h.click("Ann Reed");
    h.type_and_settle("an");
    fake.answer(lookup(vec![person("andy", "Andy Shaw")]));
    h.frame(Vec::new());
    h.click("Andy Shaw");
    h.type_and_settle("fin");
    fake.answer(lookup(vec![group("finance", "Finance")]));
    h.frame(Vec::new());
    h.click("Finance");

    let output = h.frame(Vec::new());
    for name in ["Remove Ann Reed", "Remove Andy Shaw", "Remove Finance"] {
        assert!(buttons(&output).iter().any(|b| b == name), "{name} should be a chip; saw {:?}", buttons(&output));
    }
}

#[test]
fn a_chip_takes_one_recipient_back_and_leaves_the_others() {
    let (mut h, fake) = Harness::everything();
    h.open();
    for (query, found) in [("an", person("ann", "Ann Reed")), ("bo", person("bob", "Bob Vine")), ("ca", person("cal", "Cal Nye"))] {
        h.type_and_settle(query);
        fake.answer(lookup(vec![found.clone()]));
        h.frame(Vec::new());
        h.click(&found.name);
    }
    h.typing = false;

    h.click("Remove Bob Vine");
    let output = h.frame(Vec::new());
    assert!(buttons(&output).iter().any(|b| b == "Remove Ann Reed"));
    assert!(buttons(&output).iter().any(|b| b == "Remove Cal Nye"));
    assert!(!buttons(&output).iter().any(|b| b == "Remove Bob Vine"), "the removed chip is gone; saw {:?}", buttons(&output));

    // And the share that follows carries the two that are left.
    h.click(SEND);
    let parked = fake.parked();
    let Some(Request::Grant(grant)) = parked.first() else { panic!("expected a share, saw {parked:?}") };
    assert_eq!(grant.targets, vec![ShareTarget::User("ann".into()), ShareTarget::User("cal".into())]);
}

#[test]
fn an_answer_the_user_has_typed_past_never_reaches_the_list() {
    let (mut h, fake) = Harness::everything();
    h.open();

    // Two bursts of typing, so two lookups are outstanding at once.
    h.type_and_settle("an");
    h.frame(Vec::new());
    h.type_and_settle("ann");
    h.frame(Vec::new());
    assert_eq!(fake.parked().len(), 2);

    // The newer one answers first and the older one arrives late, which is
    // what a service under load does.
    fake.answer_nth(1, lookup(vec![person("ann", "Ann Reed")]));
    h.frame(Vec::new());
    fake.answer_nth(0, lookup(vec![person("andy", "Andy Shaw"), person("anna", "Anna Beck")]));
    let output = h.frame(Vec::new());

    assert!(buttons(&output).iter().any(|b| b == "Ann Reed"), "the current answer stays");
    assert!(!says(&output, "Andy Shaw"), "the stale answer must not flicker in");
    assert!(!says(&output, "Anna Beck"));
}

#[test]
fn one_letter_is_not_worth_asking_about() {
    let (mut h, fake) = Harness::everything();
    h.open();
    h.type_and_settle("a");
    h.frame(Vec::new());
    assert!(fake.parked().is_empty(), "a single letter matches most of an address book");

    h.frame(vec![Event::Text("n".to_owned())]);
    h.wait(0.25);
    h.frame(Vec::new());
    assert_eq!(fake.parked(), vec![Request::Lookup { query: "an".into(), limit: 8 }]);
}

#[test]
fn nothing_is_asked_until_the_typing_stops() {
    let (mut h, fake) = Harness::everything();
    h.open();
    h.typing = true;
    // Five keystrokes inside the debounce window.
    for letter in ["a", "n", "n", "i", "e"] {
        h.frame(vec![Event::Text(letter.to_owned())]);
        h.wait(0.03);
    }
    assert!(fake.parked().is_empty(), "nothing asked while the user is still typing");

    h.wait(0.25);
    h.frame(Vec::new());
    assert_eq!(fake.parked(), vec![Request::Lookup { query: "annie".into(), limit: 8 }], "one question for the whole word");
}

// ---- what a share says ----------------------------------------------------

#[test]
fn sharing_with_five_where_one_address_is_wrong_names_the_one_that_failed() {
    let (mut h, fake) = Harness::everything();
    h.open();
    for (query, found) in [("an", person("ann", "Ann Reed")), ("bo", person("bob", "Bob Vine"))] {
        h.type_and_settle(query);
        fake.answer(lookup(vec![found.clone()]));
        h.frame(Vec::new());
        h.click(&found.name);
    }
    h.typing = false;
    h.click(SEND);

    fake.answer(Answer::Granted {
        shared: vec![existing("s1", ShareTarget::User("ann".into()), Permission::View)],
        failed: vec![Failure {
            target: ShareTarget::User("bob".into()),
            code: "invalid_address".into(),
            detail: "bob@example.con was rejected".into(),
        }],
    });
    let output = h.frame(Vec::new());

    assert!(says(&output, "Shared with"), "the ones that went through are named");
    assert!(says(&output, "ann"));
    assert!(says(&output, "Not shared with"), "and so is the one that did not");
    assert!(says(&output, "bob@example.con was rejected"), "with the reason, not a count; saw {:?}", text(&output));

    // The failed recipient stays in the picker so a correction is one edit.
    assert!(buttons(&output).iter().any(|b| b == "Remove Bob Vine"));
    assert!(!buttons(&output).iter().any(|b| b == "Remove Ann Reed"), "the one that worked has left");
}

#[test]
fn a_refusal_that_stopped_everything_is_shown_as_itself() {
    let (mut h, fake) = Harness::everything();
    h.open();
    h.type_and_settle("an");
    fake.answer(lookup(vec![person("ann", "Ann Reed")]));
    h.frame(Vec::new());
    h.click("Ann Reed");
    h.typing = false;
    h.click(SEND);

    let Some((_, reply)) = fake.take(0) else { panic!("expected a share in flight") };
    reply.refuse("sign in again");
    let output = h.frame(Vec::new());

    assert!(says(&output, "sign in again"));
    assert!(!says(&output, "Not shared with"), "a refusal is not a per-recipient failure");
    assert!(control(&output, SEND).is_some(), "the button comes back");
}

#[test]
fn a_note_and_a_permission_travel_with_the_share() {
    let (mut h, fake) = Harness::everything();
    h.open();
    h.type_and_settle("an");
    fake.answer(lookup(vec![person("ann", "Ann Reed")]));
    h.frame(Vec::new());
    h.click("Ann Reed");
    h.typing = false;
    // The permission is a menu now, so it takes opening before choosing.
    h.click(PERMISSION);
    h.click("Edit");
    h.click(SEND);

    let parked = fake.parked();
    let Some(Request::Grant(grant)) = parked.first() else { panic!("expected a share, saw {parked:?}") };
    assert_eq!(grant.permission, Permission::Edit);
    assert_eq!(grant.id, DOCUMENT, "the handle goes back untouched");
    assert!(grant.notify, "mail works here, and the box is ticked by default");
    assert_eq!(grant.note, None, "an empty note is no note");
}

#[test]
fn a_server_that_cannot_send_mail_is_never_promised_a_notification() {
    let (mut h, fake) = Harness::new(vec![Capability::Sharing, Capability::Contacts], Vec::new());
    h.open();
    h.type_and_settle("an");
    fake.answer(lookup(vec![person("ann", "Ann Reed")]));
    h.frame(Vec::new());
    h.click("Ann Reed");
    h.typing = false;
    h.click(SEND);

    let parked = fake.parked();
    let Some(Request::Grant(grant)) = parked.first() else { panic!("expected a share, saw {parked:?}") };
    assert!(!grant.notify, "a checkbox for mail a server cannot send must not be honoured");
}

#[test]
fn a_server_without_contacts_asks_nobody_anything() {
    let (mut h, fake) = Harness::new(vec![Capability::Sharing], Vec::new());
    h.open();
    h.type_and_settle("ann");
    h.frame(Vec::new());
    assert!(fake.parked().is_empty(), "there is no address book to search");
}

// ---- who already has access ----------------------------------------------

#[test]
fn who_has_access_is_listed_and_one_can_be_withdrawn() {
    let (mut h, fake) = Harness::new(
        vec![Capability::Sharing, Capability::Contacts, Capability::Mail],
        vec![
            existing("s1", ShareTarget::User("ann".into()), Permission::Edit),
            existing("s2", ShareTarget::Group("finance".into()), Permission::View),
        ],
    );
    h.open();
    let output = h.frame(Vec::new());
    assert!(says(&output, "Who has access"));
    assert!(says(&output, "ann"));
    assert!(says(&output, "finance (group)"));
    assert_eq!(buttons(&output).iter().filter(|b| *b == "Withdraw").count(), 2);

    h.click("Withdraw");
    assert!(fake.seen().iter().any(|r| matches!(r, Request::Withdraw { share_id } if share_id == "s1" || share_id == "s2")));

    // It stays on the list until the server agrees it is gone.
    let output = h.frame(Vec::new());
    assert!(says(&output, "Withdrawing…"));
    let Some((Request::Withdraw { share_id }, reply)) = fake.take(0) else { panic!("expected a withdrawal in flight") };
    reply.answer(Answer::Withdrawn { share_id: share_id.clone() });
    let output = h.frame(Vec::new());
    assert_eq!(buttons(&output).iter().filter(|b| *b == "Withdraw").count(), 1, "one left");
}

#[test]
fn a_document_nobody_has_been_given_says_so() {
    let (mut h, _fake) = Harness::everything();
    h.open();
    let output = h.frame(Vec::new());
    assert!(says(&output, "Nobody else yet"));
}

// ---- what the dialog refuses to offer -------------------------------------

#[test]
fn a_server_without_sharing_draws_no_dialog_at_all() {
    let (mut h, fake) = Harness::new(vec![Capability::Files, Capability::Contacts], Vec::new());
    assert!(!h.dialog.available());
    h.dialog.open(DOCUMENT, "Budget 2026.xlsx");
    assert!(!h.dialog.is_open(), "a feature the provider lacks is hidden, not offered");
    let output = h.frame(Vec::new());
    assert!(buttons(&output).is_empty(), "nothing to click; saw {:?}", buttons(&output));
    assert!(!fake.seen().iter().any(|r| matches!(r, Request::Access { .. })), "and nothing was asked about the document");
}

#[test]
fn an_unsaved_document_is_told_to_save_rather_than_refused_by_the_server() {
    let (mut h, fake) = Harness::everything();
    h.dialog.open("", "Book1");
    let output = h.frame(Vec::new());
    assert!(says(&output, "Save this document somewhere to share it."));
    assert!(control(&output, SEND).is_none(), "there is nothing to share yet");
    assert!(!fake.seen().iter().any(|r| matches!(r, Request::Access { .. })));
}

#[test]
fn closing_puts_the_dialog_away() {
    let (mut h, _fake) = Harness::everything();
    h.open();
    h.click("Close");
    assert!(!h.dialog.is_open());
    let output = h.frame(Vec::new());
    assert!(buttons(&output).is_empty());
}

#[test]
fn a_typed_address_says_how_to_add_it_and_adding_it_works() {
    let (mut h, fake) = Harness::everything();
    h.open();
    h.type_and_settle("outside@example.com");
    fake.answer(lookup(Vec::new()));
    let output = h.frame(Vec::new());
    assert!(says(&output, "Press Return to share with this address"), "saw {:?}", text(&output));
    assert!(!says(&output, "No matches"), "an address that matched nobody is still a recipient");

    // Return takes it, and the share carries it as an address.
    h.frame(vec![key(egui::Key::Enter)]);
    h.frame(Vec::new());
    h.typing = false;
    let output = h.frame(Vec::new());
    assert!(buttons(&output).iter().any(|b| b == "Remove outside@example.com"), "saw {:?}", buttons(&output));

    h.click(SEND);
    let parked = fake.parked();
    let Some(Request::Grant(grant)) = parked.first() else { panic!("expected a share, saw {parked:?}") };
    assert_eq!(grant.targets, vec![ShareTarget::Email("outside@example.com".into())]);
}

#[test]
fn a_name_that_matched_nobody_is_not_offered_as_an_address() {
    let (mut h, fake) = Harness::everything();
    h.open();
    h.type_and_settle("nobody");
    fake.answer(lookup(Vec::new()));
    let output = h.frame(Vec::new());
    assert!(says(&output, "No matches"));
    assert!(!says(&output, "Press Return"));
}
