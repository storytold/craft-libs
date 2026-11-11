//! The integration binary of craft-font-manager: every test is a registered trial.

mod missing;
mod panel;

use craft_font_manager::testing::registered;

fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &registered());
}
