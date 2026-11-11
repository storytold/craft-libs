//! The integration binary of craft-menu: every test is a registered trial.

mod layout;
mod native;
mod sample;
mod shortcuts;

fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &craft_menu::testing::registered());
}
