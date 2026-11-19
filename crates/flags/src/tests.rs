use super::*;

#[test]
fn every_flag_is_off_by_default_and_has_a_valid_unique_id() {
    let flags = Flags::default();
    for f in ALL {
        assert!(!f.default, "{} must ship off", f.id);
        assert!(!flags.enabled(f));
        assert!(valid_id(f.id), "{}", f.id);
        assert!(!f.title.is_empty() && !f.description.is_empty());
        assert_eq!(ALL.iter().filter(|g| g.id == f.id).count(), 1, "{} twice", f.id);
    }
}

#[test]
fn the_file_turns_flags_on_and_off() {
    let flags = Flags::from_parts(Some("# mine\n[flags]\nAI-MCP = true\nAI-MODELS = false\n"), None);
    assert!(flags.enabled(&AI_MCP));
    assert!(!flags.enabled(&AI_MODELS));
    assert!(flags.problems.is_empty(), "{:?}", flags.problems);
}

#[test]
fn the_environment_wins_over_the_file() {
    let file = Some("[flags]\nAI-MCP = true\n");
    let flags = Flags::from_parts(file, Some("-AI-MCP, AI-MODELS"));
    assert!(!flags.enabled(&AI_MCP));
    assert!(flags.enabled(&AI_MODELS));
    assert_eq!(flags.forced("AI-MCP"), Some(false));
    let flags = Flags::from_parts(None, Some("AI-MCP=1;AI-MODELS=off"));
    assert!(flags.enabled(&AI_MCP));
    assert!(!flags.enabled(&AI_MODELS));
    assert_eq!(flags.forced("OTHER"), None);
}

#[test]
fn bad_entries_are_reported_and_left_out_never_fatal() {
    let flags = Flags::from_parts(Some("[flags]\nAI-MCP = \"yes\"\nlower = true\nAI-MODELS = true\n"), Some("AI-MCP=maybe,bad id,"));
    assert!(!flags.enabled(&AI_MCP), "a value that is not true or false is not on");
    assert!(flags.enabled(&AI_MODELS));
    assert_eq!(flags.problems.len(), 5, "{:?}", flags.problems); // "bad id" is two words

    for text in ["not toml [[[", "flags = 3", "\u{feff}[flags]\nAI-MCP = true", ""] {
        let flags = Flags::from_parts(Some(text), None);
        assert!(!flags.enabled(&AI_MCP) || text.contains("AI-MCP = true"), "{text:?}");
    }
    assert!(!Flags::from_parts(Some("not toml [[["), None).problems.is_empty());
    assert!(!Flags::from_parts(Some("flags = 3"), None).problems.is_empty());
}

#[test]
fn oversized_text_is_not_read() {
    let mut text = String::from("[flags]\nAI-MCP = true\n");
    text.push_str(&"#".repeat(MAX_FILE_BYTES as usize));
    let flags = Flags::from_parts(Some(&text), None);
    assert!(!flags.enabled(&AI_MCP));
    assert!(flags.problems.iter().any(|p| p.contains("larger than")));
}

#[test]
fn many_entries_are_capped() {
    let mut text = String::from("[flags]\n");
    for i in 0..(MAX_ENTRIES + 10) {
        text.push_str(&format!("F{i} = true\n"));
    }
    let flags = Flags::from_parts(Some(&text), None);
    assert_eq!(flags.file.len(), MAX_ENTRIES);
    assert!(flags.problems.iter().any(|p| p.contains("more than")));
}

#[test]
fn unknown_ids_are_off_unless_set() {
    let flags = Flags::from_parts(Some("[flags]\nNEW-THING = true\n"), None);
    assert!(flags.enabled_id("NEW-THING"));
    assert!(!flags.enabled_id("OTHER-THING"));
    assert!(!flags.enabled_id("AI-MCP"));
}

