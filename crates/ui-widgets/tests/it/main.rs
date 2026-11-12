//! The integration binary of craft-ui-widgets: every test is a registered trial.

// The test files name the library as `craft_ui`, as the crates that use it do.
extern crate craft_ui_widgets as craft_ui;

mod access;
mod bars;
pub(crate) mod body;
mod commands;
mod contract;
mod disabled_fade;
mod field_text_centred;
mod keys;
mod label;
mod layer;
mod layout;
mod link;
mod list;
mod menu;
mod notices;
mod overlay;

mod pickers;
mod popover;
mod runtime;

mod suite_rtl;
mod suite_rules;
mod suite_title;
mod table;
mod tabs;
mod text;
mod tile_grid;
mod value_drag;
mod values;

use craft_ui::testing::registered;

fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &registered());
}
