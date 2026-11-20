//! `DockPanels`: what a dock asks of the host for each panel, and the requests its bodies send the running pass.

use super::flyout::GroupAt;
use super::spec::PanelSpec;
use crate::contract::BodyCx;
use crate::menu::PanelRow;
use crate::runtime::memory;
use craft_behavior::dock::{Action, Permissions};
use egui::{Context, Id, Ui};

/// The host's panels. A dock calls, in one pass and for each group it shows:
///
/// 1. `group_view`, with the group's panels and where the group goes. When it returns a height the host took the group
///    itself: the dock shows no tab strip and no panel body for that group, and calls neither `group_header` nor `body`
///    for it. A flyout measures its group before it shows it, so a host view runs at `GroupAt::Measure` too.
/// 2. Otherwise, when the group runs (every pass, or only when something it shows changed while its chrome settles;
///    see `spec_version`): `panel_rows` for its shown panel, `group_header` once, inside the group's header region
///    before its tab strip, then `body` for the panels whose bodies are shown. A floating window's menu reads
///    `panel_rows` every pass.
///
/// A panel's `spec` is read every pass, so a host may change a label or a limit between passes; a host whose chrome
/// settles (`spec_version`) changes that version with it. The dock's actions name a group by one of its panels.
pub trait DockPanels<P> {
    /// What the dock reads of `panel`.
    fn spec(&self, panel: &P) -> PanelSpec;

    /// What may happen to `panel`: whether it moves, floats, closes and takes tabs and splits (the default allows
    /// everything). A canvas is `Permissions::PROTECTED`. A locked workspace narrows these itself.
    fn permissions(&self, _panel: &P) -> Permissions {
        Permissions::ALL
    }

    /// Shows `panel`'s body in `ui`: the group's body, inset from its edges by the panel padding. The group is the
    /// body's panel and its tab the body's title, so the body shows no framework `Panel` or padding of its own; content
    /// taller than the group scrolls in the body's own scrolling container. A document panel's body
    /// (`PanelSpec::window_kind` `Document`) gets the group's body edge to edge.
    fn body(&mut self, panel: &P, ui: &mut Ui, cx: &mut BodyCx<'_>);

    /// The version of the app data `panel`'s body shows that the framework does not track (a document's revision, a
    /// job's progress). With one, the body settles (`Lazy::settles` with `Lazy::content`): it runs only when its
    /// version, its room or something the framework tracks in it changed, the pointer is over it or focus is in it, and
    /// otherwise repaints its last run. A [`group_view`](Self::group_view) settles the same way under the version of
    /// the group's shown panel, its panels and its shown tab: the version covers what the view shows, and whether the
    /// host takes the group depends only on its panels. `None` (the default) runs the body, and the view, every pass.
    fn content(&self, _panel: &P) -> Option<u64> {
        None
    }

    /// The version of what the dock's chrome shows of the host's panels: every panel's `spec` (its name, icon and
    /// limits), its `permissions`, what `group_header` shows, and whether `panel_rows` gives any rows (the group
    /// shows its panel menu button only then; the rows themselves are read when the group runs). With one, each group's header and tab strip and each
    /// column's bar settle (`Lazy::settles` with `Lazy::content`): they run only when this version, the arrangement,
    /// their room or something the framework tracks in them changed, the pointer is over them or focus is in them, and
    /// otherwise repaint their last run. A group settles only when its shown panel's body has a [`content`](Self::content)
    /// version too. `None` (the default) runs the chrome every pass.
    fn spec_version(&self) -> Option<u64> {
        None
    }

    /// Takes the group holding `panels`, whose tab `active` is selected, in place of its tab strip and body: at
    /// `GroupAt::Show(rect)` it shows its view in `ui`, which the dock holds to `rect` and clips to it; at
    /// `GroupAt::Measure(width)` it measures that view at `width` the way it shows it. It returns the height the view
    /// takes (at `Show`, the rect's). `None` (the default) leaves the group to the dock, at both. A canvas group uses
    /// the panels to tell which document it holds and draws it in the room `ui` has. At `Show` the view settles when
    /// the shown panel has a [`content`](Self::content) version: a settled view repaints its last run instead of running.
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
    memory::unique("dock area requests")
}

/// Hands `actions` to the dock area over panels `P` whose pass is running on `ctx`: a panel body, or code a body
/// calls, changes the workspace the area is showing through the area, never by writing the workspace itself. The
/// area applies none of them; it returns them in `DockAreaOutput::requested`, in the order they came, for the host to
/// apply when the pass ends, in the same frame. Where areas over `P` nest, the innermost takes them.
///
/// With no such pass running, the actions come back for the caller to apply.
pub fn request_actions<P: Clone + Send + Sync + 'static>(ctx: &Context, actions: Vec<Action<P>>) -> Result<(), Vec<Action<P>>> {
    memory::with_session(ctx, running_id(), |running: &mut Running<P>| match running.last_mut() {
        Some(open) => {
            open.extend(actions);
            Ok(())
        }
        None => Err(actions),
    })
}

/// Opens the request list of a dock area's pass on `ctx`. The area calls it before its bodies run.
#[doc(hidden)]
pub fn open_requests<P: Clone + Send + Sync + 'static>(ctx: &Context) {
    memory::with_session(ctx, running_id(), |running: &mut Running<P>| running.push(Vec::new()));
}

/// Closes the innermost request list on `ctx` and returns what it took. The area calls it when its bodies have run.
#[doc(hidden)]
pub fn close_requests<P: Clone + Send + Sync + 'static>(ctx: &Context) -> Vec<Action<P>> {
    memory::with_session(ctx, running_id(), |running: &mut Running<P>| running.pop().unwrap_or_default())
}
