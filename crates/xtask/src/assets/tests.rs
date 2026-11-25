use super::*;

/// The policy of the apps with `ASSETS.md` or a root `ATTRIBUTION.md`, exact rows and the three
/// asset folders.
const COMMON: Policy<'static> = Policy { all_files_under: &["assets/", "docs/images/", "examples/"], ..Policy::new("ASSETS.md") };

fn strings(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| (*s).to_string()).collect()
}

fn unattributed(report: &Report) -> Vec<String> {
    report
        .violations
        .iter()
        .filter_map(|v| match v {
            Violation::Unattributed { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect()
}

fn missing(files: &[String], md: &str, policy: &Policy<'_>) -> Vec<String> {
    unattributed(&check(files, md, policy, |_| true).unwrap())
}

#[test]
fn detects_unattributed_assets() {
    let files = strings(&["assets/icons/a.svg", "crates/x/src/lib.rs", "docs/images/b.png", "tests/fixture.png"]);
    let md = "| `assets/icons/a.svg` | me | here | MIT |";
    assert_eq!(missing(&files, md, &COMMON), vec!["docs/images/b.png".to_string(), "tests/fixture.png".into()]);
    assert!(!is_asset(&COMMON, "crates/x/src/lib.rs"));
    assert!(is_asset(&COMMON, "assets/fonts/OFL.txt"));
}

#[test]
fn ba_png_does_not_attribute_a_png() {
    let files = strings(&["assets/a.png"]);
    assert_eq!(missing(&files, "| `assets/ba.png` | x |", &COMMON), files);
    assert_eq!(missing(&files, "| `xassets/a.png` | x |", &COMMON), files);
}

#[test]
fn prose_and_other_columns_do_not_attribute() {
    let files = strings(&["assets/a.png"]);
    assert_eq!(missing(&files, "The file `assets/a.png` is ours.", &COMMON), files);
    assert_eq!(missing(&files, "| Icon | `assets/a.png` | MIT |", &COMMON), files);
    assert!(missing(&files, "| `assets/a.png` | Icon | MIT |", &COMMON).is_empty());
    assert!(missing(&files, "  |`assets/a.png`, `assets/b.png`| Icons |", &COMMON).is_empty());
}

#[test]
fn rows_in_code_fences_do_not_attribute() {
    let files = strings(&["assets/a.png"]);
    assert_eq!(missing(&files, "```\n| `assets/a.png` | x |\n```", &COMMON), files);
    assert!(missing(&files, "```\n| x |\n```\n| `assets/a.png` | x |", &COMMON).is_empty());
    assert_eq!(missing(&files, "````md\n```\n| `assets/a.png` | x |\n````", &COMMON), files);
    assert_eq!(missing(&files, "~~~\n```\n| `assets/a.png` | x |\n~~~", &COMMON), files);
    assert_eq!(missing(&files, "```\n| `assets/a.png` | x |\n``` not a close", &COMMON), files);
    assert_eq!(missing(&files, "Rows:\n\n    | `assets/a.png` | x |", &COMMON), files);
    assert_eq!(missing(&files, "\t| `assets/a.png` | x |", &COMMON), files);
    assert!(missing(&files, "   | `assets/a.png` | x |", &COMMON).is_empty());
}

#[test]
fn escaped_and_backticked_pipes_stay_in_their_cell() {
    let files = strings(&["assets/a|b.png", "assets/c.png"]);
    let md = "| `assets/a|b.png` | x |\n| odd \\| `assets/c.png` | y |";
    assert!(missing(&files, md, &COMMON).is_empty());
    assert_eq!(missing(&files, "| `assets/a|b.png` | x |\n| odd | `assets/c.png` | y |", &COMMON), vec!["assets/c.png".to_string()]);
}

#[test]
fn tables_without_outer_pipes_are_read() {
    let files = strings(&["assets/a.png", "assets/b.png"]);
    let md = "Path | License\n--- | :---:\n`assets/a.png` | MIT\n\nThe file `assets/b.png` | is not in the table.";
    assert_eq!(missing(&files, md, &COMMON), vec!["assets/b.png".to_string()]);
    assert_eq!(missing(&files[..1], "Path | License\nnot a delimiter | x\n`assets/a.png` | MIT", &COMMON), files[..1]);
    let lightcraft = Policy { license_column: Some(1), ..COMMON };
    let report = check(&files[..1], "Path | License\n---|---\n`assets/a.png` | `licenses/MIT`", &lightcraft, |_| false).unwrap();
    assert_eq!(report.violations, vec![Violation::MissingLicense { line: 3, file: "licenses/MIT".into() }]);
}

#[test]
fn multi_backtick_spans_name_paths() {
    let files = strings(&["assets/a.png", "assets/b`c.png"]);
    assert!(missing(&files, "| ``assets/a.png`` | x |\n| `` assets/b`c.png `` | y |", &COMMON).is_empty());
    assert_eq!(missing(&files[..1], "| ``assets/a.png` | x |", &COMMON), files[..1]);
    assert!(missing(&files[..1], "| ``x|y`` `assets/a.png` | x |", &COMMON).is_empty());
}

#[test]
fn many_unmatched_backtick_runs_are_read_once() {
    let line: String = (1..=1500).map(|n| format!("{}x", "`".repeat(n))).collect();
    let md = format!("| {line} `assets/a.png` | x |");
    assert_eq!(missing(&strings(&["assets/a.png"]), &md, &COMMON).len(), 1);
    let spans = code_spans(&format!("{line} `a`"));
    assert_eq!(spans.len(), 1);
}

#[test]
fn an_unclosed_backtick_names_nothing() {
    assert_eq!(missing(&strings(&["assets/a.png"]), "| `assets/a.png | x |", &COMMON).len(), 1);
    assert_eq!(missing(&strings(&["assets/a.png"]), "| `assets/a.png", &COMMON).len(), 1);
}

#[test]
fn extensions_compare_without_case() {
    assert!(is_asset(&COMMON, "src/Icon.PNG"));
    assert!(is_asset(&COMMON, "src/font.Woff2"));
    let p = Policy { extra_ext: &["CSV"], ..COMMON };
    assert!(is_asset(&p, "data/a.csv"));
    assert!(!is_asset(&COMMON, "src/.png"));
    assert!(!is_asset(&COMMON, "src/png"));
}

#[test]
fn a_case_only_mismatch_names_the_row() {
    let report = check(&strings(&["assets/Logo.png"]), "| `assets/logo.png` | x |", &COMMON, |_| true).unwrap();
    assert_eq!(report.violations, vec![Violation::Unattributed { path: "assets/Logo.png".into(), near: Some("assets/logo.png".into()) }]);
    assert!(report.violations[0].to_string().contains("only in letter case"));
}

#[test]
fn bad_row_entries_are_reported() {
    let md = "| `assets\\a.png` | x |\n| `/assets/b.png` | x |\n| `assets/../c.png` | x |\n| `C:/d.png` | x |\n| `C:other/e.png` | x |";
    let report = check(&[], md, &COMMON, |_| true).unwrap();
    let lines: Vec<usize> = report
        .violations
        .iter()
        .filter_map(|v| match v {
            Violation::BadEntry { line, .. } => Some(*line),
            _ => None,
        })
        .collect();
    assert_eq!(lines, vec![1, 2, 3, 4, 5]);
}

#[test]
fn forbidden_extension_wins_over_the_union() {
    let p = Policy { forbidden_ext: &["ase", "dcp"], ..COMMON };
    let report = check(&strings(&["swatches/a.ASE", "p/camera.dcp", "assets/x.png"]), "| `assets/x.png` | |", &p, |_| true).unwrap();
    assert_eq!(
        report.violations,
        vec![
            Violation::ForbiddenExtension { path: "swatches/a.ASE".into(), ext: "ase".into() },
            Violation::ForbiddenExtension { path: "p/camera.dcp".into(), ext: "dcp".into() },
        ]
    );
    assert_eq!(report.assets, 1);
}

#[test]
fn skipped_folders_are_counted() {
    let p = Policy { skip_folders: &["vendor"], forbidden_ext: &["dcp"], ..COMMON };
    let report = check(&strings(&["vendor/x/a.png", "vendorx/b.png", "vendor/camera.dcp"]), "", &p, |_| true).unwrap();
    assert_eq!(report.skipped_by_folder, 1);
    assert_eq!(unattributed(&report), vec!["vendorx/b.png".to_string()]);
    assert!(report.violations.contains(&Violation::ForbiddenExtension { path: "vendor/camera.dcp".into(), ext: "dcp".into() }));
}

#[test]
fn folders_match_whole_components() {
    assert!(!is_asset(&COMMON, "assetsx/readme.txt"));
    assert!(!is_asset(&COMMON, "assets"));
    assert!(is_asset(&COMMON, "assets/readme.txt"));
}

#[test]
fn invalid_policies_are_errors() {
    for p in [
        Policy::new("docs\\ASSETS.md"),
        Policy::new("/ASSETS.md"),
        Policy::new("docs/"),
        Policy { all_files_under: &["../assets"], ..COMMON },
        Policy { skip_folders: &[""], ..COMMON },
        Policy { extra_ext: &[".png"], ..COMMON },
        Policy { forbidden_ext: &[""], ..COMMON },
        Policy { forbidden_name_fragments: &[""], ..COMMON },
        Policy { ext_in_folders: &[FolderExtensions { folder: "a//b", nested: false, extensions: &["csv"] }], ..COMMON },
        Policy { license_column: Some(0), ..COMMON },
        Policy { max_document_bytes: 0, ..COMMON },
        Policy { max_git_output_bytes: 0, ..COMMON },
    ] {
        assert!(matches!(check(&[], "", &p, |_| true), Err(Error::Policy(_))), "{p:?}");
    }
    assert!(validate(&COMMON).is_ok());
}

#[test]
fn names_git_cannot_produce_are_errors() {
    assert!(matches!(parse_ls_files(b"a\xff.png\0"), Err(Error::GitName(_))));
    assert!(matches!(parse_ls_files(b"/etc/passwd\0"), Err(Error::GitName(_))));
    assert!(matches!(parse_ls_files(b"a/../b.png\0"), Err(Error::GitName(_))));
}

#[test]
fn names_rows_cannot_spell_are_errors() {
    assert!(matches!(parse_ls_files(b"assets\\icon.png\0"), Err(Error::GitName(_))));
    assert!(matches!(parse_ls_files(b"C:icon.png\0"), Err(Error::GitName(_))));
    assert!(parse_ls_files(b"assets/c:/icon.png\0").is_ok());
}

#[test]
fn repeated_names_are_listed_once() {
    assert_eq!(parse_ls_files(b"b.png\0a.png\0b.png\0b.png\0").unwrap(), vec!["a.png".to_string(), "b.png".into()]);
}

#[test]
fn union_holds_every_app_list() {
    const APPS: &[&[&str]] = &[
        // VectorCraft
        &[
            "png",
            "jpg",
            "jpeg",
            "gif",
            "webp",
            "bmp",
            "tif",
            "tiff",
            "ico",
            "icns",
            "svg",
            "pdf",
            "ai",
            "eps",
            "psd",
            "icc",
            "icm",
            "ttf",
            "otf",
            "woff",
            "woff2",
            "ase",
            "aco",
            "abr",
            "vectorcraft",
            "drawcraft",
            "mp4",
            "wav",
            "mp3",
        ],
        // DesignCraft, CADCraft and WordCraft add these to VectorCraft's list without `vectorcraft` and `drawcraft`.
        &["indd", "idml", "designcraft", "dxf", "dwg", "docx", "odt", "rtf"],
        // DeckCraft
        &["deckcraft", "pptx", "potx", "ppsx", "thmx", "mov", "webm", "m4a", "ogg", "flac"],
        // GridCraft
        &["xlsx", "xlsm", "xls"],
        // LightCraft
        &["avif", "dng", "cr2", "cr3", "nef", "arw", "raf", "orf", "rw2", "rwl", "raw", "pef", "srw", "heic", "jxl", "cube", "xmp"],
        // Font collections
        &["ttc", "otc"],
    ];
    for ext in APPS.iter().flat_map(|l| l.iter()) {
        assert!(ASSET_EXT.contains(ext), "{ext}");
    }
    assert_eq!(ASSET_EXT.len(), 69);
    assert!(ASSET_EXT.windows(2).all(|w| w[0] < w[1]), "sorted and unique");
}

#[test]
fn oversized_documents_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ASSETS.md"), "x".repeat(65)).unwrap();
    assert!(matches!(read_document(dir.path(), "ASSETS.md", 64), Err(Error::Document(ref m)) if m.contains("larger than 64 bytes")));
    assert_eq!(read_document(dir.path(), "ASSETS.md", 65).unwrap().len(), 65);
    std::fs::write(dir.path().join("bad.md"), b"\xff").unwrap();
    assert!(matches!(read_document(dir.path(), "bad.md", 64), Err(Error::Document(ref m)) if m.contains("not UTF-8")));
    assert!(matches!(read_document(dir.path(), "none.md", 64), Err(Error::Document(_))));
}

mod deckcraft {
    use super::*;

    const DECK: Policy<'static> = Policy {
        attribution_file: "ATTRIBUTION.md",
        row_mode: RowMode::Patterns,
        all_files_under: &["assets/", "docs/images/", "docs/brand/", "examples/"],
        ..COMMON
    };

    #[test]
    fn detects_unattributed_assets() {
        let files = strings(&["assets/icons/a.svg", "crates/x/src/lib.rs", "docs/images/b.png", "tests/fixture.png"]);
        let md = "| `assets/icons/a.svg` | me | here | MIT |";
        assert_eq!(missing(&files, md, &DECK), vec!["docs/images/b.png".to_string(), "tests/fixture.png".into()]);
        assert!(!is_asset(&DECK, "crates/x/src/lib.rs"));
        assert!(is_asset(&DECK, "assets/fonts/OFL.txt"));
        let md = "| `assets/app-icon/hicolor/*/apps/x.png` | |";
        assert!(missing(&strings(&["assets/app-icon/hicolor/16x16/apps/x.png"]), md, &DECK).is_empty());
        assert_eq!(missing(&strings(&["assets/app-icon/hicolor/a/b/apps/x.png"]), md, &DECK).len(), 1);
    }

    #[test]
    fn glob_handles_multibyte_names() {
        let g = |pat: &str, name: &str| glob(&pat.chars().collect::<Vec<_>>(), &name.chars().collect::<Vec<_>>());
        assert!(g("assets/*/x.png", "assets/żółć/x.png"));
        assert!(g("assets/*.png", "assets/日本語.png"));
        assert!(!g("assets/*/x.png", "assets/ż/ó/x.png"));
        assert!(missing(&strings(&["assets/żółć/x.png"]), "| `assets/*/x.png` |", &DECK).is_empty());
        assert_eq!(missing(&strings(&["assets/é.png"]), "| `assets/*.svg` |", &DECK), vec!["assets/é.png".to_string()]);
    }

    #[test]
    fn many_wildcards_on_a_failing_name_finish_at_once() {
        let g = |pat: &str, name: &str| glob(&pat.chars().collect::<Vec<_>>(), &name.chars().collect::<Vec<_>>());
        let pat = format!("{}b", "a*".repeat(40));
        assert!(!g(&pat, &"a".repeat(60)));
        assert!(g(&pat, &format!("{}b", "a".repeat(60))));
        assert!(!g("a*b", "a/b"));
        assert!(g("a*b", "ab"));
        // Two thousand `*` against a long name that fails at its end.
        assert!(!g(&format!("{}x", "*a".repeat(2000)), &"a".repeat(4000)));
    }

    #[test]
    fn ls_files_output_is_nul_separated_and_unquoted() {
        let names = parse_ls_files("assets/zażółć.png\0docs/images/a b\nc.png\0".as_bytes()).unwrap();
        assert_eq!(names, vec!["assets/zażółć.png".to_string(), "docs/images/a b\nc.png".into()]);
    }
}

mod wordcraft {
    use super::*;

    #[test]
    fn reads_raw_nul_separated_paths() {
        assert_eq!(
            parse_ls_files(b"assets/\xc3\xa9 b.png\0docs/images/x.svg\0").unwrap(),
            vec!["assets/é b.png".to_string(), "docs/images/x.svg".into()]
        );
        assert!(parse_ls_files(b"").unwrap().is_empty());
    }

    #[test]
    fn a_failed_file_listing_fails_the_check() {
        let _guard = crate::spawn_guard();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ASSETS.md"), "").unwrap();
        let inside_git = git(dir.path()).args(["rev-parse", "--git-dir"]).output().is_ok_and(|o| o.status.success());
        if inside_git {
            return; // the temporary folder is in a checkout here: nothing to test
        }
        let result = run(dir.path(), &COMMON);
        assert!(matches!(result, Err(Error::Git(ref m)) if m.starts_with("exited with")), "{result:?}");
    }
}

mod gridcraft {
    use super::*;

    const GRID: Policy<'static> = Policy {
        attribution_file: "ATTRIBUTION.md",
        all_files_under: &["assets/", "docs/brand/", "docs/images/"],
        ext_in_folders: &[FolderExtensions { folder: "examples", nested: true, extensions: &["csv", "tsv", "json"] }],
        ..COMMON
    };

    #[test]
    fn detects_unattributed_assets() {
        let files =
            strings(&["assets/app-icon/a.svg", "crates/x/src/lib.rs", "docs/images/b.png", "tests/fixture.xlsx", "docs/brand/LICENSE-brand.txt"]);
        let md = "| `assets/app-icon/a.svg` | me | here | MIT |\n| `docs/brand/LICENSE-brand.txt` | x | y | z |";
        assert_eq!(missing(&files, md, &GRID), vec!["docs/images/b.png".to_string(), "tests/fixture.xlsx".into()]);
    }

    #[test]
    fn classifies_paths() {
        assert!(!is_asset(&GRID, "crates/x/src/lib.rs"));
        assert!(!is_asset(&GRID, "docs/releasing.md"));
        assert!(is_asset(&GRID, "assets/app-icon/LICENSE.txt"));
        assert!(is_asset(&GRID, "examples/sales.csv"));
        assert!(is_asset(&GRID, "crates/engine/examples/budget.xlsx"));
        assert!(is_asset(&GRID, "crates/engine/examples/budget.json"));
        assert!(!is_asset(&GRID, "crates/xlsx/tests/data.csv"));
        assert!(!is_asset(&GRID, "crates/myexamples/data.csv"));
        assert!(is_asset(&GRID, "crates/ui-egui/icon.PNG"));
        let root_only = Policy { ext_in_folders: &[FolderExtensions { folder: "examples", nested: false, extensions: &["csv"] }], ..GRID };
        assert!(is_asset(&root_only, "examples/sales.csv"));
        assert!(!is_asset(&root_only, "crates/engine/examples/sales.csv"));
    }
}

