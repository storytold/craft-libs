# Docking

Docking has three layers:

| Layer | Holds |
| --- | --- |
| `craft_behavior::dock` | The model: `Workspace`, `Layout`, `Node`, `Action`, `Permissions`, `WorkspaceSet`. No egui; this is the saved data. |
| `craft_ui::docking` | The parts: `SplitArea`, `DockGroup`, `DockColumn`, `IconicColumn`, `Flyout`, `FloatingGroup`, `DockRoot`, `Splitter`; drop resolution; the dock commands. |
| `craft_ui::patterns::dock` | The patterns an app shows: `DockArea` over a workspace, `PaneArea` for task panes, `WorkspaceSwitcher`. |

An app with dockable panels shows a `DockArea`. The parts are for patterns that need one piece, such as a two-body split.

## Model

A `Workspace` is a saved arrangement: its `layout` and the `hidden` panels with the place each one goes back to. A `Layout` is a docked tree (`root: Option<Node<P>>`) plus floating windows (`floating: Vec<Floating<P>>`). `P` is the app's panel key. Every panel occurs once.

`Node` has four forms:

- `Split`: two children along an `Axis`, sized by a `SplitSize`.
- `Tabs`: a group of panels with one active tab.
- `Stack`: an accordion; each `StackEntry` keeps its open state and height when other panels move.
- `Dock`: the dock on one window edge (`DockEdge`), with its `Column`s from the edge inwards. At most one per side.

The tree is in reading order: a split's first child is its start (or top) child. Under right-to-left the renderer maps rects with `Flow::to_physical`.

Edits are `Action`s: `Move`, `Float`, `Close`, `Activate`, `Open`, `OpenAt`, `MoveFloating`, `ResizeSplit`, `ResizeWindowSplit` and the rest. `Workspace::apply` and `Workspace::apply_all` apply them against each panel's `Permissions` and the lock, and refuse what the rules forbid with a `DockError`. `Permissions::ALL` allows everything. `Permissions::PROTECTED` is a document canvas: it stays docked and open, and takes neither tabs nor side splits.

To load a saved workspace:

1. Deserialize it (`Workspace` implements `serde`).
2. Check it with `Workspace::validate`. Keep a file that fails, or that comes from a newer `Workspace::VERSION`, on disk; do not overwrite it.
3. Call `Workspace::reconcile` with the panels the app registers now and a default workspace.

`WorkspaceSet` holds the app's named workspaces (built-in and saved), the current one and Lock Workspace.

## DockArea

```rust
let mut panels = Panels { document: &mut state.document };
let mut area = DockArea::new("Dock", Bind::bound(&mut state.workspace), &mut panels, DockConfig::default());
if let Some(panel) = state.maximized {
    area = area.maximized(panel);
}
for event in area.show(ui).events {
    if let DockEvent::ToggleMaximize { panel, .. } = event {
        state.maximized = if state.maximized == Some(panel) { None } else { Some(panel) };
    }
}
```

The host implements `DockPanels<P>`:

| Method | Does |
| --- | --- |
| `spec` | The panel's `PanelSpec`, read every pass: label, icon, `SizeLimits`, `DockHeights`, float size, `allow_native`, `window_kind`, `canvas`. |
| `permissions` | The panel's `Permissions` (default `ALL`). |
| `body` | Shows the panel's body. |
| `group_view` | Takes a whole group in place of its tab strip and body; a canvas group draws its document here. Default `None`. |
| `group_header` | The host's controls in a group header, before the tab strip. |
| `panel_rows` | The host's rows of a panel's menu, before the dock's own. |

The binding decides who applies the edits. With `Bind::bound` the area writes every action to the workspace itself. With `Bind::given` it writes nothing, and the host applies `DockAreaOutput::actions` (or `edit`, the same changes as edit events) to its own copy. `DockArea::read_only` shows a layout locked.

Props: `hide` (`HidePanels::None`, `All` or `Except`), `maximized`, `auto_collapse`, `locked`, and `flyout`, which binds the open flyout to the host's value (the area keeps it otherwise). A hidden panel keeps its place for restore.

`DockAreaOutput` holds the `actions` taken, the `events` for the host, the `focused` and `picked` panels, `edit`, `float_space`, and `error`: an `InvalidGeometry` when the workspace fails `validate`, in which case the area draws nothing.

The host handles these `DockEvent`s:

| Event | Host does |
| --- | --- |
| `ToggleMaximize` | Sets or clears the panel it passes as `maximized`. |
| `ToggleAutoCollapse` | Flips its preference and passes it back. |
| `PanelCommand` | Carries out Move Panel To and Move Group To: asks for a destination and calls `move_actions`. |
| `FlyoutChanged` | Nothing when the area or a write-through binding holds the flyout; applies the request when the binding is given. |
| `FloatPlaced` | Reports for a host that anchors its own surfaces to the dock; `DockAreaOutput::columns` gives every column's rect each pass. |

