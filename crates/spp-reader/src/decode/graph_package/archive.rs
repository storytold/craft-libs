//! Unpacking a graph package (a 7z archive) into its members, and pairing each graph's description with its
//! compiled code (`assemblies/content/<n>/<graph>.xml` and the compiled graph).

use super::compiled::Assembly;
use super::description::Description;
use crate::decode::DecodeError;
use crate::format_names::{COMPILED_GRAPH_MAGIC, COMPILED_GRAPH_SUFFIX, DESCRIPTION_END};
use sevenz_rust2::{Archive, ArchiveReader, BlockDecoder, Password};
use std::io::{self, Cursor, Read};

const RECORD: &str = "graph package";
/// Graph packages contain compiled code and resources. Keep the retained output of one package bounded.
const PACKAGE_LIMITS: PackageLimits = PackageLimits { member: 128 * 1024 * 1024, total: 256 * 1024 * 1024 };

#[derive(Clone, Copy)]
struct PackageLimits {
    member: u64,
    total: u64,
}

impl PackageLimits {
    fn charge(self, remaining: &mut u64, size: u64) -> Result<(), DecodeError> {
        if size > self.member {
            return Err(DecodeError::invalid(RECORD, format!("member exceeds the {} byte limit", self.member)));
        }
        *remaining = remaining.checked_sub(size).ok_or_else(|| self.total_error())?;
        Ok(())
    }

    fn total_error(self) -> DecodeError {
        DecodeError::invalid(RECORD, format!("unpacked data exceeds the {} byte package limit", self.total))
    }

