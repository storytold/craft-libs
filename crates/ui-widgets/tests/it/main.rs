//! The integration binary of craft-ui-widgets: every test is a registered trial.

// The test files name the library as `craft_ui`, as the crates that use it do.
extern crate craft_ui_widgets as craft_ui;

pub(crate) mod access;
pub(crate) mod bars;
pub(crate) mod body;
pub(crate) mod buttons;
pub(crate) mod checkbox_sweep;
pub(crate) mod commands;
pub(crate) mod contract;
pub(crate) mod disabled_fade;
pub(crate) mod field_text_centred;
pub(crate) mod focus_ring;
pub(crate) mod form;
pub(crate) mod keys;
pub(crate) mod label;
pub(crate) mod layer;
pub(crate) mod layout;
pub(crate) mod link;
pub(crate) mod list;
pub(crate) mod menu;
pub(crate) mod notices;
pub(crate) mod overlay;
pub(crate) mod part_rings;
pub(crate) mod pickers;
pub(crate) mod picture;
pub(crate) mod popover;
pub(crate) mod runtime;

pub(crate) mod suite_rtl;
pub(crate) mod suite_rules;
pub(crate) mod suite_title;
pub(crate) mod table;
pub(crate) mod tabs;
pub(crate) mod text;
pub(crate) mod tile_grid;
pub(crate) mod value_drag;
pub(crate) mod values;

craft_ui::registry!();

fn main() {
    craft_ui::testing::run(
        module_path!(),
        env!("CARGO_PKG_NAME"),
        &craft_ui::testing::Registry::of(&[register, craft_ui::testing::register, craft_ui_test_kit::testing::register]),
    );
}
