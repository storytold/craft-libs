use craft_testkit::{Finding, predefined_quit, std_instant, wgsl_float_literals, wgsl_float_literals_in};

/// A scratch folder under the system temporary folder that this test made and removes on drop,
/// also when the test panics. A folder that already exists is never emptied or reused.
#[cfg(not(target_arch = "wasm32"))]
struct ScratchDir(std::path::PathBuf);

#[cfg(not(target_arch = "wasm32"))]
impl ScratchDir {
    fn new(tag: &str) -> Self {
        let base = std::env::temp_dir();
        for n in 0..1000u32 {
            let path = base.join(format!("craft_testkit_{tag}_{}_{n}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("create scratch folder {}: {e}", path.display()),
            }
        }
        panic!("no free scratch folder name for {tag}");
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn write(&self, relative: &str, content: &str) {
        let path = self.0.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent folder");
        }
        std::fs::write(path, content).expect("write file");
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn file_names(sources: &[(String, String)]) -> Vec<String> {
    sources.iter().map(|(p, _)| std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).collect()
}

fn lines(findings: &[Finding]) -> Vec<usize> {
    findings.iter().map(|f| f.line).collect()
}

#[test]
fn test_finding_display() {
    let f1 = Finding::new("src/menu.rs", 42, "PredefinedMenuItem::quit()");
    assert_eq!(f1.to_string(), "src/menu.rs:42: PredefinedMenuItem::quit()");

    let f2 = Finding::new("", 156, "3.40282347e38");
    assert_eq!(f2.to_string(), "156: 3.40282347e38");
}

#[test]
fn test_predefined_quit_positive() {
    let source = r#"
fn setup_menu() {
    let quit = PredefinedMenuItem::quit(Some("Quit"));
    bar.add(quit);
}
"#;
    let findings = predefined_quit([("src/menu.rs", source)]);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].path, "src/menu.rs");
    assert_eq!(findings[0].line, 3);
    assert_eq!(findings[0].text, "let quit = PredefinedMenuItem::quit(Some(\"Quit\"));");
    assert_eq!(findings[0].to_string(), "src/menu.rs:3: let quit = PredefinedMenuItem::quit(Some(\"Quit\"));");
}

#[test]
fn test_predefined_quit_negative_and_comments() {
    let source = r#"
// PredefinedMenuItem::quit(None) - do not use this!
/*
 * PredefinedMenuItem::quit is hazardous
 */
fn setup_menu() {
    let sep = PredefinedMenuItem::separator();
    let hide = PredefinedMenuItem::hide(Some("Hide"));
    let note = "PredefinedMenuItem::quit in string should not trigger";
}
"#;
    let findings = predefined_quit([("src/safe_menu.rs", source)]);
    assert!(findings.is_empty(), "expected zero findings for comments, separators, and strings: {findings:?}");
}

/// Historical failure: FilmCraft `apps/filmcraft/src/native_menu.rs:56`
/// used `PredefinedMenuItem::quit`, routing directly to `NSApplication terminate:` and bypassing save prompts.
#[test]
fn test_historical_filmcraft_native_menu_quit_regression() {
    let problematic_source = r#"
    let predefined = [
        (hide_label, PredefinedMenuItem::hide(Some(app.ui.language.tr(hide_label)))),
        (hide_others_label, PredefinedMenuItem::hide_others(Some(app.ui.language.tr(hide_others_label)))),
        (quit_label, PredefinedMenuItem::quit(Some(app.ui.language.tr(quit_label)))),
    ];
"#;
    let findings = predefined_quit([("apps/filmcraft/src/native_menu.rs", problematic_source)]);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].line, 5);
    assert_eq!(findings[0].text, "(quit_label, PredefinedMenuItem::quit(Some(app.ui.language.tr(quit_label)))),");

    // Corrected source wires Quit via MenuItem::with_id and craft-quit lifecycle
    let corrected_source = r#"
    let quit = MenuItem::with_id("app.quit", label("app.quit", "Quit FilmCraft"), true, None);
    let predefined = [
        (hide_label, PredefinedMenuItem::hide(Some(app.ui.language.tr(hide_label)))),
        (hide_others_label, PredefinedMenuItem::hide_others(Some(app.ui.language.tr(hide_others_label)))),
    ];
