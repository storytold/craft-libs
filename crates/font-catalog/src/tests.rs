use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

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

#[test]
fn font_store_folder() {
    let env = FontEnv { os: Os::Unix, home: Some("/home/me".into()), ..Default::default() };
    assert_eq!(font_store_dir(&env), Some(PathBuf::from("/home/me/.local/share/craft-fonts")));
    let env = FontEnv { xdg_data_home: Some("/data".into()), ..env };
    assert_eq!(font_store_dir(&env), Some(PathBuf::from("/data/craft-fonts")));
    let env = FontEnv { font_store: Some("/portable/fonts".into()), ..env };
    assert_eq!(font_store_dir(&env), Some(PathBuf::from("/portable/fonts")));
    assert_eq!(font_store_dir(&FontEnv { os: Os::None, home: Some("/h".into()), ..Default::default() }), None);
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
fn scoped_fonts_are_seen_only_in_their_scope() {
    let root = temp("scope");
    let cat = system(&root);
    let limits = Limits::default();
    let (a, b) = (cat.new_scope(), cat.new_scope());
    cat.rescan();
    let generation = cat.generation();
    // Each scope replaces the installed Test Sans Bold with its own (told apart by fsType); A also
    // has a family of its own.
    let a_bold = cat.add_bytes_in(a, font(&Spec { fs_type: 4, ..spec("Test Sans", "Bold", 700) }), Source::Document, &limits).unwrap();
    let file = root.join("doc-a/DocOnly.ttf");
    write(&file, font(&spec("Doc Only", "Regular", 400)));
    cat.add_file_in(a, &file, Source::Document, &limits).unwrap();
    cat.add_bytes_in(b, font(&Spec { fs_type: 8, ..spec("Test Sans", "Bold", 700) }), Source::Document, &limits).unwrap();
    assert_eq!(cat.generation(), generation, "scoped changes leave other documents alone");
    assert!(cat.generation_in(a) > generation);

    let bold = |scope: Option<Scope>| {
        let (face, m) = match scope {
            Some(s) => cat.resolve_in(s, "Test Sans", "Bold"),
            None => cat.resolve("Test Sans", "Bold"),
        }
        .unwrap();
        assert_eq!(m, Match::Exact);
        (face.source, face.fs_type)
    };
    assert_eq!(bold(Some(a)), (Source::Document, 4));
    assert_eq!(bold(Some(b)), (Source::Document, 8));
    assert_eq!(bold(None), (Source::System, 0));
    // The scope's family keeps its shared styles, and other families resolve as unscoped.
    assert_eq!(cat.resolve_in(a, "Test Sans", "Regular").unwrap(), cat.resolve("Test Sans", "Regular").unwrap());
    assert_eq!(cat.styles_in(a, "test sans").len(), 5);
    assert_eq!(cat.resolve_in(a, "Host Serif", "Bold").unwrap().0.source, Source::System);
    assert!(cat.families_in(a).iter().any(|f| f == "Doc Only"));
    assert!(!cat.families_in(b).iter().any(|f| f == "Doc Only"));
    assert!(!cat.families().iter().any(|f| f == "Doc Only"));
    assert_eq!(cat.family_in(a, "doc only").as_deref(), Some("Doc Only"));
    assert!(cat.family("Doc Only").is_none());
    assert!(cat.by_postscript_name_in(a, "DocOnly-Regular").is_some());
    assert!(cat.by_postscript_name_in(b, "DocOnly-Regular").is_none());
    assert!(cat.by_postscript_name("DocOnly-Regular").is_none());
    assert!(cat.load_family_in(a, "Test Sans").iter().any(|f| f.info.fs_type == 4));
    assert!(cat.load_family("Test Sans").iter().all(|f| f.info.fs_type == 0));

    // The limit on added faces counts each scope on its own: A is full, B isn't.
    let two = Limits { max_added_faces: 2, ..limits };
    assert!(matches!(
        cat.add_bytes_in(a, font(&spec("One More", "Regular", 400)), Source::Document, &two),
        Err(FontFileError::TooManyFaces { max: 2 })
    ));
    cat.add_bytes_in(b, font(&spec("One More", "Regular", 400)), Source::Document, &two).unwrap();

    // A face added without a scope comes after the scope's own, and the scopes' faces don't
    // count against unscoped adds.
    cat.add_bytes(font(&Spec { fs_type: 2, ..spec("Test Sans", "Bold", 700) }), Source::User, &two).unwrap();
    assert_eq!(bold(None), (Source::User, 2));
    assert_eq!(bold(Some(b)), (Source::Document, 8));

    let before = cat.generation();
    assert_eq!(cat.remove_scope(a), 2);
    assert_eq!(cat.generation(), before);
    assert_eq!(bold(Some(a)), (Source::User, 2));
    assert_eq!(bold(Some(b)), (Source::Document, 8));
    assert!(cat.family_in(a, "Doc Only").is_none());
    assert!(matches!(cat.load(&a_bold[0]), Err(FontFileError::Gone)));
    // Removing unscoped faces keeps the scopes'.
    assert_eq!(cat.remove_added(Source::User), 1);
    assert_eq!(bold(Some(b)), (Source::Document, 8));
    assert_eq!(cat.load_family_in(b, "Test Sans").iter().filter(|f| f.info.fs_type == 8).count(), 1);
}

#[test]
fn a_scope_lists_counts_and_removes_its_own_files() {
    let root = temp("scope-own");
    let cat = system(&root);
    let limits = Limits::default();
    let (a, b) = (cat.new_scope(), cat.new_scope());
    assert_eq!(cat.scope_generation(a), 0);
    let (one, two) = (root.join("doc-a/One.ttf"), root.join("doc-a/Two.ttc"));
    write(&one, font(&spec("Doc One", "Regular", 400)));
    write(&two, collection(&[font(&spec("Doc Two", "Regular", 400)), font(&spec("Doc Two", "Bold", 700))]));
    cat.add_file_in(a, &one, Source::Document, &limits).unwrap();
    cat.add_file_in(a, &two, Source::Document, &limits).unwrap();
    // The same face again is listed once.
    cat.add_bytes_in(a, font(&spec("Doc One", "Regular", 400)), Source::Document, &limits).unwrap();
    let generation = cat.scope_generation(a);
    assert_eq!(generation, 3);

    // Another scope's, unscoped and scanned changes neither show in A's faces nor move its count.
    cat.add_bytes_in(b, font(&spec("Other Doc", "Regular", 400)), Source::Document, &limits).unwrap();
    cat.add_bytes(font(&spec("Shared Face", "Regular", 400)), Source::User, &limits).unwrap();
    cat.rescan();
    let names = |s: Scope| cat.faces_in(s).iter().map(|f| format!("{} {}", f.family, f.style)).collect::<Vec<_>>();
    assert_eq!(names(a), ["Doc One Regular", "Doc Two Regular", "Doc Two Bold"]);
    assert_eq!(names(b), ["Other Doc Regular"]);
    assert_eq!(cat.scope_generation(a), generation);
    assert_eq!(cat.scope_generation(b), 1);

    assert_eq!(cat.remove_file_in(a, &two), 2);
    assert_eq!(cat.scope_generation(a), generation + 1);
    assert_eq!(names(a), ["Doc One Regular"]);
    assert!(cat.family_in(a, "Doc Two").is_none());
    assert_eq!(cat.remove_file_in(a, &two), 0);
    assert_eq!(cat.remove_file_in(b, &one), 0);
    assert_eq!(cat.scope_generation(a), generation + 1);
    assert_eq!(cat.scope_generation(b), 1);
    assert_eq!(cat.remove_scope(a), 2);
    assert_eq!(cat.scope_generation(a), generation + 2);
    assert!(cat.faces_in(a).is_empty());
}

#[test]
fn a_scope_resolves_as_one_index_of_every_face_would() {
    let root = temp("scope-names");
    let cat = system(&root);
    let limits = Limits::default();
    let scope = cat.new_scope();
    // A family spelled without the space answers to "Test Sans" only through an alias, so the
    // installed family of that exact name wins there, as it does unscoped.
    let doc = font(&Spec { fs_type: 4, ..spec("TestSans", "Bold", 700) });
    cat.add_bytes_in(scope, doc.clone(), Source::Document, &limits).unwrap();
    assert_eq!(cat.resolve_in(scope, "Test Sans", "Bold"), cat.resolve("Test Sans", "Bold"));
    assert_eq!(cat.styles_in(scope, "Test Sans"), cat.styles("Test Sans"));
    assert_eq!(cat.family_in(scope, "Test Sans").as_deref(), Some("Test Sans"));
    let (own, m) = cat.resolve_in(scope, "TestSans", "Bold").unwrap();
    assert_eq!((own.fs_type, m), (4, Match::Exact));
    assert_eq!(cat.by_postscript_name_in(scope, "TestSans-Bold").unwrap().fs_type, 4);
    assert_eq!(cat.by_postscript_name_in(scope, "TestSans-Regular"), cat.by_postscript_name("TestSans-Regular"));

    // The scope's generation never repeats, removed and added to again included.
    let added = cat.generation_in(scope);
    assert_eq!(cat.remove_scope(scope), 1);
    let removed = cat.generation_in(scope);
    assert_eq!(cat.remove_scope(scope), 0);
    assert_eq!(cat.generation_in(scope), removed);
    assert_eq!(cat.resolve_in(scope, "TestSans", "Bold"), cat.resolve("TestSans", "Bold"));
    cat.add_bytes_in(scope, doc, Source::Document, &limits).unwrap();
    let again = cat.generation_in(scope);
    assert!(added < removed && removed < again, "{added} {removed} {again}");
    assert_eq!(cat.resolve_in(scope, "TestSans", "Bold").unwrap().0.fs_type, 4);
}

#[cfg(unix)]
#[test]
fn the_scan_skips_files_that_are_not_regular() {
    let root = temp("fifo");
    let dir = root.join("usr/share/fonts");
    write(&dir.join("Real.ttf"), font(&spec("Real Face", "Regular", 400)));
    let fifo = dir.join("Blocking.ttf");
    assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
    let named = fifo.clone();
    let cat = Arc::new(Catalog::new(linux_env(&root)).with_font_service(false).with_lister(Arc::new(move || vec![named.clone()])));
    // Opening a FIFO blocks until a writer comes, so the scan runs on a thread with a deadline.
    let (tx, rx) = mpsc::channel();
    let (scanning, opened) = (Arc::clone(&cat), fifo.clone());
    std::thread::spawn(move || {
        // The open itself doesn't wait, whatever an earlier check said.
        let direct = crate::scan::file_faces(&opened, u64::MAX, Source::User);
        tx.send((direct.len(), scanning.families().to_vec()))
    });
    let (direct, families) = rx.recv_timeout(Duration::from_secs(10)).expect("the scan opened the FIFO");
    assert_eq!((direct, families.as_slice()), (0, ["Real Face".to_string()].as_slice()));
    assert!(matches!(cat.add_file(&fifo, Source::User, &Limits::default()), Err(FontFileError::Io(_))));
    assert!(matches!(cat.add_file(&dir, Source::User, &Limits::default()), Err(FontFileError::Io(_))));
}

#[cfg(unix)]
#[test]
fn a_confined_folder_skips_links_that_leave_it() {
    use std::os::unix::fs::symlink;
    let root = temp("confined");
    let picked = root.join("picked");
    write(&picked.join("Inside.ttf"), font(&spec("Inside Face", "Regular", 400)));
    write(&picked.join("sub/Deep.ttf"), font(&spec("Deep Face", "Regular", 400)));
    write(&root.join("outside/Secret.ttf"), font(&spec("Secret Face", "Regular", 400)));
    write(&root.join("outside/dir/Other.ttf"), font(&spec("Other Face", "Regular", 400)));
    symlink(root.join("outside/Secret.ttf"), picked.join("Secret.ttf")).unwrap();
    symlink(root.join("outside/dir"), picked.join("dir")).unwrap();
    symlink(&picked, root.join("picked-link")).unwrap();
    let cat = Catalog::new(linux_env(&root)).with_font_service(false);
    // The folder may itself be a link; what counts is where it leads.
    cat.set_confined_dirs(vec![root.join("picked-link")]);
    assert_eq!(cat.families().to_vec(), ["Deep Face", "Inside Face"]);
    assert_eq!(cat.styles("Inside Face")[0].source, Source::User);
    // A confined file opens by its real path without following a link swapped in for it.
    assert!(crate::scan::open_regular(&picked.join("Inside.ttf"), true).is_ok());
    assert!(crate::scan::open_regular(&picked.join("Secret.ttf"), true).is_err());
    // As an app folder, the same folder follows its links.
    cat.set_confined_dirs(vec![]);
    cat.set_app_dirs(vec![picked]);
    assert!(cat.rescan_if_changed());
    assert_eq!(cat.families().to_vec(), ["Deep Face", "Inside Face", "Other Face", "Secret Face"]);
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
fn changes_since_names_the_families_added_removed_and_changed() {
    let root = temp("changes");
    let dir = root.join("app-fonts");
    write(&dir.join("AppFace-Regular.ttf"), font(&spec("App Face", "Regular", 400)));
    let cat = Catalog::without_installed_fonts();
    cat.set_app_dirs(vec![dir.clone()]);
    let names = |v: &[String]| v.to_vec();
    let start = cat.generation();
    assert_eq!(names(&cat.changes_since(start).added), Vec::<String>::new());
    cat.rescan();
    let scanned = cat.generation();
    assert_eq!(names(&cat.changes_since(start).added), ["App Face"]);
    // Added and removed faces.
    let limits = Limits::default();
    cat.add_bytes(font(&spec("Added Face", "Regular", 400)), Source::User, &limits).unwrap();
    let changes = cat.changes_since(scanned);
    assert_eq!((names(&changes.added), changes.generation, changes.full), (vec!["Added Face".to_string()], cat.generation(), false));
    let added = changes.generation;
    cat.remove_added(Source::User);
    assert_eq!(names(&cat.changes_since(added).removed), ["Added Face"]);
    assert!(cat.changes_since(scanned).is_empty());
    assert!(cat.changes_since(cat.generation()).is_empty());
    // A scope's faces don't count.
    let scope = cat.new_scope();
    cat.add_bytes_in(scope, font(&spec("Scoped Face", "Regular", 400)), Source::Document, &limits).unwrap();
    assert!(cat.changes_since(scanned).is_empty());
    // A rescan: a family installed, a style added to one, then a family removed.
    let before = cat.generation();
    write(&dir.join("Installed-Regular.ttf"), font(&spec("Installed Face", "Regular", 400)));
    write(&dir.join("AppFace-Bold.ttf"), font(&spec("App Face", "Bold", 700)));
    cat.rescan();
    let changes = cat.changes_since(before);
    assert_eq!((names(&changes.added), names(&changes.changed)), (vec!["Installed Face".to_string()], vec!["App Face".to_string()]));
    assert!(!cat.rescan_if_changed());
    cat.rescan();
    assert_eq!(cat.changes_since(changes.generation), Changes { generation: cat.generation(), ..Changes::default() });
    std::fs::remove_file(dir.join("Installed-Regular.ttf")).unwrap();
    cat.rescan();
    let changes = cat.changes_since(before);
    assert_eq!((names(&changes.added), names(&changes.removed), names(&changes.changed)), (vec![], vec![], vec!["App Face".to_string()]));
    assert_eq!(names(&cat.changes_since(changes.generation - 1).removed), ["Installed Face"]);
    // The history keeps the last 64 changes of the family set; older generations get `full`.
    let bytes = font(&spec("Churn", "Regular", 400));
    for _ in 0..40 {
        cat.add_bytes(bytes.clone(), Source::User, &limits).unwrap();
        cat.remove_added(Source::User);
    }
    let changes = cat.changes_since(before);
    assert!(changes.full && changes.added.is_empty() && !changes.is_empty());
    assert!(!cat.changes_since(cat.generation() - 10).full);
}

#[test]
fn the_catalog_without_installed_fonts_sees_none_of_the_hosts() {
    let cat = Catalog::without_installed_fonts();
    assert!(font_dirs(cat.env()).is_empty() && font_store_dir(cat.env()).is_none());
    assert_eq!(cat.rescan(), 0);
    assert!(cat.families().is_empty());
    assert!(!cat.installed_fonts_changed());
    // It still takes bundled faces and the app's folders.
    cat.add_bytes(font(&spec("Bundled Face", "Regular", 400)), Source::Bundled, &Limits::default()).unwrap();
    let root = temp("empty");
    write(&root.join("AppFace.ttf"), font(&spec("App Face", "Regular", 400)));
    cat.set_app_dirs(vec![root]);
    cat.rescan();
    assert_eq!(cat.families().to_vec(), ["App Face", "Bundled Face"]);
}

/// Waits up to 10 seconds for `done`.
fn wait_for(done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn activations_share_one_worker_and_space_out_full_rescans() {
    use crate::app::Refresh;

    let root = temp("activate");
    // A font service that names `listed` and waits while `blocked`, so a full rescan is
    // visible (each calls it) and can be held running.
    let (calls, blocked, listed) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicBool::new(false)), Arc::new(Mutex::new(Vec::new())));
    let (c, b, l) = (Arc::clone(&calls), Arc::clone(&blocked), Arc::clone(&listed));
    let lister: Lister = Arc::new(move || {
        c.fetch_add(1, Ordering::SeqCst);
        while b.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1));
        }
        l.lock().unwrap().clone()
    });
    let cat = Arc::new(Catalog::without_installed_fonts().with_lister(lister));
    let (tx, rx) = mpsc::channel();
    let on_change: app::OnChange = Arc::new(move |c: &Catalog| {
        let _ = tx.send(c.families().to_vec());
    });
    let refresh = Arc::new(Refresh::new(Arc::clone(&cat), Duration::from_secs(60), Some(on_change)));
    let t0 = Instant::now();
    assert!(!refresh.activated_at(t0), "nothing before the first scan");
    cat.rescan();
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // Within the interval: a change check only, which doesn't ask the font service.
    assert!(refresh.activated_at(t0));
    wait_for(|| !refresh.is_running());
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // After it: a full rescan, finding a font only the service knows. Activations while it runs
    // start nothing.
    let service = root.join("service/f1");
    write(&service, font(&spec("Service Face", "Regular", 400)));
    listed.lock().unwrap().push(service);
    blocked.store(true, Ordering::SeqCst);
    assert!(refresh.activated_at(t0 + Duration::from_secs(61)));
    wait_for(|| calls.load(Ordering::SeqCst) == 2);
    assert!(!refresh.activated_at(t0 + Duration::from_secs(200)));
    assert!(!refresh.activated_at(t0 + Duration::from_secs(300)));
    blocked.store(false, Ordering::SeqCst);
    assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap(), ["Service Face"]);
    wait_for(|| !refresh.is_running());
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    // The interval counts from that full rescan.
    assert!(refresh.activated_at(t0 + Duration::from_secs(120)));
    wait_for(|| !refresh.is_running());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(refresh.activated_at(t0 + Duration::from_secs(121)));
    wait_for(|| !refresh.is_running());
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    // Nothing changed, so no call.
    assert!(rx.try_recv().is_err());
}

