//! Platform regression guards checking source code for known defects and lifecycle hazards.

use std::fmt;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

/// A diagnostic finding reported by a source guard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// File path where the violation was detected.
    pub path: String,
    /// 1-based line number of the violation.
    pub line: usize,
    /// Diagnostic context (the offending code snippet or float literal).
    pub text: String,
}

impl Finding {
    /// Create a new diagnostic finding.
    pub fn new(path: impl Into<String>, line: usize, text: impl Into<String>) -> Self {
        Self { path: path.into(), line, text: text.into() }
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() { write!(f, "{}: {}", self.line, self.text) } else { write!(f, "{}:{}: {}", self.path, self.line, self.text) }
    }
}

/// Abstraction over a source file path and its text content.
pub trait AsFileSource {
    /// Path of the source file.
    fn path(&self) -> &str;
    /// Text content of the source file.
    fn content(&self) -> &str;
}

impl AsFileSource for (String, String) {
    fn path(&self) -> &str {
        &self.0
    }
    fn content(&self) -> &str {
        &self.1
    }
}

impl AsFileSource for &(String, String) {
    fn path(&self) -> &str {
        &self.0
    }
    fn content(&self) -> &str {
        &self.1
    }
}

impl AsFileSource for (&str, &str) {
    fn path(&self) -> &str {
        self.0
    }
    fn content(&self) -> &str {
        self.1
    }
}

impl AsFileSource for &(&str, &str) {
    fn path(&self) -> &str {
        self.0
    }
    fn content(&self) -> &str {
        self.1
    }
}

impl AsFileSource for (String, &str) {
    fn path(&self) -> &str {
        &self.0
    }
    fn content(&self) -> &str {
        self.1
    }
}

impl AsFileSource for &(String, &str) {
    fn path(&self) -> &str {
        &self.0
    }
    fn content(&self) -> &str {
        self.1
    }
}

impl AsFileSource for (&str, String) {
    fn path(&self) -> &str {
        self.0
    }
    fn content(&self) -> &str {
        &self.1
    }
}

impl AsFileSource for &(&str, String) {
    fn path(&self) -> &str {
        self.0
    }
    fn content(&self) -> &str {
        &self.1
    }
}

