//! The key table's collection rows.

use craft_behavior::Intent;
use craft_behavior::keys::{KeyRole, intent};
use craft_host::target::Os;
use craft_keymap::{Key, Mods};

/// On every platform F2 renames the lead of a renamable collection and Enter is left to activation.
#[test]
fn f2_renames_and_enter_activates_on_every_platform() {
    let role = KeyRole::new().rename(true).space(true).moves(true);
    for os in [Os::Mac, Os::Windows, Os::Linux] {
        assert_eq!(intent(os, Key::F2, Mods::NONE, role), Some(Intent::Rename), "{os:?}: F2 renames");
        assert_eq!(intent(os, Key::Enter, Mods::NONE, role), None, "{os:?}: Enter is activation, not rename");
        assert_ne!(intent(os, Key::Down, Mods::CMD, role), Some(Intent::Activate), "{os:?}: Cmd+Down does not activate");
    }
}

/// Ctrl+Space (Control on macOS) toggles the lead's pick only where several items can be picked; elsewhere it is the
/// app's.
#[test]
fn ctrl_space_toggles_the_pick_only_in_a_multi_select_collection() {
    let single = KeyRole::new().space(true).moves(true);
    let multi = single.multi_select(true);
    for (os, mods) in [(Os::Mac, Mods::CTRL), (Os::Windows, Mods::CMD), (Os::Linux, Mods::CMD)] {
        assert_eq!(intent(os, Key::Space, mods, multi), Some(Intent::TogglePick), "{os:?}: multi-select");
        assert_eq!(intent(os, Key::Space, mods, single), None, "{os:?}: single-select");
    }
    assert_eq!(intent(Os::Mac, Key::Space, Mods::CMD, multi), None, "Cmd+Space stays the system's on macOS");
}

/// Shift with Left or Right is no move in rows stacked one per line, so the app's keys get it; a grid keeps it as an
/// extending move.
#[test]
fn shift_left_and_right_are_no_moves_in_stacked_rows() {
    let grid = KeyRole::new().space(true).moves(true);
    let stacked = grid.stacked(true);
    for os in [Os::Mac, Os::Windows] {
        for key in [Key::Left, Key::Right] {
            assert_eq!(intent(os, key, Mods::SHIFT, stacked), None, "{os:?} {key:?}: stacked rows");
            assert!(matches!(intent(os, key, Mods::SHIFT, grid), Some(Intent::Move(_, mods)) if mods.extend), "{os:?} {key:?}: a grid extends");
            assert!(matches!(intent(os, key, Mods::NONE, stacked), Some(Intent::Move(..))), "{os:?} {key:?}: plain arrows still move");
        }
    }
}

/// Off macOS Ctrl+Page Up and Page Down switch pages, except on a document canvas, where Cmd or Ctrl turns the page
/// moves sideways on every platform.
#[test]
fn cmd_or_ctrl_page_keys_scroll_a_document_sideways_and_switch_pages_elsewhere() {
    let document = KeyRole::new().nudge(true).document(true);
    for os in [Os::Mac, Os::Windows, Os::Linux] {
        let lead = matches!(intent(os, Key::PageDown, Mods::CMD, document), Some(Intent::Move(craft_behavior::NavMove::PageDown, mods)) if mods.lead);
        assert!(lead, "{os:?}: a page move with lead on a document");
    }
    assert_eq!(intent(Os::Windows, Key::PageDown, Mods::CMD, KeyRole::new()), Some(Intent::switch_page(true)), "a tab strip's Ctrl+Page Down");
}
