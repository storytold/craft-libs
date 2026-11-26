//! The integration binary of craft-ui-shell: every test is a registered trial.

// The test files name the library as `craft_ui`, as the crates that use it do.
extern crate craft_ui_shell as craft_ui;

pub(crate) mod body;
pub(crate) mod commands;
pub(crate) mod contract;
pub(crate) mod dialog;
pub(crate) mod measure;
pub(crate) mod notices;

pub(crate) mod shell;

craft_ui::registry!();

fn main() {
    craft_ui::testing::run(
        module_path!(),
        env!("CARGO_PKG_NAME"),
        &craft_ui::testing::Registry::of(&[register, craft_ui::testing::register, craft_ui_test_kit::testing::register]),
    );
}
