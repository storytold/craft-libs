//! The integration binary of craft-ui-panels: every test is a registered trial.

// The test files name the library as `craft_ui`, as the crates that use it do.
extern crate craft_ui_panels as craft_ui;

pub(crate) mod access;
pub(crate) mod arrange;
pub(crate) mod body;
pub(crate) mod dialogs;
pub(crate) mod dock_group;
pub(crate) mod docking;
// outer edit
pub(crate) mod edit;
// end of outer edit
pub(crate) mod focus;
pub(crate) mod internals;
pub(crate) mod list;
pub(crate) mod popup_surface;

pub(crate) mod rails;
pub(crate) mod sections;
pub(crate) mod story_shapes;

craft_ui::registry!();

fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &craft_ui::testing::Registry::of(&[register, craft_ui::testing::register]));
}
