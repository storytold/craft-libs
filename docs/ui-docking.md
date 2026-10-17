# Panel docking

`craft_ui::docking` lets an application move the same panel between docked tabs, side splits and
floating groups. The shared layer owns placement and interaction; the application owns each
panel's contents, commands, document state and persistence policy.

## Model and transactions

`Layout<P>` contains an optional root `Node<P>` and floating tab groups. A node can be:

- `Tabs`: ordered panel identities with one active index.
- `Split`: horizontal or vertical children, with a ratio or fixed first/second size.
- `Stack`: accordion entries with individual open state and optional content height.

Choose a stable, nonlocalized panel ID type implementing `Clone + Eq + Hash`. Rendering also
requires `Debug + Send + Sync + 'static`. A panel identity occurs at most once in the layout.
Keep transient document/sequence tabs separate when they represent documents inside one panel.

```rust
use craft_ui::docking::{Action, Floating, Layout, Node, Placement};

let mut layout = Layout {
    root: Some(Node::Tabs { panels: vec!["layers", "properties"], active: 0 }),
    floating: vec![Floating {
        panels: vec!["brushes"], active: 0, rect: [40.0, 80.0, 240.0, 300.0],
    }],
};
layout.apply(Action::Move {
    panel: "brushes", anchor: "layers",
    placement: Placement::Tab { before: Some("properties") },
})?;
# Ok::<(), craft_ui::docking::DockError>(())
```

Every transaction validates the original layout before cloning or recursive mutation. It checks
the resulting layout before replacing the original. Missing or self targets, duplicate IDs,
invalid active indices, excessive depth/count and invalid geometry return `DockError`; they do
not remove the source panel. Tab removals retain the selected panel by identity when possible.
Empty source groups collapse. Moving within an accordion preserves its entry state and height;
unaffected split sizing and accordion entries remain unchanged.

`OpenAt` adds a currently absent registered panel at an explicit tab or side destination. `Open`
is the simpler group-reveal operation. Registry checks remain app-owned: the shared model cannot
know whether an ID names an actual panel or whether its content is supported by the application.

Use `apply_with_permissions` to prevent moving/closing a document canvas or merging a panel into
it. `Permissions::PROTECTED` disallows these operations while allowing side splits around it.
An app that requires a permanently docked panel also checks that policy before accepting a
transaction; an empty shared root is valid when all panels float.

## Rendering and interaction

Existing renderers can convert their layout into the shared model for transactions and retain
their own panel chrome. `DropTarget` describes visible body/tab rectangles and whether a group
accepts tabs or splits. Supply targets back-to-front, including floating-window layer order.
`resolve_drop` returns a tab insertion or side/center placement and its preview rectangle.
The frontmost containing group blocks targets underneath even when it rejects a drop.

The optional `DockArea` renderer handles tab fitting and overflow, drag/drop previews,
Escape/focus-loss cancellation, split/accordion dividers and floating-group resize grips. It renders a
snapshot and returns `DockOutput.actions`; apply them through the application's command path.
With `serde` enabled, `Action` also serializes so pointer, keyboard and control-channel requests
can share that command path, including splitter changes, tab reorder and floating bounds.
Panel body callbacks receive stable global egui IDs, so moving a panel does not reset its widget
memory. The application should still keep durable panel/document data outside transient egui
widget memory.

`DockStyle` supplies colors, font, spacing and localized action labels. `show_with_limits` also
accepts per-panel minimum/maximum sizes. Visible geometry clamps to the available area; a narrow
viewport relaxes minima proportionally. Persisted preferred sizes are unchanged by this drawing
clamp. A divider gesture produces an explicit resize action.

Use `begin_drag` with the same area ID to carry an existing tab/header gesture into `DockArea`
when an app switches from its default arrangement to a customizable workspace.

Floating groups are egui areas in one viewport. Native child windows, cross-viewport drag
transport, OS window ownership and multi-monitor coordinate conversion remain the host's
responsibility. The model does not instantiate or reparent native windows.

## Persistence and visibility

Enable `craft-ui`'s optional `serde` feature to serialize the shared model, or retain an existing
schema through an adapter. The serialization feature does not choose a filesystem location or
write preferences. Applications validate panel IDs, payload size and schema version before
committing loaded state. Bound recursive deserialization/conversion in the app as well as using
`Layout::validate`; the validator cannot retroactively protect an unbounded parser.

To hide a panel without forgetting its placement, capture `Layout::location`, then close it.
Keep the resulting `Location` with the app's visibility state. `restore` uses stable neighbor
IDs, restores captured fixed sizing/floating bounds and recreates a previously closed accordion
group. If a neighbor was removed, it returns an unchanged layout and `MissingTarget`; choose an
explicit fallback such as a default side or remembered floating rectangle. Restoring multiple
panels closed from the same group in reverse close order preserves neighbor relationships.

## Acceptance

The shared tests cover lossless moves, tab reorder, stale targets, atomic errors, protected
canvases, accordion state, fixed sizing, hidden-panel restoration, bounded malformed layouts,
and a deterministic sequence of 800 panel moves. Pointer tests exercise dragging a floating
panel into the main tab strip and canceling a drag.

