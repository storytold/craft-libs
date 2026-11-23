//! The integration binary of craft-ui-contract: every test is a registered trial.

// The test files name the library as `craft_ui`, as the crates that use it do.
extern crate craft_ui_contract as craft_ui;

pub(crate) mod access;
pub(crate) mod body;
pub(crate) mod canvas;
pub(crate) mod collection;
pub(crate) mod contract;
pub(crate) mod drag;
pub(crate) mod egui_surface;
pub(crate) mod fonts;
pub(crate) mod icon_ids;
pub(crate) mod keys_consumed;
pub(crate) mod layout;
pub(crate) mod motion;
pub(crate) mod paint;

pub(crate) mod runtime;

pub(crate) mod story_command_subsets;
pub(crate) mod style;
pub(crate) mod text;
pub(crate) mod textures;

craft_ui::registry!();

fn main() {
    craft_ui::testing::run(
        module_path!(),
        env!("CARGO_PKG_NAME"),
        &craft_ui::testing::Registry::of(&[
            register,
            craft_ui::testing::register,
            craft_ui_test_kit::testing::register,
            contract::visited_type::register_fixture,
        ]),
    );
}