#[test]
fn a_file_replaced_in_place_loads_its_new_bytes() {
    let root = temp("replace");
    let path = root.join("fonts/Face.ttf");
    write(&path, font(&spec("Face", "Regular", 400)));
    let cat = Catalog::without_installed_fonts();
    cat.set_app_dirs(vec![root.join("fonts")]);
    let old = cat.load(&cat.styles("Face")[0]).unwrap();
    // Held, so the old bytes stay cached.
    let new = font(&Spec { width: 7, ..spec("Face", "Regular Wide", 400) });
    std::fs::write(&path, &new).unwrap();
    cat.rescan();
    let face = cat.styles("Face")[0].clone();
    assert_eq!(face.style, "Regular Wide");
    assert_eq!(&*cat.load(&face).unwrap().bytes, new.as_slice());
    assert_ne!(&*old.bytes, new.as_slice());
}

#[cfg(unix)]
#[test]
fn a_confined_face_never_loads_through_a_link_swapped_in_after_the_scan() {
    let root = temp("swap");
    let picked = root.join("picked");
    let inside = picked.join("Inside.ttf");
    write(&inside, font(&spec("Inside Face", "Regular", 400)));
    write(&root.join("outside/Secret.ttf"), font(&spec("Inside Face", "Regular", 400)));
    let cat = Catalog::without_installed_fonts();
    cat.set_confined_dirs(vec![picked]);
    let face = cat.styles("Inside Face")[0].clone();
    // Loaded and held, so its bytes are cached; then swapped for a link before a rescan.
    let held = cat.load(&face).unwrap();
    std::fs::remove_file(&inside).unwrap();
    std::os::unix::fs::symlink(root.join("outside/Secret.ttf"), &inside).unwrap();
    assert!(cat.load(&face).is_err());
    drop(held);
    assert!(cat.load(&face).is_err());
    // An unconfined face of the same path follows the link.
    let unconfined = FaceInfo { confined: false, ..face };
    assert!(cat.load(&unconfined).is_ok());
}

