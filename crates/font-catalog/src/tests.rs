use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use write_fonts::FontBuilder;
use write_fonts::tables::fvar::{AxisInstanceArrays, Fvar, InstanceRecord, VariationAxisRecord};
use write_fonts::tables::name::{Name, NameRecord};
use write_fonts::tables::os2::{Os2, SelectionFlags};
use write_fonts::types::{Fixed, NameId, Tag};

use crate::*;

/// A test font: only the tables the catalog reads, and an empty `glyf` so it counts as drawable.
#[derive(Clone)]
struct Spec {
    family: &'static str,
    style: &'static str,
    /// Legacy family and style (name IDs 1 and 2); the names above are then typographic.
    legacy: Option<(&'static str, &'static str)>,
    /// Japanese family name.
    ja: Option<&'static str>,
    weight: u16,
    width: u16,
    italic: bool,
    fs_type: u16,
    /// Named instances: style and `wght`; the font gets a `wght` axis from 100 to 900, default 400.
    instances: Vec<(&'static str, f64)>,
    outlines: bool,
}

fn spec(family: &'static str, style: &'static str, weight: u16) -> Spec {
    Spec { family, style, legacy: None, ja: None, weight, width: 5, italic: false, fs_type: 0, instances: vec![], outlines: true }
}

fn font(s: &Spec) -> Vec<u8> {
    let mut records = Vec::new();
    let mut add = |lang: u16, id: u16, text: String| records.push(NameRecord::new(3, 1, lang, NameId::new(id), text.into()));
    let ps = format!("{}-{}", s.family.replace(' ', ""), s.style.replace(' ', ""));
    match s.legacy {
        Some((family, style)) => {
            add(0x409, 1, family.into());
            add(0x409, 2, style.into());
            add(0x409, 16, s.family.into());
            add(0x409, 17, s.style.into());
        }
        None => {
            add(0x409, 1, s.family.into());
            add(0x409, 2, s.style.into());
        }
    }
    add(0x409, 4, format!("{} {}", s.family, s.style));
    add(0x409, 6, ps);
    if let Some(ja) = s.ja {
        add(0x411, if s.legacy.is_some() { 16 } else { 1 }, ja.into());
    }
    let mut instances = Vec::new();
    for (next_id, (style, wght)) in (256..).zip(&s.instances) {
        add(0x409, next_id, (*style).into());
        instances.push(InstanceRecord {
            subfamily_name_id: NameId::new(next_id),
            flags: 0,
            coordinates: vec![Fixed::from_f64(*wght)],
            post_script_name_id: None,
        });
    }
    records.sort_by_key(|r| (r.platform_id, r.encoding_id, r.language_id, r.name_id));
    let mut builder = FontBuilder::new();
    builder.add_table(&Name::new(records)).unwrap();
    let os2 = Os2 {
        us_weight_class: s.weight,
        us_width_class: s.width,
        fs_type: s.fs_type,
        fs_selection: if s.italic { SelectionFlags::ITALIC } else { SelectionFlags::REGULAR },
        ul_code_page_range_1: Some(1),
        ul_code_page_range_2: Some(0),
        ..Default::default()
    };
    builder.add_table(&os2).unwrap();
    if !instances.is_empty() {
        let axis = VariationAxisRecord {
            axis_tag: Tag::new(b"wght"),
            min_value: Fixed::from_f64(100.0),
            default_value: Fixed::from_f64(400.0),
            max_value: Fixed::from_f64(900.0),
            flags: 0,
            axis_name_id: NameId::new(255),
        };
        builder.add_table(&Fvar::new(AxisInstanceArrays::new(vec![axis], instances))).unwrap();
    }
    if s.outlines {
        builder.add_raw(Tag::new(b"glyf"), vec![0u8; 4]);
    }
    builder.build()
}

/// A font collection of `fonts`: their table directories, with offsets moved to the new file.
fn collection(fonts: &[Vec<u8>]) -> Vec<u8> {
    let header = 12 + 4 * fonts.len();
    let mut out = Vec::new();
    out.extend_from_slice(b"ttcf");
    out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    out.extend_from_slice(&(fonts.len() as u32).to_be_bytes());
    let mut base = header;
    let mut bodies = Vec::new();
    for f in fonts {
        out.extend_from_slice(&(base as u32).to_be_bytes());
        let mut f = f.clone();
        let tables = u16::from_be_bytes([f[4], f[5]]) as usize;
        for t in 0..tables {
            let at = 12 + 16 * t + 8;
            let offset = u32::from_be_bytes(f[at..at + 4].try_into().unwrap()) + base as u32;
            f[at..at + 4].copy_from_slice(&offset.to_be_bytes());
        }
        base += f.len();
        bodies.push(f);
    }
    for b in bodies {
        out.extend(b);
    }
    out
}

fn temp(name: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!("craft-font-catalog-{name}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(path: &Path, data: impl AsRef<[u8]>) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, data).unwrap();
}

/// A Linux system under `root` with no fontconfig configuration and no other XDG folders.
fn linux_env(root: &Path) -> FontEnv {
    FontEnv {
        os: Os::Unix,
        root: root.to_path_buf(),
        home: Some(root.join("home")),
        xdg_data_dirs: Some(root.join("share").into_os_string()),
        ..Default::default()
    }
}

// Uses Unix paths, which Windows doesn't treat as absolute (#619).
#[cfg(unix)]
#[test]
fn linux_font_dirs_cover_xdg_flatpak_and_fontconfig() {
    let root = temp("dirs");
    write(&root.join("etc/fonts/fonts.conf"), "<fontconfig><dir>/opt/texmf/fonts</dir><dir>/usr/share/fonts</dir></fontconfig>");
    let mut env = linux_env(&root);
    env.xdg_data_home = Some(root.join("data"));
    let dirs = font_dirs(&env);
    let want: Vec<PathBuf> = vec![
        root.join("usr/share/fonts"),
        root.join("usr/local/share/fonts"),
        root.join("run/host/fonts"),
        root.join("run/host/local-fonts"),
        root.join("run/host/user-fonts"),
        root.join("home/.local/share/fonts"),
        root.join("home/.fonts"),
        root.join("data/fonts"),
        root.join(root.join("share").strip_prefix("/").unwrap()).join("fonts"),
        root.join("opt/texmf/fonts"),
    ];
    assert_eq!(dirs, want);
}

// Uses Unix paths, which Windows doesn't treat as absolute (#619).
#[cfg(unix)]
#[test]
fn fontconfig_dirs_follow_includes_prefixes_and_skip_the_rest() {
    let root = temp("conf");
    let etc = root.join("etc/fonts");
    write(
        &etc.join("fonts.conf"),
        r#"<?xml version="1.0"?>
<fontconfig>
  <!-- <dir>/commented/out</dir> -->
  <dir>/sys/fonts</dir>
  <dir prefix="xdg">fonts</dir>
  <dir>~/.fonts</dir>
  <cachedir>/var/cache/fontconfig</cachedir>
  <dir prefix="relative">local-fonts</dir>
  <dir salt="x">/a&amp;b/fonts</dir>
  <include ignore_missing="yes">conf.d</include>
  <include ignore_missing="yes">missing.conf</include>
  <include>fonts.conf</include>
</fontconfig>"#,
    );
    write(&etc.join("conf.d/60-remap.conf"), "<fontconfig><remap-dir as-path=\"/usr/share/fonts\">/remapped/fonts</remap-dir></fontconfig>");
    write(&etc.join("conf.d/README"), "<dir>/not/a/conf/file</dir>");
    write(&etc.join("conf.d/nodigit.conf"), "<fontconfig><dir>/no/digit</dir></fontconfig>");
    write(&etc.join("b.conf"), "<fontconfig><include>fonts.conf</include><dir>/unclosed");
    write(
        &root.join("home/.config/fontconfig/fonts.conf"),
        "<fontconfig><dir>/user/conf/fonts</dir><include>../../../etc/fonts/b.conf</include></fontconfig>",
    );
    let home = root.join("home");
    assert_eq!(
        fontconfig_dirs(&linux_env(&root)),
        vec![
            root.join("sys/fonts"),
            home.join(".local/share/fonts"),
            home.join(".fonts"),
            etc.join("local-fonts"),
            root.join("a&b/fonts"),
            root.join("remapped/fonts"),
            root.join("user/conf/fonts"),
        ]
    );
}

// Uses Unix paths, which Windows doesn't treat as absolute (#619).
#[cfg(unix)]
#[test]
fn flatpak_adds_the_hosts_font_folders() {
    let root = temp("flatpak");
    write(&root.join("run/host/font-dirs.xml"), "<fontconfig><remap-dir as-path=\"/usr/share/fonts\">/run/host/fonts</remap-dir></fontconfig>");
    write(&root.join("run/host/etc/fonts/fonts.conf"), "<fontconfig><dir>/usr/share/texmf/fonts</dir><dir>~/MyFonts</dir></fontconfig>");
    let mut env = linux_env(&root);
    assert!(fontconfig_dirs(&env).is_empty());
    env.flatpak = true;
    assert_eq!(fontconfig_dirs(&env), vec![root.join("run/host/fonts"), root.join("run/host/usr/share/texmf/fonts"), root.join("home/MyFonts")]);
}

#[test]
fn macos_and_windows_folders() {
    let root = temp("mac");
    for asset in ["com_apple_MobileAsset_Font7/abc.asset", "com_apple_MobileAsset_Other/x"] {
        std::fs::create_dir_all(root.join("System/Library/AssetsV2").join(asset)).unwrap();
    }
    let env = FontEnv { os: Os::MacOs, root: root.clone(), home: Some("/Users/me".into()), ..Default::default() };
    let dirs = font_dirs(&env);
    assert!(dirs.contains(&root.join("System/Library/Fonts")));
    assert!(dirs.contains(&PathBuf::from("/Users/me/Library/Fonts")));
    assert!(dirs.contains(&root.join("System/Library/AssetsV2/com_apple_MobileAsset_Font7")));
    assert!(!dirs.iter().any(|d| d.ends_with("com_apple_MobileAsset_Other")));
    assert_eq!(font_store_dir(&env), Some(PathBuf::from("/Users/me/Library/Application Support/Craft Fonts")));

    let env = FontEnv {
        os: Os::Windows,
        windir: Some("C:\\Windows".into()),
        local_app_data: Some("C:\\Users\\me\\AppData\\Local".into()),
        ..Default::default()
    };
    assert_eq!(
        font_dirs(&env),
        vec![PathBuf::from("C:\\Windows").join("Fonts"), PathBuf::from("C:\\Users\\me\\AppData\\Local").join("Microsoft\\Windows\\Fonts")]
    );
    assert_eq!(font_store_dir(&env), Some(PathBuf::from("C:\\Users\\me\\AppData\\Local").join("Craft Fonts")));
    assert_eq!(office_font_dirs(&env).len(), 1);
}

// Uses Unix paths, which Windows doesn't treat as absolute (#619).
#[cfg(unix)]
#[test]
fn font_store_folder() {
    let env = FontEnv { os: Os::Unix, home: Some("/home/me".into()), ..Default::default() };
    assert_eq!(font_store_dir(&env), Some(PathBuf::from("/home/me/.local/share/craft-fonts")));
    let env = FontEnv { xdg_data_home: Some("/data".into()), ..env };
    assert_eq!(font_store_dir(&env), Some(PathBuf::from("/data/craft-fonts")));
}

/// The store override wins with or without platform defaults; unsupported platforms have no default store.
#[test]
fn font_store_override_and_no_store() {
    for os in [Os::Unix, Os::MacOs, Os::Windows, Os::Haiku, Os::None] {
        let env = FontEnv { os, font_store: Some("portable/fonts".into()), ..Default::default() };
        assert_eq!(font_store_dir(&env), Some(PathBuf::from("portable/fonts")));
        let env = FontEnv { home: Some("home".into()), local_app_data: Some("local".into()), ..env };
        assert_eq!(font_store_dir(&env), Some(PathBuf::from("portable/fonts")));
    }
    assert_eq!(font_store_dir(&FontEnv { os: Os::None, home: Some("h".into()), ..Default::default() }), None);
}

/// A Linux system with fonts in a system folder, the Flatpak host folder, the user's folder and
/// the font store, plus a collection, a lister's file without an extension, and files that aren't
/// fonts.
fn system(root: &Path) -> Arc<Catalog> {
    let sys = root.join("usr/share/fonts/test");
    write(&sys.join("TestSans-Regular.ttf"), font(&spec("Test Sans", "Regular", 400)));
    write(&sys.join("TestSans-Bold.ttf"), font(&spec("Test Sans", "Bold", 700)));
    write(&sys.join("TestSans-Italic.TTF"), font(&Spec { italic: true, ..spec("Test Sans", "Italic", 400) }));
    write(&sys.join("TestSans-Light.ttf"), font(&Spec { legacy: Some(("Test Sans Light", "Regular")), ..spec("Test Sans", "Light", 300) }));
    write(&sys.join("TestSans-Condensed.ttf"), font(&Spec { width: 3, ..spec("Test Sans", "Condensed", 400) }));
    write(&sys.join("Bitmap.ttf"), font(&Spec { outlines: false, ..spec("Bitmap Only", "Regular", 400) }));
    write(&sys.join("damaged.otf"), b"OTTO\0\x10garbage");
    write(&sys.join("notes.txt"), font(&spec("Not Scanned", "Regular", 400)));
    write(
        &root.join("run/host/fonts/HostSerif.ttc"),
        collection(&[font(&spec("Host Serif", "Regular", 400)), font(&spec("Host Serif", "Bold", 700))]),
    );
    write(
        &root.join("home/.local/share/fonts/Gothic.otf"),
        font(&Spec { legacy: Some(("Test Gothic W3", "Regular")), ja: Some("テストゴシック"), ..spec("Test Gothic", "W3", 300) }),
    );
    write(
        &root.join("home/.local/share/fonts/Variable.ttf"),
        font(&Spec { instances: vec![("Regular", 400.0), ("Thin", 100.0), ("Black", 900.0)], ..spec("Test Variable", "Regular", 400) }),
    );
    write(&root.join("home/.local/share/craft-fonts/Store/StoreFace.ttf"), font(&spec("Store Face", "Regular", 400)));
    let service = root.join("service/.hidden/f1");
    write(&service, font(&Spec { fs_type: 2, ..spec("Service Font", "Regular", 400) }));
    Arc::new(Catalog::new(linux_env(root)).with_font_service(false).with_lister(Arc::new(move || vec![service.clone()])))
}

#[test]
fn a_scan_finds_folders_store_lister_files_and_collections() {
    let root = temp("scan");
    let cat = system(&root);
    cat.scan_in_background();
    let families: Vec<String> = cat.families().to_vec();
    assert_eq!(families, ["Host Serif", "Service Font", "Store Face", "Test Gothic", "Test Sans", "Test Variable"]);
    assert!(cat.is_ready());
    let host = cat.styles("host serif");
    assert_eq!(host.iter().map(|f| (f.style.as_str(), f.index)).collect::<Vec<_>>(), [("Regular", 0), ("Bold", 1)]);
    assert_eq!(cat.styles("Store Face")[0].source, Source::Store);
    assert_eq!(host[0].source, Source::System);
    let service = &cat.styles("Service Font")[0];
    assert_eq!((service.source, service.fs_type), (Source::System, 2));
    assert_eq!(cat.styles("Test Sans").len(), 5);
    let light = cat.resolve("Test Sans", "Light").unwrap().0;
    assert_eq!((light.legacy_family.as_deref(), light.legacy_style.as_deref()), (Some("Test Sans Light"), Some("Regular")));
    assert_eq!(light.code_pages, [1, 0]);
}

#[test]
fn lookup_by_every_name_kind() {
    let root = temp("lookup");
    let cat = system(&root);
    let found = |name: &str, style: &str| cat.resolve(name, style).map(|(f, m)| (f.family, f.style, m));
    let exact = |family: &str, style: &str| Some((family.to_string(), style.to_string(), Match::Exact));
    let nearest = |family: &str, style: &str| Some((family.to_string(), style.to_string(), Match::NearestStyle));
    assert_eq!(found("TEST SANS", "bold"), exact("Test Sans", "Bold"));
    assert_eq!(found("Test Sans", ""), exact("Test Sans", "Regular"));
    // Nearest weight, italic differing costing more than any weight.
    assert_eq!(found("Test Sans", "Semibold"), nearest("Test Sans", "Bold"));
    assert_eq!(found("Test Sans", "Bold Italic"), nearest("Test Sans", "Italic"));
    assert_eq!(found("Test Sans", "Thin"), nearest("Test Sans", "Light"));
    // Equal weights: the nearest width.
    assert_eq!(found("Test Sans", "Book Condensed"), nearest("Test Sans", "Condensed"));
    // A legacy family with its legacy style, a localized family, full and PostScript names.
    assert_eq!(found("Test Sans Light", "Regular"), exact("Test Sans", "Light"));
    assert_eq!(found("Test Gothic W3", "Regular"), exact("Test Gothic", "W3"));
    assert_eq!(found("テストゴシック", "W3"), exact("Test Gothic", "W3"));
    assert_eq!(cat.family("testgothic"), Some("Test Gothic".into()));
    assert_eq!(found("Test Sans Bold", ""), exact("Test Sans", "Bold"));
    assert_eq!(found("TestSans-Bold", "Regular"), exact("Test Sans", "Bold"));
    assert_eq!(cat.by_postscript_name("TestSans-Italic").map(|f| f.style), Some("Italic".into()));
    // Named instances: by style, by their PostScript names, and the default instance names the face.
    let thin = cat.resolve("Test Variable", "Thin").unwrap().0;
    assert_eq!((thin.weight, thin.variations.clone()), (100.0, vec![(*b"wght", 100.0)]));
    assert_eq!(cat.by_postscript_name("TestVariable-Black").map(|f| f.weight), Some(900.0));
    assert_eq!(cat.styles("Test Variable").len(), 3);
    assert_eq!(found("Missing Family", "Regular"), None);
    assert_eq!(cat.by_postscript_name("Missing-Regular"), None);
}

#[test]
fn installed_and_removed_fonts_are_seen_without_a_restart() {
    let root = temp("rescan");
    let cat = system(&root);
    assert!(!cat.installed_fonts_changed());
    assert!(cat.family("Test Sans").is_some());
    let generation = cat.generation();
    assert!(!cat.installed_fonts_changed());
    let (tx, rx) = mpsc::channel();
    let _watch = cat.watch(Duration::from_millis(20), move |c| {
        let _ = tx.send((c.family("New Face"), c.family("Later")));
    });
    write(&root.join("usr/share/fonts/test/new/NewFace.otf"), font(&spec("New Face", "Regular", 400)));
    assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), (Some("New Face".into()), None));
    assert!(cat.generation() > generation);
    // A font store install, then a removal.
    write(&root.join("home/.local/share/craft-fonts/Later.ttf"), font(&spec("Later", "Regular", 400)));
    assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), (Some("New Face".into()), Some("Later".into())));
    assert_eq!(cat.styles("Later")[0].source, Source::Store);
    std::fs::remove_file(root.join("usr/share/fonts/test/new/NewFace.otf")).unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), (None, Some("Later".into())));
}

