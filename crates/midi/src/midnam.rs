//! MIDINameDocument patch names, for an existing MIDI destination.

use craft_xml::{Doctype, El, Limits, Options};
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

pub const MAX_MIDNAM_BYTES: usize = 2 * 1024 * 1024;
const MAX_PATCH_REFERENCES: usize = 65_536;
const MAX_OUTPUT_NAME_BYTES: usize = 16 * 1024 * 1024;
const NAME_SEPARATOR: &str = " · ";

#[derive(Debug, PartialEq, Eq)]
pub struct MidnamProfile {
    pub name: String,
    /// Bit zero is MIDI channel 1. None means there was no channel hint.
    pub channels: Option<u16>,
    /// Bank number is MSB * 128 + LSB.
    pub banks: BTreeMap<u16, MidnamBank>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct MidnamBank {
    pub name: String,
    pub programs: BTreeMap<u8, String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MidnamError {
    #[error(transparent)]
    Xml(#[from] craft_xml::Error),
    #[error("No named MIDI banks found")]
    NoBanks,
    #[error("Too many MIDI patch references")]
    TooManyPatches,
    #[error("Expanded MIDI patch names exceed the size limit")]
    OutputTooLarge,
}

/// Reads channel sets from master device definitions and the default custom mode when provided.
/// Patch numbers are read from ProgramChange, never from the display Number field.
/// A DOCTYPE reference is tolerated without loading it. Internal DTD subsets are rejected.
pub fn read_midnam(bytes: &[u8]) -> Result<Vec<MidnamProfile>, MidnamError> {
    let root = Options {
        doctype: Doctype::RejectInternalSubset,
        drop_blank_text: true,
        ..Options::local().strict().with_limits(Limits { max_input: MAX_MIDNAM_BYTES, max_depth: 32, ..Limits::DEFAULT })
    }
    .parse(bytes)?;
    if root.name != "MIDINameDocument" {
        return Err(MidnamError::NoBanks);
    }
    let mut profiles = Vec::new();
    let mut remaining = MAX_PATCH_REFERENCES;
    let mut name_bytes = MAX_OUTPUT_NAME_BYTES;
    let multiple = root.children("MasterDeviceNames").nth(1).is_some();
    for (index, device) in root.children("MasterDeviceNames").enumerate() {
        let model = if multiple {
            let name = device.child_text("Model").map(|v| v.into_owned()).unwrap_or_default();
            Some(format!("{} · {}", index + 1, name))
        } else {
            None
        };
        let mode = device.child("CustomDeviceMode");
        let lists = device.children("PatchNameList").chain(mode.into_iter().flat_map(|mode| mode.children("PatchNameList")));
        let lists: BTreeMap<_, _> = lists.filter_map(|list| Some((list.attr("Name")?, list))).collect();
        let assignments = mode.and_then(|mode| mode.child("ChannelNameSetAssignments"));
        for set in device.children("ChannelNameSet") {
            let Some(name) = set.attr("Name").filter(|name| !name.is_empty()) else { continue };
            let channels =
                assignments.map(|node| mask(node.children("ChannelNameSetAssign").filter(|a| a.attr("NameSet") == Some(name)))).or_else(|| {
                    set.child("AvailableForChannels")
                        .map(|node| mask(node.children("AvailableChannel").filter(|a| a.attr("Available") != Some("false"))))
                });
            let banks = read_banks(set, &lists, &mut remaining, &mut name_bytes)?;
            if !banks.is_empty() {
                let prefix_len = model.as_ref().map_or(0, |model| model.len() + NAME_SEPARATOR.len());
                charge_name_bytes(&mut name_bytes, prefix_len + name.len())?;
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

fn mask<'a>(nodes: impl Iterator<Item = &'a El>) -> u16 {
    nodes.filter_map(|node| node.attr_u32("Channel")).filter(|n| (1..=16).contains(n)).fold(0, |mask, n| mask | (1 << (n - 1)))
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

fn read_banks(
    set: &El,
    lists: &BTreeMap<&str, &El>,
    remaining: &mut usize,
    name_bytes: &mut usize,
) -> Result<BTreeMap<u16, MidnamBank>, MidnamError> {
    let mut banks = BTreeMap::new();
    for bank in set.children("PatchBank") {
        let Some(address) = bank_select(bank.child("MIDICommands"), 0) else { continue };
        let name = bank.attr("Name").unwrap_or_default();
        let list = bank.child("PatchNameList").or_else(|| lists.get(bank.child_attr("UsesPatchNameList", "Name")?).copied());
        for patch in list.into_iter().flat_map(|node| node.children("Patch")) {
            *remaining = remaining.checked_sub(1).ok_or(MidnamError::TooManyPatches)?;
            let Some(label) = patch.attr("Name").filter(|label| !label.is_empty()) else { continue };
            let commands = patch.child("PatchMIDICommands");
            let Some(program) =
                commands.and_then(|node| node.child("ProgramChange")).and_then(|pc| byte(pc, "Number")).or_else(|| byte(patch, "ProgramChange"))
            else {
                continue;
            };
            let Some(address) = bank_select(commands, address) else { continue };
            let bank = match banks.entry(address) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => {
                    charge_name_bytes(name_bytes, name.len())?;
                    entry.insert(MidnamBank { name: name.to_owned(), programs: BTreeMap::new() })
                }
            };
            charge_name_bytes(name_bytes, label.len())?;
            bank.programs.insert(program, label.to_owned());
        }
    }
    Ok(banks)
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
        assert_eq!(profiles[0].banks[&16256].programs, BTreeMap::from([(0, "Room & Ride".into())]));
        assert_eq!(profiles[0].banks[&16257].programs, BTreeMap::from([(7, "Big".into())]));
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
                r#"<ChannelNameSet Name="Set {index}"><PatchBank Name="Shared Bank"><UsesPatchNameList Name="Shared"/></PatchBank></ChannelNameSet>"#
            ));
        }
        xml.push_str("</MasterDeviceNames></MIDINameDocument>");
        assert!(xml.len() < MAX_MIDNAM_BYTES);
        assert_eq!(read_midnam(xml.as_bytes()), Err(MidnamError::OutputTooLarge));
    }
}