/// A catalog whose font service blocks while `blocked`, and counts its calls.
fn blocking_catalog(blocked: &Arc<AtomicBool>, calls: &Arc<AtomicUsize>) -> Arc<Catalog> {
    let (b, c) = (Arc::clone(blocked), Arc::clone(calls));
    let lister: Lister = Arc::new(move || {
        c.fetch_add(1, Ordering::SeqCst);
        while b.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1));
        }
        Vec::new()
    });
    Arc::new(Catalog::without_installed_fonts().with_lister(lister))
}

#[test]
fn adding_fonts_never_waits_for_a_scan() {
    let (blocked, calls) = (Arc::new(AtomicBool::new(true)), Arc::new(AtomicUsize::new(0)));
    let cat = blocking_catalog(&blocked, &calls);
    // During the first scan: the app's bundled fonts, one rebuild for all of them.
    cat.scan_in_background();
    wait_for(|| calls.load(Ordering::SeqCst) == 1);
    let bundled: Vec<Arc<[u8]>> =
        vec![font(&spec("Bundled One", "Regular", 400)).into(), b"not a font".as_slice().into(), font(&spec("Bundled Two", "Regular", 400)).into()];
    let errors = crate::app::add_app_fonts(&cat, app::AppFonts { bundled, ..app::AppFonts::default() });
    assert_eq!(errors.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [1]);
    assert!(!cat.is_ready() && blocked.load(Ordering::SeqCst));
    assert_eq!(cat.generation(), 0);
    blocked.store(false, Ordering::SeqCst);
    assert_eq!(cat.families().to_vec(), ["Bundled One", "Bundled Two"]);
    assert_eq!(cat.generation(), 1);
    // During a rescan: the face is seen at once.
    blocked.store(true, Ordering::SeqCst);
    let rescan = std::thread::spawn({
        let cat = Arc::clone(&cat);
        move || cat.rescan()
    });
    wait_for(|| calls.load(Ordering::SeqCst) == 2);
    cat.add_bytes(font(&spec("Later", "Regular", 400)), Source::User, &Limits::default()).unwrap();
    assert_eq!(cat.families().to_vec(), ["Bundled One", "Bundled Two", "Later"]);
    blocked.store(false, Ordering::SeqCst);
    assert_eq!(rescan.join().unwrap(), 3);
    assert_eq!(cat.families().to_vec(), ["Bundled One", "Bundled Two", "Later"]);
}

