//! The app's pages share the Keyboard Shortcuts dialog's width and navigation.

use craft_host::target::Os;
use craft_keymap::{Platform, Profiles};
use craft_menu::shortcuts::{Area, DialogState, DialogTab, ShortcutsDialog};
use craft_theme::ThemeId;
use craft_ui::contract::Role;
use craft_ui::runtime::egui::access::AccessTree;
use craft_ui::runtime::env::RootEnv;
use craft_ui::testing::{Fixture, Frames, pointer, trial};

use super::fixture;

fn dialog_at_125_percent(toolbar_label: &str) -> (Frames, impl FnMut(&mut egui::Ui) + '_) {
    // Use the Windows test context and a 1600 × 1000 physical-pixel window at 125%.
    let mut os = Fixture::new(|_| {});
    os.os(Os::Windows);
    let mut frames = Frames::over(os.ctx().clone());
    frames.screen(egui::vec2(1280.0, 800.0));
    frames.pixels_per_point(1.25);
    let mut root = RootEnv::default();
    root.set_theme(ThemeId::parse("craft.artcraft").unwrap());
    frames.root(root);

    let mut keys = fixture::keymap(Platform::Windows);
    let mut state = DialogState::new();
    let mut profiles = Profiles::default();
    let builtins = fixture::builtins();
    let options = fixture::options().with(|o| {
        o.title = Some("Keyboard Shortcuts and Menus".into());
        o.areas = Area::photoshop();
    });
    let show = move |ui: &mut egui::Ui| {
        let mut tabs = [
            DialogTab::new("menus", "Menus", |ui| {
                ui.label("Menu item visibility");
                false
            }),
            DialogTab::new("toolbar", toolbar_label, |ui| {
                ui.label("Toolbar tool visibility");
                false
            }),
        ];
        ShortcutsDialog::new(&mut keys).sets(&mut profiles, &builtins).options(&options).tabs(&mut tabs).show(ui, &mut state);
    };
    (frames, show)
}

fn tab_bounds(tree: &AccessTree, label: &str) -> egui::Rect {
    let tab = tree.keys().into_iter().find(|&key| tree.role_of(key) == Some(Role::TAB) && tree.name_of(key) == Some(label));
    tab.and_then(|key| tree.bounds_of(key)).unwrap_or_else(|| panic!("no tab labelled {label}"))
}

#[trial]
fn app_tabs_show_full_labels_at_125_percent_and_switch_pages() {
    let (mut frames, mut show) = dialog_at_125_percent("Toolbar");
    frames.idle(12, &mut show);
    assert_eq!(frames.ctx.pixels_per_point(), 1.25);
    let tree = frames.tree();
    let tabs = ["Keyboard Shortcuts", "Menus", "Toolbar"].map(|label| {
        let rect = tab_bounds(&tree, label);
        let text = frames.texts().iter().find(|text| text.text == label).unwrap_or_else(|| panic!("no painted label {label}"));
        assert!(!text.elided && text.clip.contains_rect(text.rect), "{label} is clipped: {text:?}");
        rect
    });
    assert!(tabs.windows(2).all(|pair| pair[0].right() <= pair[1].left()));

    for (label, page) in [("Menus", "Menu item visibility"), ("Toolbar", "Toolbar tool visibility")] {
        // A page can change the dialog's bounds, so target the tab in the current frame.
        frames.idle(2, &mut show);
        let tab = tab_bounds(&frames.tree(), label);
        pointer::click(&mut frames, tab.center(), &mut show);
        assert!(frames.texts().iter().any(|text| text.text == page), "click should show {page}");
    }
}

#[trial]
fn app_tabs_keep_overflow_when_the_labels_do_not_fit() {
    let long_label = "Toolbar ".repeat(200);
    let (mut frames, mut show) = dialog_at_125_percent(&long_label);
    frames.idle(12, &mut show);
    let tree = frames.tree();
    let overflow = tree.keys().into_iter().find(|&key| tree.role_of(key) == Some(Role::BUTTON) && tree.name_of(key) == Some("More tabs"));
    let overflow = overflow.and_then(|key| tree.bounds_of(key)).expect("oversized titles still have an overflow menu");
    pointer::click(&mut frames, overflow.center(), &mut show);
    let popup = frames.tree();
    let hidden = popup.keys().into_iter().find(|&key| popup.name_of(key) == Some(long_label.as_str()));
    let hidden = hidden.and_then(|key| popup.bounds_of(key)).expect("the hidden toolbar remains available");
    pointer::click(&mut frames, hidden.center(), &mut show);
    frames.idle(2, &mut show);
    assert!(frames.texts().iter().any(|text| text.text == "Toolbar tool visibility"), "the overflow choice opens the Toolbar page");
}
