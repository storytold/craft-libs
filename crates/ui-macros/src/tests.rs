//! The registration gate the component derive reads from the package manifest.

use proc_macro2::TokenStream;

#[test]
fn registration_is_gated_only_when_the_package_declares_suite() {
    let compact = |tokens: TokenStream| tokens.to_string().split_whitespace().collect::<String>();
    let declared = "[package]\nname = \"app\"\n\n[features]\nsuite = [\"craft-ui/testing\"]\n";
    assert_eq!(compact(crate::component::gate_for(declared)), "#[cfg(feature=\"suite\")]");
    let undeclared = "[package]\nname = \"gallery\"\nsuite = 1\n\n[dependencies]\nsuite = \"1\"\n";
    assert_eq!(compact(crate::component::gate_for(undeclared)), "");
    assert_eq!(compact(crate::component::gate_for("")), "");
}
