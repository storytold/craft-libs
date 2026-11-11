# craft-lottie

Lottie (Bodymovin) JSON: the wire layer. A Lottie document is JSON, and most of what is hard about
it is a handful of encodings that every layer, mask and shape reuses. This crate reads and writes
those on `serde_json` values. It has no document model of its own yet: which layer, shape, effect
and text feature becomes what in an application's document is the application's work.

No file access: callers pass bytes, so the crate builds for wasm32.

| Part | Read | Write |
|---|---|---|
| Document JSON and header (`w`, `h`, `fr`, `ip`, `op`, `nm`) | `parse_document`, `Header::read` | the application builds the JSON |
| Animatable property (`{a, k, x}`) and keyframes | `read_prop` | `write_prop`, `write_keys` |
| Ease of a segment (`o`, `i`) and speed / influence | `Tangents::read`, `to_speed` | `Tangents::from_speed` |
| Bezier path (`{c, v, i, o}`) | `Bezier::from_json` | `Bezier::to_json` |
| Gradient stop list (`g.k`) | `Gradient::from_flat` | `Gradient::to_flat` |
| Numbers, colours, `#rrggbb` | `nums`, `color`, `hex_color` | `round6`, `to_hex` |
| Embedded images (`data:` URIs, RFC 4648 base64) | `base64::data_uri` | `base64::to_data_uri`, `image_mime` |

Not here: the layer, shape, mask, effect and text item layouts (they are still in EffectCraft's
`effectcraft-lottie`, written against its project model), and dotLottie (`.lottie`), which is a
ZIP archive around the same JSON.

## What the readers accept

Files in the wild come from many Bodymovin versions and other exporters, so the readers are
tolerant where players are:

- a property that is a bare value (`"o": 100`), a property without `a` whose `k` is a key list,
  and a key value wrapped once more (`[[x, y]]`);
- keys before 5.0: a segment ends with `e` and the last key has no `s`; the value of a key is
  then the `e` of the key before;
- ease tangents whose `x` and `y` are a number or a list, per dimension; a missing `x` is a
  third of the way and a missing `y` is linear;
- a path without `c`: Bodymovin 3.x and early 4.x keep the flag outside the path (`cl` on a
  mask, `closed` on an `sh` item), so the caller passes it; tangent lists shorter or longer than
  the vertex list;
- colours written 0..=255 instead of 0..=1;
- a gradient list without alpha pairs, and a colour count larger than the list.

Input is hostile: nothing is indexed by a number from the file, JSON nesting is bounded by
`serde_json` (128 levels), and no reader panics. Sizes are those of the JSON the caller parsed;
bound the file size before calling.

## Ease

A Lottie keyframe holds the start value of a segment and the segment's normalised cubic Bezier
ease: `o` (out of this key) and `i` (into the next), each with `x` (time fraction) and `y` (value
progress fraction), per dimension. After Effects' temporal ease is speed (units per second) and
influence (fraction of the segment). For a segment of duration `d` and value change `Δv`:

```text
o.x = influence_out          o.y = speed_out · influence_out · d / Δv
i.x = 1 − influence_in       i.y = 1 − speed_in · influence_in · d / Δv
```

`Tangents::from_speed` and `Tangents::to_speed` are this formula and its inverse. A spatial
property eases along its motion path: one dimension, with `Δv` the arc length, which the caller
computes.

## Sources and clean room

Implemented from the public Lottie format description (the lottie-spec 1.0 schema and the
LottieFiles format documentation); no player or exporter source was used. The code comes from
EffectCraft's `effectcraft-lottie` crate. The four fixtures in `tests/fixtures` are hand-written
JSON (original work), not exported from any product.
