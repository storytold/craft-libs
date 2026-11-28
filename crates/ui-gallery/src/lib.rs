//! The craft-ui story viewer: the registered stories of the linked crates, shown under a chosen appearance.
#![warn(missing_docs)]

pub mod chrome;
pub mod viewer;

// The Foundations cards.
pub mod foundations;

// The mock editor window.
pub mod mock;

/// The gallery window's default size, in points.
pub const WINDOW: [f32; 2] = [1280.0, 800.0];

/// The registrations of every crate whose stories the viewer shows, by crate.
///
/// Each entry is a function of its crate that is passed by pointer, so the linker keeps that crate's object files and
/// the story registrations in them, which a release build for the web would otherwise drop.
type Registered = fn(&mut craft_ui::testing::Registry);

#[allow(unused_mut, clippy::vec_init_then_push)]
fn crates() -> Vec<(&'static str, Registered)> {
    let mut crates: Vec<(&'static str, Registered)> = Vec::new();
    #[cfg(feature = "automation")]
    crates.push(("craft-automation", craft_automation::testing::register));
    #[cfg(feature = "browse")]
    crates.push(("craft-browse", craft_browse::ui::testing::register));
    #[cfg(feature = "color-ui")]
    crates.push(("craft-color-ui", craft_color_ui::ui::testing::register));
    #[cfg(feature = "draw")]
    crates.push(("craft-draw", craft_draw::testing::register));
    #[cfg(feature = "imaging")]
    crates.push(("craft-imaging", craft_imaging::testing::register));
    #[cfg(feature = "layers")]
    crates.push(("craft-layers", craft_layers::testing::register));
    #[cfg(feature = "menu")]
    crates.push(("craft-menu", craft_menu::testing::register));
    #[cfg(feature = "pages")]
    crates.push(("craft-pages", craft_pages::testing::register));
    #[cfg(feature = "print")]
    crates.push(("craft-print", craft_print::testing::register));
    #[cfg(feature = "review")]
    crates.push(("craft-review", craft_review::testing::register));
    #[cfg(feature = "table")]
    crates.push(("craft-table", craft_table::ui::testing::register));
    #[cfg(feature = "text")]
    crates.push(("craft-text", craft_text::testing::register));
    #[cfg(feature = "theme-editor")]
    crates.push(("craft-theme-editor", craft_theme_editor::testing::register));
    #[cfg(feature = "theme-store")]
    crates.push(("craft-theme-store", craft_theme_store::ui::register));
    #[cfg(feature = "timeline")]
    crates.push(("craft-timeline", craft_timeline::testing::register));
    // These do not build for the web, so their stories are in the native gallery only.
    #[cfg(all(feature = "dialogs", not(target_arch = "wasm32")))]
    crates.push(("craft-dialogs", craft_dialogs::testing::register));
    #[cfg(all(feature = "font-manager", not(target_arch = "wasm32")))]
    crates.push(("craft-font-manager", craft_font_manager::testing::register));
    #[cfg(all(feature = "update", not(target_arch = "wasm32")))]
    crates.push(("craft-update", craft_update::testing::register));
    crates
}

/// Adds the viewer and each selected dependency to a registry.
pub fn register_all(registry: &mut craft_ui::testing::Registry) {
    craft_ui::testing::register_library(registry);
    testing::register(registry);
    for (_, register) in crates() {
        register(registry);
    }
}

/// The registry of the viewer and the crates whose stories it shows.
pub fn registry() -> craft_ui::testing::Registry {
    craft_ui::testing::Registry::of(&[register_all])
}

/// How many registrations each selected crate contributes, and how many stories there are in all.
pub fn linked_counts() -> (Vec<(&'static str, usize)>, usize) {
    let registry = registry();
    let mut counts: Vec<_> =
        crates().into_iter().map(|(name, _)| (name, registry.registrations().iter().filter(|entry| entry.crate_name() == name).count())).collect();
    counts.push((env!("CARGO_PKG_NAME"), registry.registrations().iter().filter(|entry| entry.crate_name() == env!("CARGO_PKG_NAME")).count()));
    (counts, registry.stories().len())
}

/// The test registry of this package's stories.
pub mod testing {
    craft_ui::registry!();
}
