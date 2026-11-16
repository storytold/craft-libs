# Task panes

`craft_ui::panes` draws task panes around an app's own document: Navigation, Styles, Comments,
Format Shape and the like. Each pane is an egui side or bottom panel, or a floating frame. The
document stays the app's `CentralPanel`.

## Panes or docking

| Use | When |
|---|---|
| `panes` | Office-style apps (WordCraft, DeckCraft, GridCraft). A few panes at the sides of a document the app draws itself; the app's commands own which panes are open. |
| `docking` | Creative apps whose whole window is panels: tab groups, splits, stacked groups, workspaces, a protected canvas panel. See [ui-docking.md](ui-docking.md). |
| plain egui panels | Fixed view parts: slide thumbnails, notes, formula bar, sheet tabs, status bar. |
| `egui::Window` | Free tool windows that never dock (Read Aloud, Power Query). |

`panes` needs no docking types; it uses only the docking chrome (`ChromeStyle`,
`FloatingFrame`, `TitleBar`, `live_move`).

## API

```rust
use craft_ui::panes::{Edge, PaneContent, PaneHost, PaneRequest, PaneState, Place, SidePolicy};

impl PaneContent<Pane> for MyPanes<'_> {
    fn title(&self, pane: &Pane) -> String { tr(pane.title()) }
    fn body(&mut self, ui: &mut egui::Ui, pane: &Pane) { /* the pane's contents */ }
    fn default_state(&self, pane: &Pane) -> PaneState { PaneState::docked(Edge::End, 250.0) }
    // Optional: size_range (200..=520), movable, closable, menu (app items in the ▾ menu),
    // title_bar (false: a docked pane draws no title bar; floating panes always draw one).
}

// Every frame, after the top and bottom bars and before the CentralPanel:
let out = PaneHost::new(egui::Id::unique("task-panes"))
    .rtl(ui_is_rtl)
    .policy(SidePolicy::Columns)
    .show(ui, &open_panes, |pane| prefs.pane_state(pane), &mut MyPanes { app });
for request in out.requests {
    match request {
        PaneRequest::Close(pane) => app.run(pane.close_command()),
        PaneRequest::Place(pane, place) => prefs.set_place(pane, place),
        PaneRequest::Resize(pane, size) => prefs.set_size(pane, size),
        PaneRequest::FloatRect(pane, rect) => prefs.set_float_rect(pane, rect),
        // Move `pane` to directly before `before` in the open list (`None`: to its end).
        PaneRequest::Before(pane, before) => app.move_pane_before(pane, before),
        PaneRequest::Focus(_) => {}
    }
}
egui::CentralPanel::default().show(ui, |ui| {
    let response = document(ui);
    if out.to_document {
        // F6 or Shift+F6 left the panes: the document takes keyboard focus.
        response.request_focus();
    }
});
```

| Piece | Behavior |
|---|---|
| `Edge::{Start, End, Bottom}` | Logical edges. `Start` is left, or right with `.rtl(true)`. |
| `SidePolicy::Columns` | Every docked pane shows; panes on one edge sit side by side in `open` order, the first outermost. A drop that picks a position on an edge sends `Before(pane, next)` after its `Place`, so the app's `open` order puts the pane there. |
| `SidePolicy::OneSlot` | One closable pane per edge: the last one in `open`. Non-closable context panes (PivotTable Fields) show beside the slot. A closable pane dropped on an edge takes the slot: `Before` moves it after the slot's pane. |
| `.max_fraction(f)` | One docked pane takes at most `f` of the host's width (height for bottom panes). |
| Chrome | Title, ▾ menu (app items, Move to Start, Move to End, Move to Bottom for bottom panes, Float, Close), ×, a resize edge with pointer and arrow keys. The edge takes presses on both sides of its line, over the document's own widgets too, and stays under floating windows. The title bar mirrors with `.rtl(true)`. Its accessible name is `PaneLabels::move_pane` with `{title}` replaced by the pane's title ("Move Styles pane"). |
| Size | A docked pane reserves exactly its size and clips its body, so a body wider than the pane never pushes into the document. |
| Drag out | A docked pane's title dragged more than `DOCK_SNAP` (32 pt) out of the pane floats it under the pointer when dropped, unless it is dropped on a dock target: within 32 pt of the host's left or right edge it docks there, outermost; over the outer third of another side pane (toward the window edge) it docks directly outside that pane, over its inner third directly inside it. The middle third is no target. While the pointer is on a target, a preview shows the rect the pane will take once the app applies the requests. A drop where the pane already is sends nothing. |
| Floating | `FloatingFrame` with its resize handles; the title bar moves it. Escape or window focus loss during a move puts it back. A drop on a dock target docks it, with the same targets and preview as a drag out. Rects are clamped to the host area. |
| Focus | `out.focused` is the pane holding keyboard focus or last clicked; a click or focus in the document clears it. F6 and Shift+F6 cycle through the shown panes and then report `out.to_document`: give focus to the document. `.f6(false)` leaves F6 to the app, which calls `panes::focus_next(ctx, id, reverse)` instead. |

