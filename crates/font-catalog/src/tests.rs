use std::collections::HashSet;
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
    /// Japanese legacy family name (name ID 1), beside a typographic family.
    legacy_ja: Option<&'static str>,
    weight: u16,
    width: u16,
    italic: bool,
    fs_type: u16,
    /// Named instances: style and `wght`; the font gets a `wght` axis from 100 to 900, default 400.
    instances: Vec<(&'static str, f64)>,
    outlines: bool,
}

fn spec(family: &'static str, style: &'static str, weight: u16) -> Spec {
    Spec { family, style, legacy: None, ja: None, legacy_ja: None, weight, width: 5, italic: false, fs_type: 0, instances: vec![], outlines: true }
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
    if let Some(ja) = s.legacy_ja {
        add(0x411, 1, ja.into());
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
fn a_rescan_with_no_changes_keeps_the_generation() {
    let root = temp("rescan-same");
    write(&root.join("home/.local/share/fonts/Same-Regular.ttf"), font(&spec("Same", "Regular", 400)));
    let cat = Catalog::new(linux_env(&root)).with_font_service(false);
    cat.rescan();
    let generation = cat.generation();
    assert_eq!(cat.families().as_ref(), ["Same"]);
    cat.rescan();
    assert_eq!(cat.generation(), generation, "nothing changed, so the catalog keeps its generation");
    write(&root.join("home/.local/share/fonts/Other-Regular.ttf"), font(&spec("Other", "Regular", 400)));
    cat.rescan();
    assert!(cat.generation() > generation, "a new font file publishes a new generation");
}

#[test]
fn a_font_store_install_shows_in_the_change_list() {
    let root = temp("store-watch");
    let store = root.join("home/.local/share/craft-fonts");
    // As the font store installs: a staged `.part` file moved into the family's folder.
    let staging = store.join(".staging/1");
    let install = |dest: &str, family: &'static str, style: &'static str| {
        let part = staging.join("face.part");
        write(&part, font(&spec(family, style, 400)));
        let dest = store.join(dest);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::rename(&part, dest).unwrap();
    };
    install("ofl/storeone/StoreOne-Regular.ttf", "Store One", "Regular");
    let cat = Arc::new(Catalog::new(linux_env(&root)).with_font_service(false));
    let start = cat.generation();
    cat.rescan();
    assert_eq!(cat.changes_since(start).added, ["Store One"]);
    assert!(!cat.store_changed());

    let (tx, rx) = mpsc::channel();
    let on_change: app::OnChange = Arc::new(move |_| {
        let _ = tx.send(());
    });
    let refresh = Arc::new(app::Refresh::new(Arc::clone(&cat), Duration::MAX, Some(on_change)));
    refresh.watch_store(Duration::from_millis(20));
    let wait_for = |since: u64, added: &[&str], removed: &[&str], changed: &[&str]| loop {
        rx.recv_timeout(Duration::from_secs(10)).expect("a store change wakes the app");
        let c = cat.changes_since(since);
        if c.added == added && c.removed == removed && c.changed == changed {
            return c.generation;
        }
    };

    let before = cat.generation();
    install("ofl/storetwo/StoreTwo-Regular.ttf", "Store Two", "Regular");
    let two = wait_for(before, &["Store Two"], &[], &[]);
    assert_eq!(cat.styles("Store Two")[0].source, Source::Store);
    install("ofl/storeone/StoreOne-Bold.ttf", "Store One", "Bold");
    std::fs::remove_dir_all(store.join("ofl/storetwo")).unwrap();
    wait_for(two, &[], &["Store Two"], &["Store One"]);
    assert!(cat.changes_since(cat.generation()).is_empty());

    // The history keeps the last 64 changes of the family set; older generations get `full`.
    let bytes = font(&spec("Churn", "Regular", 400));
    for _ in 0..40 {
        cat.add_bytes(bytes.clone(), Source::User, &Limits::default()).unwrap();
        cat.remove_added(Source::User);
    }
    let changes = cat.changes_since(before);
    assert!(changes.full && changes.added.is_empty() && !changes.is_empty());
    assert!(!cat.changes_since(cat.generation() - 10).full);
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
fn generic_families_follow_the_platforms_list_then_the_others() {
    let root = temp("generic");
    let dir = root.join("listed");
    write(&dir.join("Arial.ttf"), font(&spec("Arial", "Regular", 400)));
    write(&dir.join("NotoSans.ttf"), font(&spec("Noto Sans", "Regular", 400)));
    write(&dir.join("Mono.ttf"), font(&spec("DEJAVU SANS MONO", "Regular", 400)));
    write(&dir.join("Times.ttf"), font(&Spec { legacy: Some(("Times New Roman", "Regular")), ..spec("Times New Roman Pro", "Regular", 400) }));
    write(&dir.join("DejaVuSans.ttf"), font(&Spec { outlines: false, ..spec("DejaVu Sans", "Regular", 400) }));
    let catalog = |os: Os| {
        let listed = dir.clone();
        Catalog::new(FontEnv { os, ..linux_env(&root) }).with_font_service(false).with_lister(Arc::new(move || vec![listed.clone()]))
    };
    // Linux: DejaVu Sans has no outlines, so Noto Sans; the family's own spelling comes back.
    let linux = catalog(Os::Unix);
    assert_eq!(linux.generic_family(Generic::SansSerif).as_deref(), Some("Noto Sans"));
    assert_eq!(linux.generic_family(Generic::Monospace).as_deref(), Some("DEJAVU SANS MONO"));
    // A legacy family name finds its family.
    assert_eq!(linux.generic_family(Generic::Serif).as_deref(), Some("Times New Roman Pro"));
    assert_eq!(linux.generic_family(Generic::SystemUi).as_deref(), Some("Noto Sans"));
    // Windows tries Arial before the Linux names.
    let windows = catalog(Os::Windows);
    assert_eq!(windows.generic_family(Generic::SansSerif).as_deref(), Some("Arial"));
    // Nothing of macOS's system-ui list: sans-serif's answer.
    assert_eq!(catalog(Os::MacOs).generic_family(Generic::SystemUi).as_deref(), Some("Arial"));
    // An added face counts; with nothing installed there is no answer.
    let empty = Catalog::new(linux_env(&root.join("empty"))).with_font_service(false);
    assert_eq!(empty.generic_family(Generic::Monospace), None);
    empty.add_bytes(font(&spec("Liberation Mono", "Regular", 400)), Source::Document, &Limits::default()).unwrap();
    assert_eq!(empty.generic_family(Generic::Monospace).as_deref(), Some("Liberation Mono"));
    assert_eq!(empty.generic_family(Generic::Serif), None);
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
    let path = std::env::temp_dir().join(format!("craft-font-catalog-cap-{}", std::process::id()));
    std::fs::write(&path, b"0123456789").unwrap();
    assert_eq!(read_capped(&path, 10).unwrap(), b"0123456789");
    assert!(matches!(read_capped(&path, 9), Err(FontFileError::TooLarge { size: 10, max: 9 })));
    assert!(matches!(read_capped(&path.with_extension("missing"), 9), Err(FontFileError::Io(_))));
    let _ = std::fs::remove_file(&path);
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

/// In a confined folder a link is followed only when it leads inside a confined folder; elsewhere links are followed.
#[cfg(unix)]
#[test]
fn a_confined_folder_follows_only_links_that_stay_inside_it() {
    let root = temp("confined");
    let picked = root.join("picked");
    write(&picked.join("own/Inside.ttf"), font(&spec("Inside Face", "Regular", 400)));
    write(&root.join("elsewhere/Outside.ttf"), font(&spec("Outside Face", "Regular", 400)));
    std::os::unix::fs::symlink(picked.join("own/Inside.ttf"), picked.join("inner.ttf")).unwrap();
    std::os::unix::fs::symlink(root.join("elsewhere/Outside.ttf"), picked.join("outer.ttf")).unwrap();
    std::os::unix::fs::symlink(root.join("elsewhere"), picked.join("outer-folder")).unwrap();
    let families = |confined: bool| {
        let cat = Catalog::new(linux_env(&root)).with_font_service(false);
        cat.set_app_dirs(vec![picked.clone()]);
        if confined {
            cat.set_confined_dirs(vec![picked.clone()]);
        }
        let families = cat.families();
        (families.iter().any(|f| f == "Inside Face"), families.iter().any(|f| f == "Outside Face"))
    };
    assert_eq!(families(false), (true, true), "links are followed out of a folder that isn't confined");
    assert_eq!(families(true), (true, false), "a confined folder follows no link out of it");
    let _ = std::fs::remove_dir_all(&root);
}

/// A language the typographic family records leave out names the family by its legacy record, so an English-only
/// typographic family keeps its Japanese legacy name; a language they cover keeps the typographic name.
#[test]
fn a_language_without_a_typographic_family_name_keeps_its_legacy_name() {
    let localized = |s: &Spec| {
        let faces = crate::scan::data_faces(&font(s), 1, None, Source::System);
        faces[0].info.localized_families.iter().map(|l| l.name.clone()).collect::<Vec<_>>()
    };
    let typographic_english = Spec { legacy: Some(("Test Mincho", "Regular")), ..spec("Test Mincho", "Regular", 400) };
    assert_eq!(localized(&Spec { legacy_ja: Some("テスト明朝"), ..typographic_english.clone() }), ["テスト明朝"]);
    let both = Spec { ja: Some("テスト明朝 Pro"), legacy_ja: Some("テスト明朝"), ..typographic_english.clone() };
    assert_eq!(localized(&both), ["テスト明朝 Pro"], "the typographic record of a language wins over its legacy one");
    assert!(localized(&typographic_english).is_empty());
}

/// A face installed from two files is listed once, and each file's version is found from either, as rescans add and
/// remove the second file.
#[test]
fn every_file_of_a_face_is_one_of_its_versions() {
    let root = temp("versions");
    let cat = system(&root);
    let bold = cat.resolve("Test Sans", "Bold").unwrap().0;
    assert_eq!(cat.versions(&bold), std::slice::from_ref(&bold));
    let second = root.join("home/.local/share/fonts/TestSans-Bold.ttf");
    write(&second, font(&spec("Test Sans", "Bold", 700)));
    cat.rescan();
    assert_eq!(cat.styles("Test Sans").iter().filter(|f| f.style == "Bold").count(), 1, "the lists show a face once");
    let versions = cat.versions(&bold);
    let paths: HashSet<_> = versions.iter().filter_map(|f| f.path.clone()).collect();
    assert_eq!(paths, HashSet::from([bold.path.clone().unwrap(), second.clone()]));
    let other = versions.iter().find(|f| f.path.as_ref() == Some(&second)).unwrap();
    assert_eq!(cat.versions(other), versions);
    std::fs::remove_file(&second).unwrap();
    cat.rescan();
    assert_eq!(cat.versions(&bold).len(), 1);
    assert!(cat.versions(other).is_empty());
}

#[test]
fn a_background_scan_that_panics_lets_the_next_call_scan_again() {
    let root = temp("panic");
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let lister: Lister = Arc::new(move || {
        assert!(counted.fetch_add(1, Ordering::Relaxed) > 0, "the first scan fails");
        Vec::new()
    });
    let cat = Arc::new(Catalog::new(linux_env(&root)).with_font_service(false).with_lister(lister));
    // Each call starts a scan only when none is running; the second starts once the failed one has ended.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !cat.is_ready() && std::time::Instant::now() < deadline {
        cat.scan_in_background();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(cat.is_ready(), "a failed scan must not leave the catalog waiting on it");
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}

/// An app adds its own fonts when it makes its font database: that must not wait for a scan of every installed font,
/// neither one it starts nor one running in the background. The installed faces join the added ones when a lookup
/// scans, and a rescan counts each listed face once.
#[test]
fn adding_fonts_does_not_wait_for_the_installed_fonts() {
    let root = temp("added");
    drop(system(&root));
    let (entered, scan_entered) = mpsc::channel();
    let (release, scan_released) = mpsc::channel::<()>();
    let scan_released = std::sync::Mutex::new(scan_released);
    let waited_out = Arc::new(AtomicUsize::new(0));
    let flag = Arc::clone(&waited_out);
    let lister: Lister = Arc::new(move || {
        let _ = entered.send(());
        if let Err(mpsc::RecvTimeoutError::Timeout) = scan_released.lock().unwrap().recv_timeout(Duration::from_secs(10)) {
            flag.fetch_add(1, Ordering::Relaxed);
        }
        Vec::new()
    });
    let cat = Arc::new(Catalog::new(linux_env(&root)).with_font_service(false).with_lister(lister));
    let bold = cat.add_bytes(font(&spec("Test Sans", "Bold", 700)), Source::Bundled, &Limits::default()).unwrap();
    assert!(!cat.is_ready(), "adding started no scan");
    cat.scan_in_background();
    scan_entered.recv_timeout(Duration::from_secs(10)).unwrap();
    cat.add_bytes(font(&spec("App Face", "Regular", 400)), Source::Bundled, &Limits::default()).unwrap();
    release.send(()).unwrap();
    // Later scans (the rescans below) go straight through.
    drop(release);
    assert_eq!(cat.styles("App Face").len(), 1);
    assert_eq!(waited_out.load(Ordering::Relaxed), 0, "the add during the scan returned before the scan did");
    // The added Bold comes first; the installed file's Bold is another version of it, not another face.
    assert_eq!(cat.resolve("Test Sans", "Bold").map(|(f, _)| f), bold.first().cloned());
    assert_eq!(cat.versions(&bold[0]).len(), 2);
    let with_added = cat.rescan();
    assert_eq!(cat.remove_added(Source::Bundled), 2);
    assert_eq!(cat.rescan(), with_added - 1, "only the added face no installed file has was counted");
    assert!(cat.versions(&bold[0]).is_empty());
}
