# craft

The craft libraries in one dependency. An app pins one revision of this crate and turns on features; everything is imported as `craft::<crate>`.

```toml
[workspace.dependencies]
craft = { git = "https://github.com/storytold/craft-libs", rev = "…" }

# crates/ui-egui/Cargo.toml
craft = { workspace = true, features = ["ui", "color", "draw", "imaging", "keymap", "platform-appearance"] }
```

## Always on

`craft::theme`, `craft::i18n`, `craft::host`.

## Features

- One feature per crate, named after the crate without `craft-`: `color` turns on `craft-color` and re-exports it as `craft::color`.
- `ui` turns on `craft-ui` (`craft::ui`), re-exports `egui` (`craft::egui`, the egui the library is built with) and forwards `ui` to every enabled crate.
- `screen-testing` implies `ui` and forwards to every enabled crate.
- Each crate feature is forwarded as `<crate>-<feature>` (`platform-appearance`, `text-hyphen`, `ui-window-frame`).
- A crate whose `ui` needs another family's `ui` turns that family's feature on (`draw` turns on `color`).

The `[features]` table in `Cargo.toml` is written by `tests/features.rs` from the member crates' manifests. After changing a member's features or adding a crate, run `UPDATE_FACADE=1 cargo test -p craft -- integration::features` and commit the result. A comment on a feature line marks a crate that does not build for `wasm32-unknown-unknown`.

Not in the facade, because they are tools or test harnesses: `craft-ui-gallery`, `craft-ui-macros` (reached through `craft-ui`), `craft-fonts-build` (a build-script helper), `craft-design-lint`, `craft-plate-test`.

## Family dependencies

`tests/families.rs` fails when one family crate's `ui` depends on another family crate's `ui`. `craft-color` is the base family that any family may use.

## egui fork

`craft::egui_fork::{GIT, REV}` name the egui fork the library is built with, and `check_lockfile(lock)` checks that an app's `Cargo.lock` takes every egui package from it. An app's xtask calls it in its `ci` and `check` commands. The app's workspace must also patch egui to this fork, since `[patch]` applies only at the workspace root.