#[test]
fn app_folders_are_scanned_after_a_change() {
    let root = temp("app");
    let cat = Catalog::new(linux_env(&root)).with_font_service(false);
    assert!(cat.families().is_empty());
    write(&root.join("app-fonts/AppFace.ttf"), font(&spec("App Face", "Regular", 400)));
    cat.set_app_dirs(vec![root.join("app-fonts")]);
    assert!(cat.rescan_if_changed());
    assert_eq!(cat.styles("App Face")[0].source, Source::App);
    assert!(!cat.rescan_if_changed());
}

#[test]
fn a_family_loads_once_for_every_thread() {
    let root = temp("load");
    let cat = system(&root);
    let loads: Vec<Arc<[LoadedFace]>> = std::thread::scope(|s| {
        let threads: Vec<_> = (0..8).map(|_| s.spawn(|| cat.load_family("host serif"))).collect();
        threads.into_iter().map(|t| t.join().unwrap()).collect()
    });
    assert!(loads.iter().all(|l| Arc::ptr_eq(l, &loads[0])));
    let host = &loads[0];
    assert_eq!(host.len(), 2);
    // Both faces of the collection share its bytes.
    assert!(Arc::ptr_eq(&host[0].data.bytes, &host[1].data.bytes));
    assert_eq!(host[1].data.index, 1);
    assert!(skrifa::FontRef::from_index(&host[1].data.bytes, 1).is_ok());
    // Another family loads separately, and a rescan makes the next load read again.
    assert_eq!(cat.load_family("Test Sans").len(), 5);
    cat.rescan();
    assert!(!Arc::ptr_eq(&cat.load_family("Host Serif"), &loads[0]));
    assert!(cat.load_family("Missing").is_empty());
}

