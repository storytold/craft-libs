//! The registration gate the component derive reads from the package manifest.

use proc_macro2::TokenStream;

#[test]
fn registration_is_gated_only_when_the_package_declares_screen_testing() {
    let compact = |tokens: TokenStream| tokens.to_string().split_whitespace().collect::<String>();
    let declared = "[package]\nname = \"app\"\n\n[features]\nscreen-testing = [\"craft-ui/testing\"]\n";
    assert_eq!(compact(crate::component::gate_for(declared)), "#[cfg(feature=\"screen-testing\")]");
    let undeclared = "[package]\nname = \"gallery\"\nscreen-testing = 1\n\n[dependencies]\nscreen-testing = \"1\"\n";
    assert_eq!(compact(crate::component::gate_for(undeclared)), "");
    assert_eq!(compact(crate::component::gate_for("")), "");
}
