//! Portable `sbs://` library URI validation and relative path resolution.

/// Validate an `sbs://` alias path without allowing traversal outside the library root.
pub fn relative_path(uri: &str) -> Result<&str, String> {
    let path = uri.strip_prefix("sbs://").ok_or("not an SBS library alias")?;
    if path.is_empty()
        || path.len() > 4096
        || path.contains(['\\', ':', '?', '#', '%', '\0'])
        || path.split('/').any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err("invalid SBS library path".into());
    }

    Ok(path)
}

/// Resolve a package-relative resource or dependency without leaving the library root.
pub fn resolve(owner: &str, reference: &str) -> Result<String, String> {
    let owner = relative_path(owner)?;
    if reference.starts_with("sbs://") {
        relative_path(reference)?;
        return Ok(reference.into());
    }

    let reference = reference.trim().replace('\\', "/");
    if reference.is_empty() || reference.len() > 4096 || reference.starts_with('/') || reference.contains([':', '?', '#', '%', '\0']) {
        return Err("invalid relative SBS library path".into());
    }

    let mut parts: Vec<_> = owner.split('/').collect();
    parts.pop();
    for part in reference.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err("SBS reference escapes its library directory".into());
                }
            }
            part => parts.push(part),
        }
    }

    let uri = format!("sbs://{}", parts.join("/"));
    relative_path(&uri)?;

    Ok(uri)
}

#[cfg(test)]
mod tests {
    use super::{relative_path, resolve};

    #[test]
    fn resolves_relative_and_rejects_escapes() {
        assert_eq!(resolve("sbs://filters/blur.sbs", "../noises/perlin.sbs").as_deref(), Ok("sbs://noises/perlin.sbs"));
        assert!(resolve("sbs://blur.sbs", "../escape.sbs").is_err());
        assert!(relative_path("sbs://../bad.sbs").is_err());
    }
}
