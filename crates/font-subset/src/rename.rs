//! Renames a subset according to its font's licence (architecture, "Preview
//! subsets"). Runs on HarfBuzz's output, which is small.

use std::collections::BTreeMap;
use std::ops::Range;

use write_fonts::FontBuilder;
use write_fonts::from_obj::ToOwnedTable;
use write_fonts::read::{FontRef, TableProvider};
use write_fonts::tables::name::{Name, NameRecord};
use write_fonts::types::{NameId, Tag};

use super::Error;
use super::cff::{self, CffNames};

/// The licence of the upstream font, which decides how a subset is named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Licence {
    /// SIL Open Font License, with its Reserved Font Names (may be empty).
    Ofl {
        /// The Reserved Font Names.
        reserved_names: Vec<String>,
    },
    /// Ubuntu Font Licence 1.0.
    Ufl,
    /// Apache 2.0.
    Apache,
}

/// What renaming needs to know about the upstream font.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Naming {
    /// The upstream font's licence.
    pub licence: Licence,
    /// The family name from the index.
    pub family_name: String,
    /// The SHA-256 of the upstream font file.
    pub sha256: [u8; 32],
}

impl Naming {
    /// The number in `FPS <n>`: decimal, so it can never spell a word.
    pub fn number(&self) -> u64 {
        let [a, b, c, d, e, f, g, h, ..] = self.sha256;
        u64::from_be_bytes([a, b, c, d, e, f, g, h]) % 1_000_000_000
    }
}

const FAMILY_IDS: [u16; 3] = [1, 16, 21];
const FULL_IDS: [u16; 2] = [4, 18];
const PS_ID: u16 = 6;
const UNIQUE_ID: u16 = 3;
const DROPPED_IDS: [u16; 3] = [19, 20, 25];
const STYLE_IDS: [u16; 3] = [2, 17, 22];
const DESCRIPTION_ID: u16 = 10;
const MAX_PS_LEN: usize = 63;
const CFF: Tag = Tag::new(b"CFF ");

/// The name IDs HarfBuzz must keep beyond its default 0-6.
pub(crate) const KEPT_IDS: [u32; 12] = [7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 18, 21];

struct NewNames {
    family: String,
    full: String,
    ps: String,
    /// New PostScript names of fvar named instances, by name ID.
    instance_ps: BTreeMap<u16, String>,
    notice: Option<String>,
    /// Reserved Font Name words and the upstream family names.
    banned: Banned,
}

/// Rewrites the name table (and CFF names) of `sfnt`.
pub(crate) fn rename(sfnt: &[u8], naming: &Naming) -> Result<Vec<u8>, Error> {
    let font = FontRef::new(sfnt)?;
    let mut name: Name = font.name()?.to_owned_table();
    let new = new_names(&font, &name, naming)?;

    let mut records: Vec<NameRecord> = name
        .name_record
        .iter()
        .filter_map(|rec| {
            let string = new_string(rec.name_id.to_u16(), rec.string.as_str(), &new)?;
            let mut rec = rec.clone();
            rec.string = string.into();
            Some(rec)
        })
        .collect();
    if let Some(notice) = &new.notice
        && !records.iter().any(|r| r.name_id.to_u16() == DESCRIPTION_ID)
    {
        records.push(NameRecord::new(3, 1, 0x409, NameId::new(DESCRIPTION_ID), notice.clone().into()));
    }
    records.sort();
    records.dedup_by(|a, b| (a.platform_id, a.encoding_id, a.language_id, a.name_id) == (b.platform_id, b.encoding_id, b.language_id, b.name_id));
    name.name_record = records;

    let mut builder = FontBuilder::new();
    builder.add_table(&name)?;
    if let Some(data) = font.table_data(CFF) {
        let names = CffNames { ps: &new.ps, full: &new.full, family: &new.family, banned: &new.banned };
        builder.add_raw(CFF, cff::rename(data.as_bytes(), &names)?);
    }
    Ok(builder.copy_missing_tables(font).build())
}

fn new_string(id: u16, old: &str, new: &NewNames) -> Option<String> {
    if FAMILY_IDS.contains(&id) {
        Some(new.family.clone())
    } else if FULL_IDS.contains(&id) {
        Some(new.full.clone())
    } else if id == PS_ID || id == UNIQUE_ID {
        Some(new.ps.clone())
    } else if DROPPED_IDS.contains(&id) {
        None
    } else if let Some(ps) = new.instance_ps.get(&id) {
        Some(ps.clone())
    } else if id == DESCRIPTION_ID && new.notice.is_some() {
        new.notice.clone()
    } else if id == 0 || (7..=14).contains(&id) {
        Some(old.to_owned())
    } else if STYLE_IDS.contains(&id) || id >= 256 {
        // HarfBuzz keeps the STAT names of a fully pinned font too.
        Some(style_part(old, &new.banned).unwrap_or_else(|| if id == 2 { "Regular".to_owned() } else { format!("Style {id}") }))
    } else {
        Some(new.banned.replace(old, &new.family))
    }
}

