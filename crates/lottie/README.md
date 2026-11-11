# craft-lottie

Lottie (Bodymovin) JSON: the document as the file has it, and the encodings its values use. Bytes
in, a neutral model out, and back. Which layer, shape, effect and text feature becomes what in an
application's document is the application's work.

No file access: callers pass bytes, so the crate builds for wasm32.

## The document model

`Animation::read(bytes)` gives the document; `to_json` and `to_vec` write it. The model is the
lottie-spec 1.0 object model, one struct per object and one field per key:

- `Animation`: version, frame rate, frame range, size, `assets`, `layers`, `markers`, `fonts`,
  `chars`, `meta`.
- `Asset`: an image (`dir`, `path`, `embedded`) or a precomposition (`layers`).
- `Layer`: the keys every layer has (name, `index`, `parent`, timing, blend mode, `transform`,
  `masks`, `effects`, matte keys) and `content`, by `ty`: `Precomp`, `Solid`, `Image`, `Null`,
  `Shape`, `Text`, or `Other` for a type the model has no struct for.
- `Shape`: the keys every shape item has and `kind`, by `ty`: group, rectangle, ellipse, polystar,
  path, fill, stroke, gradient fill and stroke, transform, trim, round corners, repeater, merge,
  pucker and bloat, twist, zig-zag, offset path, or `Other`.
- `Transform`, `Mask`, `Effect` (an effect and its values), `Marker`, `Fonts`, `Font`, `Char`.
- `TextData`: the documents over time (`TextDocument`), range animators (`TextRange`,
  `TextSelector`, `TextStyle`), alignment and path options.

Every field is an `Option`: files leave keys out, and a writer sets what it needs. An animatable
property is an `Animated`, the property's JSON; `Animated::read` gives its value or keyframes (see
below). What a value means depends on the property, so the model does not decode values.

Nothing a file holds is dropped because the model does not know it. A key the struct does not
name, and a key whose value has another type than the struct expects, stay in the struct's `extra`
and are written back. A layer or shape of an unknown `ty` keeps its `ty` and its keys. So a
document read and written back is the same JSON, with three exceptions: an entry of a list that is
not an object where objects are expected is left out, a whole number written `30.0` comes back
`30`, and a flag written `1` or `0` where the schema has a boolean comes back `true` or `false`.

`Limits` bounds what is read: bytes of JSON (256 MiB), nesting (128 levels, which is also where
`serde_json` stops), layers of the document and its precompositions together (100,000), assets
(20,000) and shape items (2,000,000). Passing one is `LottieError::TooLarge`. No reader panics on
any input.

## The encodings

These work on `serde_json` values, for the values the model leaves as JSON.

| Part | Read | Write |
|---|---|---|
| Document JSON and header (`w`, `h`, `fr`, `ip`, `op`, `nm`) | `parse_document`, `Header::read` | the application builds the JSON |
| Animatable property (`{a, k, x}`) and keyframes | `read_prop` | `write_prop`, `write_keys` |
| Ease of a segment (`o`, `i`) and speed / influence | `Tangents::read`, `to_speed` | `Tangents::from_speed` |
| Bezier path (`{c, v, i, o}`) | `Bezier::from_json` | `Bezier::to_json` |
| Gradient stop list (`g.k`) | `Gradient::from_flat` | `Gradient::to_flat` |
| Numbers, colours, `#rrggbb` | `nums`, `color`, `hex_color` | `round6`, `to_hex` |
| Embedded images (`data:` URIs, RFC 4648 base64) | `base64::data_uri` | `base64::to_data_uri`, `image_mime` |

Not here: dotLottie (`.lottie`), which is a ZIP archive around the same JSON, and layer styles
(`sy`), which lottie-spec 1.0 does not define (they stay in a layer's `extra`).

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

Nothing is indexed by a number from the file, and no reader panics.

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
LottieFiles format documentation); no player or exporter source was used. The encodings come from
EffectCraft's `effectcraft-lottie` crate; the document model is written here from the same
description. The four fixtures in `tests/fixtures` are hand-written
JSON (original work), not exported from any product.
