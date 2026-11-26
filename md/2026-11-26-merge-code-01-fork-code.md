# The fork's code in craft-libs after the upstream merges

The files the merges and their repairs decided on. The upstream is `storytold/craft-libs` `next`
at `e17227177`; the fork's `next` is `e629d2315`.

## How a file got here

A file is here when a merge of this run had to decide something about it: its merged content differs
from **both** parents, or a repair commit after a merge wrote it. The fork's own identity (its crates,
documents and plugins) is not a merge decision and is not listed.

Every diff is `git diff <upstream> <head> -- <path>`, with `<head>` = the fork's branch tip of this run.

## `Cargo.lock`

**Role.** The workspace lock file.

**The code.**

```diff
diff --git a/Cargo.lock b/Cargo.lock
index 4b32db172..a9ebaf6b7 100644
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -1419,9 +1419,9 @@ dependencies = [
 
 [[package]]
 name = "clap_lex"
-version = "1.1.1"
+version = "1.1.2"
 source = "registry+https://github.com/rust-lang/crates.io-index"
-checksum = "1c133bc6a41be0d194c306b5506d15e6feeea7b1d6604bd3f8310dfb2ca96486"
+checksum = "e0921b3876b3a1df8a1c9f912e7f94342458022c2e84288015893d83c4d62a79"
 
 [[package]]
 name = "clipboard-win"
```

**Hunk by hunk.**

1. Upstream moved the egui pin to `b9770908e` and craft-libs to its own `next`; the graph resolves with them, and the file regenerates.

**Checked by.** `cargo metadata` regenerates it; `cargo clippy --workspace --all-targets` (exit 0) proves the graph.

## `.github/workflows/workspace.yml`

**Role.** The CI workflow that builds the workspace and prunes the cargo cache.

**The code.**

```diff
diff --git a/.github/workflows/workspace.yml b/.github/workflows/workspace.yml
index 5c802e799..1abc5e03d 100644
--- a/.github/workflows/workspace.yml
+++ b/.github/workflows/workspace.yml
@@ -2,7 +2,7 @@ name: Workspace
 
 on:
   push:
-    branches: [main, next]
+    branches: [main]
     tags: ["v*"]
   pull_request:
   workflow_dispatch:
@@ -246,7 +246,15 @@ jobs:
             entry=${BASH_REMATCH[1]}
             if [ -n "${kept[$entry]:-}" ]; then
               echo "delete $key"
-              gh cache delete "$id" --repo "$REPO"
+              # A scheduled run and a push prune in different concurrency groups, so both can list the
+              # same cache and one can delete it first: a missing cache is success here, while any other
+              # API error still fails the job.
+              if ! gh cache delete "$id" --repo "$REPO" 2>"$RUNNER_TEMP/cache-error"; then
+                if ! grep -Eqi 'not found|HTTP 404' "$RUNNER_TEMP/cache-error"; then
+                  cat "$RUNNER_TEMP/cache-error"
+                  exit 1
+                fi
+              fi
             else
               kept[$entry]=1
               echo "keep $key"
```

**Hunk by hunk.**

1. The push trigger names `main` only, as the fork's copy always has; upstream's lists `main, next`.
2. The cache prune tolerates a cache another run already deleted: only `not found` or `HTTP 404` counts as success, and any other API error prints the output and fails the job.

**Checked by.** No cargo command covers a workflow file; the workflow runs on the fork remote's `main`.

## `README.md`

**Role.** The workspace README's crate table.

**The code.**

```diff

```

**Hunk by hunk.**

1. The fork's README keeps upstream's new crate rows and its own heading order, so the merged table reads as one document.

**Checked by.** No command; the file is prose.

## `crates/font-manager/src/picker.rs`

**Role.** The font picker.

**The code.**

