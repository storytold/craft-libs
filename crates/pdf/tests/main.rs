//! Integration tests, built into the library's own test executable.

mod cancellation;
mod contracts;
#[path = "cos/main.rs"]
mod cos;
#[path = "crypt/main.rs"]
mod crypt;
#[path = "filters/main.rs"]
mod filters;
mod font_tables;
mod image_default_spaces;
mod interpretation;
mod knockout;
mod knockout_backdrop;
mod plates;
mod print_color;
mod settings;
mod testing_pdf;
mod unicode_writing;
mod upstream_compatibility;
mod upstream_raster;
mod writing;