"#;
    let corrected_findings = predefined_quit([("apps/filmcraft/src/native_menu.rs", corrected_source)]);
    assert!(corrected_findings.is_empty());
}

#[test]
fn test_std_instant_positive() {
    let source = r#"
use std::time::Instant;
use std::time::{Duration, Instant as StdInstant};

fn measure() {
    let t0 = std::time::Instant::now();
}
"#;
    let findings = std_instant([("src/clock.rs", source)]);
    assert_eq!(findings.len(), 3);
    assert_eq!(findings[0].line, 2);
    assert_eq!(findings[1].line, 3);
    assert_eq!(findings[2].line, 6);
}

#[test]
fn test_std_instant_negative_and_web_time() {
    let source = r#"
// std::time::Instant::now() causes wasm panic
/*
   use std::time::Instant;
*/
use web_time::Instant;
use std::time::Duration;

fn measure() {
    let t0 = web_time::Instant::now();
    let d = Duration::from_millis(100);
}
"#;
    let findings = std_instant([("src/safe_clock.rs", source)]);
    assert!(findings.is_empty(), "expected zero findings for web_time, duration, and comments: {findings:?}");
}

/// Historical failure: FilmCraft `crates/engine/src/wasm_clock_tests.rs`
/// jobs running synchronously on wasm32 called `std::time::Instant::now()`, panicking at runtime.
#[test]
fn test_historical_filmcraft_wasm_clock_instant_regression() {
    let problematic_source = r#"
    use std::time::Instant;

    pub fn run_job() {
        let t0 = std::time::Instant::now();
    }
"#;
    let findings = std_instant([("crates/engine/src/job.rs", problematic_source)]);
    assert_eq!(findings.len(), 2);

    let corrected_source = r#"
    use web_time::Instant;

    pub fn run_job() {
        let t0 = web_time::Instant::now();
    }
"#;
    let corrected_findings = std_instant([("crates/engine/src/job.rs", corrected_source)]);
    assert!(corrected_findings.is_empty());

    // User's specific conditional compilation test case:
    let native_guarded_inline = r#"
#[cfg(not(target_arch = "wasm32"))] fn timestamp() -> std::time::Instant { std::time::Instant::now() }
"#;
    assert!(std_instant([("src/native.rs", native_guarded_inline)]).is_empty());

    let native_guarded_multiline = r#"
#[cfg(not(target_arch = "wasm32"))]
fn timestamp() -> std::time::Instant {
    std::time::Instant::now()
}
"#;
    assert!(std_instant([("src/native.rs", native_guarded_multiline)]).is_empty());
}

/// Historical failure: PhotoCraft PR #1722 / #1982 `crates/gpu/src/compose.wgsl:156`
/// `const F32_MAX: f32 = 3.40282347e38;` is slightly above f32::MAX. Naga rounds it natively,
/// but Chromium WebGPU compiler (Tint) rejects it, disabling the GPU compositor.
#[test]
fn test_historical_photocraft_compose_wgsl_regression() {
    let problematic_shader = r#"
// line 156 of compose.wgsl
const F32_MAX: f32 = 3.40282347e38;
"#;
    let findings = wgsl_float_literals(problematic_shader);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].line, 3);
    assert_eq!(findings[0].text, "3.40282347e38");

    // Corrected shader uses exact hexadecimal float representation
    let corrected_shader = r#"
// exact f32::MAX bit for bit
const F32_MAX: f32 = 0x1.fffffep+127f;
"#;
    let corrected_findings = wgsl_float_literals(corrected_shader);
    assert!(corrected_findings.is_empty());
}

