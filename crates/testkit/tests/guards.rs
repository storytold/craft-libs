use craft_testkit::{Finding, predefined_quit, std_instant, wgsl_float_literals, wgsl_float_literals_in};

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
fn test_rust_sources_filtering() {
    use craft_testkit::rust_sources;
    use std::fs::{self, File};
    use std::io::Write;

    let temp_dir = std::env::temp_dir().join(format!("craft_testkit_test_{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(temp_dir.join("src").join("sub")).expect("create temp sub");
    fs::create_dir_all(temp_dir.join("tests")).expect("create temp tests");
    fs::create_dir_all(temp_dir.join("target")).expect("create temp target");

    let create_file = |path: std::path::PathBuf, content: &str| {
        let mut f = File::create(path).expect("create file");
        f.write_all(content.as_bytes()).expect("write content");
    };

    create_file(temp_dir.join("src").join("lib.rs"), "pub fn lib() {}");
    create_file(temp_dir.join("src").join("sub").join("mod.rs"), "pub fn sub() {}");
    create_file(temp_dir.join("src").join("tests.rs"), "// should exclude");
    create_file(temp_dir.join("src").join("tests_foo.rs"), "// should exclude");
    create_file(temp_dir.join("src").join("foo_tests.rs"), "// should exclude");
    create_file(temp_dir.join("src").join("foo_test.rs"), "// should exclude");
    create_file(temp_dir.join("tests").join("integ.rs"), "// should exclude");
    create_file(temp_dir.join("target").join("gen.rs"), "// should exclude");
    create_file(temp_dir.join("src").join("README.md"), "# Not a rust source");

    let sources = rust_sources(temp_dir.join("src")).expect("rust_sources");
    let names: Vec<String> = sources.iter().map(|(p, _)| std::path::Path::new(p).file_name().unwrap().to_string_lossy().into_owned()).collect();

    assert_eq!(names, vec!["lib.rs", "mod.rs"]);

    // Test passing &[(&str, &str)] to guards
    let findings = predefined_quit(&sources);
    assert!(findings.is_empty());

    let _ = fs::remove_dir_all(&temp_dir);
}
