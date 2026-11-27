//! Reading and checking one source's index file, and its URL templates.

use std::collections::{BTreeMap, HashSet};

use serde::Deserialize;

use crate::index::{
    Family, FontSource, GOOGLE_FONTS, Index, IndexError, LICENSES, Language, MAJOR_VERSION, MAX_FILE_BYTES, MAX_INDEX_BYTES, MAX_LICENSE_BYTES,
    Sample, Script,
};

pub(crate) const MAX_STR: usize = 1024;
const MAX_LIST: usize = 4096;
/// The most characters a file's own `sample_text` has.
const MAX_SAMPLE_CHARS: usize = 60;
const MAX_FAMILIES: usize = 50_000;
const MAX_FILES: usize = 200_000;
const MAX_SEGMENT: usize = 64;
const MAX_FILE_NAME: usize = 200;
const MAX_TEMPLATES: usize = 8;

/// Where an index file comes from.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Origin<'a> {
    /// A built-in source, or any file the app itself trusts: its `curated` flag is kept.
    Trusted,
    /// A source the user added by URL: never curated, its ID prefixed with `user:<host>/`.
    User { url: &'a str },
}

#[derive(Deserialize)]
struct RawSourceInfo {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    curated: bool,
    #[serde(default)]
    file_urls: Vec<String>,
}

#[derive(Deserialize)]
struct Raw {
    version: String,
    #[serde(default)]
    source: String,
    commit: String,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    fallback_base_url: Option<String>,
    #[serde(default)]
    source_info: Option<RawSourceInfo>,
    #[serde(default)]
    repo: Option<String>,
    #[serde(default, rename = "ref")]
    git_ref: Option<String>,
    #[serde(default)]
    languages: Vec<Language>,
    #[serde(default)]
    scripts: Vec<Script>,
    #[serde(default)]
    samples: BTreeMap<String, Sample>,
    families: Vec<Family>,
}

pub(crate) fn invalid<T>(msg: impl Into<String>) -> Result<T, IndexError> {
    Err(IndexError::Invalid(msg.into()))
}

fn short(s: &str) -> String {
    s.chars().take(120).collect()
}

pub(crate) fn check_version(version: &str) -> Result<(), IndexError> {
    let major = version.split_once('.').and_then(|(major, minor)| {
        let major: u32 = major.parse().ok()?;
        minor.parse::<u32>().ok()?;
        Some(major)
    });
    if major == Some(MAJOR_VERSION) { Ok(()) } else { Err(IndexError::Version(version.chars().take(40).collect())) }
}

/// Direction marks, embeddings and isolates, which can reorder the text around them.
fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{61c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

pub(crate) fn check_str(what: &str, s: &str) -> Result<(), IndexError> {
    if s.len() > MAX_STR || s.chars().any(|c| c.is_control() || is_bidi_control(c)) {
        return invalid(format!("{what} is too long or has control characters"));
    }
    Ok(())
}

pub(crate) fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A git ref or a repository part: `[A-Za-z0-9._-]+`, not starting with `.`.
pub(crate) fn is_ref(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && !s.starts_with('.') && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// `owner/name` on GitHub.
pub(crate) fn is_repo(s: &str) -> bool {
    s.split_once('/').is_some_and(|(o, n)| is_ref(o) && is_ref(n))
}

fn check_repo_ref(what: &str, repo: Option<&str>, git_ref: Option<&str>) -> Result<(), IndexError> {
    if let Some(r) = repo.filter(|r| !is_repo(r)) {
        return invalid(format!("{what} has repo `{}`, not owner/name", short(r)));
    }
    if let Some(r) = git_ref.filter(|r| !is_ref(r)) {
        return invalid(format!("{what} has ref `{}`", short(r)));
    }
    Ok(())
}

/// A file name the store can write on every platform.
pub(crate) fn safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_FILE_NAME
        && !name.starts_with('.')
        && !name.ends_with('.')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-[],.+".contains(&b))
}

fn safe_segment(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_SEGMENT && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// A family folder in the store: google/fonts' `[a-z0-9_-]+`, or `<source>/<family>` of the same
/// characters.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn safe_dir(dir: &str) -> bool {
    let mut parts = dir.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(a), None, None) => safe_segment(a),
        (Some(a), Some(b), None) => safe_segment(a) && safe_segment(b),
        _ => false,
    }
}