#[test]
fn test_wgsl_float_literals_controls() {
    // 1. Scientific notation and signed values
    let signed_and_sci = r#"
const OVER_POS: f32 = 1e39;
const OVER_NEG: f32 = -1e39;
const VALID_POS: f32 = 1.5e10;
const VALID_NEG: f32 = -1.5e10;
const VALID_TINY: f32 = 1e-40;
"#;
    let findings = wgsl_float_literals(signed_and_sci);
    assert_eq!(findings.len(), 2);
    assert_eq!(findings[0].text, "1e39");
    assert_eq!(findings[1].text, "1e39"); // Scanned magnitude of -1e39

    // 2. Hexadecimal floating-point representations
    let hex_floats = r#"
const EXACT_MAX: f32 = 0x1.fffffep+127f;
const ONE_P0: f32 = 0x1.0p0;
const HALF_HEX: f32 = 0x1.0p-1h;
"#;
    assert!(wgsl_float_literals(hex_floats).is_empty());

    // 3. Literals appearing inside comments
    let in_comments = r#"
// const F32_MAX = 3.40282347e38;
/*
   const OVERFLOW = 1e40;
*/
"#;
    assert!(wgsl_float_literals(in_comments).is_empty());

    // 4. Numeric suffixes (f, h)
    let suffixes = r#"
const BAD_F: f32 = 3.40282347e38f;
const GOOD_F: f32 = 1.0f;
const GOOD_H: f16 = 0.5h;
"#;
    let suffix_findings = wgsl_float_literals(suffixes);
    assert_eq!(suffix_findings.len(), 1);
    assert_eq!(suffix_findings[0].text, "3.40282347e38");

    // 5. Embedded in identifiers or other tokens
    let idents = r#"
let var_1 = 1.0;
let layer2 = 2.0;
let coord_0 = 0.0;
"#;
    assert!(wgsl_float_literals(idents).is_empty());
}

#[test]
fn test_wgsl_float_literals_in_multiple_files() {
    let s1 = "const A: f32 = 1.0;\nconst B: f32 = 3.40282347e38;\n";
    let s2 = "const OK: f32 = 0x1.fffffep+127f;\n";
    let s3 = "const BAD: f32 = 9.99e38;\n";

    let files = [("shaders/compose.wgsl", s1), ("shaders/clean.wgsl", s2), ("shaders/fx.wgsl", s3)];
    let findings = wgsl_float_literals_in(files);
    assert_eq!(findings.len(), 2);
    assert_eq!(findings[0].path, "shaders/compose.wgsl");
    assert_eq!(findings[0].line, 2);
    assert_eq!(findings[0].text, "3.40282347e38");
    assert_eq!(findings[1].path, "shaders/fx.wgsl");
    assert_eq!(findings[1].line, 1);
    assert_eq!(findings[1].text, "9.99e38");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_predefined_quit_qualified_and_whitespace() {
    let source = r#"
fn setup() {
    let q1 = muda::PredefinedMenuItem::quit(None);
    let q2 = PredefinedMenuItem :: quit(Some("Quit"));
    let q3 = crate::menu::PredefinedMenuItem::Quit;
}
"#;
    let findings = predefined_quit([("src/menu.rs", source)]);
    assert_eq!(findings.len(), 3);
    assert_eq!(findings[0].line, 3);
    assert_eq!(findings[1].line, 4);
    assert_eq!(findings[2].line, 5);
}

#[test]
fn test_std_instant_module_alias() {
    let source = r#"
use std::time;

fn run() {
    let t0 = time::Instant::now();
}
"#;
    let findings = std_instant([("src/worker.rs", source)]);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].line, 5);
}

