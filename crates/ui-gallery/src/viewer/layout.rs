//! The viewer's dock layouts: the panels of each workspace, the default arrangements, what the user may do with a
//! panel, and the saved form of a layout.
//!
//! A saved layout names a component tab by its component's registered name. A name the catalog no longer has loads as a
//! panel the reconciliation drops, so a layout saved by another build costs at most its component tabs.

use super::catalog::display_name;
use super::state::Workspace;
use craft_behavior::Axis;
use craft_behavior::dock::{Column, DockEdge, Group, Layout, Node, Permissions, SplitSize, Workspace as Dock};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::{Mutex, PoisonError};

/// A panel of the viewer's dock layouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PanelId {
    /// The component tree.
    Components,
    /// The target component's stories.
    Stories,
    /// The target story's facts and values.
    Properties,
    /// The decision record of the target component.
    Inspect,
    /// The token browser.
    Tokens,
    /// The centre's first tab: every component of one level.
    Overview,
    /// A centre tab with every story of one component, by the component's registered type name.
    Component(&'static str),
    /// The theme workspace's centre: the mock app or the Controls level.
    Preview,
    /// The picked token's detail.
    Detail,
    /// The theme's contrast problems.
    Problems,
    /// The app sizes editor.
    AppSizes,
}

impl PanelId {
    /// The panel's title.
    pub fn title(self) -> String {
        match self {
            PanelId::Components => "Components".to_owned(),
            PanelId::Stories => "Stories".to_owned(),
            PanelId::Properties => "Properties".to_owned(),
            PanelId::Inspect => "Inspect".to_owned(),
            PanelId::Tokens => "Tokens".to_owned(),
            PanelId::Overview => "Overview".to_owned(),
            PanelId::Component(type_name) => display_name(type_name),
            PanelId::Preview => "Preview".to_owned(),
            PanelId::Detail => "Detail".to_owned(),
            PanelId::Problems => "Problems".to_owned(),
            PanelId::AppSizes => "App sizes".to_owned(),
        }
    }

    /// Whether the layout of `workspace` holds the panel. The token browser is in both.
    pub fn in_workspace(self, workspace: Workspace) -> bool {
        match self {
            PanelId::Components | PanelId::Stories | PanelId::Properties | PanelId::Inspect | PanelId::Overview | PanelId::Component(_) => {
                workspace == Workspace::Components
            }
            PanelId::Tokens => true,
            PanelId::Preview | PanelId::Detail | PanelId::Problems | PanelId::AppSizes => workspace == Workspace::Theme,
        }
    }

    /// Panels move, float and tab freely. Only a component tab closes: any other closed panel would have no way back
    /// besides Reset layout.
    pub fn permissions(&self) -> Permissions {
        Permissions::ALL.closable(matches!(self, PanelId::Component(_)))
    }
}

/// [`PanelId`] as saved: the same variants, with a component tab named by an owned string.
#[derive(Serialize, Deserialize)]
enum PanelRepr {
    Components,
    Stories,
    Properties,
    Inspect,
    Tokens,
    Overview,
    Component(String),
    Preview,
    Detail,
    Problems,
    AppSizes,
}

/// The names of the component tabs read from saved layouts. A tab name is a `&'static str`, so each distinct name is
/// kept once for the process.
fn interned(name: String) -> &'static str {
    static NAMES: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());
    let mut names = NAMES.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(known) = names.get(name.as_str()) {
        return known;
    }
    let leaked: &'static str = Box::leak(name.into_boxed_str());
    names.insert(leaked);
    leaked
}

impl Serialize for PanelId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let repr = match *self {
            PanelId::Components => PanelRepr::Components,
            PanelId::Stories => PanelRepr::Stories,
            PanelId::Properties => PanelRepr::Properties,
            PanelId::Inspect => PanelRepr::Inspect,
            PanelId::Tokens => PanelRepr::Tokens,
            PanelId::Overview => PanelRepr::Overview,
            PanelId::Component(name) => PanelRepr::Component(name.to_owned()),
            PanelId::Preview => PanelRepr::Preview,
            PanelId::Detail => PanelRepr::Detail,
            PanelId::Problems => PanelRepr::Problems,
            PanelId::AppSizes => PanelRepr::AppSizes,
        };
        repr.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PanelId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match PanelRepr::deserialize(deserializer)? {
            PanelRepr::Components => PanelId::Components,
            PanelRepr::Stories => PanelId::Stories,
            PanelRepr::Properties => PanelId::Properties,
            PanelRepr::Inspect => PanelId::Inspect,
            PanelRepr::Tokens => PanelId::Tokens,
            PanelRepr::Overview => PanelId::Overview,
            PanelRepr::Component(name) => PanelId::Component(interned(name)),
            PanelRepr::Preview => PanelId::Preview,
            PanelRepr::Detail => PanelId::Detail,
            PanelRepr::Problems => PanelId::Problems,
            PanelRepr::AppSizes => PanelId::AppSizes,
        })
    }
}

