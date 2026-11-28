//! `ui.dock.apply` on a workspace, with the host's permissions and lock.

use craft_automation::dock::{dock_method, inspect};
use craft_behavior::DockSide;
use craft_behavior::dock::{Action, DockError, DockPlacement, Layout, Node, PanelState, Permissions, Workspace};
use craft_ui::contract::PlainData;
use craft_ui::docking::{PanelSpec, PanelSpecs};
use craft_ui::testing::trial;
use serde_json::{Value, json};

/// Two panels, the first protected as a canvas is.
#[derive(PlainData)]
struct Panels;

impl PanelSpecs<u8> for Panels {
    fn spec(&self, panel: &u8) -> PanelSpec {
        PanelSpec::new(format!("Panel {panel}"))
    }

    fn permissions(&self, panel: &u8) -> Permissions {
        if *panel == 1 { Permissions::PROTECTED } else { Permissions::ALL }
    }
}

fn call(workspace: &mut Workspace<u8>, locked: bool, params: Value) -> Result<Value, String> {
    dock_method(workspace, &Panels, locked, "ui.dock.apply", &params).expect("a ui.dock method")
}

/// A JSON action applies to the workspace with the host's permissions and lock; a refused one replies the dock error
/// and changes nothing, and `inspect` lists the workspace.
#[trial]
fn a_dock_action_from_json_applies_to_the_workspace() {
    let mut workspace = Workspace::new(Layout::new(Some(Node::tabs(vec![1u8, 2], 0)), Vec::new()));
    assert_eq!(workspace.layout().panel_state(&2), PanelState::Open);

    assert_eq!(call(&mut workspace, false, json!({"action": {"Activate": {"panel": 2}}})), Ok(json!({"applied": []})));
    assert_eq!(workspace.layout().panel_state(&2), PanelState::Front, "the activate action brings panel 2 to the front");

    let before = workspace.clone();
    assert_eq!(
        call(&mut workspace, false, json!({"action": Action::close(1u8)})),
        Err(DockError::NotAllowed.to_string()),
        "the host's permissions protect panel 1"
    );
    assert_eq!(workspace, before, "a refused action leaves the workspace as it was");
    assert!(call(&mut workspace, false, json!({"action": Action::activate(1u8), "panel": 1})).is_err(), "an unknown key is refused");
    assert!(call(&mut workspace, false, json!({})).is_err(), "a missing action is refused");
    assert!(call(&mut workspace, false, json!({"action": {"Activate": {}}})).is_err());
    assert_eq!(workspace, before);

    let split = Action::move_beside(2u8, 1, DockPlacement::Split(DockSide::End));
    assert_eq!(call(&mut workspace, true, json!({"action": split})), Err(DockError::NotAllowed.to_string()), "a locked workspace refuses a move");
    assert_eq!(workspace, before);
    assert_eq!(call(&mut workspace, true, json!({"action": Action::activate(1u8)})), Ok(json!({"applied": []})), "a locked workspace activates");
    assert_eq!(workspace.layout().panel_state(&1), PanelState::Front);

    assert_eq!(call(&mut workspace, false, json!({"action": Action::close(2u8)})), Ok(json!({"applied": [2]})), "the reply lists the closed panel");
    assert_eq!(workspace.layout().panel_state(&2), PanelState::Hidden);

    assert!(dock_method(&mut workspace, &Panels, false, "ui.dock.close", &json!({})).is_none());

    let listed = inspect(&workspace);
    assert_eq!(serde_json::from_value::<Workspace<u8>>(listed).ok(), Some(workspace.clone()), "inspect lists the whole workspace");
    assert_eq!(workspace.layout().panels(), vec![&1]);
}