#[test]
fn test_wgsl_exact_f32_max_boundary() {
    // 3.4028234663852886e38 is mathematical f32::MAX bit for bit
    let exact_valid = r#"
const EXACT_MAX: f32 = 3.4028234663852886e38;
const NEG_EXACT_MAX: f32 = -3.4028234663852886e38;
"#;
    assert!(wgsl_float_literals(exact_valid).is_empty());

    // 3.40282347e38 is slightly above f32::MAX
    let just_over = r#"
const OVER_POS: f32 = 3.40282347e38;
const OVER_NEG: f32 = -3.40282347e38;
"#;
    let findings = wgsl_float_literals(just_over);
    assert_eq!(findings.len(), 2);
    assert_eq!(findings[0].text, "3.40282347e38");
    assert_eq!(findings[1].text, "3.40282347e38");
}

#[test]
fn character_literals_do_not_hide_the_code_after_them() {
    let source = r#"
fn menu<'a>(label: &'a str) {
    let quote = '"';
    let quit = PredefinedMenuItem::quit(None);
    let apostrophe = '\'';
    let t0 = std::time::Instant::now();
    let b = b'"';
    let later = PredefinedMenuItem::quit(Some(label));
}
"#;
    assert_eq!(lines(&predefined_quit([("src/menu.rs", source)])), vec![4, 8]);
    assert_eq!(lines(&std_instant([("src/menu.rs", source)])), vec![6]);
}

#[test]
fn strings_hold_comment_markers_without_hiding_code() {
    let source = r##"
fn menu() {
    let a = "/* not a comment";
    let quit = PredefinedMenuItem::quit(None);
    let b = b"// not a comment either";
    let c = r#"*/ " still a raw string"#;
    let t0 = std::time::Instant::now();
    let d = "PredefinedMenuItem::quit std::time::Instant";
}
"##;
    assert_eq!(lines(&predefined_quit([("src/menu.rs", source)])), vec![4]);
    assert_eq!(lines(&std_instant([("src/menu.rs", source)])), vec![7]);
}

#[test]
fn nested_block_comments_stay_masked() {
    let source = "/* outer /* inner */ PredefinedMenuItem::quit(None); std::time::Instant::now(); */\nlet q = PredefinedMenuItem::quit(None);\n";
    assert_eq!(lines(&predefined_quit([("src/menu.rs", source)])), vec![2]);
    assert!(std_instant([("src/menu.rs", source)]).is_empty());
}

#[test]
fn predefined_quit_sees_tabs_and_line_breaks_around_the_path_separator() {
    let source = "fn menu() {\n    let a = PredefinedMenuItem\t::\tquit(None);\n    let b = muda::PredefinedMenuItem\n        ::quit(None);\n}\n";
    let findings = predefined_quit([("src/menu.rs", source)]);
    assert_eq!(lines(&findings), vec![2, 3]);
    assert_eq!(findings[1].text, "let b = muda::PredefinedMenuItem");
}

#[test]
fn predefined_quit_matches_whole_names_only() {
    let other_names = r#"
fn menu() {
    let a = MyPredefinedMenuItem::quit(None);
    let b = PredefinedMenuItem::quitter();
    let c = PredefinedMenuItem::quit_label();
}
"#;
    assert!(predefined_quit([("src/menu.rs", other_names)]).is_empty());

    let aliased = r#"
use muda::{PredefinedMenuItem as Item, MenuItem};
fn menu() {
    let a = Item::quit(None);
    let b = other::Item::quit(None);
}
"#;
    assert_eq!(lines(&predefined_quit([("src/menu.rs", aliased)])), vec![4]);

    let other_crate = r#"
use other_menu::PredefinedMenuItem as Item;
fn menu() {
    let a = Item::quit(None);
}
"#;
    assert!(predefined_quit([("src/menu.rs", other_crate)]).is_empty());
}

#[test]
fn std_instant_ignores_an_alias_that_only_a_native_cfg_imports() {
    let source = r#"
#[cfg(not(target_arch = "wasm32"))]
use std::time as clock;
#[cfg(target_arch = "wasm32")]
use web_time as clock;

fn now() {
    let t = clock::Instant::now();
}
"#;
    assert!(std_instant([("src/clock.rs", source)]).is_empty());
}