`tests/docking_workflows.rs` renders a document and populated Layers/Properties/Brushes panels,
then uses actual pointer events to merge, detach and redock Brushes. It checks retained values
and widget identity at two widths, two display scales and both egui themes. To retain captures:

```sh
CRAFT_UI_DOCK_VISUAL_DIR=/absolute/output/path cargo test -p craft-ui --test docking_workflows --all-features
```

These are offscreen egui checks. An adopting application still needs command, content, workspace
reload, theme and input tests using its own renderer and persistence adapter.

## Application chrome and activation

`DockArea::show_customized` accepts one mutable `DockContent<P>` adapter, with body,
panel-menu, group-header and label methods. Apps queue commands in these callbacks and
apply them after rendering the readonly snapshot. `group_header_width` reserves a bounded
trailing portion of the existing tab strip; its child UI is clipped to that region.
Collapsed accordion entries still have panel menus; collapsing tab groups is host policy.

`label` returns `WidgetText`, including an application-shaped galley, used for tab width,
painting and menu labels. The separate logical label callback supplies tab accessibility
text. `action_label` independently shapes the localized stock Float/Move/Close menu
text without receiving a panel identity. Its default returns unchanged logical `WidgetText`;
custom visual text never replaces the action's logical accessibility label. Panel targets
and overflow entries continue to use `label`. Original `show` and `show_with_limits` call
sites remain supported.

Apply `Action::Activate` in the host, then use `DockArea::raise_panel(panel)` on the next
frame to raise the owning in-viewport floating layer and focus the panel tab. This request
does not mutate the serialized layout or persist z-order. Floating area identity still
derives from the first panel; native-window identity remains host-owned.

Floating groups have a distinct 12-point grip in the existing tab strip. Moving it emits
`MoveFloating` for the whole group; dragging an individual tab detaches just that panel.
`DockOutput::group_drag` and `group_drop` expose the group identity and release position
for app-owned atomic group docking policies. Escape/lost focus restores the group's
starting rectangle. No generic group docking or app command registry is inferred.

Hosts supply overlay rectangles via `blocked_rects` before showing the surface. These
block drop previews, release actions and float fallback at the pointer, including overlays
appearing mid-drag. Hosts must provide current overlay geometry or cancel docking when
modal state prevents interaction; later-drawn overlays cannot be discovered retroactively.

`DockArea::chrome_has_focus(ctx, layout)` classifies focus on rendered dock tabs,
collapse buttons, overflow, split/stack dividers, floating resize corners and group
grips. It excludes panel content and custom header/menu widgets and rejects invalid
layouts. Hosts can permit global commands from chrome while reserving Enter, Space,
Tab, Escape and arrow keys for activation, traversal, cancellation and resizing.
Content widgets retain their usual keyboard ownership. The renderer owns the IDs;
hosts do not need to reconstruct them. Configure the querying `DockArea` with the same
`surface(...)` value used for rendering; split-divider focus belongs to that surface,
while tab, accordion and floating-control identities retain their shared session IDs.

## Host-resolved drops and measurement

`DockContent::resolve_drag` optionally owns a tab or floating-group gesture. Its `DropRequest` supplies visible targets, pointer/release state, viewport bounds and the default floating rectangle. Returning `Some(DropFeedback)` consumes the gesture, including explicit no-ops; the host stages/commits its validated transaction on release, and the shared renderer draws only the supplied preview and insertion marker. Returning `None` retains the default resolver.

`measure_layout` returns full group rectangles using the same split constraints, headers, protected-canvas rules and floating clamps as rendering. `DockContent::group_rect` reports actual rendered bounds for host inspection and regression checks. Tab proposals expose `noop`; same-group tab-strip destinations retain a bounded insertion marker, including unchanged slots. Markers stay inside the visible strip and group bounds; unchanged releases emit no move/float.

Floating groups move from their grip, empty header space or top border. Individual tabs retain their own drag behavior. Left, right and bottom edges and both bottom corners resize the group, respecting panel limits and viewport bounds; the top border remains a move target. Workspace permissions apply to movement and resizing.

## Transient UI dragging across host surfaces

Hosts can opt into `DockContent::live_drag`: the renderer leaves source bodies blank and
renders the moving content once, disabled, above the workspace. `drag_members` identifies
whole-group gestures started by custom headers. `floating_size` supplies independent
preferred dimensions for both drag tear-off and the shared float menu. `drop_targets`
publishes measured targets for applications coordinating several surfaces; use
`DockArea::surface(id)` for separate divider identities while retaining the common drag ID.
Only one surface should return true from `render_drag_ui`. Cross-surface transactions must
commit after every surface renders the source snapshot, avoiding duplicate widget IDs on
different egui layers within a frame. Live floating movement is transient until host release;
cancellation does not write clamped geometry back. The floating top-border move target keeps
the default cursor, while grips and resize handles retain their usual feedback.

`DockContent::drag_opacity` controls only the transient moving UI, using the same drag request
and measured targets as destination resolution. It defaults to opaque; finite values clamp
to 0..=1 and non-finite values fall back to opaque. Insertion and destination feedback retain
their own opacity. The moving UI remains disabled at every opacity.

