//! `DockPanels`: what a dock asks of the host for each panel, and the requests its bodies send the running pass.

use super::flyout::GroupAt;
use super::spec::PanelSpec;
use crate::contract::BodyCx;
use crate::menu::PanelRow;
use craft_behavior::dock::{Action, Permissions};
use egui::{Context, Id, Ui};

/// The host's panels. A dock calls, in one pass and for each group it shows:
///
/// 1. `group_view`, with the group's panels and where the group goes. When it returns a height the host took the group
///    itself: the dock shows no tab strip and no panel body for that group, and calls neither `group_header` nor `body`
///    for it. A flyout measures its group before it shows it, so a host view runs at `GroupAt::Measure` too.
/// 2. Otherwise `group_header` once, inside the group's header region before its tab strip, then `spec` for each of
///    its panels, then `body` for the panels whose bodies are shown. `panel_rows` runs when a panel's menu opens.
///
/// A panel's `spec` is read every pass, so a host may change a label or a limit between passes. The dock's actions
/// name a group by one of its panels.
pub trait DockPanels<P> {
    /// What the dock reads of `panel`.
    fn spec(&self, panel: &P) -> PanelSpec;

    /// What may happen to `panel`: whether it moves, floats, closes and takes tabs and splits (the default allows
    /// everything). A canvas is `Permissions::PROTECTED`. A locked workspace narrows these itself.
    fn permissions(&self, _panel: &P) -> Permissions {
        Permissions::ALL
    }

    /// Shows `panel`'s body in `ui`.
    fn body(&mut self, panel: &P, ui: &mut Ui, cx: &mut BodyCx<'_>);

    /// Takes the group holding `panels`, whose tab `active` is selected, in place of its tab strip and body: at
    /// `GroupAt::Show(rect)` it shows its view in `rect`, at `GroupAt::Measure(width)` it measures that view at `width`
    /// the way it shows it, and it returns the height the view takes (at `Show`, the rect's). `None` (the default)
    /// leaves the group to the dock, at both. A canvas group uses the panels to tell which document it holds and draws
    /// it at the group's rect.
    fn group_view(&mut self, _panels: &[P], _active: usize, _ui: &mut Ui, _at: GroupAt, _cx: &mut BodyCx<'_>) -> Option<f32> {
        None
    }

    /// Shows the host's controls in the header of the group holding `panels`, whose tab `active` is selected,
    /// before its tab strip (the default shows none). It does not run for a group whose `group_view` took it.
    fn group_header(&mut self, _panels: &[P], _active: usize, _ui: &mut Ui, _cx: &mut BodyCx<'_>) {}

    /// The host's rows of `panel`'s menu, shown before the dock's own rows (the default has none): command rows,
    /// which run by themselves.
    fn panel_rows(&mut self, _panel: &P) -> Vec<PanelRow> {
        Vec::new()
    }
}

/// The request lists of the dock areas over panels `P` whose pass runs on a context, the innermost last.
type Running<P> = Vec<Vec<Action<P>>>;

fn running_id() -> Id {
    Id::new("dock area requests")
}

/// Hands `actions` to the dock area over panels `P` whose pass is running on `ctx`: a panel body, or code a body
/// calls, changes the workspace the area is showing through the area, never by writing the workspace itself. The
/// area applies none of them; it returns them in `DockAreaOutput::requested`, in the order they came, for the host to
/// apply when the pass ends, in the same frame. Where areas over `P` nest, the innermost takes them.
///
/// With no such pass running, the actions come back for the caller to apply.
pub fn request_actions<P: Clone + Send + Sync + 'static>(ctx: &Context, actions: Vec<Action<P>>) -> Result<(), Vec<Action<P>>> {
    ctx.data_mut(|data| {
        let running = data.get_temp_mut_or_default::<Running<P>>(running_id());
        if let Some(open) = running.last_mut() {
            open.extend(actions);
            return Ok(());
        }
        data.remove::<Running<P>>(running_id());
        Err(actions)
    })
}

/// Opens the request list of a dock area's pass on `ctx`. The area calls it before its bodies run.
#[doc(hidden)]
pub fn open_requests<P: Clone + Send + Sync + 'static>(ctx: &Context) {
    ctx.data_mut(|data| data.get_temp_mut_or_default::<Running<P>>(running_id()).push(Vec::new()));
}

/// Closes the innermost request list on `ctx` and returns what it took. The area calls it when its bodies have run;
/// the last close leaves nothing in the context.
#[doc(hidden)]
pub fn close_requests<P: Clone + Send + Sync + 'static>(ctx: &Context) -> Vec<Action<P>> {
    ctx.data_mut(|data| {
        let running = data.get_temp_mut_or_default::<Running<P>>(running_id());
        let requested = running.pop().unwrap_or_default();
        if running.is_empty() {
            data.remove::<Running<P>>(running_id());
        }
        requested
    })
}
