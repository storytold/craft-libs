//! The main flows against a small index, a font folder made per test and a fake font store.
//! `CRAFT_FONT_MANAGER_SNAPSHOTS=<dir>` also saves screenshots; run that with
//! `-- --test-threads=1`, as rendering on several threads at once can crash older Vulkan loaders.
//! With `CRAFT_FONT_MANAGER_GOOGLE_FONTS=<google/fonts checkout>` the fake store also "downloads"
//! the index's files from it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use craft_font_manager::craft_font_catalog::Os;
use craft_font_manager::index::Index;
use craft_font_manager::remote::{Fetch, Previews};
use craft_font_manager::store::{Call, FakeStore, Installer};
use craft_font_manager::{Catalog, FontEnv, Source};
use craft_font_manager::{FontsPanel, Look, Options, Palette, Runner, Settings, Tab, install_interface_fonts};
use craft_font_store::{Cancel, FetchError};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{By, NodeT, Queryable};

const HACK: &str = "ofl/hack/Hack-Regular.ttf";
const UBUNTU: &str = "ufl/ubuntu/Ubuntu-Light.ttf";

fn index() -> Index {
    let file = |path: &str, size: usize| {
        format!(r#"{{"path":"{path}","size":{size},"sha256":"{}","style":"normal","weight":400,"axes":[],"postscript":[]}}"#, "0a".repeat(32))
    };
    let family = |name: &str, category: &str, languages: &str, scripts: &str, files: String| {
        format!(
            r#"{{"family":"{name}","designer":"","categories":["{category}"],"languages":[{languages}],"scripts":[{scripts}],"license":"OFL","license_path":null,"files":[{files}]}}"#
        )
    };
    let json = format!(
        r#"{{"schema":1,"version":"1.0","source":"google/fonts","commit":"45e3d5269e4a17d823f6d84470bc084dc5ae46f6","base_url":"https://cdn.jsdelivr.net/gh/google/fonts@45e3d5269e4a17d823f6d84470bc084dc5ae46f6/","fallback_base_url":"https://raw.githubusercontent.com/google/fonts/45e3d5269e4a17d823f6d84470bc084dc5ae46f6/",
        "languages":[
          {{"id":"ar_Arab","name":"Arabic","autonym":"العربية","script":"Arab","population":1,"historical":false}},
          {{"id":"en_Latn","name":"English","autonym":"English","script":"Latn","population":1,"historical":false}},
          {{"id":"ja_Jpan","name":"Japanese","autonym":"日本語","script":"Jpan","population":1,"historical":false}}],
        "scripts":[{{"id":"Arab","name":"Arabic"}},{{"id":"Jpan","name":"Japanese"}},{{"id":"Latn","name":"Latin"}}],
        "families":[{},{},{},{}]}}"#,
        family("Amiri", "SERIF", "0,1", r#""Arab","Latn""#, file("ofl/amiri/Amiri-Regular.ttf", 1000)),
        family("Hack", "MONOSPACE", "1", r#""Latn""#, file(HACK, epaint_default_fonts::HACK_REGULAR.len())),
        family("Noto Sans JP", "SANS_SERIF", "1,2", r#""Jpan","Latn""#, file("ofl/notosansjp/NotoSansJP[wght].ttf", 1000)),
        family("Ubuntu", "SANS_SERIF", "1", r#""Latn""#, file(UBUNTU, epaint_default_fonts::UBUNTU_LIGHT.len())),
    );
    Index::from_json(json.as_bytes()).unwrap()
}

/// A system with Ubuntu Light installed, an empty font store, and a fake store that can "download"
/// Hack and Ubuntu Light.
fn setup(name: &str, settings: Settings) -> (Harness<'static, FontsPanel>, Arc<FakeStore>) {
    setup_with(name, settings, Runner::Inline, |_| {})
}

/// [`setup`] with a job runner and the context set up by `fonts` (the interface fonts and
/// direction), at the launcher's window size less its header and footer.
fn setup_with(
    name: &str,
    settings: Settings,
    runner: Runner,
    fonts: impl Fn(&egui::Context) + 'static,
) -> (Harness<'static, FontsPanel>, Arc<FakeStore>) {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("flows").join(name);
    let _ = std::fs::remove_dir_all(&root);
    let system = root.join("usr/share/fonts");
    std::fs::create_dir_all(&system).unwrap();
    std::fs::write(system.join("Ubuntu-Light.ttf"), epaint_default_fonts::UBUNTU_LIGHT).unwrap();
    let store_dir = root.join("store");
    std::fs::create_dir_all(&store_dir).unwrap();
    let env = FontEnv { os: Os::Unix, root: root.clone(), font_store: Some(store_dir.clone()), ..FontEnv::default() };
    let catalog = Arc::new(Catalog::new(env).with_font_service(false));
    catalog.rescan();
    let files = HashMap::from([
        (HACK.to_owned(), epaint_default_fonts::HACK_REGULAR.to_vec()),
        (UBUNTU.to_owned(), epaint_default_fonts::UBUNTU_LIGHT.to_vec()),
    ]);
    let store = FakeStore::new(store_dir, files);
    let store = Arc::new(match std::env::var_os("CRAFT_FONT_MANAGER_GOOGLE_FONTS") {
        Some(checkout) => store.serving(checkout),
        None => store,
    });
    let options = Options { catalog, installer: store.clone(), index: Ok(index()), runner };
    let app = FontsPanel::new(options, settings);
    let mut harness = Harness::builder()
        .with_size([1120.0, 678.0])
        // Autonyms such as العربية need fonts beyond egui's; the tests don't depend on the host's.
        .allow_missing_glyphs()
        .with_context_setup(move |ctx| {
            install_interface_fonts(ctx, &[]);
            fonts(ctx);
        })
        .build_ui_state(|ui, app: &mut FontsPanel| app.ui(ui), app);
    harness.run_steps(4);
    (harness, store)
}

fn settle(h: &mut Harness<'_, FontsPanel>) {
    h.run_steps(4);
}

fn click(h: &mut Harness<'_, FontsPanel>, label: &str) {
    h.get_all_by_label(label).next().unwrap_or_else(|| panic!("no {label:?}")).click();
    settle(h);
}

/// Clicks the first node whose name has every one of `parts` (names with a placeholder are
/// isolated for right-to-left text).
fn click_parts(h: &mut Harness<'_, FontsPanel>, parts: &[&str]) {
    let wanted: Vec<String> = parts.iter().map(|p| (*p).to_owned()).collect();
    let by = By::new().predicate(move |n| n.label().is_some_and(|l| wanted.iter().all(|p| l.contains(p.as_str()))));
    h.get_all(by).next().unwrap_or_else(|| panic!("no {parts:?}")).click();
    settle(h);
}

fn click_role(h: &mut Harness<'_, FontsPanel>, role: Role, label: &str) {
    h.get_by_role_and_label(role, label).click();
    settle(h);
}

fn type_into(h: &mut Harness<'_, FontsPanel>, label: &str, text: &str) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    h.step();
    h.get_by_role_and_label(Role::TextInput, label).type_text(text);
    settle(h);
}

fn allow(h: &mut Harness<'_, FontsPanel>) {
    h.state_mut().set_downloads_allowed(true).unwrap();
    settle(h);
}

fn installed_tab(h: &mut Harness<'_, FontsPanel>) {
    click_role(h, Role::Tab, "Installed");
    assert_eq!(h.state().tab(), Tab::Installed);
}

fn has_installed(h: &Harness<'_, FontsPanel>, family: &str) -> bool {
    h.state().installed().iter().any(|f| f.name == family)
}

#[test]
fn search_and_filters_narrow_the_catalogue() {
    let (mut h, _) = setup("search", Settings::default());
    assert_eq!(h.state().tab(), Tab::Catalogue);
    assert_eq!(h.state().results(), ["Amiri", "Hack", "Noto Sans JP", "Ubuntu"]);

    type_into(&mut h, "Search", "noto");
    assert_eq!(h.state().results(), ["Noto Sans JP"]);

    type_into(&mut h, "Search", "XYZ");
    assert!(h.state().results().is_empty());
    assert!(h.query_by_label("No fonts match").is_some());
    click(&mut h, "Clear search and filters");
    assert_eq!(h.state().results().len(), 4);

    click_role(&mut h, Role::RadioButton, "Serif");
    assert_eq!(h.state().results(), ["Amiri"]);
    click_role(&mut h, Role::RadioButton, "All categories");

    // Ubuntu Light is installed on the system.
    click_role(&mut h, Role::RadioButton, "Installed");
    assert_eq!(h.state().results(), ["Ubuntu"]);
    click_role(&mut h, Role::RadioButton, "Not installed");
    assert_eq!(h.state().results(), ["Amiri", "Hack", "Noto Sans JP"]);
    click(&mut h, "Clear filters");
    assert_eq!(h.state().results().len(), 4);
}

#[test]
fn language_filter_finds_languages_by_their_own_name() {
    let (mut h, _) = setup("language", Settings::default());
    click_role(&mut h, Role::ComboBox, "Language");
    type_into(&mut h, "Find a language", "日本");
    assert!(h.query_by_label("Arabic · العربية").is_none());
    click(&mut h, "Japanese · 日本語");
    assert_eq!(h.state().results(), ["Noto Sans JP"]);
}

#[test]
fn install_a_family_then_remove_it_from_its_card() {
    let (mut h, store) = setup("install", Settings::default());
    assert!(!has_installed(&h, "Hack"));

    click(&mut h, "Hack");
    assert_eq!(h.state().details_family().as_deref(), Some("Hack"));
    click(&mut h, "Install all");
    click(&mut h, "Download");
    assert_eq!(store.calls(), [Call::Install("Hack".into())]);
    assert!(h.state().downloads_allowed());
    let hack = h.state().installed().iter().find(|f| f.name == "Hack").cloned().expect("Hack is listed");
    assert_eq!(hack.source, Source::Store);
    assert!(h.query_by_label("Remove all").is_some());

    h.key_press(egui::Key::Escape);
    settle(&mut h);
    assert!(h.state().details_family().is_none());
    installed_tab(&mut h);
    click(&mut h, "Remove Hack");
    assert_eq!(store.calls().last(), Some(&Call::Remove("Hack".into())));
    assert!(!has_installed(&h, "Hack"));
}

#[test]
fn install_and_remove_one_style() {
    let (mut h, store) = setup("style", Settings::default());
    allow(&mut h);
    click(&mut h, "Hack");
    click(&mut h, "Install Regular");
    assert_eq!(store.calls(), [Call::InstallFile("Hack".into(), HACK.into())]);
    assert!(has_installed(&h, "Hack"));

    click(&mut h, "Remove Regular");
    assert_eq!(store.calls().last(), Some(&Call::Remove("Hack".into())));
    assert!(!has_installed(&h, "Hack"));
}

#[test]
fn system_fonts_cannot_be_removed() {
    let (mut h, store) = setup("system", Settings::default());
    installed_tab(&mut h);
    let system = h.state().installed().first().cloned().expect("the system font is listed");
    assert_eq!(system.source, Source::System);
    assert!(h.query_by_label(&format!("Remove {}", system.name)).is_none());
    click(&mut h, &system.name);
    let remove = h.get_by_label("Remove all");
    assert!(remove.accesskit_node().is_disabled());
    remove.click();
    settle(&mut h);
    assert!(store.calls().is_empty());
}

#[test]
fn nothing_downloads_without_consent() {
    let (mut h, store) = setup("consent", Settings::default());
    assert!(h.query_by_label("Downloads are off").is_some());
    click(&mut h, "Hack");

    click(&mut h, "Preview");
    assert!(h.query_by_label("Download from Google Fonts?").is_some());
    click(&mut h, "Cancel");
    click(&mut h, "Install all");
    click(&mut h, "Cancel");
    assert!(store.calls().is_empty());
    assert!(!h.state().downloads_allowed());
    assert!(h.query_by_label("Download from Google Fonts?").is_none());
    h.key_press(egui::Key::Escape);
    settle(&mut h);

    // Allowed from the note, a preview downloads without asking and installs nothing.
    click_role(&mut h, Role::Button, "Allow downloads");
    assert!(h.state().downloads_allowed());
    assert!(h.query_by_label("Downloads are off").is_none());
    click(&mut h, "Preview Hack");
    assert_eq!(store.calls(), [Call::Download("Hack".into())]);
    assert!(!has_installed(&h, "Hack"));
}

#[test]
fn a_failed_install_offers_retry() {
    let (mut h, store) = setup("retry", Settings::default());
    allow(&mut h);
    store.fail_next("network unreachable");
    click(&mut h, "Install Hack");
    assert!(h.query_by_label("Couldn't install").is_some());
    assert!(!has_installed(&h, "Hack"));
    click(&mut h, "Retry");
    assert!(has_installed(&h, "Hack"));
    assert!(h.query_by_label("Couldn't install").is_none());
}

#[test]
fn arrow_keys_move_between_cards_and_enter_opens_one() {
    let (mut h, _) = setup("keys", Settings::default());
    h.get_by_role_and_label(Role::Button, "Amiri").focus();
    settle(&mut h);
    h.key_press(egui::Key::ArrowDown);
    settle(&mut h);
    h.key_press(egui::Key::Enter);
    settle(&mut h);
    assert_eq!(h.state().details_family().as_deref(), Some("Hack"));
}

const USER_INDEX: &str = "https://fonts.example.org/index.json";

/// A source a user could add: another "Hack", from an index of its own.
fn user_index() -> Vec<u8> {
    format!(
        r#"{{"schema":1,"version":"1.1","source":"examplefonts","commit":"{commit}",
        "source_info":{{"id":"examplefonts","name":"Example Fonts","curated":true,"file_urls":["https://fonts.example.org/files/{{path}}"]}},
        "languages":[{{"id":"en_Latn","name":"English","script":"Latn","population":1}}],
        "scripts":[{{"id":"Latn","name":"Latin"}}],
        "families":[{{"family":"Hack","source":"examplefonts","categories":["MONOSPACE"],"license":"OFL","license_path":null,"languages":[0],"scripts":["Latn"],
          "files":[{{"path":"{HACK}","size":{size},"sha256":"{sha}","style":"normal","weight":400,"axes":[],"postscript":[]}}]}}]}}"#,
        commit = "1".repeat(40),
        size = epaint_default_fonts::HACK_REGULAR.len(),
        sha = "0a".repeat(32),
    )
    .into_bytes()
}

/// The panel with its sources beside it, as a host's settings show them.
fn sources_setup(name: &str) -> (Harness<'static, FontsPanel>, Arc<FakeStore>) {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("flows").join(name);
    let _ = std::fs::remove_dir_all(&root);
    let store_dir = root.join("store");
    std::fs::create_dir_all(&store_dir).unwrap();
    let env = FontEnv { os: Os::Unix, root: root.clone(), font_store: Some(store_dir.clone()), ..FontEnv::default() };
    let catalog = Arc::new(Catalog::new(env).with_font_service(false));
    catalog.rescan();
    let store = Arc::new(FakeStore::new(store_dir, HashMap::new()).with_index(index()).serving_index(USER_INDEX, user_index()));
    store.allow_downloads(true).unwrap();
    let options = Options { catalog, installer: store.clone(), index: store.indexes().map_err(|e| e.0), runner: Runner::Inline };
    let mut harness = Harness::builder()
        .with_size([1400.0, 760.0])
        .allow_missing_glyphs()
        .with_context_setup(|ctx| install_interface_fonts(ctx, &[]))
        .build_ui_state(
            |ui, app: &mut FontsPanel| {
                egui::Panel::left("sources").exact_size(420.0).show(ui, |ui| app.sources_ui(ui));
                egui::CentralPanel::default().show(ui, |ui| app.ui(ui));
            },
            FontsPanel::new(options, Settings::default()),
        );
    harness.run_steps(4);
    (harness, store)
}

/// Steps until `done`, for work on the panel's own threads.
fn wait(h: &mut Harness<'_, FontsPanel>, done: impl Fn(&FontsPanel) -> bool) {
    for _ in 0..200 {
        if done(h.state()) {
            settle(h);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
        h.step();
    }
    panic!("timed out");
}

#[test]
fn an_added_source_is_not_curated_and_can_be_removed() {
    let (mut h, store) = sources_setup("sources");
    assert_eq!(h.state().results().len(), 4);
    type_into(&mut h, "Index address", USER_INDEX);
    click_role(&mut h, Role::Button, "Add");
    wait(&mut h, |p| p.results().len() == 5);
    // Two families named Hack, one from each source, each with its own card.
    assert_eq!(h.state().results().iter().filter(|f| *f == "Hack").count(), 2);
    assert!(h.query_all_by_label("Not curated").count() >= 2, "a badge on the card and one in the sources");
    assert!(h.query_by_label("Changed since added").is_none());

    store.change_index(USER_INDEX);
    click(&mut h, "Update sources");
    wait(&mut h, |p| !p.fetching_sources());
    assert!(h.query_by_label("Changed since added").is_some());

    click_parts(&mut h, &["Remove", USER_INDEX]);
    assert_eq!(h.state().results().len(), 4);
    assert!(store.user_sources().is_empty());
}

/// The launcher's colours, for screenshots that look as the panel does there.
fn launcher_look() -> Look {
    let hex = |rgb: u32| egui::Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
    let ink = |rgb: u32, a: u8| egui::Color32::from_rgba_unmultiplied((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, a);
    let dark = Palette {
        bg: hex(0x121316),
        sunken: hex(0x060607),
        ink: hex(0xf2f1ee),
        ink_strong: hex(0xffffff),
        muted: ink(0xf2f1ee, 153),
        faint: ink(0xf2f1ee, 97),
        line: ink(0xf2f1ee, 38),
        line_strong: ink(0xf2f1ee, 102),
        accent: hex(0x2d81ff),
        accent_ink: hex(0x74aaff),
        invert_bg: hex(0xffffff),
        invert_fg: hex(0x0b0b0c),
        good: hex(0x3fd6b8),
        warn: hex(0xffc14d),
        bad: hex(0xff6259),
    };
    Look { dark: Some(dark), caps: true, corner: egui::CornerRadius::ZERO, control_height: 34.0, ..Look::default() }
}

fn save(h: &mut Harness<'_, FontsPanel>, dir: &std::ffi::OsStr, name: &str) {
    h.run_steps(8);
    let image = h.render().unwrap();
    std::fs::create_dir_all(dir).unwrap();
    image.save(PathBuf::from(dir).join(format!("{name}.png"))).unwrap();
}

/// The interface fonts with the system's for the scripts egui's own faces lack.
fn system_fonts(language: &'static str) -> impl Fn(&egui::Context) + 'static {
    move |ctx| {
        use craft_ui::fonts::{I18nScripts, UiFonts};
        let provider = Arc::new(egui_system_font_provider::SystemFontProvider::new().with_locale(Some(language)));
        UiFonts::new(&I18nScripts).languages(&["en", language]).ui_language(language).system_provider(provider).install(ctx);
        craft_ui::direction::apply(ctx, craft_ui::direction::resolve(language, craft_ui::direction::Setting::Language));
    }
}

/// `CRAFT_FONT_MANAGER_SNAPSHOTS=<dir>`: the panel's main states.
#[test]
fn screenshots() {
    let Some(dir) = std::env::var_os("CRAFT_FONT_MANAGER_SNAPSHOTS") else { return };
    let shot = |name: &str, language: &'static str, f: &dyn Fn(&mut Harness<'_, FontsPanel>, &FakeStore)| {
        let (mut h, store) = setup_with(&format!("shot-{name}"), Settings::default(), Runner::Thread, system_fonts(language));
        h.state_mut().set_look(launcher_look());
        h.state_mut().set_language(language);
        settle(&mut h);
        f(&mut h, &store);
        save(&mut h, &dir, name);
        store.release();
    };
    shot("catalogue-downloads-off", "en", &|_, _| {});
    shot("catalogue", "en", &|h, _| {
        allow(h);
        click(h, "Preview Hack");
        h.run_steps(20);
    });
    shot("downloading", "en", &|h, store| {
        allow(h);
        store.hold();
        click(h, "Install Hack");
        h.run_steps(10);
    });
    shot("failed", "en", &|h, store| {
        allow(h);
        store.fail_next("couldn't reach cdn.jsdelivr.net or raw.githubusercontent.com");
        click(h, "Install Hack");
        h.run_steps(20);
    });
    shot("empty-search", "en", &|h, _| type_into(h, "Search", "Garamondish"));
    shot("installed", "en", &|h, _| {
        allow(h);
        click(h, "Install Hack");
        h.run_steps(20);
        installed_tab(h);
    });
    shot("details", "en", &|h, _| {
        allow(h);
        click(h, "Preview Hack");
        h.run_steps(20);
        click(h, "Hack");
    });
    shot("details-not-installed", "en", &|h, _| click(h, "Noto Sans JP"));
    shot("arabic", "ar", &|h, _| {
        allow(h);
        click_parts(h, &["معاينة", "Amiri"]);
        h.run_steps(20);
        click_role(h, Role::ComboBox, "اللغة");
        type_into(h, "ابحث عن لغة", "Arabic");
        click(h, "Arabic · العربية");
    });
    shot("arabic-details", "ar", &|h, _| {
        allow(h);
        click_parts(h, &["معاينة", "Amiri"]);
        h.run_steps(20);
        click(h, "Amiri");
    });
}

#[test]
fn fake_store_checks_the_preview_service_like_the_store() {
    let store = FakeStore::new(std::env::temp_dir().join("craft-fm-fake-service"), HashMap::new());
    for bad in [
        "http://fonts.example.com",
        "http://localhost:8080",
        "ftp://fonts.example.com",
        "https://user@fonts.example.com",
        "https://fonts.example.com/?k=v",
    ] {
        assert!(store.set_preview_service(Some(bad)).is_err(), "{bad}");
    }
    store.set_preview_service(Some("http://127.0.0.1:8080/")).unwrap();
    assert_eq!(store.preview_service().as_deref(), Some("http://127.0.0.1:8080"));
    store.set_preview_service(None).unwrap();
    assert_eq!(store.preview_service(), None);
}

/// A preview service that answers every subset with the same small font, and logs what it
/// answered. While held it answers nothing, and a request cancelled meanwhile isn't logged.
#[derive(Default)]
struct FakeService {
    answered: std::sync::Mutex<Vec<String>>,
    held: std::sync::Mutex<bool>,
    /// Answers sample subsets with HTTP 404, as for a sample text the service doesn't have.
    no_samples: std::sync::Mutex<bool>,
}

impl Fetch for FakeService {
    fn get(&self, url: &str, _: u64, cancel: &Cancel) -> Result<Vec<u8>, FetchError> {
        while *self.held.lock().unwrap() {
            if cancel.is_cancelled() {
                return Err(FetchError::Cancelled);
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        self.answered.lock().unwrap().push(url.to_owned());
        if *self.no_samples.lock().unwrap() && url.contains("/sample-") {
            return Err(FetchError::Failed("HTTP 404".into()));
        }
        Ok(include_bytes!("data/Tiny-glyf.woff2").to_vec())
    }
}

impl FakeService {
    /// The URLs answered since the last call.
    fn urls(&self) -> Vec<String> {
        std::mem::take(&mut *self.answered.lock().unwrap())
    }

    /// What was answered since the last call, as (family number, subset): `sample`, `ascii`, or
    /// `u` for typed characters.
    fn take(&self) -> Vec<(usize, String)> {
        let urls = std::mem::take(&mut *self.answered.lock().unwrap());
        urls.iter()
            .map(|url| {
                let path = url.split("/v1/faces/").nth(1).unwrap();
                let n = usize::from_str_radix(&path[..2], 16).unwrap();
                let subset = path[64..].trim_start_matches("/list/").split(['/', '.']).next().unwrap();
                let subset = if subset.starts_with("sample-") { "sample" } else { subset }.to_owned();
                (n, subset)
            })
            .collect()
    }
}

/// Forty Latin families, `Family 01` to `Family 40`, whose one file's sha256 starts with the
/// family's number in hex, and an index sample for Latin.
fn service_index() -> Index {
    service_index_with(r#"{"Latn":{"text":"Sphinx of black quartz, judge my vow","method":"udhr","language":"en_Latn"}}"#)
}

/// [`service_index`] with `samples` as its sample texts.
fn service_index_with(samples: &str) -> Index {
    let families: Vec<String> = (1..=40)
        .map(|n| {
            format!(
                r#"{{"family":"Family {n:02}","categories":["SANS_SERIF"],"languages":[0],"scripts":["Latn"],"license":"OFL","sample_script":"Latn",
                "files":[{{"path":"ofl/family{n}/Family-Regular.ttf","size":1000,"sha256":"{}","style":"normal","weight":400,"axes":[],"postscript":[]}}]}}"#,
                format!("{n:02x}").repeat(32)
            )
        })
        .collect();
    let json = format!(
        r#"{{"schema":1,"version":"1.2","source":"google/fonts","commit":"45e3d5269e4a17d823f6d84470bc084dc5ae46f6","base_url":"https://cdn.jsdelivr.net/gh/google/fonts@45e3d5269e4a17d823f6d84470bc084dc5ae46f6/",
        "languages":[{{"id":"en_Latn","name":"English","script":"Latn","population":1}}],
        "scripts":[{{"id":"Latn","name":"Latin"}}],
        "samples":{samples},
        "families":[{}]}}"#,
        families.join(",")
    );
    Index::from_json(json.as_bytes()).unwrap()
}

/// The panel over [`service_index`] with no installed fonts, previews fetched from `service` by
/// `workers` threads, and the preview service not yet set.
fn service_setup(name: &str, service: &Arc<FakeService>, workers: usize) -> (Harness<'static, FontsPanel>, Arc<FakeStore>) {
    service_setup_with(name, service, workers, service_index())
}

/// [`service_setup`] over `index`.
fn service_setup_with(name: &str, service: &Arc<FakeService>, workers: usize, index: Index) -> (Harness<'static, FontsPanel>, Arc<FakeStore>) {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("flows").join(name);
    let _ = std::fs::remove_dir_all(&root);
    let store_dir = root.join("store");
    std::fs::create_dir_all(&store_dir).unwrap();
    let env = FontEnv { os: Os::Unix, root: root.clone(), font_store: Some(store_dir.clone()), ..FontEnv::default() };
    let catalog = Arc::new(Catalog::new(env).with_font_service(false));
    catalog.rescan();
    let store = Arc::new(FakeStore::new(store_dir, HashMap::new()));
    let previews = Previews::new(store.clone()).with_fetcher(service.clone()).with_cache(None).with_workers(workers);
    let options = Options { catalog, installer: store.clone(), index: Ok(index), runner: Runner::Inline };
    let panel = FontsPanel::new(options, Settings::default()).with_previews(previews);
    let mut harness = Harness::builder()
        .with_size([1120.0, 678.0])
        .allow_missing_glyphs()
        .with_context_setup(|ctx| install_interface_fonts(ctx, &[]))
        .build_ui_state(|ui, app: &mut FontsPanel| app.ui(ui), panel);
    harness.run_steps(4);
    (harness, store)
}

/// Sets the preview service, as a host's settings page does.
fn set_service(h: &mut Harness<'_, FontsPanel>, store: &FakeStore) {
    store.set_preview_service(Some("http://127.0.0.1:9")).unwrap();
    h.state_mut().reload_preview_settings();
    settle(h);
}

/// Steps until the panel has fetched what it asks for and asks for nothing more.
fn idle(h: &mut Harness<'_, FontsPanel>) {
    let mut quiet = 0;
    for _ in 0..500 {
        h.step();
        quiet = if h.state().loading_previews() { 0 } else { quiet + 1 };
        if quiet == 5 {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("previews kept loading");
}

/// Steps for longer than the pause after typing.
fn pause(h: &mut Harness<'_, FontsPanel>) {
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_millis(300) {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// The family numbers of the cards on screen, in list order.
fn on_screen(h: &Harness<'_, FontsPanel>) -> Vec<usize> {
    let by = By::new().role(Role::Button).predicate(|n| n.label().is_some_and(|l| l.starts_with("Family ") && l.len() == 9));
    let mut shown: Vec<usize> = h.get_all(by).map(|n| n.accesskit_node().label().unwrap()[7..].parse().unwrap()).collect();
    shown.sort();
    shown
}

fn numbers(answered: &[(usize, String)], subset: &str) -> Vec<usize> {
    let mut n: Vec<usize> = answered.iter().filter(|(_, s)| s == subset).map(|(n, _)| *n).collect();
    n.sort();
    n
}

#[test]
fn list_previews_follow_the_screen_and_the_typed_text() {
    let service = Arc::new(FakeService::default());
    let (mut h, store) = service_setup("list-previews", &service, 8);
    // No service: the list stays as it is without one.
    settle(&mut h);
    assert!(service.take().is_empty());
    let preview = || By::new().predicate(|n| n.label().is_some_and(|l| l.contains("Preview") && l.contains("Family 01")));
    assert!(h.get_all(preview()).next().is_some(), "the whole-family preview is offered");

    // The previews setting off: nothing is fetched.
    store.set_previews_enabled(false).unwrap();
    set_service(&mut h, &store);
    assert!(!h.state().loading_previews());
    assert!(service.take().is_empty());

    // On, with downloads (installs) still off: previews load without asking.
    store.set_previews_enabled(true).unwrap();
    h.state_mut().reload_preview_settings();
    idle(&mut h);
    assert!(!h.state().downloads_allowed());
    assert!(h.query_by_label("Nothing is installed until you allow downloads.").is_some());
    assert!(h.query_all(preview()).next().is_none(), "no session download while the service previews");
    let shown = on_screen(&h);
    assert!(shown.len() >= 3 && shown.len() < 10, "{shown:?}");
    assert_eq!(shown, (1..=shown.len()).collect::<Vec<_>>());
    let answered = service.take();
    // The screen and two more below (none above the first row), then ASCII for the screen.
    assert_eq!(numbers(&answered, "sample"), (1..=3 * shown.len()).collect::<Vec<_>>());
    assert_eq!(numbers(&answered, "ascii"), shown);
    assert_eq!(answered.len(), 4 * shown.len(), "{answered:?}");

    type_into(&mut h, "Sample text", "Hello, world");
    pause(&mut h);
    idle(&mut h);
    assert!(service.take().is_empty(), "the ASCII subsets have typed ASCII");

    h.state_mut().settings_mut().sample.clear();
    type_into(&mut h, "Sample text", "สวัสดีครับ");
    pause(&mut h);
    idle(&mut h);
    let answered = service.take();
    assert_eq!(numbers(&answered, "u"), shown, "one subset of the typed characters per row on screen");
    assert_eq!(answered.len(), shown.len(), "{answered:?}");
}

#[test]
fn a_sample_the_service_lacks_falls_back_to_its_characters_once() {
    let service = Arc::new(FakeService::default());
    *service.no_samples.lock().unwrap() = true;
    let (mut h, store) = service_setup("list-previews-404", &service, 8);
    store.set_previews_enabled(true).unwrap();
    set_service(&mut h, &store);
    idle(&mut h);
    let shown = on_screen(&h);
    let answered = service.take();
    assert_eq!(numbers(&answered, "sample"), (1..=3 * shown.len()).collect::<Vec<_>>(), "{answered:?}");
    assert_eq!(numbers(&answered, "u"), shown, "one subset of the sample's characters per row on screen");
    for _ in 0..3 {
        pause(&mut h);
        idle(&mut h);
    }
    assert!(numbers(&service.take(), "sample").is_empty(), "the sample is not asked for again");
}

#[test]
fn a_family_without_a_sample_text_gets_its_rows_subset() {
    let service = Arc::new(FakeService::default());
    let (mut h, store) = service_setup_with("list-previews-no-sample", &service, 8, service_index_with("{}"));
    store.set_previews_enabled(true).unwrap();
    set_service(&mut h, &store);
    idle(&mut h);
    let shown = on_screen(&h);
    let answered = service.take();
    assert!(numbers(&answered, "sample").is_empty(), "{answered:?}");
    assert_eq!(numbers(&answered, "ascii"), shown, "{answered:?}");
}

#[test]
fn scrolling_cancels_rows_that_left() {
    let service = Arc::new(FakeService::default());
    *service.held.lock().unwrap() = true;
    let (mut h, store) = service_setup("list-previews-scroll", &service, 1);
    set_service(&mut h, &store);
    assert!(h.state().loading_previews());
    assert_eq!(on_screen(&h).first(), Some(&1));

    h.hover_at(egui::pos2(700.0, 400.0));
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -100_000.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Wheel,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(30);
    let shown = on_screen(&h);
    assert_eq!(shown.last(), Some(&40), "{shown:?}");
    *service.held.lock().unwrap() = false;
    idle(&mut h);
    let answered = service.take();
    let first = shown[0] - shown.len();
    assert!(!answered.is_empty());
    assert!(answered.iter().all(|(n, _)| *n >= first), "rows that left the window were fetched: {answered:?}");
}

/// One family: a variable upright file (`wght` 100 to 900, three named instances) whose sha256
/// is `ab…`, and a static italic, `cd…`.
fn variable_index() -> Index {
    let json = format!(
        r#"{{"schema":1,"version":"1.2","source":"google/fonts","commit":"45e3d5269e4a17d823f6d84470bc084dc5ae46f6","base_url":"https://cdn.jsdelivr.net/gh/google/fonts@45e3d5269e4a17d823f6d84470bc084dc5ae46f6/",
        "languages":[{{"id":"en_Latn","name":"English","script":"Latn","population":1}}],
        "scripts":[{{"id":"Latn","name":"Latin"}}],
        "samples":{{"Latn":{{"text":"Sphinx of black quartz, judge my vow","method":"udhr","language":"en_Latn"}}}},
        "families":[{{"family":"Varia","categories":["SANS_SERIF"],"languages":[0],"scripts":["Latn"],"license":"OFL","sample_script":"Latn",
          "files":[
            {{"path":"ofl/varia/Varia[wght].ttf","size":1000,"sha256":"{}","style":"normal","weight":400,"axes":[{{"tag":"wght","min":100,"max":900}}],"postscript":[],
              "instances":[{{"name":"Thin","coordinates":{{"wght":100}}}},{{"name":"Regular","coordinates":{{"wght":400}}}},{{"name":"Bold","coordinates":{{"wght":700}}}}],"list_instance":1}},
            {{"path":"ofl/varia/Varia-Italic.ttf","size":1000,"sha256":"{}","style":"italic","weight":400,"axes":[],"postscript":[]}}]}}]}}"#,
        "ab".repeat(32),
        "cd".repeat(32)
    );
    Index::from_json(json.as_bytes()).unwrap()
}

#[test]
fn a_variable_family_draws_one_var_subset_at_every_axis_value() {
    let service = Arc::new(FakeService::default());
    let (mut h, store) = service_setup_with("variable-details", &service, 8, variable_index());
    // The previews setting off: the detail view asks for nothing.
    store.set_previews_enabled(false).unwrap();
    set_service(&mut h, &store);
    click(&mut h, "Varia");
    assert_eq!(h.state().details_family().as_deref(), Some("Varia"));
    idle(&mut h);
    assert!(service.urls().is_empty());

    // On, with downloads still off.
    store.set_previews_enabled(true).unwrap();
    h.state_mut().reload_preview_settings();
    idle(&mut h);
    assert!(!h.state().downloads_allowed());
    let urls = service.urls();
    let var: Vec<&String> = urls.iter().filter(|u| u.contains("/var/")).collect();
    assert_eq!(var.len(), 1, "one subset for the variable file's every style: {urls:?}");
    assert!(var[0].ends_with(&format!("{}/var/sample-7eb13ed99f41.woff2", "ab".repeat(32))), "{urls:?}");
    assert!(
        urls.iter().any(|u| u.ends_with(&format!("{}/list/sample-7eb13ed99f41.woff2", "cd".repeat(32)))),
        "the static italic's own sample: {urls:?}"
    );
    for style in ["Thin", "Regular", "Bold"] {
        assert!(h.query_by_label(style).is_some(), "no {style} row");
    }

    let value = |h: &Harness<'_, FontsPanel>| h.get_by_role_and_label(Role::Slider, "Weight").accesskit_node().numeric_value();
    assert_eq!(value(&h), Some(400.0));
    h.get_by_role_and_label(Role::Slider, "Weight").focus();
    h.step();
    h.key_press(egui::Key::ArrowRight);
    settle(&mut h);
    assert_eq!(value(&h), Some(401.0));
    idle(&mut h);
    assert!(service.urls().is_empty(), "the slider asked for another subset");
}
