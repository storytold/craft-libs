//! Cell metadata (`xl/metadata.xml`, ECMA-376 Part 1, 18.9) and rich values ([MS-XLSX] 2.3.6):
//! pictures in cells, and the dynamic-array flag of a formula.
//!
//! A cell's `vm` names a value-metadata block (one-based); its `XLRICHVALUE` record points at a
//! future-metadata block, whose `rv:rvb` names a rich value; a `_localImage` rich value names a
//! relationship of `richValueRel.xml`, which leads to the image. Every table below `vm` is
//! indexed from zero, and no index is assumed to equal another. A cell's `cm` names a
//! cell-metadata block; `XLDAPR` marks a dynamic-array formula.

use std::collections::HashMap;
use std::fmt::Write as _;

use craft_opc::{Rel, Rels};
use craft_xml::El;
use craft_xml::esc;

use crate::{Error, Package};

const NS_RICH: &str = "http://schemas.microsoft.com/office/spreadsheetml/2017/richdata";
const NS_RICH_REL: &str = "http://schemas.microsoft.com/office/spreadsheetml/2022/richvaluerel";
const RICH_EXT: &str = "{3E2802C4-A4D2-4D8B-9148-E3BE6C30E623}";
/// Lower case: Excel refuses a workbook whose `XLDAPR` extension URI is in upper case.
const DYNAMIC_EXT: &str = "{bdbb8cdc-fa1e-496e-a857-3c3f30c029c3}";
const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

pub(crate) const CT_METADATA: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheetMetadata+xml";
pub(crate) const RT_RICH_VALUE: &str = "http://schemas.microsoft.com/office/2017/06/relationships/rdRichValue";
pub(crate) const RT_RICH_STRUCTURE: &str = "http://schemas.microsoft.com/office/2017/06/relationships/rdRichValueStructure";
pub(crate) const RT_RICH_REL: &str = "http://schemas.microsoft.com/office/2022/10/relationships/richValueRel";

/// Largest picture read or written.
pub const MAX_PICTURE: usize = 16 << 20;
/// Most bytes of pictures in one workbook, read or written.
pub const MAX_PICTURES_TOTAL: usize = 128 << 20;
/// Most records in one metadata or rich-value table.
pub const MAX_RECORDS: usize = 100_000;
/// Largest picture side, and most pixels.
const MAX_SIDE: u32 = 8192;
const MAX_PIXELS: u64 = 16_000_000;

/// A picture in a cell: a PNG or JPEG image and its alternative text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CellPicture {
    pub image: Vec<u8>,
    pub alt: String,
}

impl CellPicture {
    /// `image/png` or `image/jpeg`, and the extension; `None` for another format, or a picture
    /// over 8192 pixels a side or 16 million pixels, which Excel does not show in a cell.
    pub fn image_type(&self) -> Option<(&'static str, &'static str)> {
        image_type(&self.image)
    }
}

fn image_type(data: &[u8]) -> Option<(&'static str, &'static str)> {
    let (w, h, mime, ext) = if data.len() >= 33 && data.starts_with(b"\x89PNG\r\n\x1a\n") && data.get(8..16)? == b"\0\0\0\rIHDR" {
        let w = u32::from_be_bytes(data.get(16..20)?.try_into().ok()?);
        let h = u32::from_be_bytes(data.get(20..24)?.try_into().ok()?);
        (w, h, "image/png", "png")
    } else if data.starts_with(&[0xff, 0xd8]) {
        let (w, h) = jpeg_dimensions(data)?;
        (w, h, "image/jpeg", "jpeg")
    } else {
        return None;
    };
    (w > 0 && h > 0 && w <= MAX_SIDE && h <= MAX_SIDE && u64::from(w) * u64::from(h) <= MAX_PIXELS).then_some((mime, ext))
}

