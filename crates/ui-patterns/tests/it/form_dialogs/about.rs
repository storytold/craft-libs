//! About's Contributors tab (the grab bag of name links and the Table view and its order), the hosted dialog's `tab`
//! field and the app's own tabs.

use craft_ui::contract::{Bind, Role};
use craft_ui::dialog::{DialogHost, StackDialog as _};
use craft_ui::patterns::form_dialogs::{
    AboutAppTab, AboutDialog, AboutInfo, AboutLayout, AboutLink, AboutTab, Contributor, Credits, HostedAbout, ModelCredit,
};
use craft_ui::testing::{Fixture, access, trial};
use egui::Vec2;
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};

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

// The listed layout stands the name, version, blurb, links and built-with line in one column from the start edge,
// the links in the community-then-links order, one to a line; a link click reports its URL and the button is Close.
#[trial]
fn the_listed_about_tab_stands_one_item_to_a_line_and_closes_with_close() {
    let info = AboutInfo::new("PhotoCraft", "An image editor.", "1.0")
        .community(AboutLink::new("Chat", "https://chat.example"))
        .links([AboutLink::new("Website", "https://example.org"), AboutLink::new("Source", "https://example.org/source")])
        .built_with("MIT");
    let links = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&links);
    let mut fixture = Fixture::new(move |ui| {
        let shown = AboutDialog::new(&info, &CREDITS).layout(AboutLayout::Listed).show(ui);
        seen.borrow_mut().extend(shown.link);
    });
    fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
    assert!(fixture.by_label("OK").is_none(), "no OK button");
    assert!(fixture.by_label("Close").is_some(), "Close closes the listed dialog");
    let [chat, website, source] = ["Chat", "Website", "Source"].map(|name| fixture.by_label(name).expect("every link shows").rect);
    let painted = |text: &str| fixture.texts().iter().find(|painted| painted.text == text).map(|painted| painted.rect).expect("the text shows");
    let (name, version) = (painted("PhotoCraft"), painted("Version 1.0"));
    assert!(name.bottom() <= version.top() && version.bottom() <= painted("An image editor.").top(), "name, version, blurb in order");
    assert!(chat.bottom() <= website.top() && website.bottom() <= source.top(), "one link to a line, community first");
    assert!((chat.left() - website.left()).abs() < 1.0 && (website.left() - name.left()).abs() < 1.0, "every line starts at one edge");
    assert!(source.bottom() <= painted("MIT").top(), "the built-with line comes last");
    let website = fixture.by_label("Website").expect("the website link");
    fixture.click(&website).idle(4);
    assert_eq!(links.borrow().as_slice(), ["https://example.org".to_owned()]);
}

// The listed layout sets its three blocks apart, as WordCraft's About: the gap from the blurb to the first link and
// from the last link to the built-with line is wider than the gap between two lines of one block.
#[trial]
fn the_listed_about_sets_its_blocks_apart() {
    let info = AboutInfo::new("PhotoCraft", "An image editor.", "1.0")
        .links([AboutLink::new("Website", "https://example.org"), AboutLink::new("Source", "https://example.org/source")])
        .built_with("MIT");
    let mut fixture = Fixture::new(move |ui| {
        let _ = AboutDialog::new(&info, &CREDITS).layout(AboutLayout::Listed).show(ui);
    });
    fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
    // The links' own rects (a link's padding is its own) against the painted lines of text around them; the gap
    // between two lines of one block is the one from the version to the blurb.
    let [website, source] = ["Website", "Source"].map(|name| fixture.by_label(name).expect("every link shows").rect);
    let painted = |text: &str| fixture.texts().iter().find(|painted| painted.text == text).map(|painted| painted.rect).expect("the text shows");
    let within = painted("An image editor.").top() - painted("Version 1.0").bottom();
    let after_blurb = website.top() - painted("An image editor.").bottom();
    let before_caption = painted("MIT").top() - source.bottom();
    assert!(after_blurb > within + 1.0, "the links stand apart from the blurb: {after_blurb} against {within}");
    assert!(before_caption > within + 1.0, "the built-with line stands apart from the links: {before_caption} against {within}");
}

