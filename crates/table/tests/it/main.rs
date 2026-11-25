//! The integration binary of craft-table: every test is a registered trial.

pub(crate) mod conditions;
pub(crate) mod grid;
pub(crate) mod outline;
pub(crate) mod refpick;
pub(crate) mod slicer;

craft_ui::registry!();

fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &craft_ui::testing::Registry::of(&[register, craft_table::ui::testing::register]));
}
