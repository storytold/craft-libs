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