#[test]
fn std_instant_reads_cfg_from_code_not_from_strings_or_comments() {
    let cfg_in_string = r##"
fn now() -> u64 { let _s = r#"cfg(target_os = "linux")"#;
    let t = std::time::Instant::now();
    0
}
"##;
    assert_eq!(lines(&std_instant([("src/a.rs", cfg_in_string)])), vec![3]);

    let cfg_in_comment = r#"
fn web() { // cfg(target_os = "linux")
    let t = std::time::Instant::now();
}
"#;
    assert_eq!(lines(&std_instant([("src/b.rs", cfg_in_comment)])), vec![3]);
}

#[test]
fn std_instant_skips_only_the_item_a_native_cfg_covers() {
    let source = r#"
#[cfg(not(target_arch = "wasm32"))]
fn native() -> std::time::Instant { std::time::Instant::now() }
fn web() -> std::time::Instant { std::time::Instant::now() }

#[cfg(all(unix, not(target_arch = "wasm32")))]
#[inline]
fn unix_only() {
    let t = std::time::Instant::now();
}

#[cfg(feature = "native-clock")]
fn by_feature() { let t = std::time::Instant::now(); }

#[cfg(any(target_arch = "wasm32", windows))]
fn also_web() { let t = std::time::Instant::now(); }

#[cfg(target_os = "unknown")]
fn unknown_os() { let t = std::time::Instant::now(); }

#[cfg(target_os = "macos")]
use std::time::Instant as MacInstant;
"#;
    assert_eq!(lines(&std_instant([("src/clock.rs", source)])), vec![4, 13, 16, 19]);
}

#[test]
fn std_instant_inner_cfg_covers_the_whole_file() {
    let source = r#"
#![cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

fn now() -> Instant {
    std::time::Instant::now()
}
"#;
    assert!(std_instant([("src/native.rs", source)]).is_empty());
}

#[test]
fn std_instant_reports_a_std_use_on_a_line_that_also_names_web_time() {
    let source = "fn pair() {\n    let p = (web_time::Instant::now(), std::time::Instant::now());\n}\n";
    assert_eq!(lines(&std_instant([("src/pair.rs", source)])), vec![2]);
}

#[test]
fn std_instant_needs_std_behind_a_bare_time_path() {
    let source = r#"
use crate::time;

fn tick() {
    let t = time::Instant::new();
}
"#;
    assert!(std_instant([("src/tick.rs", source)]).is_empty());
}

#[test]
fn std_instant_follows_module_and_type_aliases() {
    let source = r#"
use std::time as t;
use std::time::{self as clock, Duration};
use std::{time::Instant as I, fmt};
use web_time as wt;

fn run() {
    let a = t::Instant::now();
    let b = clock::Instant::now();
    let c = wt::Instant::now();
    let d = x::t::Instant::now();
}
"#;
    assert_eq!(lines(&std_instant([("src/run.rs", source)])), vec![4, 8, 9]);
}

fn finding_paths(findings: &[Finding]) -> Vec<&str> {
    findings.iter().map(|f| f.path.as_str()).collect()
}

#[test]
fn std_instant_skips_module_files_a_native_cfg_declares() {
    let lib = r#"
#[cfg(not(target_arch = "wasm32"))]
mod native;
mod web;
#[cfg(feature = "extra")]
mod extra;
"#;
    let native = "mod clock;\npub fn now() -> std::time::Instant { std::time::Instant::now() }\n";
    let native_clock = "use std::time::Instant;\n";
    let web = "#[cfg(unix)]\nmod imp;\npub fn now() { let _t = std::time::Instant::now(); }\n";
    let web_imp = "use std::time::Instant;\n";
    let sibling_imp = "use std::time::Instant;\n";
    let extra = "use std::time::Instant;\n";
    let files = [
        ("src/lib.rs", lib),
        ("src/native.rs", native),
        ("src/native/clock.rs", native_clock),
        ("src/web.rs", web),
        ("src/web/imp.rs", web_imp),
        ("src/imp.rs", sibling_imp),
        ("src/extra.rs", extra),
    ];
    assert_eq!(finding_paths(&std_instant(files)), vec!["src/web.rs", "src/imp.rs", "src/extra.rs"]);
}

