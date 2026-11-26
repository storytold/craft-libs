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
    let q3 = ::muda::PredefinedMenuItem::Quit;
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
    // 3.4028234663852886e38 rounds to f32::MAX, as an f32 and as an f64
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
fn predefined_quit_reports_a_qualified_path() {
    let source = r#"
use muda::PredefinedMenuItem as Item;
fn menu() {
    let a = <muda::PredefinedMenuItem>::quit(None);
    let b = < PredefinedMenuItem >::quit(None);
    let c = <Item>::quit(None);
    let d = Vec::<PredefinedMenuItem>::quit(None);
    let e = Wrapper<PredefinedMenuItem>::quit(None);
}
"#;
    assert_eq!(lines(&predefined_quit([("src/menu.rs", source)])), vec![4, 5, 6]);
}

#[test]
fn predefined_quit_ignores_another_crates_type_in_a_qualified_path() {
    let source = r#"
use other_menu::PredefinedMenuItem as Other;
fn menu() {
    let a = other_crate::PredefinedMenuItem::quit(None);
    let b = <other_crate::PredefinedMenuItem>::quit(None);
    let c = crate::menu::PredefinedMenuItem::Quit;
    let d = other::muda::PredefinedMenuItem::quit(None);
    let e = <T as Menu>::PredefinedMenuItem::quit(None);
    let f = Other::quit(None);
    let g = muda::PredefinedMenuItem::quit(None);
    let h = <muda :: PredefinedMenuItem>::quit(None);
    let i = PredefinedMenuItem::quit(None);
}
"#;
    assert_eq!(lines(&predefined_quit([("src/menu.rs", source)])), vec![10, 11, 12]);
}

#[test]
fn predefined_quit_ignores_a_muda_path_that_a_local_binding_of_muda_names() {
    let source = r#"
use other_menu as muda;
fn menu() {
    let a = muda::PredefinedMenuItem::quit(None);
    let b = <muda::PredefinedMenuItem>::quit(None);
    let c = ::muda::PredefinedMenuItem::quit(None);
}
mod inner {
    use ::muda;
    fn menu() { let d = muda::PredefinedMenuItem::quit(None); }
}
mod defined {
    mod muda {}
    fn menu() { let e = muda::PredefinedMenuItem::quit(None); }
}
mod gated {
    #[cfg(unix)]
    use other_menu as muda;
    fn menu() { let f = muda::PredefinedMenuItem::quit(None); }
}
"#;
    assert_eq!(lines(&predefined_quit([("src/menu.rs", source)])), vec![6, 10, 19]);
}

#[test]
fn predefined_quit_ignores_a_rename_through_a_local_binding_of_muda() {
    let source = r#"
use other_menu as muda;
use muda::PredefinedMenuItem as Item;
type Alias = muda::PredefinedMenuItem;
fn menu() {
    let a = Item::quit(None);
    let b = Alias::quit(None);
}
mod absolute {
    use other_menu as muda;
    use ::muda::PredefinedMenuItem as Item;
    type Alias = ::muda::PredefinedMenuItem;
    use ::{muda::PredefinedMenuItem as Grouped};
    fn menu() { let c = Item::quit(None); let d = Alias::quit(None); let e = Grouped::quit(None); }
}
mod crate_root {
    use muda::PredefinedMenuItem as Item;
    type Alias = muda::PredefinedMenuItem;
    fn menu() { let f = Item::quit(None); let g = Alias::quit(None); }
}
"#;
    assert_eq!(lines(&predefined_quit([("src/menu.rs", source)])), vec![14, 19]);
}

#[test]
fn predefined_quit_follows_a_name_that_the_muda_crate_is_imported_under() {
    let source = r#"
use muda as menu;
fn menu_bar() {
    let a = menu::PredefinedMenuItem::quit(None);
    let b = <menu::PredefinedMenuItem>::quit(None);
    let c = ::menu::PredefinedMenuItem::quit(None);
}
mod grouped {
    use muda::{self as m};
    fn menu_bar() { let d = m::PredefinedMenuItem::quit(None); }
}
mod renamed {
    extern crate muda as native;
    type Item = native::PredefinedMenuItem;
    fn menu_bar() { let e = native::PredefinedMenuItem::quit(None); let f = Item::quit(None); }
}
mod imported {
    use muda::{self as m};
    use m::PredefinedMenuItem as Quit;
    fn menu_bar() { let g = Quit::quit(None); }
}
mod unbound {
    fn menu_bar() { let h = m::PredefinedMenuItem::quit(None); }
}
mod shadowed {
    use other_menu as menu;
    fn menu_bar() { let i = menu::PredefinedMenuItem::quit(None); }
}
"#;
    assert_eq!(lines(&predefined_quit([("src/menu.rs", source)])), vec![4, 5, 10, 15, 20]);
}