#[test]
fn added_fonts_come_first_and_respect_the_limits() {
    let root = temp("add");
    let cat = system(&root);
    let limits = Limits::default();
    let doc = font(&spec("Test Sans", "Bold", 700));
    let added = cat.add_bytes(doc.clone(), Source::Document, &limits).unwrap();
    assert_eq!(added.len(), 1);
    let (bold, m) = cat.resolve("Test Sans", "Bold").unwrap();
    assert_eq!((bold.source, bold.path.clone(), m), (Source::Document, None, Match::Exact));
    assert_eq!(&*cat.load(&bold).unwrap().bytes, doc.as_slice());
    assert!(matches!(cat.add_bytes(b"not a font".to_vec(), Source::User, &limits), Err(FontFileError::NotAFont)));
    let small = Limits { max_file_bytes: 10, ..limits };
    assert!(matches!(cat.add_bytes(doc.clone(), Source::User, &small), Err(FontFileError::TooLarge { .. })));
    let few = Limits { max_added_faces: 1, ..limits };
    assert!(matches!(cat.add_bytes(font(&spec("User Face", "Regular", 400)), Source::User, &few), Err(FontFileError::TooManyFaces { max: 1 })));
    let file = root.join("elsewhere/UserFace.ttf");
    write(&file, font(&spec("User Face", "Regular", 400)));
    let user = cat.add_file(&file, Source::User, &limits).unwrap();
    assert_eq!(user[0].path.as_deref(), Some(file.as_path()));
    // A rescan keeps added faces; removing a source's faces brings back the installed face.
    cat.rescan();
    assert_eq!(cat.styles("User Face").len(), 1);
    assert_eq!(cat.remove_added(Source::Document), 1);
    assert_eq!(cat.resolve("Test Sans", "Bold").unwrap().0.source, Source::System);
    assert!(matches!(cat.load(&bold), Err(FontFileError::Gone)));
}

