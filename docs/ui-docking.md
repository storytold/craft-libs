# Panel docking

`craft_ui::docking` lets the user arrange an app's panels: tab groups, docks with columns,
splits, floating windows and iconic columns. craft-ui owns the layout model, the renderer and the
gestures. The app owns panel contents, commands, document state and where layouts are saved.

Office apps that draw their own document with a few task panes beside it use
[`craft_ui::panes`](ui-panes.md) instead.

## Arrangements

One model serves every app. What differs between apps is the default layout and a few
`DockStyle` settings. An app builds its style with `DockStyle::from_ui`, `from_visuals` or
`default` and then sets fields; new fields can be added, so the struct has no literal form
outside the crate.

| Arrangement | Layout | Style settings | Apps |
|---|---|---|---|
| Docks beside a canvas (the defaults) | `Node::Dock`s on the window's start and end edges. A dock has columns; a column has stacked groups and can collapse to icons. Frames (tab groups and splits) fill the space between the docks. | `new_dock_zones` on | Creative apps with a canvas and panel docks |
| A split tree of tab groups | `Split`s and `Tabs` only. | `side_edge_zones`, `maximize_item`, `close_others_item`, `command_press_floats`, `float_hint`, `wheel_selects_tabs`, `middle_click_closes_tabs` on, `new_dock_zones`, `float_on_empty_drop` and `title_bar_docks` off; menu labels such as "Undock Panel" set on `float_label`, `float_group_label` and `close_group_label` | Apps whose whole window is frames |

Sides are logical: `Side::Start` is the left edge, or the right edge when
`DockStyle::right_to_left` is set. egui has no app-wide direction, so the app sets it from its
locale. Right to left, tab strips also mirror: the first tab is rightmost, the group header and
drag bar are at the left, and drop insertion slots follow. Accordion headers and floating title
bars (`chrome::TitleBar`, through `ChromeStyle::right_to_left`) mirror too. Frame splits keep
their sides.

## Model

`Layout<P>` holds an optional root `Node<P>` and a list of `Floating` windows.

| Node | Holds |
|---|---|
| `Tabs` | A tab group: panels and the active index. |
| `Split` | Two children, horizontal or vertical, and a size: a ratio, a fixed first size or a fixed second size. A moved divider keeps the mode: a fixed side stores its new size in points, a ratio its new ratio. |
| `Stack` | Accordion entries, each open or closed, with an optional height. |
| `Dock` | A side and its columns. A `Column` has `Group`s, a width, and `iconic`. |

