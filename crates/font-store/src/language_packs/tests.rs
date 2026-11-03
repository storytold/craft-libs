use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use crate::store::Caller;

const RELEASE: &str = "https://github.com/storytold/craft-i18n/releases/download/language-packs-1/";

/// A local HTTP server answering 200 with the body set for a path, else 404.
struct Server {
    base: String,
    routes: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        let routes: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::default();
        let r = routes.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let r = r.clone();
                std::thread::spawn(move || answer(stream, &r));
            }
        });
        Self { base, routes }
    }

    fn serve(&self, path: &str, body: &[u8]) {
        self.routes.lock().unwrap().insert(format!("/{path}"), body.to_vec());
    }

    /// A client that maps the release URL onto this server.
    fn client(&self) -> Client {
        let t = Timeouts { connect: Duration::from_secs(2), response: Duration::from_secs(5), overall: Duration::from_secs(10) };
        Client::for_tests(t, vec![(RELEASE.into(), format!("{}release/", self.base))])
    }
}

fn answer(mut s: TcpStream, routes: &Mutex<HashMap<String, Vec<u8>>>) {
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match s.read(&mut b) {
            Ok(1) => head.push(b[0]),
            _ => return,
        }
    }
    let path = String::from_utf8_lossy(&head).split_whitespace().nth(1).unwrap_or("/").to_string();
    let body = routes.lock().unwrap().get(&path).cloned();
    match body {
        Some(body) => {
            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = s.write_all(&body);
        }
        None => {
            let _ = write!(s, "HTTP/1.1 404 Nope\r\nContent-Length: 4\r\nConnection: close\r\n\r\nnope");
        }
    }
}

fn temp(name: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!("craft-language-packs-{name}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A tar of `files` (name, kind, bytes), with raw header names so hostile ones can be written.
fn tar_of(files: &[(&str, tar::EntryType, &[u8])]) -> Vec<u8> {
    let mut b = tar::Builder::new(Vec::new());
    for (name, kind, bytes) in files {
        let mut h = tar::Header::new_old();
        let field = &mut h.as_old_mut().name;
        field[..name.len()].copy_from_slice(name.as_bytes());
        h.set_entry_type(*kind);
        h.set_size(bytes.len() as u64);
        h.set_mode(0o644);
        if *kind == tar::EntryType::Symlink || *kind == tar::EntryType::Link {
            h.set_link_name("/etc/passwd").unwrap();
        }
        h.set_cksum();
        b.append(&h, *bytes).unwrap();
    }
    b.into_inner().unwrap()
}

fn zst(bytes: &[u8]) -> Vec<u8> {
    ruzstd::encoding::compress_to_vec(bytes, ruzstd::encoding::CompressionLevel::Fastest)
}

const FILE: tar::EntryType = tar::EntryType::Regular;

/// A good pack archive for `locale`, with `marker` in its word list.
fn pack_archive(locale: &str, marker: &str) -> Vec<u8> {
    let toml = format!("locale = \"{locale}\"\nname = \"Test\"\nlicence = \"MIT\"\nclass = \"Embeddable\"\n");
    let dic = format!("2\nhuis\n{marker}\n");
    zst(&tar_of(&[
        ("pack.toml", FILE, toml.as_bytes()),
        ("spelling.aff", FILE, b"SET UTF-8\n"),
        ("spelling.dic", FILE, dic.as_bytes()),
        ("hyphenation.pat", FILE, b"1na n2at\n"),
        ("LICENCES/", tar::EntryType::Directory, b""),
        ("LICENCES/COPYING", FILE, b"Test licence."),
    ]))
}

fn entry(locale: &str, url: &str, archive: &[u8]) -> serde_json::Value {
    serde_json::json!({"locale": locale, "name": "Nederlands", "version": "abc", "url": url, "sha256": sha256_hex(archive),
        "size": archive.len(), "licence": "MIT", "class": "Embeddable", "spelling": true, "hyphenation": true})
}

/// The index entry for `archive` as `locale`.
fn pack_entry(locale: &str, archive: &[u8]) -> PackEntry {
    serde_json::from_value(entry(locale, &format!("{RELEASE}{locale}.tar.zst"), archive)).unwrap()
}

fn index_json(packs: &[serde_json::Value]) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"format": 1, "release": "language-packs-1", "packs": packs})).unwrap()
}

