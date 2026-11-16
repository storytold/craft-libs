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
        if is_package(&bytes) {
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
    let mut by_name = std::collections::HashMap::<String, usize>::new();
    let mut too_many = false;
    let mut missing = Vec::new();
    let mut total = bytes.len() as u64;
    let mut exceeded = total > package_limits.max_total_bytes || total > package_limits.max_model_bytes;
    doc.diffuse_materials(limits, &mut |paths| {
        // candidates in order of preference: an earlier one that exists wins over a later cached one
        for path in paths {
            let Some(name) = below(path) else { continue };
            if let Some(data) = by_name.get(&name).and_then(|&i| files.get(i)).map(|(_, d)| d.clone()) {
                return Some(data);
            }
            // the model is an entry too; stop resolving once another texture could not fit
            if too_many || files.len() as u64 + 1 >= package_limits.max_entries {
                too_many = true;
                return None;
            }
            if let Some(data) = found(path).filter(|d| d.len() as u64 <= package_limits.max_texture_bytes) {
                total = total.saturating_add(data.len() as u64);
                if total > package_limits.max_total_bytes {
                    exceeded = true;
                    return None;
                }
                by_name.insert(name.clone(), files.len());
                files.push((name, data.clone()));
                return Some(data);
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
    if too_many {
        return Err(too_large(KIND, "too many package entries"));
    }
    let leaf = file_name.rsplit(['/', '\\']).next().filter(|n| !n.is_empty()).unwrap_or("mesh.fbx");
    // the package is found by its `.fbx` entry, so the model always carries that extension
    let leaf = if leaf.rsplit_once('.').is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("fbx")) { leaf.to_string() } else { format!("{leaf}.fbx") };
    let leaf = leaf.as_str();
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

    #[test]
    fn an_earlier_candidate_wins_over_a_cached_later_one() {
        let model = b"FBXHeaderExtension: { FBXVersion: 7400 } Objects: { Material: 1, \"Material::A\", \"\" { } Texture: 2, \"Texture::A\", \"\" { RelativeFilename: \"base.png\" } Material: 3, \"Material::B\", \"\" { } Texture: 4, \"Texture::B\", \"\" { RelativeFilename: \"maps/base.png\" FileName: \"C:/art/base.png\" } } Connections: { C: \"OP\",2,1,\"DiffuseColor\" C: \"OP\",4,3,\"DiffuseColor\" }";
        let limits = Limits::default();
        let doc = read_document(model, &limits).unwrap();
        let mut found = |p: &str| match p {
            "base.png" => Some(b"flat".to_vec()),
            "maps/base.png" => Some(b"maps".to_vec()),
            _ => None,
        };
        let (bytes, missing) = write(&doc, "asset.fbx", model, &limits, bounds(), &mut found).unwrap();
        assert!(missing.is_empty());
        let bytes = bytes.unwrap();
        let mut package = Package::open(&bytes, bounds()).unwrap();
        assert_eq!(package.texture(&["maps/base.png"]), Some(b"maps".to_vec()));
        assert_eq!(package.texture(&["base.png"]), Some(b"flat".to_vec()));
    }

    #[test]
    fn the_model_entry_always_has_an_fbx_extension() {
        let limits = Limits::default();
        let doc = read_document(MODEL, &limits).unwrap();
        let (bytes, _) = write(&doc, "dir/asset", MODEL, &limits, bounds(), &mut |_| Some(b"image".to_vec())).unwrap();
        let bytes = bytes.unwrap();
        let mut package = Package::open(&bytes, bounds()).unwrap();
        assert_eq!(package.model().unwrap(), MODEL);
    }

    #[test]
    fn texture_resolution_stops_at_the_entry_limit() {
        let mut model = String::from("FBXHeaderExtension: { FBXVersion: 7400 } Objects: {");
        let mut links = String::new();
        for i in 0..20 {
            model.push_str(&format!(
                " Material: {m}, \"Material::M{i}\", \"\" {{ }} Texture: {t}, \"Texture::T{i}\", \"\" {{ RelativeFilename: \"t{i}.png\" }}",
                m = 2 * i + 1,
                t = 2 * i + 2
            ));
            links.push_str(&format!(" C: \"OP\",{t},{m},\"DiffuseColor\"", m = 2 * i + 1, t = 2 * i + 2));
        }
        model.push_str(&format!(" }} Connections: {{{links} }}"));
        let limits = Limits::default();
        let doc = read_document(model.as_bytes(), &limits).unwrap();
        let mut calls = 0;
        let result = write(&doc, "a.fbx", model.as_bytes(), &limits, bounds(), &mut |_| {
            calls += 1;
            Some(b"image".to_vec())
        });
        assert!(matches!(result, Err(Error::TooLarge(_))));
        assert!(calls < bounds().max_entries as usize, "{calls} resolver calls past the entry limit");
    }
}
