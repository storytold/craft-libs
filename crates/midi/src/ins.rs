//! Named MIDI banks in Cakewalk instrument definitions.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Maximum size of an instrument definition read by the patch-name importer.
pub const MAX_INS_BYTES: usize = 2 * 1024 * 1024;
const MAX_EXPANDED_PATCH_BYTES: usize = 32 * 1024 * 1024;

/// Patch banks for one instrument definition.
#[derive(Debug, PartialEq, Eq)]
pub struct InsInstrument {
    /// Name of the instrument section.
    pub name: String,
    /// Bank keys use the normal MIDI bank number: MSB * 128 + LSB.
    pub banks: BTreeMap<u16, Arc<InsBank>>,
    /// The wildcard list from `Patch[*]`.
    pub default_bank: Option<Arc<InsBank>>,
}

/// A named patch list, shared among instruments that reference it.
#[derive(Debug, PartialEq, Eq)]
pub struct InsBank {
    /// The spelling from the patch-list section.
    pub name: String,
    /// Program-change numbers from 0 to 127 and their display labels.
    pub programs: BTreeMap<u8, String>,
}

/// An instrument definition could not be read.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InsError {
    /// The input or expanded patch names exceed their size limit.
    #[error("The instrument definition is too large")]
    TooLarge,
    /// There are no usable patch names for standard bank select.
    #[error("No named MIDI banks with standard bank select found")]
    NoBanks,
    /// A referenced patch list inherits from itself, directly or indirectly.
    #[error("Cyclic BasedOn references in Cakewalk patch lists")]
    CyclicPatchLists,
}

#[derive(Default)]
struct PatchList<'a> {
    base: Option<String>,
    programs: BTreeMap<u8, &'a str>,
}

#[derive(Default)]
struct Definition<'a> {
    method: Option<&'a str>,
    banks: BTreeMap<u16, String>,
    default_bank: Option<String>,
}

enum Section<'map, 'text> {
    Patch(&'map mut PatchList<'text>),
    Instrument(&'map mut Definition<'text>),
    Other,
}

/// Read the patch names and instrument profiles from UTF-8 or Windows-1252 Cakewalk .ins text.
///
/// Controller names, note names and profiles requiring nonstandard bank-select methods are
/// omitted. Patch lists inherit names through `BasedOn` and keep their own overrides.
/// Patch-list references ignore ASCII case. Missing bases contribute no names. Cycles in
/// referenced lists are rejected. Instrument sections with the same spelling are combined,
/// and profiles are returned in name order. A semicolon starts a comment at the start of a line.
/// The caller can use a profile's wildcard patch list when a bank has no explicit list.
pub fn read_ins(bytes: &[u8]) -> Result<Vec<InsInstrument>, InsError> {
    if bytes.len() > MAX_INS_BYTES {
        return Err(InsError::TooLarge);
    }
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => Cow::Borrowed(text),
        Err(_) => Cow::Owned(bytes.iter().copied().map(craft_codepage::cp1252).collect::<String>()),
    };
    let mut patch_names = BTreeMap::<String, (&str, PatchList)>::new();
    let mut instruments = BTreeMap::<&str, Definition>::new();
    let mut section = Section::Other;
    let mut heading = "";
    for line in text.trim_start_matches('\u{feff}').split(['\r', '\n']) {
        let line = line.trim();
        if line.starts_with(';') {
            continue;
        }
        if let Some(value) = line.strip_prefix('.') {
            heading = value;
            section = Section::Other;
            continue;
        }
        if let Some(value) = line.strip_prefix('[').and_then(|name| name.strip_suffix(']')) {
            let name = value.trim();
            section = if !name.is_empty() && heading.eq_ignore_ascii_case("Patch Names") {
                Section::Patch(&mut patch_names.entry(name.to_ascii_lowercase()).or_insert_with(|| (name, PatchList::default())).1)
            } else if !name.is_empty() && heading.eq_ignore_ascii_case("Instrument Definitions") {
                Section::Instrument(instruments.entry(name).or_default())
            } else {
                Section::Other
            };
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let key = key.trim();
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match &mut section {
            Section::Patch(list) => {
                if key.eq_ignore_ascii_case("BasedOn") {
                    list.base = Some(value.to_ascii_lowercase());
                } else if let Ok(program @ 0..=127) = key.parse::<u8>() {
                    list.programs.insert(program, value);
                }
            }
            Section::Instrument(def) => {
                if key.eq_ignore_ascii_case("BankSelMethod") {
                    def.method = Some(value);
                } else if let Some((prefix, bank)) = key.split_once('[')
                    && prefix.eq_ignore_ascii_case("Patch")
                    && let Some(bank) = bank.strip_suffix(']').map(str::trim)
                {
                    if bank == "*" {
                        def.default_bank = Some(value.to_ascii_lowercase());
                    } else if let Ok(bank @ 0..=16_383) = bank.parse::<u16>() {
                        def.banks.insert(bank, value.to_ascii_lowercase());
                    }
                }
            }
            Section::Other => {}
        }
    }
    let roots = instruments
        .values()
        .filter(|def| def.method.is_none_or(|m| m == "0"))
        .flat_map(|def| def.banks.values().chain(def.default_bank.iter()).map(String::as_str));
    let patch_names = resolve_patch_lists(&patch_names, roots)?;
    let result: Vec<_> = instruments
        .iter()
        .filter(|(_, def)| def.method.is_none_or(|m| m == "0"))
        .filter_map(|(name, def)| {
            let default_bank = def.default_bank.as_deref().and_then(|list| patch_names.get(list)).filter(|list| !list.programs.is_empty()).cloned();
            let banks: BTreeMap<_, _> = def
                .banks
                .iter()
                .filter_map(|(&bank, list)| {
                    let list = patch_names.get(list.as_str())?;
                    (!list.programs.is_empty()).then(|| (bank, Arc::clone(list)))
                })
                .collect();
            if banks.is_empty() && default_bank.is_none() {
                return None;
            }
            Some(InsInstrument { name: (*name).to_owned(), banks, default_bank })
        })
        .collect();
    if result.is_empty() { Err(InsError::NoBanks) } else { Ok(result) }
}

