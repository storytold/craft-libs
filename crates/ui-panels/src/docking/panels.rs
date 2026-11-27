//! `DockPanels`: what a dock asks of the host for each panel, and the requests its bodies send the running pass.

use super::flyout::GroupAt;
use super::spec::PanelSpec;
use crate::contract::{BodyCx, PlainData, Revision, Tracked};
use crate::menu::PanelRow;
use crate::runtime::memory;
use craft_behavior::dock::{Action, Permissions};
use egui::{Context, Id, Ui};

/// What a dock reads of the host's panels: each panel's spec and permissions. The host keeps an implementation in a
/// [`Tracked`] and gives the dock a reference to it; the dock reads it only when the `Tracked`'s revision or the
/// workspace changed, and keeps what it read otherwise. A host changes a label or a limit by editing its spec model,
/// which moves the revision. The model is plain data, so nothing it answers can change without that edit; a spec
/// needs no style either (a tool rail's grid is [`DockGrid::rail`](super::DockGrid::rail), which the dock sizes).
pub trait PanelSpecs<P>: PlainData {
    /// What the dock reads of `panel`.
    fn spec(&self, panel: &P) -> PanelSpec;

    /// What may happen to `panel`: whether it moves, floats, closes and takes tabs and splits (the default allows
    /// everything). A canvas is `Permissions::PROTECTED`. A locked workspace narrows these itself.
    fn permissions(&self, _panel: &P) -> Permissions {
        Permissions::ALL
    }
}

/// A host's [`PanelSpecs`] in its [`Tracked`], as a dock holds it: the revision keys what the dock derives from the
/// specs. Implemented for `Tracked<S>` of every `S: PanelSpecs<P>`.
pub trait SpecSource<P> {
    /// The revision of the spec model.
    fn revision(&self) -> Revision;

    /// What the dock reads of `panel` ([`PanelSpecs::spec`]).
    fn spec(&self, panel: &P) -> PanelSpec;

    /// What may happen to `panel` ([`PanelSpecs::permissions`]).
    fn permissions(&self, panel: &P) -> Permissions;
}

impl<P, S: PanelSpecs<P>> SpecSource<P> for Tracked<S> {
    fn revision(&self) -> Revision {
        Tracked::revision(self)
    }

    fn spec(&self, panel: &P) -> PanelSpec {
        PanelSpecs::spec(&**self, panel)
    }

    fn permissions(&self, panel: &P) -> Permissions {
        PanelSpecs::permissions(&**self, panel)
    }
}

/// The host's panel views. A dock calls, in one pass and for each group it shows:
///
/// 1. `group_view`, with the group's panels and where the group goes. When it returns a height the host took the group
///    itself: the dock shows no tab strip and no panel body for that group, and calls neither `group_header` nor `body`
///    for it. A flyout measures its group before it shows it, so a host view runs at `GroupAt::Measure` too.
/// 2. Otherwise `group_header` once, inside the group's header before its tab strip, then `body` for the panels whose
///    bodies are shown, and `footer` under the body of a shown panel whose spec has `PanelSpec::footer`.
///
/// Each of these runs every pass the group runs. Where every shown panel of a column has a version
/// ([`content`](Self::content)), the column runs only when one of them, the arrangement, the specs or its place
/// changed, or input reached it. A host memoizes inside them: a body or view shown through a
/// [`MemoView`](crate::contract::MemoView) runs only when its props changed or input reached it, and otherwise repaints
/// its last run. The dock's own chrome (each group's tab row, each column's bar) is memoized on what it shows: the
/// panels' specs ([`PanelSpecs`]), the arrangement and the look.
///
/// `panel_rows` is read while the group's menu is open. The dock's actions name a group by one of its panels.
pub trait DockPanels<P> {
    /// Shows `panel`'s body in `ui`: the group's body, inset from its edges by the panel padding. The group is the
    /// body's panel and its tab the body's title, so the body shows no framework `Panel` or padding of its own; content
    /// taller than the group scrolls in the body's own scrolling container. A document panel's body
    /// (`PanelSpec::window_kind` `Document`) gets the group's body edge to edge. The body lays out with the group's
    /// spacing, or with the style of the `Ui` the dock was shown in where the panel's body is the app's own egui code
    /// (`PanelSpec::host_body`).
    fn body(&mut self, panel: &P, ui: &mut Ui, cx: &mut BodyCx<'_>);

    /// Shows the fixed trailing controls of the rail containing `panel`, in top-to-bottom order. The dock calls this
    /// only when the area names a panel with `DockArea::rail_footer`; the controls remain in view while icons scroll.
    fn rail_footer(&mut self, _panel: &P, _ui: &mut Ui, _cx: &mut BodyCx<'_>) {}

    /// Shows `panel`'s footer in `ui`: a footer bar pinned under the body, across the group, that stays in view while
    /// the body scrolls (a panel's actions on everything its body lists). The dock calls it only for a panel whose
    /// spec has `PanelSpec::footer` (the default shows nothing).
    fn footer(&mut self, _panel: &P, _ui: &mut Ui, _cx: &mut BodyCx<'_>) {}

    /// The version of the app data the host shows for `panel` that the framework does not track (a document's
    /// revision, a job's progress): everything its `body` and `footer` show, and the `group_header`, `group_view` and
    /// `rail_footer` of a group or rail whose shown panel it is.
    ///
    /// The dock shows each column, and its docked tree, as a memoized view keyed by the versions of the panels shown
    /// in it, the arrangement, the specs and the place: while those hold and no input reaches it, the column repaints
    /// its last run and calls none of these methods. A column or tree that shows a panel without a version (`None`,
    /// the default) runs every pass. The host memoizes inside its bodies itself, for the passes a column runs.
    fn content(&self, _panel: &P) -> Option<u64> {
        None
    }

    /// Takes the group holding `panels`, whose tab `active` is selected, in place of its tab strip and body: at
    /// `GroupAt::Show(rect)` it shows its view in `ui`, which the dock holds to `rect` and clips to it; at
    /// `GroupAt::Measure(width)` it measures that view at `width` the way it shows it. It returns the height the view
    /// takes (at `Show`, the rect's). `None` (the default) leaves the group to the dock, at both. A canvas group uses
    /// the panels to tell which document it holds and draws it in the room `ui` has.
    fn group_view(&mut self, _panels: &[P], _active: usize, _ui: &mut Ui, _at: GroupAt, _cx: &mut BodyCx<'_>) -> Option<f32> {
        None
    }

    /// Shows the host's controls in the header of the group holding `panels`, whose tab `active` is selected,
    /// before its tab strip (the default shows none). It does not run for a group whose `group_view` took it.
    fn group_header(&mut self, _panels: &[P], _active: usize, _ui: &mut Ui, _cx: &mut BodyCx<'_>) {}

    /// The host's rows of `panel`'s menu, shown before the dock's own rows (the default has none): command rows,
    /// which run by themselves. Read while the menu of `panel`'s group is open, and once per pass for whether the
    /// group shows its panel menu button.
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
