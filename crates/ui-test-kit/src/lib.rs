//! The test kits the craft-ui crates' integration binaries share: components written with the derives, and a
//! frame driver for child viewports. Their registrations are this package's, so no library's suite runs them.

// The kits name the library as `craft_ui`, as the test files do.
extern crate craft_ui_contract as craft_ui;

pub mod body;
pub mod contract;
pub mod windows;
