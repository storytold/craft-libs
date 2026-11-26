# Docking

Docking has three layers:

| Layer | Holds |
| --- | --- |
| `craft_behavior::dock` (craft-behavior) | The model: `Workspace`, `Layout`, `Node`, `Action`, `Permissions`, `WorkspaceSet`. No egui; this is the saved data. |
| `craft_ui::docking` (craft-ui-panels) | The parts: `SplitArea`, `DockGroup`, `DockColumn`, `IconicColumn`, `Flyout`, `FloatingGroup`, `DockRoot`, `Splitter`; drop resolution; the dock commands. |
| `craft_ui::patterns::dock` (craft-ui-patterns) | The patterns an app shows: `DockArea` over a workspace, `PaneArea` for task panes, `WorkspaceSwitcher`. |

An app with dockable panels shows a `DockArea`. The parts are for patterns that need one piece, such as a two-body split.

## Model

A `Workspace` is a saved arrangement: its `layout()` and the `hidden()` panels with the place each one goes back to. Its fields are private; it changes only through its methods. A `Layout` is a docked tree (`root: Option<Node<P>>`) plus floating windows (`floating: Vec<Floating<P>>`). `P` is the app's panel key. Every panel occurs once.

`Node` has four forms:

- `Split`: two children along an `Axis`, sized by a `SplitSize`.
- `Tabs`: a group of panels with one active tab.
- `Stack`: an accordion; each `StackEntry` keeps its open state and height when other panels move.
- `Dock`: the dock on one window edge (`DockEdge`), with its `Column`s from the edge inwards. At most one per side.

A floating window is one `Column` that floats: its groups stacked top to bottom, nothing beside them. Every column, docked or floating, collapses to icons (`Column::iconic`, `Action::SetIconic`) on its own.

The tree is in reading order: a split's first child is its start (or top) child. Under right-to-left the renderer maps rects with `Flow::to_physical`.

Edits are `Action`s: `Move`, `Float`, `Close`, `Activate`, `Open`, `OpenAt`, `MoveFloating`, `ResizeSplit`, `SetIconic` and the rest. `Workspace::apply` and `Workspace::apply_all` apply them against each panel's `Permissions` and the lock, and refuse what the rules forbid with a `DockError`. `Permissions::ALL` allows everything. `Permissions::PROTECTED` is a document canvas: it stays docked and open, and takes neither tabs nor side splits.

To load a saved workspace:

1. Deserialize it (`Workspace` implements `serde`).
2. Check it with `Workspace::validate`. Keep a file that fails, or that comes from a newer `Workspace::VERSION`, on disk; do not overwrite it.
3. Call `Workspace::reconcile` with the panels the app registers now and a default workspace.

`WorkspaceSet` holds the app's named workspaces (built-in and saved), the current one and Lock Workspace.

## DockArea

```rust
let mut panels = Panels { document: &mut state.document };
let mut area = DockArea::new("Dock", Bind::bound(&mut state.workspace), &state.specs, &mut panels, DockConfig::default());
if let Some(panel) = state.maximized {
    area = area.maximized(panel);
}
for event in area.show(ui).events {
    if let DockEvent::ToggleMaximize { panel, .. } = event {
        state.maximized = if state.maximized == Some(panel) { None } else { Some(panel) };
    }
}
```

The host holds the workspace as an `Arc<Workspace<P>>` (`state.workspace`): a pass shares it instead of cloning it, and an edit writes a new one (`Arc::make_mut`). Keep that one `Arc` across frames and clone the `Arc`, not the workspace: a new workspace value is a new revision, so a host that wraps a fresh copy each frame makes the area read every spec each frame.

The host keeps its panels' specs in a plain-data model that implements `PanelSpecs<P>`, held in a `Tracked` (`state.specs`):

| Method | Does |
| --- | --- |
| `spec` | The panel's `PanelSpec`: label, icon, `SizeLimits`, `DockHeights`, float size, `allow_native`, `window_kind`, `canvas`, `grid`, `host_body`, `count`, `caption`, `swatch` (a data colour before the tab's name; `Some(None)` is the theme's empty swatch) and `documents`. |
| `permissions` | The panel's `Permissions` (default `ALL`). |

A panel's `documents` (`PanelDocument`: id, label, swatch, `closable`, `shown`) are tabs of its group's strip after the panel tabs, whichever panel the group shows: one fit and one overflow menu for both. The tab of the shown panel's `shown` document is the selected tab. A pick reports `DockEvent::DocumentPicked` and activates the panel; a close button or middle click reports `DockEvent::DocumentClosed`. The host opens, shows and closes documents by editing the spec. Document tabs do not drag and take no tab drop.

