//! The integration binary of craft-font-manager: every test is a registered trial.

pub(crate) mod glyphs;
pub(crate) mod missing;
pub(crate) mod panel;
pub(crate) mod picker;

craft_ui::registry!();

fn main() {
    craft_ui::testing::run(
        module_path!(),
        env!("CARGO_PKG_NAME"),
        &craft_ui::testing::Registry::of(&[register, craft_font_manager::testing::register]),
    );
}
