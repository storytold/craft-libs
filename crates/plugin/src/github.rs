//! GitHub addresses of add-ins and the release asset that suits a platform. Nothing here touches
//! the network: downloading and unpacking are the app's.

/// Which version of a repository to install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Version {
    /// The latest release, else the default branch.
    Latest,
    /// A release (`…/releases/tag/<tag>`).
    Release(String),
    /// A branch, tag or commit's source (`…/tree/<ref>`).
    Tree(String),
}

/// A GitHub repository and the version to install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    pub owner: String,
    pub name: String,
    pub version: Version,
}

fn valid_segment(s: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && s != "." && s != ".." && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Reads `https://github.com/owner/repo[.git][/releases/tag/T | /releases/latest | /tree/REF]`,
/// with or without the scheme or `www.`, or plain `owner/repo`.
pub fn parse_github_url(url: &str) -> Result<Repo, String> {
    let bad = || format!("`{url}` isn't a GitHub repository address (https://github.com/owner/repo)");
    let s = url.trim().trim_end_matches('/');
    let rest = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")).unwrap_or(s);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    let path = match rest.strip_prefix("github.com/") {
        Some(p) => p,
        None if !s.contains("://") && rest.split('/').count() == 2 => rest,
        None => return Err(bad()),
    };
    let path = path.split(['?', '#']).next().unwrap_or("");
    let mut parts = path.splitn(3, '/');
    let owner = parts.next().unwrap_or("");
    let name = parts.next().unwrap_or("");
    let name = name.strip_suffix(".git").unwrap_or(name);
    if !valid_segment(owner) || !valid_segment(name) {
        return Err(bad());
    }
    let version = match parts.next() {
        None | Some("") | Some("releases") | Some("releases/latest") => Version::Latest,
        Some(more) => {
            if let Some(tag) = more.strip_prefix("releases/tag/") {
                Version::Release(tag.to_string())
            } else if let Some(r) = more.strip_prefix("tree/") {
                Version::Tree(r.to_string())
            } else {
                return Err(bad());
            }
        }
    };
    if let Version::Release(r) | Version::Tree(r) = &version
        && (r.is_empty() || r.len() > 255 || r.contains("..") || r.chars().any(|c| c.is_whitespace() || c.is_control()))
    {
        return Err(bad());
    }
    Ok(Repo { owner: owner.to_string(), name: name.to_string(), version })
}

fn os_tokens(os: &str) -> &'static [&'static str] {
    match os {
        "macos" => &["macos", "darwin", "mac", "osx", "apple"],
        "linux" => &["linux"],
        "windows" => &["windows", "win", "win64", "win32"],
        "freebsd" => &["freebsd"],
        _ => &[],
    }
}

fn arch_tokens(arch: &str) -> &'static [&'static str] {
    match arch {
        "aarch64" => &["aarch64", "arm64"],
        "x86_64" => &["x86_64", "x64", "amd64", "x86-64"],
        _ => &[],
    }
}

const ALL_OS: [&str; 4] = ["macos", "linux", "windows", "freebsd"];
const ALL_ARCH: [&str; 2] = ["aarch64", "x86_64"];

/// The release asset to install on `os`/`arch` (as in `std::env::consts`): a `.zip` naming this
/// platform (with this architecture, `universal`, or no architecture), else a `.zip` naming no
/// platform at all. `None`: no asset fits (install the source instead).
pub fn pick_asset(names: &[String], os: &str, arch: &str) -> Option<usize> {
    let mut best: Option<(u8, usize)> = None;
    for (i, name) in names.iter().enumerate() {
        let lower = name.to_ascii_lowercase();
        let Some(stem) = lower.strip_suffix(".zip") else { continue };
        let tokens: Vec<&str> = stem.split(['-', '.', ' ', '+']).collect();
        let has = |words: &[&str]| words.iter().any(|w| tokens.contains(w) || (w.contains(['_', '-']) && stem.contains(w)));
        let names_os = ALL_OS.iter().any(|o| has(os_tokens(o)));
        let names_arch = ALL_ARCH.iter().any(|a| has(arch_tokens(a)));
        let score = if !names_os {
            1
        } else if !has(os_tokens(os)) {
            continue;
        } else if has(arch_tokens(arch)) {
            4
        } else if tokens.contains(&"universal") || !names_arch {
            3
        } else {
            continue; // This platform, another architecture.
        };
        if best.is_none_or(|(s, _)| score > s) {
            best = Some((score, i));
        }
    }
    best.map(|(_, i)| i)
}

#[cfg(test)]
mod tests;