#[test]
fn predefined_quit_reads_an_absolute_path_through_a_root_extern_crate_alias() {
    let source = r#"
extern crate muda as menu;
use ::menu::PredefinedMenuItem as Item;
fn menu_bar() {
    let a = ::menu::PredefinedMenuItem::quit(None);
    let b = <::menu::PredefinedMenuItem>::quit(None);
    let c = Item::quit(None);
}
mod nested {
    extern crate muda as native;
    fn menu_bar() { let d = ::native::PredefinedMenuItem::quit(None); let e = ::menu::PredefinedMenuItem::quit(None); }
}
"#;
    assert_eq!(lines(&predefined_quit([("src/main.rs", source)])), vec![5, 6, 7, 11]);
    // In a module file the declaration binds `menu` in that module only; `::menu` is the crate root's.
    let module = "extern crate muda as menu;\nfn menu_bar() {\n    let a = ::menu::PredefinedMenuItem::quit(None);\n    let b = menu::PredefinedMenuItem::quit(None);\n}\n";
    assert_eq!(lines(&predefined_quit([("src/child.rs", module)])), vec![4]);
    assert_eq!(lines(&predefined_quit([("src/bin/tool.rs", module)])), vec![3, 4]);
    // Cargo's auto-discovery builds a `mod.rs` directly in `src/bin` or `tests` as a target, but not one below.
    assert_eq!(lines(&predefined_quit([("src/bin/mod.rs", module)])), vec![3, 4]);
    assert_eq!(lines(&predefined_quit([("tests/mod.rs", module)])), vec![3, 4]);
    assert_eq!(lines(&predefined_quit([("src/bin/tool/mod.rs", module)])), vec![4]);
    // A `lib.rs` that another file loads through a `#[path]` attribute is a module.
    let root = "#[path = \"nested/lib.rs\"]\nmod nested;\n";
    assert_eq!(lines(&predefined_quit([("src/lib.rs", root), ("src/nested/lib.rs", module)])), vec![4]);
    // The root's alias reaches the module files it declares.
    let root = "extern crate muda as menu;\nmod child;\n";
    let child = "fn menu_bar() {\n    let a = ::menu::PredefinedMenuItem::quit(None);\n}\nuse ::menu::PredefinedMenuItem as Item;\nfn b() { Item::quit(None); }\n\
                 fn c() { menu::PredefinedMenuItem::quit(None); }\nuse menu::PredefinedMenuItem as Other;\nfn d() { Other::quit(None); }\n\
                 mod shadowed {\n    use other_menu as menu;\n    fn e() { menu::PredefinedMenuItem::quit(None); }\n}\n";
    let found = predefined_quit([("src/lib.rs", root), ("src/child.rs", child)]);
    assert_eq!(
        found.iter().map(|f| (f.path.as_str(), f.line)).collect::<Vec<_>>(),
        vec![("src/child.rs", 2), ("src/child.rs", 5), ("src/child.rs", 6), ("src/child.rs", 8)]
    );
    // Without the root that declares the alias, `menu` is not muda.
    assert!(predefined_quit([("src/child.rs", child)]).is_empty());
    // The alias follows the library's modules, not the binary's module of the same name, and `src/lib.rs`
    // stays a root when another file also declares it as a module.
    let call = "fn f() { ::menu::PredefinedMenuItem::quit(None); }\n";
    let found = predefined_quit([
        ("src/lib.rs", "extern crate muda as menu;\nmod outer;\nfn g() { ::menu::PredefinedMenuItem::quit(None); }\n"),
        ("src/outer.rs", "mod child;\n"),
        ("src/outer/child.rs", call),
        ("src/main.rs", "mod child;\n#[path = \"lib.rs\"]\nmod lib;\n"),
        ("src/child.rs", call),
    ]);
    assert_eq!(found.iter().map(|f| (f.path.as_str(), f.line)).collect::<Vec<_>>(), vec![("src/lib.rs", 3), ("src/outer/child.rs", 1)]);
    // A module file in a folder named `tests` is not a test crate's root.
    let local = "extern crate muda as menu;\nfn f() { ::menu::PredefinedMenuItem::quit(None); }\n";
    assert!(predefined_quit([("src/tests/mod.rs", "mod child;\n"), ("src/tests/child.rs", local)]).is_empty());
    assert_eq!(lines(&predefined_quit([("tests/child.rs", local)])), vec![2]);
    // A test that another test also loads by path stays a root, and its alias reaches its modules, which
    // are in the root's own folder.
    let found = predefined_quit([
        ("tests/foo.rs", "extern crate muda as menu;\nmod part;\n"),
        ("tests/part.rs", call),
        ("tests/foo/part.rs", call),
        ("tests/bar.rs", "#[path = \"foo.rs\"]\nmod foo;\n"),
    ]);
    assert_eq!(found.iter().map(|f| (f.path.as_str(), f.line)).collect::<Vec<_>>(), vec![("tests/part.rs", 1)]);
    // A binary's root in `src/bin` declares its modules beside it, not in a folder named after it.
    let found = predefined_quit([
        ("src/bin/tool.rs", "extern crate muda as menu;\nmod child;\n"),
        ("src/bin/child/mod.rs", call),
        ("src/bin/tool/child.rs", call),
    ]);
    assert_eq!(found.iter().map(|f| (f.path.as_str(), f.line)).collect::<Vec<_>>(), vec![("src/bin/child/mod.rs", 1)]);
    // A file in `tests` or `src/bin` is its own crate's root also when another target declares it as a module.
    let found = predefined_quit([
        ("tests/foo.rs", "mod part;\n"),
        ("tests/part.rs", local),
        ("src/bin/tool.rs", "mod helper;\n"),
        ("src/bin/helper.rs", local),
    ]);
    assert_eq!(found.iter().map(|f| (f.path.as_str(), f.line)).collect::<Vec<_>>(), vec![("tests/part.rs", 2), ("src/bin/helper.rs", 2)]);
    // A package's `build.rs` and a binary's `main.rs` stay roots when a file loads them through `#[path]`.
    let found = predefined_quit([
        ("src/lib.rs", "#[path = \"../build.rs\"]\nmod build;\n#[path = \"bin/tool/main.rs\"]\nmod tool;\n"),
        ("build.rs", local),
        ("src/bin/tool/main.rs", local),
    ]);
    assert_eq!(found.iter().map(|f| (f.path.as_str(), f.line)).collect::<Vec<_>>(), vec![("build.rs", 2), ("src/bin/tool/main.rs", 2)]);
    // A folder named `src` above the package, as in a path `rust_sources` returns, does not change that.
    let found = predefined_quit([
        ("/home/dev/src/app/src/bin/tool.rs", "mod helper;\n"),
        ("/home/dev/src/app/src/bin/helper.rs", local),
        ("/home/dev/src/app/src/lib.rs", "#[path = \"../build.rs\"]\nmod build;\n"),
        ("/home/dev/src/app/build.rs", local),
    ]);
    assert_eq!(
        found.iter().map(|f| (f.path.as_str(), f.line)).collect::<Vec<_>>(),
        vec![("/home/dev/src/app/src/bin/helper.rs", 2), ("/home/dev/src/app/build.rs", 2)]
    );
    // A `tests` folder inside a test's module folder holds modules, so a declared file there is not a root.
    assert!(
        predefined_quit([
            ("tests/support/mod.rs", "mod tests;\n"),
            ("tests/support/tests/mod.rs", "mod helper;\n"),
            ("tests/support/tests/helper.rs", local)
        ])
        .is_empty()
    );
    // A child's own `menu` module, imported through `self::`, is not the crate.
    let found = predefined_quit([
        ("src/lib.rs", "extern crate muda as menu;\nmod child;\n"),
        ("src/child.rs", "mod menu {}\nuse self::menu as other;\nfn f() { other::PredefinedMenuItem::quit(None); }\n"),
    ]);
    assert!(found.is_empty());
}

#[test]
fn predefined_quit_skips_root_aliases_that_are_never_compiled_or_in_macro_input() {
    let call = "fn f() { ::menu::PredefinedMenuItem::quit(None); }\n";
    let paths = |found: &[Finding]| found.iter().map(|f| (f.path.clone(), f.line)).collect::<Vec<_>>();
    // A module declaration that is never compiled does not carry the alias to the file it names.
    for gate in ["#[cfg(any())]", "#[cfg(false)]", "#[cfg(not(all()))]"] {
        let lib = format!("extern crate muda as menu;\n{gate}\nmod shared;\n{gate}\n#[path = \"other.rs\"]\nmod other;\n");
        let main = "extern crate other_menu as menu;\nmod shared;\n";
        let found = predefined_quit([("src/lib.rs", lib.as_str()), ("src/main.rs", main), ("src/shared.rs", call), ("src/other.rs", call)]);
        assert!(found.is_empty(), "{gate}: {found:?}");
    }
    // One that a target may compile still does, plainly or through `#[path]`.
    let lib = "extern crate muda as menu;\n#[cfg(unix)]\nmod shared;\n#[cfg(feature = \"x\")]\n#[path = \"other.rs\"]\nmod other;\n";
    let found = predefined_quit([("src/lib.rs", lib), ("src/shared.rs", call), ("src/other.rs", call)]);
    assert_eq!(paths(&found), vec![("src/shared.rs".to_owned(), 1), ("src/other.rs".to_owned(), 1)]);
    // An `extern crate` that is never compiled adds no name to the prelude; one that is false only on some
    // targets does.
    let lib = "#[cfg(any())]\nextern crate muda as menu;\nextern crate other_menu as menu;\nmod child;\nfn g() { ::menu::PredefinedMenuItem::quit(None); }\n";
    assert!(predefined_quit([("src/lib.rs", lib), ("src/child.rs", call)]).is_empty());
    let lib = "#[cfg(target_os = \"macos\")]\nextern crate muda as menu;\nmod child;\n";
    assert_eq!(paths(&predefined_quit([("src/lib.rs", lib), ("src/child.rs", call)])), vec![("src/child.rs".to_owned(), 1)]);
    // Neither does an `extern crate` in a macro's input, while a real one beside it does.
    for input in ["ignore!(extern crate muda as menu;);", "ignore![extern crate muda as menu;];", "ignore! { extern crate muda as menu; }"] {
        let lib = format!("{input}\nmod child;\n");
        assert!(predefined_quit([("src/lib.rs", lib.as_str()), ("src/child.rs", call)]).is_empty(), "{input}");
        let lib = format!("{input}\nextern crate muda as native;\nmod child;\n");
        let child = "fn f() { ::native::PredefinedMenuItem::quit(None); ::menu::PredefinedMenuItem::quit(None); }\n";
        assert_eq!(paths(&predefined_quit([("src/lib.rs", lib.as_str()), ("src/child.rs", child)])), vec![("src/child.rs".to_owned(), 1)]);
    }
    // In a file, a never-compiled or macro-input `extern crate muda as menu;` does not bind `menu` to muda.
    let module = "mod a {\n    #[cfg(any())]\n    extern crate muda as menu;\n    fn f() { menu::PredefinedMenuItem::quit(None); }\n}\n\
                  mod b {\n    ignore!(extern crate muda as menu;);\n    fn f() { menu::PredefinedMenuItem::quit(None); }\n}\n";
    assert!(predefined_quit([("src/child.rs", module)]).is_empty());
}