State:

- The host never stores which panes are open and never writes files.
- `states` returns the app's saved `PaneState` (place, size, float rect; serde with the `serde`
  feature). Return `None` to let the host keep the state in egui memory for the session, as
  egui panels keep widths today.
- Apply `Resize` and `FloatRect` as they come: they arrive every frame of a gesture, and an
  Escape sends the start rect back.
- `panes::float_rect` brings a saved rect back inside the window, whatever the file says.

## Rail

`panes::Rail` is a vertical strip of icons, always shown at one edge. Each icon stands for a
task pane, and one pane at a time is open beside the strip. A click on an icon opens its pane in
place of the open one; a click on the open pane's icon closes it. The rail stores nothing: the
app owns which pane is open, as with `PaneHost`.

```rust
use craft_ui::panes::{Edge, PaneHost, Rail, RailItem, RailRequest, SidePolicy};

let items = [
    RailItem::new(Pane::Comments, icons.source("comments"), tr("Comments")).marker(has_comments),
    RailItem::new(Pane::Bookmarks, icons.source("bookmark"), tr("Bookmarks")).enabled(has_outline),
];
// After the top and bottom bars, before PaneHost: the rail is the outermost panel on its edge.
let rail = Rail::new(egui::Id::unique("rail")).edge(Edge::End).rtl(ui_is_rtl).show(ui, &items, app.pane.as_ref(), |ui| {
    page_navigation(ui); // the foot: below the items, always visible; may draw nothing
});
match rail.request {
    Some(RailRequest::Open(pane)) => app.pane = Some(pane),
    Some(RailRequest::Close(_)) => app.pane = None,
    _ => {}
}
// The open pane, between the document and the rail. Its PaneContent returns `movable = false`
// and a `default_state` docked at the rail's edge.
// Apps whose panes draw their own headers return false from `title_bar`.
let open: Vec<Pane> = app.pane.into_iter().collect();
let out = PaneHost::new(egui::Id::unique("task-panes")).rtl(ui_is_rtl).policy(SidePolicy::OneSlot).show(ui, &open, |_| None, &mut MyPanes { app });
egui::CentralPanel::default().show(ui, |ui| document(ui));
```

