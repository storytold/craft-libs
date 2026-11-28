use super::*;
use std::path::PathBuf;

#[test]
fn decodes_latin2_and_utf8() {
    let aff = b"SET ISO8859-2\nTRY a\n";
    let (a, d) = decode(aff, b"2\n\xbf\xf3\xb3w\n").unwrap();
    assert!(a.contains("TRY a"));
    assert_eq!(d, "2\nżółw\n");
    let (_, d) = decode("SET UTF-8\n".as_bytes(), "żółw".as_bytes()).unwrap();
    assert_eq!(d, "żółw");
    let (_, d) = decode(b"SET ISO8859-9\n", b"\xd0\xdd\xde\xf0\xfd\xfe\xd6\xe7").unwrap();
    assert_eq!(d, "ĞİŞğışÖç");
    assert!(decode(b"SET SHIFT_JIS\n", b"").is_err());
    assert!(decode(b"SET UTF-8\n", b"\xff\xfe").is_err());
}

#[test]
fn utf8_dictionaries_keep_single_byte_flags() {
    // The Hungarian dictionary is `SET UTF-8` but writes flags as single bytes above 0x7F,
    // which aren't UTF-8; each byte is one flag. Its `NAME` line is ISO 8859-2.
    let aff = b"SET UTF-8\nNAME Magyar helyes\xedr\xe1si\nAF 2\nAF \xd5a # 1\nAF \xe9 # 2\nSFX \xd5 Y 1\nSFX \xd5 0 ok/2 . 7\nSFX \xe9 Y 1\nSFX \xe9 0 ban .\nSFX a Y 1\nSFX a 0 at .\n";
    let dic = "2\nház/1\t1\nkert/\u{e9}\n".as_bytes();
    let (a, d) = decode(aff, dic).unwrap();
    assert!(a.contains("AF \u{d5}a # 1") && a.contains("AF \u{e9} # 2") && a.contains("SFX \u{d5} 0 ok/2 . 7"));
    assert!(d.contains("kert/\u{c3}\u{a9}"), "a UTF-8 é in the flags is two one-byte flags");
    let dict = crate::affix::AffixDict::from_hunspell(&a, &d, &[]).unwrap();
    assert!(dict.check("ház") && dict.check("házok") && dict.check("házokban") && dict.check("házat"));
    assert!(!dict.check("házban"));
    // `FLAG UTF-8` keeps the whole file UTF-8, and words must still be UTF-8.
    let (_, d) = decode(b"SET UTF-8\nFLAG UTF-8\n", "kert/\u{e9}\n".as_bytes()).unwrap();
    assert_eq!(d, "kert/\u{e9}\n");
    assert!(decode(b"SET UTF-8\nSFX a Y 1\nSFX a 0 \xe9 .\n", b"").is_err(), "an affix must be UTF-8");
    assert!(decode(b"SET UTF-8\n", b"1\nh\xe1z/a\n").is_err(), "a stem must be UTF-8");
}

/// An environment for the search functions that read none of the process's own.
fn env_of(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<std::ffi::OsString> {
    move |name| vars.iter().find(|(k, _)| *k == name).map(|(_, v)| std::ffi::OsString::from(*v))
}

fn paths(dirs: &[PathBuf]) -> Vec<&str> {
    dirs.iter().filter_map(|d| d.to_str()).collect()
}

/// A host names its own folders: the one it set, the folder its environment variable holds, then
/// its own folders, in that order, and an empty variable is no folder.
#[test]
fn a_host_names_its_own_variable_and_folders() {
    let env = env_of(&[("HOST_DICTIONARIES", "/from/the/variable")]);
    let host = HostFolders { env_var: Some("HOST_DICTIONARIES".into()), folders: vec!["/own/one".into(), "/own/two".into()] };
    let tier = first_tier(Some(Path::new("/set/by/the/host")), &host, &env);
    assert_eq!(paths(&tier), ["/set/by/the/host", "/from/the/variable", "/own/one", "/own/two"]);
    let empty = env_of(&[("HOST_DICTIONARIES", "")]);
    assert_eq!(paths(&first_tier(None, &host, &empty)), ["/own/one", "/own/two"], "an empty variable names no folder");
    let unset = env_of(&[]);
    assert_eq!(paths(&first_tier(None, &host, &unset)), ["/own/one", "/own/two"]);
}

/// A host that names nothing searches no application's folder, whatever the environment holds:
/// the names of the app this crate moved out of are not built in.
#[test]
fn another_apps_names_are_not_searched_unless_the_host_names_them() {
    let env = env_of(&[
        ("WORDCRAFT_DICTIONARIES", "/other/app/variable"),
        ("HOME", "/home/u"),
        ("XDG_DATA_HOME", "/home/u/data"),
        ("LOCALAPPDATA", "/local"),
        ("APPDATA", "/roaming"),
    ]);
    assert!(first_tier(None, &HostFolders::default(), &env).is_empty());
    assert_eq!(paths(&first_tier(Some(Path::new("/set")), &HostFolders::default(), &env)), ["/set"]);
    // Naming them searches them.
    let host = HostFolders { env_var: Some("WORDCRAFT_DICTIONARIES".into()), folders: vec![] };
    assert_eq!(paths(&first_tier(None, &host, &env)), ["/other/app/variable"]);
}

/// The conventional per-user folders of an application, by system.
#[test]
fn an_apps_conventional_folders_follow_each_systems_layout() {
    let env = env_of(&[("HOME", "/home/u"), ("XDG_DATA_HOME", "/home/u/data"), ("LOCALAPPDATA", "/local"), ("APPDATA", "/roaming")]);
    let join = |base: &str, rest: &[&str]| rest.iter().fold(PathBuf::from(base), |p, r| p.join(r));
    assert_eq!(
        app_folders_for(Os::Windows, "Example", &env),
        [join("/local", &["Example", "dictionaries"]), join("/roaming", &["Example", "dictionaries"])]
    );
    assert_eq!(app_folders_for(Os::Mac, "Example", &env), [join("/home/u", &["Library/Application Support", "Example", "dictionaries"])]);
    assert_eq!(app_folders_for(Os::Other, "Example", &env), [join("/home/u/data", &["example", "dictionaries"])]);
    // A relative XDG_DATA_HOME is ignored, as the XDG Base Directory specification says.
    let relative = env_of(&[("HOME", "/home/u"), ("XDG_DATA_HOME", "data")]);
    assert_eq!(app_folders_for(Os::Other, "Example", &relative), [join("/home/u", &[".local/share", "example", "dictionaries"])]);
    // No home, no folder; a name that is a path is no application name.
    assert!(app_folders_for(Os::Other, "Example", &env_of(&[])).is_empty());
    for bad in ["", "..", "a/b", "a\\b", "."] {
        assert!(app_folders_for(Os::Other, bad, &env).is_empty(), "{bad:?}");
    }
}
