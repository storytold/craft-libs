//! FBX and referenced texture files in a ZIP container.
use super::{Document, KIND};
use crate::{Error, Limits, Result, malformed, too_large};

const ZIP_MAGIC: &[u8] = b"PK\x03\x04";

/// Whether bytes begin with a ZIP local-file header.
pub fn is_package(bytes: &[u8]) -> bool {
    bytes.starts_with(ZIP_MAGIC)
}

/// Limits for an FBX package and each referenced texture.
#[derive(Clone, Copy, Debug)]
pub struct PackageLimits {
    pub max_entries: u64,
    pub max_model_bytes: u64,
    pub max_texture_bytes: u64,
    pub max_total_bytes: u64,
}

/// An open FBX package. Reads are charged to its cumulative decompression budget.
pub struct Package<'a> {
    zip: craft_zip::Archive<'a>,
    limits: PackageLimits,
    name: String,
}

impl<'a> Package<'a> {
    pub fn open(bytes: &'a [u8], limits: PackageLimits) -> Result<Self> {
        let zip = craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: limits.max_entries, max_total_bytes: limits.max_total_bytes })
            .map_err(|e| malformed(KIND, format!("package: {e}")))?;
        let name = zip
            .names()
            .find(|n| n.rsplit('.').next().is_some_and(|ext| ext.eq_ignore_ascii_case("fbx")))
            .map(|n| n.into_owned())
            .ok_or_else(|| malformed(KIND, "the package contains no FBX file"))?;
        Ok(Self { zip, limits, name })
    }

    pub fn model(&mut self) -> Result<Vec<u8>> {
        let bytes = self
            .zip
            .read(&self.name, self.limits.max_model_bytes)
            .map_err(|e| malformed(KIND, format!("package: {e}")))?
            .ok_or_else(|| malformed(KIND, "package FBX is missing"))?;
        if is_package(bytes) {
            return Err(malformed(KIND, "the package holds another package"));
        }
        Ok(bytes)
    }

    /// Resolve candidate names within the package; outside paths are never read.
    pub fn texture(&mut self, paths: &[&str]) -> Option<Vec<u8>> {
        paths.iter().filter_map(|p| below(p)).find_map(|name| self.zip.read(&name, self.limits.max_texture_bytes).ok().flatten())
    }
}

/// Resolve a safe relative path inside a package.
fn below(path: &str) -> Option<String> {
    let path = path.replace('\\', "/");
    if path.starts_with('/') || path.contains(':') {
        return None;
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Package the FBX and its externally resolved diffuse textures, if any. Returns unresolved paths.
pub fn write(
    doc: &Document,
    file_name: &str,
    bytes: &[u8],
    limits: &Limits,
    package_limits: PackageLimits,
    found: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
) -> Result<(Option<Vec<u8>>, Vec<String>)> {
    if is_package(bytes) {
        return Ok((None, Vec::new()));
    }
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut missing = Vec::new();
    let mut total = bytes.len() as u64;
    let mut exceeded = total > package_limits.max_total_bytes || total > package_limits.max_model_bytes;
    doc.diffuse_materials(limits, &mut |paths| {
        if let Some((_, data)) = files.iter().find(|(n, _)| paths.iter().any(|p| below(p).as_ref() == Some(n))) {
            return Some(data.clone());
        }
        for path in paths {
            if let Some(name) = below(path) {
                if let Some(data) = found(path).filter(|d| d.len() as u64 <= package_limits.max_texture_bytes) {
                    total = total.saturating_add(data.len() as u64);
                    if total > package_limits.max_total_bytes {
                        exceeded = true;
                        return None;
                    }
                    files.push((name, data.clone()));
                    return Some(data);
                }
            }
        }
        if let Some(first) = paths.first().filter(|p| !missing.iter().any(|m| m == **p)) {
            missing.push((*first).to_string());
        }
        None
    })?;
    if exceeded {
        return Err(too_large(KIND, "the FBX and its textures exceed the package limit"));
    }
    if files.is_empty() {
        return Ok((None, missing));
    }
    if files.len() as u64 >= package_limits.max_entries {
        return Err(too_large(KIND, "too many package entries"));
    }
    let leaf = file_name.rsplit(['/', '\\']).next().filter(|n| !n.is_empty()).unwrap_or("mesh.fbx");
    let mut writer = craft_zip::Writer::new();
    for (name, data) in std::iter::once((leaf, bytes)).chain(files.iter().map(|(n, b)| (n.as_str(), b.as_slice()))) {
        writer.add(name, data, craft_zip::Compression::Stored).map_err(|e| Error::Malformed(format!("FBX package: {e}")))?;
    }
    Ok((Some(writer.finish().map_err(|e| Error::Malformed(format!("FBX package: {e}")))?), missing))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fbx::read_document;

    const MODEL: &[u8] = b"FBXHeaderExtension: { FBXVersion: 7400 } Objects: { Material: 1, \"Material::M\", \"\" { } Texture: 2, \"Texture::T\", \"\" { RelativeFilename: \"tex/a.png\" } } Connections: { C: \"OP\",2,1,\"DiffuseColor\" }";
    fn bounds() -> PackageLimits {
        PackageLimits { max_entries: 4, max_model_bytes: 4096, max_texture_bytes: 32, max_total_bytes: 8192 }
    }
    #[test]
    fn package_roundtrip_and_limits() {
        let limits = Limits::default();
        let doc = read_document(MODEL, &limits).unwrap();
        let (bytes, missing) = write(&doc, "asset.fbx", MODEL, &limits, bounds(), &mut |p| (p == "tex/a.png").then(|| b"image".to_vec())).unwrap();
        assert!(missing.is_empty());
        let bytes = bytes.unwrap();
        let mut package = Package::open(&bytes, bounds()).unwrap();
        assert_eq!(package.model().unwrap(), MODEL);
        assert_eq!(package.texture(&["../tex/a.png", "tex/a.png"]), Some(b"image".to_vec()));
        assert!(package.texture(&["../tex/a.png"]).is_none());
        let small = PackageLimits { max_total_bytes: MODEL.len() as u64 + 4, ..bounds() };
        assert!(matches!(write(&doc, "a.fbx", MODEL, &limits, small, &mut |_| Some(b"image".to_vec())), Err(Error::TooLarge(_))));
        assert!(Package::open(&bytes, PackageLimits { max_entries: 1, ..bounds() }).is_err());
    }
}
