//! MIDINameDocument patch names, for an existing MIDI destination.

use craft_xml::{Doctype, El, Limits, Options};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::sync::Arc;

/// Maximum encoded size in bytes of a MIDINameDocument.
pub const MAX_MIDNAM_BYTES: usize = 2 * 1024 * 1024;
const MAX_BANK_ENTRIES: usize = 65_536;
const MAX_PATCH_VISITS: usize = 1_048_576;
const MAX_OUTPUT_NAME_BYTES: usize = 16 * 1024 * 1024;
const NAME_SEPARATOR: &str = " · ";

/// Patch names for one named channel set.
#[derive(Debug, PartialEq, Eq)]
pub struct MidnamProfile {
    /// Channel set name, prefixed with the device number and model for multiple devices.
    pub name: String,
    /// Bit zero is MIDI channel 1. Assignments from all custom modes are combined.
    /// Without an assignment, use the valid declarations in AvailableForChannels.
    /// None means no hint, and Some(0) means the declared channels are all disabled.
    pub channels: Option<u16>,
    /// Bank number is MSB * 128 + LSB.
    pub banks: BTreeMap<u16, MidnamBank>,
}

/// Programs at one MIDI bank address.
#[derive(Debug, PartialEq, Eq)]
pub struct MidnamBank {
    /// Display label from the first patch bank to populate this address.
    pub name: String,
    /// Shared program labels keyed by MIDI Program Change number (0–127).
    pub programs: Arc<BTreeMap<u8, String>>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MidnamError {
    /// The file is malformed XML or exceeds an XML parser limit.
    #[error(transparent)]
    Xml(#[from] craft_xml::Error),
    /// The document contains no supported banks with named programs.
    #[error("No named MIDI banks found")]
    NoBanks,
    /// Resolving the lists would visit too many patch entries.
    #[error("Too many MIDI patches to resolve")]
    TooManyPatches,
    /// The returned profiles would retain too many bank entries.
    #[error("Too many MIDI bank entries")]
    TooManyBanks,
    /// Profile, bank, or program names exceed the allocated name budget.
    #[error("Expanded MIDI patch names exceed the size limit")]
    OutputTooLarge,
}

/// Reads channel sets from master device definitions and named lists across custom modes.
/// Duplicate list names and duplicate program labels at a bank address use the last entry.
/// Patch numbers are read from ProgramChange, never from the display Number field.
/// A DOCTYPE reference is tolerated without loading it. Internal DTD subsets are rejected.
pub fn read_midnam(bytes: &[u8]) -> Result<Vec<MidnamProfile>, MidnamError> {
    if bytes.len() > MAX_MIDNAM_BYTES {
        return Err(craft_xml::Error::TooLarge(MAX_MIDNAM_BYTES).into());
    }
    let text = decode_declared_encoding(bytes);
    let root = Options {
        doctype: Doctype::RejectInternalSubset,
        drop_blank_text: true,
        ..Options::local().strict().with_limits(Limits { max_input: text.len().max(MAX_MIDNAM_BYTES), max_depth: 32, ..Limits::DEFAULT })
    }
    .parse(&text)?;
    if root.name != "MIDINameDocument" {
        return Err(MidnamError::NoBanks);
    }
    let mut profiles = Vec::new();
    let mut budget = ReadBudget { patch_visits: MAX_PATCH_VISITS, name_bytes: MAX_OUTPUT_NAME_BYTES, bank_entries: MAX_BANK_ENTRIES };
    let multiple = root.children("MasterDeviceNames").nth(1).is_some();
    for (index, device) in root.children("MasterDeviceNames").enumerate() {
        let model = if multiple {
            let name = device.child_text("Model").map(|v| v.into_owned()).unwrap_or_default();
            Some(format!("{} · {}", index + 1, name))
        } else {
            None
        };
        let lists = device.children("PatchNameList").chain(device.children("CustomDeviceMode").flat_map(|mode| mode.children("PatchNameList")));
        let lists: BTreeMap<_, _> = lists.filter_map(|list| Some((list.attr("Name")?, list))).collect();
        let mut cache = BTreeMap::new();
        let mut assignments = None;
        for set in device.children("ChannelNameSet") {
            let Some(name) = set.attr("Name").filter(|name| !name.is_empty()) else { continue };
            let banks = read_banks(set, &lists, &mut cache, &mut budget)?;
            if !banks.is_empty() {
                let channels = assignments
                    .get_or_insert_with(|| channel_masks(device))
                    .get(name)
                    .copied()
                    .or_else(|| set.child("AvailableForChannels").and_then(availability_mask));
                let prefix_len = model.as_ref().map_or(0, |model| model.len() + NAME_SEPARATOR.len());
                charge_name_bytes(&mut budget.name_bytes, prefix_len + name.len())?;
                profiles.push(MidnamProfile {
                    name: model.as_ref().map_or_else(|| name.to_owned(), |model| format!("{model}{NAME_SEPARATOR}{name}")),
                    channels,
                    banks,
                });
            }
        }
    }
    if profiles.is_empty() { Err(MidnamError::NoBanks) } else { Ok(profiles) }
}

fn decode_declared_encoding(bytes: &[u8]) -> Cow<'_, [u8]> {
    let prefix = bytes.get(..bytes.len().min(512)).unwrap_or_default();
    let encoding = std::str::from_utf8(prefix.split(|&byte| byte == b'>').next().unwrap_or_default())
        .ok()
        .and_then(|tag| tag.strip_prefix("<?xml"))
        .filter(|rest| rest.starts_with([' ', '\t', '\r', '\n']))
        .and_then(|declaration| declaration.split_once("encoding"))
        .and_then(|(_, value)| value.trim_start().strip_prefix('='))
        .map(str::trim_start)
        .and_then(|value| {
            let quote = value.chars().next().filter(|&c| matches!(c, '\'' | '"'))?;
            value.get(1..)?.split_once(quote).map(|(name, _)| name)
        });
    match encoding.map(str::to_ascii_lowercase).as_deref() {
        Some("iso-8859-1" | "iso_8859-1" | "iso_8859-1:1987" | "latin1" | "l1" | "ibm819" | "cp819" | "csisolatin1" | "iso-ir-100") => {
            Cow::Owned(bytes.iter().copied().map(char::from).collect::<String>().into_bytes())
        }
        Some("windows-1252" | "cswindows1252" | "cp1252") => {
            Cow::Owned(bytes.iter().copied().map(craft_codepage::cp1252).collect::<String>().into_bytes())
        }
        _ => Cow::Borrowed(bytes),
    }
}

fn channel_masks(device: &El) -> BTreeMap<&str, u16> {
    let mut masks = BTreeMap::new();
    for assignment in device
        .children("CustomDeviceMode")
        .flat_map(|mode| mode.children("ChannelNameSetAssignments"))
        .flat_map(|set| set.children("ChannelNameSetAssign"))
    {
        if let Some(name) = assignment.attr("NameSet")
            && let Some(bit) = channel_bit(assignment)
        {
            *masks.entry(name).or_default() |= bit;
        }
    }
    masks
}

fn channel_bit(node: &El) -> Option<u16> {
    let n = node.attr_u32("Channel").filter(|n| (1..=16).contains(n))?;
    Some(1 << (n - 1))
}

fn availability_mask(node: &El) -> Option<u16> {
    node.children("AvailableChannel")
        .filter_map(|channel| {
            let bit = channel_bit(channel)?;
            match channel.attr("Available") {
                None | Some("true") => Some(bit),
                Some("false") => Some(0),
                _ => None,
            }
        })
        .reduce(|mask, bit| mask | bit)
}

fn byte(el: &El, attr: &str) -> Option<u8> {
    el.attr_u32(attr).and_then(|n| u8::try_from(n).ok()).filter(|&n| n < 128)
}

fn bank_select(commands: Option<&El>, mut bank: u16) -> Option<u16> {
    for cc in commands.into_iter().flat_map(|node| node.children("ControlChange")) {
        match cc.attr_u32("Control") {
            Some(0) => bank = bank % 128 + 128 * u16::from(byte(cc, "Value")?),
            Some(32) => bank = bank / 128 * 128 + u16::from(byte(cc, "Value")?),
            _ => {}
        }
    }
    Some(bank)
}

fn charge_name_bytes(remaining: &mut usize, bytes: usize) -> Result<(), MidnamError> {
    *remaining = remaining.checked_sub(bytes).ok_or(MidnamError::OutputTooLarge)?;
    Ok(())
}

struct ReadBudget {
    patch_visits: usize,
    name_bytes: usize,
    bank_entries: usize,
}

type Programs = Arc<BTreeMap<u8, String>>;
type ResolvedList = BTreeMap<u16, Programs>;

fn read_banks<'a>(
    set: &El,
    lists: &BTreeMap<&'a str, &El>,
    cache: &mut BTreeMap<(&'a str, u16), ResolvedList>,
    budget: &mut ReadBudget,
) -> Result<BTreeMap<u16, MidnamBank>, MidnamError> {
    let mut banks = BTreeMap::<u16, MidnamBank>::new();
    for bank in set.children("PatchBank") {
        let Some(address) = bank_select(bank.child("MIDICommands"), 0) else { continue };
        let name = bank.attr("Name").unwrap_or_default();
        let inline;
        let resolved = if let Some(list) = bank.child("PatchNameList") {
            inline = resolve_list(list, address, &mut budget.patch_visits, &mut budget.name_bytes)?;
            &inline
        } else {
            let Some(reference) = bank.child_attr("UsesPatchNameList", "Name").and_then(|key| lists.get_key_value(key)) else { continue };
            match cache.entry((*reference.0, address)) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => entry.insert(resolve_list(reference.1, address, &mut budget.patch_visits, &mut budget.name_bytes)?),
            }
        };
        // Cache hits avoid decoding and copying, but each address still requires a merge lookup.
        budget.patch_visits = budget.patch_visits.checked_sub(resolved.len()).ok_or(MidnamError::TooManyPatches)?;
        for (&address, programs) in resolved {
            match banks.entry(address) {
                Entry::Vacant(entry) => {
                    budget.bank_entries = budget.bank_entries.checked_sub(1).ok_or(MidnamError::TooManyBanks)?;
                    charge_name_bytes(&mut budget.name_bytes, name.len())?;
                    entry.insert(MidnamBank { name: name.to_owned(), programs: Arc::clone(programs) });
                }
                Entry::Occupied(mut entry) => {
                    let current = &mut entry.get_mut().programs;
                    if Arc::ptr_eq(current, programs) {
                        continue;
                    }
                    if Arc::strong_count(current) > 1 {
                        charge_programs(&mut budget.name_bytes, current)?;
                    }
                    charge_programs(&mut budget.name_bytes, programs)?;
                    Arc::make_mut(current).extend(programs.iter().map(|(&program, label)| (program, label.clone())));
                }
            }
        }
    }
    Ok(banks)
}

