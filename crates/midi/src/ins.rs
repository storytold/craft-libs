//! Named MIDI banks in Cakewalk instrument definitions.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Maximum size of an instrument definition read by the patch-name importer.
pub const MAX_INS_BYTES: usize = 2 * 1024 * 1024;
const MAX_EXPANDED_PATCH_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct InsInstrument {
    pub name: String,
    /// Bank keys use the normal MIDI bank number: MSB * 128 + LSB.
    pub banks: BTreeMap<u16, Arc<InsBank>>,
    pub default_bank: Option<Arc<InsBank>>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct InsBank {
    pub name: String,
    pub programs: BTreeMap<u8, String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InsError {
    #[error("The instrument definition is too large")]
    TooLarge,
    #[error("The instrument definition must use UTF-8 text")]
    Encoding,
    #[error("No named MIDI banks with standard bank select found")]
    NoBanks,
    #[error("Cyclic BasedOn references in Cakewalk patch lists")]
    CyclicPatchLists,
}

#[derive(Default)]
struct PatchList<'a> {
    base: Option<&'a str>,
    programs: BTreeMap<u8, &'a str>,
}

#[derive(Default)]
struct Definition<'a> {
    method: Option<&'a str>,
    banks: BTreeMap<u16, &'a str>,
    default_bank: Option<&'a str>,
}

/// Read the patch names and instrument profiles from a UTF-8 (or ASCII) Cakewalk .ins file.
///
/// Controller names, note names and profiles requiring nonstandard bank-select methods are
/// omitted. Patch lists inherit names through `BasedOn` and keep their own overrides.
/// Missing bases contribute no names. Cyclic inheritance is rejected.
/// The caller can use a profile's wildcard patch list when a bank has no explicit list.
pub fn read_ins(bytes: &[u8]) -> Result<Vec<InsInstrument>, InsError> {
    if bytes.len() > MAX_INS_BYTES {
        return Err(InsError::TooLarge);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| InsError::Encoding)?;
    let mut patch_names = BTreeMap::<&str, PatchList>::new();
    let mut instruments = BTreeMap::<&str, Definition>::new();
    let mut section = "";
    let mut heading = "";
    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.split(';').next().unwrap_or_default().trim();
        if let Some(value) = line.strip_prefix('.') {
            heading = value;
            section = "";
            continue;
        }
        if let Some(value) = line.strip_prefix('[').and_then(|name| name.strip_suffix(']')) {
            section = value.trim();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let key = key.trim();
        let value = value.trim();
        if section.is_empty() || value.is_empty() {
            continue;
        }
        if heading.eq_ignore_ascii_case("Patch Names") {
            if key.eq_ignore_ascii_case("BasedOn") {
                patch_names.entry(section).or_default().base = Some(value);
            } else if let Ok(program @ 0..=127) = key.parse::<u8>() {
                patch_names.entry(section).or_default().programs.insert(program, value);
            }
        } else if heading.eq_ignore_ascii_case("Instrument Definitions") {
            let def = instruments.entry(section).or_default();
            if key.eq_ignore_ascii_case("BankSelMethod") {
                def.method = Some(value);
            } else if let Some((prefix, bank)) = key.split_once('[')
                && prefix.eq_ignore_ascii_case("Patch")
                && let Some(bank) = bank.strip_suffix(']').map(str::trim)
            {
                if bank == "*" {
                    def.default_bank = Some(value);
                } else if let Ok(bank @ 0..=16_383) = bank.parse::<u16>() {
                    def.banks.insert(bank, value);
                }
            }
        }
    }
    let patch_names = resolve_patch_lists(&patch_names)?;
    let result: Vec<_> = instruments
        .into_iter()
        .filter(|(_, def)| def.method.is_none_or(|m| m == "0"))
        .filter_map(|(name, def)| {
            let default_bank = def.default_bank.and_then(|list| patch_names.get(list)).filter(|list| !list.programs.is_empty()).cloned();
            let banks: BTreeMap<_, _> = def
                .banks
                .into_iter()
                .filter_map(|(bank, list)| {
                    let list = patch_names.get(list)?;
                    (!list.programs.is_empty()).then(|| (bank, Arc::clone(list)))
                })
                .collect();
            if banks.is_empty() && default_bank.is_none() {
                return None;
            }
            Some(InsInstrument { name: name.to_owned(), banks, default_bank })
        })
        .collect();
    if result.is_empty() { Err(InsError::NoBanks) } else { Ok(result) }
}

fn resolve_patch_lists<'a>(lists: &BTreeMap<&'a str, PatchList<'a>>) -> Result<BTreeMap<&'a str, Arc<InsBank>>, InsError> {
    let mut resolved = BTreeMap::<&str, Arc<InsBank>>::new();
    let mut remaining = MAX_EXPANDED_PATCH_BYTES;
    for &name in lists.keys() {
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
            let Some(list) = lists.get(name) else { break };
            if !visiting.insert(name) {
                return Err(InsError::CyclicPatchLists);
            }
            chain.push((name, list));
            current = list.base;
        }
        for (name, list) in chain.into_iter().rev() {
            let base = list.base.and_then(|name| resolved.get(name));
            let mut programs: BTreeMap<u8, &str> =
                base.into_iter().flat_map(|bank| bank.programs.iter().map(|(&program, label)| (program, label.as_str()))).collect();
            programs.extend(list.programs.iter().map(|(&program, &label)| (program, label)));
            let bytes = programs.values().map(|label| label.len() + std::mem::size_of::<(u8, String)>()).sum::<usize>();
            remaining = remaining.checked_sub(name.len() + bytes).ok_or(InsError::TooLarge)?;
            let programs = programs.into_iter().map(|(program, label)| (program, label.to_owned())).collect();
            resolved.insert(name, Arc::new(InsBank { name: name.to_owned(), programs }));
        }
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_programs_by_profile_and_packed_bank() -> Result<(), InsError> {
        let bank = b"\xef\xbb\xbf.Patch Names\r\n[Lead]\r\n0=First=Lead ; comment\r\n128=Invalid\n\
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
    fn rejects_unusable_inputs() {
        assert_eq!(read_ins(b".Patch Names\n[Voices]\n0=Lead"), Err(InsError::NoBanks));
        assert_eq!(read_ins(b"\xff"), Err(InsError::Encoding));
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