/// Lowercase letters and digits, other runs as one `-`: `notofonts`, `google-fonts`,
/// `noto-sans-cjk-jp`. A name with no ASCII letters or digits gets a hash of itself.
pub(crate) fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.truncate(MAX_SEGMENT);
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { crate::verify::sha256_hex(s.as_bytes()).chars().take(16).collect() } else { out }
}

/// A user source's folder name: its ID's slug and a hash of the whole ID, since IDs that differ
/// only in punctuation, case or past the slug's length share a slug.
fn user_slug(id: &str) -> String {
    let hash: String = crate::verify::sha256_hex(id.as_bytes()).chars().take(8).collect();
    let mut s = slug(id);
    s.truncate(MAX_SEGMENT - hash.len() - 1);
    format!("{}-{hash}", s.trim_end_matches('-'))
}

/// A relative path in a repository: `/`-separated, no empty, `.` or `..` segment, no `\`, and a
/// file name the store can write.
pub(crate) fn check_path(path: &str) -> Result<&str, IndexError> {
    let bad = path.is_empty()
        || path.len() > MAX_STR
        || path.contains('\\')
        || path.chars().any(char::is_control)
        || path.split('/').any(|s| s.is_empty() || s == "." || s == "..");
    let name = path.rsplit('/').next().unwrap_or(path);
    if bad || !safe_file_name(name) { invalid(format!("`{}` is not a safe relative path", short(path))) } else { Ok(name) }
}

/// Percent-encodes each `/`-separated segment, keeping the unreserved characters.
pub(crate) fn encode_path(p: &str) -> String {
    let mut out = String::with_capacity(p.len() + 8);
    for b in p.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn check_template(t: &str) -> Result<(), IndexError> {
    let rest = t.replace("{path}", "").replace("{repo}", "").replace("{ref}", "");
    let ok = t.len() <= MAX_STR
        && t.starts_with("https://")
        && t.len() > "https://".len()
        && t.contains("{path}")
        && !rest.contains(['{', '}', '\\'])
        && !t.chars().any(|c| c.is_control() || c.is_whitespace());
    if ok { Ok(()) } else { invalid(format!("file URL template `{}` is not an https URL with {{path}}", short(t))) }
}

fn check_base_url(u: &str) -> Result<(), IndexError> {
    let ok = u.len() <= MAX_STR
        && u.len() > "https://".len()
        && u.starts_with("https://")
        && u.ends_with('/')
        && !u.chars().any(|c| c.is_control() || c.is_whitespace() || matches!(c, '\\' | '{' | '}'));
    if ok { Ok(()) } else { invalid(format!("base URL `{}` is not an https folder URL", short(u))) }
}

/// The URLs of `path`, filling each template in turn. `repo` and `ref` are needed only when a
/// template names them.
pub(crate) fn fill(templates: &[String], path: &str, repo: Option<&str>, git_ref: Option<&str>) -> Result<Vec<String>, IndexError> {
    check_path(path)?;
    let encoded = encode_path(path);
    let mut urls = Vec::with_capacity(templates.len());
    for t in templates {
        let mut url = t.replace("{path}", &encoded);
        for (key, value, ok) in [("{repo}", repo, is_repo as fn(&str) -> bool), ("{ref}", git_ref, is_ref)] {
            if url.contains(key) {
                match value {
                    Some(v) if ok(v) => url = url.replace(key, v),
                    _ => return invalid(format!("`{}` has no valid {key} for its URL", short(path))),
                }
            }
        }
        if !url.starts_with("https://") || url.contains('{') {
            return invalid(format!("`{}` doesn't resolve to an https URL", short(path)));
        }
        urls.push(url);
    }
    Ok(urls)
}

/// The host of an https URL, lowercased: `fonts.example.org`.
pub(crate) fn https_host(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?.to_ascii_lowercase();
    let ok = !host.is_empty() && host.len() <= 253 && host.bytes().all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b));
    ok.then_some(host)
}

/// Whether `url` is an https URL a user may add as a source.
pub(crate) fn is_source_url(url: &str) -> bool {
    url.len() <= MAX_STR && https_host(url).is_some() && !url.chars().any(|c| c.is_control() || c.is_whitespace() || c == '\\')
}

/// What [`check_family`] needs to know about the source.
struct Ctx<'a> {
    /// The ID the file declares.
    declared: &'a str,
    /// The ID the app uses (prefixed for a user source).
    id: &'a str,
    slug: &'a str,
    google: bool,
    curated: bool,
    templates: &'a [String],
    repo: Option<&'a str>,
    git_ref: Option<&'a str>,
    languages: usize,
}

