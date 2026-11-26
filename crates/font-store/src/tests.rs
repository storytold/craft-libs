use serde_json::json;

use crate::*;

const COMMIT: &str = "45e3d5269e4a17d823f6d84470bc084dc5ae46f6";

/// A font with the tables a scan and [`is_font`] need: `name`, `OS/2` and an empty `glyf`.
fn font(family: &str, style: &str, weight: u16) -> Vec<u8> {
    use write_fonts::FontBuilder;
    use write_fonts::tables::name::{Name, NameRecord};
    use write_fonts::tables::os2::Os2;
    use write_fonts::types::{NameId, Tag};
    let ps = format!("{}-{}", family.replace(' ', ""), style.replace(' ', ""));
    let mut records: Vec<NameRecord> = [(1, family.to_string()), (2, style.to_string()), (4, format!("{family} {style}")), (6, ps)]
        .into_iter()
        .map(|(id, text)| NameRecord::new(3, 1, 0x409, NameId::new(id), text.into()))
        .collect();
    records.sort_by_key(|r| r.name_id);
    let mut builder = FontBuilder::new();
    builder.add_table(&Name::new(records)).unwrap();
    builder.add_table(&Os2 { us_weight_class: weight, ul_code_page_range_1: Some(1), ul_code_page_range_2: Some(0), ..Default::default() }).unwrap();
    builder.add_raw(Tag::new(b"glyf"), vec![0u8; 4]);
    builder.build()
}

/// An index and the files it lists, by their URL-encoded google/fonts paths.
struct Fixture {
    files: Vec<(String, Vec<u8>)>,
    index: serde_json::Value,
}

fn file_entry(path: &str, bytes: &[u8], style: &str, weight: u16, axes: serde_json::Value, ps: &[&str]) -> serde_json::Value {
    json!({"path": path, "size": bytes.len(), "sha256": sha256_hex(bytes), "style": style, "weight": weight, "axes": axes, "postscript": ps})
}

/// An index of four families, and the bytes of their files.
fn fixture() -> Fixture {
    let regular = font("Test Sans", "Regular", 400);
    let bold = font("Test Sans", "Bold", 700);
    let italic = font("Test Sans", "Italic", 400);
    let var = font("Test Var", "Regular", 400);
    let one = font("Shared One", "Regular", 400);
    let two = font("Shared Two", "Regular", 400);
    let index = json!({
        "schema": 1, "version": "1.0", "source": "google/fonts", "commit": COMMIT,
        "base_url": format!("https://cdn.jsdelivr.net/gh/google/fonts@{COMMIT}/"),
        "fallback_base_url": format!("https://raw.githubusercontent.com/google/fonts/{COMMIT}/"),
        "a_field_from_a_later_minor_version": true,
        "languages": [{"id": "en_Latn", "name": "English", "autonym": "English", "script": "Latn", "population": 1, "historical": false}],
        "scripts": [{"id": "Latn", "name": "Latin"}],
        "families": [
            {"family": "Shared One", "license": "OFL", "license_path": null, "languages": [0], "scripts": ["Latn"],
             "files": [file_entry("ofl/sharedone/SharedOne-Regular.ttf", &one, "normal", 400, json!([]), &["Shared-Regular"])]},
            {"family": "Shared Two", "license": "OFL", "license_path": null,
             "files": [file_entry("ofl/sharedtwo/SharedTwo-Regular.ttf", &two, "normal", 400, json!([]), &["Shared-Regular"])]},
            {"family": "Test Sans", "designer": "Someone", "categories": ["SANS_SERIF"], "license": "OFL", "license_path": "ofl/testsans/OFL.txt",
             "languages": [0], "scripts": ["Latn"], "files": [
                file_entry("ofl/testsans/TestSans-Bold.ttf", &bold, "normal", 700, json!([]), &["TestSans-Bold"]),
                file_entry("ofl/testsans/TestSans-Italic.ttf", &italic, "italic", 400, json!([]), &["TestSans-Italic"]),
                file_entry("ofl/testsans/TestSans-Regular.ttf", &regular, "normal", 400, json!([]), &["TestSans-Regular"])]},
            {"family": "Test Var", "license": "APACHE2", "license_path": "apache/testvar/LICENSE.txt", "files": [
                file_entry("apache/testvar/TestVar[wght].ttf", &var, "normal", 400, json!([{"tag": "wght", "min": 300.0, "max": 700.0}]),
                    &["TestVar-Light", "TestVar-Regular", "TestVar-SemiBold", "TestVar-Bold"])]},
        ]
    });
    let files = vec![
        ("ofl/sharedone/SharedOne-Regular.ttf".into(), one),
        ("ofl/sharedtwo/SharedTwo-Regular.ttf".into(), two),
        ("ofl/testsans/TestSans-Bold.ttf".into(), bold),
        ("ofl/testsans/TestSans-Italic.ttf".into(), italic),
        ("ofl/testsans/TestSans-Regular.ttf".into(), regular),
        ("ofl/testsans/OFL.txt".into(), b"SIL Open Font License".to_vec()),
        ("apache/testvar/TestVar%5Bwght%5D.ttf".into(), var),
        ("apache/testvar/LICENSE.txt".into(), b"Apache License".to_vec()),
    ];
    Fixture { files, index }
}

fn index_of(v: &serde_json::Value) -> Result<Index, IndexError> {
    Index::from_json(v.to_string().as_bytes())
}

#[test]
fn the_index_is_read_and_checked() {
    let f = fixture();
    let index = index_of(&f.index).unwrap();
    let source = &index.sources()[0];
    assert_eq!((source.version.as_str(), source.commit.as_str(), index.families().len()), ("1.0", COMMIT, 4));
    // A format 1.0 index is Google Fonts, curated, with URLs from its base URLs.
    assert_eq!((source.id.as_str(), source.name.as_str(), source.curated), (GOOGLE_FONTS, "Google Fonts", true));
    let sans = index.family("test sans").unwrap();
    assert_eq!((sans.dir(), sans.license_file_name(), sans.designer.as_str()), ("testsans", Some("OFL.txt"), "Someone"));
    assert_eq!((sans.source.as_str(), sans.is_curated()), (GOOGLE_FONTS, true));
    assert_eq!(sans.languages(&index).map(|l| l.id.as_str()).collect::<Vec<_>>(), ["en_Latn"]);

    let with = |edit: &dyn Fn(&mut serde_json::Value)| {
        let mut v = f.index.clone();
        edit(&mut v);
        index_of(&v)
    };
    assert!(with(&|v| v["version"] = json!("1.7")).is_ok(), "a later minor version only adds fields");
    assert!(matches!(with(&|v| v["version"] = json!("2.0")), Err(IndexError::Version(_))));
    assert!(matches!(with(&|v| v["version"] = json!("1")), Err(IndexError::Version(_))));
    assert!(matches!(
        with(&|v| {
            v.as_object_mut().unwrap().remove("version");
        }),
        Err(IndexError::Version(_))
    ));
    // A version 2 index may change any other field: the version is checked first.
    assert!(matches!(
        with(&|v| {
            v["version"] = json!("2.0");
            v["families"] = json!("other");
        }),
        Err(IndexError::Version(_))
    ));
    let invalid = [
        with(&|v| v["base_url"] = json!("http://cdn.jsdelivr.net/gh/google/fonts/")),
        with(&|v| v["commit"] = json!("main")),
        with(&|v| v["families"][2]["files"][0]["path"] = json!("ofl/../TestSans-Bold.ttf")),
        with(&|v| v["families"][2]["files"][0]["path"] = json!("ofl/testsans/.hidden.ttf")),
        with(&|v| v["families"][2]["files"][0]["path"] = json!("ofl/other/TestSans-Bold.ttf")),
        with(&|v| v["families"][2]["files"][0]["sha256"] = json!("00")),
        with(&|v| v["families"][2]["files"][0]["weight"] = json!(0)),
        with(&|v| v["families"][2]["license_path"] = json!("ofl/elsewhere/OFL.txt")),
        with(&|v| v["families"][2]["license_path"] = json!("ofl/testsans/TestSans-Bold.ttf")),
        with(&|v| v["families"][2]["source"] = json!("notofonts")),
        with(&|v| v["base_url"] = json!("https://cdn.jsdelivr.net/gh/{repo}/")),
        with(&|v| v["families"][2]["languages"] = json!([5])),
        with(&|v| v["families"][3]["family"] = json!("TEST SANS")),
    ];
    for (i, r) in invalid.into_iter().enumerate() {
        assert!(matches!(r, Err(IndexError::Invalid(_))), "case {i}: {r:?}");
    }
    assert!(matches!(Index::from_json(b"{"), Err(IndexError::Json(_))));
}

