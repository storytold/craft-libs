//! The integration binary of craft-ui: every test is a registered trial.

mod api_map;
mod canvas;
mod components_index;
mod form_dialogs;

mod shell;

use craft_ui::testing::registered;

fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &registered());
}
