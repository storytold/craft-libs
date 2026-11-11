//! The integration binary of craft-ui-panels: every test is a registered trial.

// The test files name the library as `craft_ui`, as the crates that use it do.
extern crate craft_ui_panels as craft_ui;

mod access;
pub(crate) mod body;
mod dialogs;
// outer edit
mod edit;
// end of outer edit
mod focus;
mod internals;
mod list;

mod rails;
mod sections;

use craft_ui::testing::registered;

fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &registered());
}