`DockContent::drag_opacity_duration` optionally smooths moving-UI opacity changes over the
returned duration in seconds (default zero). The transition starts at the current opacity,
requests repaint while active, and is cleared on drag completion or cancellation. Destination
feedback remains independent of this animation.

## Display-only docking candidates

`DockContent::preview_layout` can provide a validated temporary layout before bodies render.
It receives the pending drag request against the authoritative surface's cached targets, or
None when no gesture is active. A host coordinating surfaces should prepare one transaction
and return its corresponding projection for each surface. The saved model remains the layout
passed to DockArea; preview controls are disabled and render-time structural actions are discarded.
Targets are frozen during live dragging, so displaying a split cannot change the target beneath
the pointer. `preview_targets` reports measured display targets separately, allowing insertion
feedback to follow the visible strip via `tab_insertion_marker` without changing drop resolution.
`paint_preview` paints additional host feedback after the moving UI, independently of its opacity.
A host must discard candidates on cancellation and commit only through resolve_drag on release.

`preview_geometry_duration` opts into smooth, display-only group bounds (seconds, default zero).
Bounds are measured with `measure_layout`, keyed by panel identity and interpolated with
smoothstep easing. Retargeting starts from the current interpolation without resetting an
unchanged target each frame. Joined tabs share the receiver's transition; new reserved slots
remain at their measured destination. Return and post-release transitions finish independently
of the gesture, with controls disabled and authoritative targets retained until settling.
Normal resize actions remain immediate. `preview_geometry` reports animated group bounds and
their destination for attached feedback; `preview_targets` reports the animated tab strips.
Hosts may seed previously external content bounds through `preview_geometry_origin` and
invalidate history on workspace replacement through `preview_geometry_generation`.
`geometry_transition_active` lets a host retain its temporary shell while returning to its
original layout. Animation state is transient and requests repaint only while bounds move.

`DockContent::preview_tab_strip` optionally returns an authoritative `DropTarget` snapshot
for a receiving group. Its anchor must remain in the receiver, independently of the dragged
members. The renderer keeps the snapshot's header position, width, original tab order and
visible/overflow slots while rendering the candidate body at its animated bounds. Headers
paint once in a disabled foreground overlay with an opaque theme background; moving content
and destination feedback retain their own layers. Hosts resolve strip hits against the same
authoritative snapshot and should give these visible slots priority over coincident edges.
Incoming tabs appear only after commit. The header settles to the committed group over the
geometry transition duration; cancellation preserves the original strip while bodies return.
History is surface-local, generation-scoped and transient. Consumers returning None retain
their existing header rendering. `geometry_transition_active` includes pending header settling
so hosts can retain their temporary presentation shell until the complete transition finishes.

Floating areas paint their complete background beneath the tab strip and disable automatic Area
appearance fading. Transparent theme tab-strip colors therefore reveal the panel background,
not the canvas. The host's intentional moving-UI opacity continues to apply during dragging.

## Shared-divider insertion

`measure_layout_with_boundaries` returns group rectangles and actual split dividers from the
same constrained traversal used by `measure_layout`. Each SplitBoundary includes orientation,
parent/child bounds and stable panel members on each side; floating windows are not seams.

`insert_between` accepts a BetweenRequest with moving members, selected member and ordered
before/after neighbor identities. It removes sources in a staged clone, finds the surviving
boundary and inserts a new group between its children. New space starts at one third after
the two dividers; neighbors retain their measured proportion with min/max constraints. Invalid
identities, permissions, geometry, insufficient space and final depth limits leave the original
layout untouched. Hosts choose the hit band and apply the same operation for commands and
pointer gestures. PhotoCraft uses an 8-point band and preserves both receiving headers.

Host-resolved and default tab-drop feedback explicitly raise the foreground destination layer
when painting, so fully opaque insertion lines stay above stationary header and moving UI layers.

Pointer resizing releases divider/corner focus when its drag stops, so the focused accent
highlight does not remain after release and pointer exit. Ordinary hover and drag feedback
remain, and keyboard focus through traversal still supports arrow-key resizing.


## Docked headers and anchored resizing

Hosts opt into docked empty-header and top-4-point whole-group dragging with
`DockContent::drag_docked_headers`. Group gestures retain every member and the selected
panel, use host-owned atomic drop resolution, and keep the normal top-border pointer.
Header menu and overflow controls keep their own hit areas. The default remains disabled.

Source-center drops, including singleton and floating groups, resolve as explicit no-ops.
PhotoCraft highlights the source content area while retaining a clear tab strip and its
fully opaque insertion line; real reorder slots and unchanged slots share this feedback.

`AnchoredResize` captures the displayed rectangle, fixed opposite edges and pointer grab
offset. Pointer movement sets the dragged edges absolutely, bounded by panel limits and
the viewport before constructing the rectangle. Overshoot never translates the opposite
edge or creates immediate growth on reversal outside the valid range. Impossible minimums
suspend resizing. Pointer anchors clear on release, focus loss, Escape and host cancellation;
keyboard resizing continues in fixed steps. Hosts can reuse the helper for document windows.
