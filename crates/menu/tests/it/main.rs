//! The integration binary of craft-menu: every test is a registered trial.

pub(crate) mod layout;
pub(crate) mod mnemonic;
pub(crate) mod native;
mod sample;
pub(crate) mod shortcuts;

craft_ui::registry!();

fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &craft_ui::testing::Registry::of(&[register, craft_menu::testing::register]));
}
