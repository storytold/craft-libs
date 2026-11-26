# The upstream code behind the answers in craft-libs

The upstream lines each answer had to meet.

Every quotation is `git show 93c2e0825:<path>`, with that commit's line numbers.

## 1. `shown_name` takes the localized names, not the whole `Info`

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

**What it does.** Upstream's `shown_name` reads the family's localized names through a parameter of its own instead of reaching into the `Info` it was given.

**What the merge had to answer.** The fork's `FamilyCodec::shown` called the old signature; the call had to pass `&info.localized` and the old call no longer compiles.

**The answer.** The call site passes the localized names, which is the only change the fork needed.

## 2. The fonts' classification is private to the picker module

**The upstream code.**

`crates/font-manager/src/picker.rs`, lines 142-145:

```rust
  142 |     pub fn of(catalog: &Catalog, family: &str) -> Option<FontClass> {
  143 |         rank::class_of(catalog, family)
  144 |     }
  145 | }
```

**What it does.** The crate offers `FontClass::of`, which classifies a family already in the catalog, and nothing that classifies a bare name.

**What the merge had to answer.** The fork's type-tool trials classify names that have no catalog entry (a name from a document), so they need the classifier itself.

**The answer.** `FontClass::classify` is exposed over `rank::classify`, and the trial uses it.

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

**What it does.** Upstream's icon button leaves `KEYS` at the component default, so its activation keys are the pointer's alone.

**What the merge had to answer.** Upstream's new menu trials press Space on a focused icon button that opens a menu and expect the menu to open; the fork's rule is that Space belongs to the canvas unless a control asks for it, so nothing opened the menu.

**The answer.** `IconButton` asks for Space (`KeyUse::of(Input::Pointer).with_space_activation()`), and the menu opens from the keyboard as a click would.

## 4. Space is routed straight to the activation

**The upstream code.**

`crates/ui-contract/src/runtime/keys/route.rs`, lines 806-808:

```rust
  806 |         PressRoute::Activate => route_activate(ctx, rec, routed, activated),
  807 |         PressRoute::Close(layer) => overlay::request_close(ctx, layer, overlay::CloseReason::Requested),
  808 |         PressRoute::Intent(intent) => push(routed, id, intent, false),
```

**What it does.** Upstream activates a stop on the Space press itself.

**What the merge had to answer.** The fork activates on the release - that is what a click does - and keeps the stop whose Space is down, so a release that arrives after the focus moved still reaches the stop that was pressed.

**The answer.** `PressRoute::Space` and `space_held` carry the fork's rule, and the router accepts a stop that asks for Space by name or among its activation keys.