| Piece | Behavior |
|---|---|
| `RailItem` | `pane`, `icon` (an `egui::ImageSource`, like docking's `DockContent::icon`), `label` (tooltip and accessible name; translate it in the app), `enabled`, `marker`, `marker_label` (the accessible description while the marker shows, such as "Has comments"). `RailItem::new` makes an enabled item without a marker. |
| `.edge(e)` | `End` (default) or `Start`. `Start` is left, or right with `.rtl(true)`. The rail is vertical, so `Bottom` draws at `End`. |
| `.width(w)`, `.button_size(s)` | Rail width (48) and item size (34; the icon is 16 smaller). Non-finite values fall back to the defaults. |
| `.frame(f)` | Default: the panel fill, 8 pt above and below. The panel's separator line draws the inner edge. Item colors come from `ChromeStyle`, like the pane title bars. |
| Panel | Exact width, not resizable. Shown before `PaneHost::show`, it is outermost, so a docked pane on the same edge sits between it and the document. |
| Items | Top-down in a vertical scroll area. The foot is a bottom sub-panel, so in a short window the items scroll and never run into it. A pane listed twice is drawn once. An empty list draws an empty rail. |
| States | The open pane's item draws selected. `marker` draws a dot while the pane is not open. A disabled item draws disabled, takes no focus and sends nothing. An item disabled while focused hands focus to the nearest enabled item, the one below first. |
| `RailOutput` | `#[non_exhaustive]`. `request`: `Open(p)` or `Close(p)` (`#[non_exhaustive]`), at most one per frame. `focused`: the item holding keyboard focus. `rect`: where the rail was drawn. |
| Accessibility | Each item is a toggle button (accesskit `Button` with a toggled state) named by its `label`. |
| Keyboard | Items take Tab focus. Enter or Space clicks the focused item. Up and Down move focus between enabled items and stop at the ends. Escape is the app's: the rail neither closes the pane nor moves focus on it. |

F6 stays with the app. To include the rail in an F6 cycle, give `PaneHost` `.f6(false)` and,
when the cycle reaches the rail, focus the open pane's item if it is enabled, else the first
enabled one:

```rust
let enabled = || items.iter().filter(|item| item.enabled);
let target = enabled().find(|item| Some(&item.pane) == app.pane.as_ref()).or_else(|| enabled().next());
if let Some(item) = target {
    ctx.memory_mut(|m| m.request_focus(Rail::new(egui::Id::unique("rail")).item_id(&item.pane)));
}
```

While `rail.focused` is `Some`, focus is in the rail: the next F6 moves on, for example to the
panes with `panes::focus_next`.

## Adoption

### WordCraft (`Columns`)

- Add a `Pane` enum for the nine panes and implement `PaneContent` by calling the existing
  bodies; drop their `header()` calls, since the host draws the title and ×.
- Build `open` each frame from the `ViewState` flags, in today's order: SmartArt Text Pane (only
  while SmartArt is selected), Navigation and Clipboard at `Start`; Styles, Style Inspector,
  Accessibility, Alt Text, Restrict Editing and Comments at `End`. Default widths as today.
- `Close(p)` runs the pane's existing command (`view.navigationPane {value: false}`, …). Command
  ids and `nav_pane` in `ui.json` stay as they are.
- Styles takes #2584's behavior through the host: map `StylesPaneDock::{Left, Right, Floating}`
  to `Place` (`Left` is `Docked(Start)` in left-to-right interfaces), run `view.stylesPaneDock`
  on `Place`, keep the float rect in UI prefs. #2584's own drag, snap and `float_rect` code goes.
- Other panes return `None` from `states`; their widths stay session-only, as now.
- Pane title bars are focusable. Check that Tab in the canvas still stays in the document, so
  panes stay off its keyboard focus. Use `.f6(false)` if F6 is bound elsewhere.

### DeckCraft (`OneSlot`)

- `open` is `ui.pane` (one entry) at `End`; `states` returns `PaneState::docked(Edge::End,
  pane_width)`; `Resize` writes `pane_width`; `Close` clears `ui.pane` as the × does today.
  `size_range` is 240..=520. The `ui.json` fields stay.
- Format, Animation and Selection return `movable = true`; the rest `false`. Saving their place
  needs new optional `ui.json` fields (`#[serde(default)]`).
- Thumbnails, outline, notes and the #835 canvas shell stay plain panels.
- F6 from the host closes the ui-parity F6 gap.

### GridCraft (`OneSlot`, `.max_fraction(0.5)`)

- `open` is the task slot pane (`grid.pane`, or "comments" while `session.comments_pane`) plus
  PivotTable Fields while the active cell is in a pivot. PivotTable Fields returns
  `closable = false` and `movable = false`, so it stays beside the slot.
- `Close("comments")` runs `review.showComments {on: false}`; agents still drive it through the
  engine.
- No pane prefs today: return `None` from `states`.
- Code Editor (#1193) and Queries & Connections (#1203) become slot panes. The formula bar,
  sheet tabs and status bar stay plain panels.
