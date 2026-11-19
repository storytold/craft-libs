//! The registration gate the component derive reads from the package manifest.

#[test]
fn registration_is_gated_only_when_the_package_declares_suite() {
    let declared = "[package]\nname = \"app\"\n\n[features]\nsuite = [\"craft-ui/testing\"]\n";
    assert!(crate::component::declares_suite_in(declared));
    let undeclared = "[package]\nname = \"gallery\"\nsuite = 1\n\n[dependencies]\nsuite = \"1\"\n";
    assert!(!crate::component::declares_suite_in(undeclared));
    assert!(!crate::component::declares_suite_in(""));
}