```diff
diff --git a/crates/font-manager/src/picker.rs b/crates/font-manager/src/picker.rs
index c89214e6f..f6a1e3fdc 100644
--- a/crates/font-manager/src/picker.rs
+++ b/crates/font-manager/src/picker.rs
@@ -142,6 +142,11 @@ impl FontClass {
     pub fn of(catalog: &Catalog, family: &str) -> Option<FontClass> {
         rank::class_of(catalog, family)
     }
+
+    /// Classify a font family from its name, `sFamilyClass`, PANOSE, and fixed-pitch flag.
+    pub fn classify(family: &str, family_class: i16, panose: [u8; 10], fixed_pitch: bool) -> Option<FontClass> {
+        rank::classify(family, family_class, panose, fixed_pitch)
+    }
 }
 
 /// Photoshop's Font Preview Size (Type › Font Preview Size): how large the sample in each row is,
@@ -556,7 +561,7 @@ struct FamilyCodec {
 
 impl FamilyCodec {
     fn shown(&self, family: &str, info: &Info) -> String {
-        shown_name(info, family, &self.language, false)
+        shown_name(&info.localized, family, &self.language, false)
     }
 }
 
```

**Hunk by hunk.**

1. `FontClass::classify` exposes the name, `sFamilyClass`, PANOSE and fixed-pitch reading to callers outside the crate.
2. `FamilyCodec::shown` passes the localized names, the signature upstream changed this run.

**Checked by.** `cargo nextest run -p craft-font-manager` (green in the 18,710-trial workspace run).

## `crates/font-manager/tests/it/picker.rs`

**Role.** The font picker's trials.

**The code.**

```diff
diff --git a/crates/font-manager/tests/it/picker.rs b/crates/font-manager/tests/it/picker.rs
index 2b8992950..f566a3bba 100644
--- a/crates/font-manager/tests/it/picker.rs
+++ b/crates/font-manager/tests/it/picker.rs
@@ -302,6 +302,36 @@ fn a_click_on_the_focused_closed_field_selects_the_name_so_typing_replaces_it()
     }
 }
 
+#[trial]
+fn classifies_japanese_and_cjk_families() {
+    use craft_font_manager::picker::FontClass;
+    const NONE: [u8; 10] = [0; 10];
+    assert_eq!(FontClass::classify("MS Mincho", 0, NONE, false), Some(FontClass::Serif));
+    assert_eq!(FontClass::classify("ＭＳ 明朝", 0, NONE, false), Some(FontClass::Serif));
+    assert_eq!(FontClass::classify("Yu Gothic", 0, NONE, false), Some(FontClass::SansSerif));
+    assert_eq!(FontClass::classify("游ゴシック", 0, NONE, false), Some(FontClass::SansSerif));
+    assert_eq!(FontClass::classify("Meiryo", 0, NONE, false), Some(FontClass::SansSerif));
+    assert_eq!(FontClass::classify("メイリオ", 0, NONE, false), Some(FontClass::SansSerif));
+    assert_eq!(FontClass::classify("HG行書体", 0, NONE, false), Some(FontClass::Script));
+    assert_eq!(FontClass::classify("HG正楷書体-PRO", 0, NONE, false), Some(FontClass::Script));
+    assert_eq!(FontClass::classify("UD デジタル 教科書体", 0, NONE, false), Some(FontClass::Handwritten));
+    assert_eq!(FontClass::classify("HG創英角ﾎﾟｯﾌﾟ体", 0, NONE, false), Some(FontClass::Handwritten));
+    assert_eq!(FontClass::classify("MS Gothic 等幅", 0, NONE, false), Some(FontClass::Monospace));
+    // A Latin word that merely ends in `song` is not a CJK serif (WindSong is a script family); the foundry stems
+    // keep their Song/Songti reading.
+    assert_eq!(FontClass::classify("WindSong", 0, NONE, false), None, "an English word ending in song is not a serif");
+    assert_eq!(FontClass::classify("STSong", 0, NONE, false), Some(FontClass::Serif));
+    assert_eq!(FontClass::classify("STZhongsong", 0, NONE, false), Some(FontClass::Serif));
+    assert_eq!(FontClass::classify("FZFangSong-Z02", 0, NONE, false), Some(FontClass::Serif));
+    // A generic `pop` fallback sits after the Script markers: `Pop Script` keeps Script, a bare `Pop` is the rounded
+    // display face.
+    assert_eq!(FontClass::classify("Pop Script", 0, NONE, false), Some(FontClass::Script));
+    assert_eq!(FontClass::classify("Pop", 0, NONE, false), Some(FontClass::Handwritten));
+    // A `hei` (黑) suffix is a CJK sans, but not against an explicit script marker: `Xhei Script` is a script.
+    assert_eq!(FontClass::classify("SimHei", 0, NONE, false), Some(FontClass::SansSerif));
+    assert_eq!(FontClass::classify("Xhei Script", 0, NONE, false), Some(FontClass::Script));
+}
+
 fn three_faces() -> Arc<Catalog> {
     catalog_of(&[epaint_default_fonts::HACK_REGULAR, epaint_default_fonts::UBUNTU_LIGHT, INTER_REGULAR])
 }
```

