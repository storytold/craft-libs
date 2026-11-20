//! Catalog names across the linked library crates.

/// Guards against a registered type that no row places (it would list under no level) and against names a
/// viewer cannot show as one identifier or that two types share (the tree would list two rows alike).
#[craft_ui::testing::trial]
fn catalog_names_are_unique_pascal_case_and_every_type_has_a_level() {
    let registry = craft_ui::testing::Registry::of(&[craft_ui::testing::register, craft_ui_test_kit::testing::register]);
    let linked: Vec<_> =
        registry.registrations().iter().filter(|registration| registration.crate_target() == registration.crate_name().replace('-', "_")).collect();
    let catalog = craft_ui::contract::catalog(&linked);
    assert!(catalog.unleveled.is_empty(), "types with no level: {:?}", catalog.unleveled);
    let mut seen = std::collections::HashSet::new();
    for entry in &catalog.entries {
        let mut chars = entry.name.chars();
        assert!(
            chars.next().is_some_and(|first| first.is_ascii_uppercase()) && chars.all(|c| c.is_ascii_alphanumeric()),
            "`{}` ({}) is not a PascalCase identifier",
            entry.name,
            entry.type_name
        );
        assert!(seen.insert((entry.level, entry.name.to_lowercase())), "`{}` is listed twice in {:?}", entry.name, entry.level);
    }
}