/// Replace code comments and string literals with whitespace while preserving newlines and line counts.
///
/// Handles single-line comments (`//`), block comments (`/* ... */`, including nested comments),
/// normal string literals (`"..."`), and raw string literals (`r#"..."#`).
fn mask_comments_and_strings(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let chars: Vec<char> = code.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut in_line_comment = false;
    let mut block_comment_depth: usize = 0;
    let mut in_normal_string = false;
    let mut raw_string_hashes: Option<usize> = None;
    let mut escape = false;

    while i < len {
        let c = chars[i];
        if in_line_comment {
            if c == '\n' {
                in_line_comment = false;
                out.push('\n');
            } else {
                out.push(' ');
            }
            i += 1;
        } else if block_comment_depth > 0 {
            if c == '*' && i + 1 < len && chars[i + 1] == '/' {
                block_comment_depth = block_comment_depth.saturating_sub(1);
                out.push(' ');
                out.push(' ');
                i += 2;
            } else if c == '/' && i + 1 < len && chars[i + 1] == '*' {
                block_comment_depth = block_comment_depth.saturating_add(1);
                out.push(' ');
                out.push(' ');
                i += 2;
            } else if c == '\n' {
                out.push('\n');
                i += 1;
            } else {
                out.push(' ');
                i += 1;
            }
        } else if let Some(hashes) = raw_string_hashes {
            if c == '"' {
                let mut match_hashes = true;
                if i + hashes < len {
                    for h in 0..hashes {
                        if chars[i + 1 + h] != '#' {
                            match_hashes = false;
                            break;
                        }
                    }
                } else {
                    match_hashes = false;
                }
                if match_hashes {
                    raw_string_hashes = None;
                    for _ in 0..=hashes {
                        out.push(' ');
                    }
                    i += 1 + hashes;
                    continue;
                }
            }
            if c == '\n' {
                out.push('\n');
            } else {
                out.push(' ');
            }
            i += 1;
        } else if in_normal_string {
            if escape {
                escape = false;
                out.push(' ');
            } else if c == '\\' {
                escape = true;
                out.push(' ');
            } else if c == '"' {
                in_normal_string = false;
                out.push(' ');
            } else if c == '\n' {
                out.push('\n');
            } else {
                out.push(' ');
            }
            i += 1;
        } else {
            if c == '/' && i + 1 < len && chars[i + 1] == '/' {
                in_line_comment = true;
                out.push(' ');
                out.push(' ');
                i += 2;
            } else if c == '/' && i + 1 < len && chars[i + 1] == '*' {
                block_comment_depth = 1;
                out.push(' ');
                out.push(' ');
                i += 2;
            } else if c == '"' {
                in_normal_string = true;
                out.push(' ');
                i += 1;
            } else if c == 'r' && i + 1 < len && (chars[i + 1] == '"' || chars[i + 1] == '#') {
                let mut j = i + 1;
                let mut hashes = 0;
                while j < len && chars[j] == '#' {
                    hashes += 1;
                    j += 1;
                }
                if j < len && chars[j] == '"' {
                    raw_string_hashes = Some(hashes);
                    for _ in i..=j {
                        out.push(' ');
                    }
                    i = j + 1;
                } else {
                    out.push(c);
                    i += 1;
                }
            } else {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// Guard: detects direct calls to `muda`'s predefined Quit menu item.
///
/// Using `PredefinedMenuItem::quit` connects the macOS menu directly to `NSApplication terminate:`,
/// which bypasses dirty document checks, unsaved changes save prompts, and lifecycle cleanup.
/// Applications should route Quit through custom menu items or interceptors connected to `craft-quit`.
pub fn predefined_quit<I>(files: I) -> Vec<Finding>
where
    I: IntoIterator,
    I::Item: AsFileSource,
{
    let mut findings = Vec::new();
    for file in files {
        let path = file.path();
        let content = file.content();
        let masked = mask_comments_and_strings(content);

        for (orig_line, (idx, masked_line)) in content.lines().zip(masked.lines().enumerate()) {
            let normalized = masked_line.replace(' ', "");
            if normalized.contains("PredefinedMenuItem::quit") || normalized.contains("PredefinedMenuItem::Quit") {
                findings.push(Finding::new(path, idx + 1, orig_line.trim()));
            }
        }
    }
    findings
}

/// Guard: detects uses of `std::time::Instant` outside comments in code paths running on the web.
///
/// On `wasm32-unknown-unknown`, `std::time::Instant::now()` panics with "time not implemented
/// on this platform". Crates running in the browser should use `web_time::Instant`, which maps to
/// `std::time::Instant` on native platforms and `performance.now()` on the web.
///
/// ### Syntactic Scope and Limitations
///
/// * **Targeted files**: Callers should pass files from crates that compile into the application's web build.
/// * **Allowed patterns**: Comments, string literals, lines containing `web_time`, and items decorated with
///   explicit non-wasm target conditions (e.g. `#[cfg(not(target_arch = "wasm32"))]` or `#[cfg(target_os = ...)]`)
///   are skipped.
/// * **Limitations**: This is a lexical source guard. It does not perform semantic type resolution, macro
///   expansion, or track arbitrary module aliases (e.g. `use std::time as t; t::Instant`).
pub fn std_instant<I>(files: I) -> Vec<Finding>
where
    I: IntoIterator,
    I::Item: AsFileSource,
{
    let mut findings = Vec::new();
    for file in files {
        let path = file.path();
        let content = file.content();
        let masked = mask_comments_and_strings(content);

        let mut in_cfg_non_wasm = false;
        let mut cfg_brace_depth: usize = 0;

        for (orig_line, (idx, masked_line)) in content.lines().zip(masked.lines().enumerate()) {
            let is_comment_only = masked_line.trim().is_empty();
            let line_has_cfg_non_wasm = !is_comment_only
                && (orig_line.contains("cfg(not(target_arch = \"wasm32\"))")
                    || orig_line.contains("cfg(target_arch = \"x86_64\")")
                    || orig_line.contains("cfg(target_arch = \"aarch64\")")
                    || orig_line.contains("cfg(target_os ="));

            if line_has_cfg_non_wasm {
                in_cfg_non_wasm = true;
            }

            let names_std_instant =
                masked_line.contains("time::Instant") || (masked_line.contains("use std::time::") && masked_line.contains("Instant"));

            if names_std_instant && !masked_line.contains("web_time") && !in_cfg_non_wasm {
                findings.push(Finding::new(path, idx + 1, orig_line.trim()));
            }

            if in_cfg_non_wasm && !is_comment_only {
                let opens = masked_line.chars().filter(|&c| c == '{').count();
                let closes = masked_line.chars().filter(|&c| c == '}').count();
                if cfg_brace_depth == 0 {
                    if opens > 0 {
                        cfg_brace_depth = cfg_brace_depth.saturating_add(opens).saturating_sub(closes);
                        if cfg_brace_depth == 0 {
                            in_cfg_non_wasm = false;
                        }
                    } else if masked_line.contains(';') {
                        in_cfg_non_wasm = false;
                    }
                } else {
                    cfg_brace_depth = cfg_brace_depth.saturating_add(opens).saturating_sub(closes);
                    if cfg_brace_depth == 0 {
                        in_cfg_non_wasm = false;
                    }
                }
            }
        }
    }
    findings
}

/// Guard: scans a single WGSL shader string and detects decimal float literals outside `f32`'s finite range.
///
/// Browser WebGPU compilers (such as Chrome's Tint) reject float literals exceeding `f32::MAX`
/// (e.g. `3.40282347e38`), triggering fatal pipeline build errors and fallback to CPU rendering.
/// Exact values equal to `f32::MAX` should be written as hex float literals (e.g. `0x1.fffffep+127f`).
///
/// ### Policy and Limitations
///
/// * **Conservative mathematical bound**: The guard evaluates the parsed mathematical magnitude against
///   `f64::from(f32::MAX)`. Any decimal literal exceeding this threshold is flagged, even if native
///   compilers like `naga` would round it down to `f32::MAX`.
/// * **Hexadecimal floats**: Exact hex float literals (`0x...p...`) are skipped as they represent exact bit patterns.
/// * **Comments and strings**: Comments (`//` and `/* ... */`) and string contents are masked out prior to scanning.
pub fn wgsl_float_literals(wgsl: &str) -> Vec<Finding> {
    wgsl_float_literals_in([("", wgsl)])
}

/// Guard: scans multiple WGSL shader files and detects decimal float literals outside `f32`'s finite range.
pub fn wgsl_float_literals_in<I>(files: I) -> Vec<Finding>
where
    I: IntoIterator,
    I::Item: AsFileSource,
{
    let mut findings = Vec::new();
    for file in files {
        let path = file.path();
        let content = file.content();
        let masked = mask_comments_and_strings(content);

        for (n, line) in masked.lines().enumerate() {
            let code = line.as_bytes();
            let mut i = 0;
            while i < code.len() {
                let digit_or_dot = code[i].is_ascii_digit() || (code[i] == b'.' && code.get(i + 1).is_some_and(u8::is_ascii_digit));
                let word_before = i > 0 && (code[i - 1].is_ascii_alphanumeric() || code[i - 1] == b'_' || code[i - 1] == b'.');
                if !digit_or_dot || word_before {
                    i += 1;
                    continue;
                }
                // Skip hex float literals (e.g. 0x1.fffffep+127f, 0x1.0p0h)
                if code[i] == b'0' && matches!(code.get(i + 1), Some(b'x' | b'X')) {
                    i += 2;
                    while i < code.len() && (code[i].is_ascii_hexdigit() || matches!(code[i], b'.' | b'p' | b'P' | b'+' | b'-' | b'f' | b'h')) {
                        i += 1;
                    }
                    continue;
                }
                let start = i;
                while i < code.len() && (code[i].is_ascii_digit() || code[i] == b'.') {
                    i += 1;
                }
                if i < code.len() && matches!(code[i], b'e' | b'E') {
                    let mut j = i + 1;
                    if j < code.len() && matches!(code[j], b'+' | b'-') {
                        j += 1;
                    }
                    if j < code.len() && code[j].is_ascii_digit() {
                        i = j;
                        while i < code.len() && code[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                }
                let literal = String::from_utf8_lossy(&code[start..i]);
                if i < code.len() && matches!(code[i], b'f' | b'h') {
                    i += 1;
                }
                let is_overflow = match literal.parse::<f64>() {
                    Ok(v) => !v.is_finite() || v.abs() > f64::from(f32::MAX),
                    Err(_) => true,
                };
                if is_overflow {
                    findings.push(Finding::new(path, n + 1, literal.into_owned()));
                }
            }
        }
    }
    findings
}

/// Recursively collect all Rust source files under `dir`, excluding test files and test directories.
///
/// Excluded files:
/// * `tests.rs`
/// * `tests_*.rs`
/// * `*_tests.rs`
/// * `*_test.rs`
/// * `test.rs`
/// * Any file within a directory component named `tests`, `benches`, `examples`, `.git`, or `target`.
///
/// Results are sorted lexicographically by file path for deterministic behavior across platforms.
#[cfg(not(target_arch = "wasm32"))]
pub fn rust_sources(dir: impl AsRef<Path>) -> std::io::Result<Vec<(String, String)>> {
    let root = dir.as_ref();
    let mut files = Vec::new();
    let mut dirs = vec![root.to_path_buf()];

    while let Some(d) = dirs.pop() {
        let entries = std::fs::read_dir(&d)?;
        for entry in entries {
            let entry = entry?;
            let p = entry.path();
            let metadata = std::fs::metadata(&p)?;
            if metadata.is_dir() {
                let dirname = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                if dirname.starts_with('.') || dirname == "target" || dirname == "tests" || dirname == "benches" || dirname == "examples" {
                    continue;
                }
                dirs.push(p);
            } else if metadata.is_file() {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                if !name.ends_with(".rs") {
                    continue;
                }
                if name == "tests.rs" || name.starts_with("tests_") || name.ends_with("_tests.rs") || name.ends_with("_test.rs") || name == "test.rs"
                {
                    continue;
                }
                if p.components().any(|c| c.as_os_str() == "tests") {
                    continue;
                }
                let content = std::fs::read_to_string(&p)?;
                files.push((p.display().to_string(), content));
            }
        }
    }

    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}