**Hunk by hunk.**

1. `classifies_japanese_and_cjk_families` pins 明朝/ゴシック/メイリオ/行書/教科書/ﾎﾟｯﾌﾟ/等幅 and the boundaries: an English word ending in `song` is no CJK serif, `Xhei Script` stays a script, `Pop` is handwritten while `Pop Script` is a script.

**Checked by.** `cargo nextest run -p craft-font-manager` (green).

## `crates/pages/tests/it/views.rs`

**Role.** The pages crate's screen trials.

**The code.**

```diff
diff --git a/crates/pages/tests/it/views.rs b/crates/pages/tests/it/views.rs
index bc77a2420..077de5599 100644
--- a/crates/pages/tests/it/views.rs
+++ b/crates/pages/tests/it/views.rs
@@ -1198,6 +1198,26 @@ fn the_reading_view_turns_on_the_wheel_but_leaves_cmd_or_ctrl_with_the_wheel_to_
     assert!(state.borrow().column > 0, "the wheel alone turns");
 }
 
+#[trial]
+fn the_reading_view_turns_a_column_on_space_and_enter() {
+    let state = Rc::new(RefCell::new(ReadingState::default()));
+    let kept = Rc::clone(&state);
+    let pages = DemoPages::new(6);
+    let mut fixture = Fixture::new(move |ui| {
+        ReadingView::new("Document", &pages, &mut kept.borrow_mut(), PAPER).show(ui);
+    });
+    fixture.room(vec2(900.0, 600.0));
+    fixture.settle();
+    fixture.key(Key::Tab, Modifiers::NONE);
+    fixture.settle();
+    fixture.key(Key::Space, Modifiers::NONE);
+    fixture.idle(1);
+    assert_eq!(state.borrow().column, 1, "Space turns one column");
+    fixture.key(Key::Enter, Modifiers::NONE);
+    fixture.idle(1);
+    assert_eq!(state.borrow().column, 2, "Enter still turns one column");
+}
+
 /// A US Letter page, points.
 const LETTER: kurbo::Size = kurbo::Size::new(612.0, 792.0);
 
```

**Hunk by hunk.**

1. `the_reading_view_turns_a_column_on_space_and_enter` turns the column with Space and then Enter, one column per press.

**Checked by.** `cargo test -p craft-pages --features screen-testing --lib` (green).

## `crates/ui-contract/src/runtime/focus/registration.rs`

**Role.** The focus stop.

**The code.**

```diff
diff --git a/crates/ui-contract/src/runtime/focus/registration.rs b/crates/ui-contract/src/runtime/focus/registration.rs
index c504c221d..ccaa95cc5 100644
--- a/crates/ui-contract/src/runtime/focus/registration.rs
+++ b/crates/ui-contract/src/runtime/focus/registration.rs
@@ -116,6 +116,13 @@ impl Stop {
         self
     }
 
+    /// Activates on Space as well as Enter, as a button does: the runtime presses it on the Space press and
+    /// activates it on the release, which is what a click does.
+    pub fn space(mut self) -> Self {
+        self.keys = self.keys.with_space_activation();
+        self
+    }
+
     /// Sits in the arena of the root `arena` (a canvas body), if any, whether or not it joined it.
     pub fn in_arena(mut self, arena: Option<Id>) -> Self {
         self.arena = arena;
```

**Hunk by hunk.**

1. `Stop::space()` adds Space to a stop's activation keys, so a component asks for the key without naming the enum.

**Checked by.** `cargo nextest run -p craft-ui-contract` (green).

## `crates/ui-contract/src/runtime/keys.rs`

**Role.** The key batch of a pass.

**The code.**