/// A store whose network is on, a server with the index and the `nl` pack, and the pin.
fn setup(name: &str) -> (FontStore, Server, PackIndexPin, Vec<u8>) {
    let server = Server::start();
    let archive = pack_archive("nl", "zzmarker");
    let index = index_json(&[entry("nl", &format!("{RELEASE}nl.tar.zst"), &archive)]);
    server.serve("release/nl.tar.zst", &archive);
    server.serve("release/index.json", &index);
    let sha: &'static str = Box::leak(sha256_hex(&index).into_boxed_str());
    let pin = PackIndexPin { url: "https://github.com/storytold/craft-i18n/releases/download/language-packs-1/index.json", sha256: sha };
    let store = FontStore::new(temp(name), Caller::User).with_client(server.client());
    store.enable_network().unwrap();
    (store, server, pin, archive)
}

#[test]
fn install_list_replace_and_remove_a_pack() {
    let (store, server, pin, _) = setup("install");
    let source = IndexSource::Pinned(pin);
    let index = store.fetch_pack_index_from(&source, &Cancel::new()).unwrap();
    assert_eq!(store.cached_pack_index_from(&source), Some(index.clone()));
    let nl = index.pack("nl").unwrap().clone();
    let mut seen = Vec::new();
    let got = store.install_pack_from(false, &index, &nl, &Cancel::new(), &mut |p| seen.push(p)).unwrap();
    assert_eq!((got.locale.as_str(), got.release.as_str(), got.licence.as_str()), ("nl", "language-packs-1", "MIT"));
    assert!(seen.last().is_some_and(|p| p.done == nl.size && p.total == nl.size));
    let dir = store.language_packs_dir().join("nl");
    assert_eq!(store.language_packs_dir(), store.dir().join("language-packs"));
    for f in ["pack.toml", "spelling.aff", "spelling.dic", "hyphenation.pat", "LICENCES/COPYING"] {
        assert!(dir.join(f).is_file(), "{f}");
    }
    assert!(std::fs::read_to_string(dir.join("spelling.dic")).unwrap().contains("zzmarker"));
    assert_eq!(store.installed_language_packs(), vec![got]);
    // A newer archive replaces the old one in one move.
    let newer = pack_archive("nl", "zznewer");
    server.serve("release/nl.tar.zst", &newer);
    let newer_entry = PackEntry { sha256: sha256_hex(&newer), size: newer.len() as u64, ..nl.clone() };
    store.install_pack_from(false, &index, &newer_entry, &Cancel::new(), &mut |_| {}).unwrap();
    assert!(std::fs::read_to_string(dir.join("spelling.dic")).unwrap().contains("zznewer"));
    assert_eq!(store.installed_language_packs().len(), 1);
    assert!(std::fs::read_dir(store.dir().join(".staging")).into_iter().flatten().flatten().next().is_none(), "nothing left staged");
    store.remove_language_pack("nl").unwrap();
    assert!(!dir.exists() && store.installed_language_packs().is_empty());
    assert!(matches!(store.remove_language_pack("nl"), Err(Error::NotInstalled(_))));
    assert!(matches!(store.remove_language_pack("../x"), Err(Error::NotInstalled(_))));
}