fn charge_programs(remaining: &mut usize, programs: &BTreeMap<u8, String>) -> Result<(), MidnamError> {
    charge_name_bytes(remaining, programs.values().map(|label| label.len() + std::mem::size_of::<(u8, String)>()).sum())
}

fn resolve_list(list: &El, address: u16, remaining: &mut usize, name_bytes: &mut usize) -> Result<ResolvedList, MidnamError> {
    let mut banks = BTreeMap::<u16, BTreeMap<u8, String>>::new();
    for patch in list.children("Patch") {
        *remaining = remaining.checked_sub(1).ok_or(MidnamError::TooManyPatches)?;
        let Some(label) = patch.attr("Name").filter(|label| !label.is_empty()) else { continue };
        let commands = patch.child("PatchMIDICommands");
        let Some(program) =
            commands.and_then(|node| node.child("ProgramChange")).and_then(|pc| byte(pc, "Number")).or_else(|| byte(patch, "ProgramChange"))
        else {
            continue;
        };
        let Some(address) = bank_select(commands, address) else { continue };
        charge_name_bytes(name_bytes, label.len() + std::mem::size_of::<(u8, String)>())?;
        banks.entry(address).or_default().insert(program, label.to_owned());
    }
    Ok(banks.into_iter().map(|(address, programs)| (address, Arc::new(programs))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_channel_sets_references_and_per_patch_commands() -> Result<(), MidnamError> {
        let profiles = read_midnam(br#"<!DOCTYPE MIDINameDocument SYSTEM "http://invalid.example/test.dtd">
            <MIDINameDocument><MasterDeviceNames><Model>Test</Model>
              <CustomDeviceMode><ChannelNameSetAssignments><ChannelNameSetAssign Channel="10" NameSet="Kit"/></ChannelNameSetAssignments></CustomDeviceMode>
              <ChannelNameSet Name="Kit"><PatchBank Name="Kits">
                <MIDICommands><ControlChange Control="0" Value="127"/></MIDICommands><UsesPatchNameList Name="Shared"/>
              </PatchBank></ChannelNameSet>
              <PatchNameList Name="Shared"><Patch Number="64" Name="Room &amp; Ride" ProgramChange="0"/>
                <Patch Number="8" Name="Big"><PatchMIDICommands><ControlChange Control="32" Value="1"/><ProgramChange Number="7"/></PatchMIDICommands></Patch>
                <Patch Number="6" Name="Display only"/>
              </PatchNameList>
            </MasterDeviceNames></MIDINameDocument>"#)?;
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].channels, Some(1 << 9));
        assert_eq!(*profiles[0].banks[&16256].programs, BTreeMap::from([(0, "Room & Ride".into())]));
        assert_eq!(*profiles[0].banks[&16257].programs, BTreeMap::from([(7, "Big".into())]));
        Ok(())
    }

    #[test]
    fn reads_shared_workstation_patch_lists() -> Result<(), MidnamError> {
        use std::fmt::Write;
        let mut xml = String::from("<MIDINameDocument><MasterDeviceNames>");
        for bank in 0..48 {
            write!(xml, r#"<PatchNameList Name="List {bank}">"#).unwrap();
            for program in 0..128 {
                write!(xml, r#"<Patch Name="Voice {bank}-{program}" ProgramChange="{program}"/>"#).unwrap();
            }
            xml.push_str("</PatchNameList>");
        }
        for channel in 1..=16 {
            write!(xml, r#"<ChannelNameSet Name="Part {channel}">"#).unwrap();
            for bank in 0..48 {
                write!(xml, r#"<PatchBank><MIDICommands><ControlChange Control="32" Value="{bank}"/></MIDICommands><UsesPatchNameList Name="List {bank}"/></PatchBank>"#).unwrap();
            }
            xml.push_str("</ChannelNameSet>");
        }
        xml.push_str("</MasterDeviceNames></MIDINameDocument>");
        let profiles = read_midnam(xml.as_bytes())?;
        assert_eq!(profiles.len(), 16);
        for profile in profiles {
            assert_eq!(profile.banks.len(), 48);
            assert_eq!(profile.banks[&47].programs[&127], "Voice 47-127");
        }
        Ok(())
    }

    #[test]
    fn fallback_distinguishes_missing_hints_from_explicit_channel_exclusions() -> Result<(), MidnamError> {
        for (availability, expected) in [
            ("", None),
            ("<AvailableForChannels/>", None),
            (r#"<AvailableForChannels><AvailableChannel Channel="17"/></AvailableForChannels>"#, None),
            (r#"<AvailableForChannels><AvailableChannel Channel="5" Available="false"/></AvailableForChannels>"#, Some(0)),
            (
                r#"<AvailableForChannels><AvailableChannel Channel="5" Available="false"/><AvailableChannel Channel="6"/></AvailableForChannels>"#,
                Some(1 << 5),
            ),
        ] {
            let xml = format!(
                r#"<MIDINameDocument><MasterDeviceNames><ChannelNameSet Name="Keys">{availability}
                <PatchBank><PatchNameList><Patch Name="Piano" ProgramChange="0"/></PatchNameList></PatchBank>
                </ChannelNameSet></MasterDeviceNames></MIDINameDocument>"#
            );
            assert_eq!(read_midnam(xml.as_bytes())?[0].channels, expected, "{availability}");
        }
        Ok(())
    }

    #[test]
    fn combines_valid_channel_assignments_across_custom_modes() -> Result<(), MidnamError> {
        let profiles = read_midnam(br#"<MIDINameDocument><MasterDeviceNames>
          <CustomDeviceMode><ChannelNameSetAssignments>
            <ChannelNameSetAssign Channel="1" NameSet="Normal"/>
            <ChannelNameSetAssign Channel="0" NameSet="Drums"/>
          </ChannelNameSetAssignments></CustomDeviceMode>
          <CustomDeviceMode><ChannelNameSetAssignments>
            <ChannelNameSetAssign Channel="10" NameSet="Drums"/>
            <ChannelNameSetAssign Channel="17" NameSet="Normal"/>
            <ChannelNameSetAssign Channel="2" NameSet="Normal"/>
          </ChannelNameSetAssignments></CustomDeviceMode>
          <ChannelNameSet Name="Normal"><PatchBank><PatchNameList><Patch Name="Piano" ProgramChange="0"/></PatchNameList></PatchBank></ChannelNameSet>
          <ChannelNameSet Name="Drums"><PatchBank><PatchNameList><Patch Name="Kit" ProgramChange="0"/></PatchNameList></PatchBank></ChannelNameSet>
          <ChannelNameSet Name="Other"><AvailableForChannels><AvailableChannel Channel="4"/></AvailableForChannels><PatchBank><PatchNameList><Patch Name="Flute" ProgramChange="0"/></PatchNameList></PatchBank></ChannelNameSet>
        </MasterDeviceNames></MIDINameDocument>"#)?;
        assert_eq!((profiles[0].channels, profiles[1].channels, profiles[2].channels), (Some(3), Some(1 << 9), Some(1 << 3)));
        Ok(())
    }

    #[test]
    fn cached_banks_charge_merge_work_and_keep_last_definition() -> Result<(), MidnamError> {
        let device = Options::local().strict().parse(
            br#"
          <MasterDeviceNames>
            <PatchNameList Name="A"><Patch Name="Piano" ProgramChange="0"/></PatchNameList>
            <PatchNameList Name="B"><Patch Name="Organ" ProgramChange="0"/></PatchNameList>
            <ChannelNameSet Name="Keys">
              <PatchBank Name="First"><UsesPatchNameList Name="A"/></PatchBank>
              <PatchBank><UsesPatchNameList Name="B"/></PatchBank>
              <PatchBank><UsesPatchNameList Name="A"/></PatchBank>
            </ChannelNameSet>
          </MasterDeviceNames>"#,
        )?;
        let lists: BTreeMap<_, _> = device.children("PatchNameList").filter_map(|list| Some((list.attr("Name")?, list))).collect();
        let set = device.child("ChannelNameSet").ok_or(MidnamError::NoBanks)?;
        let mut budget = ReadBudget { patch_visits: 4, name_bytes: 1024, bank_entries: 1 };
        assert_eq!(read_banks(set, &lists, &mut BTreeMap::new(), &mut budget), Err(MidnamError::TooManyPatches));

        let mut budget = ReadBudget { patch_visits: 5, name_bytes: 1024, bank_entries: 1 };
        let banks = read_banks(set, &lists, &mut BTreeMap::new(), &mut budget)?;
        assert_eq!(budget.patch_visits, 0);
        assert_eq!(banks[&0].programs[&0], "Piano");
        assert_eq!(banks[&0].name, "First");
        Ok(())
    }

    #[test]
    fn cached_unnamed_banks_count_against_the_document_limit() -> Result<(), MidnamError> {
        let device = Options::local().strict().parse(
            br#"
          <MasterDeviceNames>
            <PatchNameList Name="Shared"><Patch Name="Piano" ProgramChange="0"/></PatchNameList>
            <ChannelNameSet Name="A"><PatchBank><UsesPatchNameList Name="Shared"/></PatchBank></ChannelNameSet>
            <ChannelNameSet Name="B"><PatchBank><UsesPatchNameList Name="Shared"/></PatchBank></ChannelNameSet>
          </MasterDeviceNames>"#,
        )?;
        let lists: BTreeMap<_, _> = device.children("PatchNameList").filter_map(|list| Some((list.attr("Name")?, list))).collect();
        let mut sets = device.children("ChannelNameSet");
        let mut budget = ReadBudget { patch_visits: 3, name_bytes: 1024, bank_entries: 1 };
        let mut cache = BTreeMap::new();
        assert_eq!(read_banks(sets.next().ok_or(MidnamError::NoBanks)?, &lists, &mut cache, &mut budget)?.len(), 1);
        assert_eq!(budget.patch_visits, 1);
        assert_eq!(read_banks(sets.next().ok_or(MidnamError::NoBanks)?, &lists, &mut cache, &mut budget), Err(MidnamError::TooManyBanks));
        Ok(())
    }

    #[test]
    fn accepts_declared_latin1_and_windows_1252_aliases() -> Result<(), MidnamError> {
        for (alias, label, byte) in [("latin1", "Piéno", 0xE9), ("CP819", "Piéno", 0xE9), ("cp1252", "Pi–no", 0x96)] {
            let mut xml = format!("<?xml version='1.0' encoding='{alias}'?>").into_bytes();
            xml.extend_from_slice(
                b"<MIDINameDocument><MasterDeviceNames><ChannelNameSet Name='Keys'>\
                <PatchBank><PatchNameList><Patch Name='Pi",
            );
            xml.push(byte);
            xml.extend_from_slice(
                b"no' ProgramChange='0'/></PatchNameList></PatchBank>\
                </ChannelNameSet></MasterDeviceNames></MIDINameDocument>",
            );
            assert_eq!(read_midnam(&xml)?[0].banks[&0].programs[&0], label);
        }
        Ok(())
    }

    #[test]
    fn latin1_declarations_accept_xml_whitespace() -> Result<(), MidnamError> {
        for &separator in b" \t\r\n" {
            let mut xml = b"<?xml version='1.0' encoding='ISO-8859-1'?>\
                <MIDINameDocument><MasterDeviceNames><ChannelNameSet Name='Keys'>\
                <PatchBank><PatchNameList><Patch Name='Pi\xe9no' ProgramChange='0'/></PatchNameList></PatchBank>\
                </ChannelNameSet></MasterDeviceNames></MIDINameDocument>"
                .to_vec();
            xml[5] = separator;
            assert_eq!(read_midnam(&xml)?[0].banks[&0].programs[&0], "Piéno");
        }
        Ok(())
    }

    #[test]
    fn retains_declared_latin1_patch_names() -> Result<(), MidnamError> {
        let profiles = read_midnam(
            b"<?xml version='1.0' encoding='ISO-8859-1'?>\
            <MIDINameDocument><MasterDeviceNames><ChannelNameSet Name='Normal'>\
            <PatchBank><PatchNameList><Patch Name='Pi\xe9no' ProgramChange='0'/></PatchNameList></PatchBank>\
            </ChannelNameSet></MasterDeviceNames></MIDINameDocument>",
        )?;
        assert_eq!(profiles[0].banks[&0].programs[&0], "Piéno");
        Ok(())
    }

    #[test]
    fn rejects_internal_dtds_and_unrelated_xml() {
        assert!(matches!(read_midnam(b"<!DOCTYPE MIDINameDocument [<!ENTITY a 'a'>]><MIDINameDocument/>"), Err(MidnamError::Xml(_))));
        assert_eq!(read_midnam(b"<other/>"), Err(MidnamError::NoBanks));
    }

    #[test]
    fn bounds_names_expanded_from_shared_patch_lists_across_profiles() {
        let label = "x".repeat(256 * 1024);
        let mut xml =
            format!(r#"<MIDINameDocument><MasterDeviceNames><PatchNameList Name="Shared"><Patch Name="{label}" ProgramChange="0"/></PatchNameList>"#);
        for index in 0..65 {
            xml.push_str(&format!(
                r#"<ChannelNameSet Name="Set {index}"><PatchBank Name="Shared Bank"><MIDICommands><ControlChange Control="32" Value="{index}"/></MIDICommands><UsesPatchNameList Name="Shared"/></PatchBank></ChannelNameSet>"#
            ));
        }
        xml.push_str("</MasterDeviceNames></MIDINameDocument>");
        assert!(xml.len() < MAX_MIDNAM_BYTES);
        assert_eq!(read_midnam(xml.as_bytes()), Err(MidnamError::OutputTooLarge));
    }
}