    fn check_archive(self, archive: &Archive) -> Result<(), DecodeError> {
        let mut remaining = self.total;
        for entry in &archive.files {
            self.charge(&mut remaining, entry.size)?;
        }
        // A solid block may be decoded to reach any member in it, including by the per-member fallback.
        let mut block_remaining = self.total;
        for block in &archive.blocks {
            block_remaining = block_remaining.checked_sub(block.get_unpack_size()).ok_or_else(|| self.total_error())?;
        }
        Ok(())
    }
}

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
        Self::read_with_limits(bytes, PACKAGE_LIMITS)
    }

    fn read_with_limits(bytes: &[u8], limits: PackageLimits) -> Result<Self, DecodeError> {
        let error = match ArchiveReader::new(Cursor::new(bytes), Password::empty()) {
            Ok(mut reader) => {
                // Budget rejection is terminal, including when the raw signatures would be readable.
                limits.check_archive(reader.archive())?;
                match Self::read_7z(&mut reader) {
                    Ok(package) => return Ok(package),
                    Err(error) => {
                        // The reference app writes uncompressed members with an empty coder id, which the 7z reader rejects.
                        // Read the remaining members one by one, finding raw compiled graphs by their signature.
                        if let Some(package) = Self::read_per_member(reader.archive(), bytes, limits)? {
                            return Ok(package);
                        }
                        error
                    }
                }
            }
            Err(error) => DecodeError::invalid(RECORD, error.to_string()),
        };
        Self::read_stored(bytes, limits)?.ok_or(error)
    }

    fn read_7z(reader: &mut ArchiveReader<Cursor<&[u8]>>) -> Result<GraphPackage, DecodeError> {
        let mut members = Vec::new();
        reader
            .for_each_entries(|entry, data| {
                if !entry.is_directory {
                    let data = read_member(data, entry.size)?;
                    members.push(Member { name: entry.name().replace('\\', "/"), data });
                }
                Ok(true)
            })
            .map_err(|error| DecodeError::invalid(RECORD, error.to_string()))?;
        Ok(GraphPackage { members })
    }

    /// Each member decoded on its own (its block only); a compiled graph whose block cannot be decoded is
    /// taken from the raw bytes by its `SBAM` signature. `None` if a description cannot be read.
    fn read_per_member(archive: &Archive, bytes: &[u8], limits: PackageLimits) -> Result<Option<Self>, DecodeError> {
        let mut source = Cursor::new(bytes);
        let mut stored = stored_graphs(bytes);
        let mut remaining = limits.total;
        let mut members = Vec::new();
        for (index, entry) in archive.files.iter().enumerate().filter(|(_, entry)| !entry.is_directory) {
            let name = entry.name();
            let mut after_member = remaining;
            limits.charge(&mut after_member, entry.size)?;
            let data = match read_one_member(archive, &mut source, index) {
                Ok(data) => data,
                Err(_) if name.ends_with(COMPILED_GRAPH_SUFFIX) => {
                    let Some(data) = stored.next() else { return Ok(None) };
                    after_member = remaining;
                    limits.charge(&mut after_member, data.len() as u64)?;
                    data.to_vec()
                }
                // A stored icon, say: only the graphs and their descriptions matter.
                Err(_) if !name.ends_with(".xml") => continue,
                Err(_) => return Ok(None),
            };
            remaining = after_member;
            members.push(Member { name: name.replace('\\', "/"), data });
        }
        Ok(Some(GraphPackage { members }))
    }

    /// Members of a stored (uncompressed) archive: each compiled graph and XML description, paired in order.
    /// `None` when the counts do not match.
    fn read_stored(bytes: &[u8], limits: PackageLimits) -> Result<Option<GraphPackage>, DecodeError> {
        let asms = stored_graphs(bytes);
        let mut at = 0;
        let mut remaining = limits.total;
        let mut members = Vec::new();
        for (i, asm) in asms.enumerate() {
            let xml = find(bytes, b"<?xml", at).and_then(|start| {
                let end = find(bytes, DESCRIPTION_END, start)?.checked_add(DESCRIPTION_END.len())?;
                at = end;
                bytes.get(start..end)
            });
            let Some(xml) = xml else { return Ok(None) };
            limits.charge(&mut remaining, asm.len() as u64)?;
            limits.charge(&mut remaining, xml.len() as u64)?;
            members.push(Member { name: format!("stored/{i:04}/graph{COMPILED_GRAPH_SUFFIX}"), data: asm.to_vec() });
            members.push(Member { name: format!("stored/{i:04}/graph.xml"), data: xml.to_vec() });
        }
        if members.is_empty() || find(bytes, b"<?xml", at).is_some() {
            return Ok(None);
        }
        Ok(Some(GraphPackage { members }))
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

/// Read a member by archive index. A solid block's earlier members are drained by the reader
/// without allocating their contents.
fn read_one_member(archive: &Archive, source: &mut Cursor<&[u8]>, index: usize) -> Result<Vec<u8>, sevenz_rust2::Error> {
    let Some(block) = archive.stream_map.file_block_index.get(index).copied().flatten() else { return Ok(Vec::new()) };
    let mut at = archive.stream_map.block_first_file_index.get(block).copied().ok_or(sevenz_rust2::Error::FileNotFound)?;
    let mut out = None;
    let password = Password::empty();
    let reader = BlockDecoder::new(1, block, archive, &password, source);
    reader.for_each_entries(&mut |entry, data| {
        if at == index {
            out = Some(read_member(data, entry.size)?);
            return Ok(false);
        }
        at += 1;
        Ok(true)
    })?;
    out.ok_or(sevenz_rust2::Error::FileNotFound)
}

/// The declared size was checked against both limits before opening the decompression stream.
fn read_member(data: &mut dyn Read, size: u64) -> io::Result<Vec<u8>> {
    let size = usize::try_from(size).map_err(io::Error::other)?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(size).map_err(io::Error::other)?;
    bytes.resize(size, 0);
    data.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// Every compiled graph stored raw in `bytes` (`SBAM`, its length at byte 0x10), in order.
fn stored_graphs(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut at = 0;
    std::iter::from_fn(move || {
        while let Some(i) = find(bytes, COMPILED_GRAPH_MAGIC, at) {
            let len = bytes.get(i.saturating_add(0x10)..i.saturating_add(0x14)).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes).unwrap_or(0)
                as usize;
            at = i.saturating_add(len.max(4));
            if let Some(data) = bytes.get(i..at)
                && len >= 0x34
            {
                return Some(data);
            }
            at = i.saturating_add(4);
        }
        None
    })
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::graph_package::root_xml;

    fn stored_package() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let mut asm = b"SBAM".to_vec();
        asm.extend([0u8; 12]);
        asm.extend(0x38u32.to_le_bytes());
        asm.extend([7u8; 0x38 - 20]);
        let xml = root_xml("<?xml version=\"1.0\"?><ROOT></ROOT>").into_bytes();
        let mut file = b"7z junk header".to_vec();
        file.extend(&asm);
        file.extend(&xml);
        file.extend(b"trailing 7z header");
        (file, asm, xml)
    }

    #[test]
    fn stored_members_are_found_by_signature() {
        let (file, asm, xml) = stored_package();
        let a = GraphPackage::read(&file).unwrap();
        let g = a.graphs();
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].compiled_graph, asm.as_slice());
        assert_eq!(g[0].xml, xml.as_slice());
    }

    #[test]
    fn archive_admission_charges_each_member_and_their_total() {
        let mut archive = Archive::default();
        for name in ["graph.xml", "graph.sbsasm"] {
            let mut entry = sevenz_rust2::ArchiveEntry::new_file(name);
            entry.size = 6;
            archive.files.push(entry);
        }
        assert!(PackageLimits { member: 6, total: 12 }.check_archive(&archive).is_ok());
        assert!(PackageLimits { member: 5, total: 12 }.check_archive(&archive).is_err());
        assert!(PackageLimits { member: 6, total: 11 }.check_archive(&archive).is_err());
    }

    #[test]
    fn member_read_stops_at_its_declared_end() {
        let mut data = Cursor::new(b"one-two");
        assert_eq!(read_member(&mut data, 3).unwrap(), b"one");
        assert_eq!(data.position(), 3);
        assert_eq!(read_member(&mut data, 5).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn stored_assembly_and_xml_share_the_package_budget() {
        let (file, asm, xml) = stored_package();
        let member = asm.len().max(xml.len()) as u64;
        let total = (asm.len() + xml.len()) as u64;
        assert_eq!(GraphPackage::read_with_limits(&file, PackageLimits { member, total }).unwrap().graphs().len(), 1);
        assert!(GraphPackage::read_with_limits(&file, PackageLimits { member, total: total - 1 }).is_err());
        assert!(GraphPackage::read_with_limits(&file, PackageLimits { member: member - 1, total }).is_err());
    }
}