#[test]
fn format_1_2_adds_tags_and_samples_and_1_1_still_loads() {
    let f = fixture();
    let mut v1_1 = f.index.clone();
    v1_1["version"] = json!("1.1");
    let old = index_of(&v1_1).unwrap();
    let sans = old.family("Test Sans").unwrap();
    assert!(sans.tags.is_empty() && sans.sample_script.is_none() && old.samples().is_empty());
    assert_eq!(old.sample_text(sans, &sans.files[0]), None);

    let sample = |text: &str| json!({"text": text, "method": "udhr", "language": "xx"});
    let mut first = f.index.clone();
    first["version"] = json!("1.2");
    first["samples"] = json!({"Latn": sample("Hello"), "Thai": sample("first")});
    first["families"][2]["tags"] = json!(["Sans", "Display/Playful"]);
    first["families"][2]["sample_script"] = json!("Latn");
    first["families"][2]["reserved_names"] = json!(["Test"]);
    first["families"][2]["files"][0]["sample_script"] = json!("Thai");
    first["families"][2]["files"][1]["sample_text"] = json!("Own text");
    first["families"][2]["files"][2]["sample_script"] = json!("Zsym");
    first["families"][3]["files"][0]["instances"] = json!([{"name": "Regular", "coordinates": {"wght": 400.0}}]);
    first["families"][3]["files"][0]["list_instance"] = json!(0);
    let mut second = noto_fixture().index;
    second["samples"] = json!({"Thai": sample("second"), "Jpan": sample("Japanese")});
    let index = Index::merge([index_of(&first).unwrap(), index_of(&second).unwrap()]);

    let sans = index.family_from(GOOGLE_FONTS, "Test Sans").unwrap();
    assert_eq!(
        (sans.tags.as_slice(), sans.reserved_names.as_slice()),
        (&["Sans".to_string(), "Display/Playful".to_string()][..], &["Test".to_string()][..])
    );
    assert_eq!(index.samples().keys().collect::<Vec<_>>(), ["Jpan", "Latn", "Thai"]);
    assert_eq!(index.sample("Thai").unwrap().text, "first");
    // Own text, then the file's script, which has no fallback (Zsym has no sample).
    assert_eq!(index.sample_text(sans, &sans.files[1]), Some("Own text"));
    assert_eq!(index.sample_text(sans, &sans.files[0]), Some("first"));
    assert_eq!(index.sample_text(sans, &sans.files[2]), None);
    let var = index.family_from(GOOGLE_FONTS, "Test Var").unwrap();
    assert_eq!((var.files[0].instances[0].name.as_str(), var.files[0].list_instance), ("Regular", Some(0)));

    first["families"][3]["files"][0]["list_instance"] = json!(1);
    assert!(index_of(&first).is_err());
    first["families"][3]["files"][0]["list_instance"] = json!(0);
    first["samples"]["Latn"]["text"] = json!("bad\u{7}");
    assert!(index_of(&first).is_err());
    first["samples"]["Latn"] = sample("Hello");
    // An instance gives a coordinate for every axis, and only those.
    first["families"][3]["files"][0]["instances"] = json!([{"name": "Regular", "coordinates": {"wdth": 100.0}}]);
    assert!(index_of(&first).is_err());
    first["families"][3]["files"][0]["instances"] = json!([{"name": "Regular", "coordinates": {"wght": 400.0}}]);
    assert!(index_of(&first).is_ok());
    // Named instances belong to variable fonts only.
    first["families"][2]["files"][0]["instances"] = json!([{"name": "Bold", "coordinates": {}}]);
    assert!(index_of(&first).is_err());

    // Every 1.2 string a user source can set is checked, one case at a time.
    let mut check = first.clone();
    check["families"][2]["files"][0]["instances"] = json!([]);
    check["families"][2]["copyright"] = json!("Copyright 2020 Test");
    check["families"][3]["files"][0]["instances"] = json!([{"name": "Regular", "coordinates": {"wght": 400.0}}]);
    assert!(index_of(&check).is_ok());
    fn reject(v: &mut serde_json::Value, path: &dyn Fn(&mut serde_json::Value) -> &mut serde_json::Value, bad: serde_json::Value, what: &str) {
        let good = std::mem::replace(path(v), bad);
        assert!(index_of(v).is_err(), "{what}");
        *path(v) = good;
        assert!(index_of(v).is_ok(), "{what} restored");
    }
    reject(&mut check, &|v| &mut v["families"][2]["tags"][0], json!("Sans\u{202e}"), "bidi control in a tag");
    reject(&mut check, &|v| &mut v["families"][2]["copyright"], json!("Copyright 2020\nTest"), "line break in a copyright");
    reject(&mut check, &|v| &mut v["families"][2]["files"][1]["sample_text"], json!("x".repeat(61)), "long sample text");
    reject(&mut check, &|v| &mut v["families"][3]["files"][0]["instances"][0]["coordinates"]["wght"], json!(900.0), "coordinate out of range");
    check["families"][3]["upstream"] = json!("https://example.org/repo");
    check["families"][3]["notice_file"] = json!({"path": "apache/testvar/NOTICE", "size": 1, "sha256": sha256_hex(b"x")});
    assert!(index_of(&check).is_ok());
    reject(&mut check, &|v| &mut v["families"][3]["upstream"], json!("http://example.org/repo"), "non-https upstream");
    reject(&mut check, &|v| &mut v["families"][3]["notice_file"]["sha256"], json!("abc"), "bad notice sha256");
    reject(&mut check, &|v| &mut v["families"][3]["notice_file"]["path"], json!("apache/testvar/TestVar[wght].ttf"), "notice named like a font");
}

const NOTO_COMMIT: &str = "93fac8abebe6c7fed0c5b7d2456d8fa484fa146c";
const VAR_PATH: &str = "fonts/NotoSerifTest/full/variable-ttf/NotoSerifTest[wdth,wght].ttf";
const CJK_PATH: &str = "Sans/OTF/Japanese/NotoSansCJKtest-Regular.otf";
const CONDENSED_PATH: &str = "fonts/TestSans/full/ttf/TestSans-CondensedBold.ttf";

/// A format 1.1 index of another source, as craft-font-index's Noto index has it: URL templates
/// with `{repo}` and `{ref}`, a family in another repository at a tag, licences with a sha256
/// (one in a third repository), and a family Google Fonts also has, in a width it lacks.
fn noto_fixture() -> Fixture {
    let var = font("Noto Serif Test", "Regular", 400);
    let cjk = font("Noto Sans CJK Test", "Regular", 400);
    let condensed = font("Test Sans", "Condensed Bold", 700);
    let ofl = b"Noto OFL".to_vec();
    let cjk_licence = b"noto-cjk LICENSE".to_vec();
    let lic = |path: &str, bytes: &[u8]| json!({"path": path, "size": bytes.len(), "sha256": sha256_hex(bytes)});
    let mut var_entry = file_entry(
        VAR_PATH,
        &var,
        "normal",
        400,
        json!([{"tag": "wght", "min": 100.0, "max": 900.0}, {"tag": "wdth", "min": 62.5, "max": 100.0}]),
        &["NotoSerifTest-Regular", "NotoSerifTest-Bold"],
    );
    var_entry["a_later_field"] = json!(1);
    let mut condensed_entry = file_entry(CONDENSED_PATH, &condensed, "normal", 700, json!([]), &["TestSans-CondensedBold"]);
    condensed_entry["width"] = json!(75.0);
    let mut serif_licence = lic("OFL.txt", &ofl);
    serif_licence["repo"] = json!("notofonts/serif-test");
    serif_licence["ref"] = json!("NotoSerifTest-v2.106");
    let index = json!({
        "schema": 1, "version": "1.1", "source": "notofonts", "commit": NOTO_COMMIT,
        "source_info": {"id": "notofonts", "name": "Noto", "homepage": "https://notofonts.github.io", "curated": true,
            "file_urls": ["https://cdn.jsdelivr.net/gh/{repo}@{ref}/{path}", "https://raw.githubusercontent.com/{repo}/{ref}/{path}"]},
        "repo": "notofonts/notofonts.github.io", "ref": NOTO_COMMIT,
        "languages": [
            {"id": "en_Latn", "name": "English", "script": "Latn", "population": 1},
            {"id": "ja_Jpan", "name": "Japanese", "autonym": "日本語", "script": "Jpan", "population": 1}],
        "scripts": [{"id": "Jpan", "name": "Japanese"}, {"id": "Latn", "name": "Latin"}],
        "families": [
            {"family": "Noto Sans CJK Test", "source": "notofonts", "license": "OFL", "license_path": null,
             "license_file": lic("LICENSE", &cjk_licence), "repo": "notofonts/noto-cjk", "ref": "Sans2.004",
             "languages": [0, 1], "scripts": ["Jpan", "Latn"],
             "files": [file_entry(CJK_PATH, &cjk, "normal", 400, json!([]), &["NotoSansCJKtest-Regular"])]},
            {"family": "Noto Serif Test", "source": "notofonts", "license": "OFL", "license_path": null,
             "license_file": serif_licence, "languages": [0], "files": [var_entry]},
            {"family": "Test Sans", "source": "notofonts", "license": "OFL", "license_path": null,
             "license_file": lic("OFL.txt", &ofl), "files": [condensed_entry]},
        ]
    });
    let files = vec![
        (format!("notofonts/noto-cjk/Sans2.004/{CJK_PATH}"), cjk),
        ("notofonts/noto-cjk/Sans2.004/LICENSE".into(), cjk_licence),
        (format!("notofonts/notofonts.github.io/{NOTO_COMMIT}/fonts/NotoSerifTest/full/variable-ttf/NotoSerifTest%5Bwdth%2Cwght%5D.ttf"), var),
        ("notofonts/serif-test/NotoSerifTest-v2.106/OFL.txt".into(), ofl.clone()),
        (format!("notofonts/notofonts.github.io/{NOTO_COMMIT}/{CONDENSED_PATH}"), condensed),
        (format!("notofonts/notofonts.github.io/{NOTO_COMMIT}/OFL.txt"), ofl),
    ];
    Fixture { files, index }
}

