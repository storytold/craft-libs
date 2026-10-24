# UI shadow implementation and manual acceptance

The UI implementation is a local pilot. Visual acceptance is assigned to the maintainer;
no app screenshots or gallery PNGs have been accepted by the implementation agent.
Canvas CPU/GPU shadows are a separate follow-up.

## Shared API

`craft_ui::theme::shadow` accepts slices of `egui::Shadow`. `shapes` returns shadow shapes;
`paint` respects the supplied painter's opacity and clip rectangle. Call it before painting
the surface. `visual_bounds` includes the surface and all visible shadow margins; it does not
expand clipping or input bounds. Units are logical points, and blur is egui penumbra width.

`Elevation::{Low, Medium, High, Modal}.layers(color)` preserves PhotoCraft's four-layer
reference profiles, including hue. `floating_toolbar(color)` supplies a quieter two-layer
candidate. Apps choose which recipe and color a surface uses. Neither chooses z-order,
modal behavior, a theme, fonts or user preferences.

`show_frame(ui, frame, layers, content)` owns a paint slot before its content. It replaces
the frame's ordinary shadow, preserves its layout/margins and paints surface fill/border
after the shadows. An empty layer slice removes its shadow. Use this for custom frames.
Owned paint slots are excluded from the automatic container adapter, even if a deliberately
single-layer recipe matches a registered signature.

For built-in egui 0.36 containers, `install(ctx, rules)` enables an end-of-pass adapter.
Each `FrameShadow` rule registers the complete original single-shadow signature and its
replacement layers. The adapter considers only known non-background Areas, requires a frame
rectangle equal to the Area bounds, and matches exactly `[shadow_rect, frame_rect]` against
the registered shadow geometry, styling and inherited opacity. Ambiguous candidates, plain
rounded rectangles, other vectors and unsupported structures retain their original painting.
Scrims, surface styling, clip rectangles and layer transforms remain intact.

`set_layer_shadows` selects an override for a known container in the current pass. The next
pass clears overrides. Its boolean result reports whether the adapter is installed, not
whether that particular frame structure was supported. Reinstalling replaces theme rules;
it does not stack plugins. The matcher is specific to egui 0.36 and must be retested when
updating egui. Faded shadows retain the inherited opacity, applied once.
After rendering, `frame_status(ctx, layer)` reports `Applied` or `Unchanged` for inspected
Areas; `None` means the Area was not inspected. Use this diagnostic when inventorying fallbacks.
Per-pass state is scoped to the viewport so child windows do not clear a parent's overrides.
Rules are checked in registration order. Signatures differing only in alpha can become
indistinguishable during a fade; use equivalent normalized recipes for such signatures, as
FigCraft does for its light/dark scopes. Use explicit owned painting or a known-layer override
when different roles need different falloff. Alpha reconstruction is limited to 8-bit color
precision; tinted or otherwise unrecognized source shadows keep the single-layer fallback.

## Consumer coverage and limitations

| Consumer | Implementation |
| --- | --- |
| PhotoCraft | Shared four-layer recipes; registered popup/window and custom floating-frame signatures. Existing per-container calls select modal/high overrides. Additional built-in popups use registered defaults. |
| FigCraft | Direct two-layer toolbar; owned frames for menus, Actions palette, comment popovers, canvas banners and prototype editor; direct low-elevation annotation shadows. Built-in containers use registered light/dark frame signatures, including local dark scopes. |
| LightCraft | Registered popup/window signatures for Charcoal, Midnight, Silver and Paper. Nested menus and ComboBoxes use the same adapter. |
| Shared docking / print controls | Their built-in popups inherit the host's registered adapter. Installing the module is opt-in; existing hosts keep their old rendering unless they install it. Floating docking-panel backgrounds remain unchanged. |

Containers whose Area and frame bounds differ (for example, an outer margin or additional
content outside the frame), rotated frame shapes, unregistered local shadow signatures,
and changed egui paint structures use the original single-layer fallback. Tooltips use the
popup recipe when their frame matches. Document drop-shadow effects are never registered.
PhotoCraft canvas geometry, blur and theme/visibility exceptions remain application-owned.

The three application manifests temporarily patch `craft-ui` to `../craft-libs/crates/ui` so
this four-repository workspace can be built and reviewed before publication. Their lockfiles
reflect that path patch. Keep the sibling checkout layout for the pilot. After acceptance,
publish the shared commit, update each immutable `rev`, remove the temporary patch and
regenerate the lockfiles. Do not ship a manifest requiring the local sibling checkout.