/// The listed layout is a wide dialog (`LeastWidth::WideDialog`), as WordCraft's About: wider than the centred one
/// with the same text.
#[trial]
fn the_listed_about_is_a_wide_dialog() {
    let width = |layout: AboutLayout| {
        let mut fixture = Fixture::new(move |ui| {
            let info = AboutInfo::new("PhotoCraft", "An image editor.", "1.0");
            let _ = AboutDialog::new(&info, &CREDITS).layout(layout).show(ui);
        });
        fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
        fixture.by_role(Role::DIALOG).first().expect("the dialog shows").rect.width()
    };
    let (listed, centred) = (width(AboutLayout::Listed), width(AboutLayout::Centred));
    assert!(listed > centred + 200.0, "the listed About is a wide dialog: {listed} against {centred}");
}

/// The listed layout's tabs are tab faces, as WordCraft's About's strip: each title is set in from its tab's start edge.
/// The centred layout's are their text alone, starting at the tab's edge.
#[trial]
fn the_listed_about_shows_tab_faces() {
    let inset = |layout: AboutLayout| {
        let mut fixture = Fixture::new(move |ui| {
            let _ = AboutDialog::new(&INFO, &CREDITS).layout(layout).show(ui);
        });
        fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
        let tab = fixture.by_role(Role::TAB).into_iter().find(|tab| fixture.tree().name_of(tab.node) == Some("Contributors")).expect("the tab");
        let text = fixture.texts().iter().find(|painted| painted.text == "Contributors").map(|painted| painted.rect).expect("its title");
        text.left() - tab.rect.left()
    };
    let (listed, centred) = (inset(AboutLayout::Listed), inset(AboutLayout::Centred));
    assert!(centred.abs() < 1.0, "a centred tab is its text: {centred}");
    assert!(listed > 3.0, "a listed tab is a face with its title set in: {listed}");
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

/// Guards the app tabs of the About dialog: an app tab is listed after Models, `tab` takes its name and shows its
/// body, a click on it reports its name in the field, and a name that is neither built in nor an app tab's is refused.
#[trial]
fn an_app_tab_shows_after_models_and_is_a_tab_field_value() {
    let drawn = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&drawn);
    let languages = AboutAppTab::new("languageData", "Languages", move |_| {
        counted.fetch_add(1, Ordering::Relaxed);
    });
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(Vec2::new(1400.0, 1000.0));
    let info = AboutInfo::new("PhotoCraft", "An image editor.", "1.0");
    let id = host.borrow_mut().open(HostedAbout::new(info, CREDITS.clone()).app_tab(languages));
    fixture.idle(10);
    assert_eq!(drawn.load(Ordering::Relaxed), 0, "the dialog opens on About");

    let tabs: Vec<String> = fixture.by_role(Role::TAB).iter().filter_map(|tab| access::name(fixture.tree(), tab.node).map(str::to_owned)).collect();
    let models = tabs.iter().position(|name| name == "Models").expect("the strip lists Models");
    assert_eq!(tabs.get(models + 1).map(String::as_str), Some("Languages"), "the app tab follows Models: {tabs:?}");

    let ctx = fixture.ctx().clone();
    host.borrow_mut().set_json_field(&ctx, id, "tab", json!("languageData")).expect("`tab` takes the app tab's name");
    fixture.idle(4);
    assert!(drawn.load(Ordering::Relaxed) > 0, "setting `tab` to the app tab's name shows its body");

    host.borrow_mut().set_json_field(&ctx, id, "tab", json!("about")).expect("`tab` takes a built-in name");
    fixture.idle(4);
    let fields = host.borrow().get::<HostedAbout>().and_then(|about| about.json_fields());
    assert_eq!(fields, Some(json!({ "tab": "about" })));
    let app = fixture.by_label("Languages").expect("the strip offers the app tab");
    fixture.click(&app).idle(4);
    let fields = host.borrow().get::<HostedAbout>().and_then(|about| about.json_fields());
    assert_eq!(fields, Some(json!({ "tab": "languageData" })), "the field names the app tab a click picked");

    let refused = host.borrow_mut().set_json_field(&ctx, id, "tab", json!("languages"));
    assert!(refused.is_err(), "a name that is no tab's is refused");
}

