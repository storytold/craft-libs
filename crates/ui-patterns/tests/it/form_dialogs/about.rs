//! About's Contributors tab (the grab bag of name links and the Table view and its order), the hosted dialog's `tab`
//! field and the app's own tabs.

use craft_ui::contract::{Bind, Role};
use craft_ui::dialog::{DialogHost, StackDialog as _};
use craft_ui::patterns::form_dialogs::{
    AboutAppTab, AboutDialog, AboutInfo, AboutLayout, AboutLicense, AboutLink, AboutTab, Contributor, Credits, HostedAbout, ModelCredit,
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

// The centred layout (the default) sets its blocks apart: the gap from the version to the community button, from the
// button to the link row and from the link row to the built-with line is wider than the one from the blurb to the
// version. The button and the link row stand centred in the dialog, also when the dialog is wider than its lines.
#[trial]
fn the_centred_about_sets_its_blocks_apart_and_centres_the_button_and_links() {
    let info = AboutInfo::new("PhotoCraft", "An image editor.", "1.0")
        .community(AboutLink::new("Chat", "https://chat.example"))
        .links([AboutLink::new("Website", "https://example.org"), AboutLink::new("Source", "https://example.org/source")])
        .built_with("MIT");
    let mut fixture = Fixture::new(move |ui| {
        let _ = AboutDialog::new(&info, &CREDITS).show(ui);
    });
    fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
    let [chat, website, source] = ["Chat", "Website", "Source"].map(|name| fixture.by_label(name).expect("every link shows").rect);
    let painted = |text: &str| fixture.texts().iter().find(|painted| painted.text == text).map(|painted| painted.rect).expect("the text shows");
    let (blurb, version, caption) = (painted("An image editor."), painted("Version 1.0"), painted("MIT"));
    let within = version.top() - blurb.bottom();
    for (gap, what) in [
        (chat.top() - version.bottom(), "the button stands apart from the version"),
        (website.top() - chat.bottom(), "the links stand apart from the button"),
        (caption.top() - source.bottom(), "the built-with line stands apart from the links"),
    ] {
        assert!(gap > within + 1.0, "{what}: {gap} against {within}");
    }
    // Centred across the body: the button and the link row share one middle, the dialog's.
    let middle = fixture.by_role(Role::DIALOG).first().expect("the dialog shows").rect.center().x;
    let links = website.union(source);
    assert!((chat.center().x - links.center().x).abs() < 1.0, "the button and the link row share a middle: {chat:?}, {links:?}");
    assert!((chat.center().x - middle).abs() < 1.0, "the button is centred: {} against {middle}", chat.center().x);
    assert!(blurb.left() < chat.left() - 10.0, "the blurb still starts at the start edge");
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

/// The names of the grab bag, in the order it shows them.
fn grab_bag(fixture: &Fixture) -> Vec<String> {
    fixture.by_role(Role::LINK).iter().filter_map(|link| fixture.tree().name_of(link.node).map(str::to_owned)).collect()
}

/// Picks `entry` from the Sort list.
fn pick_sort(fixture: &mut Fixture, entry: &str) {
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

    pick_sort(&mut fixture, "Last commit");
    assert_eq!(grab_bag(&fixture), ["@alice", "@carl", "@zed"], "oldest last commit first, the undated last");

    let display = fixture.by_label("Display name").expect("Show offers Display name");
    fixture.click(&display).idle(10);
    pick_sort(&mut fixture, "Name (A\u{2013}Z)");
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

/// The tab titles in the strip, in order.
fn tab_titles(fixture: &Fixture) -> Vec<String> {
    fixture.by_role(Role::TAB).iter().filter_map(|tab| access::name(fixture.tree(), tab.node).map(str::to_owned)).collect()
}

/// Whether a painted run of text holds `text`.
fn painted(fixture: &Fixture, text: &str) -> bool {
    fixture.texts().iter().any(|painted| painted.text.contains(text))
}

/// Guards the Licenses tab: it follows Models and comes before an app tab, it opens on the first license's text, and
/// the License picker shows the license picked in it in place of the first.
#[trial]
fn the_licenses_tab_follows_models_and_shows_the_picked_license() {
    let licenses = [AboutLicense::new("MIT License", "Permission is hereby granted"), AboutLicense::new("Font License", "Reserved Font Name")];
    let mut app = [AboutAppTab::new("languageData", "Languages", |_| {})];
    let mut fixture = Fixture::new(move |ui| {
        let _ = AboutDialog::new(&INFO, &CREDITS).tab(Bind::given(AboutTab::Licenses)).licenses(&licenses).app_tabs(&mut app).show(ui);
    });
    fixture.room(Vec2::new(1400.0, 1000.0)).idle(30);
    assert_eq!(tab_titles(&fixture), ["About", "Contributors", "Models", "Licenses", "Languages"], "Licenses follows Models");
    assert!(painted(&fixture, "Permission is hereby granted"), "the tab opens on the first license");
    assert!(!painted(&fixture, "Reserved Font Name"), "one license shows at a time");

    let picker =
        fixture.by_role(Role::COMBO_BOX).into_iter().find(|target| fixture.tree().name_of(target.node) == Some("License")).expect("the picker");
    fixture.click(&picker).idle(4);
    let font = fixture.by_label("Font License").expect("the picker lists the second license");
    fixture.click(&font).idle(10);
    assert!(painted(&fixture, "Reserved Font Name"), "the picked license shows");
    assert!(!painted(&fixture, "Permission is hereby granted"), "in place of the first");
}

/// Guards the Licenses tab's absence: without licenses the strip has no Licenses tab and the hosted dialog's `tab`
/// field refuses its name; with them `tab` takes the name, shows the text without a picker for one license, and lists
/// it back.
#[trial]
fn the_licenses_tab_shows_only_with_licenses_and_is_a_tab_field_value() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(Vec2::new(1400.0, 1000.0));
    let info = AboutInfo::new("PhotoCraft", "An image editor.", "1.0");
    let id = host.borrow_mut().open(HostedAbout::new(info.clone(), CREDITS.clone()));
    fixture.idle(10);
    assert!(!tab_titles(&fixture).iter().any(|title| title == "Licenses"), "no Licenses tab without licenses");
    let ctx = fixture.ctx().clone();
    assert!(host.borrow_mut().set_json_field(&ctx, id, "tab", json!("licenses")).is_err(), "`tab` refuses Licenses without licenses");
    host.borrow_mut().close_by_id(id);
    fixture.idle(4);

    let licensed = HostedAbout::new(info, CREDITS.clone()).licenses([AboutLicense::new("MIT License", "Permission is hereby granted")]);
    let id = host.borrow_mut().open(licensed);
    fixture.idle(10);
    assert!(tab_titles(&fixture).iter().any(|title| title == "Licenses"), "the Licenses tab shows with licenses");
    host.borrow_mut().set_json_field(&ctx, id, "tab", json!("licenses")).expect("`tab` takes the Licenses tab's name");
    fixture.idle(4);
    assert!(painted(&fixture, "Permission is hereby granted"), "setting `tab` shows the license");
    assert!(fixture.by_role(Role::COMBO_BOX).is_empty(), "one license needs no picker");
    let fields = host.borrow().get::<HostedAbout>().and_then(|about| about.json_fields());
    assert_eq!(fields, Some(json!({ "tab": "licenses" })));
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

/// A crowd of contributors whose grab bag is taller than a small window.
static CROWD: LazyLock<Credits> = LazyLock::new(|| {
    let people: Vec<Contributor> = (0..400).map(|at| Contributor::new(format!("contributor-number-{at}")).counts(1, 1)).collect();
    Credits::new(people, Vec::new(), 400)
});

/// photocraft#16326: in a small window the Contributors tab, whose grab bag fills the room it is offered, keeps the
/// About dialog inside the window, and back on About the dialog stands centred again, as it opened.
#[trial]
fn about_stays_inside_a_small_window_on_contributors_and_comes_back_to_the_middle() {
    let host = Rc::new(RefCell::new(DialogHost::new()));
    let shown = Rc::clone(&host);
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.room(Vec2::new(1200.0, 700.0));
    let id = host.borrow_mut().open(HostedAbout::new(AboutInfo::new("PhotoCraft", "An image editor.", "1.0"), CROWD.clone()));
    fixture.settle();
    let screen = fixture.ctx().content_rect();
    let dialog = |fixture: &Fixture| fixture.by_role(Role::DIALOG).first().expect("the dialog shows").rect;
    let opened = dialog(&fixture);
    assert!((opened.center().y - screen.center().y).abs() < 1.0, "About opens centred: {opened:?} in {screen:?}");

    let ctx = fixture.ctx().clone();
    host.borrow_mut().set_json_field(&ctx, id, "tab", json!("contributors")).expect("`tab` takes Contributors");
    fixture.settle();
    let tall = dialog(&fixture);
    assert!(screen.contains_rect(tall), "Contributors keeps the dialog inside the window: {tall:?} in {screen:?}");
    let ok = fixture.by_label("OK").expect("the footer's OK").rect;
    assert!(tall.bottom() - ok.bottom() < 20.0, "OK is the dialog's last row: {ok:?} in {tall:?}");
    assert!(tall.height() <= 0.9 * screen.height() + 1.0, "the dialog keeps to its share of the window: {tall:?} in {screen:?}");

    host.borrow_mut().set_json_field(&ctx, id, "tab", json!("about")).expect("`tab` takes About");
    fixture.settle();
    let back = dialog(&fixture);
    assert!((back.center().y - screen.center().y).abs() < 1.0, "back on About the dialog is centred again: {back:?} in {screen:?}");
}