/// A style name without the banned names in it ("Roboto Flex Bold" becomes
/// "Bold"), or `None` when nothing else is left.
fn style_part(old: &str, banned: &Banned) -> Option<String> {
    if !banned.found_in(old) {
        return Some(old.to_owned());
    }
    let rest = banned.replace(old, "");
    let rest = rest.split_whitespace().collect::<Vec<_>>().join(" ");
    let rest = rest.trim_matches(|c: char| c.is_whitespace() || "-_,;:".contains(c));
    (!rest.is_empty() && !banned.found_in(rest)).then(|| rest.to_owned())
}

fn new_names(font: &FontRef, name: &Name, naming: &Naming) -> Result<NewNames, Error> {
    let n = naming.number();
    let reserved: Vec<&str> = match &naming.licence {
        Licence::Ofl { reserved_names } => reserved_names.iter().flat_map(|s| words(s).chain([s.as_str()])).collect(),
        Licence::Ufl | Licence::Apache => Vec::new(),
    };
    let rfn = Banned::new(reserved.iter().copied());
    let banned = banned_names(font, name, naming, &reserved);

    let ufl = naming.licence == Licence::Ufl;
    let raw_style = english(name, 17).or_else(|| english(name, 2));
    let (family, ps) = if ufl {
        let orig_ps = source_ps(name, PS_ID, naming, raw_style.as_deref());
        (format!("{} derivative FPS{n}", naming.family_name), derivative_ps(&orig_ps, n))
    } else {
        (format!("FPS {n}"), format!("FPS-{n}"))
    };
    let style = raw_style.and_then(|s| style_part(&s, &banned)).filter(|s| s != "Regular");
    let full = match style {
        Some(style) => format!("{family} {style}"),
        None => family.clone(),
    };
    let instance_ps = instance_ps_names(font, name, naming, &banned);
    let notice = (naming.licence == Licence::Apache).then(|| format!("Subset of {}, changed by font-preview-service", naming.family_name));

    for s in [&family, &full, &ps].into_iter().chain(instance_ps.values()) {
        if rfn.found_in(s) {
            return Err(Error::ReservedName(s.clone()));
        }
    }
    Ok(NewNames { family, full, ps, instance_ps, notice, banned })
}

/// The Reserved Font Name words and every family name the upstream font
/// goes by: the index's, name IDs 16 (else 1) and 6, and CFF's.
fn banned_names(font: &FontRef, name: &Name, naming: &Naming, reserved: &[&str]) -> Banned {
    let ps = english(name, PS_ID).unwrap_or_default();
    let ps_family = ps.split('-').next().unwrap_or_default().to_owned();
    let cff = font.table_data(CFF).and_then(|data| cff::family_names(data.as_bytes()).ok()).unwrap_or_default();
    let family: Vec<String> = [Some(naming.family_name.clone()), english(name, 16).or_else(|| english(name, 1)), Some(ps_family)]
        .into_iter()
        .flatten()
        .chain(cff)
        .collect();
    Banned::new(reserved.iter().copied().chain(family.iter().map(String::as_str)))
}

/// The upstream PostScript name in name ID `id`, else one made from the
/// family name and `style`.
fn source_ps(name: &Name, id: u16, naming: &Naming, style: Option<&str>) -> String {
    english(name, id).filter(|ps| !ps_chars(ps, MAX_PS_LEN).is_empty()).unwrap_or_else(|| match style {
        Some(style) if !ps_chars(style, MAX_PS_LEN).is_empty() => {
            format!("{}-{}", ps_chars(&naming.family_name, MAX_PS_LEN), style)
        }
        _ => naming.family_name.clone(),
    })
}

/// New PostScript names for the fvar named instances that have one.
fn instance_ps_names(font: &FontRef, name: &Name, naming: &Naming, banned: &Banned) -> BTreeMap<u16, String> {
    let Ok(fvar) = font.fvar() else {
        return BTreeMap::new();
    };
    let Ok(instances) = fvar.instances() else {
        return BTreeMap::new();
    };
    let n = naming.number();
    let mut out = BTreeMap::new();
    for (i, inst) in instances.iter().enumerate() {
        let Ok(inst) = inst else { continue };
        let Some(id) = inst.post_script_name_id.filter(|id| id.to_u16() != 0xffff) else {
            continue;
        };
        let style = english(name, inst.subfamily_name_id.to_u16());
        let ps = if naming.licence == Licence::Ufl {
            derivative_ps(&source_ps(name, id.to_u16(), naming, style.as_deref()), n)
        } else {
            let style = style.and_then(|s| style_part(&s, banned)).unwrap_or_default();
            let ps = ps_chars(&format!("FPS-{n}-{style}"), MAX_PS_LEN);
            if banned.found_in(&ps) { format!("FPS-{n}-{i}") } else { ps }
        };
        out.insert(id.to_u16(), ps);
    }
    out
}