## Automated validation (2026-10-13)

- Formatting passes in all four repositories.
- `cargo test --workspace --lib` passes in craft-libs, including 88 passing craft-ui tests and
  the 10 shadow regression tests. The optional CPU profile is ignored by the ordinary gate
  and was run separately. One existing filesystem ACL test is also environment-gated.
- Targeted application tests pass: 49 PhotoCraft theme-related tests, the FigCraft toolbar
  regression (light/dark, 1x/2x), and 11 LightCraft theme/appearance tests.
- Native UI checks, application layer checks and wasm32 UI checks pass for all three apps.
  craft-ui's own wasm check also passes. Existing optional-font and wasm-only unused-code
  warnings are unrelated to this change.
- Strict workspace/all-target Clippy passes in craft-libs. A small existing docking match
  was mechanically rewritten to satisfy the current `collapsible_match` lint.
- Strict application UI Clippy encounters baseline lints: PhotoCraft's analysis/canvas code
  (and raw/cr3 when linting dependencies), FigCraft's branch/comments/dev-compare/style-picker/
  versions code and configured unknown lint, and LightCraft's `panels/detail.rs:782` boolean.
  These application expressions were not changed for the shadow work. Compatibility checks
  use `--no-deps -- -D warnings` with only the relevant baseline categories allowed on the
  command line: `-A clippy::collapsible_match` for PhotoCraft, the same plus
  `-A unknown_lints` for FigCraft, and `-A clippy::nonminimal_bool` for LightCraft. No lint
  suppressions were added to application source. These checks are not a claim that the full
  existing application CI is clean.
- The gallery example is checked for compilation and PNG generation; visual review is left
  to the maintainer. Performance profiling against real application workloads remains a
  release acceptance step; no measured frame-time budget is claimed here.

The CPU-only fixture uses 16 simultaneous Areas with eight text rows each, 1x scale, eight
warm-up frames and 100 measured samples. It includes egui UI construction and tessellation,
without GPU rendering. In the default unoptimized test profile, two rounds (alternating order)
measured median single-shadow times of 3.014/2.953 ms and layered times of 3.831/3.790 ms:
an extra 0.817/0.837 ms (+27.1/+28.4%). Vertices increased from 18,304 to 21,760 (+18.9%).
All 16 Areas were verified as adapted. These are fixture results, not production frame times.
Reproduce with:

```bash
cargo test -p craft-ui --lib profile_shadow_adapter -- --ignored --nocapture
```

## Maintainer visual checks

Run these from the repository named in each step. Use synthetic documents and retain
before/after screenshots locally. The implementation agent does not perform visual acceptance.

1. In craft-libs, generate the comparison gallery:

   ```bash
   cargo run -p craft-ui --example shadows -- /tmp/craft-shadows
   ```

   Inspect its light/dark 1x/2x PNGs. Compare the single-layer, PhotoCraft and toolbar recipes
   over plain and patterned backgrounds. Look for smooth falloff, unwanted rings and clipping.

2. In FigCraft, run `cargo run -p figcraft` and create a blank design document. Check the bottom toolbar on empty and busy
   canvases in both themes, including normal, vector-edit, shape-builder and view-only modes.
   The stepped gray rings should be gone; its outline should remain readable. Check flyouts,
   comment threads/drafts, Actions, canvas banners and the prototype interaction editor.
   Confirm tools, focus and keyboard navigation still work. Inspect the bottom edge at 1x/2x
   and small window sizes for clipped bands or painting over neighboring chrome.
3. In PhotoCraft, run `cargo run -p photocraft`. Compare menus/submenus, ComboBoxes, dialogs,
   floating brush/type/gradient panels and the command palette against the source branch.
   Check light/dark Studio and Pro variants, plus Classic's flat treatment. Inspect modal
   scrims and opening/closing fades; confirm the canvas shadow retains its existing behavior.
4. In LightCraft, run `cargo run -p lightcraft`. Check menus, nested menus, ComboBoxes and
   dialogs in Charcoal, Midnight, Silver and Paper. Inspect text/surface separation, fades,
   edge clipping and dismissal. Check docking/printing popups where available.
5. In all apps, switch themes with overlays open, drag/resize floating panels, repeat open/close operations, and inspect
   1x/2x scaling. Note any remaining single-layer fallback that needs a dedicated adapter.

Record pass/fail per application, theme, scale and surface, with a screenshot path for each
failure. Visual acceptance and release pinning remain pending until that review is complete.
