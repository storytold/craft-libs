//! The integration binary of craft-ui-canvas: every test is a registered trial.

// The test files name the library as `craft_ui`, as the crates that use it do.
extern crate craft_ui_canvas as craft_ui;

pub(crate) mod canvas;
pub(crate) mod canvas_controls;
pub(crate) mod contract;
// full-window parts
pub(crate) mod full_window_parts;
// end of full-window parts

craft_ui::registry!();

fn main() {
    craft_ui::testing::run(
        module_path!(),
        env!("CARGO_PKG_NAME"),
        &craft_ui::testing::Registry::of(&[register, craft_ui::testing::register, craft_ui_test_kit::testing::register]),
    );
}
