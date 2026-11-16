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
    /// Textures read so far by name: a texture several materials share is read, and charged, once.
    textures: std::collections::HashMap<String, Vec<u8>>,
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
        Ok(Self { zip, limits, name, textures: std::collections::HashMap::new() })
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
        for name in paths.iter().filter_map(|p| below(p)) {
            if let Some(data) = self.textures.get(&name) {
                return Some(data.clone());
            }
            if let Some(data) = self.zip.read(&name, self.limits.max_texture_bytes).ok().flatten() {
                self.textures.insert(name, data.clone());
                return Some(data);
            }
        }
        None
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
    let mut missing_seen = std::collections::HashSet::<String>::new();
    let mut total = bytes.len() as u64;
    if total > package_limits.max_total_bytes || total > package_limits.max_model_bytes {
        return Err(too_large(KIND, "the FBX and its textures exceed the package limit"));
    }
    let mut exceeded = false;
    doc.diffuse_materials(limits, &mut |paths| {
        // candidates in order of preference: an earlier one that exists wins over a later cached one
        for path in paths {
            let Some(name) = below(path) else { continue };
            if let Some(data) = by_name.get(&name).and_then(|&i| files.get(i)).map(|(_, d)| d.clone()) {
                return Some(data);
            }
            // once a resolved texture has not fit, the package fails: stop calling the resolver
            if too_many || exceeded {
                return None;
            }
            if let Some(data) = found(path).filter(|d| d.len() as u64 <= package_limits.max_texture_bytes) {
                // the model is an entry too; only a texture that resolves needs a new entry
                if files.len() as u64 + 1 >= package_limits.max_entries {
                    too_many = true;
                    return None;
                }
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
        if let Some(first) = paths.first().filter(|p| missing_seen.insert((**p).to_string())) {
            missing.push((*first).to_string());
        }
        None
    })?;
    if exceeded {
        return Err(too_large(KIND, "the FBX and its textures exceed the package limit"));
    }
    if too_many {
        return Err(too_large(KIND, "too many package entries"));
    }
    if files.is_empty() {
        return Ok((None, missing));
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
        assert!(calls <= bounds().max_entries as usize, "{calls} resolver calls past the entry limit");
    }

    #[test]
    fn resolution_stops_once_the_byte_limit_is_passed() {
        let mut model = String::from("FBXHeaderExtension: { FBXVersion: 7400 } Objects: {");
        let mut links = String::new();
        for i in 0..10 {
            model.push_str(&format!(
                " Material: {m}, \"Material::M{i}\", \"\" {{ }} Texture: {t}, \"Texture::T{i}\", \"\" {{ RelativeFilename: \"big.png\" }}",
                m = 2 * i + 1,
                t = 2 * i + 2
            ));
            links.push_str(&format!(" C: \"OP\",{t},{m},\"DiffuseColor\"", m = 2 * i + 1, t = 2 * i + 2));
        }
        model.push_str(&format!(" }} Connections: {{{links} }}"));
        let limits = Limits::default();
        let doc = read_document(model.as_bytes(), &limits).unwrap();
        let tight = PackageLimits { max_total_bytes: model.len() as u64 + 4, max_model_bytes: 1 << 20, ..bounds() };
        let mut calls = 0;
        let result = write(&doc, "a.fbx", model.as_bytes(), &limits, tight, &mut |_| {
            calls += 1;
            Some(b"image".to_vec())
        });
        assert!(matches!(result, Err(Error::TooLarge(_))));
        assert_eq!(calls, 1, "one texture past the limit ends resolution");
        // a model past its own limit is refused before any texture is resolved
        let small = PackageLimits { max_model_bytes: 8, ..bounds() };
        let mut calls = 0;
        assert!(matches!(
            write(&doc, "a.fbx", model.as_bytes(), &limits, small, &mut |_| {
                calls += 1;
                None
            }),
            Err(Error::TooLarge(_))
        ));
        assert_eq!(calls, 0);
    }

    #[test]
    fn the_entry_limit_counts_only_textures_that_resolve() {
        let limits = Limits::default();
        let doc = read_document(MODEL, &limits).unwrap();
        // the model alone fills a one-entry package, so a resolved texture cannot fit
        let one = PackageLimits { max_entries: 1, ..bounds() };
        assert!(matches!(write(&doc, "a.fbx", MODEL, &limits, one, &mut |_| Some(b"image".to_vec())), Err(Error::TooLarge(_))));
        // an unresolved texture adds no entry, so it is reported missing rather than overflowing
        let (bytes, missing) = write(&doc, "a.fbx", MODEL, &limits, one, &mut |_| None).unwrap();
        assert!(bytes.is_none());
        assert_eq!(missing, vec!["tex/a.png".to_string()]);
    }

    #[test]
    fn a_shared_texture_is_read_and_charged_once() {
        let limits = Limits::default();
        let doc = read_document(MODEL, &limits).unwrap();
        let (bytes, _) = write(&doc, "asset.fbx", MODEL, &limits, bounds(), &mut |_| Some(b"image".to_vec())).unwrap();
        let bytes = bytes.unwrap();
        // room for the model and one read of the texture
        let tight = PackageLimits { max_total_bytes: MODEL.len() as u64 + 5, ..bounds() };
        let mut package = Package::open(&bytes, tight).unwrap();
        package.model().unwrap();
        assert_eq!(package.texture(&["tex/a.png"]), Some(b"image".to_vec()));
        assert_eq!(package.texture(&["tex/a.png"]), Some(b"image".to_vec()), "served from the cache");
    }
}