/// The size in a JPEG's start-of-frame segment.
fn jpeg_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let mut offset = 2usize;
    while offset < data.len() {
        if *data.get(offset)? != 0xff {
            return None;
        }
        while data.get(offset) == Some(&0xff) {
            offset += 1;
        }
        let marker = *data.get(offset)?;
        offset += 1;
        if matches!(marker, 0xd9 | 0xda) {
            return None;
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        let len = usize::from(u16::from_be_bytes(data.get(offset..offset.checked_add(2)?)?.try_into().ok()?));
        let end = offset.checked_add(len)?;
        if len < 2 || end > data.len() {
            return None;
        }
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) && len >= 8 {
            let h = u16::from_be_bytes(data.get(offset + 3..offset + 5)?.try_into().ok()?);
            let w = u16::from_be_bytes(data.get(offset + 5..offset + 7)?.try_into().ok()?);
            return Some((u32::from(w), u32::from(h)));
        }
        offset = end;
    }
    None
}

/// The one internal relationship of kind `kind`; `None` (with a warning) for an external one,
/// or two that lead to different parts.
fn one_part(p: &mut Package<'_>, rels: &[Rel], kind: &str) -> Option<String> {
    let mut target: Option<&str> = None;
    for rel in rels.iter().filter(|r| r.kind() == kind) {
        if rel.external {
            p.warn(format!("an external {kind} relationship is not read; cell pictures may be missing"));
            return None;
        }
        if target.is_some_and(|t| !t.eq_ignore_ascii_case(&rel.target)) {
            p.warn(format!("conflicting {kind} relationships; cell pictures may be missing"));
            return None;
        }
        target = Some(&rel.target);
    }
    target.map(str::to_string)
}

fn part_root(p: &mut Package<'_>, part: &str, root: &str) -> Result<Option<El>, Error> {
    let Some(x) = p.optional_xml(part)? else { return Ok(None) };
    if x.name != root {
        p.warn(format!("{part} is not a {root} part; cell pictures were not loaded"));
        return Ok(None);
    }
    Ok(Some(x))
}

/// The children `child` of `el`, refused over [`MAX_RECORDS`]; a `count` that does not match is
/// a warning.
fn records<'a>(p: &mut Package<'_>, el: &'a El, child: &'a str) -> Result<Vec<&'a El>, Error> {
    let list: Vec<&El> = el.children(child).take(MAX_RECORDS + 1).collect();
    if list.len() > MAX_RECORDS {
        return Err(Error::TooLarge(format!("more than {MAX_RECORDS} {child} records in {}", el.name)));
    }
    if el.attr("count").is_some() && el.attr_u32("count").map(|n| n as usize) != Some(list.len()) {
        p.warn(format!("the count of {} does not match its records", el.name));
    }
    Ok(list)
}

/// Where a `_localImage` structure keeps the image and the alternative text.
struct Layout {
    keys: usize,
    image: Option<usize>,
    text: Option<usize>,
}

impl Layout {
    fn of(s: &El) -> Option<Layout> {
        if s.attr("t") != Some("_localImage") {
            return None;
        }
        let keys: Vec<&El> = s.children("rv:k").collect();
        // The one key of that name, if it has that type.
        let one = |name: &str, ty: &str| {
            let mut found = keys.iter().enumerate().filter(|(_, k)| k.attr("n") == Some(name));
            let (i, key) = found.next()?;
            (found.next().is_none() && key.attr("t") == Some(ty)).then_some(i)
        };
        Some(Layout { keys: keys.len(), image: one("_rvRel:LocalImageIdentifier", "i"), text: one("Text", "s") })
    }
}