/// The share a split beside a dock records. The dock's width comes from its columns, so the split never reads it.
const BESIDE_DOCK: SplitSize = SplitSize::Ratio(0.5);

/// The component tree column's width; the side column takes the dock column token.
const TREE_WIDTH: f32 = 220.0;

/// The components workspace: the component tree at the start edge, the overview in the centre, the stories above the
/// properties, the inspector and the token browser at the end edge.
pub fn default_components() -> Dock<PanelId> {
    let tree = Column::new(vec![Group::new(vec![PanelId::Components], 0)]).with_width(TREE_WIDTH);
    let side = Column::new(vec![Group::new(vec![PanelId::Stories], 0), Group::new(vec![PanelId::Properties, PanelId::Inspect, PanelId::Tokens], 0)]);
    let centre = Node::split(Axis::X, BESIDE_DOCK, Node::tabs(vec![PanelId::Overview], 0), Node::dock(DockEdge::End, vec![side]));
    let root = Node::split(Axis::X, BESIDE_DOCK, Node::dock(DockEdge::Start, vec![tree]), centre);
    Dock::new(Layout::new(Some(root), Vec::new()))
}

/// The theme workspace: the preview in the centre; the problems, detail and app sizes above the token browser at the
/// end edge. The problems come first in the tree, so they are shown before the tokens in a pass: the token browser
/// reveals the row a problem picks in the same pass.
pub fn default_theme() -> Dock<PanelId> {
    let side = Column::new(vec![Group::new(vec![PanelId::Problems, PanelId::Detail, PanelId::AppSizes], 0), Group::new(vec![PanelId::Tokens], 0)]);
    let root = Node::split(Axis::X, BESIDE_DOCK, Node::tabs(vec![PanelId::Preview], 0), Node::dock(DockEdge::End, vec![side]));
    Dock::new(Layout::new(Some(root), Vec::new()))
}

/// The default layout of `workspace`.
pub fn default_for(workspace: Workspace) -> Dock<PanelId> {
    match workspace {
        Workspace::Components => default_components(),
        Workspace::Theme => default_theme(),
    }
}

/// `saved` made valid for `workspace`: a layout that fails validation is replaced by the default, panels of the other
/// workspace and component tabs `known` rejects are dropped, and missing panels come back where the default has them.
pub fn reconciled(mut saved: Dock<PanelId>, workspace: Workspace, known: &dyn Fn(&str) -> bool) -> Dock<PanelId> {
    let registered = |panel: &PanelId| {
        panel.in_workspace(workspace)
            && match panel {
                PanelId::Component(name) => known(name),
                _ => true,
            }
    };
    saved.reconcile(registered, PanelId::permissions, &default_for(workspace));
    saved.hidden.retain(|(panel, _)| registered(panel));
    saved
}

/// The saved text of `layout`.
pub fn save(layout: &Dock<PanelId>) -> String {
    serde_json::to_string(layout).unwrap_or_default()
}

/// The layout saved as `text`, made valid for `workspace`; the default when the text is empty or unreadable.
pub fn load(text: &str, workspace: Workspace, known: &dyn Fn(&str) -> bool) -> Dock<PanelId> {
    serde_json::from_str::<Dock<PanelId>>(text).ok().map_or_else(|| default_for(workspace), |saved| reconciled(saved, workspace, known))
}

/// The component tabs `layout` holds, in layout order.
pub fn open_tabs(layout: &Dock<PanelId>) -> Vec<&'static str> {
    layout
        .layout
        .panels()
        .into_iter()
        .filter_map(|panel| match panel {
            PanelId::Component(name) => Some(*name),
            _ => None,
        })
        .collect()
}