#[test]
fn the_scan_reads_only_what_it_may() {
    let root = temp("bounded");
    let path = root.join("Big.ttf");
    let mut data = font(&spec("Big Face", "Regular", 400));
    data.resize(data.len() + (1 << 20), 0);
    write(&path, &data);
    assert_eq!(crate::scan::file_faces(&path, 1 << 16, Source::System).len(), 1);
    assert!(crate::scan::file_faces(&path, 64, Source::System).is_empty());
    // A collection claiming more faces than it has gives the faces it has.
    let mut ttc = collection(&[font(&spec("A Face", "Regular", 400))]);
    ttc[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
    write(&root.join("Claims.ttc"), &ttc);
    assert_eq!(crate::scan::file_faces(&root.join("Claims.ttc"), u64::MAX, Source::System).len(), 1);
}

#[test]
fn suitcase_resources_are_read() {
    let fonts = vec![font(&spec("Suit A", "Regular", 400)), font(&spec("Suit B", "Bold", 700))];
    let fork = fork_of(&fonts);
    let found = crate::suitcase::sfnt_resources(&fork);
    assert_eq!(found, fonts.iter().map(Vec::as_slice).collect::<Vec<_>>());
    assert!(crate::suitcase::sfnt_resources(&fork[..300]).is_empty());
}

fn fork_of(fonts: &[Vec<u8>]) -> Vec<u8> {
    let mut data = Vec::new();
    let mut offsets = Vec::new();
    for body in fonts.iter().map(Vec::as_slice).chain([&b"1.0"[..]]) {
        offsets.push(data.len() as u32);
        data.extend_from_slice(&(body.len() as u32).to_be_bytes());
        data.extend_from_slice(body);
    }
    let n = fonts.len();
    // The type list: `vers` (1 resource) then `sfnt`, each entry pointing at its references.
    let list = 2 + 2 * 8;
    let mut types = Vec::new();
    types.extend_from_slice(&1u16.to_be_bytes());
    types.extend_from_slice(b"vers");
    types.extend_from_slice(&0u16.to_be_bytes());
    types.extend_from_slice(&(list as u16).to_be_bytes());
    types.extend_from_slice(b"sfnt");
    types.extend_from_slice(&((n - 1) as u16).to_be_bytes());
    types.extend_from_slice(&((list + 12) as u16).to_be_bytes());
    let reference = |id: i16, offset: u32| {
        let mut r = Vec::new();
        r.extend_from_slice(&id.to_be_bytes());
        r.extend_from_slice(&0xFFFFu16.to_be_bytes());
        r.extend_from_slice(&offset.to_be_bytes());
        r.extend_from_slice(&[0; 4]);
        r
    };
    types.extend(reference(1, offsets[n]));
    for (i, o) in offsets[..n].iter().enumerate() {
        types.extend(reference(5000 + i as i16, *o));
    }
    // The map: a copy of the header, a handle, attributes, then the offsets of the lists.
    let mut map = vec![0; 24];
    map.extend_from_slice(&28u16.to_be_bytes());
    map.extend_from_slice(&((28 + types.len()) as u16).to_be_bytes());
    map.extend(types);
    let mut fork = Vec::new();
    for v in [256u32, 256 + data.len() as u32, data.len() as u32, map.len() as u32] {
        fork.extend_from_slice(&v.to_be_bytes());
    }
    fork.resize(256, 0);
    fork.extend(data);
    fork.extend(map);
    fork
}

#[test]
fn repeated_name_records_are_bounded_before_pairing() {
    // Thousands of legacy family and style records in one language: pairing them all would
    // allocate millions of strings. write-fonts refuses unsorted or repeated records, so the
    // `name` table is written by hand.
    let mut names: Vec<(u16, String)> = Vec::new();
    for i in 0..2000 {
        names.push((1, format!("Family {}", i % 1000)));
        names.push((2, format!("Style {}", i % 1000)));
    }
    names.push((16, "Typo Family".into()));
    names.push((17, "Regular".into()));
    let mut records = Vec::new();
    let mut strings = Vec::new();
    let mut stored: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (id, text) in &names {
        let utf16: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        let offset = *stored.entry(text.as_str()).or_insert_with(|| {
            strings.extend_from_slice(&utf16);
            strings.len() - utf16.len()
        });
        for v in [3u16, 1, 0x409, *id, utf16.len() as u16, offset as u16] {
            records.extend_from_slice(&v.to_be_bytes());
        }
    }
    let mut name = Vec::new();
    for v in [0u16, names.len() as u16, (6 + records.len()) as u16] {
        name.extend_from_slice(&v.to_be_bytes());
    }
    name.extend(records);
    name.extend(strings);
    let mut builder = FontBuilder::new();
    builder.add_raw(Tag::new(b"name"), name);
    builder.add_raw(Tag::new(b"glyf"), vec![0u8; 4]);
    let faces = crate::scan::data_faces(&builder.build(), 1, None, Source::User);
    let face = faces.first().expect("a face");
    assert_eq!(face.info.family, "Typo Family");
    assert!(!face.keys.legacy.is_empty());
    assert_eq!(face.keys.legacy.len(), 256, "the pair budget counts distinct pairs");
}

/// A read stops at its cap, whatever the file has grown to since its size was checked.
#[test]
fn a_read_is_capped_where_the_file_has_grown() {
    use crate::catalog::read_capped;
    use std::io::Write as _;
    let path = std::env::temp_dir().join(format!("craft-font-catalog-cap-{}", std::process::id()));
    std::fs::write(&path, b"0123456789").unwrap();
    let observed_size = std::fs::metadata(&path).unwrap().len();
    assert_eq!(read_capped(&path, observed_size).unwrap(), b"0123456789");
    assert!(matches!(read_capped(&path, 9), Err(FontFileError::TooLarge { size: 10, max: 9 })));
    // Model growth after the earlier size observation, without a scheduling-dependent race.
    std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"growth past the cap").unwrap();
    assert!(std::fs::metadata(&path).unwrap().len() > observed_size + 1);
    assert!(matches!(read_capped(&path, observed_size), Err(FontFileError::TooLarge { size: 11, max: 10 })));
    assert!(matches!(read_capped(&path.with_extension("missing"), 9), Err(FontFileError::Io(_))));
    let _ = std::fs::remove_file(&path);
}