#[test]
fn std_instant_skips_native_module_files_by_mod_rs_path_attribute_and_inline_module() {
    let main = r#"
#[cfg(unix)]
pub(crate) mod sys;
#[cfg(target_os = "macos")]
#[path = "platform/mac_clock.rs"]
mod mac;
#[cfg(windows)]
mod outer {
    mod inner;
}
#[cfg(unix)]
#[path = "platform/native.rs"]
pub(in crate) mod native;
"#;
    let clock = "use std::time::Instant;\n";
    let files = [
        ("C:\\app\\src\\main.rs", main),
        ("C:\\app\\src\\platform\\native.rs", clock),
        ("C:\\app\\src\\sys\\mod.rs", "mod detail;\n"),
        ("C:\\app\\src\\sys\\detail.rs", clock),
        ("C:\\app\\src\\platform\\mac_clock.rs", clock),
        ("C:\\app\\src\\outer\\inner.rs", clock),
        ("C:\\app\\src\\mac.rs", clock),
    ];
    assert_eq!(finding_paths(&std_instant(files)), vec!["C:\\app\\src\\mac.rs"]);
}

#[test]
fn std_instant_checks_a_module_file_that_a_web_declaration_also_names() {
    let lib = r#"
#[cfg(windows)]
mod imp;
#[cfg(target_arch = "wasm32")]
mod imp;
#[cfg(unix)]
mod sys;
"#;
    let files = [
        ("src/lib.rs", lib),
        ("src/imp.rs", "mod detail;\n"),
        ("src/imp/detail.rs", "use std::time::Instant;\n"),
        ("src/sys.rs", "#[path = \"imp/detail.rs\"]\nmod shared;\n"),
    ];
    assert_eq!(finding_paths(&std_instant(files)), vec!["src/imp/detail.rs"]);
}

#[test]
fn std_instant_reads_an_inline_module_path_attribute_as_the_folder_of_its_modules() {
    let lib = r#"
#[cfg(unix)]
#[path = "native"]
mod platform {
    mod clock;
}
mod web;
"#;
    let web = r#"
#[path = "shared"]
mod inner {
    #[cfg(windows)]
    mod tick;
}
"#;
    let clock = "use std::time::Instant;\n";
    let files = [
        ("src/lib.rs", lib),
        ("src/native/clock.rs", clock),
        ("src/platform/clock.rs", clock),
        ("src/web.rs", web),
        ("src/shared/tick.rs", clock),
        ("src/web/shared/tick.rs", clock),
    ];
    assert_eq!(finding_paths(&std_instant(files)), vec!["src/platform/clock.rs", "src/web/shared/tick.rs"]);
}

#[test]
fn std_instant_reads_top_level_path_attributes_of_a_non_mod_rs_file_from_its_folder() {
    let a = r#"
#[cfg(unix)]
#[path = "native.rs"]
mod imp;
#[cfg(unix)]
#[path = "native"]
mod platform {
    mod clock;
}
"#;
    let clock = "use std::time::Instant;\n";
    let files = [
        ("src/lib.rs", "mod a;\n"),
        ("src/a.rs", a),
        ("src/native.rs", clock),
        ("src/native/clock.rs", clock),
        ("src/a/native.rs", clock),
        ("src/a/native/clock.rs", clock),
    ];
    assert_eq!(finding_paths(&std_instant(files)), vec!["src/a/native.rs", "src/a/native/clock.rs"]);
}

