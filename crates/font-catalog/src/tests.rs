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
fn the_stores_face_wins_over_its_copy_in_the_os_folder() {
    // A Flatpak's store sits under ~/.var/app, which sorts after the host's ~/.local/share/fonts
    // the store copies its fonts into.
    let root = temp("store-copy");
    let bytes = font(&spec("Copied Sans", "Regular", 400));
    write(&root.join("home/.local/share/fonts/Craft/Copied Sans/CopiedSans-Regular.ttf"), &bytes);
    let store = root.join("home/.var/app/org.example.Fonts/data/craft-fonts");
    write(&store.join("Copied Sans/CopiedSans-Regular.ttf"), &bytes);
    let cat = Catalog::new(FontEnv { font_store: Some(store), ..linux_env(&root) }).with_font_service(false);
    cat.rescan();
    let styles = cat.styles("Copied Sans");
    assert_eq!(styles.len(), 1);
    assert_eq!(styles[0].source, Source::Store);
    assert_eq!(cat.scanned_faces().iter().filter(|f| f.family == "Copied Sans").count(), 2);
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
