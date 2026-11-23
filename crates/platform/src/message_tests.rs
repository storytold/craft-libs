//! The messages' registry.
use crate::texts;

// Every `PlatformText` constant of a module is in its `TEXTS` table (counted in the module's source), ids are `module.camelCase` and one id has one text.
#[test]
fn every_message_is_in_the_registry_once() {
    let sources: Vec<(&str, &str, usize)> = vec![
        ("open", include_str!("open.rs"), crate::open::TEXTS.len()),
        ("watch", include_str!("watch.rs"), crate::watch::TEXTS.len()),
        ("temp", include_str!("temp.rs"), crate::temp::TEXTS.len()),
        ("default_app", include_str!("default_app.rs"), crate::default_app::TEXTS.len()),
        #[cfg(feature = "clipboard")]
        ("clipboard", include_str!("clipboard/types.rs"), crate::clipboard::TEXTS.len()),
    ];
    for (module, source, listed) in sources {
        let declared = source.lines().filter(|l| l.starts_with("pub const ") && l.contains(": PlatformText") && !l.contains("TEXTS")).count();
        assert_eq!(declared, listed, "{module}: constants and TEXTS differ");
    }
    let all = texts();
    let mut seen = std::collections::HashMap::new();
    for text in &all {
        let (module, name) = text.id.split_once('.').unwrap_or(("", ""));
        let camel = |s: &str| s.chars().next().is_some_and(|c| c.is_ascii_lowercase()) && s.chars().all(|c| c.is_ascii_alphanumeric());
        assert!(camel(module) && camel(name), "{:?} is not module.camelCase", text.id);
        assert!(seen.insert(text.key(), text.english).is_none(), "{:?} twice", text.id);
    }
    assert!(all.len() >= 15);
}
