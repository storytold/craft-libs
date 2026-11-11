# Writing a component

A component is a Rust type on the contract in `craft_ui::contract`. It supplies its own parts and look; the library runs one show pipeline for it: id, style, sizing, focus, pointer sensing, state, value binding, commands, popups, accessibility, mirroring and the shared test cases. Nothing in a component calls egui's behavioural APIs.

## File layout

One file per component in its family module: `<family>/<component>.rs`, for example `choice/radio.rs`. The file holds the type, its style, its words (`words!`) and a `stories` submodule:

```rust
#[cfg(feature = "testing")]
mod stories {
    use super::*;
    use craft_ui::testing::{StoryData, StoryOutput, story};

    #[story(Block, covers = [Default])]
    fn block(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
        Block::new(CELL).show(ui).into()
    }
}
```

Tests that need more than the shared cases go in `crates/ui/tests/it/<family>/` as `#[craft_ui::testing::trial]` functions.

A family trial drives frames through `Fixture` (`Fixture::new` or `Fixture::story`, then `click`, `drag`, `key`, `by_label`, `part` and the other helpers), the kit's one frame driver. It writes no click or drag helper of its own.

## A new crate with components

A crate that holds contract types has its own `testing` feature, enables it on itself, and runs the suite from one integration binary:

```toml
[features]
testing = ["craft-ui/testing"]

[dev-dependencies]
my-crate = { path = ".", features = ["testing"] }

[[test]]
name = "it"
path = "tests/it/main.rs"
harness = false
```

The crate's `testing` module calls `craft_ui::testing_registered!()` once, which writes `registered()` for the package. The binary's `main` collects the `#[trial]` functions and adds `suite::trials(env!("CARGO_PKG_NAME"), &my_crate::testing::registered())`, as `crates/ui/tests/it/main.rs` does. The derive writes registrations and stories only under `feature = "testing"`, so release builds carry none.

## Shapes

The shape is a fact of the type, set by what the caller supplies:

| Shape | Declare | Caller supplies | Shown with |
|---|---|---|---|
| Leaf | `#[derive(Component)]` | nothing | `Show::show` |
| Container | `#[component(container)]` and a `#[container] ContainerProps` field, then `impl Container` | a body closure | `ShowContainer::show` |
| Collection | `#[component(collection)]` and a `CollectionProps` field, then `impl Collection` | an item source and an item closure | `ShowCollection::show` |

A type that only assembles components has no pipeline. It is a pattern and implements the `Pattern` marker trait.

The show traits are sealed. A component never has an inherent `show`; `contract/shadow` rejects one.

## The type

```rust
#[derive(Component, Props)]
pub struct Badge {
    #[common]
    common: Common,
    /// The count shown beside the label.
    #[prop]
    count: Option<u32>,
    /// Working: shown dimmed.
    #[prop(flag)]
    busy: bool,
}
```

- `Common` holds what every component has: label, id, variant, size, enabled, read-only, selected, tooltip. Its setters come from `Props`. A component adds no `enabled`, `id_salt` or `tooltip` field of its own (`props/*`).
- `#[prop]` fields become setters. A `#[prop]` field is not a size in points (`f32`, `Vec2`, `Px`): sizes come from the style, a token step or the content (`props/point-size`). A number that is not a size, such as an opacity, declares `#[prop(quantity = "opacity")]`.
- The derive writes `<Type>StyleKey`, the setters and the registration that lets the suite find the type. A type that implements `Component` by hand registers with `register_contract!`.

## The trait

`impl Component for Badge` states the type's facts and implements only the hooks it needs:

| Item | Purpose |
|---|---|
| `const ROLE: Role` | The AccessKit role. |
| `type Style`, `fn style(cx, key)` | The look, built from style parts. |
| `type Output` | What `show` returns: `Response`, `EditResponse<T>`, a collection response. |
| `fn sense(&self) -> Sense` | The default follows the shape; override to add or remove click, drag and focus. |
| `fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2` | The content size. Padding and minimum size are added by the pipeline. |
| `fn parts(&mut self, rect, cx: &mut PartsCx<'_, Self>)` | Named sub-rects with `PartKey`, `Sense` and `HitShape`. |
| `fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>)` | Content paint through `PaintUi` and `Draw`. |
| `fn content_env(&self, cx, values: &mut AppValues)` | App values set for the body or items. |
| `fn finish(self, cx: Cx<'_, Self>) -> Self::Output` | The output. |

A component is shown through a parent's `show`, never by allocating a rect itself: `contract/allocate` rejects `allocate_*`, `ui.interact` and `ui.put`.

### Parts and faces

`cx.part(KEY, rect)` registers a part. `ui.face(KEY, Other::new())` paints another component as the look of a part, so a composite reuses the look of the component it wears. A part that only a pointer reaches states why keyboard users do not need it: `.pointer_only("Enter activates the component")`.

### Values

A value component's `Output` is `EditResponse<T>`. It binds with `Bind<T>` and reports through `cx.propose`, `cx.commit` and `cx.adopt`; each gesture ends in one `EditEvent::Commit`. Components with `Input::Editor` own their caret through `ImeSurface`.

## Looks

A style is a plain struct deriving `ComponentStyle`:

```rust
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
pub struct BadgeStyle {
    #[style(metrics)]
    metrics: Metrics,
}
```