#[test]
fn predefined_quit_reads_target_folders_below_the_package_folder_only() {
    let local = "extern crate muda as menu;\nfn f() { ::menu::PredefinedMenuItem::quit(None); }\n";
    let lib = "#[path = \"../build.rs\"]\nmod build;\n#[path = \"../tests/t.rs\"]\nmod t;\n";
    for dir in ["/work/tests/app", "tests/app", "C:/src/bin/app"] {
        let (lib_path, build, test) = (format!("{dir}/src/lib.rs"), format!("{dir}/build.rs"), format!("{dir}/tests/t.rs"));
        let found = predefined_quit([(lib_path.as_str(), lib), (build.as_str(), local), (test.as_str(), local)]);
        let found: Vec<(&str, usize)> = found.iter().map(|f| (f.path.as_str(), f.line)).collect();
        assert_eq!(found, vec![(build.as_str(), 2), (test.as_str(), 2)], "{dir}");
    }
    // Inside the package a `tests` folder in `src` still holds modules.
    let lib = "mod tests;\n";
    assert!(predefined_quit([("/work/tests/app/src/lib.rs", lib), ("/work/tests/app/src/tests/mod.rs", local)]).is_empty());
    // So do target folder names deeper in the package's modules.
    let found = predefined_quit([
        ("src/lib.rs", "mod foo;\nmod src;\n"),
        ("src/foo.rs", "mod tests;\nmod src;\n"),
        ("src/foo/tests.rs", "mod helper;\n"),
        ("src/foo/tests/helper.rs", local),
        ("src/foo/src.rs", "mod bin;\n"),
        ("src/foo/src/bin.rs", "mod helper;\n"),
        ("src/foo/src/bin/helper.rs", local),
        ("src/src.rs", "mod bin;\n"),
        ("src/src/bin.rs", "mod helper;\n"),
        ("src/src/bin/helper.rs", local),
    ]);
    assert!(found.is_empty(), "{found:?}");
    // A package with only binaries, beside the one with a library, keeps its binaries as roots.
    let found =
        predefined_quit([("outer/src/lib.rs", ""), ("outer/tools/src/bin/tool.rs", "mod helper;\n"), ("outer/tools/src/bin/helper.rs", local)]);
    assert_eq!(found.iter().map(|f| (f.path.as_str(), f.line)).collect::<Vec<_>>(), vec![("outer/tools/src/bin/helper.rs", 2)]);
    // A file in the package's modules that a `#[path]` attribute loads from a `tests` folder is a module too,
    // so its `extern crate` does not reach the crate's extern prelude.
    let lib = "extern crate other_menu as menu;\n#[path = \"foo/tests/helper.rs\"]\nmod helper;\n";
    let found = predefined_quit([("src/lib.rs", lib), ("src/foo/tests/helper.rs", local)]);
    assert!(found.is_empty(), "{found:?}");
    // So is a `src/lib.rs` below the package's `src` folder that a `#[path]` attribute loads.
    let lib = "extern crate other_menu as menu;\n#[path = \"nested/src/lib.rs\"]\nmod nested;\n";
    let found = predefined_quit([("src/lib.rs", lib), ("src/nested/src/lib.rs", local)]);
    assert!(found.is_empty(), "{found:?}");
    // A nested package's library that only its own binary loads stays a crate root, so its alias reaches its modules.
    let nested = "extern crate muda as menu;\nmod child;\n";
    let found = predefined_quit([
        ("src/lib.rs", ""),
        ("src/nested/src/lib.rs", nested),
        ("src/nested/src/child.rs", "fn f() { ::menu::PredefinedMenuItem::quit(None); }\n"),
        ("src/nested/src/bin/tool.rs", "#[path = \"../lib.rs\"]\nmod lib;\n"),
    ]);
    assert_eq!(found.iter().map(|f| (f.path.as_str(), f.line)).collect::<Vec<_>>(), vec![("src/nested/src/child.rs", 1)]);
    // One that a module beside its `src` folder loads is a module of the outer crate.
    let found = predefined_quit([
        ("src/lib.rs", "mod nested;\n"),
        ("src/nested/mod.rs", "#[path = \"src/lib.rs\"]\nmod inner;\n"),
        ("src/nested/src/lib.rs", nested),
        ("src/nested/src/child.rs", "fn f() { ::menu::PredefinedMenuItem::quit(None); }\n"),
    ]);
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn predefined_quit_does_not_follow_a_mod_declaration_in_a_macro_input() {
    let call = "fn f() { ::menu::PredefinedMenuItem::quit(None); }\n";
    for lib in [
        "extern crate muda as menu;\nignore!(mod child;);\n",
        "extern crate muda as menu;\nignore! {\n    mod child;\n}\n",
        "extern crate muda as menu;\n#[attr(mod child;)]\nfn h() {}\n",
    ] {
        assert!(predefined_quit([("src/lib.rs", lib), ("src/child.rs", call)]).is_empty(), "{lib}");
    }
    let lib = "extern crate muda as menu;\nmod child;\n";
    assert_eq!(lines(&predefined_quit([("src/lib.rs", lib), ("src/child.rs", call)])), vec![1]);
}

#[test]
fn predefined_quit_reads_an_extern_crate_in_a_block_inside_call_arguments() {
    let call = "fn f() {\n    consume({\n        extern crate muda as menu;\n        menu::PredefinedMenuItem::quit(None)\n    });\n}\n";
    assert_eq!(lines(&predefined_quit([("src/child.rs", call)])), vec![4]);
    let array = "fn f() {\n    let items = [{\n        extern crate muda as menu;\n        menu::PredefinedMenuItem::quit(None)\n    }];\n}\n";
    assert_eq!(lines(&predefined_quit([("src/child.rs", array)])), vec![4]);
    // In a crate root, a block in an argument binds the name in the block, not in the extern prelude.
    let lib = "fn f() {\n    consume({\n        extern crate muda as menu;\n    });\n}\nmod child;\n";
    let child = "fn g() { ::menu::PredefinedMenuItem::quit(None); }\n";
    assert!(predefined_quit([("src/lib.rs", lib), ("src/child.rs", child)]).is_empty());
    // A `macro_rules!` template is not a declaration where it is written, in any delimiter.
    for body in [
        "macro_rules! m {\n    () => (\n        extern crate muda as menu;\n        menu::PredefinedMenuItem::quit(None)\n    );\n}\n",
        "macro_rules! m {\n    () => [\n        extern crate muda as menu;\n        menu::PredefinedMenuItem::quit(None)\n    ];\n}\n",
        "macro_rules! m (\n    () => {\n        extern crate muda as menu;\n        menu::PredefinedMenuItem::quit(None)\n    };\n);\n",
    ] {
        assert!(predefined_quit([("src/child.rs", body)]).is_empty(), "{body}");
    }
    // Nor is an attribute's input, in a crate root or a module.
    for attr in ["#[attr(extern crate muda as menu;)]\nfn h() {}\n", "#![attr(extern crate muda as menu;)]\n"] {
        let lib = format!("{attr}mod child;\n");
        assert!(predefined_quit([("src/lib.rs", lib.as_str()), ("src/child.rs", child)]).is_empty(), "{attr}");
        let module = format!("{attr}fn g() {{ menu::PredefinedMenuItem::quit(None); }}\n");
        assert!(predefined_quit([("src/child.rs", module.as_str())]).is_empty(), "{attr}");
    }
}

#[test]
fn predefined_quit_reads_a_crate_name_in_an_import_where_the_import_is() {
    let source = r#"
mod native {
    use muda as menu;
    use menu::PredefinedMenuItem as Item;
    type Alias = menu::PredefinedMenuItem;
    fn menu_bar() { let a = Item::quit(None); let b = Alias::quit(None); }
}
mod other {
    use other_menu as menu;
    use menu::PredefinedMenuItem as Item;
    type Alias = menu::PredefinedMenuItem;
    fn menu_bar() { let c = Item::quit(None); let d = Alias::quit(None); }
}
mod external {
    use muda as menu;
    use ::menu::PredefinedMenuItem as Item;
    type Alias = ::menu::PredefinedMenuItem;
    fn menu_bar() { let e = Item::quit(None); let f = Alias::quit(None); }
}
mod grouped {
    use muda as menu;
    use {::menu::PredefinedMenuItem as Item};
    fn menu_bar() { let g = Item::quit(None); }
}
mod shadowed {
    use muda as menu;
    fn menu_bar() {
        extern crate other_menu as menu;
        let h = menu::PredefinedMenuItem::quit(None);
    }
}
mod renamed_module {
    use other_menu as muda;
    use muda as menu;
    fn menu_bar() { let i = menu::PredefinedMenuItem::quit(None); }
}
mod renamed_crate {
    mod muda {}
    use ::muda as menu;
    fn menu_bar() { let j = menu::PredefinedMenuItem::quit(None); }
}
"#;
    assert_eq!(lines(&predefined_quit([("src/menu.rs", source)])), vec![6, 40]);
}

#[test]
fn predefined_quit_ignores_a_bare_name_that_another_import_or_a_local_type_binds_in_its_scope() {
    let imported = r#"
use other_menu::PredefinedMenuItem;
fn menu() {
    let a = PredefinedMenuItem::quit(None);
    let b = muda::PredefinedMenuItem::quit(None);
}
"#;
    let grouped = "use other_menu::{PredefinedMenuItem, Menu};\nfn menu() { let a = PredefinedMenuItem::quit(None); }\n";
    let local = r#"
struct PredefinedMenuItem;
fn menu() {
    let a = PredefinedMenuItem::quit(None);
    let b = <PredefinedMenuItem>::quit(None);
}
"#;
    let both = r#"
use muda::PredefinedMenuItem;
mod other { pub struct PredefinedMenuItem; }
fn menu() {
    let a = PredefinedMenuItem::quit(None);
}
"#;
    let scoped = r#"
use muda::*;
mod other {
    use other_menu::PredefinedMenuItem;
    fn menu() { let a = PredefinedMenuItem::quit(None); }
}
mod local { struct PredefinedMenuItem; fn menu() { let b = PredefinedMenuItem::quit(None); } }
fn menu() {
    let c = PredefinedMenuItem::quit(None);
    { use other_menu::PredefinedMenuItem; let d = PredefinedMenuItem::quit(None); }
    let e = PredefinedMenuItem::quit(None);
}
"#;
    assert_eq!(lines(&predefined_quit([("src/imported.rs", imported)])), vec![5]);
    assert!(predefined_quit([("src/grouped.rs", grouped)]).is_empty());
    assert!(predefined_quit([("src/local.rs", local)]).is_empty());
    assert_eq!(lines(&predefined_quit([("src/both.rs", both)])), vec![5]);
    assert_eq!(lines(&predefined_quit([("src/scoped.rs", scoped)])), vec![9, 11]);
    let child = r#"
use other_menu::PredefinedMenuItem;
mod child {
    use muda::*;
    fn menu() { let a = PredefinedMenuItem::quit(None); }
}
fn menu() { let b = PredefinedMenuItem::quit(None); }
"#;
    assert_eq!(lines(&predefined_quit([("src/child.rs", child)])), vec![5]);
    let renamed = r#"
mod a {
    use muda::PredefinedMenuItem as Item;
    fn menu() { let a = Item::quit(None); }
}
mod b {
    struct Item;
    fn menu() { let b = Item::quit(None); }
}
mod c { fn menu() { let c = Item::quit(None); } }
mod d { type Quit = muda::PredefinedMenuItem; fn menu() { let d = Quit::quit(None); } }
"#;
    assert_eq!(lines(&predefined_quit([("src/renamed.rs", renamed)])), vec![4, 11]);
    let aliased_or_gated = r#"
mod alias {
    type PredefinedMenuItem = ::muda::PredefinedMenuItem;
    fn menu() { let a = PredefinedMenuItem::quit(None); }
}
mod gated {
    use muda::*;
    #[cfg(windows)]
    struct PredefinedMenuItem;
    #[cfg(windows)] use other_menu::PredefinedMenuItem;
    fn menu() { let b = PredefinedMenuItem::quit(None); }
}
mod other { type PredefinedMenuItem = other_menu::PredefinedMenuItem; fn menu() { let c = PredefinedMenuItem::quit(None); } }
mod assoc {
    use muda::PredefinedMenuItem;
    trait Menu { type PredefinedMenuItem; fn menu() { let d = PredefinedMenuItem::quit(None); } }
    #[allow(dead_code)] unsafe impl<const N: usize> Menu for Wrap<{ N }> {
        fn menu() { let e = PredefinedMenuItem::quit(None); }
        type PredefinedMenuItem = u8;
    }
}
mod reexported {
    use crate::PredefinedMenuItem;
    fn menu() { let f = PredefinedMenuItem::quit(None); }
    mod inner { use super::super::{self as up, PredefinedMenuItem}; fn menu() { let g = PredefinedMenuItem::quit(None); } }
    mod aliased { type PredefinedMenuItem = crate::menu::PredefinedMenuItem; fn menu() { let h = PredefinedMenuItem::quit(None); } }
}
"#;
    assert_eq!(lines(&predefined_quit([("src/aliased.rs", aliased_or_gated)])), vec![4, 11, 16, 18, 24, 25, 26]);
}

