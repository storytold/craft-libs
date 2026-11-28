//! The screen tests of craft-theme-store, folded into the lib's test target.

use craft_theme::builtins;
use craft_theme_store::ui::AppearancePage;
use craft_theme_store::{SelectionChange, SuiteClient};
use craft_ui::contract::Bind;
use craft_ui::testing::{Fixture, trial};

/// The page over a memory suite with the first built-in theme a favourite; `keep_only` leaves out favourites, Size and
/// Contrast, as an app with its own appearance setting does.
fn page(keep_only: bool) -> Fixture {
    let favorites = vec![builtins()[0].id.clone()];
    let mut draft = SelectionChange::default();
    let mut fixture = Fixture::new(move |ui| {
        let client = SuiteClient::memory().view();
        let page = AppearancePage::new(&client, Bind::bound(&mut draft)).favorites(&favorites);
        let page = if keep_only { page.can_favorite(false).size(false).contrast(false) } else { page };
        page.show(ui);
    });
    fixture.settle();
    fixture
}

// An app that keeps its own appearance shows only what it keeps: no Size or Contrast row and no favourite star, while
// the mode, theme and Motion rows stay; the full page has them all.
#[trial]
fn an_app_with_its_own_appearance_shows_only_the_rows_it_keeps() {
    let star = format!("Remove from Favorites: {}", builtins()[0].name);
    let full = page(false);
    for label in ["Size", "Contrast", star.as_str()] {
        assert!(full.by_label(label).is_some(), "the full page has {label}");
    }
    let kept = page(true);
    for label in ["Size", "Contrast", star.as_str()] {
        assert!(kept.by_label(label).is_none(), "the kept page leaves out {label}");
    }
    for label in ["Appearance Mode", "Light Theme", "Dark Theme", "Motion"] {
        assert!(kept.by_label(label).is_some(), "the kept page keeps {label}");
    }
}