```diff
diff --git a/crates/ui-contract/src/runtime/keys.rs b/crates/ui-contract/src/runtime/keys.rs
index d02ff012b..ae96d039c 100644
--- a/crates/ui-contract/src/runtime/keys.rs
+++ b/crates/ui-contract/src/runtime/keys.rs
@@ -187,6 +187,11 @@ struct Batch {
     tabs: Vec<TabPress>,
     /// A lone Alt in a field that holds drafts was refused: the input hook calls `frame::refuse`.
     refused: bool,
+    /// A bare Space release of the pass: it ends a held Space activation (`route::space_held`).
+    space_released: bool,
+    /// A Space release the pass skipped because its press had not come yet: the hook reads it once, so it waits for
+    /// a press in a later pass (`route::space_held`).
+    space_release_pending: bool,
     /// The place of the event the strip reads now among the events the IME engine left in the frame's input: the
     /// engine places its edits by the same count (`ime::take_placed`), so an editor part reads the two in the order
     /// they arrived.
```

**Hunk by hunk.**

1. `space_released` records a bare Space release in this pass.
2. `space_release_pending` records a Space release whose press has not arrived, because the hook reads a pass's keys once.

**Checked by.** `cargo nextest run -p craft-ui-contract` (green).

## `crates/ui-contract/src/runtime/keys/route.rs`

**Role.** The key router.

**The code.**

