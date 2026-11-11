//! The integration binary of craft-ui-shell: every test is a registered trial.

// The test files name the library as `craft_ui`, as the crates that use it do.
extern crate craft_ui_shell as craft_ui;

pub(crate) mod body;
mod commands;
mod contract;
mod dialog;
mod measure;
mod notices;

mod shell;

use craft_ui::testing::registered;

fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &registered());
}