#[test]
fn std_instant_reads_a_std_time_alias_only_in_its_scope() {
    let source = r#"
mod native {
    use std::time as clock;
    fn now() { let a = clock::Instant::now(); }
    use clock::Instant as A;
}
mod web {
    use web_time as clock;
    fn now() { let b = clock::Instant::now(); }
    use clock::Instant as B;
    use {clock::*, std::time::*};
}
mod child { fn now() { let c = clock::Instant::now(); } }
fn run() {
    use std::time as t;
    let d = t::Instant::now();
    { mod t {} let e = t::Instant::now(); }
}
"#;
    assert_eq!(lines(&std_instant([("src/lib.rs", source)])), vec![4, 5, 11, 16]);
}

#[test]
fn std_instant_keeps_an_alias_that_a_cfg_gated_import_may_shadow() {
    let source = r#"
use std::time as clock;
fn now() {
    #[cfg(feature = "web-clock")]
    use web_time as clock;
    let a = clock::Instant::now();
}
fn gated() {
    #[cfg(feature = "web-clock")]
    mod clock {}
    let b = clock::Instant::now();
}
#[cfg(feature = "web-clock")]
fn web() {
    use web_time as clock;
    let c = clock::Instant::now();
}
fn wasm() {
    #[cfg(target_arch = "wasm32")]
    use web_time as clock;
    let d = clock::Instant::now();
}
"#;
    assert_eq!(lines(&std_instant([("src/lib.rs", source)])), vec![6, 11]);
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
fn std_instant_decodes_escapes_in_cfg_values() {
    let source = r#"
#[cfg(target_arch = "\x77asm32")]
fn web() { let t = std::time::Instant::now(); }
#[cfg(target_arch = "w\u{61}sm32")]
fn web_two() { let t = std::time::Instant::now(); }
#[cfg(target_arch = r"\x77asm32")]
fn native() { let t = std::time::Instant::now(); }
#[cfg(not(target_arch = "\q"))]
fn unknown() { let t = std::time::Instant::now(); }
#[cfg(target_os = "lin\
    ux")]
fn native_two() { let t = std::time::Instant::now(); }
fn after() { let t = std::time::Instant::now(); }
"#;
    assert_eq!(lines(&std_instant([("src/clock.rs", source)])), vec![3, 5, 9, 13]);
}

#[test]
fn std_instant_reads_cfg_true_and_false() {
    let source = r#"
#[cfg(false)]
fn disabled() { let t = std::time::Instant::now(); }
#[cfg(true)]
fn enabled() { let t = std::time::Instant::now(); }
#[cfg(not(true))]
fn never() { let t = std::time::Instant::now(); }
#[cfg(r#false)]
fn named_option() { let t = std::time::Instant::now(); }
#[cfg(not(r#true))]
fn other_option() { let t = std::time::Instant::now(); }
"#;
    assert_eq!(lines(&std_instant([("src/clock.rs", source)])), vec![5, 9, 11]);
}

#[test]
fn std_instant_checks_an_item_whose_cfg_is_nested_past_the_limit() {
    let source = format!(
        "#[cfg({}target_arch = \"wasm32\"{})]\nfn web() {{ let t = std::time::Instant::now(); }}\n#[cfg({}unix{})]\nfn native() {{ let t = std::time::Instant::now(); }}\n",
        "any(".repeat(65),
        ")".repeat(65),
        "all(".repeat(10),
        ")".repeat(10),
    );
    assert_eq!(lines(&std_instant([("src/clock.rs", source.as_str())])), vec![2]);
}

#[test]
fn std_instant_checks_the_fields_after_a_native_cfg_function_pointer_field() {
    let source = r#"
struct S {
    #[cfg(unix)]
    callback: fn(),
    clock: std::time::Instant,
    #[cfg(unix)]
    native: fn() -> std::time::Instant,
    #[cfg(unix)]
    pub(crate) boxed: Box<dyn Fn()>,
    other: std::time::Instant,
}
fn run() {
    #[cfg(unix)]
    'outer: loop { let _a = std::time::Instant::now(); }
    let t = std::time::Instant::now();
}
"#;
    assert_eq!(lines(&std_instant([("src/clock.rs", source)])), vec![5, 10, 15]);
}

#[test]
fn std_instant_ends_a_native_cfg_block_statement_at_its_closing_brace() {
    let source = r#"
fn run() {
    #[cfg(unix)] {
        let _a = std::time::Instant::now();
    }
    let t = std::time::Instant::now();
}
"#;
    assert_eq!(lines(&std_instant([("src/run.rs", source)])), vec![6]);
}