/// `<ps>-derivativeFPS<n>`, cutting `ps` so the whole fits PostScript rules.
fn derivative_ps(ps: &str, n: u64) -> String {
    let suffix = format!("-derivativeFPS{n}");
    let base = ps_chars(ps, MAX_PS_LEN - suffix.len());
    format!("{base}{suffix}")
}

/// Keeps the characters PostScript names allow (printable ASCII except
/// spaces and `[](){}<>/%`), at most `max` of them.
fn ps_chars(s: &str, max: usize) -> String {
    s.chars().filter(|c| c.is_ascii_graphic() && !"[](){}<>/%".contains(*c)).take(max).collect()
}

/// The English Windows string for `id`, else the first one.
fn english(name: &Name, id: u16) -> Option<String> {
    let mut matching = name.name_record.iter().filter(|r| r.name_id.to_u16() == id);
    let first = matching.clone().next()?;
    let best = matching.find(|r| r.platform_id == 3 && r.language_id == 0x409).unwrap_or(first);
    Some(best.string.as_str().to_owned())
}

fn words(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty())
}

/// Shorter banned names match whole words only: as substrings they would
/// hit ordinary style names ("pt" in "Script").
const MIN_SUBSTRING: usize = 3;

/// Names a subset must not carry over from its upstream font (Reserved Font
/// Name words, old family names). Matching ignores case and everything but
/// letters and digits, so "ABeeZeeBold" and "A Bee Zee" contain "ABeeZee".
#[derive(Debug, Default)]
pub(crate) struct Banned(Vec<Vec<char>>);

impl Banned {
    pub(crate) fn new<'a>(names: impl IntoIterator<Item = &'a str>) -> Self {
        let mut folded: Vec<Vec<char>> = names.into_iter().map(|s| fold(s).0).filter(|f| !f.is_empty()).collect();
        // Longest first, so "Roboto Flex" wins over "Roboto".
        folded.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        folded.dedup();
        Self(folded)
    }

    pub(crate) fn found_in(&self, s: &str) -> bool {
        !self.matches(s).is_empty()
    }

    /// `s` with every banned name in it replaced by `with`.
    pub(crate) fn replace(&self, s: &str, with: &str) -> String {
        let mut out = String::new();
        let mut at = 0;
        for range in self.matches(s) {
            out.push_str(&s[at..range.start.max(at)]);
            out.push_str(with);
            at = range.end.max(at);
        }
        out.push_str(&s[at..]);
        out
    }

    /// The byte ranges of `s` that spell a banned name, in order.
    fn matches(&self, s: &str) -> Vec<Range<usize>> {
        let (chars, spans) = fold(s);
        let mut taken = vec![false; chars.len()];
        let mut out = Vec::new();
        for name in &self.0 {
            let mut i = 0;
            while i + name.len() <= chars.len() {
                let end = i + name.len();
                let range = spans[i].start..spans[end - 1].end;
                if chars[i..end] == name[..] && !taken[i..end].contains(&true) && (name.len() >= MIN_SUBSTRING || whole_word(s, &range)) {
                    taken[i..end].fill(true);
                    out.push(range);
                    i = end;
                } else {
                    i += 1;
                }
            }
        }
        out.sort_by_key(|r| r.start);
        out
    }
}

/// The lowercase letters and digits of `s`, each with the byte range of the
/// character it comes from.
fn fold(s: &str) -> (Vec<char>, Vec<Range<usize>>) {
    let mut chars = Vec::new();
    let mut spans = Vec::new();
    for (at, c) in s.char_indices().filter(|(_, c)| c.is_alphanumeric()) {
        for lower in c.to_lowercase() {
            chars.push(lower);
            spans.push(at..at + c.len_utf8());
        }
    }
    (chars, spans)
}

fn whole_word(s: &str, range: &Range<usize>) -> bool {
    !s[..range.start].chars().next_back().is_some_and(char::is_alphanumeric) && !s[range.end..].chars().next().is_some_and(char::is_alphanumeric)
}

#[cfg(test)]
mod tests {
    use super::{Banned, style_part};

    #[test]
    fn banned_names_match_across_case_and_separators() {
        let banned = Banned::new(["ABeeZee", "Roboto Flex", "PT"]);
        assert!(banned.found_in("ABeeZeeBold"));
        assert!(banned.found_in("roboto-flex"));
        assert!(!banned.found_in("Script"));
        assert!(banned.found_in("PT Serif"));
        assert_eq!(style_part("Roboto Flex Bold", &banned).as_deref(), Some("Bold"));
        assert_eq!(style_part("ABeeZee-Italic", &banned).as_deref(), Some("Italic"));
        assert_eq!(style_part("ABeeZee", &banned), None);
        assert_eq!(banned.replace("Version 1.0; ABeeZee", "FPS 1"), "Version 1.0; FPS 1");
    }
}