#[test]
fn network_off_pins_and_checksums_are_enforced() {
    let (store, server, pin, archive) = setup("pins");
    let source = IndexSource::Pinned(pin);
    store.disable_network().unwrap();
    assert!(matches!(store.fetch_pack_index_from(&source, &Cancel::new()), Err(Error::NetworkOff)));
    store.enable_network().unwrap();
    // Another index than the pinned one.
    let wrong = IndexSource::Pinned(PackIndexPin { sha256: "0000000000000000000000000000000000000000000000000000000000000000", ..pin });
    assert!(matches!(store.fetch_pack_index_from(&wrong, &Cancel::new()), Err(Error::Verify { error: VerifyError::Sha256, .. })));
    let index = store.fetch_pack_index_from(&source, &Cancel::new()).unwrap();
    assert_eq!(store.cached_pack_index_from(&wrong), None, "the cache is the pinned index only");
    let nl = index.pack("nl").unwrap().clone();
    // An archive that isn't the one the index names.
    server.serve("release/nl.tar.zst", &pack_archive("nl", "tampered"));
    assert!(store.install_pack_from(false, &index, &nl, &Cancel::new(), &mut |_| {}).is_err());
    server.serve("release/nl.tar.zst", &archive);
    let bad_sha = PackEntry { sha256: sha256_hex(b"other"), ..nl.clone() };
    assert!(matches!(
        store.install_pack_from(false, &index, &bad_sha, &Cancel::new(), &mut |_| {}),
        Err(Error::Verify { error: VerifyError::Sha256, .. })
    ));
    assert!(store.installed_language_packs().is_empty() && !store.language_packs_dir().join("nl").exists());
    // Hand-made entries are checked again.
    for e in [PackEntry { locale: "../nl".into(), ..nl.clone() }, PackEntry { url: "http://example.org/nl.tar.zst".into(), ..nl.clone() }] {
        assert!(store.install_pack_from(false, &index, &e, &Cancel::new(), &mut |_| {}).is_err());
    }
    store.disable_network().unwrap();
    assert!(matches!(store.install_pack_from(false, &index, &nl, &Cancel::new(), &mut |_| {}), Err(Error::NetworkOff)));
}

#[test]
fn indexes_are_checked() {
    let a = pack_archive("nl", "x");
    let ok = index_json(&[entry("nl", &format!("{RELEASE}nl.tar.zst"), &a)]);
    assert_eq!(PackIndex::from_json(&ok).unwrap().packs.len(), 1);
    let bad = [
        serde_json::to_vec(&serde_json::json!({"format": 2, "release": "r", "packs": []})).unwrap(),
        index_json(&[entry("NL", &format!("{RELEASE}nl.tar.zst"), &a)]),
        index_json(&[entry("../x", &format!("{RELEASE}nl.tar.zst"), &a)]),
        index_json(&[entry("nl", &format!("{RELEASE}nl.tar.zst"), &a), entry("nl", &format!("{RELEASE}nl.tar.zst"), &a)]),
        index_json(&[entry("nl", "http://127.0.0.1/nl.tar.zst", &a)]),
        index_json(&[entry("nl", "/tmp/nl.tar.zst", &a)]),
        b"not json".to_vec(),
    ];
    for b in bad {
        assert!(PackIndex::from_json(&b).is_err(), "{}", String::from_utf8_lossy(&b));
    }
    // The developer override also takes loopback http and paths, never other http hosts.
    assert!(PackIndex::parse(&index_json(&[entry("nl", "http://127.0.0.1:9/nl.tar.zst", &a)]), true).is_ok());
    assert!(PackIndex::parse(&index_json(&[entry("nl", "/tmp/nl.tar.zst", &a)]), true).is_ok());
    assert!(PackIndex::parse(&index_json(&[entry("nl", "http://example.org/nl.tar.zst", &a)]), true).is_err());
    for (s, ok) in [
        ("http://127.0.0.1:8000/i.json", true),
        ("http://localhost/i.json", true),
        ("http://[::1]:8/i.json", true),
        ("http://127.0.0.1.evil.org/i.json", false),
        ("http://evil.org@127.0.0.1/i.json", true),
        ("http://127.0.0.1@evil.org/i.json", false),
        ("ftp://127.0.0.1/i.json", false),
        ("file:///tmp/i.json", false),
    ] {
        assert_eq!(local_source(s).is_some(), ok, "{s}");
    }
}