mod lightcraft {
    use super::*;

    const LIGHT: Policy<'static> = Policy {
        row_mode: RowMode::Patterns,
        forbidden_ext: &["dcp", "lcp", "lrtemplate", "lrcat", "lrsmcol", "aco", "ase", "abr"],
        forbidden_name_fragments: &["vendor", "creative cloud"],
        license_column: Some(4),
        ..Policy::new("assets/ATTRIBUTION.md")
    };

    const MD: &str = "| Path | Asset | Creator | Source | License | Added | Mods |\n|---|---|---|---|---|---|---|\n\
        | `assets/fonts/Foo-*.ttf` | Foo | A | url | OFL (`assets/fonts/OFL-Foo.txt`) | d | none |\n\
        | `crates/scenes/` | gen | us | original | MIT | d | n/a |\n";

    fn g(pat: &str, name: &str) -> bool {
        let mut rows = Rows::default();
        rows.add(RowMode::Patterns, 1, pat);
        rows.bad.is_empty() && rows.covers(name)
    }

    #[test]
    fn globbing() {
        assert!(g("docs/images/*-tetons.jpg", "docs/images/ba-tetons.jpg"));
        assert!(!g("docs/images/*.jpg", "docs/images/sub/x.jpg"));
        assert!(g("crates/scenes/", "crates/scenes/a/b.png"));
        // forty `*` against a long name that fails at its very end
        let pat = format!("{}x", "*a".repeat(40));
        assert!(!g(&pat, &"a".repeat(200)));
        assert!(g("*a*b*", "xaybz") && !g("*a*b*", "xa/b"));
        assert!(!g("crates/scenes/", "crates/scenesx/b.png"));
        assert!(g("crates/*/", "crates/scenes/a/b.png"));
        assert!(!g("crates/*/", "crates/b.png"));
    }

    #[test]
    fn flags_unattributed_forbidden_and_missing_licence() {
        let files = strings(&[
            "assets/fonts/Foo-Bold.ttf",
            "assets/icons/x.svg",
            "crates/scenes/t.png",
            "src/lib.rs",
            "p/camera.dcp",
            "assets/fonts/Vendor-Foo.otf",
        ]);
        let report = check(&files, MD, &LIGHT, |_| false).unwrap();
        let p: Vec<String> = report.violations.iter().map(Violation::to_string).collect();
        assert!(p.iter().any(|m| m.starts_with("`assets/icons/x.svg` has no row")), "{p:?}");
        assert!(p.iter().any(|m| m.starts_with("`p/camera.dcp`")));
        assert!(p.iter().any(|m| m.contains("Vendor-Foo.otf`: the file name contains `vendor`")));
        assert!(p.iter().any(|m| m.contains("OFL-Foo.txt")));
        assert!(!p.iter().any(|m| m.starts_with("`assets/fonts/Foo-Bold.ttf") || m.starts_with("`crates/scenes") || m.starts_with("`src/")));
        assert!(check(&files[..1], MD, &LIGHT, |f| f == "assets/fonts/OFL-Foo.txt").unwrap().is_ok());
    }

    #[test]
    fn unsafe_license_references_are_reported_without_a_lookup() {
        let md = "| `a/x.png` | | | | `../outside/LICENSE` |";
        let report = check(&[], md, &LIGHT, |_| panic!("looked up")).unwrap();
        assert!(matches!(report.violations.as_slice(), [Violation::BadEntry { line: 1, .. }]), "{:?}", report.violations);
    }
}

