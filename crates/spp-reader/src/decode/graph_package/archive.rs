//! Unpacking a graph package (a 7z archive) into its members, and pairing each graph's description with its
//! compiled code (`assemblies/content/<n>/<graph>.xml` and the compiled graph).

use super::compiled::Assembly;
use super::description::Description;
use crate::decode::DecodeError;
use crate::format_names::{COMPILED_GRAPH_MAGIC, COMPILED_GRAPH_SUFFIX, DESCRIPTION_END};
use std::io::Cursor;

const RECORD: &str = "graph package";

/// One file inside the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    pub data: Vec<u8>,
}

/// All members of one graph package.
#[derive(Debug, Clone, Default)]
pub struct GraphPackage {
    pub members: Vec<Member>,
}

/// The two files that make up one compiled graph.
#[derive(Debug, Clone, Copy)]
pub struct PackageGraph<'a> {
    /// The member name without extension, e.g. `assemblies/content/0000/zits`.
    pub stem: &'a str,
    pub xml: &'a [u8],
    pub compiled_graph: &'a [u8],
}

impl PackageGraph<'_> {
    /// The graph's name: the last part of its stem (`zits`).
    pub fn name(&self) -> &str {
        self.stem.rsplit('/').next().unwrap_or(self.stem)
    }

    /// The graph's XML description (inputs, outputs, package url).
    pub fn description(&self) -> Result<Description, DecodeError> {
        Description::parse(self.xml)
    }

    /// The compiled graph.
    pub fn assembly(&self) -> Result<Assembly, DecodeError> {
        Assembly::parse(self.compiled_graph)
    }
}

impl GraphPackage {
    /// Unpacks a package from its bytes (as stored in a project, or a package file read by the caller).
    pub fn read(bytes: &[u8]) -> Result<GraphPackage, DecodeError> {
        match Self::read_7z(bytes) {
            Ok(a) => Ok(a),
            // The reference app writes uncompressed members with an empty coder id, which the 7z reader rejects. Read
            // the members one by one, and find the stored ones by their own signatures.
            Err(e) => Self::read_per_member(bytes).or_else(|| Self::read_stored(bytes)).ok_or(e),
        }
    }

    fn read_7z(bytes: &[u8]) -> Result<GraphPackage, DecodeError> {
        let invalid = |e: sevenz_rust2::Error| DecodeError::invalid(RECORD, e.to_string());
        let mut reader = sevenz_rust2::ArchiveReader::new(Cursor::new(bytes), sevenz_rust2::Password::empty()).map_err(invalid)?;
        let mut members = Vec::new();
        reader
            .for_each_entries(|entry, data| {
                if !entry.is_directory {
                    let mut buf = Vec::new();
                    data.read_to_end(&mut buf)?;
                    members.push(Member { name: entry.name().replace('\\', "/"), data: buf });
                }
                Ok(true)
            })
            .map_err(invalid)?;
        Ok(GraphPackage { members })
    }

    /// Each member decoded on its own (its block only); a compiled graph whose block cannot be decoded is
    /// taken from the raw bytes by its `SBAM` signature. `None` if a description cannot be read.
    fn read_per_member(bytes: &[u8]) -> Option<GraphPackage> {
        let mut reader = sevenz_rust2::ArchiveReader::new(Cursor::new(bytes), sevenz_rust2::Password::empty()).ok()?;
        let names: Vec<String> = reader.archive().files.iter().filter(|f| !f.is_directory).map(|f| f.name().to_string()).collect();
        let mut stored = stored_graphs(bytes).into_iter();
        let mut members = Vec::new();
        for name in names {
            let data = match reader.read_file(&name) {
                Ok(d) => d,
                Err(_) if name.ends_with(COMPILED_GRAPH_SUFFIX) => stored.next()?,
                // a stored icon, say: only the graphs and their descriptions matter
                Err(_) if !name.ends_with(".xml") => continue,
                Err(_) => return None,
            };
            members.push(Member { name: name.replace('\\', "/"), data });
        }
        Some(GraphPackage { members })
    }

    /// Members of a stored (uncompressed) archive: each compiled graph (`SBAM`, length at byte 0x10) and each
    /// XML description, paired in order. `None` when the counts do not match.
    fn read_stored(bytes: &[u8]) -> Option<GraphPackage> {
        let asms = stored_graphs(bytes);
        let mut xmls = Vec::new();
        let mut at = 0;
        const END: &[u8] = DESCRIPTION_END;
        while let Some(i) = find(bytes, b"<?xml", at) {
            let e = find(bytes, END, i)? + END.len();
            xmls.push(bytes.get(i..e)?.to_vec());
            at = e;
        }
        if asms.is_empty() || asms.len() != xmls.len() {
            return None;
        }
        let mut members = Vec::new();
        for (i, (asm, xml)) in asms.into_iter().zip(xmls).enumerate() {
            members.push(Member { name: format!("stored/{i:04}/graph{COMPILED_GRAPH_SUFFIX}"), data: asm });
            members.push(Member { name: format!("stored/{i:04}/graph.xml"), data: xml });
        }
        Some(GraphPackage { members })
    }

    pub fn member(&self, name: &str) -> Option<&Member> {
        self.members.iter().find(|m| m.name == name)
    }

    /// Every graph that has both a description and compiled code, in member order.
    pub fn graphs(&self) -> Vec<PackageGraph<'_>> {
        self.members
            .iter()
            .filter_map(|m| {
                let stem = m.name.strip_suffix(COMPILED_GRAPH_SUFFIX)?;
                let xml = self.member(&format!("{stem}.xml"))?;
                Some(PackageGraph { stem, xml: &xml.data, compiled_graph: &m.data })
            })
            .collect()
    }
}

/// Every compiled graph stored raw in `bytes` (`SBAM`, its length at byte 0x10), in order.
fn stored_graphs(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = find(bytes, COMPILED_GRAPH_MAGIC, at) {
        let len = bytes.get(i + 0x10..i + 0x14).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes).unwrap_or(0) as usize;
        match bytes.get(i..i.saturating_add(len)) {
            Some(data) if len >= 0x34 => {
                out.push(data.to_vec());
                at = i + len;
            }
            _ => at = i + 4,
        }
    }
    out
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::graph_package::root_xml;

    #[test]
    fn stored_members_are_found_by_signature() {
        let mut asm = b"SBAM".to_vec();
        asm.extend([0u8; 12]);
        asm.extend(0x38u32.to_le_bytes());
        asm.extend([7u8; 0x38 - 20]);
        let xml = root_xml("<?xml version=\"1.0\"?><ROOT></ROOT>").into_bytes();
        let mut file = b"7z junk header".to_vec();
        file.extend(&asm);
        file.extend(&xml);
        file.extend(b"trailing 7z header");
        let a = GraphPackage::read(&file).unwrap();
        let g = a.graphs();
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].compiled_graph, asm.as_slice());
        assert_eq!(g[0].xml, xml.as_slice());
    }
}
