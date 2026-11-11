//! The integration binary of craft-layers: every test is a registered trial.

mod panel_input;
mod panel_order;

fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &craft_layers::testing::registered());
}