/// The developer override (`CRAFT_LANGUAGE_PACK_INDEX`): an index from a local path or a
/// loopback http server, with no index pin, while archives are still checked against its sha256.
#[test]
fn developer_override_reads_a_local_index_without_the_pin() {
    let (store, server, _, archive) = setup("override");
    // A local file index pointing at a local archive.
    let dir = temp("override-files");
    std::fs::write(dir.join("nl.tar.zst"), &archive).unwrap();
    let index = index_json(&[entry("nl", &dir.join("nl.tar.zst").display().to_string(), &archive)]);
    std::fs::write(dir.join("index.json"), &index).unwrap();
    let source = IndexSource::Override(dir.join("index.json").display().to_string());
    let got = store.fetch_pack_index_from(&source, &Cancel::new()).unwrap();
    assert_eq!(store.cached_pack_index_from(&source), Some(got.clone()));
    store.install_pack_from(true, &got, got.pack("nl").unwrap(), &Cancel::new(), &mut |_| {}).unwrap();
    assert!(store.language_packs_dir().join("nl/spelling.dic").is_file());
    // A loopback http index and archive.
    let http_index = index_json(&[entry("nl", &format!("{}local/nl.tar.zst", server.base), &archive)]);
    server.serve("local/index.json", &http_index);
    server.serve("local/nl.tar.zst", &archive);
    let source = IndexSource::Override(format!("{}local/index.json", server.base));
    let got = store.fetch_pack_index_from(&source, &Cancel::new()).unwrap();
    store.install_pack_from(true, &got, got.pack("nl").unwrap(), &Cancel::new(), &mut |_| {}).unwrap();
    // The archive is still verified.
    server.serve("local/nl.tar.zst", &pack_archive("nl", "tampered"));
    assert!(store.install_pack_from(true, &got, got.pack("nl").unwrap(), &Cancel::new(), &mut |_| {}).is_err());
    // Only loopback http.
    let source = IndexSource::Override("http://example.org/index.json".into());
    assert!(matches!(store.fetch_pack_index_from(&source, &Cancel::new()), Err(Error::BadUrl(_))));
    // Without the variable the pin is used.
    let pin = PackIndexPin { url: "https://example.org/index.json", sha256: "00" };
    if std::env::var_os(LANGUAGE_PACK_INDEX_ENV).is_none() {
        assert_eq!(IndexSource::from_env(&pin), IndexSource::Pinned(pin));
    }
}

