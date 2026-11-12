//! The sections of a drawing: header variables, tables, blocks, entities and objects, each
//! record a slice of the tag list.

use std::collections::HashMap;

use super::tables::{AppId, BlockRecord, DimStyle, Layer, Linetype, TextStyle, Ucs, View, Vport};
use super::{Groups, Point, parse_handle};
use crate::{SectionRef, Tag, Version, section_slices};

/// One record: its type (the value of its `0` group, trimmed) and the groups up to the next one.
#[derive(Clone, Copy, Debug)]
pub struct Record<'a> {
    pub kind: &'a str,
    pub tags: &'a [Tag],
}

impl<'a> Record<'a> {
    pub fn groups(&self) -> Groups<'a> {
        Groups(self.tags)
    }
    /// The record's handle: group 5, or 105 in a DIMSTYLE record.
    pub fn handle(&self) -> Option<u64> {
        self.groups().handle(if self.kind == "DIMSTYLE" { 105 } else { 5 })
    }
    /// The record's owner: the first group 330 outside application-defined groups.
    pub fn owner(&self) -> Option<u64> {
        super::outside_app(self.tags).find(|(_, t)| t.code == 330).and_then(|(_, t)| parse_handle(&t.str()))
    }
}

/// The records of a run of tags: each from a `0` group up to the next. Tags before the first `0`
/// belong to no record and are left out.
pub fn records(tags: &[Tag]) -> Vec<Record<'_>> {
    let starts: Vec<usize> = tags.iter().enumerate().filter(|(_, t)| t.code == 0).map(|(i, _)| i).collect();
    starts
        .iter()
        .enumerate()
        .map(|(n, &i)| {
            let end = starts.get(n + 1).copied().unwrap_or(tags.len());
            let kind = tags.get(i).and_then(Tag::str_ref).unwrap_or_default().trim();
            Record { kind, tags: tags.get(i + 1..end).unwrap_or(&[]) }
        })
        .collect()
}

/// An entity with the records that belong to it: a POLYLINE's vertices or an INSERT's attributes,
/// and the SEQEND closing them.
#[derive(Clone, Debug)]
pub struct EntityRecord<'a> {
    pub kind: &'a str,
    pub tags: &'a [Tag],
    /// The VERTEX records of a POLYLINE, or the ATTRIB records of an INSERT.
    pub followers: Vec<Record<'a>>,
    pub seqend: Option<Record<'a>>,
}

impl<'a> EntityRecord<'a> {
    pub fn record(&self) -> Record<'a> {
        Record { kind: self.kind, tags: self.tags }
    }
    pub fn groups(&self) -> Groups<'a> {
        Groups(self.tags)
    }
    /// In paper space (group 67 is 1).
    pub fn paper_space(&self) -> bool {
        self.groups().i(67) == Some(1)
    }
}

/// The entities among `records`, each with its followers, and how many records were strays: a
/// VERTEX or ATTRIB with no POLYLINE or INSERT before it (a stray SEQEND holds nothing and is not
/// counted).
pub fn entity_records<'a>(records: &[Record<'a>]) -> (Vec<EntityRecord<'a>>, usize) {
    let mut out = Vec::new();
    let mut strays = 0usize;
    let mut i = 0;
    while let Some(head) = records.get(i) {
        i += 1;
        let follower = match head.kind {
            "POLYLINE" => "VERTEX",
            "INSERT" => "ATTRIB",
            "VERTEX" | "ATTRIB" => {
                strays = strays.saturating_add(1);
                continue;
            }
            "SEQEND" => continue,
            _ => "",
        };
        let mut e = EntityRecord { kind: head.kind, tags: head.tags, followers: Vec::new(), seqend: None };
        if !follower.is_empty() {
            while let Some(r) = records.get(i).filter(|r| r.kind == follower) {
                e.followers.push(*r);
                i += 1;
            }
            // The SEQEND belongs to an entity that has followers, or says it has (group 66).
            if let Some(r) = records.get(i).filter(|r| r.kind == "SEQEND" && (!e.followers.is_empty() || e.groups().i(66) == Some(1))) {
                e.seqend = Some(*r);
                i += 1;
            }
        }
        out.push(e);
    }
    (out, strays)
}