```diff
diff --git a/crates/ui-contract/src/runtime/keys/route.rs b/crates/ui-contract/src/runtime/keys/route.rs
index f78178bfb..f0c30fd20 100644
--- a/crates/ui-contract/src/runtime/keys/route.rs
+++ b/crates/ui-contract/src/runtime/keys/route.rs
@@ -19,6 +19,23 @@ use std::sync::Arc;
 /// The begin hook's intents: the pass's key events, AccessKit requests and claimed chords, routed to the
 /// focused component (or to the component that owns a requested node). Returns how the pass's Escapes split
 /// between the captured press and the components.
+/// A held Space: a pass with neither a release nor a pending one clears the hold (a Space pressed in an earlier pass
+/// is still down), and the pass that brings the release activates the stop that was pressed - as a mouse release
+/// clicks, and even when the focus has moved since, since the held stop's member and root are what the release has.
+fn space_release(ctx: &Context, batch: &Batch, target: Option<&StopRec>, routed: &mut HashMap<Option<Id>, Routed>, activated: &mut IdMap<Id>) {
+    if !batch.space_released && !batch.space_release_pending {
+        space_held::clear(ctx);
+        return;
+    }
+    if target.is_none() {
+        return;
+    }
+    if let Some(held) = space_held::held(ctx) {
+        space_held::clear(ctx);
+        route_activate(ctx, &held.record(), routed, activated);
+    }
+}
+
 pub(crate) fn begin(ctx: &Context, press_captured: bool) -> EscapeRouting {
     let os = crate::runtime::windows::os_of(ctx);
     let target = focus::focused_rec(ctx);
@@ -94,6 +111,7 @@ pub(crate) fn begin(ctx: &Context, press_captured: bool) -> EscapeRouting {
         .map(|(at, _)| at)
         .chain(batch.presses.iter().filter(|press| held_press(press)).filter_map(|press| press.text))
         .collect();
+    space_release(ctx, &batch, target.as_ref(), &mut routed, &mut activated);
     if let Some(rec) = &target {
         for (press, taken) in batch.presses.iter().zip(&mut consumed).filter(|(press, _)| !press.canvas) {
             *taken = if let Some(id) = hover_delete.filter(|id| Some(*id) != target.as_ref().map(|rec| rec.stop.id))
@@ -695,6 +713,8 @@ enum PressRoute {
     Move(NavMove, MoveMods),
     /// The stop's activation.
     Activate,
+    /// The press of Space on a stop that activates on it: the release is the activation.
+    Space,
     /// A close of the dropdown layer whose opener has the focus.
     Close(Id),
     /// An intent of the stop.
@@ -754,7 +774,13 @@ fn press_route(ctx: &Context, os: Os, rec: &StopRec, press: &Press, driven: Opti
         // app's.
         None if press.key == Key::Enter && stop.keys.activate.enter() && (plain || !commands::binds_scoped(ctx, &chord)) => PressRoute::Activate,
         None if press.key == Key::Enter && !press.mods.cmd && !press.mods.ctrl => PressRoute::Intent(Intent::Submit),
-        None if press.key == Key::Space && plain && stop.keys.activate.space() => PressRoute::Activate,
+        // Space activates only a stop that asks for it (a button, a rail tool, a member of a roving group): the
+        // release activates, as a mouse release does, and the press alone does nothing. `apply_press` holds the
+        // button down for the frames between. Every other stop leaves Space to the app, where it is the canvas's Hand
+        // key.
+        // Space activates a stop that asks for it, as a button does, whether it names Space there
+        // (`space_activates`) or among its activation keys (`ActivateKeys::Space`/`Both`).
+        None if press.key == Key::Space && plain && (stop.keys.space_activates || stop.keys.activate.space()) => PressRoute::Space,
         None if press.key == Key::Tab && plain && stop.keys.keep == focus::KeepKeys::Tab => PressRoute::Intent(Intent::Tab),
         // A document canvas's editor reads Shift+Tab itself (a word processor's outdent, the previous slot of an
         // equation). No other stop that keeps Tab gets it as an intent.
@@ -805,7 +831,16 @@ fn apply_press(
                 push(routed, id, Intent::Move(mv, mods), horizontal);
             }
         }
-        PressRoute::Activate => route_activate(ctx, rec, routed, activated),
+        PressRoute::Activate => {
+            space_held::clear(ctx);
+            route_activate(ctx, rec, routed, activated);
+        }
+        // The Space key activates: the input hook holds a tap's press until its release, so both reach this pass and
+        // the stop activates once, as a click does.
+        PressRoute::Space => {
+            space_held::clear(ctx);
+            route_activate(ctx, rec, routed, activated);
+        }
         PressRoute::Close(layer) => overlay::request_close(ctx, layer, overlay::CloseReason::Requested),
         PressRoute::Intent(intent) => push(routed, id, intent, false),
         PressRoute::Nudge(nudge) => routed.entry(id).or_default().nudge = Some(nudge),
@@ -813,6 +848,50 @@ fn apply_press(
     Taken::Yes
 }
 
+/// The stop whose Space is down: the press takes nothing, and the pass that brings the release activates the stop,
+/// which is what a mouse release does. A button that pressed its widget down here would stay pressed forever when the
+/// key up came before that widget was drawn.
+mod space_held {
+    use crate::runtime::egui::memory;
+    use crate::runtime::focus::{Stop, StopRec};
+    use egui::{Context, Id};
+
+    /// The stop a Space is down on. The runtime keeps it rather than that pass's record: the release may come with
+    /// another stop focused, and the activation belongs to the one that was pressed.
+    #[derive(Clone, Debug)]
+    pub(super) struct Held {
+        pub(super) id: Id,
+        pub(super) root: Id,
+        pub(super) group: Option<Id>,
+    }
+
+    /// The key of the hold in the context's memory: one per context, so two fixtures never share a hold.
+    fn key(ctx: &Context) -> Id {
+        memory::unique(("craft_ui.keys.space_held", ctx.viewport_id()))
+    }
+
+    /// The stop whose Space was released this pass.
+    pub(super) fn held(ctx: &Context) -> Option<Held> {
+        memory::peek_clone(ctx, key(ctx))
+    }
+
+    /// The pass is over: the stop is no longer the one being activated.
+    pub(super) fn clear(ctx: &Context) {
+        memory::take::<Held>(ctx, key(ctx));
+    }
+
+    impl Held {
+        /// The stop the release activates, as a record of its own: the release may come with nothing focused or with
+        /// another stop focused.
+        pub(super) fn record(&self) -> StopRec {
+            let mut stop = Stop::new(self.id, crate::runtime::focus::KeyUse::of(craft_behavior::Input::Pointer));
+            stop.root = self.root;
+            stop.group = self.group;
+            StopRec { stop, scope: None, group_position: false }
+        }
+    }
+}
+
 /// `Activate` of a group member goes to the group's root, which reads `activated_member`. A child or part of a
 /// collection row activates itself.
 fn route_activate(ctx: &Context, rec: &StopRec, routed: &mut HashMap<Option<Id>, Routed>, activated: &mut IdMap<Id>) {
```

**Hunk by hunk.**

