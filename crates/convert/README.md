# Craft Convert

`craft-convert` converts between two models that have the same shape but are separate Rust
types: a library's document model and the copy an app extends, or two versions of one schema.
It has no dependencies and builds for `wasm32-unknown-unknown`.

Going through a serialized form works, but it is slow, it links a serializer into the binary,
and a field only one side has is dropped without a word. Here the conversion is field by field
and the compiler checks it.

- **`Convert<T>`** takes the source by value and moves what it can; **`ConvertRef<T>`** reads
  it through a reference. Both are implemented for the primitives, `String`, `Option`, `Vec`,
  `VecDeque`, `Box`, `Rc`, `Arc`, the standard maps and sets, tuples up to eight members and
  arrays.
- **`convert_types!`** writes the impls for your structs and enums. A struct is taken apart with
  a pattern that names every field and built with a literal that names every field; an enum is
  matched variant by variant. **A field or variant added to either model is a compile error**
  until the declaration says what happens to it.

```rust
craft_convert::convert_types! {
    // Twins: by value and by reference, both ways.
    struct wire::Stop = app::Stop { at, color }
    enum wire::Fill = app::Fill { None, Solid(rgb), Gradient { stops, angle } }

    // One way, with the differences spelled out.
    struct wire::Size => app::Size {
        w => width,          // renamed
        h => height,
        dpi => _,            // only the source has it: dropped
        _ => scale = 1.0,    // only the target has it: this value
    }
}

let fill: app::Fill = wire_fill.convert();
```

A member can also take its own expression (`name = expr`), and a variant its own arm
(`Old(a) = expr`). The crate documentation lists every form.

## Two models you do not own

Rust lets a crate implement a foreign trait only for its own types. Both traits take a second
*marker* parameter for that: declare a unit struct, name it with `marker = Bridge;` at the top
of the macro, and the impls are yours to write. A marker implements `Floats`, which sees every
`f32` and `f64` on its way through; a mapping that stands in for a text format can note or
replace NaN and the infinities there.

## Limits

- Types are named without generic parameters.
- Tuple structs and structs with a private field are written by hand.
- An `Rc` or `Arc` converts to a new pointer: one owner is unwrapped, a shared value is cloned.
  Give a field its own expression where sharing has to survive.
- Members are walked one at a time: a struct or enum with more than about fifty members needs
  `#![recursion_limit = "256"]` in the crate that declares it.