fn bounded_catalog() -> Catalog {
    Catalog::new(FontEnv { os: Os::None, ..Default::default() }).with_font_service(false)
}

#[test]
fn bounded_memory_load_enforces_policy_without_changing_default() {
    let cat = bounded_catalog();
    let data = font(&spec("Bounded Memory", "Regular", 400));
    let size = data.len() as u64;
    let face = cat.add_bytes(data, Source::Document, &Limits::default()).unwrap().remove(0);
    let original = cat.load(&face).unwrap();
    let allowed = cat.load_with_limit(&face, size).unwrap();
    assert!(Arc::ptr_eq(&original.bytes, &allowed.bytes));
    assert_eq!(allowed.index, original.index);
    assert!(matches!(cat.load_with_limit(&face, size - 1), Err(FontFileError::TooLarge { size: actual, max }) if actual == size && max == size - 1));
    assert!(matches!(cat.load_with_limit(&face, 0), Err(FontFileError::TooLarge { size: actual, max: 0 }) if actual == size));
    let unchanged = cat.load(&face).unwrap();
    assert!(Arc::ptr_eq(&original.bytes, &unchanged.bytes));
    assert_eq!(cat.remove_added(Source::Document), 1);
    assert!(matches!(cat.load_with_limit(&face, size), Err(FontFileError::Gone)));
}