## DockConfig

| Field | Default | Effect |
| --- | --- | --- |
| `side_edge_zones` | off | Picture-frame profile: deep side zones on groups, and the outer start and end edges add full-height columns. |
| `drop_edges` | `DockSides::ALL` | Which outer edges (and sides of a group that takes no tabs) take a drop. |
| `new_dock_zones` | on | The start and end edges of the docked area take a drop as a new dock. |
| `close_others_item`, `maximize_item` | off | Group menu items Close Other Panels in Group and Maximize Frame. |
| `float_on_empty_drop` | on | A release over no drop target floats what the drag carries. Off: it cancels the drag. |
| `title_bar_docks` | on | Dragging a floating window's title bar onto a target docks the whole window. |
| `close_menu_items`, `close_button` | on | Built-in Close items, and the close action on floating windows. |
| `tab_close_buttons` | off | Close buttons on tabs (middle click closes too), for tabs that act as documents. |
| `native_windows` | off | Floating windows open as OS windows where the platform and `PanelSpec::allow_native` allow. |

The theme's docking sizes (`DockSizes`) set the drop bands and zones. `DockWords` holds the words.

## Rules

- A tab drag carries a `PanelDrag`; a group or column drag carries a `GroupDrag`. Groups and columns register `DockSpot`s in `DockTargets`, and `resolve_drop` maps the spot under the pointer to an `Action`.
- Ctrl or Cmd held at the release floats the drop. A release over the dragged unit's own group leaves it where it is.
- A drag ends without a change when another arrangement replaces the workspace it started on: it records the workspace's revision, which `Workspace::replace` and the `WorkspaceSet` switch and reset change. The dock's own edits keep the revision, so they keep the drag.
- Split handles are Tab stops, and the arrow keys move them. A divider drag is one gesture in `DockAreaOutput::edit`.
- A panel command (`DockCommands`) acts on the shown panel of the group that takes it. A menu row, a key and the command palette run the same code: `actions` turns the command into `Action`s.

## Commands and workspaces

| Set | Commands | Run with |
| --- | --- | --- |
| `DockCommands` | Float, move, close, step, minimize, collapse to icons, maximize | The `DockArea` itself. A host that shows its own dock calls `actions` and `Workspace::apply_all`. |
| `WorkspaceCommands` | Reset, New, Delete, Lock | `run_workspace`, which returns a `WorkspaceAsk` when the host must ask for a name or a workspace. `WorkspaceSet::select` switches. |
| `ArrangeCommands` | Arranging document windows | `arrange` over `DocumentWindows`. |

`WorkspaceSwitcher` is a select over the set's names with the workspace commands in its list.

## Floating and native windows

A floating window is a `Floating` in the layout: its `rect` in logical points in the app's viewport, and `native` and `monitor` while it is an OS window. `FloatingGroup` shows it. With `DockConfig::native_windows`, a floating window opens in its own OS window (`docking::native`). A panel with `allow_native` off keeps its window inside the main window. `window_kind` says whether an OS window is a document or a tool window.

## SplitArea on its own

`SplitArea::new(label, layout, rules)` shows the docked tree of a `Layout` with handles between groups; `FrameRules` give each panel's limits. `SplitArea::pair(label, axis, bodies, share, rules)` is a two-body split with one divider. The host keeps the share and reads its edits from `SplitAreaOutput::share`; the timeline's track split works this way.

## PaneArea

`PaneArea` shows task panes around a document, without a dock `Layout`. The app keeps which panes are open, where they are and how big:

- `PaneArea::new(label, panes, content).show(ui, centre)`: `panes` are the open panes in edge order, and `centre` fills what the docked panes leave.
- `PaneContent<P>` gives each pane's `PaneSpec`, its `PaneState` (`None` shows the default), its body and its menu rows.
- The output lists `PaneRequest`s (`Close`, `Place`, `FloatRect`, `Minimize`, `Resize`, `Focus`, `Before`) in the order they happened. The app applies them in that order.
- `SidePolicy::Columns` shows every pane on an edge side by side. `SidePolicy::OneSlot` shows one closable pane per edge.
- `PaneCommands` (move to an edge, float, close) act on the pane that holds focus.

## What stays in the app

- The panel keys, the panel registry and the panel bodies.
- Workspace files: where they live, their names and their size caps.
- The answers to `DockEvent`s and `WorkspaceAsk`s.
