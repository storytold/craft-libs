//! The viewer's integration tests: one binary, run by `libtest-mimic` over the registered trials.

mod overview;
mod viewer;

use craft_ui_gallery::testing::registered;

fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &registered());
}