#[test]
fn bounded_disk_load_enforces_policy_before_read_and_from_cache() {
    let root = temp("bounded-load-disk");
    let path = root.join("Bounded.ttf");
    let data = font(&spec("Bounded Disk", "Regular", 400));
    let size = data.len() as u64;
    write(&path, &data);
    let cat = bounded_catalog();
    cat.set_app_dirs(vec![root]);
    let face = cat.styles("Bounded Disk").remove(0);
    assert!(matches!(cat.load_with_limit(&face, size - 1), Err(FontFileError::TooLarge { size: actual, max }) if actual == size && max == size - 1));
    assert!(matches!(cat.load_with_limit(&face, 0), Err(FontFileError::TooLarge { size: actual, max: 0 }) if actual == size));
    // Prime the ordinary loader's shared cache, then make a fresh read impossible.
    let original = cat.load(&face).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(matches!(cat.load_with_limit(&face, size - 1), Err(FontFileError::TooLarge { size: actual, max }) if actual == size && max == size - 1));
    let allowed = cat.load_with_limit(&face, size).unwrap();
    assert_eq!(&*allowed.bytes, data.as_slice());
    assert_eq!(allowed.index, face.index);
    assert!(Arc::ptr_eq(&original.bytes, &allowed.bytes));
    let unchanged = cat.load(&face).unwrap();
    assert!(Arc::ptr_eq(&original.bytes, &unchanged.bytes));
}

#[test]
fn bounded_collection_load_preserves_nonzero_index_and_shared_bytes() {
    let root = temp("bounded-load-collection");
    let data = collection(&[font(&spec("Bounded Collection", "Regular", 400)), font(&spec("Bounded Collection", "Bold", 700))]);
    let size = data.len() as u64;
    write(&root.join("Bounded.ttc"), &data);
    let cat = bounded_catalog();
    cat.set_app_dirs(vec![root]);
    let faces = cat.styles("Bounded Collection");
    let second = faces.iter().find(|face| face.index == 1).unwrap();
    let loaded_second = cat.load_with_limit(second, size).unwrap();
    assert_eq!(loaded_second.index, 1);
    assert_eq!(&*loaded_second.bytes, data.as_slice());
    assert!(skrifa::FontRef::from_index(&loaded_second.bytes, loaded_second.index).is_ok());
    let first = faces.iter().find(|face| face.index == 0).unwrap();
    let loaded_first = cat.load_with_limit(first, size).unwrap();
    assert_eq!(loaded_first.index, 0);
    assert!(Arc::ptr_eq(&loaded_first.bytes, &loaded_second.bytes));
    assert!(Arc::ptr_eq(&cat.load(second).unwrap().bytes, &loaded_second.bytes));
}

#[test]
fn bounded_suitcase_load_caps_raw_fork_and_selects_only_requested_resource() {
    use crate::suitcase::read_font_fork_capped;
    let root = temp("bounded-load-fork");
    let path = root.join("fork");
    let fonts = vec![font(&spec("Bounded Fork A", "Regular", 400)), font(&spec("Bounded Fork B", "Bold", 700))];
    let mut fork = fork_of(&fonts);
    let size = fork.len() as u64;
    write(&path, &fork);
    let selected = read_font_fork_capped(&path, 1, size).unwrap();
    assert_eq!(selected, fonts[1]);
    assert!(skrifa::FontRef::from_index(&selected, 0).is_ok());
    assert!(
        matches!(read_font_fork_capped(&path, 0, size - 1), Err(FontFileError::TooLarge { size: actual, max }) if actual == size && max == size - 1)
    );
    assert!(matches!(read_font_fork_capped(&path, 0, 0), Err(FontFileError::TooLarge { size: 1, max: 0 })));
    assert!(matches!(read_font_fork_capped(&path, 2, size), Err(FontFileError::NotAFont)));
    fork.extend_from_slice(b"growth");
    write(&path, fork);
    assert!(matches!(read_font_fork_capped(&path, 1, size), Err(FontFileError::TooLarge { size: actual, max }) if actual == size + 1 && max == size));
}

