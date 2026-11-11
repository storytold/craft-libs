# Glossary

One row per name in the author table (Overview) and per `pub` item a component author touches (`contract`, `style`, `primitives`, the `Cx` traits and their re-exports); a doc test checks them against it. "Lives in" is the defining module, from the layer map; a re-exported item has one row, under its defining module. Definitions are in the "Defined in" sections.

| Term | What it is | Lives in | Defined in |
|---|---|---|---|
| `Access` | The builder a component's `access` hook fills | `craft_ui::runtime::access` | 12.1 |
| `Accessible` | The `Cx` trait for `announce` and `field_label` | `craft_ui::contract` | 1.5, 12 |
| `AccessRequest` | An AccessKit action that arrives as `Intent::Access` | craft-behavior | 9.2 |
| Active part | The part of a focused component that has focus, held by the runtime focus service | `craft_ui::runtime::focus` | 1.7 |
| `Animate` | The motion trait on every context | `craft_ui::runtime::motion` | 13.1 |
| App layer | Layer 8, in app repos only | each app repo | Overview |
| App-modal | The rule for `Rank::Modal` layers | `craft_ui::runtime` (layer stack) | 15.5 |
| `AppSizes`, `sizes.json` | An app's overrides of library base sizes | craft-theme | 4.6 |
| `AppValues` | The private set of app values a root, a scope or a component's `content_env` sets with `set::<T>(v)` | `craft_ui::runtime::env` | 2.2 |
| Arena | The set of shapes one press is resolved among | `craft_ui::primitives` (the registration store and the sensing core; the resolve rule is a craft-behavior pure function; `ArenaId` in `craft_ui::runtime::env`) | 8.3 |
| Arena member | A component that joins the innermost `Env.arena` because it is neither an arena root nor a text or editor leaf | `craft_ui::contract` | 8.3 |
| Arena root | A container with `ContainerProps.arena`, or any collection | `craft_ui::contract` | 8.3 |
| `ArenaId` | The id of an arena, carried in `Env.arena` | `craft_ui::runtime::env` | 8.3 |
| `Backdrop` | A modal's scrim (`None`, `Scrim`, `ScrimWithHole`), carried by `Rank::Modal` | `craft_ui::runtime` (layer stack) | 15.2 |
| `Bind<T>` | The one value binding | `craft_ui::contract` | 10.2 |
| `BindMut<T>` | A reborrowed `Bind<T>` that `bind()`, `open()`, `scroll()` and `selection()` return | `craft_ui::contract` | 10.2 |
| Body | The caller's closure that a container shows, its one caller region | — | 16.3 |
| `BarItems`, `BarItem` | A leaf inside a `Bar`'s body that shows command items in a strip and overflows them by `fit` into a menu of command rows | the bar family | 20.3 |
| `BodyCx` | The non-generic context a body closure, header, footer or empty hook and popup content receive, as `&mut` | `craft_ui::contract` | 1.5 |
| `BodyLayout`, `RowAlign` | How the body step lays out a body (`ContainerProps.layout`) | `craft_ui::layout` | 6.3, 16.2 |
| `CancelReason` | Why a gesture or press was cancelled (`Refused`, `Dismissed`, `HostChanged`, `UndoDuringGesture`, ...) | craft-behavior | 10.2 |
| `cancel_all` | `runtime::edit::cancel_all(ctx, reason)`: cancels every running gesture; the app's undo and redo call it first | `craft_ui::runtime::edit` | 10.3 |
| Canvas control | A leaf a `Canvas` body shows, declared with `#[component(canvas_control)]` | the component's family | 8.3, 20.6 |
| Command form | A canvas tool's keyboard path | the tool's crate | 8.3 |
| `CanvasTransform` | The transform a canvas body is shown through, from `ContainerProps.transform` | craft-behavior | 8.3 |
| `CellSelection` | The sheet's selection model | craft-behavior | 17.7 |
| `Chord` | A key with modifiers, the one shortcut type, shown with `ChordText` | craft-keymap | 11.1 |
| `ClipboardHandle`, `ClipItem` | The handle `cx.clipboard()` returns over `craft_platform::clipboard` | `craft_ui::runtime::commands` | 11.3 |
| `Codec` library codecs | `PlainText` and `NumberCodec` beside `Codec` and `quantity::Reader` (craft-behavior); `HexCodec`, `DateCodec` and the text fields' other codecs in craft-ui | craft-behavior, craft-ui | 18.3 |
| Collection | A component that also implements `Collection` | `craft_ui::contract` | 1.3, 17 |
| `CollectionEvent`, `ColumnEvent` | The events a collection reports, and its column events | `craft_ui::contract` | 17.1, 17.6 |
| `CollectionProps` | A collection's plain data | `craft_ui::contract` | 17.1 |
| `CollectionResponse` | A collection's output | `craft_ui::contract` | 17.6 |
| Column, `ColumnSpec` | A `ColumnSpec` in `CollectionProps.columns` | `craft_ui::contract` | 17.1 |
| `ColumnGroup` | Rows that are not one `Grid` (`Field` rows) sharing the widest label per column key, by the same function as a `Grid` `Content` column | `craft_ui::layout` | 6.3 |
| `ColumnWidth`, `ColumnKey` | A column's width rule | craft-behavior | 17.1, 6.3 |
| Command scope | A `CommandScope`, live while a container declaring it is live | craft-keymap | 11.3 |
| `CommandRef`, `CommandKind`, `ModalRule` | A command's id, its kind (`Command`, `Toggle`, `Radio`, `Tool`, `Momentary`) and whether it fires while a dialog is open | craft-keymap | 11.2 |
| `CommandGate` | The one gate that can disable commands (dialog open, workspace, recorder, pending operation) | `craft_ui::runtime::commands` | 11.3 |
| `Commands` | The context trait on `Cx`, `BodyCx` and `ItemCx` for commands and the clipboard | `craft_ui::runtime::commands` | 11.3 |
| `CommandSet` | A typed set of commands with one scope, written by `#[derive(CommandSet)]` and read with `take_command::<S>()` | craft-keymap (trait), `craft_ui::runtime::commands` (board) | 11.2, 11.3 |
| `CommandState`, `CommandInput` | A command's state this frame, and the app-scope states in `FrameInput.commands` | `craft_ui::runtime::commands` | 11.3 |
| `Common` | The props every component has, with no public fields | `craft_ui::contract` | 3.1 |
| Component | Any type implementing the core trait | `craft_ui::contract` | 1.2 |
| `ComponentStyle` | The trait every `type Style` implements, usually with `#[derive(ComponentStyle)]` | `craft_ui::style` | 4.3 |
| `ContainerProps` | The plain-data props of containers and collections | `craft_ui::contract` | 16.2 |
| Content painter | A `ContentPainter` type that paints document content with a raw painter | `craft_ui::contract` (trait) | 5.4 |
| Container | A component that also implements `Container` | — | 1.3, 16 |
| `ContentPainterCx` | What `PaintUi::content_painter(cx)` returns | `craft_ui::primitives` | 5.3, 5.4 |
| Content step | Step 14 | `craft_ui::contract` | 1.6 |
| `content_env` | The `Component` hook, run once at step 13, that sets app values for content the component does not build, its body or items | `Component` method | 2.2 |
| `ControlMetrics` | Height, icon, horizontal padding and gap per `ControlSize`, read from a craft-theme `BySize` table to build `Metrics` | craft-behavior | 4.3 |
| `ControlSize` | The control size scale `BySize` tables are read by | craft-theme | Overview, 2.1 |
| `Corners` | Which corners of a surface are rounded | `craft_ui::style` | 4.3 |
| `runtime::cursor` | The cursor service: it picks the winning cursor request in the end hook and records it as data | `craft_ui::runtime` | 8.5 |
| `Cursor`, `PaintedCursor`, `CursorBadge`, `CursorPriority` | A cursor request (system or painted), a painted cursor's shape, its badge, and the request's priority | craft-behavior | 8.5 |
| `CustomVariant` | A `const` variant built for one component's `StyleKey` with a crate-prefixed id and a look function over semantic tokens | craft-theme | 4.4 |
| `CxCore`, `Cx` | The per-show interaction context | `craft_ui::contract` | 1.5 |
| `DataColor` | A colour from data (a swatch, a label dot), the one colour `Draw` accepts besides `StyleColor` | `craft_ui::style` | 4.3 |
| Decision record | The record each step writes about why it decided what it did, read by `runtime::inspect` and shown by `DebugInspector` | `craft_ui::runtime::inspect` | 1.7 |
| Decorator | A container whose body shows the decorated component (a badge, an attention dot) | component families | 16.1 |
| Dense item | An ordinary item with many parts (a sheet row, a timeline track) | — | 17.2 |
| `Dismiss` | An egui layer's dismissal rules, in `Overlay.dismiss` | `craft_ui::contract` | 15.4 |
| `DocKey` | A stable, crate-prefixed semantic key for presence, automation, undo names and programmatic focus | craft-behavior (re-exported by craft-ui) | 3.1 |
| `Direction`, `Scale` | The layout direction (left-to-right or right-to-left) and the interface scale, inherited through `Env` | craft-i18n, `craft_ui::runtime::env` | 2.1 |
| `DocText`, `DocRun`, `FontRef` | Document text a content painter lays out with `ContentPainterCx::layout` | `craft_ui::primitives`, `craft_ui::fonts` | 5.4 |
| `DragPreview` | What the overlay host paints under a drag | `craft_ui::runtime` (drag session) | 8.4 |
| `Draw` | The content-paint API `PaintUi::draw` gives | `craft_ui::primitives` | 5.3 |
| `DropOperation` | A drop's operation | craft-behavior | 8.4 |
| `DropTarget<P>`, `DropMark` | A drop target registered with `cx.drop_target(&target)` | `craft_ui::runtime` (drag session) | 8.4 |
| `Edit` | The value trait on `Cx` | `craft_ui::contract` | 10.2 |
| `EditHistory<T>` | A local undo history for dialogs, tool sessions and the keymap editor, fed by `EditEvent`s | craft-behavior | 10.5 |
| `EditEvent`, `EditResponse` | The value events a value component emits, and the output that carries one per pass | craft-behavior (`EditEvent`), `craft_ui::contract` (`EditResponse`) | 10.2 |
| `EditCommand` | An edit command delivered as `Intent::Edit` or `EditorInput::Edit` | craft-behavior | 9.2, 11.3 |
| `EditKind` | The kind of an edit event a story recorded | `craft_ui::testing` | 22.4 |
| `EditorInput`, `EditorState` | The input stream an own-caret editor reads, and the state it reports (with `can_undo` and `can_redo`) | craft-behavior (`EditorInput`), `craft_ui::primitives` (`EditorState`) | 18.4 |
| `EditorLine`, `EditorPurpose` | One visible line an editor reports, and the editor part's purpose | `craft_ui::primitives` | 18.4 |
| `EditorRun`, `LayoutRun` | One bidi run of a line | `craft_ui::primitives` | 18.4 |
| `EditPolicy` | When a write-through source is written | craft-behavior | 10.2 |
| egui layer | An egui `LayerId` | egui | 15.2 |
| egui options the runtime owns | The table of egui options and calls only the runtime sets | `craft_ui::runtime::egui` | 1.8 |
| `Emphasis` | The emphasis axis of a variant | craft-theme | 4.4 |
| `Env` | The inherited context of a `Ui` | `craft_ui::runtime::env` | 2.1 |
| Env scope | A stretch of code with changed inherited values, from `with_env(ui, set, body)` and `Overrides` | `craft_ui::runtime::env` | 2.3 |
| `EnvKey` | The key of everything a style depends on besides its own key | `craft_ui::style` | 4.5 |
| `EnvValue`, `StyleValue` | An app-defined value inherited down the tree like the library's own | `craft_ui::runtime::env` | 2.2 |
| `EnvView` | The view of `Env` that component code reads | `craft_ui::runtime::env` | 2.1 |
| Face | An instance of a component painted by another component's `paint` as the look of one of its parts, with `PaintUi::face` | — | 5.3, 20.1 |
| Field container | `Field` | `craft_ui` form family | 20.1, 20.3 |
| `Help`, `LabelKind`, `FieldLayout` | A field's help text, its label's kind and its layout | `craft_ui` form family | 20.3 |
| `Fired`, `FireSource`, `FirePhase` | One fired command in the queue | `craft_ui::runtime::commands` | 11.3 |
| `Flags` | Value-driven and interaction flags, merged at step 12 | craft-theme | 4.1 |
| `Flow` | Start and end geometry resolved once for the layout direction | craft-behavior | 6.2 |
| `Focus` | The `Cx` trait for `intents`, `claim`, `take_claimed`, `active_part`, `is_focused`, `focus_visible` and `reveal` | `craft_ui::contract` | 1.5, 9 |
| Focus on press | Which presses move key focus | `craft_ui::runtime::focus` | 8.3 |
| Commit now | The mark the focus service sets at H7 on a focused text part with a draft when a press lands elsewhere, so the draft commits in the press pass | `craft_ui::runtime::focus` | 8.3 |
| Focus scope | A part of the UI with a `FocusPolicy` | `craft_ui::runtime::focus` | 9.4 |
| `FocusLook`, `IconLook`, `SurfaceLook`, `TextLook` | The style parts a `*Style` struct holds for the focus ring, icons, the surface and text | `craft_ui::style` | 4.3 |
| `FocusPolicy` | A focus scope's rules | `craft_ui::runtime::focus` | 9.4 |
| `FocusTarget` | What `runtime::focus::request` focuses | `craft_ui::runtime::focus` | 9.4 |
| `FrameInput`, `FrameOutput` | What an app sets before each pass and drains after it | `craft_ui::runtime::frame` | 1.8 |
| `InstallSpec` | What `install` takes: shell info, the window system, style replacements and the ghost-layer painter (`frame_layers`) | `craft_ui::runtime` | 1.8 |
| `install` | The install entry apps and the fixture call: fills `InstallSpec.frame_layers` and calls `runtime::install` | `craft_ui` | 1.8 |
| `paint_frame_layers` | Paints the ghost egui layer (drag preview, drop mark, painted cursor) once per pass from the end hook | `craft_ui::contract::overlay` | 15.1 |
| `Gap` | A body's item spacing token, `ContainerProps.body_gap` | `craft_ui::contract` | 16.2 |
| Gate | A container's `bool` view of its shown value that enables or disables its body, edited through `BodyCx::gate()` | — | 10.4 |
| `Facts<T>` | Cross-widget facts registered in one pass and read in the next | `craft_ui::runtime::memory` | 7.2 |
| `Gesture`, `GesturePhase`, `GestureId` | The pipeline-owned edit in progress, its phase this frame, and its id | craft-behavior | 8.2, 10 |
| `GridView`, `GridSection` | A sheet layout's column sections (one per frozen area, rects relative to the viewport) given to each row through `ItemCx::grid()` | craft-behavior | 17.7 |
| Group form | A container and arena root whose members, placed anywhere in its body, join its arena and register with its `GroupHandle` | `craft_ui::contract` (handle), component families | 20.3 |
| Group's pick | The member a group form selects at step 9, from `cx.activated_member()` | — | 20.3 |
| `GroupHandle<G>` | A group form's handle, put into its body's env with `content_env` | `craft_ui::contract` | 20.3 |
| `GroupKind` | The kind of a group form (radio, chip), naming its member value type | `craft_ui::contract` | 20.3 |
| Header and footer regions | Areas the body step lays out from `ContainerProps.header` and `footer`, filled by `Container::header` and `footer` | `craft_ui::contract` | 16.3 |
| `HeaderRegion`, `FooterRegion` | The header and footer regions in `ContainerProps` | `craft_ui::contract` | 16.2 |
| `HitShape`, `HitLevel`, `HitPriority` | A part's hit and paint shape, its level, and its level with a sub-level | craft-behavior | 8.3 |
| Hook steps H0 to H8 | The numbered input-hook and begin-hook steps of the frame loop, beside the end hook | `craft_ui::runtime` | 1.8 |
| Host `Ui` | The `Ui` a mount gives a container (the parent's, an egui Area's, a viewport's child root) | — | 1.6 |
| `IconId` | An SVG-set or tool icon's egui-free key, its name | craft-behavior | 5.2 |
| `IconSpec` | An icon to paint | `craft_ui::primitives` | 5.2 |
| `IdSource` | Where an id comes from | `craft_ui::contract` | 7.1 |
| `ImeSurface` | The public editor contract for components that draw their own caret | `craft_ui::primitives` | 18.4 |
| Inherited and layer-local env | The two groups of `Scoped` | `craft_ui::runtime::env` | 2.1, 2.3 |
| `Ink` | The text and icon colour that reads on a surface, resolved at step 12 | `craft_ui::style` | 5.1 |
| `Inner` | A container's show output | `craft_ui::contract` | 1.1 |
| `Input` | How input reaches a component | craft-behavior | 1.1 |
| `Insets` | Start, end, top and bottom distances, mirrored by `Flow` | craft-behavior | 6.2 |
| `Intent` | A key, AccessKit action or typed text turned into meaning | craft-behavior | 9.2 |
| `Intent::MoveItem` | Moves the lead item of a reorderable collection one place | craft-behavior | 9.2, 17.3 |
| `Intent::Open` | Opens a `Trigger::Hold` or dropdown opener | craft-behavior | 9.2, 15.5 |
| `Intents` | The owned copy of the intents routed to one id this pass, read with `cx.intents()` and its `take_*` calls | `craft_ui::runtime::keys` | 9.2 |
| `Internal` | A token only craft-ui can build, taken by the derive-written accessors | `craft_ui::contract` | 1.1 |
| `Interpolate`, `Tween`, `Presence`, `MotionSpec` | An animatable value, a running tween, an open-close presence, and a motion constant | `craft_ui::runtime` (motion service) | 13.1 |
| Item step | The collection's content step | `craft_ui::contract` | 17.3 |
| `item_layout` | The `Collection` hook that builds this frame's `ItemLayout` from the source, width and `StyleCx` | `Collection` method | 1.3, 17.3 |
| `ItemCx` | The context of the item closure, `ItemCx<'a, 'r, K>` | `craft_ui::contract` | 1.5, 17.2 |
| `ItemLayout` | A collection's geometry, logical (x from the start edge), relative to the viewport and computed in f64 inside; the item step mirrors it unless `mirrors()` is false | craft-behavior | 17.4 |
| `ItemOutput` | The trait an item's `Output` implements, converting it into `Option<ItemEvent>` | `craft_ui::contract` | 17.1 |
| `DisplayOrder` | The source trait: keys, depth, open state and text in display order | craft-behavior | 17.2 |
| `ItemSource` | The source a collection shows | craft-behavior | 17.2 |
| `SizeHint` | An item's size hint from its source | craft-behavior | 17.2 |
| `Key`, `Mods`, `KeyInput` | Craft-keymap's key types | craft-keymap | 9.2, 11.1 |
| `KeyUse` | A component's declared keys | `craft_ui::runtime::focus` | 9.3 |
| Key table, `KeyRole` | The one table per OS that maps keys and modifiers to intents and `MoveMods`, as the pure function `keys::intent(os, key, mods, role)` | craft-behavior | 9.2 |
| `craft_keymap::layout` | The keyboard-layout rule for shortcut matching and key labels, used by the dispatcher, the key table and `ChordText` | craft-keymap | 11.1 |
| `Label` | Given or library text | `craft_ui::runtime::env` | 19.2 |
| Label scope | The scope a container with a side label opens for the controls inside; a control reads the label's `FieldLabel` (id, rect, press state, text) with `cx.field_label()` | `craft_ui::runtime::access` | 12.4 |
| Label step | Body sub-step B2, run for every container with a side label and by `PaintUi::labelled` | `craft_ui::contract` | 16.3 |
| Layer | One of the architecture layers 0 to 8 | — | Overview |
| Layer stack | The runtime's per-viewport record of shown egui layers and their order | `craft_ui::runtime` | 15.2 |
| `Arrangement`, `arrange` | A logical custom placement for a body's keyed children, and the function that measures, mirrors (unless `mirrors()` is false) and shows them once | craft-behavior (`Arrangement`), `craft_ui::layout` (`arrange`) | 6.3 |
| `runtime::layout_memory` | Values measured in one pass and read in the next, with size tweens and the tolerance rule for discard passes | `craft_ui::runtime` | 6.3 |
| Leaf | A component with neither body nor items | — | 1.1 |
| Lifetime | How long a memory slot lives | `craft_ui::runtime::memory` | 7.2 |
| Lock rule | No app-supplied code runs while a runtime or context lock is held | `craft_ui::runtime` | 7.2 |
| `Looks` | The app's style replacements (`Looks::style`), passed to `install` and frozen | `craft_ui::contract` | 4.4 |
| `MAX_EXTENT` | The largest side a component may allocate, 32,768 points | `craft_ui::contract` | 1.3 |
| `MeasureCx` | What `measure` gets | `craft_ui::contract` | 6.2 |
| Memory stage | The pipeline-owned set of a component's memory slots, moved in at step 3 and back after `finish` | `craft_ui::runtime::memory` | 7.2 |
| `Message` | The text of a `Validity` verdict | `craft_ui::runtime::env` | 10.2 |
| `Meta`, `HasProps` | Traits the derive writes | `craft_ui::contract` | 1.1 |
| `Metrics` | A style's padding, minimum size, wrap width, gap and least text width | craft-behavior | 6.2 |
| `Mixed<T>` | A value that is the same across a multi-selection, or mixed | craft-behavior | 10.2 |
| `MotionMode`, `MotionKind` | The motion level (`Full`, `Reduced`, `Off`) and an animation's kind (`Fade`, `Travel`, `Loop`, `Blink`), inputs of the motion table | craft-behavior | 13 |
| `MotionPolicy` | The motion mode in effect with the motion tokens, carried in `Env.motion` | `craft_ui::runtime::env` | 13.1 |
| `MoveMods`, `Granularity` | A move's modifiers (extend the selection, move the lead only) and its unit (`Unit`, `Word`, `Line`, `Paragraph`, `Document`) | craft-behavior | 9.2 |
| `Mount` | Where a container lives | `craft_ui::contract` | 16.2 |
| `Move` and `Size` commands | The `window` command set's keyboard path for moving and resizing the focused floating container (Alt+F7 and Alt+F8 on Windows and Linux, menu only on macOS) | `WindowCommand` variants | 16.3 |
| `NavMove` | A logical navigation move (`Back`, `Forward`, `Up`, ...) | craft-behavior | 9.2 |
| `Name` | How a node is named | `craft_ui::runtime::access` | 12.1 |
| `NavOutline`, `NavShape` | The key-free outline the navigation core steps through, and the navigation shape | craft-behavior | 9.5 |
| `NodeGraph`, `Node`, `NodeKind` | The node-editor collection, its item component and the app trait for node kinds, each generic over the graph's edit type `E: 'static = ()` | `craft_ui::graph` | 20.6 |
| `NodeEvent` | A node graph's event: `Wired`, `WireEnded`, or `Edit(E)` from a kind's inline controls | `craft_ui::graph` | 20.6 |
| `Numeric` | A node's numeric value, range, step and value text, set with `a.numeric(..)` | `craft_ui::runtime::access` | 12.1 |
| One press per pass | The input hook's rule (H0) | `craft_ui::runtime::egui` (input) | 1.8, 8.3 |
| `Opener` | The component that opens a free-standing popup | `craft_ui::contract` | 15.4 |
| `Outcome` | The trait every `Output` implements | `craft_ui::contract` | 10.2 |
| `OpenEntry` | One shown egui layer in the layer stack | `craft_ui::runtime` (layer stack) | 15.2 |
| `Overlay` | Data describing an egui layer | `craft_ui::contract` | 15.4 |
| `Overrides` | The values an env scope changes | `craft_ui::runtime::env` | 2.3 |
| `OverlayFit` | How an egui layer sizes | `craft_ui::contract` | 15.4 |
| `Overlays` | The `Cx` trait with `popup(name)` | `craft_ui::contract` | 1.5, 15 |
| `OverlayUi` | What the `overlay` hook gets | `craft_ui::contract` | 15.4 |
| Owner (memory) | The component id registered at step 3 whose pipeline opened the stage a runtime entry was written through | `craft_ui::runtime::memory` | 7.2 |
| `OwnerClass` | A key class, the input of key ownership | craft-behavior | 9.3 |
| Paint record | The one interior-mutable cell in `Cx`, written during `paint` | `craft_ui::primitives` | 1.5 |
| Painters | `Surface`, `StateLayer`, `FocusRing`, `Rule`, `TextRun`, `ChordText`, `IconSpec`, `Handle`, `ScrollBar` and `ResetMark` | `craft_ui::primitives` | 5.2 |
| `PaintShape` | The shape `Draw::fill` and `Draw::stroke` paint: a rect with corners, a circle, a polygon or a path | `craft_ui::primitives` | 5.3 |
| `PaintUi` | What `paint` receives | `craft_ui::contract` | 5.3 |
| Pane | An F6 stop | `craft_ui::runtime::focus` | 9.4 |
| Part | A named sub-rect of a component's own surface with its own hit shape, state, tooltip and sub-node, keyed by a `PartKey` | — | 8.3 |
| `PartBuilder` | What `PartsCx::part` returns | `craft_ui::contract` | 1.5, 9.4 |
| `PartKey`, `PartKey::CELL` | The one key type of parts, declared as consts | craft-behavior | 1.5, 17.2 |
| `PartRef` | A part's name | craft-behavior | 1.5 |
| `PartResult` | A part's results this pass | `craft_ui::primitives` | 1.5, 8.3 |
| `PartsCx` | What `parts` gets, with last pass's `hovered` and `focus_visible` | `craft_ui::contract` | 1.5 |
| Pattern | A type that only assembles components | `craft_ui::contract` | 24.4 |
| `Place`, `Resize` | Where an egui layer sits (`Anchored { anchor, side, align }` or `Float`) and how the user may resize it, in `Overlay` | `craft_ui::contract` | 15.4 |
| `Pending<T, E>` | A memory slot holding a background result | `craft_ui::runtime::pending` | 14 |
| `Placed` | A keyed child given to `arrange` for a custom `Arrangement` | `craft_ui::layout` | 6.3 |
| Placement rule | A type lives in the lowest layer that names it | — | Overview |
| `Pointer` | The `Cx` trait for part results, presses, activation, wheel, cursors and drag and drop | `craft_ui::contract` | 1.5, 8 |
| `PointerFrame` | The pass's pointer snapshot, per viewport | `craft_ui::runtime::pointer` | 8.1 |
| `PointerSample` | A pen or touch sample | craft-behavior | 8.1 |
| `MultiTouch` | A two-finger gesture summary | craft-behavior | 8.1 |
| `PointerButton`, `Buttons` | Pointer buttons, egui-free | craft-behavior | 8.1 |
| `Politeness` | An announcement's or live region's urgency | `craft_ui::runtime::access` | 12.1 |
| `PopupHandle` | A `Copy` id and name for one of a component's own popups, from `cx.popup(name)`, with `trigger`, `open`, `close` and `is_open` | `craft_ui::contract` | 15.5 |
| Prelude | The stable surface of a crate | each crate | 1.11 |
| Press-target rule | A root starts a press lifecycle only when egui's press target is itself or no widget | `craft_ui::primitives` (sensing core) | 8.3 |
| `PressPhase`, `PressState` | A press lifecycle's phase, and its state with mods, sample and capture | craft-behavior | 8.2 |
| `PressStart` | A press's start: origin, button, time and click count | craft-behavior | 8.1 |
| Probe | The derive-written test entry for a registered type, with its required states | `craft_ui::testing` | 22.4 |
| `Proposal` | The question a measure call answers | craft-behavior | 6.2 |
| `Props` | The blanket setter trait over `Common` | `craft_ui::contract` | 3.2 |
| `Rank` | An overlay's place in the order (window, toast, popup, modal with its backdrop, modal chrome, tooltip) | `craft_ui::runtime` (layer stack) | 15.2 |
| Read-only | A component or scope whose values cannot change | — | 3.1, 12.4 |
| Region | A sub-rect a container lays out | — | 16.3 |
| Registration macro | The one craft-ui macro the `Component` and `CommandSet` derives call to register | `craft_ui` (macros) | 1.11 |
| Read-only output | A struct craft-ui builds and apps only read (`CxCore`, `MeasureCx`, `PartsCx`, `TextLayout`) | — | 1.11 |
| `Remember` | The memory trait on every context, over four lifetimes | `craft_ui::runtime::memory` | 7.2 |
| `ReorderMove<K>` | The value of a reorder edit in a collection | `craft_ui::contract` | 17.6 |
| `Resolved` | `Common` merged with the env and the command, assembled at step 3 | `craft_ui::contract` | 3.3 |
| `ResolvedCommand` | A command resolved for this frame and instance | `craft_ui::runtime::commands` | 11.3 |
| `Role` | A component's or sub-node's AccessKit role | `craft_ui::runtime::access` | 12.1 |
| `RootEnv` | The app's root values each frame | `craft_ui::runtime::env` | 2.3 |
| `Roving` | A roving group | craft-behavior | 9.5 |
| `RowState` | An item's row facts from the collection; the row's id scope is its key | `craft_ui::contract` | 17.2, 17.5 |
| Row shape | The theme's row drawing shape, `RowStyle` | craft-theme | 4.6 |
| `ListRowStyle` | `ListRow`'s style struct; it reads the theme's `RowStyle` | `craft_ui::list` | 17.7 |
| `runtime::toast::post` | Posts a toast from any code with a `Context` | `craft_ui::runtime::toast` | 15.5 |
| `SaveKey` | A crate-prefixed key for the saved lifetime, never derived from an id | `craft_ui::runtime::memory` | 7.2 |
| `ScopeKind`, `CancelRule`, `Initial`, `EnterRule` | A focus scope's kind, Escape rule, initial focus and Enter rule | craft-behavior (`ScopeKind`, `CancelRule`), `craft_ui::runtime::focus` (`Initial`, `EnterRule`) | 9.4 |
| `Scroll`, `ScrollAnchor`, `Notch` | `ContainerProps.scroll`, its anchor and its wheel notch distance | `craft_ui::contract` | 16.2 |
| Scroll part | The track and thumb of one axis, registered with `PartsCx::scroll(key, rect, axis, span)`, run by the pipeline and painted by `ScrollBar` | `craft_ui::contract` | 8.6 |
| Shape record | The per-viewport, egui-free record of last pass's registered shapes that H7 classifies presses from | `craft_ui::runtime::pointer` | 8.3 |
| `ScrollSpan` | What a scroll part shows on one axis | craft-behavior | 8.6 |
| `ScrollPos` | A scroll offset or extent in content units, as `f64` | craft-behavior | 8.6 |
| `SelectionModel` | A collection's selection type | craft-behavior | 17.4 |
| `SelectMode` | A collection's selection mode | `craft_ui::contract` | 17.1 |
| `Sense` | The library's sense set for a root or part | craft-behavior | 1.1 |
| `Sense::TAKE_FOCUS` | A sense bit: a press this shape wins moves key focus to its component | craft-behavior | 1.1, 8.3 |
| `Sense::WHEEL_X` | A sense bit: takes the horizontal wheel share without focus or arming | craft-behavior | 1.1, 8.6 |
| `SetPosition` | An item's index, set size and level for AccessKit | `craft_ui::runtime::access` | 12.1 |
| Settle | Step 15 | `craft_ui::contract` | 1.6 |
| `ShellInfo` | OS, runtime, backend, capabilities, menu placement and window info | craft-host (`target`) | 21.1 |
| `ShellPane` | One of `AppShell`'s panes, named by `Mount::Pane` | `craft_ui::contract` | 16.2 |
| `Show`, `ShowContainer`, `ShowCollection` | The sealed show traits the library writes for each shape | `craft_ui::contract` | 1.3 |
| `ShowAt` | Where the pipeline places a component | `craft_ui::contract` | 1.6 |
| `ShowShape` | The shape a type has (`Leaf`, `Body`, `Items`), which picks its show trait and default sense | `craft_ui::contract` | 1.1 |
| `SideLabel` | `ContainerProps.label` | `craft_ui::contract` | 16.2 |
| `FieldLabelRole` | Whether a field label names or describes its control, in `SideLabel` | `craft_ui::runtime::access` | 16.2 |
| `Sizing` | `ContainerProps.sizing` | `craft_ui::contract` | 16.2 |
| `Slot`, `Savable` | The bounds of a memory slot type, and of one the saved lifetime can store | `craft_ui::runtime::memory` | 7.2 |
| Stack position | An arena's first ordering key | — | 8.3 |
| `Stateful<T>` | The only per-state type a style field holds, resolved by one rule | craft-theme | 4.2 |
| `StateSet` | A component's or part's state, flags and animated mix for one frame | `craft_ui::style` | 4.1 |
| `Story`, `StoryOutput` | A named way to show one registered type, and its type-erased output | `craft_ui::testing` | 22.4 |
| `StoryData` | A story's own values, `value<T: Value>(name, init)`, snapshotted by the runner before and after a drive | `craft_ui::testing` | 22.4 |
| `*Style` structs | A component's `type Style` | the component's family | 1.11 |
| `StyleColor` | A colour a style builder picked from tokens | `craft_ui::style` | 4.3 |
| `StyleCx` | What `style` and the app colour hooks get | `craft_ui::style` | 1.5, 4 |
| `Styled` | The `Cx` trait with `resolve`, `resolve_part`, `state()` and `ink()`, all reading step 12's output | `craft_ui::contract` | 1.5, 4.2 |
| `StyleKey` | The derive-written `'static` key of the instance inputs a style depends on, generic over a generic component's type parameters (one per instantiation) | `craft_ui::style` | 1.1 |
| `StyleReplacements` | The type-erased, frozen form of `Looks` that `InstallSpec.looks` holds | `craft_ui::style` | 1.8, 4.4 |
| Sub-node, `SubNode` | An AccessKit node below a component's node, built with `Access::node(part, role)` for parts, cells and content, keyed by `PartRef` | `craft_ui::runtime::access` | 12.1 |
| Runtime sub-id | An id the runtime derives from a component id through `sub(base, tag)` | `craft_ui::runtime::memory` | 7.1 |
| `SurfaceKind` | The surface a container or part sits on (`Panel`, `Raised`, `Well`, `Popup`, ...), which picks fill, edge, shadow and ink | `craft_ui::style` | 5.2 |
| `TextKey`, `Codec` | The key of a text part and the conversion between its text and value | `craft_ui::primitives` (`TextKey`), craft-behavior (`Codec`) | 18.2 |
| `TextPos` | A position in an editor's visible lines | craft-behavior | 18.4 |
| `TextOptions`, `Completer`, `Completion` | A text part's options, and the trait and candidate type of its completion list | `craft_ui::primitives` (text adapter) | 18.2 |
| `TextOutcome` | A text part's result this pass, with its `TextLayout`, from `cx.text(&key)` | `craft_ui::primitives` | 18.2 |
| Text parts | `layout`, text outcomes and the editor stream and state, methods of `MeasureCx` and `PartsCx` | `craft_ui::contract` | 1.5, 18 |
| `TextRun`, `TextLayout` | The one description of UI text, and its laid-out form | `craft_ui::primitives` | 5.2 |
| `TextureCache` | The one texture path | `craft_ui::primitives::textures` | 5.5 |
| `ThemeScope` | Token-changing values (mode, contrast, a named theme, density, text size) for a container's body | `craft_ui::runtime::env` | 2.3 |
| `TitleRow`, `FooterBar` | The layer-5 components library containers show in their header and footer regions | `craft_ui` title-row and bar families | 16.4 |
| `Tokens` | craft-theme's resolved semantic token set | craft-theme | 4.4, 4.5 |
| Token group | A crate-prefixed group of semantic colour and shape tokens for a crate's own concepts | craft-theme (trait), `craft_ui::style` (cache) | 4.5 |
| `TokenGroup` | The trait a token group implements | craft-theme | 4.5 |
| `Tone` | The tone axis of a variant | craft-theme | 4.4 |
| `Tooltip`, `TooltipSpec` | The one tooltip type, for components (`Common.tooltip`) and parts (`PartBuilder::tooltip`), and its boxed data | `craft_ui::contract` | 3.4 |
| Trial | One test in an integration binary, registered through the one registration macro | `craft_ui::testing` | 22.2 |
| `Trigger` | How a popup opens (`Manual`: only through `PopupHandle::open` and the binding) | `craft_ui::contract` | 15.5 |
| `UiTokens` | The runtime's egui-ready view of one resolved token set | `craft_ui::style` | 4.5 |
| `Validity<T>` | A binding check's verdict | craft-behavior | 10.2 |
| `Value` | The bound every value type meets | craft-behavior | 10.2 |
| `ValueOf<C>` | The value type of component `C`, `<C::Output as Outcome>::Value` | `craft_ui::contract` | 10.2 |
| Variant | A component's look within its style | craft-theme | 4.4 |
| Viewport mount | `Mount::Viewport` | `craft_ui::contract` | 16.2 |
| `ViewportShare` | A fraction of the viewport per axis, the cap of `OverlayFit::ContentCapped` | `craft_ui::contract` | 15.4 |
| `Wheel` | A component's share of wheel and pinch input after the split rule | craft-behavior | 8.6 |
| `WheelUnit`, `WheelPhase` | A wheel event's unit and phase, craft-behavior copies of egui's | craft-behavior | 8.6 |
| `WindowChrome` | How a window's frame is drawn (OS, custom, none), part of `WindowInfo` | craft-host (`target`) | 21.1 |
| `WindowInfo` | A window's kind, chrome, state, insets and `active` (OS focus), refreshed in `set_input` | craft-host (`target`) | 21.1 |
| `WordSet` | The trait a `words!` block writes | the word core (owner question 13) | 19.2 |
| `WordSetCx` | The trait on every context with `label()` and `words::<L>()` | `craft_ui::contract` | 19.2 |
| `words::<L>()` | The `WordSetCx` method that returns the word set `L` | `WordSetCx` method | 19.2 |

