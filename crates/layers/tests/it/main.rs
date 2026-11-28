//! The integration binary of craft-layers: every test is a registered trial.

pub(crate) mod blend_rows;
pub(crate) mod comps;
pub(crate) mod panel_drag_menu;
pub(crate) mod panel_eye;
pub(crate) mod panel_filter;
pub(crate) mod panel_input;
pub(crate) mod panel_memo;
pub(crate) mod panel_order;
pub(crate) mod panel_reveal;
pub(crate) mod panel_rows;

craft_ui::registry!();

fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &craft_ui::testing::Registry::of(&[register, craft_layers::testing::register]));
}