#[test]
fn std_instant_skips_a_method_call_after_a_native_cfg_block() {
    let source = r#"
fn run(x: Option<u64>) -> Option<u64> {
    #[cfg(unix)]
    { 0u64 }.saturating_add(std::time::Instant::now().elapsed().as_nanos() as u64);
    #[cfg(unix)]
    unsafe { x }.map(|_| std::time::Instant::now())?;
    let t = std::time::Instant::now();
    x
}
"#;
    assert_eq!(lines(&std_instant([("src/run.rs", source)])), vec![7]);
}

#[test]
fn std_instant_skips_an_operator_after_a_block_inside_a_native_cfg_expression() {
    let source = r#"
fn run(flag: bool) -> u64 {
    #[cfg(unix)]
    let _ = if flag { 1 } else { 2 } + std::time::Instant::now().elapsed().as_secs();
    #[cfg(unix)]
    return if flag { 1 } else { 2 } * std::time::Instant::now().elapsed().as_secs();
    let t = std::time::Instant::now();
    0
}
"#;
    assert_eq!(lines(&std_instant([("src/run.rs", source)])), vec![7]);
}

#[test]
fn std_instant_skips_a_cast_after_a_match_inside_a_native_cfg_expression() {
    let source = r#"
fn run(x: Option<u32>) -> u64 {
    #[cfg(unix)]
    let n = match x { Some(v) => v, None => 0 } as u64 + std::time::Instant::now().elapsed().as_secs();
    let t = std::time::Instant::now();
    0
}
"#;
    assert_eq!(lines(&std_instant([("src/run.rs", source)])), vec![5]);
}

#[test]
fn std_instant_runs_a_native_cfg_match_arm_to_its_comma_when_a_block_is_inside_its_value() {
    let source = r#"
fn run(x: Option<u32>, y: bool) -> u64 {
    match x {
        #[cfg(unix)]
        Some(v) => v as u64 + match y { true => 1, false => 0 } + std::time::Instant::now().elapsed().as_secs(),
        #[cfg(unix)]
        None => if y { 1 } else { 0 },
        #[cfg(unix)] Some(0) => { 0 } _ => std::time::Instant::now().elapsed().as_secs(),
    }
}
"#;
    assert_eq!(lines(&std_instant([("src/run.rs", source)])), vec![8]);
}

#[test]
fn std_instant_ends_a_native_cfg_const_or_labeled_block_at_its_closing_brace() {
    let source = r#"
fn run(x: Option<u32>) -> u64 {
    #[cfg(unix)]
    const { assert!(true) }
    let a = std::time::Instant::now();
    #[cfg(unix)]
    'done: { let _ = std::time::Instant::now(); break 'done; }
    let b = std::time::Instant::now();
    match x {
        #[cfg(unix)]
        Some(_) => 'arm: { let _ = std::time::Instant::now(); break 'arm; }
        _ => { let c = std::time::Instant::now(); }
    }
    0
}
"#;
    assert_eq!(lines(&std_instant([("src/run.rs", source)])), vec![5, 8, 12]);
}

#[test]
fn std_instant_ends_a_native_cfg_if_let_at_its_body_not_at_a_struct_pattern() {
    let source = r#"
enum State { Ready { at: u64 }, Idle }
fn run(state: State, other: State) {
    #[cfg(unix)]
    if let State::Ready { .. } = state { let _a = std::time::Instant::now(); }
    let b = std::time::Instant::now();
    #[cfg(unix)]
    while let State::Ready { at } = other { let _c = std::time::Instant::now(); }
    let d = std::time::Instant::now();
    #[cfg(unix)]
    if let State::Idle = state { 0 } else if let State::Ready { at: 0..=9 } = state { let _e = std::time::Instant::now(); } else { let _f = std::time::Instant::now(); }
    let g = std::time::Instant::now();
    #[cfg(unix)]
    if let State::Ready { .. } = state && let State::Ready { at } = other { let _h = std::time::Instant::now(); }
    let i = std::time::Instant::now();
    #[cfg(unix)]
    for State::Ready { at } in [state] { let _j = std::time::Instant::now(); }
    let k = std::time::Instant::now();
}
"#;
    assert_eq!(lines(&std_instant([("src/run.rs", source)])), vec![6, 9, 12, 15, 18]);
}

#[test]
fn std_instant_ends_a_native_cfg_if_at_its_body_not_at_a_block_in_its_condition() {
    let source = r#"
fn run(ready: bool, items: Vec<u64>) {
    #[cfg(unix)]
    if { ready } { let _a = std::time::Instant::now(); }
    let b = std::time::Instant::now();
    #[cfg(unix)]
    while !{ ready } { let _c = std::time::Instant::now(); }
    let d = std::time::Instant::now();
    #[cfg(unix)]
    if ready && unsafe { check() } { let _e = std::time::Instant::now(); }
    let f = std::time::Instant::now();
    #[cfg(unix)]
    for item in { items } { let _g = std::time::Instant::now(); }
    let h = std::time::Instant::now();
    #[cfg(unix)]
    match { 1 } { _ => { let _i = std::time::Instant::now(); } }
    let j = std::time::Instant::now();
    #[cfg(unix)]
    if let true = { ready } { let _k = std::time::Instant::now(); }
    let l = std::time::Instant::now();
    #[cfg(unix)]
    if loop { break ready } { let _m = std::time::Instant::now(); }
    let n = std::time::Instant::now();
    #[cfg(unix)]
    if async move { ready }.await { let _o = std::time::Instant::now(); }
    let p = std::time::Instant::now();
}
"#;
    assert_eq!(lines(&std_instant([("src/run.rs", source)])), vec![5, 8, 11, 14, 17, 20, 23, 26]);
}

#[test]
fn std_instant_ends_a_native_cfg_loop_at_its_body_after_an_if_or_match_in_its_head() {
    let source = r#"
fn run(flag: bool, n: u8) {
    #[cfg(unix)]
    for _ in if flag { [0] } else { [1] } {
        let _a = std::time::Instant::now();
    }
    let b = std::time::Instant::now();
    #[cfg(unix)]
    for _ in if flag { [0] } else if n > 1 { [1] } else { [2] }.iter() { let _c = std::time::Instant::now(); }
    let d = std::time::Instant::now();
    #[cfg(unix)]
    if let Some(_) = match n { 0 => None, _ => Some(n) } { let _e = std::time::Instant::now(); }
    let f = std::time::Instant::now();
    #[cfg(unix)]
    while n == if if flag { true } else { false } { 1 } else { 2 } { let _g = std::time::Instant::now(); }
    let h = std::time::Instant::now();
    #[cfg(unix)]
    if flag { let _i = std::time::Instant::now(); } else if if flag { true } else { false } { let _j = std::time::Instant::now(); }
    let k = std::time::Instant::now();
}
"#;
    assert_eq!(lines(&std_instant([("src/run.rs", source)])), vec![7, 10, 13, 16, 19]);
}

#[test]
fn std_instant_ends_a_native_cfg_let_else_and_match_with_struct_patterns_where_they_end() {
    let source = r#"
enum State { Ready { at: u64 }, Idle }
fn run(state: State, other: State) -> u64 {
    #[cfg(unix)]
    let State::Ready { at } = state else { let _a = std::time::Instant::now(); return 0; };
    let b = std::time::Instant::now();
    #[cfg(unix)]
    match other { State::Ready { at } => { let _c = std::time::Instant::now(); } State::Idle => {} }
    let d = std::time::Instant::now();
    match state {
        #[cfg(unix)]
        State::Ready { at } => if let State::Ready { .. } = other { let _e = std::time::Instant::now(); },
        _ => { let f = std::time::Instant::now(); }
    }
    0
}
"#;
    assert_eq!(lines(&std_instant([("src/run.rs", source)])), vec![6, 9, 13]);
}

