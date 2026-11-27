//! The integration binary of craft-ui-patterns: every test is a registered trial.

// The test files name the library as `craft_ui`, as the crates that use it do.
extern crate craft_ui_patterns as craft_ui;

pub(crate) mod blend_keys;
pub(crate) mod docking;
pub(crate) mod form_dialogs;

// Panels family.
pub(crate) mod panels;

pub(crate) mod preferences;
pub(crate) mod presets;

craft_ui::registry!();

fn main() {
    craft_ui::testing::run(
        module_path!(),
        env!("CARGO_PKG_NAME"),
        &craft_ui::testing::Registry::of(&[register, craft_ui::testing::register, craft_ui_test_kit::testing::register]),
    );
}
