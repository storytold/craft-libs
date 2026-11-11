//! The integration binary of craft-ui-canvas: every test is a registered trial.

// The test files name the library as `craft_ui`, as the crates that use it do.
extern crate craft_ui_canvas as craft_ui;

mod canvas;
mod canvas_controls;
mod contract;
// full-window parts
mod full_window_parts;
// end of full-window parts

use craft_ui::testing::registered;

fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &registered());
}