impl Package<'_> {
    /// The picture each value-metadata block shows: entry `vm - 1` for a cell's `vm`. `None` for
    /// a block that is not a picture in a cell, or one that does not read (a warning names it;
    /// the cell keeps its saved value). Empty when the workbook has no cell metadata.
    pub fn cell_pictures(&mut self) -> Result<Vec<Option<CellPicture>>, Error> {
        let book = self.workbook_rels()?;
        let Some(meta_part) = one_part(self, &book.list, "sheetMetadata") else { return Ok(Vec::new()) };
        let Some(meta) = part_root(self, &meta_part, "metadata")? else { return Ok(Vec::new()) };
        let Some(types) = meta.child("metadataTypes") else { return Ok(Vec::new()) };
        let types = records(self, types, "metadataType")?;
        let rich: Vec<usize> = types.iter().enumerate().filter(|(_, t)| t.attr("name") == Some("XLRICHVALUE")).map(|(i, _)| i + 1).collect();
        let type_id = match rich.as_slice() {
            [] => return Ok(Vec::new()),
            [one] => *one,
            _ => {
                self.warn("two XLRICHVALUE metadata types; cell pictures were not loaded");
                return Ok(Vec::new());
            }
        };
        // The parts are the workbook's, or the metadata's and the rich values' (MS-XLSX allows
        // either owner).
        let mut rels = book.list.clone();
        rels.extend(self.opc().rels(&meta_part)?.list);
        let Some(data_part) = one_part(self, &rels, "rdRichValue") else {
            self.warn("no rich-value data part; cell pictures were not loaded");
            return Ok(Vec::new());
        };
        rels.extend(self.opc().rels(&data_part)?.list);
        let (Some(structure_part), Some(rel_part)) = (one_part(self, &rels, "rdRichValueStructure"), one_part(self, &rels, "richValueRel")) else {
            self.warn("no rich-value structure or relationship part; cell pictures were not loaded");
            return Ok(Vec::new());
        };
        let Some(data) = part_root(self, &data_part, "rv:rvData")? else { return Ok(Vec::new()) };
        let Some(structures) = part_root(self, &structure_part, "rv:rvStructures")? else { return Ok(Vec::new()) };
        let Some(rel_list) = part_root(self, &rel_part, "rvrel:richValueRels")? else { return Ok(Vec::new()) };
        let image_rels: Rels = self.opc().rels(&rel_part)?;
        let layouts: Vec<Option<Layout>> = records(self, &structures, "rv:s")?.into_iter().map(Layout::of).collect();
        let rel_list = records(self, &rel_list, "rvrel:rel")?;
        // By id, a repeated id marked unusable.
        let mut by_id: HashMap<&str, Option<&Rel>> = HashMap::new();
        for r in &image_rels.list {
            by_id.entry(r.id.as_str()).and_modify(|e| *e = None).or_insert(Some(r));
        }
        let mut cache: HashMap<String, Vec<u8>> = HashMap::new();
        let mut total = 0usize;
        let mut pictures = Vec::new();
        for value in records(self, &data, "rv:rv")? {
            let picture = self.rich_picture(value, &layouts, &rel_list, &by_id, &mut cache, &mut total)?;
            if picture.is_none() {
                self.warn("some rich values are not pictures this crate reads; their cells keep their saved values");
            }
            pictures.push(picture);
        }
        let Some(future) = meta.children("futureMetadata").find(|f| f.attr("name") == Some("XLRICHVALUE")) else {
            self.warn("no XLRICHVALUE future metadata; cell pictures were not loaded");
            return Ok(Vec::new());
        };
        let future: Vec<Option<usize>> = records(self, future, "bk")?
            .into_iter()
            .map(|bk| {
                let ext = bk.child("extLst")?.children("ext").find(|e| e.attr("uri").is_some_and(|u| u.eq_ignore_ascii_case(RICH_EXT)))?;
                ext.child("rv:rvb")?.attr_u32("i").map(|n| n as usize)
            })
            .collect();
        let Some(blocks) = meta.child("valueMetadata") else { return Ok(Vec::new()) };
        Ok(records(self, blocks, "bk")?
            .into_iter()
            .map(|bk| {
                let rc = bk.children("rc").find(|rc| rc.attr_u32("t").map(|n| n as usize) == Some(type_id))?;
                let v = (*future.get(rc.attr_u32("v")? as usize)?)?;
                pictures.get(v).cloned().flatten()
            })
            .collect())
    }

    fn rich_picture(
        &mut self,
        value: &El,
        layouts: &[Option<Layout>],
        rel_list: &[&El],
        by_id: &HashMap<&str, Option<&Rel>>,
        cache: &mut HashMap<String, Vec<u8>>,
        total: &mut usize,
    ) -> Result<Option<CellPicture>, Error> {
        let Some(Some(layout)) = value.attr_u32("s").and_then(|n| layouts.get(n as usize)) else { return Ok(None) };
        let values: Vec<&El> = value.children("rv:v").collect();
        if layout.keys != values.len() {
            return Ok(None);
        }
        let slot = |i: Option<usize>| i.and_then(|i| values.get(i)).map(|v| v.text().into_owned());
        let rel_id = slot(layout.image).and_then(|v| v.trim().parse::<usize>().ok()).and_then(|n| rel_list.get(n)).and_then(|r| r.attr("r:id"));
        let Some(rel_id) = rel_id else { return Ok(None) };
        let Some(found) = by_id.get(rel_id) else { return Ok(None) };
        let Some(rel) = found.filter(|r| r.kind() == "image" && !r.external) else {
            self.warn("a cell picture has an external or repeated image relationship; its cell keeps its saved value");
            return Ok(None);
        };
        let alt = slot(layout.text).unwrap_or_default();
        let key = rel.target.to_ascii_lowercase();
        if let Some(image) = cache.get(&key) {
            return Ok(Some(CellPicture { image: image.clone(), alt }));
        }
        let image = match self.opc().read_optional_limited(&rel.target, MAX_PICTURE as u64) {
            Ok(Some(b)) => b,
            Ok(None) => {
                self.warn(format!("missing cell picture {}; its cell keeps its saved value", rel.target));
                return Ok(None);
            }
            Err(e) if matches!(e, craft_opc::Error::PackageTooLarge { .. }) => return Err(e.into()),
            Err(e) => {
                self.warn(format!("cell picture {} not read: {e}; its cell keeps its saved value", rel.target));
                return Ok(None);
            }
        };
        if image_type(&image).is_none() {
            self.warn("a cell picture is not a PNG or JPEG of at most 8192 pixels a side and 16 million pixels; its cell keeps its saved value");
            return Ok(None);
        }
        if total.saturating_add(image.len()) > MAX_PICTURES_TOTAL {
            self.warn("cell pictures over 128 MiB in all; the cells after keep their saved values");
            return Ok(None);
        }
        *total += image.len();
        cache.insert(key, image.clone());
        Ok(Some(CellPicture { image, alt }))
    }
}

