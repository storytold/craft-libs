//! The integration binary of craft-table: every test is a registered trial.

mod grid;
mod outline;
mod refpick;

use craft_table::ui::testing::registered;

fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &registered());
}