#[test]
fn rescan_now_counts_as_the_full_rescan() {
    use crate::app::Refresh;

    let (blocked, calls) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicUsize::new(0)));
    let cat = blocking_catalog(&blocked, &calls);
    let root = temp("rescan-now");
    cat.set_app_dirs(vec![root.clone()]);
    let (tx, rx) = mpsc::channel();
    let on_change: app::OnChange = Arc::new(move |c: &Catalog| {
        let _ = tx.send(c.families().to_vec());
    });
    let refresh = Arc::new(Refresh::new(Arc::clone(&cat), Duration::from_secs(60), Some(on_change)));
    let t0 = Instant::now();
    cat.rescan();
    assert!(refresh.activated_at(t0));
    wait_for(|| !refresh.is_running());
    // Reload fonts, after a font is installed: on this thread, with on_change.
    write(&root.join("New.ttf"), font(&spec("New Face", "Regular", 400)));
    let changes = refresh.rescan_now(Some(t0 + Duration::from_secs(59)));
    assert_eq!(changes.added, ["New Face"]);
    assert_eq!(rx.try_recv().unwrap(), ["New Face"]);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    // The interval counts from it: no full rescan at t0 + 60 s.
    assert!(refresh.activated_at(t0 + Duration::from_secs(60)));
    wait_for(|| !refresh.is_running());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(refresh.activated_at(t0 + Duration::from_secs(119)));
    wait_for(|| !refresh.is_running());
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert!(refresh.rescan_now(None).is_empty());
    assert!(rx.try_recv().is_err());
}