#[test]
fn a_user_index_is_never_curated_and_cant_shadow_a_built_in_source() {
    let mut v = noto_fixture().index;
    v["source_info"]["file_urls"] = json!(["https://fonts.example.org/files/{path}"]);
    v["families"][1]["license"] = json!("Proprietary");
    let url = "https://fonts.example.org/index.json";
    let index = Index::from_user_bytes(v.to_string().as_bytes(), url).unwrap();
    let source = &index.sources()[0];
    assert_eq!((source.id.as_str(), source.curated, source.user_url.as_deref()), ("user:fonts.example.org/notofonts", false, Some(url)));
    assert_eq!(source.warnings.len(), 1);
    let cjk = index.family("Noto Sans CJK Test").unwrap();
    assert!(!cjk.is_curated());
    assert_eq!(cjk.dir(), "user-fonts-example-org-notofonts-6b687c53/noto-sans-cjk-test");
    assert_eq!(index.file_urls(cjk, &cjk.files[0]), [format!("https://fonts.example.org/files/{CJK_PATH}")]);
    // Merged after a built-in source of the ID it claims, it is a separate source.
    let merged = Index::merge([index_of(&noto_fixture().index).unwrap(), index]);
    assert_eq!(merged.sources().len(), 2);
    assert!(merged.family("Noto Sans CJK Test").unwrap().is_curated());
    assert!(Index::from_user_bytes(v.to_string().as_bytes(), "http://fonts.example.org/index.json").is_err());
}

#[cfg(not(target_arch = "wasm32"))]
mod store {
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use craft_font_catalog::{Catalog, FontEnv, Os};

    use super::*;

    /// What the server answers for a path.
    #[derive(Clone)]
    enum Reply {
        Ok(Vec<u8>),
        Status(u16),
    }

    /// A local HTTP server for `/cdn/` (jsDelivr) and `/raw/` (raw GitHub).
    struct Server {
        base: String,
        routes: Arc<Mutex<HashMap<String, Reply>>>,
        log: Arc<Mutex<Vec<String>>>,
    }

