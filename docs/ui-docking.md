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
| `Split` | Two children, horizontal or vertical, with a ratio. A fixed first or second size from a saved layout is drawn at that size until the divider moves; a moved divider stores a ratio. |
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
- A `Target` is `At { anchor, placement }`, `Float { rect }` or `NewDock { side }`.
- A `Placement` is a tab position, a split zone, a new group above or below (with an optional
  preferred `height`), a new column, or a new frame on an outer edge.
- Containers take different placements: a dock column takes groups and columns, not splits.
  `Placement::for_zone(layout, anchor, zone, right_to_left)` maps a left, right, top, bottom or
  center zone onto what the anchor's container takes: a split in the frame tree, a new column on
  that side or a new group above or below in a dock. A floating window takes splits as the
  docked layout does; `Placement::Group` on a floating anchor splits its group above or below.
- A side split zone puts what moves in a new half-size split of the anchor's frame, and no
  other frame changes size. The new split has its own divider, and the drop preview shows the
  half the new frame takes.
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
| `set_visible(panel, visible, fallback, permissions)` | Hide a panel that may close, or show it at its recorded place, else beside `fallback`. A group or window closed one panel at a time comes back as one group as its panels are shown again. |
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
permissions, size limits, preferred floating size, panel menu and group header. The group header
sits at the end of the tab strip; `group_header_width` gets the strip width, so a header that
draws its own tabs can size them to the room left. An icon is an image (`icon`), or drawn in code by `paint_icon`, which
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
| `auto_collapse_iconic(on)` | Close a flyout on Escape or a click outside (default on). |
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
`docking::chrome` has the floating frame, the title bar and `drop_preview` (the drop preview
docking and `craft_ui::panes` both draw) for apps that draw their own windows.

## Tab strips

Every shown tab carries its panel's full name. Tabs that do not fit go to the "…" button at the
end of the strip, whose menu lists the hidden panels; choosing one makes it active. The active tab
is always shown, in its logical place: when it lies past the tabs that fit, it takes the place of
the last of them. Only an active tab wider than the room beside the "…" button, or a lone tab
wider than the strip, is cut with "…", and its tooltip names it in full. The drag bar always keeps
its reserved width: tabs go to the overflow menu before it narrows, and only a strip narrower than
the bar itself shrinks it. A tab dropped on the "…" button goes to the end of the group.

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
  moves.
- Inside a floating window, tabs and drag bars drag and dock as they do in the docked layout,
  into any window. Its groups split, join and resize like frames, with dividers and four-way
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
- Over a frame, the tab strip and the middle join its tabs, and the nearest side splits it. The
  side zones are trapezoids along the four edges of the body below the tab strip, like a picture
  frame, all four the same depth: `DockStyle::zone_depth` (40 pt), or a quarter of the body's
  smaller side when that is less, so every frame keeps a center. A frame too narrow or short for two
  halves of `DockStyle::min_pane` still splits when the frames beyond it can give the room, as they
  do for a dragged divider; when they cannot, the drop joins the frame's tabs instead. The gap between two frames is no target.
- The outer edge band (`DockStyle::edge_band`) takes a drop as a new frame across the whole
  window side: top and bottom, and left and right with `side_edge_zones`. It takes a quarter of
  the window and is offered only when every frame keeps its minimum. Past the band, a frame's own
  side zones apply.
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
- Drop previews show the destination: an outline for tabs, a line for a new group, column or dock,
  a filled half for a split. With `side_edge_zones`, a drag over a group instead outlines each of
  its zones that a drop would change something in (the side trapezoids, the center rectangle and
  the tab strip; `zone_shape` gives the outlines) and fills the zone under the pointer. Sides the
  group does not take, by its permissions or because a split leaves no room, are not drawn; nor is
  the center of the group a tab comes from. Escape or losing window focus cancels
  a drag.
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
  new frame on that outer edge (`Placement::Edge`). A hidden panel is shown first.
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

