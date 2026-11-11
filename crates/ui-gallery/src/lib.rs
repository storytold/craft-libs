//! The craft-ui story viewer: the registered stories of the linked crates, shown under a chosen appearance.

pub mod chrome;
pub mod viewer;

// The theme workspace.
pub mod theme;

// The mock editor window.
pub mod mock;

/// The gallery window's default size, in points.
pub const WINDOW: [f32; 2] = [1280.0, 800.0];

/// The registrations of every crate whose stories the viewer shows, by crate.
///
/// Each entry is a function of its crate that is passed by pointer, so the linker keeps that crate's object files and
/// the story registrations in them, which a release build for the web would otherwise drop.
type Registered = fn() -> Vec<&'static craft_ui::testing::Registration>;

fn crates() -> Vec<(&'static str, Registered)> {
    #[allow(unused_mut, clippy::vec_init_then_push)]
    let mut crates: Vec<(&'static str, Registered)> = Vec::new();
    #[cfg(feature = "automation")]
    crates.push(("craft-automation", craft_automation::testing::registered));
    #[cfg(feature = "browse")]
    crates.push(("craft-browse", craft_browse::ui::testing::registered));
    #[cfg(feature = "color")]
    crates.push(("craft-color", craft_color::ui::testing::registered));
    #[cfg(feature = "draw")]
    crates.push(("craft-draw", craft_draw::testing::registered));
    #[cfg(feature = "imaging")]
    crates.push(("craft-imaging", craft_imaging::testing::registered));
    #[cfg(feature = "layers")]
    crates.push(("craft-layers", craft_layers::testing::registered));
    #[cfg(feature = "menu")]
    crates.push(("craft-menu", craft_menu::testing::registered));
    #[cfg(feature = "pages")]
    crates.push(("craft-pages", craft_pages::testing::registered));
    #[cfg(feature = "print")]
    crates.push(("craft-print", craft_print::testing::registered));
    #[cfg(feature = "review")]
    crates.push(("craft-review", craft_review::testing::registered));
    #[cfg(feature = "table")]
    crates.push(("craft-table", craft_table::ui::testing::registered));
    #[cfg(feature = "text")]
    crates.push(("craft-text", craft_text::testing::registered));
    #[cfg(feature = "theme-store")]
    crates.push(("craft-theme-store", craft_theme_store::ui::registered));
    #[cfg(feature = "timeline")]
    crates.push(("craft-timeline", craft_timeline::testing::registered));
    // These do not build for the web, so their stories are in the native gallery only.
    #[cfg(all(feature = "dialogs", not(target_arch = "wasm32")))]
    crates.push(("craft-dialogs", craft_dialogs::testing::registered));
    #[cfg(all(feature = "font-manager", not(target_arch = "wasm32")))]
    crates.push(("craft-font-manager", craft_font_manager::testing::registered));
    #[cfg(all(feature = "update", not(target_arch = "wasm32")))]
    crates.push(("craft-update", craft_update::testing::registered));
    std::hint::black_box(crates)
}

/// How many registrations each linked crate contributes, with the viewer's own, and how many stories there are in all.
pub fn linked_counts() -> (Vec<(&'static str, usize)>, usize) {
    let mut counts: Vec<(&'static str, usize)> = crates().into_iter().map(|(name, registered)| (name, registered().len())).collect();
    counts.push((env!("CARGO_PKG_NAME"), testing::registered().len()));
    (counts, craft_ui::testing::stories().len())
}

/// The test registry of this package's stories.
pub mod testing {
    craft_ui::testing_registered!();
}