#[test]
fn bounded_suitcase_load_selects_from_aliased_resources() {
    use crate::suitcase::{read_font_fork_capped, sfnt_resources};
    let data = font(&spec("Bounded Alias", "Regular", 400));
    let mut fork = fork_of(std::slice::from_ref(&data));
    let map = u32::from_be_bytes(fork[4..8].try_into().unwrap()) as usize;
    let types = map + u16::from_be_bytes(fork[map + 24..map + 26].try_into().unwrap()) as usize;
    // fork_of puts vers first, then sfnt, whose sole reference is at the end of the map.
    let sfnt = types + 2 + 8;
    let refs = types + u16::from_be_bytes(fork[sfnt + 6..sfnt + 8].try_into().unwrap()) as usize;
    let reference = fork[refs..refs + 12].to_vec();
    let copies = 8u16;
    fork[sfnt + 4..sfnt + 6].copy_from_slice(&(copies - 1).to_be_bytes());
    for i in 1..copies {
        let mut repeated = reference.clone();
        repeated[..2].copy_from_slice(&(5000 + i as i16).to_be_bytes());
        fork.extend(repeated);
    }
    let map_len = fork.len() - map;
    fork[12..16].copy_from_slice(&(map_len as u32).to_be_bytes());
    fork[map + 26..map + 28].copy_from_slice(&(map_len as u16).to_be_bytes());
    let resources = sfnt_resources(&fork);
    assert_eq!(resources.len(), copies as usize);
    assert_eq!(resources[0].as_ptr(), resources[copies as usize - 1].as_ptr());
    assert!(resources.iter().map(|font| font.len()).sum::<usize>() > fork.len());
    let root = temp("bounded-load-aliased-fork");
    let path = root.join("fork");
    let size = fork.len() as u64;
    write(&path, fork);
    assert_eq!(read_font_fork_capped(&path, u32::from(copies - 1), size).unwrap(), data);
}

#[test]
fn physical_view_keeps_duplicate_legacy_owner_without_changing_canonical_lookup() {
    let root = temp("physical-legacy-owner");
    let first_path = root.join("a-first.ttf");
    let second_path = root.join("z-second.ttf");
    let mut first_spec = spec("Duplicate Family", "Light", 300);
    first_spec.legacy = Some(("Legacy First", "Light"));
    let mut second_spec = first_spec.clone();
    second_spec.legacy = Some(("Legacy Second", "Light"));
    let first = font(&first_spec);
    let second = font(&second_spec);
    assert_ne!(first, second);
    write(&first_path, &first);
    write(&second_path, &second);
    // The second font's names really are readable when it is the only contributor.
    let control = bounded_catalog();
    control.add_bytes(second.clone(), Source::Document, &Limits::default()).unwrap();
    assert_eq!(control.resolve("Legacy Second", "Light").unwrap().0.family, "Duplicate Family");

    let cat = bounded_catalog();
    cat.set_app_dirs(vec![root.clone()]);
    let canonical = cat.faces();
    assert_eq!(canonical.len(), 1);
    assert_eq!(canonical[0].path.as_ref(), Some(&first_path));
    assert_eq!(cat.families().as_ref(), &["Duplicate Family".to_owned()]);
    assert_eq!(cat.styles("Duplicate Family"), canonical.to_vec());
    assert_eq!(cat.resolve("Duplicate Family", "Light").unwrap(), (canonical[0].clone(), Match::Exact));
    assert_eq!(cat.by_postscript_name("DuplicateFamily-Light"), Some(canonical[0].clone()));
    assert_eq!(cat.resolve("Legacy Second", "Light"), None, "canonical aliases keep the old first winner");

    let physical = cat.faces_including_duplicates();
    assert_eq!(physical.len(), 2);
    assert_eq!(physical[0], canonical[0]);
    assert_eq!(physical[1].path.as_ref(), Some(&second_path));
    assert_eq!(physical[1].index, 0);
    assert_eq!(physical[1].source, Source::App);
    assert_eq!(physical[1].legacy_family.as_deref(), Some("Legacy Second"));
    let selected = cat.load_with_limit(&physical[1], second.len() as u64).unwrap();
    assert_eq!(selected.bytes.as_ref(), second.as_slice(), "an alias on first_path is not its old owner");
    assert_ne!(selected.bytes.as_ref(), first.as_slice());
    assert_eq!(selected.index, 0);
    let again = cat.load_with_limit(&physical[1], second.len() as u64).unwrap();
    assert!(Arc::ptr_eq(&selected.bytes, &again.bytes));
    assert!(Arc::ptr_eq(&physical, &cat.faces_including_duplicates()));
    assert!(Arc::ptr_eq(&canonical, &cat.faces()));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn physical_view_keeps_duplicate_collection_index_and_shared_bytes() {
    let root = temp("physical-collection-owner");
    let path = root.join("duplicates.ttc");
    let mut first_spec = spec("Duplicate Collection", "Light", 300);
    first_spec.legacy = Some(("Collection First", "Light"));
    let mut second_spec = first_spec.clone();
    second_spec.legacy = Some(("Collection Second", "Light"));
    let bytes = collection(&[font(&first_spec), font(&second_spec)]);
    write(&path, &bytes);
    let cat = bounded_catalog();
    cat.set_app_dirs(vec![root.clone()]);
    assert_eq!(cat.faces().len(), 1);
    assert_eq!(cat.faces()[0].index, 0);
    let physical = cat.faces_including_duplicates();
    assert_eq!(physical.len(), 2);
    assert_eq!(physical[0].index, 0);
    assert_eq!(physical[1].index, 1);
    assert_eq!(physical[1].path.as_ref(), Some(&path));
    assert_eq!(physical[1].legacy_family.as_deref(), Some("Collection Second"));
    let first = cat.load_with_limit(&physical[0], bytes.len() as u64).unwrap();
    let second = cat.load_with_limit(&physical[1], bytes.len() as u64).unwrap();
    assert_eq!(second.index, 1);
    assert_eq!(second.bytes.as_ref(), bytes.as_slice());
    assert!(Arc::ptr_eq(&first.bytes, &second.bytes));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn physical_view_retains_metadata_not_removed_document_blob_payloads() {
    let cat = bounded_catalog();
    let mut document = spec("Duplicate Memory", "Light", 300);
    document.legacy = Some(("Document Legacy", "Light"));
    let mut bundled = document.clone();
    bundled.legacy = Some(("Bundled Legacy", "Light"));
    let document_bytes = font(&document);
    let bundled_bytes = font(&bundled);
    cat.add_bytes(document_bytes.clone(), Source::Document, &Limits::default()).unwrap();
    cat.add_bytes(bundled_bytes.clone(), Source::Bundled, &Limits::default()).unwrap();
    let canonical = cat.faces();
    let physical = cat.faces_including_duplicates();
    let generation = cat.generation();
    assert_eq!(canonical.len(), 1);
    assert_eq!(canonical[0].source, Source::Document);
    assert_eq!(physical.len(), 2);
    assert_eq!(physical[0].source, Source::Document);
    assert_eq!(physical[1].source, Source::Bundled);
    let loaded = cat.load_with_limit(&physical[0], document_bytes.len() as u64).unwrap();
    let weak = Arc::downgrade(&loaded.bytes);
    drop(loaded);
    assert!(weak.upgrade().is_some(), "the live catalog owns the document bytes");
    assert_eq!(cat.remove_added(Source::Document), 1);
    assert!(cat.generation() > generation);
    assert!(weak.upgrade().is_none(), "held canonical and physical metadata must not retain payloads");
    assert_eq!(canonical[0].source, Source::Document);
    assert_eq!(physical.len(), 2, "the retained view is one older metadata generation");
    assert!(matches!(cat.load_with_limit(&physical[0], u64::MAX), Err(FontFileError::Gone)));
    let current = cat.faces_including_duplicates();
    assert!(!Arc::ptr_eq(&physical, &current));
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].source, Source::Bundled);
    assert_eq!(cat.faces().as_ref(), current.as_ref());
    assert!(Arc::ptr_eq(&cat.faces(), &current), "without duplicates the metadata allocation is shared");
    assert_eq!(cat.load_with_limit(&current[0], bundled_bytes.len() as u64).unwrap().bytes.as_ref(), bundled_bytes.as_slice());
}

