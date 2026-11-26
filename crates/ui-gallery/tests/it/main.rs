//! The viewer's integration tests: one binary, run by `libtest-mimic` over the registered trials.

pub(crate) mod mock;
pub(crate) mod overview;
pub(crate) mod panels;
pub(crate) mod registry;
pub(crate) mod viewer;

craft_ui::registry!();

fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &craft_ui::testing::Registry::of(&[register, craft_ui_gallery::register_all]));
}