    impl Server {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}/", listener.local_addr().unwrap());
            let routes: Arc<Mutex<HashMap<String, Reply>>> = Arc::default();
            let log: Arc<Mutex<Vec<String>>> = Arc::default();
            let (r, l) = (routes.clone(), log.clone());
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { continue };
                    let (r, l) = (r.clone(), l.clone());
                    std::thread::spawn(move || answer(stream, &r, &l));
                }
            });
            Self { base, routes, log }
        }

        fn serve(&self, prefix: &str, path: &str, reply: Reply) {
            self.routes.lock().unwrap().insert(format!("/{prefix}/{path}"), reply);
        }

        /// The fixture's files on both hosts.
        fn serve_fixture(&self, f: &Fixture) {
            for (path, bytes) in &f.files {
                self.serve("cdn", &format!("gh/google/fonts@{COMMIT}/{path}"), Reply::Ok(bytes.clone()));
                self.serve("raw", &format!("google/fonts/{COMMIT}/{path}"), Reply::Ok(bytes.clone()));
            }
        }

        fn requests(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }

        fn client(&self) -> Client {
            let t = Timeouts { connect: Duration::from_secs(2), response: Duration::from_secs(5), overall: Duration::from_secs(10) };
            Client::for_tests(
                t,
                vec![
                    ("https://cdn.jsdelivr.net/".into(), format!("{}cdn/", self.base)),
                    ("https://raw.githubusercontent.com/".into(), format!("{}raw/", self.base)),
                    ("https://fonts.example.org/".into(), format!("{}user/", self.base)),
                ],
            )
        }
    }

    fn answer(mut s: TcpStream, routes: &Mutex<HashMap<String, Reply>>, log: &Mutex<Vec<String>>) {
        let mut head = Vec::new();
        let mut b = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            match s.read(&mut b) {
                Ok(1) => head.push(b[0]),
                _ => return,
            }
        }
        let text = String::from_utf8_lossy(&head).into_owned();
        let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
        log.lock().unwrap().push(path.clone());
        let reply = routes.lock().unwrap().get(&path).cloned().unwrap_or(Reply::Status(404));
        match reply {
            Reply::Ok(body) => {
                let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = s.write_all(&body);
            }
            Reply::Status(code) => {
                let _ = write!(s, "HTTP/1.1 {code} Nope\r\nContent-Length: 4\r\nConnection: close\r\n\r\nnope");
            }
        }
    }

    fn temp(name: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("craft-font-store-{name}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> =
            std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    /// No font files, staged downloads or family folders anywhere in the store.
    fn assert_no_family_files(dir: &Path) {
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    let name = e.file_name().to_string_lossy().into_owned();
                    assert!(["settings.json", ".lock"].contains(&name.as_str()), "left behind: {}", p.display());
                }
            }
        }
        assert!(!dir.join("testsans").exists());
        assert!(entries(&dir.join(".staging")).is_empty(), "{:?}", entries(&dir.join(".staging")));
    }

    fn user_store(server: &Server, dir: &Path) -> FontStore {
        let store = FontStore::new(dir.to_path_buf(), Caller::User).with_client(server.client());
        store.enable_network().unwrap();
        store
    }

    fn none(_: Progress) {}

    #[test]
    fn preview_service_is_a_mirror_tried_first() {
        let server = Server::start();
        let f = fixture();
        server.serve_fixture(&f);
        let index = index_of(&f.index).unwrap();
        let face = index.find_face("Test Sans", "Bold").unwrap();
        let sha = face.file.sha256.clone();
        let dir = temp("mirror");
        let user = FontStore::new(dir.clone(), Caller::User).with_client(server.client()).with_preview_env(None);
        user.enable_network().unwrap();

        // No service: only the index's URLs.
        user.install_face(&index, face, &Cancel::new(), &mut none).unwrap();
        assert!(server.requests().iter().all(|r| !r.starts_with("/mirror/")));
        user.remove_family(&face.family.source, &face.family.family).unwrap();
        let before = server.requests().len();

        // A service answering with the right bytes is the only request for the file.
        let bytes = f.files.iter().find(|(p, _)| p == &face.file.path).unwrap().1.clone();
        server.serve("mirror", &format!("v1/files/{sha}"), Reply::Ok(bytes));
        user.set_preview_service(Some(&format!("{}mirror/", server.base))).unwrap();
        user.install_face(&index, face, &Cancel::new(), &mut none).unwrap();
        let after = server.requests()[before..].to_vec();
        assert_eq!(after.first().map(String::as_str), Some(format!("/mirror/v1/files/{sha}").as_str()));
        assert!(after.iter().all(|r| !r.starts_with("/cdn/") || r.contains("LICENSE") || r.contains("OFL")), "{after:?}");
        user.remove_family(&face.family.source, &face.family.family).unwrap();

        // The mirror is plain http to 127.0.0.1 and is fetched by a client that is https only
        // for everything else.
        let strict = {
            let mut c = Client::new(Timeouts::default());
            c.rewrite = vec![
                ("https://cdn.jsdelivr.net/".into(), format!("{}cdn/", server.base)),
                ("https://raw.githubusercontent.com/".into(), format!("{}raw/", server.base)),
            ];
            c
        };
        let strict_user = FontStore::new(dir.clone(), Caller::User).with_client(strict).with_preview_env(None);
        server.serve("mirror", &format!("v1/files/{sha}"), Reply::Ok(f.files.iter().find(|(p, _)| p == &face.file.path).unwrap().1.clone()));
        let before = server.requests().len();
        // The licence has no mirror address here, and its index URLs are refused, so the install
        // fails; the file itself came from the mirror and nothing else was requested.
        let err = strict_user.install_face(&index, face, &Cancel::new(), &mut none).unwrap_err();
        let licence = face.family.license_file_name().unwrap();
        assert!(matches!(&err, Error::Download { what, .. } if what == licence), "{err:?}");
        assert_eq!(server.requests()[before..], [format!("/mirror/v1/files/{sha}")]);

        // Bytes that fail the sha256 fall through to the index's URLs.
        server.serve("mirror", &format!("v1/files/{sha}"), Reply::Ok(b"not the font".to_vec()));
        user.install_face(&index, face, &Cancel::new(), &mut none).unwrap();
        assert!(user.is_installed(face.family, face.file));

        // A family from a source the user added never goes to the service.
        let added = Index::from_user_bytes(f.index.to_string().as_bytes(), "https://fonts.example.org/index.json").unwrap();
        let face = added.find_face("Test Sans", "Bold").unwrap();
        let before = server.requests().len();
        user.legal_text(&added, face.family, crate::LegalText::License, &Cancel::new()).unwrap();
        user.install_face(&added, face, &Cancel::new(), &mut none).unwrap();
        assert!(server.requests()[before..].iter().all(|r| !r.starts_with("/mirror/")), "{:?}", &server.requests()[before..]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn preview_service_setting_is_user_only_and_https() {
        let dir = temp("service");
        let user = FontStore::new(dir.clone(), Caller::User).with_preview_env(None);
        let agent = FontStore::new(dir.clone(), Caller::Agent).with_preview_env(None);
        assert!(matches!(agent.set_preview_service(Some("https://fonts.example.com")), Err(Error::NotAllowed)));
        assert!(matches!(user.set_preview_service(Some("http://fonts.example.com")), Err(Error::BadServiceUrl(_))));
        assert!(matches!(user.set_preview_service(Some("ftp://fonts.example.com")), Err(Error::BadServiceUrl(_))));
        assert!(matches!(user.set_preview_service(Some("https://user@fonts.example.com")), Err(Error::BadServiceUrl(_))));
        assert!(matches!(user.set_preview_service(Some("http://localhost:8080/")), Err(Error::BadServiceUrl(_))));
        user.set_preview_service(Some("http://[::1]:8080/")).unwrap();
        assert_eq!(crate::check_preview_service(" http://127.0.0.1:9/x/ ").as_deref().ok(), Some("http://127.0.0.1:9/x"));
        assert!(matches!(crate::check_preview_service("https://fonts.example.com/?k=v"), Err(Error::BadServiceUrl(_))));
        user.set_preview_service(Some("https://fonts.example.com/")).unwrap();
        assert_eq!(agent.preview_service().as_deref(), Some("https://fonts.example.com"));
        user.set_preview_service(None).unwrap();
        assert_eq!(agent.preview_service(), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_preview_service_that_refuses_connections_is_tried_once_per_install() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let accepted = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&accepted);
        // Takes each connection and drops it unanswered: a network failure.
        std::thread::spawn(move || {
            for _ in listener.incoming() {
                count.fetch_add(1, Ordering::Relaxed);
            }
        });
        let server = Server::start();
        let f = fixture();
        server.serve_fixture(&f);
        let index = index_of(&f.index).unwrap();
        let family = index.family("Test Sans").unwrap();
        let dir = temp("down-mirror");
        let client = server.client();
        let user = FontStore::new(dir.clone(), Caller::User).with_client(client.clone()).with_preview_env(None);
        user.enable_network().unwrap();
        user.set_preview_service(Some(&format!("http://127.0.0.1:{port}"))).unwrap();
        user.install_family(&index, family, &Cancel::new(), &mut none).unwrap();
        // Three files and a licence, one attempt at the mirror.
        assert_eq!(accepted.load(Ordering::Relaxed), 1);
        assert!(client.service_is_down(&format!("http://127.0.0.1:{port}")));
        assert!(!client.service_is_down("https://fonts.example.com"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn mirror_bytes_that_fail_the_check_are_not_a_corrupt_file_when_the_index_is_unreachable() {
        let server = Server::start();
        let f = fixture();
        let index = index_of(&f.index).unwrap();
        let face = index.find_face("Test Sans", "Bold").unwrap();
        server.serve("mirror", &format!("v1/files/{}", face.file.sha256), Reply::Ok(b"not the font".to_vec()));
        let dir = temp("bad-mirror");
        let user = FontStore::new(dir.clone(), Caller::User).with_client(server.client()).with_preview_env(None);
        user.enable_network().unwrap();
        user.set_preview_service(Some(&format!("{}mirror/", server.base))).unwrap();
        // The index's URLs are not served: they fail, so every reason is reported.
        match user.install_face(&index, face, &Cancel::new(), &mut none) {
            Err(Error::Download { reasons, .. }) => assert!(reasons.len() > 1 && reasons[0].contains("127.0.0.1"), "{reasons:?}"),
            other => panic!("{other:?}"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_client_with_other_timeouts_shares_the_down_state() {
        let client = Client::default();
        let short =
            client.with_timeouts(Timeouts { connect: Duration::from_secs(1), response: Duration::from_secs(1), overall: Duration::from_secs(2) });
        client.mark_service_down("https://fonts.example.com");
        assert!(short.service_is_down("https://fonts.example.com/v1/x"));
    }

    #[test]
    fn a_service_marked_down_does_not_block_another() {
        let server = Server::start();
        server.serve("b", "x", Reply::Ok(b"ok".to_vec()));
        let client = server.client();
        client.mark_service_down("https://fonts.example.com/a");
        assert!(client.service_is_down("https://Fonts.example.com/b"));
        let url = format!("{}b/x", server.base);
        assert_eq!(client.get_from_service(&url, 100, &Cancel::new(), &mut |_| {}).unwrap(), b"ok");
        client.mark_service_down("https://other.example.com");
        assert!(client.service_is_down("https://fonts.example.com/a"));
    }

    #[test]
    fn loopback_http_is_refused_outside_the_preview_service() {
        let server = Server::start();
        server.serve("user", "index.json", Reply::Ok(b"{}".to_vec()));
        let mut client = Client::new(Timeouts::default());
        client.rewrite = vec![("https://fonts.example.org/".into(), server.base.clone())];
        let url = "https://fonts.example.org/user/index.json";
        let got = client.get(url, 1000, &Cancel::new(), &mut |_| {});
        assert!(matches!(got, Err(FetchError::Failed(_))), "{got:?}");
        assert!(server.requests().is_empty());
        assert_eq!(client.get_from_service(url, 1000, &Cancel::new(), &mut |_| {}).unwrap(), b"{}");
    }

    #[test]
    fn previews_are_on_by_default_and_only_the_user_turns_them_off() {
        let dir = temp("previews-setting");
        let user = FontStore::new(dir.clone(), Caller::User);
        let agent = FontStore::new(dir.clone(), Caller::Agent);
        assert!(user.previews_enabled());
        assert!(matches!(agent.set_previews_enabled(false), Err(Error::NotAllowed)));
        user.set_previews_enabled(false).unwrap();
        assert!(!agent.previews_enabled());
        // Other settings don't bring it back.
        user.set_preview_service(Some("https://fonts.example.com")).unwrap();
        assert!(!user.previews_enabled());
        user.set_previews_enabled(true).unwrap();
        assert!(user.previews_enabled());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn preview_service_env_beats_the_setting() {
        let (env, setting) = (Some("https://a.example.com/"), Some("https://b.example.com"));
        assert_eq!(crate::store::resolve_preview_service(env, setting).as_deref(), Some("https://a.example.com"));
        assert_eq!(crate::store::resolve_preview_service(Some(""), setting).as_deref(), Some("https://b.example.com"));
        assert_eq!(crate::store::resolve_preview_service(None, None), None);
        let source = |env, setting| crate::store::resolve_preview_source(env, setting, None).1;
        assert_eq!(
            (source(env, setting), source(Some(" "), setting), source(None, None)),
            (PreviewServiceSource::Env, PreviewServiceSource::Saved, PreviewServiceSource::None)
        );
        assert_eq!(source(Some("http://fonts.example.com"), setting), PreviewServiceSource::None);
        assert_eq!(crate::store::resolve_preview_service(Some("http://fonts.example.com"), setting), None);
        assert_eq!(crate::store::resolve_preview_service(Some("http://127.0.0.1:9"), None).as_deref(), Some("http://127.0.0.1:9"));
        assert_eq!(crate::store::resolve_preview_service(Some("http://[::1]:9/x/"), None).as_deref(), Some("http://[::1]:9/x"));
    }

    #[test]
    fn host_default_preview_service_comes_after_env_and_setting_and_is_never_saved() {
        let dir = temp("preview-default");
        let store =
            |env| FontStore::new(dir.clone(), Caller::User).with_default_preview_service("https://default.example.com/").with_preview_env(env);
        let user = store(None);
        let in_effect = |s: &FontStore| (s.preview_service(), s.preview_service_source());
        assert_eq!(in_effect(&user), (Some("https://default.example.com".into()), PreviewServiceSource::Default));
        user.set_preview_service(Some("https://saved.example.com")).unwrap();
        assert_eq!(in_effect(&user), (Some("https://saved.example.com".into()), PreviewServiceSource::Saved));
        assert_eq!(in_effect(&store(Some("https://env.example.com"))), (Some("https://env.example.com".into()), PreviewServiceSource::Env));
        user.set_preview_service(None).unwrap();
        assert_eq!(in_effect(&user), (Some("https://default.example.com".into()), PreviewServiceSource::Default));
        assert!(!std::fs::read_to_string(dir.join("settings.json")).unwrap().contains("default.example.com"));
        // Another app sharing the store without a default has no service.
        assert_eq!(FontStore::new(dir.clone(), Caller::User).with_preview_env(None).preview_service(), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn saved_network_choice_beats_the_host_default_which_beats_off() {
        let dir = temp("network-default");
        let host = |network| FontStore::new(dir.clone(), Caller::User).with_default_network(network);
        assert_eq!(FontStore::new(dir.clone(), Caller::User).network(), Network::Off);
        assert_eq!(host(Network::On).network(), Network::On);
        assert_eq!(FontStore::new(dir.clone(), Caller::Agent).with_default_network(Network::On).network(), Network::Off);
        // Another setting saved meanwhile is not a network choice.
        host(Network::On).set_previews_enabled(false).unwrap();
        assert_eq!(host(Network::On).network(), Network::On);
        assert!(!std::fs::read_to_string(dir.join("settings.json")).unwrap().contains("network"));
        host(Network::On).disable_network().unwrap();
        assert_eq!(host(Network::On).network(), Network::Off);
        host(Network::Off).enable_network().unwrap();
        assert_eq!(host(Network::Off).network(), Network::On);
        // A file from before the choice could be absent: its "off" is the user's.
        std::fs::write(dir.join("settings.json"), br#"{"network":"off","user_sources":[]}"#).unwrap();
        assert_eq!(host(Network::On).network(), Network::Off);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_settings_file_that_does_not_parse_fails_closed() {
        let dir = temp("settings-broken");
        let host = |network| FontStore::new(dir.clone(), Caller::User).with_default_network(network);
        std::fs::write(dir.join("settings.json"), b"{not json").unwrap();
        // No network and no preview service, not even the host's; a setting saved over the file
        // keeps the network off.
        let broken = host(Network::On).with_default_preview_service("https://default.example.com").with_preview_env(None);
        assert_eq!((broken.network(), broken.preview_service(), broken.previews_enabled()), (Network::Off, None, true));
        broken.set_previews_enabled(true).unwrap();
        assert_eq!(host(Network::On).network(), Network::Off);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn install_verify_and_remove_a_family() {
        let server = Server::start();
        let f = fixture();
        server.serve_fixture(&f);
        let index = index_of(&f.index).unwrap();
        let dir = temp("install");
        let root = temp("install-root");
        let env = FontEnv { os: Os::Unix, root: root.clone(), font_store: Some(dir.clone()), ..Default::default() };
        let catalog = Arc::new(Catalog::new(env).with_font_service(false));
        catalog.rescan();
        assert!(catalog.by_postscript_name("TestSans-Bold").is_none());
        let store = user_store(&server, &dir).with_catalog(catalog.clone());

        let mut seen = Vec::new();
        let family = index.family("Test Sans").unwrap();
        let installed = store.install_family(&index, family, &Cancel::new(), &mut |p| seen.push(p)).unwrap();
        assert_eq!(entries(&dir.join("testsans")), ["OFL.txt", "TestSans-Bold.ttf", "TestSans-Italic.ttf", "TestSans-Regular.ttf"]);
        assert_eq!(installed.files.len(), 3);
        assert_eq!((installed.dir.as_str(), installed.license_file.as_deref()), ("testsans", Some("OFL.txt")));
        assert!(installed.files.iter().all(|x| x.commit == COMMIT && x.sha256.len() == 64));
        assert_eq!(store.installed(), std::slice::from_ref(&installed));
        let last = seen.last().unwrap();
        assert_eq!((last.done, last.total, last.files), (family.size(), family.size(), 3));
        assert!(seen.windows(2).all(|w| w[0].done <= w[1].done));
        // The catalog was rescanned.
        assert_eq!(catalog.by_postscript_name("TestSans-Bold").unwrap().family, "Test Sans");
        assert!(store.verify().is_empty());
        assert!(store.is_installed(family, index.find_face("Test Sans", "Italic").unwrap().file));

        // Installed files aren't downloaded again.
        let before = server.requests().len();
        store.install_face(&index, index.find_face("Test Sans", "Bold").unwrap(), &Cancel::new(), &mut none).unwrap();
        assert_eq!(server.requests().len(), before);

        // Damaged and missing files are reported.
        std::fs::write(dir.join("testsans/TestSans-Italic.ttf"), b"damaged").unwrap();
        std::fs::remove_file(dir.join("testsans/TestSans-Regular.ttf")).unwrap();
        let problems = store.verify();
        assert_eq!(problems.len(), 2);
        assert!(problems.iter().any(|p| p.file == "TestSans-Italic.ttf" && matches!(p.error, Some(VerifyError::Size { .. }))));
        assert!(problems.iter().any(|p| p.file == "TestSans-Regular.ttf" && p.error.is_none()));

        // A single face of another family, with its licence.
        let face = index.find_face("Test Var", "Bold").unwrap();
        store.install_face(&index, face, &Cancel::new(), &mut none).unwrap();
        assert_eq!(entries(&dir.join("testvar")), ["LICENSE.txt", "TestVar[wght].ttf"]);
        assert_eq!(store.installed().iter().map(|f| f.family.as_str()).collect::<Vec<_>>(), ["Test Sans", "Test Var"]);

        // One style out, then the last one takes its family with it.
        store.remove_files(GOOGLE_FONTS, "Test Sans", &["TestSans-Bold.ttf".into()]).unwrap();
        assert!(!dir.join("testsans/TestSans-Bold.ttf").exists());
        assert_eq!(store.installed_family(GOOGLE_FONTS, "Test Sans").unwrap().files.len(), 2);
        store.remove_files(GOOGLE_FONTS, "Test Var", &["TestVar[wght].ttf".into()]).unwrap();
        assert!(!dir.join("testvar").exists());
        assert!(store.installed_family(GOOGLE_FONTS, "Test Var").is_none());

        store.remove_family(GOOGLE_FONTS, "test sans").unwrap();
        assert!(!dir.join("testsans").exists());
        assert!(store.installed().is_empty());
        assert!(catalog.by_postscript_name("TestSans-Bold").is_none());
        assert!(matches!(store.remove_family(GOOGLE_FONTS, "Test Sans"), Err(Error::NotInstalled(_))));
        assert!(entries(&dir.join(".staging")).is_empty());
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn corrupt_downloads_are_rejected_and_leave_nothing() {
        let server = Server::start();
        let f = fixture();
        server.serve_fixture(&f);
        let index = index_of(&f.index).unwrap();
        let dir = temp("corrupt");
        let store = user_store(&server, &dir);

        // Wrong bytes of the right size on both hosts: rejected on the sha256.
        let mut other = font("Test Sans", "Italic", 400);
        *other.last_mut().unwrap() ^= 1;
        for (host, path) in [("cdn", format!("gh/google/fonts@{COMMIT}")), ("raw", format!("google/fonts/{COMMIT}"))] {
            server.serve(host, &format!("{path}/ofl/testsans/TestSans-Italic.ttf"), Reply::Ok(other.clone()));
        }
        let family = index.family("Test Sans").unwrap();
        let r = store.install_family(&index, family, &Cancel::new(), &mut none);
        assert!(matches!(r, Err(Error::Verify { error: VerifyError::Sha256, .. })), "{r:?}");
        assert_no_family_files(&dir);
        assert!(store.installed().is_empty());

        // Bytes that match the index but aren't a font.
        let junk = b"this is not a font file".to_vec();
        let mut v = f.index.clone();
        v["families"][2]["files"][0]["sha256"] = json!(sha256_hex(&junk));
        v["families"][2]["files"][0]["size"] = json!(junk.len());
        server.serve("cdn", &format!("gh/google/fonts@{COMMIT}/ofl/testsans/TestSans-Bold.ttf"), Reply::Ok(junk.clone()));
        server.serve("raw", &format!("google/fonts/{COMMIT}/ofl/testsans/TestSans-Bold.ttf"), Reply::Ok(junk));
        let junk_index = index_of(&v).unwrap();
        let r = store.install_face(&junk_index, junk_index.find_face("Test Sans", "Bold").unwrap(), &Cancel::new(), &mut none);
        assert!(matches!(r, Err(Error::Verify { error: VerifyError::NotAFont, .. })), "{r:?}");
        assert_no_family_files(&dir);

        // A file missing on both hosts, after others downloaded: nothing of the family is written.
        server.serve_fixture(&f);
        for (host, path) in [("cdn", format!("gh/google/fonts@{COMMIT}")), ("raw", format!("google/fonts/{COMMIT}"))] {
            server.serve(host, &format!("{path}/ofl/testsans/TestSans-Regular.ttf"), Reply::Status(404));
        }
        let r = store.install_family(&index, family, &Cancel::new(), &mut none);
        assert!(matches!(&r, Err(Error::Download { reasons, .. }) if reasons.len() == 2), "{r:?}");
        assert_no_family_files(&dir);
        assert!(!dir.join("manifest.json").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        use flate2::write::GzEncoder;
        let mut e = GzEncoder::new(Vec::new(), flate2::Compression::best());
        e.write_all(bytes).unwrap();
        e.finish().unwrap()
    }

    /// `sources.json` listing the Google Fonts fixture (gzipped) and the Noto fixture (plain
    /// JSON), with the index bytes each path serves.
    fn sources_fixture(noto_sha256: Option<&str>) -> (Vec<u8>, Vec<(String, Vec<u8>)>) {
        let google = fixture().index.to_string().into_bytes();
        let google_gz = gzip(&google);
        let noto = noto_fixture().index.to_string().into_bytes();
        let sources = json!({"schema": 1, "version": "1.1", "indexes": [
            {"source": GOOGLE_FONTS, "name": "Google Fonts", "curated": true, "path": "indexes/google-fonts.json",
             "size": google.len(), "sha256": sha256_hex(&google), "gz_path": "indexes/google-fonts.json.gz",
             "gz_size": google_gz.len(), "gz_sha256": sha256_hex(&google_gz), "commit": COMMIT, "families": 4, "files": 6},
            {"source": "notofonts", "name": "Noto", "curated": true, "path": "indexes/noto.json",
             "size": noto.len(), "sha256": noto_sha256.map_or_else(|| sha256_hex(&noto), str::to_string), "commit": NOTO_COMMIT}
        ]});
        let files = vec![("indexes/google-fonts.json.gz".into(), google_gz), ("indexes/noto.json".into(), noto)];
        (sources.to_string().into_bytes(), files)
    }

    fn pin_of(sources: &[u8]) -> IndexPin {
        IndexPin { repo: "owner/index", commit: "abc", sha256: Box::leak(sha256_hex(sources).into_boxed_str()) }
    }

    fn serve_sources(server: &Server, sources: &[u8], files: &[(String, Vec<u8>)]) {
        server.serve("cdn", "gh/owner/index@abc/sources.json", Reply::Status(500));
        server.serve("raw", "owner/index/abc/sources.json", Reply::Ok(sources.to_vec()));
        for (path, bytes) in files {
            server.serve("cdn", &format!("gh/owner/index@abc/{path}"), Reply::Ok(bytes.clone()));
        }
    }

    #[test]
    fn the_indexes_are_fetched_only_with_consent_and_checked() {
        let server = Server::start();
        let (sources, files) = sources_fixture(None);
        let pin = pin_of(&sources);
        serve_sources(&server, &sources, &files);
        let dir = temp("index");
        let store = FontStore::new(dir.clone(), Caller::User).with_client(server.client());
        assert!(store.cached_indexes(&pin).is_none());
        assert!(matches!(store.fetch_indexes(&pin, &Cancel::new()), Err(Error::NetworkOff)));
        assert!(server.requests().is_empty());

        store.enable_network().unwrap();
        let index = store.fetch_indexes(&pin, &Cancel::new()).unwrap();
        assert!(index.problems().is_empty(), "{:?}", index.problems());
        assert_eq!(index.sources().iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), [GOOGLE_FONTS, "notofonts"]);
        assert_eq!(index.families().len(), 7);
        assert_eq!(entries(&dir.join("indexes")), ["google-fonts.json.gz", "notofonts.json", "sources.json"]);
        let cached = store.cached_indexes(&pin).unwrap();
        assert_eq!((cached.families().len(), cached.sources().len()), (7, 2));

        let wrong = IndexPin { sha256: "0000000000000000000000000000000000000000000000000000000000000000", ..pin };
        assert!(store.cached_indexes(&wrong).is_none());
        assert!(matches!(store.fetch_indexes(&wrong, &Cancel::new()), Err(Error::Verify { error: VerifyError::Sha256, .. })));

        // A cached index that was changed is not used.
        std::fs::write(dir.join("indexes/notofonts.json"), b"{}").unwrap();
        assert!(store.cached_indexes(&pin).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A store with Test Sans installed, its catalog and an OS user font folder, all in temporary
    /// folders: the store, the system root (`<root>/usr/share/fonts`) and the home folder.
    fn shared_store(server: &Server, name: &str) -> (FontStore, PathBuf, PathBuf, PathBuf) {
        let index = index_of(&fixture().index).unwrap();
        let dir = temp(name);
        let root = temp(&format!("{name}-root"));
        let env = FontEnv { os: Os::Unix, root: root.clone(), home: Some(root.join("home")), font_store: Some(dir.clone()), ..Default::default() };
        let catalog = Arc::new(Catalog::new(env).with_font_service(false));
        let shared = root.join("home/.local/share/fonts/Craft");
        let store = user_store(server, &dir).with_catalog(catalog).with_os_fonts(OsFontFolder::at(&shared, OsFontLayout::Folders));
        store.install_family(&index, index.family("Test Sans").unwrap(), &Cancel::new(), &mut none).unwrap();
        (store, dir, root, shared)
    }

    fn ledger(dir: &Path) -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(dir.join("os-fonts.json")).unwrap()).unwrap()
    }

    /// Each copy in `family` matches the store's file, and the ledger lists exactly these copies.
    fn assert_copies_match_store_and_ledger(dir: &Path, family: &Path, names: &[&str]) {
        for name in names {
            assert_eq!(std::fs::read(family.join(name)).unwrap(), std::fs::read(dir.join("testsans").join(name)).unwrap());
        }
        let l = ledger(dir);
        assert_eq!((l["version"].as_u64(), l["families"][0]["family"].as_str()), (Some(1), Some("Test Sans")));
        let files = l["families"][0]["files"].as_array().unwrap();
        assert_eq!(files.len(), names.len());
        for f in files {
            let path = PathBuf::from(f["path"].as_str().unwrap());
            assert_eq!(f["sha256"].as_str().unwrap(), sha256_hex(&std::fs::read(&path).unwrap()));
            assert_eq!((f["state"].as_str(), f["mechanism"].as_str()), (Some("active"), Some("copy")));
        }
        assert_eq!(files[0]["postscript"], json!(["TestSans-Bold"]));
    }

    #[test]
    fn read_only_store_files_are_made_available() {
        let server = Server::start();
        server.serve_fixture(&fixture());
        let (store, dir, root, shared) = shared_store(&server, "os-fonts-read-only");
        let set_read_only = |read_only: bool| {
            for name in entries(&dir.join("testsans")) {
                let path = dir.join("testsans").join(name);
                let mut permissions = std::fs::metadata(&path).unwrap().permissions();
                permissions.set_readonly(read_only);
                std::fs::set_permissions(&path, permissions).unwrap();
            }
        };
        set_read_only(true);
        let copies = store.make_available(GOOGLE_FONTS, "Test Sans");
        set_read_only(false);
        assert_eq!(copies.unwrap().len(), 3, "a read-only store file still copies");
        assert!(entries(&shared.join("testsans")).iter().all(|name| !name.starts_with('.')), "no copy is left aside");
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_family_made_available_to_all_apps_is_withdrawn_exactly() {
        let server = Server::start();
        server.serve_fixture(&fixture());
        let (store, dir, root, shared) = shared_store(&server, "os-fonts");
        let family = shared.join("testsans");
        assert!(store.available_to_all_supported());
        assert!(!store.is_available(GOOGLE_FONTS, "Test Sans"));
        assert!(matches!(store.make_available(GOOGLE_FONTS, "Test Var"), Err(Error::NotInstalled(_))));

        let copies = store.make_available(GOOGLE_FONTS, "Test Sans").unwrap();
        let names = ["TestSans-Bold.ttf", "TestSans-Italic.ttf", "TestSans-Regular.ttf"];
        assert_eq!(entries(&family), names, "fonts only: no licence, no temporary files");
        assert_eq!(copies, names.map(|n| family.join(n)));
        assert_copies_match_store_and_ledger(&dir, &family, &names);
        assert!(store.is_available(GOOGLE_FONTS, "test sans"));
        // Again: the copies, which the catalog now lists as system fonts, aren't conflicts.
        assert_eq!(store.make_available(GOOGLE_FONTS, "Test Sans").unwrap(), copies);

        // A crash after a copy was renamed into place, before the ledger said so: the next change
        // settles the entry and deletes the temporary file.
        std::fs::write(family.join(".craft-TestSans-Regular.ttf.part"), b"half").unwrap();
        let mut l = ledger(&dir);
        l["families"][0]["files"][2]["state"] = json!("adding");
        std::fs::write(dir.join("os-fonts.json"), l.to_string()).unwrap();
        store.make_available(GOOGLE_FONTS, "Test Sans").unwrap();
        assert_eq!(entries(&family), names);
        assert!(ledger(&dir)["families"][0]["files"].as_array().unwrap().iter().all(|f| f["state"] == "active"));

        // Withdrawing leaves a changed copy and files the store didn't add, and reports the copy.
        let italic = family.join("TestSans-Italic.ttf");
        std::fs::write(&italic, b"the user's own").unwrap();
        std::fs::write(family.join("Unlisted.ttf"), b"x").unwrap();
        assert_eq!(store.withdraw(GOOGLE_FONTS, "Test Sans").unwrap(), std::slice::from_ref(&italic));
        assert_eq!(entries(&family), ["TestSans-Italic.ttf", "Unlisted.ttf"]);
        assert!(!store.is_available(GOOGLE_FONTS, "Test Sans"));
        assert_eq!(ledger(&dir)["families"], json!([]));
        assert!(store.verify().is_empty(), "the store's own files are untouched");

        // The changed file is no longer the store's: it sits where a copy would go.
        match store.make_available(GOOGLE_FONTS, "Test Sans") {
            Err(Error::Conflict(c)) => assert_eq!(c, [Conflict { postscript: None, path: italic.clone() }]),
            other => panic!("{other:?}"),
        }
        std::fs::remove_file(&italic).unwrap();
        std::fs::remove_file(family.join("Unlisted.ttf")).unwrap();

        // Removing the family from the store withdraws it.
        store.make_available(GOOGLE_FONTS, "Test Sans").unwrap();
        assert!(store.copies_out_of_reach(GOOGLE_FONTS, "Test Sans").is_empty());
        store.remove_family(GOOGLE_FONTS, "Test Sans").unwrap();
        assert!(!family.exists());
        assert!(!store.is_available(GOOGLE_FONTS, "Test Sans"));

        // A build that can't change the OS folder removes the store's files and leaves the copies
        // it can't reach, still listed, for a build that can.
        let index = index_of(&fixture().index).unwrap();
        store.install_family(&index, index.family("Test Sans").unwrap(), &Cancel::new(), &mut none).unwrap();
        store.make_available(GOOGLE_FONTS, "Test Sans").unwrap();
        let blind = FontStore::new(dir.clone(), Caller::User);
        let out_of_reach = blind.copies_out_of_reach(GOOGLE_FONTS, "Test Sans");
        assert_eq!(out_of_reach, names.map(|n| family.join(n)));
        blind.remove_family(GOOGLE_FONTS, "Test Sans").unwrap();
        assert!(blind.installed_family(GOOGLE_FONTS, "Test Sans").is_none());
        assert!(out_of_reach.iter().all(|c| c.is_file()));
        assert!(blind.is_available(GOOGLE_FONTS, "Test Sans"));
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn making_available_refuses_postscript_conflicts_and_agents() {
        let server = Server::start();
        server.serve_fixture(&fixture());
        let (store, dir, root, shared) = shared_store(&server, "os-fonts-conflict");
        let agent = FontStore::new(dir.clone(), Caller::Agent).with_os_fonts(OsFontFolder::at(&shared, OsFontLayout::Folders));
        assert!(matches!(agent.make_available(GOOGLE_FONTS, "Test Sans"), Err(Error::NotAllowed)));
        assert!(matches!(agent.withdraw(GOOGLE_FONTS, "Test Sans"), Err(Error::NotAllowed)));
        assert!(!FontStore::new(dir.clone(), Caller::User).available_to_all_supported());

        // A different font of the same PostScript name in a system folder.
        let system = root.join("usr/share/fonts");
        std::fs::create_dir_all(&system).unwrap();
        let other = system.join("TestSans-Bold.ttf");
        std::fs::write(&other, font("Test Sans", "Bold", 600)).unwrap();
        match store.make_available(GOOGLE_FONTS, "Test Sans") {
            Err(Error::Conflict(c)) => assert_eq!(c, [Conflict { postscript: Some("TestSans-Bold".into()), path: other.clone() }]),
            other => panic!("{other:?}"),
        }
        assert!(!shared.exists(), "nothing copied");
        assert!(!store.is_available(GOOGLE_FONTS, "Test Sans"));

        // The same file is no conflict: the system has it, so it isn't copied or recorded.
        std::fs::write(&other, std::fs::read(dir.join("testsans/TestSans-Bold.ttf")).unwrap()).unwrap();
        let copies = store.make_available(GOOGLE_FONTS, "Test Sans").unwrap();
        assert_eq!(copies, ["TestSans-Italic.ttf", "TestSans-Regular.ttf"].map(|n| shared.join("testsans").join(n)));
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(root);
    }

    /// A catalog of the same system as [`shared_store`]'s, to look at what a scan sees.
    #[cfg(unix)]
    fn catalog_of(dir: &Path, root: &Path) -> Catalog {
        let env = FontEnv {
            os: Os::Unix,
            root: root.to_path_buf(),
            home: Some(root.join("home")),
            font_store: Some(dir.to_path_buf()),
            ..Default::default()
        };
        Catalog::new(env).with_font_service(false)
    }

    #[test]
    #[cfg(unix)]
    fn copies_reached_by_another_path_are_still_the_stores_and_links_are_refused() {
        let server = Server::start();
        server.serve_fixture(&fixture());
        let (store, dir, root, shared) = shared_store(&server, "os-fonts-alias");
        // As Flatpak's /run/host/user-fonts: the user's font folder again, scanned first.
        let fonts = root.join("home/.local/share/fonts");
        std::fs::create_dir_all(&fonts).unwrap();
        std::fs::create_dir_all(root.join("usr/share/fonts")).unwrap();
        std::os::unix::fs::symlink(&fonts, root.join("usr/share/fonts/user-fonts")).unwrap();

        let copies = store.make_available(GOOGLE_FONTS, "Test Sans").unwrap();
        assert_eq!(copies.len(), 3);
        let seen = catalog_of(&dir, &root);
        seen.rescan();
        assert!(seen.scanned_faces().iter().any(|f| f.path.as_ref().is_some_and(|p| p.starts_with(root.join("usr/share/fonts/user-fonts")))));
        assert_eq!(store.make_available(GOOGLE_FONTS, "Test Sans").unwrap(), copies);
        assert_eq!(store.make_available(GOOGLE_FONTS, "Test Sans").unwrap(), copies);
        assert!(copies.iter().all(|c| c.is_file()));
        assert!(store.pending_files(GOOGLE_FONTS, "Test Sans").is_empty());

        // A family folder that is a link leads outside: nothing is written or deleted through it.
        let outside = temp("os-fonts-outside");
        let mut l = ledger(&dir);
        l["families"][0]["files"][0]["path"] = json!(outside.join("TestSans-Bold.ttf"));
        std::fs::write(dir.join("os-fonts.json"), l.to_string()).unwrap();
        std::fs::copy(&copies[0], outside.join("TestSans-Bold.ttf")).unwrap();
        let kept = store.withdraw(GOOGLE_FONTS, "Test Sans").unwrap();
        assert_eq!(kept, [outside.join("TestSans-Bold.ttf")], "a path the store doesn't put copies at is left alone");
        assert!(outside.join("TestSans-Bold.ttf").is_file());
        std::fs::remove_file(&copies[0]).unwrap();
        std::fs::remove_dir(shared.join("testsans")).unwrap();
        std::os::unix::fs::symlink(&outside, shared.join("testsans")).unwrap();
        assert!(matches!(store.make_available(GOOGLE_FONTS, "Test Sans"), Err(Error::Conflict(_))));
        assert_eq!(entries(&outside), ["TestSans-Bold.ttf"]);
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }

    #[test]
    fn a_ledger_that_cant_be_understood_is_kept_and_blocks_changes() {
        let server = Server::start();
        server.serve_fixture(&fixture());
        let (store, dir, root, shared) = shared_store(&server, "os-fonts-bad-ledger");
        store.make_available(GOOGLE_FONTS, "Test Sans").unwrap();
        let mut l = ledger(&dir);
        l["families"][0]["files"][0]["mechanism"] = json!("hardlink");
        let bytes = l.to_string();
        std::fs::write(dir.join("os-fonts.json"), &bytes).unwrap();

        assert!(!store.is_available(GOOGLE_FONTS, "Test Sans"));
        assert!(matches!(store.make_available(GOOGLE_FONTS, "Test Sans"), Err(Error::Ledger(_))));
        assert!(matches!(store.withdraw(GOOGLE_FONTS, "Test Sans"), Err(Error::Ledger(_))));
        assert!(matches!(store.remove_family(GOOGLE_FONTS, "Test Sans"), Err(Error::Ledger(_))));
        assert_eq!(std::fs::read_to_string(dir.join("os-fonts.json")).unwrap(), bytes);
        assert_eq!(entries(&shared.join("testsans")).len(), 3);
        assert!(store.installed_family(GOOGLE_FONTS, "Test Sans").is_some());
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn store_families_and_named_instances_conflict_by_postscript_name() {
        let server = Server::start();
        server.serve_fixture(&fixture());
        let index = index_of(&fixture().index).unwrap();
        let (store, dir, root, shared) = shared_store(&server, "os-fonts-families");
        for name in ["Shared One", "Shared Two", "Test Var"] {
            store.install_family(&index, index.family(name).unwrap(), &Cancel::new(), &mut none).unwrap();
        }
        // Both families' index entries name the face Shared-Regular.
        store.make_available(GOOGLE_FONTS, "Shared One").unwrap();
        match store.make_available(GOOGLE_FONTS, "Shared Two") {
            Err(Error::Conflict(c)) => {
                assert_eq!(c, [Conflict { postscript: Some("Shared-Regular".into()), path: shared.join("sharedone/SharedOne-Regular.ttf") }])
            }
            other => panic!("{other:?}"),
        }
        // A static system face named as one of a variable font's instances.
        let system = root.join("usr/share/fonts");
        std::fs::create_dir_all(&system).unwrap();
        std::fs::write(system.join("TestVar-Bold.ttf"), font("Test Var", "Bold", 700)).unwrap();
        match store.make_available(GOOGLE_FONTS, "Test Var") {
            Err(Error::Conflict(c)) => assert_eq!(c, [Conflict { postscript: Some("TestVar-Bold".into()), path: system.join("TestVar-Bold.ttf") }]),
            other => panic!("{other:?}"),
        }
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_deleted_copy_is_pending_and_copied_again() {
        let server = Server::start();
        server.serve_fixture(&fixture());
        let (store, dir, root, _shared) = shared_store(&server, "os-fonts-deleted");
        let copies = store.make_available(GOOGLE_FONTS, "Test Sans").unwrap();
        std::fs::remove_file(&copies[0]).unwrap();
        assert_eq!(store.pending_files(GOOGLE_FONTS, "Test Sans").len(), 1);
        assert_eq!(store.make_available(GOOGLE_FONTS, "Test Sans").unwrap(), copies);
        assert!(copies.iter().all(|c| c.is_file()));
        assert!(store.pending_files(GOOGLE_FONTS, "Test Sans").is_empty());
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_agent_never_changes_the_os_font_folder() {
        let server = Server::start();
        server.serve_fixture(&fixture());
        let index = index_of(&fixture().index).unwrap();
        let (store, dir, root, shared) = shared_store(&server, "os-fonts-agent");
        store.remove_files(GOOGLE_FONTS, "Test Sans", &["TestSans-Italic.ttf".into()]).unwrap();
        store.make_available(GOOGLE_FONTS, "Test Sans").unwrap();
        let family = shared.join("testsans");
        let before = entries(&family);
        assert_eq!(before, ["TestSans-Bold.ttf", "TestSans-Regular.ttf"]);

        // An agent's install stays in the store, listed as not yet available.
        let agent =
            FontStore::new(dir.clone(), Caller::Agent).with_client(server.client()).with_os_fonts(OsFontFolder::at(&shared, OsFontLayout::Folders));
        agent.install_face(&index, index.find_face("Test Sans", "Italic").unwrap(), &Cancel::new(), &mut none).unwrap();
        assert_eq!(entries(&family), before);
        assert_eq!(store.pending_files(GOOGLE_FONTS, "Test Sans"), ["TestSans-Italic.ttf"]);

        // An agent can't remove what has copies; the user can.
        assert!(matches!(agent.remove_family(GOOGLE_FONTS, "Test Sans"), Err(Error::NotAllowed)));
        assert!(matches!(agent.remove_files(GOOGLE_FONTS, "Test Sans", &["TestSans-Bold.ttf".into()]), Err(Error::NotAllowed)));
        assert_eq!(store.installed_family(GOOGLE_FONTS, "Test Sans").unwrap().files.len(), 3);
        assert_eq!(entries(&family), before);
        agent.remove_files(GOOGLE_FONTS, "Test Sans", &["TestSans-Italic.ttf".into()]).unwrap();

        store.install_face(&index, index.find_face("Test Sans", "Italic").unwrap(), &Cancel::new(), &mut none).unwrap();
        assert_eq!(entries(&family), ["TestSans-Bold.ttf", "TestSans-Italic.ttf", "TestSans-Regular.ttf"], "the user's install copies it");
        assert!(store.pending_files(GOOGLE_FONTS, "Test Sans").is_empty());
        store.remove_family(GOOGLE_FONTS, "Test Sans").unwrap();
        assert!(!family.exists());
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_os_folder_follows_the_platform_and_flatpak_grants() {
        let root = temp("os-fonts-env");
        let home = root.join("home");
        let flatpak = FontEnv { os: Os::Unix, root: root.clone(), home: Some(home.clone()), flatpak: true, ..Default::default() };
        let with = |vars: &[(&str, &str)], env: &FontEnv| {
            let vars: Vec<(String, String)> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            OsFontFolder::for_env_with(env, &move |k| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.into())).map(|f| f.dir().to_path_buf())
        };
        let info = |filesystems: &str| std::fs::write(root.join(".flatpak-info"), format!("[Context]\nfilesystems={filesystems}\n")).unwrap();

        let unix = FontEnv { flatpak: false, xdg_data_home: Some(root.join("data")), ..flatpak.clone() };
        assert_eq!(with(&[], &unix), Ok(root.join("data/fonts/Craft")));
        assert!(with(&[("SNAP", "/snap/x")], &unix).is_err());
        let mac = FontEnv { os: Os::MacOs, ..unix.clone() };
        assert_eq!(with(&[], &mac), Ok(home.join("Library/Fonts")));
        assert!(with(&[("APP_SANDBOX_CONTAINER_ID", "x")], &mac).is_err());
        let windows = FontEnv { os: Os::Windows, local_app_data: Some(root.join("Local")), ..unix.clone() };
        assert_eq!(with(&[], &windows), Ok(root.join("Local/Microsoft/Windows/Fonts/Craft")));
        assert_eq!(with(&[], &FontEnv { local_app_data: None, ..windows.clone() }), Ok(home.join("AppData/Local/Microsoft/Windows/Fonts/Craft")));

        info("xdg-data/fonts:create;");
        assert_eq!(with(&[], &flatpak), Ok(home.join(".local/share/fonts/Craft")));
        info("xdg-data/fonts:create;!xdg-data/fonts;");
        assert!(with(&[], &flatpak).is_err(), "revoked");
        info("home:ro;");
        assert!(with(&[], &flatpak).is_err(), "read only");
        info("~/.local/share/fonts;");
        assert!(with(&[], &flatpak).is_ok());
        // Absolute on every platform: `/elsewhere` isn't on Windows.
        let elsewhere = root.join("elsewhere");
        let host = [("HOST_XDG_DATA_HOME", elsewhere.to_str().unwrap())];
        assert!(with(&host, &flatpak).is_err(), "the grant is for another folder");
        info("xdg-data/fonts;");
        assert_eq!(with(&host, &flatpak), Ok(elsewhere.join("fonts/Craft")));
        let _ = std::fs::remove_dir_all(root);
    }
}