#[test]
fn std_instant_checks_module_files_declared_deeper_than_its_nesting_limit() {
    let lib = format!("{}#[cfg(unix)]\nmod deep;\n{}", "mod m {\n".repeat(70), "}\n".repeat(70));
    let shallow_lib = format!("{}#[cfg(unix)]\nmod deep;\n{}", "mod m {\n".repeat(3), "}\n".repeat(3));
    let deep_path = format!("src/{}deep.rs", "m/".repeat(70));
    let shallow_path = format!("src/{}deep.rs", "m/".repeat(3));
    let clock = "use std::time::Instant;\n";
    let files = [("src/lib.rs", lib.as_str()), (deep_path.as_str(), clock)];
    assert_eq!(finding_paths(&std_instant(files)), vec![deep_path.as_str()]);
    let files = [("src/lib.rs", shallow_lib.as_str()), (shallow_path.as_str(), clock)];
    assert!(std_instant(files).is_empty());
}

#[test]
fn wgsl_integer_literals_are_not_floats() {
    let shader = r#"
const BIG_ABSTRACT = 400000000000000000000000000000000000000000;
const BIG_U = 1000000000000000000000000000000000000000u;
const BIG_I = 1000000000000000000000000000000000000000i;
const HEX_INT = 0xFFFFFFFFu;
const HEX_F_DIGIT = 0x1f;
const FLOAT_DOT: f32 = 340282356779733661637539395458142568448.0;
const FLOAT_SUFFIX: f32 = 400000000000000000000000000000000000000f;
const NEG_EXP: f32 = -4e38;
"#;
    let findings = wgsl_float_literals(shader);
    assert_eq!(lines(&findings), vec![7, 8, 9]);
    assert_eq!(findings[0].text, "340282356779733661637539395458142568448.0");
    assert_eq!(findings[1].text, "400000000000000000000000000000000000000");
    assert_eq!(findings[2].text, "4e38");
}

#[test]
fn wgsl_hex_floats_outside_the_range_are_reported() {
    let shader = r#"
const EXACT_MAX: f32 = 0x1.fffffep+127f;
const HUGE: f32 = 0x1p+500f;
const JUST_OVER: f32 = 0x1.000002p+128;
const UPPER: f32 = 0X1P128;
const TINY: f32 = 0x1p-500;
const ZERO: f32 = 0x0.0p99999;
const F16_MAX: f16 = 0x1.ffcp15h;
const F16_OVER: f16 = 0x1.0p16h;
"#;
    let findings = wgsl_float_literals(shader);
    assert_eq!(lines(&findings), vec![3, 4, 5, 9]);
    assert_eq!(findings[0].text, "0x1p+500");
    assert_eq!(findings[3].text, "0x1.0p16");
}

#[test]
fn wgsl_block_comment_markers_inside_line_comments_do_not_open_a_comment() {
    let shader = "// a /* inside a line comment\nconst BAD: f32 = 4e38;\n/* block // with a line marker */ const ALSO_BAD: f32 = 5e38;\n";
    let findings = wgsl_float_literals(shader);
    assert_eq!(lines(&findings), vec![2, 3]);
}