#[test]
fn hostile_archives_are_refused() {
    let toml: &[u8] = b"locale = \"nl\"\nlicence = \"MIT\"\n";
    let nl = PackEntry { spelling: false, hyphenation: false, ..pack_entry("nl", b"") };
    let big = vec![0u8; 10];
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("traversal", zst(&tar_of(&[("pack.toml", FILE, toml), ("../evil", FILE, b"x")]))),
        ("deep traversal", zst(&tar_of(&[("pack.toml", FILE, toml), ("LICENCES/../../evil", FILE, b"x")]))),
        ("absolute", zst(&tar_of(&[("pack.toml", FILE, toml), ("/tmp/evil", FILE, b"x")]))),
        ("symlink", zst(&tar_of(&[("pack.toml", FILE, toml), ("spelling.dic", tar::EntryType::Symlink, b"")]))),
        ("hard link", zst(&tar_of(&[("pack.toml", FILE, toml), ("spelling.dic", tar::EntryType::Link, b"")]))),
        ("too deep", zst(&tar_of(&[("pack.toml", FILE, toml), ("a/b/c", FILE, b"x")]))),
        ("duplicate", zst(&tar_of(&[("pack.toml", FILE, toml), ("pack.toml", FILE, toml)]))),
        ("backslash", zst(&tar_of(&[("pack.toml", FILE, toml), ("..\\evil", FILE, b"x")]))),
        ("no pack.toml", zst(&tar_of(&[("spelling.dic", FILE, b"1\nx\n")]))),
        ("not zstd", tar_of(&[("pack.toml", FILE, toml)])),
        ("not tar", zst(b"hello, this is not a tar archive at all")),
        ("empty", Vec::new()),
        // Only the contract's names.
        ("other root file", zst(&tar_of(&[("pack.toml", FILE, toml), ("readme.txt", FILE, b"x")]))),
        ("other folder", zst(&tar_of(&[("pack.toml", FILE, toml), ("extra/", tar::EntryType::Directory, b"")]))),
        ("nested licence", zst(&tar_of(&[("pack.toml", FILE, toml), ("LICENCES/sub/x", FILE, b"x")]))),
        ("hidden licence", zst(&tar_of(&[("pack.toml", FILE, toml), ("LICENCES/.x", FILE, b"x")]))),
        ("space in licence", zst(&tar_of(&[("pack.toml", FILE, toml), ("LICENCES/a b", FILE, b"x")]))),
        ("trailing dot", zst(&tar_of(&[("pack.toml", FILE, toml), ("LICENCES/COPYING.", FILE, b"x")]))),
        ("reserved CON", zst(&tar_of(&[("pack.toml", FILE, toml), ("LICENCES/CON", FILE, b"x")]))),
        ("reserved nul.txt", zst(&tar_of(&[("pack.toml", FILE, toml), ("LICENCES/nul.txt", FILE, b"x")]))),
        ("reserved com1", zst(&tar_of(&[("pack.toml", FILE, toml), ("LICENCES/com1.md", FILE, b"x")]))),
        // pack.toml must name the entry's locale.
        ("other locale", zst(&tar_of(&[("pack.toml", FILE, b"locale = \"de\"\n")]))),
        ("no locale", zst(&tar_of(&[("pack.toml", FILE, b"licence = \"MIT\"\n")]))),
        ("bad toml", zst(&tar_of(&[("pack.toml", FILE, b"locale = \n")]))),
        // Nothing may follow the archive: not another frame, not raw bytes.
        ("second archive", [zst(&tar_of(&[("pack.toml", FILE, toml)])), zst(&tar_of(&[("spelling.dic", FILE, b"1\nx\n")]))].concat()),
        ("trailing bytes", [zst(&tar_of(&[("pack.toml", FILE, toml)])), b"garbage".to_vec()].concat()),
        ("truncated", zst(&tar_of(&[("pack.toml", FILE, toml), ("spelling.dic", FILE, &big)]))[..20].to_vec()),
    ];
    for (what, archive) in cases {
        let root = temp("hostile");
        let dest = root.join("nl");
        assert!(unpack(&archive, &dest, &nl).is_err(), "{what}");
        assert!(!root.join("evil").exists() && !root.parent().unwrap().join("evil").exists(), "{what}");
    }
    // Too many entries.
    let names: Vec<String> = (0..=MAX_PACK_ENTRIES).map(|i| format!("LICENCES/f{i}")).collect();
    let mut files: Vec<(&str, tar::EntryType, &[u8])> = vec![("pack.toml", FILE, toml)];
    files.extend(names.iter().map(|n| (n.as_str(), FILE, &b""[..])));
    assert!(unpack(&zst(&tar_of(&files)), &temp("many").join("nl"), &nl).is_err());
    // A header claiming a file larger than the cap is refused before anything is read.
    let mut t = tar::Header::new_gnu();
    t.set_path("pack.toml").unwrap();
    t.set_size(toml.len() as u64);
    t.set_cksum();
    let mut b = tar::Builder::new(Vec::new());
    b.append(&t, toml).unwrap();
    let mut tarball = b.into_inner().unwrap();
    tarball.truncate(tarball.len() - 1024); // drop the end marker
    let mut h = tar::Header::new_gnu();
    h.set_path("spelling.dic").unwrap();
    h.set_size(MAX_PACK_FILE_BYTES + 1);
    h.set_cksum();
    tarball.extend_from_slice(h.as_bytes());
    tarball.extend_from_slice(&[0u8; 1024]);
    let err = unpack(&zst(&tarball), &temp("huge").join("nl"), &nl).unwrap_err();
    assert!(err.contains("larger than"), "{err}");
    // A good one unpacks.
    let dest = temp("good").join("nl");
    let good = pack_archive("nl", "x");
    unpack(&good, &dest, &pack_entry("nl", &good)).unwrap();
    assert!(dest.join("LICENCES/COPYING").is_file());
    // A tar split over several zstd frames is read to the end.
    let tarball = tar_of(&[("pack.toml", FILE, toml), ("LICENCES/", tar::EntryType::Directory, b""), ("LICENCES/COPYING", FILE, b"t")]);
    let (a, b) = tarball.split_at(700);
    let dest = temp("frames").join("nl");
    unpack(&[zst(a), zst(b)].concat(), &dest, &nl).unwrap();
    assert!(dest.join("LICENCES/COPYING").is_file());
    // Empty frames are read past, but only so many.
    let one = zst(&tar_of(&[("pack.toml", FILE, toml)]));
    let empty = zst(b"");
    unpack(&[one.clone(), empty.clone(), empty.clone()].concat(), &temp("empties").join("nl"), &nl).unwrap();
    let many: Vec<u8> = std::iter::once(one).chain(std::iter::repeat_n(empty, MAX_PACK_FRAMES)).flatten().collect();
    let err = unpack(&many, &temp("frames-cap").join("nl"), &nl).unwrap_err();
    assert!(err.contains("frames"), "{err}");
    // The index's promises are checked against the files.
    let bare = zst(&tar_of(&[("pack.toml", FILE, toml)]));
    for e in [PackEntry { spelling: true, ..nl.clone() }, PackEntry { hyphenation: true, ..nl.clone() }] {
        let err = unpack(&bare, &temp("promise").join("nl"), &e).unwrap_err();
        assert!(err.contains("promises"), "{err}");
    }
}