/// Reads one source's index file.
pub(crate) fn parse(bytes: &[u8], origin: Origin<'_>) -> Result<Index, IndexError> {
    if bytes.len() > MAX_INDEX_BYTES {
        return Err(IndexError::TooLarge { max: MAX_INDEX_BYTES });
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| IndexError::Json(e.to_string()))?;
    // The version first: a newer major version may change any other field.
    let version = value.get("version").and_then(|v| v.as_str()).ok_or_else(|| IndexError::Version("(none)".into()))?;
    check_version(version)?;
    let raw: Raw = serde_json::from_value(value).map_err(|e| IndexError::Json(e.to_string()))?;
    check(raw, origin)
}

fn check(raw: Raw, origin: Origin<'_>) -> Result<Index, IndexError> {
    check_str("commit", &raw.commit)?;
    if raw.commit.len() != 40 || !raw.commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return invalid(format!("commit `{}` is not 40 hex characters", short(&raw.commit)));
    }
    let info_id = raw.source_info.as_ref().map(|i| i.id.as_str()).unwrap_or_default();
    let declared = match (raw.source.as_str(), info_id) {
        ("", "") => GOOGLE_FONTS,
        (s, "") | ("", s) => s,
        (s, i) if s == i => s,
        _ => return invalid("source and source_info.id differ"),
    }
    .to_string();
    if !declared.split('/').all(is_ref) || declared.len() > 128 {
        return invalid(format!("source `{}` is not a valid ID", short(&declared)));
    }
    let host = match origin {
        Origin::Trusted => None,
        Origin::User { url } => Some(https_host(url).ok_or_else(|| IndexError::Invalid("the source's URL is not https".into()))?),
    };
    let id = host.as_ref().map_or_else(|| declared.clone(), |h| format!("user:{h}/{declared}"));
    for u in raw.base_url.iter().chain(&raw.fallback_base_url) {
        check_base_url(u)?;
    }
    check_repo_ref("the index", raw.repo.as_deref(), raw.git_ref.as_deref())?;
    let (name, homepage, curated, templates) = match &raw.source_info {
        Some(info) => {
            check_str("source name", &info.name)?;
            if info.file_urls.is_empty() || info.file_urls.len() > MAX_TEMPLATES {
                return invalid("source_info.file_urls must list 1 to 8 templates");
            }
            for t in &info.file_urls {
                check_template(t)?;
            }
            let homepage = info.homepage.clone().filter(|h| is_source_url(h));
            let name = if info.name.trim().is_empty() { declared.clone() } else { info.name.clone() };
            (name, homepage, info.curated, info.file_urls.clone())
        }
        None => {
            let Some(base) = &raw.base_url else { return invalid("an index without source_info needs base_url") };
            let templates = std::iter::once(base).chain(&raw.fallback_base_url).map(|b| format!("{b}{{path}}")).collect();
            let name = host.clone().unwrap_or_else(|| "Google Fonts".into());
            (name, None, true, templates)
        }
    };
    let curated = curated && host.is_none();
    if raw.families.len() > MAX_FAMILIES || raw.languages.len() > MAX_FAMILIES * 4 || raw.scripts.len() > MAX_LIST || raw.samples.len() > MAX_LIST {
        return invalid("too many families, languages, scripts or samples");
    }
    for (script, sample) in &raw.samples {
        for s in [script, &sample.method, &sample.language] {
            check_str("sample", s)?;
        }
        // Some samples are several lines (Nüshu, Carian).
        if sample.text.len() > MAX_STR || sample.text.chars().any(|c| c.is_control() && c != '\n') {
            return invalid(format!("the `{}` sample is too long or has control characters", short(script)));
        }
    }
    for l in &raw.languages {
        check_str("language", &l.id)?;
    }
    for s in &raw.scripts {
        check_str("script", &s.id)?;
    }
    let slug = if host.is_some() { user_slug(&id) } else { slug(&id) };
    let ctx = Ctx {
        declared: &declared,
        id: &id,
        slug: &slug,
        google: host.is_none() && declared == GOOGLE_FONTS,
        curated,
        templates: &templates,
        repo: raw.repo.as_deref(),
        git_ref: raw.git_ref.as_deref(),
        languages: raw.languages.len(),
    };
    let mut warnings = Vec::new();
    let mut families = Vec::with_capacity(raw.families.len());
    let mut names = HashSet::new();
    let mut dirs = HashSet::new();
    let mut total = 0usize;
    for mut fam in raw.families {
        check_str("license", &fam.license)?;
        if !LICENSES.contains(&fam.license.as_str()) {
            warnings.push(format!("{} is left out: its licence `{}` doesn't allow redistribution", short(&fam.family), short(&fam.license)));
            continue;
        }
        check_family(&mut fam, &ctx)?;
        total = total.saturating_add(fam.files.len());
        if total > MAX_FILES {
            return invalid("too many files");
        }
        if !names.insert(fam.family.to_lowercase()) {
            return invalid(format!("family `{}` is listed twice", fam.family));
        }
        if !dirs.insert(fam.dir.clone()) {
            return invalid(format!("folder `{}` holds more than one family", fam.dir));
        }
        families.push(fam);
    }
    let source = FontSource {
        id,
        name,
        homepage,
        curated,
        version: raw.version,
        commit: raw.commit,
        user_url: match origin {
            Origin::Trusted => None,
            Origin::User { url } => Some(url.to_string()),
        },
        changed: false,
        warnings,
        families: families.len(),
        templates,
        repo: raw.repo,
        git_ref: raw.git_ref,
    };
    Ok(Index::assemble(vec![(source, raw.languages, raw.scripts, raw.samples, families)], Vec::new()))
}