- **Identity.** `Floating::id` names the OS window, so a window keeps it when its panels change,
  including when its first panel moves out. `Floating::new` and loading draw fresh ids, and
  `Floating::with_id` sets one; a layout
  loads with unique ids; a window that merges into another takes the receiver's id, and a whole
  window that moves (floated again, or its only group dragged) keeps its own.
- **Level.** The windows stay above other windows (`WindowLevel::AlwaysOnTop`), so clicking the
  document does not bury them. While none of the app's windows has focus they hide
  (`ViewportCommand::Visible(false)`), and they come back when one has it again. The hide waits
  250 ms, so focus moving between the app's own windows does not flicker.
- **Bar.** Each window has a bar at the top, 18 points high, in the floating title bar's style:
  the title (the shown panel of its first group) and ×. Dragging the bar moves the window. On
  Wayland, where a window cannot be placed, the bar hands the move to the compositor. On
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
  floating window's × does. They reopen from the app's Window menu.
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
- **Drags.** Tabs, groups and whole windows drag between the app's OS windows.
  - *Ownership.* The window where the button went down owns the drag: the window system sends it
    the pointer's moves and the release while the button is held, inside the window or not. So
    the pointer leaving a window (`PointerGone`) is not a release. A drag ends on release,
    Escape (pressed in any of the app's windows, since keys go to the one with keyboard focus),
    when none of the app's windows has focus, or when its window closes or its panel goes.
  - *Hit-testing.* The owner adds its desktop origin to the pointer: a native window's reported
    `inner_rect.min`, or while it moves by its bar, the place it was last put less the pointer's
    offset from the pressed point. The window under that
    desktop point is a floating native window first, most recently focused first, then the main
    window. A window moving by its bar is never its own target. A point in no window is on the
    desktop.
  - *Targets and previews.* Each window draws its drop targets in its own coordinates, and the
    window under the pointer draws the drop preview (zone, dock line, ghost) in its own frame.
    The source window shows only the "Hold {key} to undock" hint. The release is resolved at the
    end of the main window's frame, after every native window has drawn.
  - *Drops.* Over a window, a tab or group docks or joins there with the usual targets: dock lines
    and frame zones in the main window, frame zones and tab rows in floating windows. Over the
    desktop, it floats into a new OS window placed so the grabbed tab stays under the pointer
    (the window's top-left is the pointer minus the grab offset, less the bar's height). A
    Ctrl/Cmd-drag floats there wherever it is released.
  - *Moving a window.* Dragging the bar, the only tab of a window, or the drag area of its only
    group moves the window, so the pressed point stays under the pointer. Each move is worked
    out from a pointer move of that frame and the window's reported place, which the pointer's
    position is relative to; a move commanded earlier may land late. A frame with no pointer
    move leaves the window. X11 sends no move whose position in the window repeats the last
    one, so while the pointer keeps exact pace with the window the window can trail it by one
    move until the next. Released over a target in another window, the whole window docks or
    joins there as its groups (`Action::MoveWindow`). The bar docks only with
    `DockStyle::title_bar_docks`; the only tab or group docks either way, since it is the way
    back in with that flag off. Targets: in the
    main window a dock line, a frame's tab row or edge zone, or the layout's outer edge; in a
    floating window a frame zone or tab row. A frame body's center takes a window moved by its
    only tab or group, as that tab or group drag would, but not one moved by its bar. Over
    the desktop, or with Ctrl/Cmd held, it stays where it was moved, and the layout takes its
    place once, at the release. Escape puts it back. An
    in-window floating window's title bar still splits only groups of other floating windows.
  - *Wayland.* Where `outer_rect` is `None`, desktop positions are unknown, so drops across
    windows cannot be resolved. A drag works inside its own window, and a release outside it
    floats a new window that the compositor places. Moving a window hands the move to the
    compositor, which never docks it.

On the web and wherever egui embeds viewports, nothing changes: floating windows stay in the
page at their in-window rect, `native` and the monitor fields are kept but unused, and
`forwarded_events` stays empty. Without the feature, the fields exist and are never set by the
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
and report no positions (`no_position`, as on Wayland).

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