#[test]
fn names_are_plain_text_and_reserved_locales_refused() {
    let a = pack_archive("nl", "x");
    let mut e = entry("nl", &format!("{RELEASE}nl.tar.zst"), &a);
    e["name"] = serde_json::json!(" Neder\u{202e}lands\u{7}\u{200b}\n");
    assert_eq!(PackIndex::from_json(&index_json(&[e])).unwrap().packs[0].name, "Nederlands");
    for l in ["con", "nul", "aux", "prn", "com1", "lpt9"] {
        assert!(!is_locale(l), "{l}");
        assert!(PackIndex::from_json(&index_json(&[entry(l, &format!("{RELEASE}{l}.tar.zst"), &a)])).is_err(), "{l}");
    }
    assert!(is_locale("com") && is_locale("lpt") && is_locale("com10"));
}

#[test]
fn a_corrupt_record_is_rebuilt_from_the_folders() {
    let (store, server, pin, _) = setup("corrupt");
    let index = store.fetch_pack_index_from(&IndexSource::Pinned(pin), &Cancel::new()).unwrap();
    let nl = index.pack("nl").unwrap().clone();
    store.install_pack_from(false, &index, &nl, &Cancel::new(), &mut |_| {}).unwrap();
    let record = store.dir().join("language-packs.json");
    std::fs::write(&record, b"{ not json").unwrap();
    // Still listed, with the name from its pack.toml.
    let listed = store.installed_language_packs();
    assert_eq!(listed.iter().map(|p| (p.locale.as_str(), p.name.as_str())).collect::<Vec<_>>(), [("nl", "Test")]);
    // craft-libs#486 review: listing holds no store lock, so it never writes the record (it
    // could race an install); the next install or removal repairs it.
    assert_eq!(std::fs::read(&record).unwrap(), b"{ not json");
    assert!(!store.dir().join("language-packs.json.bad").exists());
    // Installing another keeps it.
    std::fs::write(&record, b"{ not json").unwrap();
    let de = pack_archive("de", "zzde");
    server.serve("release/de.tar.zst", &de);
    let de_entry = PackEntry { url: format!("{RELEASE}de.tar.zst"), ..pack_entry("de", &de) };
    store.install_pack_from(false, &index, &de_entry, &Cancel::new(), &mut |_| {}).unwrap();
    let mut locales: Vec<String> = store.installed_language_packs().into_iter().map(|p| p.locale).collect();
    locales.sort();
    assert_eq!(locales, ["de", "nl"]);
    assert_eq!(std::fs::read(store.dir().join("language-packs.json.bad")).unwrap(), b"{ not json", "the unreadable record is kept aside");
    assert!(serde_json::from_slice::<serde_json::Value>(&std::fs::read(&record).unwrap()).is_ok(), "rebuilt in place");
    // A missing record with pack folders is rebuilt too.
    std::fs::remove_file(&record).unwrap();
    assert_eq!(store.installed_language_packs().len(), 2);
    assert!(!record.exists(), "listing writes nothing");
    let nl_again = PackEntry { url: format!("{RELEASE}nl.tar.zst"), ..nl.clone() };
    store.install_pack_from(false, &index, &nl_again, &Cancel::new(), &mut |_| {}).unwrap();
    assert_eq!(store.installed_language_packs().len(), 2, "de kept when the record was missing");
    // Removing works on a rebuilt record too.
    std::fs::write(&record, b"\xff").unwrap();
    store.remove_language_pack("nl").unwrap();
    assert_eq!(store.installed_language_packs().into_iter().map(|p| p.locale).collect::<Vec<_>>(), ["de"]);
}