1. `space_release` holds the Space decision: a pass with neither release nor pending release clears the hold, and the pass that brings the release activates the stop that was pressed, even when the focus moved. It exists because the guard's `cognitive_complexity` reached 31/30 inside `begin`, and the workspace builds with `-D warnings`.
2. `PressRoute::Space` is a route of its own: Space on a stop that asks for it activates on the release, as a click does; a stop asks by name or among its activation keys, and every other stop leaves Space to the canvas's Hand key.
3. `space_held` keeps the stop whose Space is down in the context's memory, one hold per context, so a release reaches the stop that was pressed.

**Checked by.** `cargo nextest run -p craft-ui-contract` and `-p craft-ui-widgets` (green); the menu-opener, link, select and word-run trials need the route.

## `crates/ui-panels/src/rails/flyout.rs`

**Role.** The rail's flyout button.

**The code.**

```diff
diff --git a/crates/ui-panels/src/rails/flyout.rs b/crates/ui-panels/src/rails/flyout.rs
index deca16fdb..10a991019 100644
--- a/crates/ui-panels/src/rails/flyout.rs
+++ b/crates/ui-panels/src/rails/flyout.rs
@@ -5,7 +5,7 @@ use super::words::RailWords;
 use crate::buttons::{IconButton, PopupMark};
 use crate::contract::overlay::{Anchor, Place};
 use crate::contract::{
-    Access, Common, Component, Cx, DragPreview, Focus as _, MeasureCx, Name, NoValue, Outcome, OverlayUi, Overlays as _, PaintUi, PartsCx,
+    Access, Common, Component, Cx, DragPreview, Focus as _, Input, MeasureCx, Name, NoValue, Outcome, OverlayUi, Overlays as _, PaintUi, PartsCx,
     Pointer as _, PopupKind, Props as _, Remember as _, Role, Show as _, ShowContainer as _, Trigger, WordSetCx as _,
 };
 use crate::display::TargetMark;
@@ -14,6 +14,7 @@ use crate::menu::{self, Menu, MenuItem};
 use crate::primitives::{IconSpec, PaintShape, StrokeWidth};
 use crate::runtime::commands::{CommandRef, FireSource, ResolvedCommand};
 use crate::runtime::env::Label;
+use crate::runtime::focus::KeyUse;
 use crate::style::{ComponentStyle, FocusLook, GlyphRole, StyleColor, StyleCx};
 use craft_behavior::overlay::{Align, Side};
 use craft_behavior::{CursorIcon, HitLevel, HitPriority, IconId, Metrics, PartKey, PointerButton, PressPhase, Sense, Value};
@@ -237,6 +238,8 @@ impl<'a, T: Value + 'static> FlyoutButton<'a, T> {
 
 impl<T: Value + 'static> Component for FlyoutButton<'_, T> {
     const ROLE: Role = Role::BUTTON;
+    /// Space runs the face's command as a tap does: the focus has no held press, so it opens no flyout.
+    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_space_activation();
     const SHOWS_LABEL: bool = false;
     type Style = FlyoutButtonStyle;
     type Output = FlyoutOutput<T>;
```

**Hunk by hunk.**

1. `FlyoutButton` declares Space among its activation keys, merged with upstream's new `FocusLook` field: Space runs the face's command as a tap does and opens no flyout.

**Checked by.** `cargo nextest run -p craft-ui-panels` (green).

## `crates/ui-panels/src/dialog/prompt.rs`

**Role.** The dialog prompt.

**The code.**

```diff
diff --git a/crates/ui-panels/src/dialog/prompt.rs b/crates/ui-panels/src/dialog/prompt.rs
index 9ae51f814..75b50f3c7 100644
--- a/crates/ui-panels/src/dialog/prompt.rs
+++ b/crates/ui-panels/src/dialog/prompt.rs
@@ -227,7 +227,8 @@ pub struct Prompt<'a> {
     escape: Option<DialogRole>,
     /// Whether the dialog reports the alert-dialog role, described by the message and any tone.
     alert: bool,
-    /// What the dialog's layer is for.
+    /// What the dialog's layer is for: `LayerRole::Progress` for a job's progress window.
+    #[prop]
     layer_role: LayerRole,
 }
 
@@ -255,13 +256,6 @@ impl<'a> Prompt<'a> {
         self
     }
 
-    /// What the dialog's layer is for: `LayerRole::Progress` for a job's progress window.
-    #[must_use]
-    pub(super) fn layer_role(mut self, role: LayerRole) -> Self {
-        self.layer_role = role;
-        self
-    }
-
     /// The answers, one footer button each.
     #[must_use]
     pub fn answers(mut self, answers: impl IntoIterator<Item = FooterButton>) -> Self {
```