#[test]
fn physical_view_refresh_preserves_scan_policy_instances_and_permissions() {
    let root = temp("physical-refresh-policy");
    let first_path = root.join("a-variable.ttf");
    let second_path = root.join("z-variable.ttf");
    let mut variable = spec("Physical Variable", "Regular", 400);
    variable.instances = vec![("Medium", 500.0), ("Bold", 700.0)];
    variable.fs_type = 2;
    let bytes = font(&variable);
    write(&first_path, &bytes);
    let mut undrawable = spec("Not An Outline", "Regular", 400);
    undrawable.outlines = false;
    write(&root.join("not-drawable.ttf"), font(&undrawable));
    let cat = bounded_catalog();
    cat.set_app_dirs(vec![root.clone()]);
    let canonical = cat.faces();
    let physical = cat.faces_including_duplicates();
    assert!(!physical.is_empty());
    assert_eq!(physical.as_ref(), canonical.as_ref());
    assert!(Arc::ptr_eq(&physical, &canonical));
    assert!(physical.iter().all(|face| face.family == "Physical Variable" && face.source == Source::App && face.fs_type == 2 && face.variable));
    assert!(physical.iter().any(|face| !face.variations.is_empty()), "existing named instances stay metadata, not new discovery");
    let generation = cat.generation();
    write(&second_path, &bytes);
    cat.rescan();
    let refreshed = cat.faces_including_duplicates();
    assert!(cat.generation() > generation);
    assert!(!Arc::ptr_eq(&physical, &refreshed));
    assert_eq!(refreshed.len(), physical.len() * 2);
    assert_eq!(cat.faces().as_ref(), canonical.as_ref());
    assert!(refreshed.iter().all(|face| face.fs_type == 2 && face.source == Source::App));
    assert_eq!(&refreshed[..physical.len()], physical.as_ref());
    assert!(refreshed[physical.len()..].iter().all(|face| face.path.as_ref() == Some(&second_path)));
    cat.set_app_dirs(vec![]);
    cat.rescan();
    assert!(cat.faces_including_duplicates().is_empty(), "removed app roots cannot remain visible");
    assert!(cat.faces().is_empty());
    assert!(!physical.is_empty(), "old metadata views do not silently switch generations");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn shape_plan_cache_reuses_and_evicts_plans() {
    let data = font(&spec("Test", "Regular", 400));
    let face = harfrust::FontRef::from_index(&data, 0).unwrap();
    let shaper_data = harfrust::ShaperData::new(&face);
    let shaper = shaper_data.shaper(&face).build();
    let cache = ShapePlanCache::default();
    let make = |language: &harfrust::Language| cache.get_or_create(&shaper, None, harfrust::Direction::LeftToRight, None, Some(language), &[]);
    let languages: Vec<harfrust::Language> = ('a'..='z').flat_map(|a| ['a', 'b'].map(move |b| format!("{a}{b}").parse().unwrap())).collect();
    let first = make(&languages[0]);
    assert!(Arc::ptr_eq(&first, &make(&languages[0])));
    for language in languages.iter().skip(1) {
        make(language);
    }
    assert_eq!(cache.len(), ShapePlanCache::CAPACITY);
    assert!(!Arc::ptr_eq(&first, &make(&languages[0])));
    assert_eq!(cache.len(), ShapePlanCache::CAPACITY);
}

#[test]
fn bounded_sfnt_reader_preserves_face_names_and_collection_indices() {
    let mut bold = spec("Example", "Bold Italic", 700);
    bold.italic = true;
    let data = font(&bold);
    let faces = crate::sfnt::read_faces_bytes(&data);
    assert_eq!(faces.len(), 1);
    assert_eq!((&*faces[0].family, &*faces[0].style, faces[0].weight, faces[0].italic), ("Example", "Bold Italic", 700, true));
    let mut hidden = spec("Hidden", "Regular", 400);
    hidden.outlines = false;
    let combined = collection(&[font(&hidden), data]);
    let faces = crate::sfnt::read_faces_bytes(&combined);
    assert_eq!(faces.len(), 1);
    assert_eq!((faces[0].index, faces[0].family.as_str()), (1, "Example"));
    assert!(crate::sfnt::read_faces_bytes(b"invalid").is_empty());
}