#[test]
fn wgsl_half_floats_use_the_f16_range() {
    let shader = "const OK: f16 = 65504.0h;\nconst OVER: f16 = 70000.0h;\nconst F32_OK: f32 = 70000.0f;\n";
    let findings = wgsl_float_literals(shader);
    assert_eq!(lines(&findings), vec![2]);
    assert_eq!(findings[0].text, "70000.0");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_rust_sources_filtering() {
    use craft_testkit::rust_sources;

    let dir = ScratchDir::new("filtering");
    dir.write("src/lib.rs", "pub fn lib() {}");
    dir.write("src/sub/mod.rs", "pub fn sub() {}");
    dir.write("src/tests.rs", "// should exclude");
    dir.write("src/tests_foo.rs", "// should exclude");
    dir.write("src/foo_tests.rs", "// should exclude");
    dir.write("src/foo_test.rs", "// should exclude");
    dir.write("tests/integ.rs", "// should exclude");
    dir.write("target/gen.rs", "// should exclude");
    dir.write(".github/action.rs", "// should exclude dot-dir");
    dir.write("src/README.md", "# Not a rust source");

    let sources = rust_sources(dir.path()).expect("rust_sources");
    assert_eq!(file_names(&sources), vec!["lib.rs", "mod.rs"]);

    // Test passing &[(String, String)] to guards
    let findings = predefined_quit(&sources);
    assert!(findings.is_empty());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn rust_sources_lists_a_root_inside_a_folder_named_tests() {
    let dir = ScratchDir::new("root_in_tests");
    dir.write("tests/app/src/lib.rs", "pub fn lib() {}");
    let sources = craft_testkit::rust_sources(dir.path().join("tests").join("app")).expect("rust_sources");
    assert_eq!(file_names(&sources), vec!["lib.rs"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn rust_sources_skips_build_output_folders_marked_with_cachedir_tag() {
    let dir = ScratchDir::new("cachedir");
    dir.write("src/lib.rs", "pub fn lib() {}");
    dir.write("out/CACHEDIR.TAG", "Signature: 8a477f597d28d172789f06886806bc55\n# build output\n");
    dir.write("out/debug/build/gen.rs", "pub fn generated() {}");
    dir.write("notes/CACHEDIR.TAG", "not a cache tag\n");
    dir.write("notes/kept.rs", "pub fn kept() {}");
    let sources = craft_testkit::rust_sources(dir.path()).expect("rust_sources");
    assert_eq!(file_names(&sources), vec!["kept.rs", "lib.rs"]);
}

#[cfg(unix)]
#[test]
fn rust_sources_does_not_follow_a_symlink_out_of_the_root() {
    let outside = ScratchDir::new("symlink_outside");
    outside.write("evil.rs", "pub fn evil() {}");
    let dir = ScratchDir::new("symlink_root");
    dir.write("src/lib.rs", "pub fn lib() {}");
    std::os::unix::fs::symlink(outside.path(), dir.path().join("src").join("linked")).expect("symlink folder");
    std::os::unix::fs::symlink(outside.path().join("evil.rs"), dir.path().join("src").join("evil.rs")).expect("symlink file");
    let sources = craft_testkit::rust_sources(dir.path()).expect("rust_sources");
    assert_eq!(file_names(&sources), vec!["lib.rs"]);
}

#[cfg(unix)]
#[test]
fn rust_sources_does_not_loop_on_a_symlink_cycle() {
    let dir = ScratchDir::new("symlink_cycle");
    dir.write("src/lib.rs", "pub fn lib() {}");
    std::os::unix::fs::symlink(dir.path().join("src"), dir.path().join("src").join("again")).expect("symlink cycle");
    let sources = craft_testkit::rust_sources(dir.path()).expect("rust_sources");
    assert_eq!(file_names(&sources), vec!["lib.rs"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn scratch_dirs_leave_existing_folders_alone_and_clean_up_on_panic() {
    let taken = std::env::temp_dir().join(format!("craft_testkit_sentinel_{}_0", std::process::id()));
    std::fs::create_dir(&taken).expect("create the folder the helper would pick first");
    std::fs::write(taken.join("sentinel"), "keep").expect("write sentinel");

    let made = std::panic::catch_unwind(|| {
        let dir = ScratchDir::new("sentinel");
        let path = dir.path().to_path_buf();
        assert!(path.exists());
        std::panic::panic_any(path);
    });
    let payload = made.expect_err("the closure panics");
    let path = payload.downcast_ref::<std::path::PathBuf>().expect("panic payload is the path").clone();

    assert_ne!(path, taken);
    assert!(!path.exists(), "the scratch folder is removed when the test panics");
    assert_eq!(std::fs::read_to_string(taken.join("sentinel")).expect("sentinel survives"), "keep");
    std::fs::remove_dir_all(&taken).expect("remove the pre-made folder");
}