**Hunk by hunk.**

1. `layer_role` becomes a `#[prop]` field and the hand-written setter is gone: the derive writes it, which is what the design lint's `props/hand-setter` asks for (upstream's own tree carried the finding).

**Checked by.** `cargo nextest run -p craft-design-lint` (28 trials, green) and `-p craft-ui-panels`.

## `crates/draw/src/ui/pen/bezier/model.rs`

**Role.** The pen's bezier overlay and its trials' counters.

**The code.**

```diff
diff --git a/crates/draw/src/ui/pen/bezier/model.rs b/crates/draw/src/ui/pen/bezier/model.rs
index d95f1d053..1e89aa60f 100644
--- a/crates/draw/src/ui/pen/bezier/model.rs
+++ b/crates/draw/src/ui/pen/bezier/model.rs
@@ -332,6 +332,7 @@ impl SelectedAnchors {
 pub(crate) mod reads {
     use std::cell::Cell;
 
+    // design: allow[contract/no-global-state](a trial's counter of overlay reads: it measures the drawing, it drives none of it)
     thread_local! {
         static READS: Cell<usize> = const { Cell::new(0) };
     }
```

**Hunk by hunk.**

1. The trial's `thread_local!` read counter carries a `design: allow[contract/no-global-state]` marker: it measures the drawing and drives none of it, and the lint offers no runtime replacement for a counter.

**Checked by.** `cargo nextest run -p craft-design-lint` (green).

## `crates/menu/src/mac.rs`

**Role.** The macOS native menu driver.

**The code.**

```diff
diff --git a/crates/menu/src/mac.rs b/crates/menu/src/mac.rs
index ea200bd45..0a76fc4b9 100644
--- a/crates/menu/src/mac.rs
+++ b/crates/menu/src/mac.rs
@@ -154,7 +154,9 @@ impl Native {
     fn update(&mut self, bar: &MenuBar) {
         let (spec, targets) = lower(bar);
         let (native, live) = (&mut self.bar, &self.targets);
-        let attempt = self.retry.attempt(&spec, Instant::now, || {
+        // design: allow[contract/motion](the retry clock of a native menu driver: no frame draws while it waits)
+        let now = Instant::now;
+        let attempt = self.retry.attempt(&spec, now, || {
             let rebuilt = native.apply(&spec);
             if rebuilt {
                 *lock(live) = targets;
@@ -165,7 +167,7 @@ impl Native {
         // due asks again.
         if let Attempt::Retry(wait) = attempt {
             self.pending = Some(bar.clone());
-            self.ctx.request_repaint_after(wait);
+            self.ctx.request_repaint_after(wait); // design: allow[contract/motion](a native menu driver asks the context for the retry frame: it has no frame of its own)
         }
     }
 }
```

**Hunk by hunk.**

1. The retry clock is bound under a `design: allow[contract/motion]` marker: the driver asks the context for its retry frame and reads the clock, and no frame draws while the native menus rebuild.
2. `request_repaint_after` carries the same allowance: a native menu driver has no frame of its own.

**Checked by.** `cargo nextest run -p craft-design-lint` (green) and `cargo clippy -p craft-menu --all-targets -- -D warnings`.

## `crates/ui-widgets/src/buttons/icon_button.rs`

**Role.** The icon button.

**The code.**

```diff
diff --git a/crates/ui-widgets/src/buttons/icon_button.rs b/crates/ui-widgets/src/buttons/icon_button.rs
index cbf0e0fe5..1279598e2 100644
--- a/crates/ui-widgets/src/buttons/icon_button.rs
+++ b/crates/ui-widgets/src/buttons/icon_button.rs
@@ -163,6 +163,9 @@ impl<'a> IconButton<'a> {
 impl Component for IconButton<'_> {
     const ROLE: Role = Role::BUTTON;
     const ALT_ROLES: &'static [Role] = &[Role::COMBO_BOX];
+    /// Space presses it as a click does, as [`Button`](crate::buttons::button::Button) does: a focused icon that opens a
+    /// menu opens it from the keyboard, and the runtime activates on the release.
+    const KEYS: crate::runtime::focus::KeyUse = crate::runtime::focus::KeyUse::of(crate::contract::Input::Pointer).with_space_activation();
     const PARTS: bool = false;
     const SHOWS_LABEL: bool = false;
     const OVERLAYS: bool = false;
```