/// The header variables: each `9 $NAME` with the groups that follow it.
#[derive(Clone, Debug, Default)]
pub struct Header<'a> {
    /// In file order; the name as written, with its `$`.
    pub vars: Vec<(&'a str, &'a [Tag])>,
}

impl<'a> Header<'a> {
    fn read(tags: &'a [Tag]) -> Self {
        let starts: Vec<usize> = tags.iter().enumerate().filter(|(_, t)| t.code == 9).map(|(i, _)| i).collect();
        let vars = starts
            .iter()
            .enumerate()
            .map(|(n, &i)| {
                let end = starts.get(n + 1).copied().unwrap_or(tags.len());
                (tags.get(i).and_then(Tag::str_ref).unwrap_or_default().trim(), tags.get(i + 1..end).unwrap_or(&[]))
            })
            .collect();
        Header { vars }
    }
    /// The groups of variable `name` (any case, with or without its `$`); the first one of
    /// that name.
    pub fn var(&self, name: &str) -> Option<Groups<'a>> {
        let name = name.trim_start_matches('$');
        self.vars.iter().find(|(n, _)| n.trim_start_matches('$').eq_ignore_ascii_case(name)).map(|(_, g)| Groups(g))
    }
    /// A variable of one number.
    pub fn f(&self, name: &str) -> Option<f64> {
        self.var(name)?.0.first().map(Tag::f64)
    }
    /// A variable of one whole number.
    pub fn i(&self, name: &str) -> Option<i64> {
        self.var(name)?.0.first().map(Tag::i64)
    }
    /// A variable of one string or handle.
    pub fn s(&self, name: &str) -> Option<String> {
        self.var(name)?.0.first().map(Tag::str)
    }
    /// A point variable (groups 10/20/30).
    pub fn point(&self, name: &str) -> Option<Point> {
        self.var(name)?.point_opt(10)
    }
    /// `$ACADVER` (`AC1032`...), trimmed.
    pub fn acadver(&self) -> Option<String> {
        self.s("ACADVER").map(|v| v.trim().to_string())
    }
    /// The version this crate writes that `$ACADVER` names or is nearest to.
    pub fn version(&self) -> Option<Version> {
        Version::for_acadver(&self.acadver()?)
    }
    /// `$INSUNITS`: the drawing unit (0: unitless, also when absent).
    pub fn insunits(&self) -> i64 {
        self.i("INSUNITS").unwrap_or(0)
    }
    /// `$MEASUREMENT`: 0 imperial, 1 metric.
    pub fn measurement(&self) -> Option<i64> {
        self.i("MEASUREMENT")
    }
    /// `$LTSCALE`: the global linetype scale (1 when absent or not positive).
    pub fn ltscale(&self) -> f64 {
        self.f("LTSCALE").filter(|v| *v > 0.0).unwrap_or(1.0)
    }
    /// `$LWDEFAULT`: the default lineweight in hundredths of a millimetre (25 when absent or
    /// negative).
    pub fn lwdefault(&self) -> u16 {
        self.i("LWDEFAULT").and_then(|v| u16::try_from(v).ok()).unwrap_or(25)
    }
    /// `$HANDSEED`: the next handle the writer of the file would have used.
    pub fn handseed(&self) -> Option<u64> {
        self.s("HANDSEED").and_then(|h| parse_handle(&h))
    }
}

/// A block definition: its BLOCK record, the entities up to its ENDBLK.
#[derive(Clone, Debug)]
pub struct Block<'a> {
    /// The BLOCK record.
    pub tags: &'a [Tag],
    /// Group 2, else group 3, trimmed; empty when the record names none.
    pub name: &'a str,
    pub entities: Vec<EntityRecord<'a>>,
    /// The ENDBLK record; `None` when the section ended first.
    pub end: Option<Record<'a>>,
}