A `Floating` window holds a tree like the docked layout's (`root`: splits, tab groups and
accordions, no docks), a rect, an optional native-window rect with the monitor it is on, and
`minimized`. Its `id` names the window across panel moves (see [Native windows](#native-windows)).
`Floating::single` makes a window with one group, `Floating::new` one with a tree, and
`Floating::stack` one with groups stacked top to bottom (a vertical split sized by the groups'
heights, as an app's own stacked layouts migrate); `panels()` and `contains()` read it. Each of
these calls makes a new window with a fresh `id`. Build a layout once and keep it, or give its
windows ids with `with_id`: a layout built anew each frame, as an unsaved default layout might
be, otherwise opens new OS windows each frame and loses their drags and places. Debug builds log
a warning when native windows are replaced by new ones with the same panels three frames running. Saved layouts keep loading: a window stored as one group (v1) or as a
stack of groups loads as that group or as a vertical split of the stacked groups, sized by their
heights.

`P` is the app's panel id: stable, not localized, `Clone + Eq + Hash`. The renderer also needs
`Debug + Send + Sync + 'static`. A panel occurs at most once in a layout.

Change a layout only through `Action`s:

```rust
use craft_ui::docking::{Action, Floating, Layout, Node, Placement};

let mut layout = Layout {
    root: Some(Node::Tabs { panels: vec!["layers", "properties"], active: 0 }),
    floating: vec![Floating::single(vec!["brushes"], 0, [40.0, 80.0, 240.0, 300.0]).with_id(1)],
};
layout.apply(Action::Move {
    panel: "brushes",
    anchor: "layers",
    placement: Placement::Tab { before: Some("properties") },
})?;
# Ok::<(), craft_ui::docking::DockError>(())
```

- `Move`, `MovePanel`, `Float` and `Close` act on one panel. `MoveGroup`, `MoveColumn` and
  `MoveWindow` move a group, a dock column or a floating window, named by any panel in it.
- `Open` reveals an absent panel as the last tab of its `anchor`'s group. Without an anchor it
  becomes the root of an empty layout, else joins the first dock group that accepts tabs and is
  shown, not minimized or in an iconic column (the end-side dock's first), else gets a new frame a quarter wide at the right edge of the docked
  area. `OpenAt` takes any placement.
- A `Target` is `At { anchor, placement }`, `Float { rect }` or `NewDock { side }`.
- A `Placement` is a tab position, a split zone, a new group above or below (with an optional
  preferred `height`), a new column, or a new frame on an outer edge.
- Containers take different placements: a dock column takes groups and columns, not splits.
  `Placement::for_zone(layout, anchor, zone, right_to_left)` maps a left, right, top, bottom or
  center zone onto what the anchor's container takes: a split in the frame tree, a new column on
  that side or a new group above or below in a dock. A floating window takes splits as the
  docked layout does; `Placement::Group` on a floating anchor splits its group above or below.
- A side split zone puts what moves in a new half-size split of the anchor's frame, and no
  other frame changes size. The new split has its own divider. `Placement::SplitSized` sizes
  that split instead (`size`) and resizes the enclosing split whose divider runs along the
  anchor frame's side (`outer`), so the frames beyond that divider give the new frame its room.
- Other actions select, minimize, collapse to icons, resize splits (`ResizeSplit` in the docked
  layout, `ResizeWindowSplit` in the floating window holding a panel), columns and groups, and
  move a floating window to its own OS window (`SetNative`, and `SetMonitor` for the monitor it
  is on, feature `native-windows`).
- A move between windows (docked to floating, floating to floating) is one action, checked as a
  whole like any other.

Every action checks the whole result before it commits. A bad target, a duplicate id, a stale
split path or invalid geometry returns a `DockError` and leaves the layout unchanged. The source
panel is never lost. `apply`, `apply_with_permissions` and `apply_all` return the panels the
actions closed, so the app can release their state; `closed_by` lists them before applying.

`Permissions` limit what may happen to a panel: `movable`, `floatable`, `closable`,
`accepts_tabs` and `accepts_splits`. `apply_with_permissions` enforces them for every action,
whoever issues it. `Permissions::PROTECTED` keeps a canvas docked and open and takes neither tabs
nor side splits; a group dropped on it floats there. An app that opens panels beside its canvas
uses `Permissions { accepts_splits: true, ..Permissions::PROTECTED }`.

## Workspace

`Workspace<P>` is what an app saves: the layout, the hidden panels with where each one goes back,
an optional saved arrangement for Reset, and `locked`.

| Method | Does |
|---|---|
| `apply`, `apply_all` | Apply actions all or none, with the app's permissions. Closed panels are recorded in `hidden`. While `locked`, moving, floating and closing are refused; `SetNative`, which records the OS windows, still applies. |
| `set_visible(panel, visible, fallback, permissions)` | Hide a panel that may close, or show it at its recorded place, else as `Open` beside `fallback`, or where `Open` without an anchor puts it. A group or window closed one panel at a time comes back as one group as its panels are shown again. |
| `reconcile(registered, permissions, default)` | Fit a loaded workspace to the panels the app has now. Unknown panels move to `hidden`, so they come back when registered again. A registered panel that cannot close, such as the canvas, is always shown. |
| `reset(default)` | Return to the saved arrangement, or to `default` for a built-in workspace. |
| `validate` | Check a loaded workspace before use. |

With the `serde` feature, `Workspace`, `Layout` and `Action` serialize. Files without a version
are v1 and load unchanged. craft-ui does not choose where files go; the app does.

## Rendering

`DockArea::new(id).show_content(ui, &layout, &style, &mut content)` draws a layout snapshot and
returns a `DockOutput`. It does not change the layout. The app applies `output.actions` through
its own command path, so pointer, keyboard and control commands share one path.

`DockContent<P>` is the app's adapter. It draws each panel body and gives its label, icon,
permissions, size limits, dock column heights, preferred floating size, panel menu and group header. The group header
sits at the end of the tab strip; `group_header_width` gets the strip width, so a header that
draws its own tabs can size them to the room left. Extra tabs (`group_extra_tabs_width` and
`group_extra_tabs`) sit right after the group's panel tabs, before the drag bar, and fit with
them: both shrink widest first to `min_tab_width`, the extra tabs as one tab, and in a row
narrower still the panel tabs overflow while the extra tabs keep that width. The host fits its
own tabs into the rect it gets. An icon is an image (`icon`), or drawn in code by `paint_icon`, which
gets a painter, the icon rect and the tint color and returns true when it painted. Panel bodies keep a stable egui id when they move, so widget memory
survives.

`DockOutput` also has:

- `focused`: the panel the user focused this frame: a clicked tab, a drop, or a press of any
  pointer button in a panel body (protected panels included) or in the group header,
  which focuses the group's shown panel. Commands, `raise_panel` and
  workspace resets never set it; a host that tracks the last focused panel clears it when a reset
  or command hides, closes or moves that panel.
- `events`: requests outside the layout, such as `DockEvent::ToggleMaximize` (`maximize_item`) and
  `DockEvent::ToggleAutoCollapse` and `DockEvent::FlyoutChanged { panel }` (docks): the
  flyout opened, switched or closed (`None`). It comes once per change, whatever made it: an
  icon, Escape, a press outside, a raised panel, `open_flyout` or `close_flyout`.
- `error`: a rejected gesture.

Builder options on `DockArea`:

| Option | Use |
|---|---|
| `maximized(panel)` | Maximize Frame: that frame fills the area. |
| `hide_panels(HidePanels)` | Hide panels for Tab and Shift+Tab. Protected panels stay. |
| `auto_collapse_iconic(on)` | Close a flyout on Escape or a click outside (default off: a flyout stays open until its icon or another icon is clicked). |
| `raise_panel(panel)` | Raise a panel's floating window, or open its flyout, this frame. It moves no keyboard focus and does not set `DockOutput::focused`. |
| `locked(on)` | Draw `Workspace::locked`: every panel gets `Permissions::locked`, so nothing starts a move, float or close the workspace would refuse. |
| `blocked_rects(rects)` | App overlays that docking gestures must ignore. |
| `float_bounds(rect)` | Where floating windows, flyouts and drag ghosts may go (default: the whole viewport). Pass the viewport below a title bar drawn under a transparent OS title bar, so no floating window sits where a press moves the app window. |

`DockStyle::close_menu_items` and `DockStyle::close_button` hide the built-in Close menu items
and the floating window ×. They hide controls only: a panel that stays `closable` still closes
by the app's own Close items and commands.

`DockArea::chrome_has_focus` tells whether keyboard focus is on docking chrome (any control a
`Hit` names) rather than on a panel body. Focus follows only the user's input: a tab keeps keyboard focus until a command or a
workspace reset hides, closes or moves its panel, and then gives it up. A divider or a floating
window's resize grip holds focus only while the pointer drags it, so the app's shortcuts and
canvas arrow keys work right after; one reached with Tab or F6 keeps its focus, and arrow keys
resize it. Hovering focuses nothing. `begin_drag` hands a drag that started in the app's own chrome to the area.
`docking::chrome` has the floating frame, the title bar and `drop_preview` (the drag ghost
and region preview docking and `craft_ui::panes` both draw) for apps that draw their own windows.

## Tab strips

Tabs start at their natural width, wide enough for the panel's name. When the strip is too
narrow, the widest tabs shrink first, to a common width; tabs whose names fit that width keep
their natural width. A tab narrower than its name cuts the name with "…" and shows it in full as a
tooltip. Shrunk tabs keep their hit areas, drag and keyboard behaviour, and accessibility labels
with the full names.

No tab shrinks below `DockStyle::min_tab_width`: by default about three characters and the "…"
plus the tab's padding, 54 pt at the 12 pt font. Only when the tabs at that width do not all fit
do the rest go to the "…" button at the end of the strip, whose menu lists the hidden panels;
choosing one makes it active. The active tab is always shown, in its logical place, and the
others take the remaining room in their order. An active tab with less room than its minimum
width, or a lone tab wider than the strip, is cut the same way. The drag bar always keeps its
reserved width: tabs shrink, then overflow, before it narrows, and only a strip narrower than the
group header plus the bar shrinks it. A tab dropped on the "…" button goes to the end of the group.

## Floating size

A panel or group floated by a drag, a menu or a command without a rect takes its content height:
the title bar, the tab strip and the height the active panel's content used when it was last
drawn (the content ui's `min_rect`), at least 120 points. The most it takes is the height of
`DockContent::floating_size`, or 320 points for a panel leaving a dock or frame and a group
leaving a dock. A short panel therefore floats as a short window. Content that reaches the bottom
of its body (a panel that allocates its whole rect) has no height of its own: it floats at the
`floating_size` height, else at 320 points. A panel's width is its `floating_size` width, else
its docked width; a group keeps its docked width.
A flyout fits its content the same way, without the title bar.

- A panel the area has not drawn floats at the `floating_size` height, else the docked height
  capped at 320, or at a `FloatSpace::with_panel_size` size for commands.
- A group from a frame without a `floating_size` keeps its frame size. A dock column of several
  groups floats at the tallest `floating_size` height, else its column height capped at 320.
- A rect from a command and a window the user resized keep their size; floating windows are not
  refitted while open.
- Docked layout does not change: a column's last group still fills the column.

## Gestures

- A tab drags one panel. A group's drag bar (the space beside its tabs) or its top border drags
  the whole group. A column bar drags a column. A floating window's title bar drags the window;
  with `title_bar_docks` a release over a target docks the whole window there, else it only
  moves. While the drag highlights a drop target, the window is drawn at 75% opacity so the
  target shows through it; over empty space, and once the drag ends, it is opaque. A native
  window moved by its only tab or tab row fades the same way.
- Inside a floating window, tabs and drag bars drag and dock as they do in the docked layout,
  into any window. A drag bar or border of a window's only group takes the whole window along,
  so, like the title bar, it looks through the window: the groups, docks and windows behind it
  take the drop while the pointer is still inside it. In a window of several groups, the other
  groups stay targets. Its groups split, join and resize like frames, with dividers and four-way
  handles (`Divider::WindowSplit` and `Divider::WindowIntersection`, named by the window's first
  panel). Double-clicking the title bar minimizes the window to its title bar, or to the tab strip
  of its only group; so does double-clicking that group's strip. The group menu's float item
  floats one group of a window that has several. Closing the window closes its panels; each
  reopens where it was while the window or its neighbour in it is open, else in a window at the
  window's last rect.
- A movable floating window resizes from every edge and corner. The top edge and corners are a
  4 pt band above the title bar (`chrome::RESIZE_BAND`), so they take no moves and no clicks on
  the title bar's buttons. A resize keeps the opposite edges in place and stays within the size
  limits and the screen. A focused resize handle moves its edge 8 pt per arrow key.
- Over a group (a frame, a dock column's group or a floating group), the pointer's place
  decides:
  - The tab strip joins the group's tabs at the insertion slot under the pointer.
  - In the body below the strip, the outer `DockStyle::side_zone_fraction` (one third) of its
    width from the left or right edge, or of its height from the top or bottom edge, docks
    beside the group on that side. Where two sides meet, at a corner, the side the pointer is
    relatively nearer wins.
  - The middle third joins the group's tabs.
  - A group that takes no tabs, such as a protected canvas, takes nothing on its strip, and its
    body has no middle: every point docks beside it on the relatively nearest side.
  - In the docking profile (`new_dock_zones` without `side_edge_zones`), the body of a frame or
    floating group that takes no tabs takes nothing, whatever its `accepts_splits`: a release
    there is a release over no target, so a floating window stays floating and a docked panel
    or group floats as `float_on_empty_drop` says. Only the bands along borders (below) dock
    beside it, as in Adobe apps, whose document area is no drop zone. A dock column's group
    keeps its sides.
- Beside a frame is a split of that frame. Beside a group in a dock column, the top and bottom
  stack a new group above or below it, and the left and right make a new column on that side.
  A group dragged to an edge of its own column, such as the window edge of the column it heads,
  makes a new column there, as one of its tabs would; when it is the column's only group,
  nothing changes.
- A frame too narrow or short for two halves of `DockStyle::min_pane` still splits when the
  frames beyond it can give the room, as they do for a dragged divider. When they cannot, the
  frame keeps its size and the new frame takes the width (or height) the moved panels have now,
  at least its minimum, from the frames beyond the frame's divider on that side
  (`Placement::SplitSized`). When those have no room either, that side joins the frame's tabs
  instead, or takes nothing when the frame takes no tabs. The gap
  between two frames is no target.
- Bands along borders come before the group zones. Each is centred on its border where the
  pointer can reach both sides:

  | Band | Drop | Field (default) |
  |---|---|---|
  | The seam between two stacked groups of a dock column | A new group between them | `seam_band` (28 pt) |
  | Each edge of a dock column | A new column on that side | `column_edge_band` (28 pt) |
  | The docked area's left or right edge without a dock, and an empty dock | A new dock | `new_dock_band` (28 pt) |
  | The edge area's top and bottom in `drop_edges` (all four sides with `side_edge_zones`) | A new frame along that whole side, a quarter of the edge area | `edge_band` (28 pt, 8 pt with `side_edge_zones`; at most a quarter of the edge area) |

  Where a band overlaps a group's outer third, both mean the same side. Without
  `side_edge_zones`, a band reaches only 4 pt past its line into a tab strip that takes tabs;
  the rest of the strip joins. The outer
  edge band is offered only when every frame keeps its minimum.
- The edge area is the root, or the part between the docks. Without `side_edge_zones` it also
  narrows to the canvas column: where neither side of a side-by-side split is a dock and only
  one side holds a protected panel, that side. The top (bottom) band also narrows a split of
  rows to its first (last) row when only that row holds the canvas, so a full-width row below
  the canvas keeps the bottom band. Side panels in split frames then get no band across the
  whole window, and the drop wraps only the canvas column. A drop on a narrowed band anchors
  on the canvas, so `Placement::Edge` narrows the same way when applied; on a band that is not
  narrowed it anchors on a panel that is not protected. The renderer tries the drop with its
  own permissions and offers the band only when the drop wraps the panels the band shows.
  `window.panel.dock` to an outer edge picks its anchor the same way, from the permissions
  `apply` (or `to_actions`) gets and the area's `FloatSpace` (`with_side_edge_zones` before it
  has drawn).
- `DockStyle::drop_edges` (a `Sides`, all four by default) lists which sides of the layout's
  outer edge, and of a group that takes no tabs (a canvas) without `new_dock_zones`, take a
  drop. There a canvas's whole body goes to the nearest listed side; with none listed it takes
  nothing. In the docking profile the canvas body takes no drop and the outer left and right
  edges are new docks, so only the outer top and bottom apply. Groups that take tabs keep all
  four sides. A dock-column app whose palettes dock only at the left
  and right, as in AutoCAD, sets `Sides::NONE` with `new_dock_zones` on. Not used with
  `side_edge_zones`.
- With `side_edge_zones` (the split-tree arrangement), a frame's zones are a picture frame
  instead: trapezoids along the four edges of the body below the tab strip, all four the same
  depth, `docking::SIDE_EDGE_ZONE_DEPTH` (40 pt), or a quarter of the body's smaller side when
  that is less, so every frame keeps a center. A dock column's groups take the same depth.
  `drop_zone` and `resolve_drop` give these zones to hosts that draw their own frames.
- A host that resolves drops over its own groups calls `resolve_drop_by` with
  `DockStyle::zone_rule()`, so its zones and previews match the dock area's in either profile.
  `resolve_drop` always gives the split-tree zones.
- A dragged divider grows the frames that touch it on one side, up to their maximums. On the
  other side the frame touching it shrinks to its minimum, then the next frame along the same
  axis, and so on: the divider stops when every frame on that side is at its minimum. Frames
  side by side across the axis shrink together, and the next frame gives way once one of them
  is at its minimum. The line of frames ends at a split across the axis or at a dock; every
  other frame keeps its size. Dividers between stacked panels and between the groups of a dock
  column work the same way. Where a divider ends on another divider, a four-way handle
  (`Divider::Intersection`) moves both.
- Every frame has the same minimum, `DockStyle::min_pane` (plus its tab strip), unless its panels
  ask for more through `PanelLimits::min`. Frames have no maximum unless every panel in them asks
  for one through `PanelLimits::max`: a panel without a maximum lifts the maximum of the panels it
  joins.
- A dock column stacks its groups top to bottom. A minimized group shows only its tab strip, and a
  group the user resized keeps that `Group::height`. A group the user never resized (`height` is
  `None`) takes the default height its panels ask for through `DockContent::dock_heights`. Groups
  whose panels ask for a `fill` height share the rest of the column, and groups whose panels ask
  for no heights take their minimum. When no group asks for a fill, the groups without heights
  share the rest, and when every open group has a height, the last one fills the column. In a short column the groups at their default give way first, down
  to their `compact` heights, so the filling group gets its `fill` height. When the column is still
  too short, every group gives way down to its minimum, and past that the column scrolls. Defaults
  give way nearest the filling group first: one is at its compact height before the next gives
  way. Towards the minimums, groups give way in proportion to their room. A group takes the
  largest default, compact and fill height of its panels. All three are body heights, without
  the tab strip, like `PanelLimits`.
- Drop previews have four shapes, the same for tab and group drags:
  - A drop beside something shows the rect the dragged panel, group or column will take, at its
    real position and size, as a 2 pt accent outline with a light fill. The renderer applies
    the drop to a copy of the layout and lays it out with the code that draws it, so the rect
    is the one the panel's group gets on release: the new frame for a split (including
    `Placement::SplitSized`), the new group for a stack above or below a dock group or on a
    seam, and the whole column, bar included, for a new column between columns or beside a
    dock group's side.
  - A drop on a band along the layout's outer edge shows a `DockStyle::drop_line_width` (4 pt)
    accent line along that edge: the new-dock band, the window edge of a dock's outermost
    column, and the outer edge band (`Placement::Edge`), whose line spans the edge area it
    wraps. A drop between the icon groups of an iconic column shows the same line there.
  - A drop that joins a group shows the same outline and fill around the whole group, and on
    its strip the 2 pt insertion marker.
  - A drop that floats shows the floating rect.

  A drop that would leave every panel where it is, changing only sizes, shows nothing and
  changes nothing: a lone group dropped beside the group it already borders, or a frame's tab
  on the layout edge the frame already holds. Split sizes, column widths, group heights and
  window rectangles are not compared, and a run of splits along one axis is the same however
  it nests.

  A group drag also shows its ghost at the pointer. With `side_edge_zones`, a drag over a group
  instead outlines each of its zones that a drop would change something in (the side
  trapezoids, the center rectangle and the tab strip; `zone_shape` gives the outlines) and
  fills the zone under the pointer. Sides the group does not take, by its permissions or
  because a split leaves no room, are not drawn; nor is the center of the group a tab comes
  from. A drop there that the zones do not show, such as the outer edge band, fills the region
  it takes, and dock drops show the line along the edge they dock to. Escape or losing window focus cancels a drag.
- A drag in progress (a tab, group, column, window bar, native window bar or divider, in any of
  the app's windows) also ends when the layout passed to the area is not the one it drew last
  frame, with or without its returned actions applied: the app switched workspaces, ran Reset
  or changed the layout by command. Sizes and rectangles are not compared, so an app may clamp
  a moved window or adjust a split during the drag. Nothing is restored from before the drag;
  windows go where the new layout has them. `cancel_drag` ends the area's drags at any other
  time. A native window moved by the compositor (`ViewportCommand::StartDrag`, as on Wayland)
  is not the area's drag: it ends when the user releases the button. On macOS, where AppKit
  moves a native window by its bar, a cancel ends the area's drag, so the release drops
  nothing, but AppKit keeps moving the window until the release.
- With `float_on_empty_drop`, a release over no target floats what moves at the pointer, such
  as a panel dropped on a protected canvas. Without it, that release cancels the drag and
  nothing moves: the gap between two groups, a menu bar or a status bar is no target. A release
  outside the app window floats in both cases.
- Ctrl (⌘ on macOS) held at the drop floats it wherever it lands. With `command_press_floats`,
  the key held when a tab drag of a frame starts is enough. With `float_hint`, a hint under the
  pointer names the modifier while a drag offers a docking target: `float_hint_label`, where
  `{key}` becomes ⌘ or Ctrl.
- Double-click a tab or drag bar to minimize a group to its strip.
- Right-click a tab for its menu, or a drag bar for the group menu; Enter or Space on a focused
  drag bar opens it too. A left click on the drag bar only focuses it.
- With `DockStyle::wheel_selects_tabs`, the mouse wheel over a tab strip selects the next or
  previous tab. With `middle_click_closes_tabs`, a middle click on a closable panel's tab closes
  it.
- A column's »/« button collapses it to icons, one icon per panel with a separator between groups.
  Widen an iconic column to show panel names. Click an icon to open its group as a flyout on the
  canvas side. Drag an icon to move its panel, or an icon group's top edge (the separator line above it) to move the group.

## Commands

With the `serde` feature, `DockCommand<P>` is the shared control vocabulary. The app puts the
command id under `"command"` in the params and deserializes them. craft-ui has no `serde_json`
dependency.

```rust,ignore
let mut params = params.clone();
params["command"] = json!(id);
let command: DockCommand<PanelId> = serde_json::from_value(params)?;
if command.is_app_owned() {
    return app.workspace_command(command);
}
let space = float_space(ctx, DOCK_ID).unwrap_or_else(|| FloatSpace::new(ctx.content_rect()));
let closed = command.apply(&mut app.workspace, style.right_to_left, &space, |panel| app.permissions(panel))?;
```

| Id | Params (`?` is optional) |
|---|---|
| `window.panel.layout` | `{action}`: a serialized `Action` |
| `window.panel.move` | `{panel, anchor, zone?, before?, group?, placement?}` |
| `window.panel.float` | `{panel, x?, y?, width?, height?, group?, column?}` |
| `window.panel.dock` | `{panel, anchor?, zone?, before?, group?, column?, side?}` |
| `window.panel.activate`, `.close`, `.closeGroup`, `.closeOthers` | `{panel}` |
| `window.panel.minimize` | `{panel, minimized?}` |
| `window.panel.iconic` | `{panel, iconic?}` |
| `window.panel.window` | `{panel, native?}` |
| `window.workspace`, `.new`, `.delete` | `{name}` |
| `window.workspace.reset`, `ui.dock.reset` | `{}` |
| `window.workspace.lock` | `{locked?}` |
| `ui.dock` | `{action}`, or `{operation, …}` |

- `zone` is `tab`, `center`, `left`, `right`, `top`, `bottom`, `above` or `below`. `before` names
  the tab to insert before; `null` appends. `left`, `right`, `top` and `bottom` follow
  `Placement::for_zone`, so beside a docked panel they make a new column or group. `apply` and
  `to_actions` take the layout's `right_to_left` for that.
- `group` moves the panel's whole group; `column` moves its dock column, or its whole window when
  it floats. `side` is `Start` or `End`: what moves becomes a new last column of that dock, or
  a new dock; when it already is that dock's last column, the command does nothing.
- `window.panel.dock` with `left`, `right`, `top` or `bottom` and no `anchor` or `side` makes a
  new frame on that outer edge (`Placement::Edge`), wrapping what a drag to that edge would
  (see the edge area above). A hidden panel is shown first.
- Showing a panel (`activate`, `dock` without a destination, `ui.dock` `open`) restores a hidden
  panel's place; a panel never laid out opens as `Open` without an anchor does.
- `window.panel.float` without a rect floats what it moves at the size a drag floats it at (see
  Floating size). It goes
  inside the dock area, 24 points from its top-left corner, so it stays clear of the menu bar.
  The area records that geometry in a `FloatSpace` each frame; `float_space(ctx, id)` reads it
  and `apply` and `to_actions` take it. Before the area has drawn, `FloatSpace::new(area)` floats
  a panel at 260x320 and a group or column at its column width by 320.
  `FloatSpace::with_panel_size(panel, size)` gives a panel the area has not drawn (a hidden
  panel, a headless run) the host's own size; a size the area recorded wins. `x`, `y`, `width` and
  `height` replace the matching values, and a group or column that is already a whole floating
  window keeps its rect.
- Without a value, `minimized`, `iconic` and `locked` toggle.
- `ui.dock` operations are those of LightCraft, PDFCraft, CADCraft and SoundCraft: `open`,
  `close`, `activate`, `float {rect}`, `moveFloating {rect}`, `move {target, zone?, before?}`,
  `setStackOpen {open}`, `resizeStack {height}` and `resizeSplit {path, size}`. `rect` is
  `[x, y, width, height]`.

`to_actions` returns the actions a command makes without applying them. `apply` applies them all
or none and returns the panels it closed, so the app can release their state. Showing a hidden
panel and Lock change the workspace directly. Named workspaces and Reset belong to the app:
`is_app_owned` is true for them and `apply` refuses them. `DockCommand::IDS` lists every id for
the app's command registry. App-specific aliases stay in the app.

## Native windows

With feature `native-windows`, on a backend with egui viewports (`!ctx.embed_viewports()`,
eframe on desktop), every floating window opens in its own OS window: a Ctrl/Cmd-drag, Undock
Panel and Undock Panel Group, every other drop that floats, and the floating windows of a
restored layout. A window still in-window opens over its place in the app window (the main
viewport's `inner_rect.min` plus its rect). A window with a panel whose
`DockContent::allow_native` returns false (it returns true unless the app overrides it), or
that is not floatable, stays in-window. Floating windows get no taskbar or Alt-Tab entry
(`with_taskbar(false)`).

On Wayland floating windows stay in-window: the compositor reports no window positions, so a
native window could not dock again by drag or reopen where it was. Their `native` rects are
kept, and the same layout opens native windows on X11, macOS and Windows. The dock area takes
Wayland to be the backend when `WAYLAND_DISPLAY` or `WAYLAND_SOCKET` is set (winit's rule) and
the main window has never reported its `outer_rect`; an app that forces X11 in code gets native
windows from the first frame. `DockArea::native_windows(bool)` overrides the choice either way.

- **Identity.** `Floating::id` names the OS window, so a window keeps it when its panels change,
  including when its first panel moves out. `Floating::new` draws a fresh id and
  `Floating::with_id` sets one. A saved layout stores every window's id and loads with unique
  ids: a window saved without an id, or with one an earlier window took, gets a fresh one. A
  window that merges into another takes the receiver's id, and a whole window that moves
  (floated again, or its only group dragged) keeps its own.
- **Kind and level.** `DockContent::window_kind` sorts panels into palettes
  (`WindowKind::Palette`, the default) and viewers (`WindowKind::Normal`), such as a video
  monitor on a second display. A window holding any `Normal` panel is a normal window, whichever
  of its tabs shows; the rest are palettes.
  - Palettes stay above other windows (`WindowLevel::AlwaysOnTop`), so clicking the document
    does not bury them. They stay shown while the app is in the background, so files dragged
    from another app reach them and an OS sheet attached to one stays open. While none of the
    app's windows has focus they take `WindowLevel::Normal`, so they do not cover other apps
    (on Windows `WindowLevel::AlwaysOnBottom`: there `Normal` would put them in front of the
    app the user switched to), and they go back to `AlwaysOnTop` when one has it again. The app counts as in the
    background after 250 ms without focus, so focus moving between its own windows changes
    nothing. The level is sent only when it changes.
  - Normal windows have `WindowLevel::Normal` and stay shown while the app is in the
    background. They stack with the main window by focus, below every palette; a drag over a
    normal window that lies behind the main window targets the main window where the two
    overlap, and the normal window outside the main window.
  - Like palettes, normal windows have no taskbar entry; they come forward with the app
    instead. When one of the app's windows gains focus after none had it for 250 ms,
    the main window and each normal window get `WindowLevel::AlwaysOnTop` and then
    `WindowLevel::Normal`, one window per frame, in their focus order from back to front.
    They keep their order, nothing takes focus, and the window the user activated is raised
    last, so it stays in front. The focus order counts a press in the main window as focusing
    it, since the window system raises it then. The main window is left at the normal level,
    so an app that keeps its main window always on top should not use normal windows. On
    Windows the toggle puts the window in front of every window that is not topmost, without
    activating it; on X11 the window manager raises a window as it takes
    `_NET_WM_STATE_ABOVE`. egui 0.36 cannot make the main window a native window's owner,
    which on Windows would bring the window forward with the main window, and
    `ViewportCommand::Focus` would take focus. On
    macOS nothing is raised: activation from the Dock or the application switcher brings all
    of the app's windows forward, and a click brings the clicked window alone, as with the
    app's other windows.
- **Bar.** Each window has a bar at the top, 18 points high, in the floating title bar's style:
  the title (the shown panel of its first group) and ×. Dragging the bar moves the window. On
  macOS the press on the bar hands the move to AppKit, and on Wayland, where a window cannot
  be placed, the bar hands the move to the compositor. On
  Windows and Linux the OS frame is off. On macOS the frame stays, with its title bar and
  buttons hidden, for its resizing, rounded corners and standard window shadow. The dock area
  paints no shadow of its own around a native window.
- **The ×.** Its place is set in `render/native.rs` (`MACOS_CLOSE`, `OTHER_CLOSE`): the
  glyph's inset from the top and trailing edges, its size, and how far the hit area reaches
  past the glyph on each side. On macOS the 9 pt glyph sits where macOS puts the close button
  of its own windows: centered in the bar, its trailing side 8 pt in. AppKit's corner resize
  area covers the glyph's trailing half and takes presses there, so the hit area reaches 10 pt
  further toward the leading edge and down to the bar's bottom. Elsewhere the × is an 18 pt
  square, its trailing side 7 pt in. With `CRAFT_DOCK_DEBUG_CLOSE=1` in the environment,
  native windows outline the ×'s hit area in red and, on macOS, AppKit's resize areas in
  orange.
- **Resize.** A movable window resizes from its edges and corners. On macOS AppKit does it,
  with its own areas and cursors. Its corner area reaches about 15 pt into the window along
  each edge (a 19 pt square, measured on macOS 26) and takes presses before the content sees
  them. Over AppKit's areas the window sets no cursor of its own, which would flicker with
  AppKit's. Elsewhere the window resizes from bands 6 pt deep along its edges, and each corner
  is an L along both edges, 16 pt long and as deep as the edges. The bands leave out the bar's
  buttons, so the corner's grab runs down beside the ×; elsewhere the top band lies over the
  bar's drag area. The OS runs the resize from a band (`ViewportCommand::BeginResize`).
- **Close.** The bar's × and the OS close request close the window's panels, as the
  floating window's × does. They reopen from the app's Window menu. A window with a panel that
  may not close, or in a locked area, has no × and answers the OS close request with
  `ViewportCommand::CancelClose`.
- **Place.** `Floating::native` is the desktop rect of the window's content (egui's
  `inner_rect`), and the in-window `rect` follows it for builds that draw the window in-window.
  The content is also the window's origin for drags; only `ViewportCommand::OuterPosition`
  takes the outer position, which the renderer works out from the reported outer and inner
  rects. Where the window system puts a new window's frame, not its content, at the place asked
  for, the window is moved once so its content is there. When the app changes `native` (undo,
  a workspace reset, a reloaded layout whose ids match), the OS window moves there; a report
  that differs from `native` while a commanded move has not landed (up to 0.5 s) is ignored. `Floating::monitor` is the monitor
  holding the window's center, from the list the app passes to `DockArea::monitors` (each a
  `Monitor { name, rect, scale }` in desktop logical points). egui 0.36 reports no monitors, so
  an app fills the list from its backend when it can; without one, `Floating::monitor_size`
  keeps the monitor size egui reports.
- **Restore.** Only the windows of a loaded layout (a `native` the session has not shown yet)
  are restored. A window made this session (a drag out, Undock, a Ctrl/Cmd drop) opens at the
  rect it was given, only made safe, wherever that is on the desktop. A restored window opens on
  its saved monitor when one with that name (or else that rect) is connected, moved in
  proportion if the monitor's rect changed, and moved onto it when too little of its bar lies on
  a monitor to grab (40 by 9 points). Otherwise it opens on the main window's monitor at the
  same offset from that monitor's origin, wholly on screen. Without a monitor list, it is
  clamped onto the main window's monitor, taken from `monitor_size` and the main window's
  position. Hostile rects are made safe first. With a monitor list, a window whose `native` the
  app changes, or whose bar drag ends with too little of the bar on a monitor, goes onto the
  nearest monitor too.
- **Mixed scale factors.** Rects are stored in egui points as the window reports them, and
  `Monitor::scale` is kept but not used. On macOS every window shares one point space, so this
  is exact. On Windows egui-winit divides each window's physical position by that window's own
  scale factor, so with monitors at different scales (say 100 % and 150 %) the points of windows
  on different monitors are in different units. Across such monitors, a restored or
  app-changed place can land off by the ratio of the scales, and a drop across windows on
  different monitors can resolve against the wrong point. Same-scale setups are unaffected.
- **Shortcuts.** egui gives each OS window only its own input. Key, text, copy, cut and paste
  events that reach a native window and that none of its widgets used come back in
  `DockOutput::forwarded_events`, with the modifiers held in that window in
  `DockOutput::forwarded_modifiers` (egui-winit sends Cmd/Ctrl+C, X and V as `Copy`, `Cut` and
  `Paste` without the key). The app feeds them to the shortcut handling it runs on the main
  window. A window where any widget has keyboard focus forwards nothing, and an Escape that
  cancels a drag is not forwarded.
- **Input methods.** egui's IME descriptor (`PlatformOutput::ime`) goes out from the pass of
  the window whose editor has the keyboard. A `craft_ui::ime::Composer` in a panel body of a
  native window records its descriptor while the window draws, and the dock area publishes it
  once per pass, after the window's last body. A body drawn after the editor's cannot clear it
  (egui-winit turns the IME off in a pass without one, ending the composition). A descriptor
  written directly in that pass, as a focused `TextEdit` writes one, is kept. An app whose own
  editor publishes the descriptor does it in `DockContent::window_drawn`, called once per
  window pass after the last body. Floating windows drawn in the main window (the web,
  Wayland) are part of its pass, which publishes as it does without docking.
- **Modal dialogs.** egui's `Modal` blocks input only in the viewport it is shown in. While the
  app shows one, it passes `DockArea::input_blocked(true)`. Each native window is then drawn
  under `DockStyle::blocked_dim` (by default the panel fill at 60 %), and its bar, tabs, body
  and resize edges take no input: its widgets are disabled, the pointer, key, text and scroll
  events are dropped at the start of its frame, and a layer over its content takes the pointer,
  so a popup left open takes no click either. It forwards only key releases, so a key pressed
  before the block does not stay down in the app's shortcuts. It reports no `window_files`,
  offers no drop targets, starts no move or resize, is not focused by `raise_panel`, and
  answers the OS close request with `ViewportCommand::CancelClose`. On macOS it is not
  resizable meanwhile.
  - *Stacking.* A palette drops to `WindowLevel::Normal` while blocked, so a click on the main
    window brings it in front of a palette that covers the modal. It stays shown and at that
    level when the app goes to the background and returns. When the app is activated meanwhile, the
    main window is raised last, in front of the normal windows and of a palette the user
    clicked.
  - *Edges.* Turning blocking on cancels a drag in progress, and a widget of a native window
    loses keyboard focus. If a native window has the OS keyboard focus then (the modal was
    opened from it, say by a double-click in a floating panel) and still has it 0.2 s later,
    the main window gets `ViewportCommand::Focus`, so the modal gets the keys. The wait lets an
    OS dialog opened with the modal (a file dialog) take focus first; then nothing is sent. A
    zoom gesture still shows in egui's `InputState::zoom_delta`, which cannot be cleared; a
    widget that zooms only while hovered ignores it. egui smooths wheel
    scrolling, so a scroll area in a native window may still scroll for about 100 ms after
    blocking ends, with the remainder of a wheel turn made just before it began.
  - Floating windows drawn in the main window need nothing: the modal covers them.
- **Input hook.** eframe runs `App::raw_input_hook` for the main window only. An app that
  rewrites input there (say, Ctrl+click to a secondary click on macOS, or blocking input
  during automation) passes the same rewrite to `DockArea::native_input_hook` every frame as a
  closure. eframe's hook takes `&mut self`, so state it shares with the closure goes in an
  `Arc<Mutex<…>>`. The dock area runs it on each native window's `RawInput` (`viewport_id` names the window; see
  `DockArea::window_viewport`). It runs as an egui `Plugin::input_hook`, before egui reads the
  input, so widgets see rewritten pointer and key events as if the window system had sent them.
  It runs once per pass, so a frame that egui runs twice calls it a second time with the
  events already taken. The hook must be `Send + 'static`, as egui plugins are.
- **Dropped files.** Files dragged over or dropped on a native window come back in
  `DockOutput::window_files`, one `WindowFiles` per window that had any this frame: the
  window's id and viewport, the panel whose body is under the pointer (else the window's
  shown panel, since the window system may send no pointer moves during a file drag), the
  pointer's last position in the window, and the window's `hovered_files` and
  `dropped_files`. Files on the main window stay in its own `RawInput`.
- **Drags.** Tabs, groups and whole windows drag between the app's OS windows, and the app's
  own drags in panel bodies reach the window under the pointer.
  - *Ownership.* The window where the button went down owns the drag: the window system sends it
    the pointer's moves and the release while the button is held, inside the window or not. So
    the pointer leaving a window (`PointerGone`) is not a release. A drag ends on release,
    Escape (pressed in any of the app's windows, since keys go to the one with keyboard focus),
    when none of the app's windows has focus, or when its window closes or its panel goes. A
    window that AppKit moves (macOS, below) ignores Escape.
  - *Hit-testing.* The owner adds its desktop origin to the pointer: a native window's reported
    `inner_rect.min`, or while it moves by its bar, the place it was last put less the pointer's
    offset from the pressed point. On macOS a window moving by its bar is hit-tested at
    AppKit's cursor (`NSEvent::mouseLocation`). The window under that
    desktop point is a floating native window in front of the main window first (palettes, then
    normal windows, most recently focused first), then the main window, then a normal window
    behind it. A window moving by its bar is never its own target. A point in no window is on the
    desktop.
  - *Targets and previews.* Each window draws its drop targets in its own coordinates, and the
    window under the pointer draws the drop preview (zone, result rect, ghost) in its own frame.
    The source window shows only the "Hold {key} to undock" hint. The release is resolved at the
    end of the main window's frame, after every native window has drawn.
  - *Drops.* Over a window, a tab or group docks or joins there with the usual targets: dock lines
    and frame zones in the main window, frame zones and tab rows in floating windows. Over the
    desktop, it floats into a new OS window placed so the grabbed tab stays under the pointer
    (the window's top-left is the pointer minus the grab offset, less the bar's height). A
    Ctrl/Cmd-drag floats there wherever it is released.
  - *Moving a window.* Dragging the bar, the only tab of a window, or the drag area of its only
    group moves the window, so the pressed point stays under the pointer.
    - *macOS.* AppKit moves the window (`ViewportCommand::StartDrag`, which winit turns into
      `performWindowDragWithEvent`), as it moves its own windows: from the press on the bar,
      or from the start of the tab or group drag. The dock area sends no `OuterPosition`
      meanwhile. egui keeps running frames, but the window's reported place lags the move by up
      to a tenth of a second, so each frame takes the window's place as AppKit's cursor
      (`NSEvent::mouseLocation`, flipped against the primary screen's height and divided by the
      zoom factor) less the pressed point, and hit-tests targets, fades the window and routes
      the drag at the cursor. The drag ends on the first frame where AppKit
      (`NSEvent::pressedMouseButtons`) or egui reports the release; egui's can come a frame
      later, and the drag is resolved once. Ctrl/Cmd is read from AppKit's modifier flags,
      since AppKit sends egui no key events during the move; for the same reason Escape does
      not cancel it, as with Finder's windows. A press on the bar released within the click
      distance docks nothing.
    - *Elsewhere.* The dock area moves the window with `OuterPosition`. Each move is worked
      out from a pointer move of that frame and the window's reported place, which the
      pointer's position is relative to; a move commanded earlier may land late. A frame with
      no pointer move leaves the window. X11 sends no move whose position in the window repeats
      the last one, so while the pointer keeps exact pace with the window the window can trail
      it by one move until the next. Escape puts the window back.
    - *Drops.* Released over a target in another window, the whole window docks or joins there
      as its groups (`Action::MoveWindow`). The bar docks only with
      `DockStyle::title_bar_docks`; the only tab or group docks either way, since it is the way
      back in with that flag off. Targets: in the main window a dock line, a frame's tab row or
      edge zone, or the layout's outer edge; in a floating window a frame zone or tab row. A
      frame body's center takes a window moved by its only tab or group, as that tab or group
      drag would, but not one moved by its bar. Over the desktop, or with Ctrl/Cmd held, it
      stays where it was moved, and the layout takes its place once, at the release. An
      in-window floating window's title bar still splits only groups of other floating windows.
  - *Wayland.* With `DockArea::native_windows(true)` on Wayland, or wherever `outer_rect` is
    `None`, desktop positions are unknown, so drops across windows cannot be resolved. A drag
    works inside its own window, and a release outside it floats a new window that the
    compositor places. Moving a window hands the move to the
    compositor, which never docks it.
  - *App drags in panel bodies.* A drag the app starts in a panel body can reach the window
    under the pointer, so a drop site there works as it does inside one window.
    - *Which drags.* An egui drag of one of the app's widgets, pressed with the primary button
      in a panel body, that carries something: egui's `DragAndDrop` holds a payload
      (`dnd_set_drag_payload`), or the app calls `docking::route_app_drag(ctx, area_id)` each
      frame while the drag lasts, from the body or before the area shows. An app that keeps its
      payload in the context's data makes that call. Other drags (sliders, scrollbars, text
      selection, canvas pans) stay in their window and keep getting its pointer. A call counts
      for its frame and the next; once routing starts, it lasts until the drag ends.
    - *Events.* While the drag lasts, the owner's pointer moves over another of the app's
      windows (found as for tab drags, front window first) go to that window in its own
      coordinates: `PointerMoved` while the pointer is over it, `PointerGone` when it leaves,
      and the release as a primary `PointerButton` with `pressed: false`. That window sees
      `pointer_hover_pos` and `any_released` as if the window system had sent them. It sees no
      press, so `any_down` stays false there and nothing in it is clicked. The owner sees the
      pointer gone while it is over another window, so its panels under a palette see no hover
      and no drop.
    - *Input path.* Events go in through the area's egui input plugin, before any widget reads
      the input; the app calls nothing more. In a native window this runs before the app's
      `native_input_hook`. eframe runs the main window's `App::raw_input_hook` earlier, so that
      hook sees the main window's input without the routed events.
    - *Release timing.* A window drawn after the owner in the same pass gets the events in that
      pass (from the main window, every native window does). The main window, and a native
      window drawn before the owner, get them on the next pass, which is requested. When the
      release goes to another window, the owner gets its own release, and the pointer events
      that came after it, in the main window's next pass (a rerun of a discarded pass does not
      count), so its drag stops (`drag_stopped`) then. egui clears a `DragAndDrop` payload when the owner sees the
      release, so the target always reads its release while the payload is there. An app that
      keeps its own payload clears it no earlier than the owner's release.
    - *Limits.* Nothing is routed where desktop positions are unknown (Wayland,
      `native_windows(false)`, embedded viewports) or while input is blocked. Escape in any of
      the app's windows, a focus loss, `cancel_drag` and a replaced layout stop the routing:
      the window under the pointer sees it go and gets no release it has not read yet. The drag
      goes on in its own window until the button comes up, and the owner still gets its
      release.

On the web and wherever egui embeds viewports, nothing changes: floating windows stay in the
page at their in-window rect, `native` and the monitor fields are kept but unused, the input
hook never runs, and `forwarded_events` and `window_files` stay empty. Without the feature, the fields exist and are never set by the
renderer.

## Labels

Every visible and accessibility label is a `*_label` field of `DockStyle`, with an English
default: title case for menu items, sentence case for names only screen readers announce. `DockStyle::localized` sets them all from one keyed lookup:

```rust,ignore
let style = DockStyle::from_ui(ui).localized(|key| app.strings.get(key).cloned());
```

Keys are `dock.close`, `dock.float`, `dock.collapseToIcons` and so on; the rustdoc of `localized`
lists them all. `DockContent::label` and `action_label` can shape panel and action text further;
the logical label stays the accessibility name.

## Testing

`DockArea::hit_rects(ctx, id)` lists every gesture source drawn last frame as a `Hit` with its
rect: tabs, drag bars, borders, column bars, »/« buttons, title bars, close buttons, icons, icon
grips, resize edges and dividers. It is always compiled, so an app can return it from a
`ui.docking` control method next to the layout and the focused panel.

`DockArea::hit_id(id, &hit)` is the egui widget id of a control, for tests and automation that
focus it or read its response. `DockArea::floating_layer(id, &window)` is a floating window's
egui layer: compare its place in `memory.layer_ids()` with another layer to tell whether the
window covers it, and read its rect with `memory.area_rect(layer.id)`. A window the area does not
draw (its panels hidden, or a frame maximized) keeps its last rect there, so check
`memory.areas().is_visible(&layer)` first.

The `testing` feature adds `docking::testing::DockDriver`. It runs a `DockArea` in a headless
egui context with real pointer and key events, and applies the actions each frame emits.

```rust,ignore
let mut driver = DockDriver::new(app_layout(), Panels::default());
let bar = driver.hit(&Hit::DragBar(PanelId::Layers)).unwrap();
driver.drag(bar.center(), egui::pos2(2.0, 300.0));
driver.settle();
assert!(driver.hit(&Hit::ColumnBar(PanelId::Layers)).is_some());
```

It also has `press`, `release`, `click`, `hold`, `key`, `focus`, `hits`, and checks for painted
previews (`filled`, `outlined`). With `native-windows`, `with_native_host(origin)` stands in for
OS windows: the main window's content sits at `origin` on the desktop, and each native window
runs its own frame with its own input and desktop rect (`host()`, `window_step`,
`window_click`). `NativeHost` can report a monitor size (`monitor_size`), give windows a frame
between their outer position and content (`frame`), land `OuterPosition` a frame late (`lag`),
and report no positions (`no_position`, as on Wayland). `window_pointer` moves the pointer to a
desktop point in a native window. With `moves_windows` the host moves a window for
`StartDrag` itself, as AppKit does: the window keeps the pointer at the pressed point, reports
its place as where the drag began until the release, and the host tells the cursor and the
button (`held`; a test sets it to `false` to report a release before egui does). A `HostWindow` takes files dragged
over it (`hovered_files`) or dropped on it (`dropped_files`), and `with_native_input_hook`
passes a hook to `DockArea::native_input_hook` every frame.

`crates/ui/examples/docking_native.rs` (`cargo run -p craft-ui --example docking_native
--features native-windows`) shows a dock with two native floating windows and prints every
action and window place, for checking moves on a real window system.

craft-ui's own tests in `crates/ui/tests/docking_workflows.rs`:

- Load every app's v1 layout fixture, dock groups into a new dock, a bottom band and a seam,
  collapse and expand the column, float it by command and drag it back, then check the layout
  reads back as written.
- Render Layers, Properties and Brushes panels with real content, and merge, detach and redock
  Brushes by pointer at two widths, two scales and both themes. Set `CRAFT_UI_DOCK_VISUAL_DIR`
  to keep the captures:

```sh
CRAFT_UI_DOCK_VISUAL_DIR=/absolute/output/path cargo test -p craft-ui --test docking_workflows --all-features
```

An adopting app still tests its own commands, contents, saved workspaces and themes.

## What stays in the app

- The panel registry, panel contents and document state.
- Named workspaces, Reset, and when and where layouts are saved.
- Menus and key bindings outside the docking chrome.
- Native window ownership on platforms without egui viewports.