**Hunk by hunk.**

1. `IconButton` declares Space among its activation keys, as `Button` does: upstream's menu trials press Space on a focused opener, and the fork's rule is that a control takes Space only by asking.

**Checked by.** `cargo nextest run -p craft-ui-widgets -E 'test(menu::openers)'` (13 trials, green).

## `design/exceptions/craft-draw/bezier.toml`

**Role.** The draw crate's design-lint exemptions.

**The code.**

```diff
diff --git a/design/exceptions/craft-draw/bezier.toml b/design/exceptions/craft-draw/bezier.toml
new file mode 100644
index 000000000..0d5309473
--- /dev/null
+++ b/design/exceptions/craft-draw/bezier.toml
@@ -0,0 +1,9 @@
+config_version = 1
+
+[[exception]]
+name = "bezier-overlay-read-counter"
+kind = "marker"
+rule = "contract/no-global-state"
+path = "crates/draw/src/ui/pen/bezier/model.rs"
+reason = "A trial's per-thread count of overlay reads: it measures the drawing and drives none of it."
+count = 1
```

**Hunk by hunk.**

1. `bezier-overlay-read-counter` registers the trial counter's marker with its reason and its one site.

**Checked by.** `cargo nextest run -p craft-design-lint` (green); an entry whose count does not match its sites is `exception/stale`.

## `design/exceptions/craft-menu/mac.toml`

**Role.** The menu crate's design-lint exemptions.

**The code.**

```diff
diff --git a/design/exceptions/craft-menu/mac.toml b/design/exceptions/craft-menu/mac.toml
new file mode 100644
index 000000000..0dc8b32ad
--- /dev/null
+++ b/design/exceptions/craft-menu/mac.toml
@@ -0,0 +1,9 @@
+config_version = 1
+
+[[exception]]
+name = "native-menu-retry-clock"
+kind = "marker"
+rule = "contract/motion"
+path = "crates/menu/src/mac.rs"
+reason = "A native menu driver's retry waits on the context's clock: no frame draws while the native menus are rebuilt."
+count = 2
```

**Hunk by hunk.**

1. `native-menu-retry-clock` registers both motion markers of the native driver: the clock it reads and the frame it asks for.

**Checked by.** `cargo nextest run -p craft-design-lint` (green).

## Correction, 2026-11-27

The macOS menu driver's exemption is upstream's, not this fork's, and the two files above no
longer describe the tree.

- Upstream `76322b729` registers the AppKit rebuild retry in
  `design/exceptions/craft-menu/crate.toml` (`mac-menu-rebuild-retry-clock`, `contract/motion`,
  one site) and keeps `std::time::Instant` behind the marker on `crates/menu/src/mac.rs` line 157.
  That is the same site this fork had registered twice.
- `design/exceptions/craft-menu/mac.toml` (this fork's entry, `count = 2`) is therefore removed:
  with it, `craft-design-lint` reported `exception/stale` for both files (one entry claimed two
  sites where one exists, the other claimed one site where none was left). Without it, the lint
  reports **28 passed / 0 failed**.
- `Prompt::layer_role` is `#[prop(vis = "pub(super)")]` upstream; the plain `#[prop]` recorded
  above is superseded. The hand-written setter is gone on both sides.
- The fork keeps its own markers where upstream has none: `design/exceptions/craft-draw/bezier.toml`
  is untouched.

### Correction, 2026-11-27: the hold lives on the press

The quoted `PressRoute::Space` arm of that round cleared the hold and activated at once; the tree now
holds the stop on the press and activates it on the release (`crates/ui-contract/src/runtime/keys/route.rs`:
`PressRoute::Space => space_held::hold(ctx, &rec.stop)`, and `space_release` activates the held record
when the pass carries the release). The input hook no longer defers a Space press for its release
(`crates/ui-contract/src/runtime/keys/input.rs`), so a tap reaches the stop once and nothing waits
behind the pair.

