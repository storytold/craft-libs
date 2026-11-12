//! The integration binary of craft-layers: every test is a registered trial.

mod blend_rows;
mod panel_drag_menu;
mod panel_filter;
mod panel_input;
mod panel_order;
mod panel_reveal;
mod panel_rows;

fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &craft_layers::testing::registered());
}