#[test]
fn ids_are_checked() {
    assert!(valid_id("AI-MCP"));
    assert!(!valid_id(""));
    assert!(!valid_id("ai-mcp"));
    assert!(!valid_id("AI MCP"));
    assert!(!valid_id(&"A".repeat(MAX_ID_LEN + 1)));
}

#[cfg(not(target_arch = "wasm32"))]
mod file {
    use super::super::*;
    use crate::store;

    #[test]
    fn set_creates_the_file_and_keeps_what_else_is_in_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ArtCraft").join(FILE_NAME);
        set(&path, "AI-MCP", true).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(Flags::from_parts(Some(&text), None).enabled(&AI_MCP));

        std::fs::write(&path, "# keep me\nother = 1\n[flags]\nAI-MCP = true # on for testing\nX-OLD = false\n").unwrap();
        set(&path, "AI-MODELS", true).unwrap();
        set(&path, "AI-MCP", false).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# keep me") && text.contains("other = 1") && text.contains("X-OLD = false"), "{text}");
        let flags = Flags::from_parts(Some(&text), None);
        assert!(!flags.enabled(&AI_MCP));
        assert!(flags.enabled(&AI_MODELS));
    }

    #[test]
    fn set_keeps_the_comment_after_a_flag() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        std::fs::write(&path, "[flags]\nAI-MCP = true # on for testing\n").unwrap();
        set(&path, "AI-MCP", false).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("AI-MCP = false # on for testing"), "{text}");
    }

    #[test]
    fn set_refuses_to_write_what_could_not_be_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        // Readable (just under the limit), so the refusal comes from the size of what `set` would
        // write, not from reading.
        let mut text = String::from("[flags]\n# ");
        let pad = MAX_FILE_BYTES as usize - 4 - text.len();
        text.push_str(&"x".repeat(pad));
        assert_eq!(text.len() as u64, MAX_FILE_BYTES - 4);
        std::fs::write(&path, &text).unwrap();
        assert!(Flags::from_parts(Some(&text), None).problems.is_empty(), "the fixture itself reads");
        assert!(matches!(set(&path, "AI-MODELS", true), Err(FlagError::Unreadable(e)) if e.contains("would grow")));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "left as it was");
    }

    #[test]
    fn a_missing_file_is_mentioned_only_when_named() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.toml");
        let named = store::load_from(Some(missing.clone()), true, None);
        assert!(named.problems.iter().any(|p| p.contains("does not exist")), "{:?}", named.problems);
        assert!(store::load_from(Some(missing), false, None).problems.is_empty(), "the usual file not existing yet is normal");
    }

    #[test]
    fn two_writers_keep_both_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let ids: Vec<String> = (0..8).map(|i| format!("F{i}")).collect();
        std::thread::scope(|s| {
            for id in &ids {
                let path = &path;
                s.spawn(move || set(path, id, true).unwrap());
            }
        });
        let flags = Flags::from_parts(Some(&std::fs::read_to_string(&path).unwrap()), None);
        for id in &ids {
            assert!(flags.enabled_id(id), "{id} lost");
        }
    }

    #[test]
    fn set_never_overwrites_a_file_it_cannot_read() {
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in [("broken.toml", "[[[ not toml"), ("scalar.toml", "flags = 3\n")] {
            let path = dir.path().join(name);
            std::fs::write(&path, text).unwrap();
            assert!(matches!(set(&path, "AI-MCP", true), Err(FlagError::Unreadable(_))), "{name}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "{name} changed");
        }
        let big = dir.path().join("big.toml");
        std::fs::write(&big, vec![b'#'; MAX_FILE_BYTES as usize + 1]).unwrap();
        assert!(matches!(set(&big, "AI-MCP", true), Err(FlagError::Unreadable(_))));
        let folder = dir.path().join("folder.toml");
        std::fs::create_dir(&folder).unwrap();
        assert!(set(&folder, "AI-MCP", true).is_err());
        assert!(matches!(set(&dir.path().join("ok.toml"), "bad id", true), Err(FlagError::BadId(_))));
    }
}
