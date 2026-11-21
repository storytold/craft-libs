//! About's Contributors tab (the grab bag of name links and the Table view) and the hosted dialog's `tab` field.

use craft_ui::contract::{Bind, Role};
use craft_ui::dialog::{DialogHost, StackDialog as _};
use craft_ui::patterns::form_dialogs::{AboutDialog, AboutInfo, AboutLink, AboutTab, Contributor, Credits, HostedAbout, ModelCredit};
use craft_ui::testing::{Fixture, trial};
use egui::Vec2;
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::LazyLock;

static INFO: LazyLock<AboutInfo> = LazyLock::new(|| AboutInfo::new("PhotoCraft", "An image editor.", "1.0"));
static CREDITS: LazyLock<Credits> = LazyLock::new(|| {
    Credits::new(
        vec![
            Contributor::new("alice").profile("https://github.com/alice").counts(1, 12),
            Contributor::new("bob").profile("https://github.com/bob").counts(3, 40),
        ],
        Vec::new(),
        52,
    )
});

#[trial]
fn icon_links_leave_room_for_their_visible_labels() {
    let info = AboutInfo::new("App", "An app", "1").links([
        AboutLink::new("A much longer community link", "https://example.org").icon(craft_behavior::IconId::new("globe")),
        AboutLink::new("Go", "https://example.org").icon(craft_behavior::IconId::new("globe")),
    ]);
    let mut fixture = Fixture::new(move |ui| {
        let _ = AboutDialog::new(&info, &CREDITS).show(ui);
    });
    fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
    let long = fixture.by_label("A much longer community link").expect("long link").rect;
    let short = fixture.by_label("Go").expect("short link").rect;
    assert!(long.width() > short.width(), "the icon link must show its label");
}

#[trial]
fn a_click_on_a_name_in_the_grab_bag_reports_its_profile() {
    let links = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&links);
    let mut fixture = Fixture::new(move |ui| {
        let shown = AboutDialog::new(&INFO, &CREDITS).tab(Bind::given(AboutTab::Contributors)).show(ui);
        seen.borrow_mut().extend(shown.link);
    });
    fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
    assert!(fixture.by_role(Role::TABLE).is_empty(), "the tab opens on the grab bag");
    let bob = fixture.by_label("@bob").expect("the grab bag lists bob");
    fixture.click(&bob).idle(4);
    assert_eq!(links.borrow().as_slice(), ["https://github.com/bob".to_owned()], "one click reports bob's profile");
    let table = fixture.by_label("Table").expect("the View control offers Table");
    fixture.click(&table).idle(10);
    assert!(!fixture.by_role(Role::TABLE).is_empty(), "Table shows the contributors as a table");
}

/// The names of the grab bag, in the order it shows them.
fn grab_bag(fixture: &Fixture) -> Vec<String> {
    fixture.by_role(Role::LINK).iter().filter_map(|link| fixture.tree().name_of(link.node).map(str::to_owned)).collect()
}

/// Picks `entry` from the Sort list.
fn sort_by(fixture: &mut Fixture, entry: &str) {
    let sort = fixture.by_label("Sort").expect("the Sort list");
    fixture.click(&sort).idle(4);
    let pick = fixture.by_label(entry).unwrap_or_else(|| panic!("the Sort list offers {entry}"));
    fixture.click(&pick).idle(10);
}

// The name order ignores the `@` of a username and the case, so display names and usernames interleave; someone
// with no commit date sorts after everyone with one, by first and by last commit. A slip reorders the credits.
#[trial]
fn contributors_sort_by_name_without_at_or_case_and_undated_last() {
    let credits = Credits::new(
        vec![
            Contributor::new("carl").counts(0, 9).active("2025-01-01T10:00:00Z", "2026-01-01T10:00:00Z"),
            Contributor::new("zed").display_name("Bea").counts(1, 0),
            Contributor::new("alice").counts(0, 3).active("2025-03-01T10:00:00Z", "2025-04-01T10:00:00Z"),
        ],
        Vec::new(),
        12,
    );
    let mut fixture = Fixture::new(move |ui| {
        let _ = AboutDialog::new(&INFO, &credits).tab(Bind::given(AboutTab::Contributors)).show(ui);
    });
    fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
    assert_eq!(grab_bag(&fixture), ["@carl", "@alice", "@zed"], "oldest first commit first, the undated last");

    sort_by(&mut fixture, "Last commit");
    assert_eq!(grab_bag(&fixture), ["@alice", "@carl", "@zed"], "oldest last commit first, the undated last");

    let display = fixture.by_label("Display name").expect("Show offers Display name");
    fixture.click(&display).idle(10);
    sort_by(&mut fixture, "Name (A\u{2013}Z)");
    assert_eq!(grab_bag(&fixture), ["@alice", "Bea", "@carl"], "names compare without the `@` and the case");
}

#[trial]
fn about_tab_is_a_field() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(Vec2::new(1400.0, 1000.0));
    let models = vec![ModelCredit::new("Acme", "Writer", "2").work(4, 100, 10)];
    let info = AboutInfo::new("PhotoCraft", "An image editor.", "1.0");
    let id = host.borrow_mut().open(HostedAbout::new(info, Credits::new(Vec::new(), models, 4)));
    fixture.idle(10);
    assert!(fixture.by_role(Role::TABLE).is_empty(), "the dialog opens on the About tab");

    let ctx = fixture.ctx().clone();
    host.borrow_mut().set_json_field(&ctx, id, "tab", json!("models")).expect("`tab` takes a tab's name");
    fixture.idle(4);
    assert_eq!(fixture.by_role(Role::TABLE).len(), 1, "the Models table shows once `tab` is set");
    let refused = host.borrow_mut().set_json_field(&ctx, id, "tab", json!("credits"));
    assert!(refused.is_err_and(|message| message.contains("can't take")), "a name that is no tab is refused");

    let contributors = fixture.by_label("Contributors").expect("the tab strip offers Contributors");
    fixture.click(&contributors).idle(4);
    let fields = host.borrow().get::<HostedAbout>().and_then(|about| about.json_fields());
    assert_eq!(fields, Some(json!({ "tab": "contributors" })), "the field lists the tab a click picked");
}
