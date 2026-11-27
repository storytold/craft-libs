use super::{Repo, Version, parse_github_url, pick_asset};

fn repo(owner: &str, name: &str, version: Version) -> Repo {
    Repo { owner: owner.into(), name: name.into(), version }
}

#[test]
fn github_addresses_are_read() {
    for url in ["https://github.com/acme/fin-tools", "github.com/acme/fin-tools/", "http://www.github.com/acme/fin-tools.git", "acme/fin-tools"] {
        assert_eq!(parse_github_url(url), Ok(repo("acme", "fin-tools", Version::Latest)), "{url}");
    }
    assert_eq!(parse_github_url("https://github.com/acme/fin/releases/tag/v1.2.0"), Ok(repo("acme", "fin", Version::Release("v1.2.0".into()))));
    assert_eq!(parse_github_url("https://github.com/acme/fin/tree/feat/x"), Ok(repo("acme", "fin", Version::Tree("feat/x".into()))));
    assert_eq!(parse_github_url("https://github.com/acme/fin/releases/latest"), Ok(repo("acme", "fin", Version::Latest)));
    for bad in [
        "",
        "https://gitlab.com/acme/fin",
        "https://github.com/acme",
        "https://github.com/../fin",
        "https://github.com/acme/fin/blob/main/x",
        "https://github.com/acme/fin/tree/../../x",
        "ftp://github.com/acme/fin",
        "a/b/c",
    ] {
        assert!(parse_github_url(bad).is_err(), "{bad}");
    }
}

#[test]
fn the_release_zip_for_this_platform_is_picked() {
    let names: Vec<String> = [
        "fin-1.0-linux-x86_64.zip",
        "fin-1.0-macos-x86_64.zip",
        "fin-1.0-macos-aarch64.zip",
        "fin-1.0-windows-x64.zip",
        "fin-1.0-macos-aarch64.tar.gz",
        "checksums.txt",
    ]
    .map(String::from)
    .to_vec();
    assert_eq!(pick_asset(&names, "macos", "aarch64"), Some(2));
    assert_eq!(pick_asset(&names, "macos", "x86_64"), Some(1));
    assert_eq!(pick_asset(&names, "windows", "x86_64"), Some(3));
    assert_eq!(pick_asset(&names, "linux", "aarch64"), None, "no build for it: install the source");
    let universal: Vec<String> = ["fin-darwin-universal.zip", "fin-linux-amd64.zip"].map(String::from).to_vec();
    assert_eq!(pick_asset(&universal, "macos", "aarch64"), Some(0));
    let any: Vec<String> = ["fin-plugin.zip", "fin-linux-arm64.zip"].map(String::from).to_vec();
    assert_eq!(pick_asset(&any, "macos", "aarch64"), Some(0), "a zip naming no platform suits every one");
    assert_eq!(pick_asset(&any, "linux", "aarch64"), Some(1), "this platform's build beats the generic zip");
    assert_eq!(pick_asset(&["darwin-tools-windows.zip".to_string()], "macos", "x86_64"), Some(0));
    assert_eq!(pick_asset(&[], "macos", "aarch64"), None);
}
