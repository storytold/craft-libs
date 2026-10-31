# Frosted ribbon and panel backdrops

`craft_ui::backdrop` extracts the CPU sampling and blur from
[WordCraft #4019](https://github.com/storytold/wordcraft/pull/4019). It can serve ribbons, rulers,
floating palettes and inspectors without depending on a page renderer or an app's theme.

1. Capture the contributing content in the host. Build `Thumbnail::from_image` when its pixels
   change; select a block size appropriate to capture resolution (WordCraft uses about four
   logical points per cell).
2. Supply `BackdropLayer`s in screen coordinates and call `compose` for the covered area.
   Layers without captures use their fallback color, so first-frame rendering remains defined.
3. Keep a `BackdropTexture` per region/context and upload the composed image. Paint its full
   UV rectangle behind controls using normal egui clipping and apply a caller-chosen tint.

The output uses bilinear thumbnail samples and three separable box-blur passes (five cells
sideways/two vertically), intended for soft UI glass, not photographic filtering. Capture pixels
are premultiplied; they composite over the supplied paper's opaque RGB. Output is opaque and
the host's overlaid tint supplies translucency. No brightness threshold or dark/light alpha is
hard-coded. Choose tint for text contrast and provide an opaque fallback when transparency is off.

Inputs are validated before multiplication/allocation. Source images are capped at 16,777,216
pixels, thumbnails/output at 262,144 texels, output and uploaded texture sides at 1024. A long,
thin upload is rejected even when its total pixel count fits; the existing cache is preserved.
Invalid captures/areas return
`None`; invalid layer rectangles are ignored. Identical texture images skip uploads. Distinct
windows/contexts use distinct cache instances; dropping one releases its texture.

The host owns capture and invalidation, including changes while transparency is off. When the
effect is re-enabled, backfill missing/stale captures before using them. Keep captures out of
the composition being captured to avoid feedback. Consumer adoption and GPU framebuffer capture
are separate work; this module adds no renderer callback, global UI flag or document dependency.