/// The contributors of the order trial: a capitalised login with a display name, an undated (pull request only)
/// contributor, and one with a real name.
static PEOPLE: LazyLock<Credits> = LazyLock::new(|| {
    let dated = |login: &'static str, day: &'static str| Contributor::new(login).counts(0, 1).active(day, day);
    Credits::new(
        vec![
            dated("zed", "2026-01-02T00:00:00Z").lines(5, 50),
            dated("Bob", "2026-01-01T00:00:00Z").display_name("Ann").lines(10, 0),
            Contributor::new("alice").counts(1, 0).lines(7, 1),
            dated("carol", "2026-01-03T00:00:00Z").real_name("Dana").lines(1, 0),
        ],
        Vec::new(),
        3,
    )
});

/// The contributors' names among `names` in the order the table shows them, top to bottom.
fn rows(fixture: &Fixture, names: &[&str]) -> Vec<String> {
    let mut placed: Vec<(f32, &str)> =
        names.iter().map(|name| (fixture.by_label(name).expect("the table lists every contributor").rect.top(), *name)).collect();
    placed.sort_by(|a, b| a.0.total_cmp(&b.0));
    placed.into_iter().map(|(_, name)| name.to_owned()).collect()
}

/// A press on the column head `head`. The idle frames outlast the double-click time, so a second press of the same
/// head is a click of its own.
fn sort_by(fixture: &mut Fixture, head: &str) {
    let cell = fixture
        .by_role(Role::COLUMN_HEADER)
        .into_iter()
        .find(|cell| fixture.tree().name_of(cell.node) == Some(head))
        .expect("the table has the column");
    fixture.click(&cell).idle(30);
}

/// Guards the contributors' order: names compare without the `@` and without case, a display name sorts among the
/// usernames, and a contributor with no commit date follows everyone with one whichever way the dates sort.
#[trial]
fn names_sort_without_at_and_case_and_undated_contributors_sort_last_both_ways() {
    let mut fixture = Fixture::new(move |ui| {
        let _ = AboutDialog::new(&INFO, &PEOPLE).tab(Bind::given(AboutTab::Contributors)).show(ui);
    });
    fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
    let table = fixture.by_label("Table").expect("the View control offers Table");
    fixture.click(&table).idle(30);
    let logins = ["@zed", "@Bob", "@alice", "@carol"];

    assert_eq!(rows(&fixture, &logins), ["@Bob", "@zed", "@carol", "@alice"], "oldest first commit first, the undated last");
    sort_by(&mut fixture, "First");
    assert_eq!(rows(&fixture, &logins), ["@carol", "@zed", "@Bob", "@alice"], "newest first commit first, the undated still last");

    sort_by(&mut fixture, "Name");
    assert_eq!(rows(&fixture, &logins), ["@alice", "@Bob", "@carol", "@zed"], "A to Z without the @ and without case");
    sort_by(&mut fixture, "Name");
    assert_eq!(rows(&fixture, &logins), ["@zed", "@carol", "@Bob", "@alice"], "Z to A");

    let display = fixture.by_label("Display name").expect("the Show control offers Display name");
    fixture.click(&display).idle(30);
    sort_by(&mut fixture, "Name");
    assert_eq!(rows(&fixture, &["@zed", "Ann", "@alice", "@carol"]), ["@alice", "Ann", "@carol", "@zed"], "a display name sorts among the usernames");
}