#[test]
fn index_text_is_plain_and_sources_are_checked() {
    let a = pack_archive("nl", "x");
    let mut e = entry("nl", &format!("{RELEASE}nl.tar.zst"), &a);
    e["licence"] = serde_json::json!("MIT\u{202e}");
    e["version"] = serde_json::json!("ab\u{0}c");
    e["class"] = serde_json::json!("\u{200f}Embeddable");
    e["source"] = serde_json::json!("https://example.org/nl");
    let json = serde_json::to_vec(&serde_json::json!({"format": 1, "release": "r1\u{2066}", "packs": [e.clone()]})).unwrap();
    let index = PackIndex::from_json(&json).unwrap();
    let p = &index.packs[0];
    assert_eq!((index.release.as_str(), p.licence.as_str(), p.version.as_str(), p.class.as_str()), ("r1", "MIT", "abc", "Embeddable"));
    assert_eq!(p.source.as_deref(), Some("https://example.org/nl"));
    assert_eq!(PackIndex::from_json(&index_json(&[entry("nl", &format!("{RELEASE}nl.tar.zst"), &a)])).unwrap().packs[0].source, None);
    for bad in ["javascript:alert(1)", "http://example.org", "https://a b", "https://a\u{202e}b"] {
        e["source"] = serde_json::json!(bad);
        let err = PackIndex::from_json(&index_json(std::slice::from_ref(&e))).unwrap_err();
        assert!(!err.to_string().contains("font"), "{err}");
    }
}

#[test]
fn bad_hand_made_entries_are_index_errors() {
    let (store, _server, pin, _) = setup("variant");
    let index = store.fetch_pack_index_from(&IndexSource::Pinned(pin), &Cancel::new()).unwrap();
    let nl = index.pack("nl").unwrap().clone();
    for e in [PackEntry { locale: "../nl".into(), ..nl.clone() }, PackEntry { sha256: "x".into(), ..nl.clone() }] {
        let got = store.install_pack_from(false, &index, &e, &Cancel::new(), &mut |_| {});
        assert!(matches!(got, Err(Error::Index(IndexError::Invalid(_)))), "{got:?}");
    }
}

#[test]
fn the_override_is_visible() {
    let set = std::env::var(LANGUAGE_PACK_INDEX_ENV).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    assert_eq!(language_pack_index_overridden(), set);
}

/// A staging folder of the same name (an earlier process, or another one in a PID namespace) is
/// never used or deleted: the next name is taken, so an unpack can't mix in its files.
#[test]
fn a_stale_stage_folder_is_cleared() {
    let store = temp("stale-stage");
    let staging = store.join(".staging");
    for n in 0..512 {
        let d = staging.join(format!("{}-{n}", std::process::id()));
        std::fs::create_dir_all(d.join("nl")).unwrap();
        std::fs::write(d.join("nl/pack.toml"), b"old").unwrap();
    }
    let stage = Stage::new(&store).unwrap();
    assert_eq!(std::fs::read_dir(&stage.dir).unwrap().count(), 0, "{}", stage.dir.display());
    // The others are left alone: they may be another process's (PID namespaces).
    let left = std::fs::read_dir(&staging).unwrap().flatten().filter(|e| e.path().join("nl/pack.toml").is_file()).count();
    assert_eq!(left, 512);
}
