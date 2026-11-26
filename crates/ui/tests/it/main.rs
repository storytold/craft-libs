//! The integration binary of craft-ui: every test is a registered trial.

pub(crate) mod api_map;
pub(crate) mod canvas;
pub(crate) mod components_index;
pub(crate) mod form_dialogs;

pub(crate) mod shell;

craft_ui::registry!();

fn main() {
    craft_ui::testing::run(
        module_path!(),
        env!("CARGO_PKG_NAME"),
        &craft_ui::testing::Registry::of(&[register, craft_ui::testing::register_library]),
    );
}