#[test]
fn std_instant_reads_raw_identifiers_as_names_not_keywords() {
    let source = r#"
fn run(r#let: bool, r#in: [u8; 1]) {
    #[cfg(unix)]
    if r#let { let _a = std::time::Instant::now(); }
    let b = std::time::Instant::now();
    #[cfg(unix)]
    for x in r#in { let _c = std::time::Instant::now(); }
    let d = std::time::Instant::now();
}
"#;
    assert_eq!(lines(&std_instant([("src/run.rs", source)])), vec![5, 8]);

    let lib = "#[cfg(unix)]\nmod r#match;\nmod r#loop;\n";
    let native = "use std::time::Instant;\n";
    let web = "use std::time::Instant;\n";
    assert_eq!(finding_paths(&std_instant([("src/lib.rs", lib), ("src/match.rs", native), ("src/loop.rs", web)])), vec!["src/loop.rs"]);
}

#[test]
fn std_instant_runs_a_native_cfg_async_block_statement_to_its_semicolon() {
    let source = r#"
async fn run() {
    #[cfg(unix)]
    async { std::time::Instant::now() }
        .await;
    #[cfg(unix)]
    async move { std::time::Instant::now() };
    let a = std::time::Instant::now();
}
"#;
    assert_eq!(lines(&std_instant([("src/run.rs", source)])), vec![8]);
}

#[test]
fn std_instant_checks_code_before_and_after_a_native_cfg_item_on_the_same_line() {
    let source = "fn run() {
    let a = std::time::Instant::now(); #[cfg(unix)] let b = std::time::Instant::now(); let c = std::time::Instant::now();
    let d = 1; #[cfg(unix)] let e = std::time::Instant::now();
    #[cfg(unix)] let f = std::time::Instant::now(); let g = std::time::Instant::now();
}
";
    assert_eq!(lines(&std_instant([("src/run.rs", source)])), vec![2, 4]);
}

#[test]
fn std_instant_checks_an_item_after_a_native_cfg_item_on_the_same_line() {
    let source = r#"#[cfg(target_os="linux")] fn native() {} fn web() { std::time::Instant::now(); }
"#;
    assert_eq!(lines(&std_instant([("src/clock.rs", source)])), vec![1]);
}

#[test]
fn std_instant_skips_gated_code_that_follows_a_native_cfg_item_on_the_same_line() {
    let source = r#"#[cfg(target_os = "linux")] fn a() {} #[cfg(target_os = "macos")] fn b() { std::time::Instant::now(); }
#[cfg(unix)] fn c() {} #[cfg(windows)] use std::time::Instant as Native;
"#;
    assert!(std_instant([("src/clock.rs", source)]).is_empty());
}

#[test]
fn std_instant_ends_native_cfg_impl_mod_and_struct_items_where_they_end_on_the_line() {
    let source = r#"struct S; #[cfg(unix)] impl S { fn now() { std::time::Instant::now(); } } fn web() { std::time::Instant::now(); }
#[cfg(unix)] mod native { fn now() { std::time::Instant::now(); } } fn web_two() { std::time::Instant::now(); }
#[cfg(unix)] struct P { at: std::time::Instant } struct Q { at: std::time::Instant }
#[cfg(unix)] struct T(std::time::Instant); struct U(std::time::Instant);
#[cfg(unix)] impl S { fn a() {} } #[cfg(unix)] mod m { fn b() { std::time::Instant::now(); } }
"#;
    assert_eq!(lines(&std_instant([("src/clock.rs", source)])), vec![1, 2, 3, 4]);
}

#[test]
fn std_instant_reads_blocks_in_generic_brackets_as_part_of_a_native_cfg_item_head() {
    let source = r#"#[cfg(unix)] struct Native<const N: usize = { 1 + 2 }> { clock: std::time::Instant }
#[cfg(unix)] enum E<const N: usize = { 3 }> { A(std::time::Instant) }
#[cfg(unix)] trait Tr<const N: usize = { 1 }>: Sized where Self: Into<W<{ 2 }>> { fn now() -> std::time::Instant { std::time::Instant::now() } }
#[cfg(unix)] impl<const N: usize> Tr<{ N }> for Native<{ N }> { fn now() -> std::time::Instant { std::time::Instant::now() } }
#[cfg(unix)] fn native<T>() -> W<{ 1 + 1 }> where T: Into<W<{ 2 }>> { std::time::Instant::now(); W }
#[cfg(unix)] fn arrow<F: Fn() -> u8>(f: F) -> impl Fn() -> W<{ 3 }> where F: Copy { let _t = std::time::Instant::now(); f }
struct Web<const N: usize = { 1 }> { clock: std::time::Instant }
fn web() -> W<{ 1 }> where u8: Into<W<{ 2 }>> { std::time::Instant::now(); W }
#[cfg(unix)] struct A<const N: usize = { 1 }> { a: std::time::Instant } fn after() { std::time::Instant::now(); }
"#;
    assert_eq!(lines(&std_instant([("src/clock.rs", source)])), vec![7, 8, 9]);
}

#[test]
fn std_instant_reads_paths_split_over_lines_and_comments_with_crlf() {
    let source = "use std::time::\r\n    Instant;\r\nfn run() {\r\n    let t = std::time\r\n        ::Instant::now();\r\n    let u = std /* a */ :: time // b\r\n        :: Instant::now();\r\n}\r\n";
    assert_eq!(lines(&std_instant([("src/run.rs", source)])), vec![1, 4, 6]);
}

#[test]
fn std_instant_reports_a_use_on_the_line_where_its_path_starts() {
    let source = "use std::time::\n    Instant as Clock;\nuse std::time::{\n    Duration,\n    Instant,\n};\nuse std::\n    time::*;\n";
    assert_eq!(lines(&std_instant([("src/run.rs", source)])), vec![1, 5, 7]);
}

#[test]
fn std_instant_ignores_other_time_modules_longer_names_and_a_renamed_duration() {
    let source = r#"
use tokio::time::Instant;
use std::time::{Duration as Instant};
fn run() {
    let a = my_time::Instant::now();
    let b = runtime::Instant::now();
    let c = crate::time::Instant::now();
    let d = tokio::time::Instant::now();
    let e = core::time::Instant::now();
    let f = std::time::InstantLike::now();
    let g = mystd::time::Instant::now();
    let h = std::time::Instant::now();
}
"#;
    assert_eq!(lines(&std_instant([("src/run.rs", source)])), vec![12]);
}

#[test]
fn std_instant_ignores_paths_in_literals_and_comments_and_reads_labels_as_code() {
    let source = r##"
fn run<'a>(s: &'a str) -> char {
    let _x = "std::time::Instant"; let _r = r#"std::time::Instant"#; let _b = b"std::time::Instant"; let _c = c"std::time::Instant";
    // std::time::Instant
    /* std::time::Instant */
    'outer: loop { break 'outer; }
    let q = '"'; let t = std::time::Instant::now();
    'x'
}
"##;
    assert_eq!(lines(&std_instant([("src/run.rs", source)])), vec![7]);
}