impl<'a> Block<'a> {
    pub fn groups(&self) -> Groups<'a> {
        Groups(self.tags)
    }
    /// The base point (10/20/30).
    pub fn base(&self) -> Point {
        self.groups().point(10)
    }
    /// Group 70: 1 anonymous, 2 has attribute definitions, 4 an external reference, 8 an
    /// overlay, 16 externally dependent, 32 resolved, 64 referenced.
    pub fn flags(&self) -> i64 {
        self.groups().i_or(70, 0)
    }
    /// An external reference or overlay: its art is another drawing's ([`Block::xref_path`]).
    pub fn is_xref(&self) -> bool {
        self.flags() & 12 != 0
    }
    /// The path of the drawing an external reference names (group 1).
    pub fn xref_path(&self) -> Option<String> {
        self.groups().s(1).filter(|p| !p.is_empty())
    }
    /// Group 4.
    pub fn description(&self) -> Option<String> {
        self.groups().s(4).filter(|p| !p.is_empty())
    }
    /// The block's BLOCK_RECORD (group 330, R13 and later).
    pub fn block_record(&self) -> Option<u64> {
        Record { kind: "BLOCK", tags: self.tags }.owner()
    }
}

/// A LAYOUT object.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub handle: Option<u64>,
    /// The tab's name (`Model` for model space).
    pub name: String,
    /// Group 71: the tab's place (0 for model space).
    pub tab_order: i64,
    /// The BLOCK_RECORD that holds the layout's entities ([`Document::block_name`]).
    pub block_record: Option<u64>,
    /// Groups 10/20 and 11/21: the limits, when both corners are given.
    pub limits: Option<([f64; 2], [f64; 2])>,
}

impl Layout {
    /// The typed form of a LAYOUT record. Its own groups follow the `AcDbLayout` marker (the
    /// plot settings before it reuse the codes); a record without the marker is read whole.
    pub fn read(r: &Record<'_>) -> Option<Layout> {
        let start = r.tags.iter().position(|t| t.code == 100 && t.str_ref().is_some_and(|s| s.trim() == "AcDbLayout")).unwrap_or(0);
        let own = Groups(r.tags.get(start..).unwrap_or(&[]));
        let name = own.name(1)?;
        let corner = |x: i32, y: i32| Some([own.f(x)?, own.f(y)?]);
        // A LAYOUT has two 330s, its dictionary and its block record: the block record is the
        // one among its own groups, or the last where there is no marker.
        let block_record = own.handle(330).or_else(|| r.tags.iter().rfind(|t| t.code == 330).and_then(|t| parse_handle(&t.str())));
        Some(Layout { handle: r.handle(), name, tab_order: own.i_or(71, 0), block_record, limits: corner(10, 20).zip(corner(11, 21)) })
    }
}

/// A DICTIONARY object: names (group 3) and the handles of what they name (350 or 360).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Dictionary {
    pub handle: Option<u64>,
    pub entries: Vec<(String, u64)>,
}

impl Dictionary {
    pub fn read(r: &Record<'_>) -> Dictionary {
        let mut entries = Vec::new();
        let mut name: Option<String> = None;
        for t in r.tags {
            match t.code {
                3 => name = Some(t.str()),
                350 | 360 => {
                    if let (Some(n), Some(h)) = (name.take(), parse_handle(&t.str())) {
                        entries.push((n, h));
                    }
                }
                _ => {}
            }
        }
        Dictionary { handle: r.handle(), entries }
    }
    /// The handle named `name` (any case).
    pub fn get(&self, name: &str) -> Option<u64> {
        self.entries.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, h)| *h)
    }
}

/// A drawing: the records of each section of a tag list.
#[derive(Clone, Debug, Default)]
pub struct Document<'a> {
    pub header: Header<'a>,
    /// The CLASS records of the CLASSES section.
    pub classes: Vec<Record<'a>>,
    /// The records of the TABLES section in file order, the `TABLE` and `ENDTAB` records
    /// included. The typed tables ([`Document::layers`]...) read them by record type.
    pub tables: Vec<Record<'a>>,
    pub blocks: Vec<Block<'a>>,
    /// The ENTITIES section. A damaged file may split its entities over several: they are joined.
    pub entities: Vec<EntityRecord<'a>>,
    /// The OBJECTS section.
    pub objects: Vec<Record<'a>>,
    /// Every section this model has no place for (THUMBNAILIMAGE, ACDSDATA, an unknown name),
    /// as it was split.
    pub other_sections: Vec<SectionRef<'a>>,
    /// How many sections the file has (none: not a drawing).
    pub sections: usize,
    /// Records the model left out, by section name (`(no section)` for records between
    /// sections): stray VERTEX and ATTRIB records, and records of BLOCKS outside any block.
    pub skipped: Vec<(&'static str, usize)>,
}