/// A `git` command in `dir`, apart from any repository the test itself runs in.
fn git(dir: &Path) -> Command {
    let mut c = Command::new("git");
    c.current_dir(dir).env_remove("GIT_DIR").env_remove("GIT_WORK_TREE").env_remove("GIT_INDEX_FILE");
    c
}

fn git_ok(dir: &Path, args: &[&str]) {
    let out = git(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn write(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

/// A repository with a tracked file in an ignored folder, an untracked file, an ignored file and
/// unusual names.
fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    git_ok(d, &["init", "-q"]);
    write(d, ".gitignore", "ignored/\n");
    write(d, "ignored/tracked.png", "x");
    git_ok(d, &["add", "-f", "ignored/tracked.png"]);
    write(d, "ignored/untracked.png", "x");
    write(d, "assets/new.png", "x");
    write(d, "assets/a b.png", "x");
    write(d, "assets/zażółć.png", "x");
    write(d, "src/lib.rs", "");
    dir
}

#[test]
fn git_lists_tracked_and_unignored_files() {
    let _guard = crate::spawn_guard();
    let dir = repo();
    let listing = list_files(dir.path(), DEFAULT_GIT_OUTPUT_LIMIT).unwrap();
    assert_eq!(listing.files, strings(&[".gitignore", "assets/a b.png", "assets/new.png", "assets/zażółć.png", "ignored/tracked.png", "src/lib.rs"]));
    assert_eq!(listing.skipped_entries, 0);
}

#[test]
fn git_output_over_the_limit_is_an_error() {
    let _guard = crate::spawn_guard();
    let dir = repo();
    assert!(matches!(list_files(dir.path(), 8), Err(Error::Git(ref m)) if m.contains("more than 8 bytes")));
}

#[test]
fn run_reads_git_and_the_document() {
    let _guard = crate::spawn_guard();
    let dir = repo();
    let d = dir.path();
    write(d, "assets/ATTRIBUTION.md", "| Path | License |\n|---|---|\n| `assets/new.png` | MIT |\n| `assets/zażółć.png` | MIT |\n");
    let p = Policy { all_files_under: &["assets"], ..Policy::new("assets/ATTRIBUTION.md") };
    let report = run(d, &p).unwrap();
    assert_eq!(report.assets, 5);
    assert_eq!(unattributed(&report), strings(&["assets/ATTRIBUTION.md", "assets/a b.png", "ignored/tracked.png"]));
}

#[cfg(unix)]
#[test]
fn links_are_not_followed_and_newlines_are_names() {
    let _guard = crate::spawn_guard();
    let dir = repo();
    let d = dir.path();
    std::os::unix::fs::symlink(d.join("assets/new.png"), d.join("assets/link.png")).unwrap();
    std::os::unix::fs::symlink(d.join("assets"), d.join("linked")).unwrap();
    write(d, "assets/two\nlines.png", "x");
    let listing = list_files(d, DEFAULT_GIT_OUTPUT_LIMIT).unwrap();
    assert!(listing.files.contains(&"assets/two\nlines.png".to_string()));
    assert!(!listing.files.iter().any(|f| f == "assets/link.png" || f.starts_with("linked")));
    assert_eq!(listing.skipped_entries, 2);
}

#[cfg(unix)]
#[test]
fn files_through_a_linked_folder_are_not_listed() {
    let _guard = crate::spawn_guard();
    let dir = repo();
    let d = dir.path();
    write(d, "real/a.png", "x");
    git_ok(d, &["add", "real/a.png"]);
    std::fs::remove_dir_all(d.join("real")).unwrap();
    let outside = tempfile::tempdir().unwrap();
    write(outside.path(), "a.png", "x");
    std::os::unix::fs::symlink(outside.path(), d.join("real")).unwrap();
    let listing = list_files(d, DEFAULT_GIT_OUTPUT_LIMIT).unwrap();
    assert!(!listing.files.iter().any(|f| f.starts_with("real")), "{:?}", listing.files);
    // `real/a.png` from the index and the untracked link `real` itself.
    assert_eq!(listing.skipped_entries, 2);
}

#[cfg(unix)]
#[test]
fn a_linked_license_file_does_not_count() {
    let _guard = crate::spawn_guard();
    let dir = repo();
    let d = dir.path();
    let outside = tempfile::tempdir().unwrap();
    write(outside.path(), "LICENSE", "x");
    write(d, "licenses/OWN", "x");
    std::os::unix::fs::symlink(outside.path().join("LICENSE"), d.join("licenses/LINKED")).unwrap();
    std::os::unix::fs::symlink(outside.path(), d.join("outside")).unwrap();
    write(
        d,
        "ATTRIBUTION.md",
        "| Path | License |\n|---|---|\n| `assets/new.png` | `licenses/OWN` |\n| `assets/a b.png` | `licenses/LINKED` |\n| `assets/x.png` | `outside/LICENSE` |\n",
    );
    let p = Policy { license_column: Some(1), ..Policy::new("ATTRIBUTION.md") };
    let report = run(d, &p).unwrap();
    let licenses: Vec<&Violation> = report.violations.iter().filter(|v| matches!(v, Violation::MissingLicense { .. })).collect();
    assert_eq!(
        licenses,
        vec![
            &Violation::MissingLicense { line: 4, file: "licenses/LINKED".into() },
            &Violation::MissingLicense { line: 5, file: "outside/LICENSE".into() },
        ]
    );
}
