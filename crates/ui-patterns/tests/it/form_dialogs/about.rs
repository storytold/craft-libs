//! About's Contributors tab (the grab bag of name links and the Table view) and the hosted dialog's `tab` field.

use craft_ui::contract::{Bind, Role};
use craft_ui::dialog::{DialogHost, StackDialog as _};
use craft_ui::patterns::form_dialogs::{AboutDialog, AboutInfo, AboutTab, Contributor, Credits, HostedAbout, ModelCredit};
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