fn resolve_patch_lists<'a>(
    lists: &'a BTreeMap<String, (&str, PatchList<'_>)>,
    roots: impl Iterator<Item = &'a str>,
) -> Result<BTreeMap<&'a str, Arc<InsBank>>, InsError> {
    let mut resolved = BTreeMap::<&str, Arc<InsBank>>::new();
    let mut remaining = MAX_EXPANDED_PATCH_BYTES;
    for name in roots {
        if resolved.contains_key(name) {
            continue;
        }
        let mut chain = Vec::new();
        let mut visiting = BTreeSet::new();
        let mut current = Some(name);
        while let Some(name) = current {
            if resolved.contains_key(name) {
                break;
            }
            let Some(&(spelling, ref list)) = lists.get(name) else { break };
            if !visiting.insert(name) {
                return Err(InsError::CyclicPatchLists);
            }
            chain.push((name, spelling, list));
            current = list.base.as_deref();
        }
        for (name, spelling, list) in chain.into_iter().rev() {
            let base = list.base.as_deref().and_then(|name| resolved.get(name));
            let mut programs: BTreeMap<u8, &str> =
                base.into_iter().flat_map(|bank| bank.programs.iter().map(|(&program, label)| (program, label.as_str()))).collect();
            programs.extend(list.programs.iter().map(|(&program, &label)| (program, label)));
            let bytes = programs.values().map(|label| label.len() + std::mem::size_of::<(u8, String)>()).sum::<usize>();
            remaining = remaining.checked_sub(spelling.len() + bytes).ok_or(InsError::TooLarge)?;
            let programs = programs.into_iter().map(|(program, label)| (program, label.to_owned())).collect();
            resolved.insert(name, Arc::new(InsBank { name: spelling.to_owned(), programs }));
        }
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reopened_patch_section_keeps_earlier_programs_and_first_spelling() -> Result<(), InsError> {
        let input = b".Patch Names\n[Grand]\n0=Piano\n[Other]\n0=Strings\n[GRAND]\n1=Organ\n\
            .Note Names\n[GRAND]\n2=Ignored\n.Instrument Definitions\n[Synth]\nPatch[0]=grand";
        let instruments = read_ins(input)?;
        let bank = &instruments[0].banks[&0];
        assert_eq!(bank.name, "Grand");
        assert_eq!(bank.programs, BTreeMap::from([(0, "Piano".into()), (1, "Organ".into())]));
        Ok(())
    }

    #[test]
    fn resolves_programs_by_profile_and_packed_bank() -> Result<(), InsError> {
        let bank = b"\xef\xbb\xbf.Patch Names\r\n[Lead]\r\n0=First=Lead\r\n; comment\r\n128=Invalid\n\
            [Drums]\n127=Last Kit\n.Instrument Definitions\n[Melodic]\nPatch[6192]=Lead\nPatch[*]=Lead\n\
            Patch[16384]=Lead\nPatch[9]=Undefined\n[Percussion]\nBankSelMethod=0\nPatch[6192]=Drums\n\
            [Other method]\nBankSelMethod=1\nPatch[0]=Lead\n.Note Names\n[Drums]\n127=Ignored";
        let instruments = read_ins(bank)?;
        assert_eq!(instruments.len(), 2);
        let melodic = &instruments[0];
        let percussion = &instruments[1];
        assert_eq!(melodic.banks.len(), 1);
        assert_eq!(melodic.banks[&6192].programs, BTreeMap::from([(0, "First=Lead".into())]));
        assert!(melodic.default_bank.as_ref().is_some_and(|bank| Arc::ptr_eq(bank, &melodic.banks[&6192])));
        assert_eq!(percussion.banks[&6192].programs[&127], "Last Kit");
        Ok(())
    }

    #[test]
    fn accepts_utf8_and_windows_1252_names() -> Result<(), InsError> {
        for input in [
            ".Patch Names\n[GM]\n1=Piéno – ™\n.Instrument Definitions\n[Synth]\nPatch[0]=GM".as_bytes(),
            b".Patch Names\n[GM]\n1=Pi\xe9no \x96 \x99\n.Instrument Definitions\n[Synth]\nPatch[0]=GM",
        ] {
            assert_eq!(read_ins(input)?[0].banks[&0].programs[&1], "Piéno – ™");
        }
        Ok(())
    }

    #[test]
    fn ignores_unreferenced_cycles() -> Result<(), InsError> {
        let input = b".Patch Names\n[GM]\n0=Piano\n[A]\nBasedOn=B\n[B]\nBasedOn=A\n\
            .Instrument Definitions\n[Synth]\nPatch[0]=GM";
        assert_eq!(read_ins(input)?[0].banks[&0].programs[&0], "Piano");
        Ok(())
    }

    #[test]
    fn retains_semicolons_in_patch_names() -> Result<(), InsError> {
        let input = b".Patch Names\n[GM]\n ; ignored\n0=Pad; warm\n.Instrument Definitions\n[Synth]\nPatch[0]=GM";
        assert_eq!(read_ins(input)?[0].banks[&0].programs[&0], "Pad; warm");
        Ok(())
    }

    #[test]
    fn resolves_section_references_case_insensitively() -> Result<(), InsError> {
        let input =
            b".Patch Names\n[GM Voices]\n0=Piano\n[Extra]\nBasedOn=GM VOICES\n.Instrument Definitions\n[Synth]\nPatch[0]=extra\nPatch[*]=gm voices";
        let profiles = read_ins(input)?;
        assert_eq!(profiles[0].banks[&0].programs[&0], "Piano");
        assert_eq!(profiles[0].default_bank.as_ref().unwrap().name, "GM Voices");
        Ok(())
    }

    #[test]
    fn accepts_cr_only_line_endings() -> Result<(), InsError> {
        let input = b".Patch Names\r[GM]\r0=Piano\r.Instrument Definitions\r[Synth]\rPatch[0]=GM";
        assert_eq!(read_ins(input)?[0].banks[&0].programs[&0], "Piano");
        Ok(())
    }

    #[test]
    fn rejects_unusable_inputs() {
        assert_eq!(read_ins(b".Patch Names\n[Voices]\n0=Lead"), Err(InsError::NoBanks));
        assert_eq!(read_ins(b"\xff"), Err(InsError::NoBanks));
        assert_eq!(read_ins(&vec![0; MAX_INS_BYTES + 1]), Err(InsError::TooLarge));
    }

    #[test]
    fn inherits_patch_lists_and_applies_local_overrides() -> Result<(), InsError> {
        let input = b".Patch Names\n[Base]\n0=Piano\n1=Organ\n[Inherited]\nBasedOn=Base\n\
            [Override]\nbasedon=Inherited\n1=Strings\n[Missing]\nBasedOn=Unknown\n2=Flute\n\
            .Instrument Definitions\n[Keyboard]\nPatch[0]=Inherited\nPatch[1]=Override\nPatch[2]=Missing";
        let profiles = read_ins(input)?;
        let banks = &profiles[0].banks;
        assert_eq!(banks[&0].programs, BTreeMap::from([(0, "Piano".into()), (1, "Organ".into())]));
        assert_eq!(banks[&1].programs, BTreeMap::from([(0, "Piano".into()), (1, "Strings".into())]));
        assert_eq!(banks[&2].programs, BTreeMap::from([(2, "Flute".into())]));
        Ok(())
    }

    #[test]
    fn rejects_cyclic_patch_list_inheritance() {
        let input = b".Patch Names\n[A]\nBasedOn=B\n[B]\nBasedOn=A\n.Instrument Definitions\n[Keyboard]\nPatch[0]=A";
        assert_eq!(read_ins(input), Err(InsError::CyclicPatchLists));
    }
}
