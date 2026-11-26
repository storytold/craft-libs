# The upstream code behind the answers in craft-libs

The upstream lines each answer had to meet.

Quotations are `git show e17227177:<path>` with that commit's numbers.

## 1. `shown_name` takes localized names

**The upstream code.**

`crates/font-manager/src/picker/rank.rs`, lines 159-168:

```rust
  159 | pub(super) fn shown_name(info: &Info, family: &str, language: &str, english: bool) -> String {
  160 |     let primary = |tag: &str| tag.split(['-', '_']).next().unwrap_or(tag).to_ascii_lowercase();
  161 |     if english || primary(language) == "en" {
  162 |         return family.to_owned();
  163 |     }
  164 |     let exact = |n: &&LocalizedName| n.language.as_deref().is_some_and(|l| l.replace('_', "-").eq_ignore_ascii_case(&language.replace('_', "-")));
  165 |     let same_language = |n: &&LocalizedName| n.language.as_deref().is_some_and(|l| primary(l) == primary(language));
  166 |     info.localized.iter().find(exact).or_else(|| info.localized.iter().find(same_language)).map_or_else(|| family.to_owned(), |n| n.name.clone())
  167 | }
  168 | 
```

**What it does.** Upstream's `shown_name` reads the localized names through a parameter of its own.

**What the merge had to answer.** The fork's `FamilyCodec::shown` called the old signature.

**The answer.** The call passes `&info.localized`.

## 2. The classifier is private to the picker module

**The upstream code.**

`crates/font-manager/src/picker.rs`, lines 142-145:

```rust
  142 |     pub fn of(catalog: &Catalog, family: &str) -> Option<FontClass> {
  143 |         rank::class_of(catalog, family)
  144 |     }
  145 | }
```

**What it does.** The crate offers `FontClass::of` for a family already in the catalog, and nothing for a bare name.

**What the merge had to answer.** The fork's trials classify names that have no catalog entry.

**The answer.** `FontClass::classify` is exposed over `rank::classify`.

## 3. An icon button declares no Space

**The upstream code.**

`crates/ui-widgets/src/buttons/icon_button.rs`, lines 163-168:

```rust
  163 | impl Component for IconButton<'_> {
  164 |     const ROLE: Role = Role::BUTTON;
  165 |     const ALT_ROLES: &'static [Role] = &[Role::COMBO_BOX];
  166 |     const PARTS: bool = false;
  167 |     const SHOWS_LABEL: bool = false;
  168 |     const OVERLAYS: bool = false;
```

**What it does.** Upstream's icon button leaves `KEYS` at the default, so its activation keys are the pointer's alone.

**What the merge had to answer.** Upstream's new menu trials press Space on a focused icon button that opens a menu; the fork's rule is that Space belongs to the canvas unless a control asks.

**The answer.** `IconButton` asks for Space, and the menu opens from the keyboard as a click would.

## 4. Space is routed straight to the activation

**The upstream code.**

`crates/ui-contract/src/runtime/keys/route.rs`, lines 808-810:

```rust
  808 |         PressRoute::Activate => route_activate(ctx, rec, routed, activated),
  809 |         PressRoute::Close(layer) => overlay::request_close(ctx, layer, overlay::CloseReason::Requested),
  810 |         PressRoute::Intent(intent) => push(routed, id, intent, false),
```

**What it does.** Upstream activates a stop on the Space press itself.

**What the merge had to answer.** The fork activates on the release, and keeps the stop whose Space is down so a late release still reaches it.

**The answer.** `PressRoute::Space` and `space_held` carry that rule.

## 5. The prompt sets its layer role by hand

**The upstream code.**

`crates/ui-panels/src/dialog/prompt.rs`, lines 260-264:

```rust
  260 |     pub(super) fn layer_role(mut self, role: LayerRole) -> Self {
  261 |         self.layer_role = role;
  262 |         self
  263 |     }
  264 | 
```

**What it does.** Upstream's `Prompt::layer_role` stores its argument behind a hand-written setter.

**What the merge had to answer.** The design lint's `props/hand-setter` asks the field to declare itself and the derive to write the setter; upstream's tree fails the workspace check on it.

**The answer.** The field is `#[prop]` and the hand setter is gone.

## 6. A native menu driver reads the clock and asks for a frame

**The upstream code.**

`crates/menu/src/mac.rs`, lines 157-160:

```rust
  157 |         let attempt = self.retry.attempt(&spec, Instant::now, || {
  158 |             let rebuilt = native.apply(&spec);
  159 |             if rebuilt {
  160 |                 *lock(live) = targets;
```

**What it does.** Upstream's native driver passes `Instant::now` and calls `request_repaint_after` for its retry.

**What the merge had to answer.** The lint's `contract/motion` asks for the runtime's motion clock; a native menu driver has no frame of its own and the runtime exposes no clock to it.

**The answer.** Both sites are registered exemptions with their reasons.

## Correction, 2026-11-27: `crates/menu/src/mac.rs`

The lines quoted above are the round's; upstream `76322b729` landed later and this is the code
the tree carries now (line numbers of the current file):

```rust
157:         // design: allow[contract/motion](the AppKit menu rebuild retry is a native timer outside any pass, not a motion clock; the runtime clock is a web_time Instant, not std, on wasm)
158:         let attempt = self.retry.attempt(&spec, Instant::now, || {
169:             craft_ui::runtime::frame::request_repaint_after(&self.ctx, wait.as_secs_f64());
```

The fork's second marker and its `mac.toml` entry are gone; upstream's single entry in
`design/exceptions/craft-menu/crate.toml` covers line 157.