/// The metadata part, the rich-value parts, the images, and the workbook relationships each
/// needs (`(full type, part name)`).
pub(crate) struct MetadataParts {
    /// (name, content type, bytes); a relationships part has no content type.
    pub parts: Vec<(String, Option<&'static str>, Vec<u8>)>,
    pub workbook_rels: Vec<(&'static str, String)>,
}

/// The parts for `pictures` (picture `i` is shown by `vm = i + 1`) and, when `dynamic_arrays`,
/// the `XLDAPR` cell metadata (`cm = 1`), written the way Excel writes them: one `_localImage`
/// structure, `cellImageN` media. `fresh(stem, ext, numbered)` names a new part.
pub(crate) fn metadata_parts(
    pictures: &[CellPicture],
    dynamic_arrays: bool,
    mut fresh: impl FnMut(&str, &str, bool) -> String,
) -> Result<MetadataParts, Error> {
    if pictures.len() > MAX_RECORDS {
        return Err(Error::TooLarge(format!("more than {MAX_RECORDS} cell pictures")));
    }
    let mut out = MetadataParts { parts: Vec::new(), workbook_rels: Vec::new() };
    let meta = fresh("xl/metadata", "xml", false);
    out.workbook_rels.push(("http://schemas.openxmlformats.org/officeDocument/2006/relationships/sheetMetadata", meta.clone()));
    if pictures.is_empty() {
        out.parts.push((meta, Some(CT_METADATA), metadata_xml(0, dynamic_arrays).into_bytes()));
        return Ok(out);
    }
    let rel_part = fresh("xl/richData/richValueRel", "xml", false);
    let mut data = format!("{DECL}<rvData xmlns=\"{NS_RICH}\" count=\"{}\">", pictures.len());
    let mut rel_list =
        format!("{DECL}<richValueRels xmlns=\"{NS_RICH_REL}\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">");
    let mut image_rels = craft_opc::RelsOut::default();
    let mut total = 0usize;
    for (i, picture) in pictures.iter().enumerate() {
        total = total.saturating_add(picture.image.len());
        if picture.image.len() > MAX_PICTURE || total > MAX_PICTURES_TOTAL {
            return Err(Error::TooLarge("cell pictures over 16 MiB each or 128 MiB in all".into()));
        }
        let (mime, ext) = picture
            .image_type()
            .ok_or_else(|| Error::Invalid("a cell picture must be a PNG or JPEG of at most 8192 pixels a side and 16 million pixels".into()))?;
        let image = fresh("xl/media/cellImage", ext, true);
        let rid = image_rels.add_new("image", &craft_opc::relative(&rel_part, &image), false);
        let _ = write!(rel_list, "<rel r:id=\"{rid}\"/>");
        let _ = write!(data, "<rv s=\"0\"><v>{i}</v><v>5</v><v>{}</v></rv>", esc(&picture.alt));
        out.parts.push((image, Some(mime), picture.image.clone()));
    }
    data.push_str("</rvData>");
    rel_list.push_str("</richValueRels>");
    let structures = format!(
        "{DECL}<rvStructures xmlns=\"{NS_RICH}\" count=\"1\"><s t=\"_localImage\"><k n=\"_rvRel:LocalImageIdentifier\" t=\"i\"/><k n=\"CalcOrigin\" t=\"i\"/><k n=\"Text\" t=\"s\"/></s></rvStructures>"
    );
    for (name, ct, rt, bytes) in [
        (fresh("xl/richData/rdrichvalue", "xml", false), "application/vnd.ms-excel.rdRichValue+xml", RT_RICH_VALUE, data.into_bytes()),
        (
            fresh("xl/richData/rdrichvaluestructure", "xml", false),
            "application/vnd.ms-excel.rdRichValueStructure+xml",
            RT_RICH_STRUCTURE,
            structures.into_bytes(),
        ),
        (rel_part.clone(), "application/vnd.ms-excel.richvaluerel+xml", RT_RICH_REL, rel_list.into_bytes()),
    ] {
        out.workbook_rels.push((rt, name.clone()));
        out.parts.push((name, Some(ct), bytes));
    }
    out.parts.push((craft_opc::rels_path(&rel_part), None, image_rels.to_xml()));
    out.parts.push((meta, Some(CT_METADATA), metadata_xml(pictures.len(), dynamic_arrays).into_bytes()));
    Ok(out)
}

/// The metadata part: `XLDAPR` (type 1) when `dynamic`, then `XLRICHVALUE` for `pictures`
/// rich values when there are any.
fn metadata_xml(pictures: usize, dynamic: bool) -> String {
    const XLDAPR: &str = "<metadataType name=\"XLDAPR\" minSupportedVersion=\"120000\" copy=\"1\" pasteAll=\"1\" pasteValues=\"1\" merge=\"1\" splitFirst=\"1\" rowColShift=\"1\" clearFormats=\"1\" clearComments=\"1\" assign=\"1\" coerce=\"1\" cellMeta=\"1\"/>";
    let dynamic_future = format!(
        "<futureMetadata name=\"XLDAPR\" count=\"1\"><bk><extLst><ext uri=\"{DYNAMIC_EXT}\"><xda:dynamicArrayProperties fDynamic=\"1\" fCollapsed=\"0\"/></ext></extLst></bk></futureMetadata>"
    );
    const DYNAMIC_CELLS: &str = "<cellMetadata count=\"1\"><bk><rc t=\"1\" v=\"0\"/></bk></cellMetadata>";
    let xda = "xmlns:xda=\"http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray\"";
    let main = crate::ns::MAIN;
    if pictures == 0 {
        return format!(
            "{DECL}<metadata xmlns=\"{main}\" {xda}><metadataTypes count=\"1\">{XLDAPR}</metadataTypes>{dynamic_future}{DYNAMIC_CELLS}</metadata>"
        );
    }
    let type_id = if dynamic { 2 } else { 1 };
    let mut x = format!("{DECL}<metadata xmlns=\"{main}\" xmlns:rv=\"{NS_RICH}\" {xda}><metadataTypes count=\"{type_id}\">");
    if dynamic {
        x.push_str(XLDAPR);
    }
    x.push_str("<metadataType name=\"XLRICHVALUE\" minSupportedVersion=\"120000\" copy=\"1\" pasteAll=\"1\" pasteValues=\"1\" merge=\"1\" splitFirst=\"1\" rowColShift=\"1\" clearFormats=\"1\" clearComments=\"1\" assign=\"1\" cellMeta=\"0\"/></metadataTypes>");
    if dynamic {
        x.push_str(&dynamic_future);
    }
    let _ = write!(x, "<futureMetadata name=\"XLRICHVALUE\" count=\"{pictures}\">");
    for i in 0..pictures {
        let _ = write!(x, "<bk><extLst><ext uri=\"{RICH_EXT}\"><rv:rvb i=\"{i}\"/></ext></extLst></bk>");
    }
    x.push_str("</futureMetadata>");
    if dynamic {
        x.push_str(DYNAMIC_CELLS);
    }
    let _ = write!(x, "<valueMetadata count=\"{pictures}\">");
    for i in 0..pictures {
        let _ = write!(x, "<bk><rc t=\"{type_id}\" v=\"{i}\"/></bk>");
    }
    x.push_str("</valueMetadata></metadata>");
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn png(w: u32, h: u32) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        b.extend(w.to_be_bytes());
        b.extend(h.to_be_bytes());
        b.extend([8, 6, 0, 0, 0, 0, 0, 0, 0]);
        b
    }

    #[test]
    fn image_limits() {
        assert_eq!(image_type(&png(10, 20)), Some(("image/png", "png")));
        assert_eq!(image_type(&png(8193, 1)), None);
        assert_eq!(image_type(&png(5000, 4000)), None);
        assert_eq!(image_type(&png(0, 4)), None);
        let jpeg = [0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0, 0xff, 0xc0, 0, 11, 8, 0, 30, 0, 40, 3, 0, 0, 0];
        assert_eq!(jpeg_dimensions(&jpeg), Some((40, 30)));
        assert_eq!(image_type(&jpeg), Some(("image/jpeg", "jpeg")));
        // A segment length that runs past the end.
        assert_eq!(jpeg_dimensions(&[0xff, 0xd8, 0xff, 0xe0, 0xff, 0xff]), None);
        assert_eq!(image_type(b"GIF89a"), None);
    }

    #[test]
    fn write_limits() {
        let mut n = 0;
        let mut fresh = |stem: &str, ext: &str, _: bool| {
            n += 1;
            format!("{stem}{n}.{ext}")
        };
        let many = vec![CellPicture { image: png(1, 1), alt: String::new() }; MAX_RECORDS + 1];
        assert!(matches!(metadata_parts(&many, false, &mut fresh), Err(Error::TooLarge(_))));
        let big = [CellPicture { image: [png(1, 1), vec![0; MAX_PICTURE]].concat(), alt: String::new() }];
        assert!(matches!(metadata_parts(&big, false, &mut fresh), Err(Error::TooLarge(_))));
        let gif = [CellPicture { image: b"GIF89a".to_vec(), alt: String::new() }];
        assert!(matches!(metadata_parts(&gif, false, &mut fresh), Err(Error::Invalid(_))));
    }
}