The area reads them only when the model's revision, the workspace's revision or the lock changes (`runtime::derived`), and keeps what it read otherwise, so an idle pass reads no spec. A host changes a label or a limit by editing the model. A spec reads no style: a tool rail's grid is `DockGrid::rail(cells)`, which the dock sizes from its own tokens.

The host implements `DockPanels<P>` for the views:

| Method | Does |
| --- | --- |
| `body` | Shows the panel's body in the group's body, which the group insets by the panel padding (`Pad::Regular`). The group is the body's panel, so the body adds no `Panel` or padding of its own. A document panel's body (`WindowKind::Document`) is shown edge to edge. |
| `group_view` | Takes a whole group in place of its tab strip and body, in a `Ui` held to the group's rect; a canvas group draws its document here. Default `None`. |
| `group_header` | The host's controls in a group header, before the tab strip. |
| `panel_rows` | The host's rows of a panel's menu, before the dock's own. |

A body lays out with the group's spacing, which framework components expect. A panel whose body is the app's own egui code sets `PanelSpec::host_body()`: the dock then hands its `body`, `footer`, `group_header`, `rail_footer` and, while it is the panel its group shows, its group's `group_view` a `Ui` with the style of the `Ui` the area was shown in, so that code lays out as it does outside the dock. Its body also keeps the app's background and room: the dock fills the body with that style's panel fill (`Visuals::panel_fill`) and gives it no body padding, and a press on the body is the app's, so it does not focus the group's tabs. That code runs once per pass, in the pass's real run: where the framework measures a group before it shows it, the code's parts take the size they took when they last ran, and egui sees each of the app's widgets once (`HostStyle::show`). The dock's own parts keep the framework style. An app hosting its old egui panels moves onto the framework panel by panel, clearing `host_body` on each panel it ports.

The binding decides who applies the edits. With `Bind::bound` the area writes every action to the workspace itself. With `Bind::given` it writes nothing, and the host applies `DockAreaOutput::actions` (or `edit`, the same changes as edit events) to its own copy: the actions hold every change of the pass, divider sizes and window moves included. `DockArea::read_only` shows a workspace locked, keyed by its revision.

A panel body, or code a body calls, changes the workspace during the pass with `docking::request_actions(ctx, actions)`, never by writing the workspace the area shows. The running area takes them unapplied and returns them in `DockAreaOutput::requested`, in order, for the host to apply after `actions` in the same frame. With no area over that panel type running, `request_actions` hands the actions back for the caller to apply.

Props: `hide` (`HidePanels::None`, `All` or `Except`), `maximized`, `auto_collapse`, `locked`, `raise` (the floating window holding the panel comes to the front in this pass: an OS window gets the focus, an in-app window goes to the top of the floating windows; a host passes it in the pass after it opens or activates a floating panel), and `flyout`, which binds the open flyout to the host's value (the area keeps it otherwise). A hidden panel keeps its place for restore. Inside an `AppShell`, the shell's hiding wins over `hide`: while it hides the panels and keeps the tools (Shift+Tab, `ShellCx::panels_hidden`), only the panels an `Except` names stay, docked or floating (a toolbar); while it hides the dock (Tab), no panel shows. Under Tab and Shift+Tab, an edge the pointer reveals (`ShellCx::edge_revealed`) shows its `Start` or `End` dock, with the host's `hide` applied to it. Protected panels always show.

A panel in a column collapsed to icons shows in its flyout (`Layout::opens_in_flyout`). The host opens and closes it by writing the `flyout` binding, not the layout: `WindowMenuRows` checks such a panel's row while its flyout is open (`WindowMenuRows::flyout`) and reports a pick in `WindowMenuRowsOutput::flyout`.