/// Records between sections: everything outside a SECTION ... ENDSEC pair. Stray ENDSECs and the
/// EOF hold nothing and don't count.
fn outside_sections(tags: &[Tag]) -> usize {
    let mut inside = false;
    let mut n = 0usize;
    for t in tags.iter().filter(|t| t.code == 0) {
        match t.str_ref().map(str::trim_end) {
            Some("SECTION") => inside = true,
            Some("ENDSEC") => inside = false,
            Some("EOF") => {}
            _ if !inside => n = n.saturating_add(1),
            _ => {}
        }
    }
    n
}

impl<'a> Document<'a> {
    /// Index the sections of `tags`.
    pub fn read(tags: &'a [Tag]) -> Self {
        let mut d = Document::default();
        let skip = |d: &mut Document<'a>, section: &'static str, n: usize| {
            if n > 0 {
                match d.skipped.iter_mut().find(|(s, _)| *s == section) {
                    Some((_, count)) => *count = count.saturating_add(n),
                    None => d.skipped.push((section, n)),
                }
            }
        };
        skip(&mut d, "(no section)", outside_sections(tags));
        for s in section_slices(tags) {
            d.sections += 1;
            match s.name {
                // A second HEADER adds its variables after the first's: the first of a name wins.
                "HEADER" => d.header.vars.extend(Header::read(s.tags).vars),
                "CLASSES" => d.classes.extend(records(s.tags)),
                "TABLES" => d.tables.extend(records(s.tags)),
                "BLOCKS" => {
                    let n = d.read_blocks(&records(s.tags));
                    skip(&mut d, "BLOCKS", n);
                }
                "ENTITIES" => {
                    let (entities, strays) = entity_records(&records(s.tags));
                    d.entities.extend(entities);
                    skip(&mut d, "ENTITIES", strays);
                }
                "OBJECTS" => d.objects.extend(records(s.tags)),
                _ => d.other_sections.push(s),
            }
        }
        d
    }

    /// The blocks of a BLOCKS section; returns how many records were left out.
    fn read_blocks(&mut self, recs: &[Record<'a>]) -> usize {
        let mut skipped = 0usize;
        let mut i = 0;
        while let Some(r) = recs.get(i) {
            i += 1;
            if r.kind != "BLOCK" {
                // A record outside any BLOCK ... ENDBLK pair.
                skipped = skipped.saturating_add(usize::from(r.kind != "ENDBLK"));
                continue;
            }
            let body_len = recs.get(i..).unwrap_or(&[]).iter().position(|r| r.kind == "ENDBLK" || r.kind == "BLOCK");
            let body = recs.get(i..i + body_len.unwrap_or(recs.len() - i)).unwrap_or(&[]);
            i += body.len();
            let end = recs.get(i).filter(|r| r.kind == "ENDBLK").copied();
            i += usize::from(end.is_some());
            let (entities, strays) = entity_records(body);
            skipped = skipped.saturating_add(strays);
            let g = r.groups();
            let name = [2, 3].iter().filter_map(|c| g.0.iter().find(|t| t.code == *c)?.str_ref()).map(str::trim).find(|n| !n.is_empty());
            self.blocks.push(Block { tags: r.tags, name: name.unwrap_or_default(), entities, end });
        }
        skipped
    }

    fn table<T>(&self, kind: &str, read: impl Fn(&Record<'a>) -> Option<T>) -> Vec<T> {
        self.tables.iter().filter(|r| r.kind == kind).filter_map(read).collect()
    }
    /// The LAYER records, in file order (a name may repeat: which wins is the caller's).
    pub fn layers(&self) -> Vec<Layer> {
        self.table("LAYER", Layer::read)
    }
    pub fn linetypes(&self) -> Vec<Linetype> {
        self.table("LTYPE", Linetype::read)
    }
    pub fn text_styles(&self) -> Vec<TextStyle> {
        self.table("STYLE", TextStyle::read)
    }
    pub fn dim_styles(&self) -> Vec<DimStyle<'a>> {
        self.table("DIMSTYLE", DimStyle::read)
    }
    pub fn block_records(&self) -> Vec<BlockRecord> {
        self.table("BLOCK_RECORD", BlockRecord::read)
    }
    pub fn views(&self) -> Vec<View> {
        self.table("VIEW", View::read)
    }
    pub fn ucss(&self) -> Vec<Ucs> {
        self.table("UCS", Ucs::read)
    }
    pub fn vports(&self) -> Vec<Vport> {
        self.table("VPORT", Vport::read)
    }
    pub fn app_ids(&self) -> Vec<AppId> {
        self.table("APPID", AppId::read)
    }

    /// Table record handle → name for the records of type `kind` (`LAYER`, `STYLE`, `LTYPE`...):
    /// what a handle group (a DIMSTYLE's 340, a VIEWPORT's 331) names.
    pub fn table_names(&self, kind: &str) -> HashMap<u64, &'a str> {
        let name = |r: &Record<'a>| r.tags.iter().find(|t| t.code == 2).and_then(Tag::str_ref);
        self.tables.iter().filter(|r| r.kind == kind).filter_map(|r| Some((r.handle()?, name(r)?))).collect()
    }