/// The family folder in google/fonts: every file at `<ofl|apache|ufl>/<dir>/<file>`.
fn google_dir(fam: &Family) -> Result<String, IndexError> {
    let dir_of = |path: &str| -> Option<String> {
        let mut parts = path.split('/');
        match (parts.next(), parts.next(), parts.next(), parts.next()) {
            (Some(lic), Some(dir), Some(_), None) if ["ofl", "apache", "ufl"].contains(&lic) && safe_segment(dir) => Some(dir.to_string()),
            _ => None,
        }
    };
    let mut dir: Option<String> = None;
    for path in fam.files.iter().map(|f| f.path.as_str()).chain(fam.license_path.as_deref()) {
        let d = dir_of(path).ok_or_else(|| IndexError::Invalid(format!("`{}` is not <ofl|apache|ufl>/<folder>/<file>", short(path))))?;
        match &dir {
            None => dir = Some(d),
            Some(x) if *x == d => {}
            Some(_) => return invalid(format!("the files of `{}` are in different folders", fam.family)),
        }
    }
    dir.ok_or_else(|| IndexError::Invalid(format!("`{}` has no files", fam.family)))
}

fn check_family(fam: &mut Family, ctx: &Ctx<'_>) -> Result<(), IndexError> {
    check_str("family name", &fam.family)?;
    if fam.family.trim().is_empty() {
        return invalid("a family has no name");
    }
    let name = fam.family.clone();
    if !fam.source.is_empty() && fam.source != ctx.declared {
        return invalid(format!("`{name}` is from source `{}`, not the index's", short(&fam.source)));
    }
    fam.source = ctx.id.to_string();
    fam.curated = ctx.curated;
    check_str("designer", &fam.designer)?;
    if fam.files.is_empty() {
        return invalid(format!("`{name}` has no files"));
    }
    let lists =
        [fam.categories.len(), fam.subsets.len(), fam.languages.len(), fam.scripts.len(), fam.files.len(), fam.tags.len(), fam.reserved_names.len()];
    if lists.iter().any(|&n| n > MAX_LIST) {
        return invalid(format!("`{name}` has too many entries"));
    }
    for s in fam.categories.iter().chain(&fam.subsets).chain(&fam.scripts) {
        check_str("category, subset or script", s)?;
    }
    for s in fam.tags.iter().chain(&fam.reserved_names).chain(&fam.sample_script).chain(&fam.copyright) {
        check_str("tag, reserved name, sample script or copyright", s)?;
    }
    if fam.upstream.as_deref().is_some_and(|u| !is_source_url(u)) {
        return invalid(format!("the upstream of `{name}` is not an https URL"));
    }
    if fam.languages.iter().any(|&i| i >= ctx.languages) {
        return invalid(format!("`{name}` names a language the index doesn't have"));
    }
    check_repo_ref(&name, fam.repo.as_deref(), fam.git_ref.as_deref())?;
    let (fam_repo, fam_ref) = (fam.repo.clone(), fam.git_ref.clone());
    let repo = |own: Option<String>| own.or_else(|| fam_repo.clone()).or_else(|| ctx.repo.map(str::to_string));
    let git_ref = |own: Option<String>| own.or_else(|| fam_ref.clone()).or_else(|| ctx.git_ref.map(str::to_string));
    let mut names = HashSet::new();
    for f in &fam.files {
        let file = check_path(&f.path)?.to_ascii_lowercase();
        if ![".ttf", ".otf", ".ttc", ".otc"].iter().any(|ext| file.ends_with(ext)) {
            return invalid(format!("`{}` is not a font file", short(&f.path)));
        }
        if !names.insert(file) {
            return invalid(format!("`{}` is listed twice", short(&f.path)));
        }
        if !is_sha256(&f.sha256) {
            return invalid(format!("`{}` has a bad sha256", short(&f.path)));
        }
        if f.size == 0 || f.size > MAX_FILE_BYTES {
            return invalid(format!("`{}` has size {}", short(&f.path), f.size));
        }
        if !(1..=1000).contains(&f.weight) {
            return invalid(format!("`{}` has weight {}", short(&f.path), f.weight));
        }
        if !(f.width.is_finite() && f.width > 0.0 && f.width <= 1000.0) {
            return invalid(format!("`{}` has width {}", short(&f.path), f.width));
        }
        check_str("style", &f.style)?;
        if f.axes.len() > 64 || f.postscript.len() > MAX_LIST {
            return invalid(format!("`{}` has too many axes or PostScript names", short(&f.path)));
        }
        for a in &f.axes {
            if a.tag.len() != 4 || !a.tag.is_ascii() || !a.min.is_finite() || !a.max.is_finite() || a.min > a.max {
                return invalid(format!("`{}` has a bad axis", short(&f.path)));
            }
        }
        if (!f.instances.is_empty() && f.axes.is_empty()) || f.instances.len() > MAX_LIST || f.list_instance.is_some_and(|i| i >= f.instances.len()) {
            return invalid(format!("`{}` has a bad list instance", short(&f.path)));
        }
        // Every instance has a coordinate for each axis and for no other.
        for s in f.sample_script.iter().chain(&f.sample_text) {
            check_str("sample script or text", s)?;
        }
        if f.sample_text.as_deref().is_some_and(|t| t.chars().count() > MAX_SAMPLE_CHARS) {
            return invalid(format!("`{}` has a sample text over {MAX_SAMPLE_CHARS} characters", short(&f.path)));
        }
        for i in &f.instances {
            check_str("instance name", &i.name)?;
            if i.coordinates.len() != f.axes.len() || f.axes.iter().any(|a| !i.coordinates.contains_key(&a.tag)) {
                return invalid(format!("`{}` has an instance without its axes", short(&f.path)));
            }
            let in_range = |a: &crate::index::Axis| i.coordinates.get(&a.tag).is_some_and(|c| c.is_finite() && (a.min..=a.max).contains(c));
            if !f.axes.iter().all(in_range) {
                return invalid(format!("`{}` has an instance outside its axes' ranges", short(&f.path)));
            }
        }
        for ps in &f.postscript {
            check_str("PostScript name", ps)?;
            if ps.is_empty() {
                return invalid(format!("`{}` has an empty PostScript name", short(&f.path)));
            }
        }
        check_repo_ref(&f.path, f.repo.as_deref(), f.git_ref.as_deref())?;
        fill(ctx.templates, &f.path, repo(f.repo.clone()).as_deref(), git_ref(f.git_ref.clone()).as_deref())?;
    }
    let beside = |what: &str, path: &str| -> Result<(), IndexError> {
        if names.contains(&check_path(path)?.to_ascii_lowercase()) {
            return invalid(format!("the {what} of `{name}` has a font file's name"));
        }
        Ok(())
    };
    // The licence and the NOTICE file are checked alike.
    for (what, lf) in [("licence", &fam.license_file), ("notice", &fam.notice_file)] {
        let Some(lf) = lf else { continue };
        beside(what, &lf.path)?;
        if !is_sha256(&lf.sha256) || lf.size == 0 || lf.size > MAX_LICENSE_BYTES {
            return invalid(format!("the {what} of `{name}` has a bad sha256 or size"));
        }
        check_repo_ref(&lf.path, lf.repo.as_deref(), lf.git_ref.as_deref())?;
        fill(ctx.templates, &lf.path, repo(lf.repo.clone()).as_deref(), git_ref(lf.git_ref.clone()).as_deref())?;
    }
    if let Some(lp) = &fam.license_path {
        beside("licence", lp)?;
        if fam.license_file.is_none() {
            fill(ctx.templates, lp, repo(None).as_deref(), git_ref(None).as_deref())?;
        }
    }
    fam.dir = if ctx.google { google_dir(fam)? } else { format!("{}/{}", ctx.slug, slug(&fam.family)) };
    Ok(())
}