#[test]
fn std_instant_scans_a_large_source_once() {
    let mut source = String::from("fn run() {\n");
    for i in 0..20_000 {
        if i % 2 == 0 {
            source.push_str("    #[cfg(unix)] let a = std::time::Instant::now();\n");
        } else {
            source.push_str("    let b = std::time::Instant::now();\n");
        }
    }
    source.push_str("}\n");
    let found = lines(&std_instant([("src/run.rs", source.as_str())]));
    assert_eq!(found.len(), 10_000);
    assert_eq!(found.first(), Some(&3));
    assert_eq!(found.last(), Some(&20_001));
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

#[test]
fn std_instant_does_not_read_an_absolute_path_through_a_local_alias() {
    let source = r#"
use std::time as web_time;
use ::web_time::{Instant};
use ::web_time::Instant as Clock;

fn now() {
    let a = ::web_time::Instant::now();
    let b = web_time::Instant::now();
    let c = ::std::time::Instant::now();
}
"#;
    assert_eq!(lines(&std_instant([("src/now.rs", source)])), vec![8, 9]);
}

#[test]
fn std_instant_reads_a_use_group_split_over_several_lines() {
    let source = r#"
use std::time::{
    Instant
};
use std::time::{
    Duration,
    Instant as Clock,
};
"#;
    let findings = std_instant([("src/clock.rs", source)]);
    assert_eq!(lines(&findings), vec![3, 7]);
    assert_eq!(findings[0].text, "Instant");
}

#[test]
fn std_instant_reads_a_use_after_a_precise_capturing_bound() {
    let source = "fn clock() -> impl Fn() + use<> {\n    use std::{time::Instant};\n    || { let _ = Instant::now(); }\n}\n";
    assert_eq!(lines(&std_instant([("src/clock.rs", source)])), vec![2]);
}

#[test]
fn std_instant_does_not_count_brace_character_literals_as_braces() {
    let source = r#"
#[cfg(unix)]
fn native() { let open = '{'; let b = b'{'; let t = std::time::Instant::now(); }
fn web() { let close = '}'; let t = std::time::Instant::now(); }
#[cfg(unix)]
fn native_two() { let close = '}'; let t = std::time::Instant::now(); }
fn web_two() { let t = std::time::Instant::now(); }
"#;
    assert_eq!(lines(&std_instant([("src/clock.rs", source)])), vec![4, 7]);
}

#[test]
fn an_escaped_line_break_in_a_string_keeps_line_numbers() {
    let source =
        "fn web() {\n    let s = \"one \\\n        two\";\n    let t = std::time::Instant::now();\n    let q = PredefinedMenuItem::quit(None);\n}\n";
    assert_eq!(lines(&std_instant([("src/web.rs", source)])), vec![4]);
    assert_eq!(lines(&predefined_quit([("src/web.rs", source)])), vec![5]);
}

#[test]
fn deeply_nested_use_groups_and_cfg_predicates_do_not_exhaust_the_stack() {
    let depth = 100_000;
    let use_groups = format!("use std::time::{}Instant{};\nfn f() {{}}\n", "{".repeat(depth), "}".repeat(depth));
    assert!(std_instant([("src/deep.rs", use_groups.as_str())]).is_empty());
    assert!(predefined_quit([("src/deep.rs", use_groups.as_str())]).is_empty());

    let cfg = format!("#[cfg({}unix{})]\nfn f() {{ let t = std::time::Instant::now(); }}\n", "not(".repeat(depth), ")".repeat(depth));
    assert_eq!(lines(&std_instant([("src/deep.rs", cfg.as_str())])), vec![2]);

    let shallow = "use std::time::{{{Instant}}};\n";
    assert_eq!(lines(&std_instant([("src/shallow.rs", shallow)])), vec![1]);
}

#[test]
fn a_long_use_path_shared_by_many_names_is_read_without_copying_it_to_each() {
    // 10,000 path segments shared by 10,000 names, followed by a `use` the guard reports.
    let names: Vec<String> = (0..10_000).map(|n| format!("x{n}")).collect();
    let source = format!("use {}{{{}}};\nuse std::time::Instant;\n", "a::".repeat(10_000), names.join(", "));
    assert_eq!(lines(&std_instant([("src/wide.rs", source.as_str())])), vec![2]);
    assert!(predefined_quit([("src/wide.rs", source.as_str())]).is_empty());
}

#[test]
fn a_long_name_shared_by_many_use_leaves_is_read_without_copying_it_to_each() {
    // A 40,000-byte name shared by 10,000 leaves, followed by a `use` the guard reports.
    let source = format!("use {}::{{{}}};\nuse std::time::Instant;\n", "n".repeat(40_000), ["self"; 10_000].join(", "));
    assert_eq!(lines(&std_instant([("src/wide.rs", source.as_str())])), vec![2]);
    assert!(predefined_quit([("src/wide.rs", source.as_str())]).is_empty());
}

#[test]
fn guards_read_names_that_hold_combining_marks() {
    // `t` with U+0301 (a nonspacing mark) and Devanagari `ka` with U+093E (a spacing mark) are each one
    // identifier, as rustc reads them.
    let time = "use std::time as t\u{301};\nuse std::time as \u{915}\u{93E};\nfn run() {\n    let a = t\u{301}::Instant::now();\n    let b = \u{915}\u{93E}::Instant::now();\n}\n";
    assert_eq!(lines(&std_instant([("src/run.rs", time)])), vec![4, 5]);

    let menu = "use muda::{PredefinedMenuItem as Ite\u{301}m};\nfn menu() {\n    let a = Ite\u{301}m::quit(None);\n}\n";
    assert_eq!(lines(&predefined_quit([("src/menu.rs", menu)])), vec![3]);

    // A byte order mark and the left-to-right and right-to-left marks are not part of a name.
    let marked = "\u{FEFF}use std::time::Instant;\nuse std::time as\u{200E}t;\nfn run() { t\u{200F}::Instant::now(); }\n";
    assert_eq!(lines(&std_instant([("src/marked.rs", marked)])), vec![1, 3]);
}

#[test]
fn std_instant_reports_a_grouped_import_through_a_long_alias_of_std_time() {
    let source = r#"
use std::time as standard_time_module_with_a_long_name;
use standard_time_module_with_a_long_name::{Instant as Clock};
fn now() {
    let _ = Clock::now();
}
"#;
    assert_eq!(lines(&std_instant([("src/clock.rs", source)])), vec![3]);
}

#[test]
fn std_instant_reports_a_glob_through_a_long_alias_of_std_time() {
    let source = r#"
use std::time as standard_time_module_with_a_long_name;
use standard_time_module_with_a_long_name::*;
use standard_time_module_with_a_long_name::{Duration, *};
"#;
    assert_eq!(lines(&std_instant([("src/clock.rs", source)])), vec![3, 4]);
}

#[test]
fn std_instant_reports_a_path_of_nine_or_more_segments_that_starts_at_std_time() {
    let source = r#"
use std::time as standard_time_module_with_a_long_name;
use std::time::Instant::a::b::c::d::e::f;
use standard_time_module_with_a_long_name::Instant::a::b::c::d::e::f::g;
"#;
    assert_eq!(lines(&std_instant([("src/clock.rs", source)])), vec![3, 4]);
}

#[test]
fn the_guards_read_an_absolute_path_after_a_return_arrow_or_a_match_arrow() {
    let clock = r#"
fn now() -> ::std::time::Instant {
    match 0 { _ => ::std::time::Instant::now() }
}
fn closure() { let _ = || -> ::std::time::Instant { todo() }; }
"#;
    assert_eq!(lines(&std_instant([("src/clock.rs", clock)])), vec![2, 3, 5]);

    let menu = r#"
fn item() -> Item {
    match 0 { _ => ::muda::PredefinedMenuItem::quit(None) }
}
"#;
    assert_eq!(lines(&predefined_quit([("src/menu.rs", menu)])), vec![3]);
}

#[test]
fn a_long_alias_of_std_time_shared_by_many_use_leaves_is_read_without_copying_it_to_each() {
    // A 40,000-byte alias of `std::time` shared by 10,000 leaves.
    let alias = "n".repeat(40_000);
    let source = format!("use std::time as {alias};\nuse {alias}::{{{}}};\n", ["Instant"; 10_000].join(", "));
    assert_eq!(lines(&std_instant([("src/wide.rs", source.as_str())])), vec![2]);
}

#[test]
fn a_use_past_the_path_limits_still_binds_its_name() {
    let segment = r#"
use other_menu::MenuItemWithAnExceptionallyLongName as PredefinedMenuItem;
fn quit() { PredefinedMenuItem::quit(None); }
"#;
    let length = r#"
use a::b::c::d::e::f::g::h::Item as PredefinedMenuItem;
fn quit() { PredefinedMenuItem::quit(None); }
"#;
    let shared = r#"
use other_menu::MenuItemWithAnExceptionallyLongName::{self as PredefinedMenuItem};
fn quit() { PredefinedMenuItem::quit(None); }
"#;
    assert!(predefined_quit([("src/menu.rs", segment), ("src/len.rs", length), ("src/shared.rs", shared)]).is_empty());

    // A path into this crate past the limits may still be a re-export of muda's type.
    let local = r#"
use crate::module_with_an_exceptionally_long_name::PredefinedMenuItem;
fn quit() { PredefinedMenuItem::quit(None); }
"#;
    assert_eq!(lines(&predefined_quit([("src/local.rs", local)])), vec![3]);

    let clock = r#"
use std::time as t;
fn now() {
    use other_clock::ClockModuleWithAnExceptionallyLongName as t;
    let _ = t::Instant::now();
}
fn later() {
    use other_clock::ClockModuleWithAnExceptionallyLongName::{self as t};
    let _ = t::Instant::now();
}
fn web() { let _ = t::Instant::now(); }
"#;
    assert_eq!(lines(&std_instant([("src/clock.rs", clock)])), vec![11]);
}

#[test]
fn std_instant_reports_an_underscore_import_of_instant_in_a_group() {
    let source = r#"
use std::{time::Instant as _};
use std::time::{Instant as _};
use std::time::Instant as _;
use std::time::{self as _};
"#;
    assert_eq!(lines(&std_instant([("src/clock.rs", source)])), vec![2, 3, 4]);
    assert!(predefined_quit([("src/menu.rs", "use muda::{PredefinedMenuItem as _};\nfn f() { _::quit(); }\n")]).is_empty());
}

#[test]
fn std_instant_keeps_an_import_that_an_alias_in_a_nested_module_shares_a_name_with() {
    let source = r#"
mod unrelated { use ::std::time as std; }
use std::{time::Instant};
"#;
    assert_eq!(lines(&std_instant([("src/clock.rs", source)])), vec![3]);
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
fn std_instant_finds_the_modules_of_a_path_attribute_file_next_to_it() {
    let lib = r#"
#[cfg(not(target_arch = "wasm32"))]
#[path = "platform/native.rs"]
mod native;
"#;
    let clock = "use std::time::Instant;\n";
    let files = [("src/lib.rs", lib), ("src/platform/native.rs", "mod clock;\n"), ("src/platform/clock.rs", clock), ("src/web.rs", clock)];
    assert_eq!(finding_paths(&std_instant(files)), vec!["src/web.rs"]);
}

#[test]
fn std_instant_resolves_a_file_loaded_both_plainly_and_by_path_attribute_once_for_each() {
    // `platform.rs` is a web module, and a native-only `#[path]` declaration loads it again. Its
    // `mod clock;` is `src/platform/clock.rs` for the first and `src/clock.rs` for the second.
    let lib = "mod platform;\n#[cfg(unix)]\n#[path = \"platform.rs\"]\nmod native;\n";
    let clock = "use std::time::Instant;\n";
    let files = [("src/lib.rs", lib), ("src/platform.rs", "mod clock;\n"), ("src/platform/clock.rs", clock), ("src/clock.rs", clock)];
    assert_eq!(finding_paths(&std_instant(files)), vec!["src/platform/clock.rs"]);
}

#[test]
fn std_instant_checks_a_file_that_a_path_attribute_module_names_for_the_web_build() {
    // The web module `entry.rs`, loaded through `#[path]`, names `src/clock.rs`, which a native-only
    // module also names through `#[path]`.
    let lib = "#[path = \"entry.rs\"]\nmod web;\n#[cfg(unix)]\nmod native;\n";
    let clock = "use std::time::Instant;\n";
    let files =
        [("src/lib.rs", lib), ("src/entry.rs", "mod clock;\n"), ("src/native.rs", "#[path = \"clock.rs\"]\nmod clock;\n"), ("src/clock.rs", clock)];
    assert_eq!(finding_paths(&std_instant(files)), vec!["src/clock.rs"]);
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
fn std_instant_checks_a_crate_root_that_a_disabled_declaration_names() {
    let lib = r#"
#[cfg(any())]
#[path = "lib.rs"]
mod never;

fn web() {
    std::time::Instant::now();
}
"#;
    assert_eq!(lines(&std_instant([("src/lib.rs", lib)])), vec![7]);

    // A cycle of two files that only each other name, through declarations the web build leaves out.
    let a = "#[cfg(unix)]\n#[path = \"b.rs\"]\nmod b;\nfn web() { std::time::Instant::now(); }\n";
    let b = "#[cfg(unix)]\n#[path = \"a.rs\"]\nmod a;\n";
    assert_eq!(finding_paths(&std_instant([("src/a.rs", a), ("src/b.rs", b)])), vec!["src/a.rs"]);

    // In a cycle with `lib.rs`, only `lib.rs` is a crate root, so its native-only module stays out.
    let lib = "#[cfg(unix)]\n#[path = \"a.rs\"]\nmod a;\nfn web() {}\n";
    let a = "#[cfg(any())]\n#[path = \"lib.rs\"]\nmod root;\nuse std::time::Instant;\n";
    assert!(std_instant([("src/lib.rs", lib), ("src/a.rs", a)]).is_empty());

    // A cycle that a crate root reaches keeps its native-only member out.
    let lib = "mod a;\n";
    let a = "#[cfg(unix)]\nmod b;\n";
    let b = "#[path = \"../a.rs\"]\nmod again;\nuse std::time::Instant;\n";
    assert!(std_instant([("src/lib.rs", lib), ("src/a.rs", a), ("src/a/b.rs", b)]).is_empty());
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
fn wgsl_literals_after_a_hex_literal_and_an_operator_are_checked() {
    let shader = "const A = 0x1.0p0+1e39;\nconst B = 0xFF+1e39;\nconst C = 0x1f-4e38;\nconst D = 0xAu*2.0+5e38;\nconst E = 0x1p1-0x1p+500;\n";
    let findings = wgsl_float_literals(shader);
    assert_eq!(lines(&findings), vec![1, 2, 3, 4, 5]);
    let texts: Vec<&str> = findings.iter().map(|f| f.text.as_str()).collect();
    assert_eq!(texts, vec!["1e39", "1e39", "4e38", "5e38", "0x1p+500"]);
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

#[test]
fn wgsl_line_comments_end_and_lines_count_at_every_line_break() {
    let shader = "// note\rconst A: f32 = 4e38;\r\nconst B: f32 = 5e38;\n// x\u{2028}const C: f32 = 6e38;\u{2029}const D: f32 = 7e38;\u{000B}// y\u{0085}const E: f32 = 8e38;\u{000C}const F: f32 = 9e38;";
    let findings = wgsl_float_literals(shader);
    assert_eq!(lines(&findings), vec![2, 3, 5, 6, 8, 9]);
    assert_eq!(findings[0].text, "4e38");
}

#[test]
fn wgsl_digits_inside_unicode_identifiers_are_not_literals() {
    let shader = "const π1e39 = 1.0;\nconst é2e40: f32 = 2.0;\nconst A: f32 =\u{200E}1e39;\n";
    let findings = wgsl_float_literals(shader);
    assert_eq!(lines(&findings), vec![3]);
    assert_eq!(findings[0].text, "1e39");
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
fn rust_sources_does_not_read_a_cachedir_tag_through_a_symlink() {
    let outside = ScratchDir::new("cachedir_link_target");
    outside.write("CACHEDIR.TAG", "Signature: 8a477f597d28d172789f06886806bc55\n");
    let dir = ScratchDir::new("cachedir_link");
    dir.write("src/lib.rs", "pub fn lib() {}");
    dir.write("linked/kept.rs", "pub fn kept() {}");
    std::os::unix::fs::symlink(outside.path().join("CACHEDIR.TAG"), dir.path().join("linked").join("CACHEDIR.TAG")).expect("symlink marker");
    let sources = craft_testkit::rust_sources(dir.path()).expect("rust_sources");
    assert_eq!(file_names(&sources), vec!["kept.rs", "lib.rs"]);
}

#[cfg(unix)]
#[test]
fn rust_sources_does_not_open_a_cachedir_tag_fifo() {
    let dir = ScratchDir::new("cachedir_fifo");
    dir.write("src/lib.rs", "pub fn lib() {}");
    dir.write("piped/kept.rs", "pub fn kept() {}");
    let fifo = dir.path().join("piped").join("CACHEDIR.TAG");
    match std::process::Command::new("mkfifo").arg(&fifo).status() {
        Ok(status) if status.success() => {}
        other => {
            eprintln!("skipped: mkfifo is not available here ({other:?})");
            return;
        }
    }
    // Opening the FIFO for reading and writing does not wait for another end on Linux or macOS. The
    // signature waits in the pipe, so a scan that opened the FIFO would read it and leave out `kept.rs`
    // without blocking.
    let mut pipe = std::fs::OpenOptions::new().read(true).write(true).open(&fifo).expect("open FIFO");
    std::io::Write::write_all(&mut pipe, b"Signature: 8a477f597d28d172789f06886806bc55\n").expect("write signature");
    let sources = craft_testkit::rust_sources(dir.path()).expect("rust_sources");
    drop(pipe);
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

#[cfg(unix)]
#[test]
fn rust_sources_skips_a_broken_symlink() {
    let dir = ScratchDir::new("symlink_broken");
    dir.write("src/lib.rs", "pub fn lib() {}");
    std::os::unix::fs::symlink(dir.path().join("missing.rs"), dir.path().join("src").join("gone.rs")).expect("symlink file");
    std::os::unix::fs::symlink(dir.path().join("missing"), dir.path().join("src").join("gone")).expect("symlink folder");
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