`#[style(metrics)]` marks the `Metrics` field (padding, minimum size, gap); `#[style(surface)]` and `#[style(focus)]` mark the surface and focus-ring parts. The style function reads semantic tokens and `BySize` tables through `StyleCx`. It does not match on state: state colours come from `States`, and `style/*` rejects colour picks from response state, size arms and per-instance setters.

An app changes a look with `Looks::style::<Badge>(f)` before `install`; the replacement is frozen at install. A variant is a `const CustomVariant` with a crate-prefixed id and a look function over semantic tokens; a variant describes itself and needs no registration.

## Tokens

A component reads only semantic tokens. It has no tokens of its own and no unnamed size literals. A crate's own concepts go in a `#[derive(TokenGroup)]` struct whose `derive(tokens)` function gives defaults from the resolved semantic tokens; a theme overrides them under the group's `PATH`.

## Stories

A story shows one registered type in one state. `#[story(Type, covers = [..])]` registers it. The probe of the type lists the states it must show:

| Type has | Required stories |
|---|---|
| any | `Default` |
| a value output | `Mixed`, `Invalid` |
| `Input::Text` or `Input::Editor` | `LongText` |
| `#[component(overlay)]` | `Open` |
| items | `Empty`, `One`, `Many` |

A value a story edits comes from `StoryData::value(name, init)`. The suite fails when a required state has no story. `Drive::Auto` picks how the suite changes the value; a story names another drive (`Drive::Drag(Grip::part(HANDLE.at(i)).toward(vec2(1.0, 1.0)))`, `Drive::Pick { row }`, `Drive::Custom(f)`) only when the default does not fit; a drag of a part of the subject is a `Drive::Drag`, which also gets the run cancelled by Escape, never a `Drive::Custom`. The runner makes the disabled, read-only, right-to-left and narrow-room variants of each story itself, so a story shows only the type in its state. The cases the runner runs are the `Case::new` entries in `crates/ui/src/testing/suite.rs` and its `suite/` modules: required states, shows, access, tab, values, mixed, activation, text size, motion, language, text input, and the container and collection cases for the shapes that have them.

Suite rules:

- Drawing code changes no state and sends no request. The suite's `measure-pure` case runs each story's `show` twice per pass (a measuring run first, then the real one) and again on rerun passes, which an edit's no-input rerun adds. It fails when either changes story values or sends a command, announcement, toast or request. State changes belong in event branches (`if out.clicked()`, `for edit in out.edits`) or before the container.

A type that cannot meet a shared case lists it, with a reason:

```rust
#[component(exempt(case = "face-only", reason = "A face of the title row, shown only inside it."))]
```

Each exemption is an entry of kind `probe` in `design/exceptions/<crate>/<family>.toml`, with the rule `probe/exempt/<case>` and the file that declares it. A case is a case name of the runner, `face-only` (a type shown only inside another type's stories) or `state:<Required>` (one required state the type cannot show, such as `state:Mixed`). `craft-design-lint structure` reads the `exempt(..)` declarations from the sources, rejects one without a reason, and fails when a declaration has no entry or an entry lists more than exist; the count cannot rise.

## Seeing and checking the result

- `cargo run -p craft-ui-gallery` shows every registered story in both directions and both control sizes, in light or dark, with a filter.
- `cargo test -p craft-ui --features testing --test it` runs the shared cases and the trials.
- `cargo run -p craft-design-lint -- structure` checks the layer map, imports, exceptions and clones; `check` reports the source rules.
- `cargo run -p craft-ui-gallery --bin components-index` rewrites `docs/components-index.md`, the list of what exists. Read it before writing a type, to avoid a second copy of one that exists.

## Decisions an author meets

- **One press per pass.** A pass carries at most one press. The press is classified once from the hit list, and a focused text field commits in the press pass, so a click handler elsewhere sees the committed text.
- **Popups close on an outside press by default.** `Outside::Close` closes the layer and the press reaches the widget below; `Outside::Keep` ignores the press. A dialog and a floating panel keep.
- **A popup or layer body sees the opener's environment.** The overlay host installs the opener's captured values (tokens, size, app values) and resets the layer-local ones: the arena, the transform, the row, the in-scroll flag and the ink.
- **`content_env` reaches the body and the items only.** A child shown by `PaintUi` inherits the composite's environment with the composite's resolved `enabled` and size, not the values `content_env` set.
- **A new body measures zero once.** A container with no `LayoutMemory` entry measures zero in its first pass, records its real size, and the pass is discarded once. A `measure` that returns the content size avoids the discard.
- **A roving group is one Tab stop.** A container with `ContainerProps.roving` puts its group id in the body environment; its members join that stop, arrows move between them, and the group's pick arrives through `activated_member()`.
- **Reading a text outcome.** `PartsCx::text` returns `&TextOutcome<V>`: an idle outcome for a part first registered in this pass. A key that was never registered panics, like `part`.
- **Memory and animation share an owner.** An animation slot is keyed by `("anim", name)` under the component's id and goes with the owner's entries on eviction. A press entry is dropped when its press ends.
- **A face registers no parts.** A component worn by `ui.face(KEY, ..)` runs its own `access` for the wearing part's node. Its `paint` computes what its parts would hold from the value, because the part registry is not read in face mode.
- **A story output has one edit kind at most**, because `EditResponse` holds one event.
- **A content painter's context owns its painter.** `ContentPainterCx` has no lifetime parameter; its painter is clipped to the component's rect.