`DockAreaOutput` holds the `actions` taken, the bodies' `requested` actions, the `events` for the host, the `focused` and `picked` panels, the `pressed` panel (the shown panel of the group whose body the primary press went down in, whatever took it, so a host whose bodies are its own egui code knows the panel clicked into), the gesture `sources` (a panel tab, a document tab, a group bar, a tab strip's overflow button, window grips, dividers), `edit`, `float_space`, `columns`, the OS `windows` the floating windows showed in (by a panel of each), `window_files` (one `WindowFiles` per drop on a floating OS window: the shown panel of the group under the drop, the first group's for a drop on the title row, and the files), and `error`: an `InvalidGeometry` when the workspace fails `validate`, in which case the area draws nothing.

The host handles these `DockEvent`s:

| Event | Host does |
| --- | --- |
| `ToggleMaximize` | Sets or clears the panel it passes as `maximized`. |
| `ToggleAutoCollapse` | Flips its preference and passes it back. |
| `PanelCommand` | Carries out Move Panel To and Move Group To: asks for a destination and calls `move_actions`. |
| `FlyoutChanged` | Nothing when the area or a write-through binding holds the flyout; applies the request when the binding is given. |
| `DocumentPicked` | Shows the document in the panel (the area has activated the panel). |
| `DocumentClosed` | Closes the document and removes it from the panel's spec. |
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
| `panel_menu_always` | off | Every group shows its panel menu button, with the dock's own rows when the shown panel has none. Off: only a panel with rows of its own (`DockPanels::panel_rows`) gets the button. |
| `panel_menu_icon` | `more` | The panel menu button's icon; `menu` gives the three-line mark. |
| `native_windows` | on | Floating windows open as OS windows where the window host opens them and reports their positions (not on Wayland) and `PanelSpec::allow_native` allows. |

The theme's docking sizes (`DockSizes`) set the drop bands and zones. `DockWords` holds the words.

## Rules

- A tab drag carries a `PanelDrag`; a group or column drag carries a `GroupDrag`. Groups and columns register `DockSpot`s in `DockTargets`, and `resolve_drop` maps the spot under the pointer to an `Action`.
- A column collapsed to icons takes drops as an expanded column does: its edges (a new column beside it, docked only), the seams between its icon groups, the top of the first group and the bottom of the last (a new group above or below), and each icon group's cells (join its tabs). The column's state wins: a panel dropped into a collapsed column shows as an icon, one dropped into an expanded column shows expanded.
- Nothing goes beside a floating window: its column registers no edge bands, and its groups (`DockGroup::in_window`) take no drop on their start and end sides.
- A collapsed column's flyouts open on the side `IconicColumn::opens` names: away from a dock's window edge, and towards the room of the float area for a floating window. A floating window keeps to its edge of the float area: `Floating::edge`, else the one nearer its rect. A move keeps the edge it began with. A collapsed window's move ends at the edge nearer its strip where it was dropped, with the rect that shows the strip there (`WindowColumn::dropped`), and the area records that edge (`Action::MoveFloating`'s `edge`); a unit dragged out of a collapsed column (`start_icon_drag`) floats collapsed the same way, its strip under the pointer (`Target::Float`'s `edge`). Collapsing a floating column and moving an expanded window clear the edge. A column that neither collapses nor has steps shows no double arrow and offers no collapse in its menu (`ColumnArrow::Fixed`).
- A panel that lays its content in a grid (a toolbar) declares it with `PanelSpec::grid` (`DockGrid`: the width at one content column, the width each further column adds, and the most columns; `DockGrid::rail` for a tool rail, sized by the dock). Its column, given steps (`Column::steps`), is a whole number of content columns wide. Docked, it takes the dock's height like any column, its content at the top (the references' docked toolbar keeps its surface down to the dock's foot), and content taller than the dock scrolls inside; floating, it is as tall as the content measures at that width, under the title row, at the window's place. A drag of the column's edge, or of a floating window's side, sets the most steps that fit, and a drag of a floating window's left side keeps its right edge, one of its top its bottom edge; a drag of a floating window's top or bottom sets the fewest whose content fits. When the column's panels do not collapse, its double arrow switches between one and two steps (`ColumnArrow::Steps`), named for the count it switches to and pointing the way the column grows or shrinks. A dock side's total width (`DockSizes::resize_side`) leaves a stepped column's width alone.
- A floating window whose every group is minimized shows its title row and their tab strips, at its rect's place and width (`WindowColumn::minimized`). Its sides resize its width, never its height, and its rect, in the app window and in an OS window's recorded placement, keeps the expanded height, which the window takes back when a group is restored. A window with some groups open keeps its size, and its open groups share the room the strips leave.
- Ctrl or Cmd held at the release floats the drop. A release over the dragged unit's own group leaves it where it is.
- A drag ends without a change when another arrangement replaces the workspace it started on: it records the workspace's generation, which `Workspace::replace` and the `WorkspaceSet` switch and reset change. The dock's own edits keep the generation, so they keep the drag. Every write moves the workspace's revision (`Workspace::revision`), which keys what the area derives.
- A floating window's id comes from a counter its layout owns (`Layout::fresh_window_id`). It starts above every id the layout is built or loaded with and only grows while the layout lives; `Workspace::replace` gives the incoming windows ids the replaced layout never gave out, and a dock area keeps the windows its actions add above every id it has shown, so a workspace an undo brings back draws no id twice (`Workspace::raise_window_floor`). An OS window never passes to another window.
- A dock column's last open group fills the column: it takes the height the other groups leave and asks for its `DockHeights::fill`. The other groups keep the height the user gave them, or take their `DockHeights::default`, which shrinks towards `compact` before the filling group gets less than its fill. Groups that ask for no heights share the free height with the filling group.
- Split handles are Tab stops, and the arrow keys move them. A pointer press or drag on a handle leaves key focus where it was, so the arrows after a drag still reach the app; only a handle reached by Tab takes them. A divider drag is one gesture in `DockAreaOutput::edit`.
- A pointer press on a panel tab (a click, a drag's press, the tab menu's secondary press) leaves key focus where it was, so the document's keys keep working; Tab still reaches the tab row.
- Floating windows stack with the app's own egui windows in egui's area order: the one shown, pressed or raised last (`FloatingGroup::raise`, which the host's `DockArea::focus` request sets on the window holding the panel) is on top, so a dialog the app opens over a floating panel shows above it.
- A panel command (`DockCommands`) acts on the shown panel of the group that takes it. A menu row, a key and the command palette run the same code: `actions` turns the command into `Action`s.

## Commands and workspaces

| Set | Commands | Run with |
| --- | --- | --- |
| `DockCommands` | Float, move, close, step, minimize, collapse to icons, maximize | The `DockArea` itself. A host that shows its own dock calls `actions` and `Workspace::apply_all`. |
| `WorkspaceCommands` | Reset, New, Delete, Lock | `run_workspace`, which returns a `WorkspaceAsk` when the host must ask for a name or a workspace. `WorkspaceSet::select` switches. |
| `ArrangeCommands` | Arranging document windows | `arrange` over `DocumentWindows`. |

`WorkspaceSwitcher` is a select over the set's names with the workspace commands in its list.

## Floating and native windows

A floating window is a `Floating` in the layout: its one `column`, its `rect` in logical points in the app's viewport, and `native` and `monitor` while it is an OS window. `FloatingGroup` shows it, with the column inside drawn as a dock column is (`ColumnBar::Window`): the window's title row is the column's bar, whose double arrow, like a double click on the row, collapses the column to icons. The title row is one text row high (`HeaderStrength::Bar`). A window in the main window keeps a band above it for its top grip (`contract::grip_band`), so the grip never covers the row; an OS window keeps none, and on macOS AppKit resizes it from its own edges, with no app grips there (`DockSizes::window_chrome`). A collapsed window shows its icons against the edge of the float area it keeps to, under a title row of the band and the double arrow alone (a context request on the band opens the window's menu), and keeps its expanded size for when it expands. Its sides resize its width, and only its width, within the bounds of a docked collapsed column (`DockSizes::iconic_bounds`), on every platform and in the main window alike; names show beside the icons from `IconicSizes::labels`, and the width is the column's `iconic_width` (`Action::ResizeIconic`, `FloatingGroupOutput::collapsed_width`), kept across collapse and expand. With `DockConfig::native_windows` (on by default), every floating window opens in its own OS window (`docking::native`) where the window host opens OS windows and reports where they are (`WindowHost::positions_reported`; not on Wayland); a window never shown native yet opens over its place in the main window. Elsewhere, and for a panel with `allow_native` off, the window stays inside the main window. The OS close of a native window is its Close action: the window closes when every panel in it may close. Dragging a native window by its title row moves the whole window and docks it where it is released over a group of the main window, as a floating window inside the main window does: the app runs that move (`Overlay::moved_by_app`, which the group's mount carries, so the window is opened knowing it). On macOS the row's press hands the move to the OS, the app follows the system pointer and button (`Caps::OS_MOVES_TRACKED`), and a press that ends within the click distance is a click, which docks nothing; elsewhere the app moves the window, and Escape puts it back. The app sends an OS window its place only when the app changes it (an undo, a reloaded layout); the OS's own moves and resizes are recorded and never sent back. `window_kind` says whether an OS window is a document or a tool window. A tool window's title row is its only title bar, whatever frame the main window has (`craft_window::policy::tool_frame`).

A panel or group floated out of the dock (a drag, the Float command, a tab torn out of the window) opens at the width it had docked and at the height its group measures at in a floating window's room (`Proposal::Size`), at most `size.float.height`: content that fills its body opens at that height, other content at its own. A column opens at its docked size, at most that height. The area measures once, where the float starts: for a drag in the pass it starts, which adds the size to the drag's payload (`runtime::drag::amend`). A panel's own `PanelSpec` float size wins, and a window that already floats keeps its size.

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
