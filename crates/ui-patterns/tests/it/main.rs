//! The integration binary of craft-ui-patterns: every test is a registered trial.

// The test files name the library as `craft_ui`, as the crates that use it do.
extern crate craft_ui_patterns as craft_ui;

mod docking;
mod form_dialogs;

// Panels family.
mod panels;

mod preferences;
mod presets;

use craft_ui::testing::registered;

fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &registered());
}