    /// BLOCK_RECORD handle → block name: from the BLOCK_RECORD table, and from each BLOCK that
    /// names its record (group 330) where the table has none for it.
    pub fn block_names(&self) -> HashMap<u64, &'a str> {
        let mut names = self.table_names("BLOCK_RECORD");
        for b in &self.blocks {
            if let Some(h) = b.block_record() {
                names.entry(h).or_insert(b.name);
            }
        }
        names
    }

    /// The block named `name` (any case); the first of that name.
    pub fn block(&self, name: &str) -> Option<&Block<'a>> {
        self.blocks.iter().find(|b| b.name.eq_ignore_ascii_case(name))
    }

    /// The LAYOUT objects, in file order.
    pub fn layouts(&self) -> Vec<Layout> {
        self.objects.iter().filter(|r| r.kind == "LAYOUT").filter_map(Layout::read).collect()
    }

    /// The DICTIONARY with handle `handle`.
    pub fn dictionary(&self, handle: u64) -> Option<Dictionary> {
        self.objects.iter().find(|r| r.kind == "DICTIONARY" && r.handle() == Some(handle)).map(Dictionary::read)
    }

    /// The OBJECTS records of type `kind` (`GROUP`, `IMAGEDEF`, `SCALE`...), for the readers of
    /// [`super::objects`].
    pub fn objects_of<'s>(&'s self, kind: &'s str) -> impl Iterator<Item = &'s Record<'a>> {
        self.objects.iter().filter(move |r| r.kind == kind)
    }

    /// Object handle → the name a dictionary lists it under: how a GROUP, an MLINESTYLE, a
    /// TABLESTYLE or an IMAGEDEF gets its name. Where several dictionaries name a handle, the
    /// last one read wins.
    pub fn dictionary_names(&self) -> HashMap<u64, &'a str> {
        let mut names = HashMap::new();
        for r in self.objects.iter().filter(|r| r.kind == "DICTIONARY" || r.kind == "ACDBDICTIONARYWDFLT") {
            let mut name = None;
            for t in r.tags {
                match t.code {
                    3 => name = t.str_ref(),
                    350 | 360 => {
                        if let (Some(n), Some(h)) = (name.take(), parse_handle(&t.str())) {
                            names.insert(h, n);
                        }
                    }
                    _ => {}
                }
            }
        }
        names
    }

    /// The named object dictionary: the first DICTIONARY of the OBJECTS section.
    pub fn root_dictionary(&self) -> Option<Dictionary> {
        self.objects.iter().find(|r| r.kind == "DICTIONARY").map(Dictionary::read)
    }
}
