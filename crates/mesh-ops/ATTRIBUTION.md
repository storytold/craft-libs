# Attribution

`craft-mesh-ops` generates tangents with the `mikktspace` crate.

| Package | Authors | Source | License |
|---|---|---|---|
| `mikktspace` 0.3.0 | Benjamin Wasty, David Harvey-Macaulay, Layl Bongers | [crates.io](https://crates.io/crates/mikktspace/0.3.0), [gltf-rs/mikktspace](https://github.com/gltf-rs/mikktspace) | MIT OR Apache-2.0 (the package's `license` field) |

The package's `src/generated.rs` is a translation of Morten S. Mikkelsen's C implementation of
the MikkTSpace algorithm and carries that implementation's notice, reproduced here unchanged:

```text
Copyright (C) 2011 by Morten S. Mikkelsen

This software is provided 'as-is', without any express or implied
warranty.  In no event will the authors be held liable for any damages
arising from the use of this software.

Permission is granted to anyone to use this software for any purpose,
including commercial applications, and to alter it and redistribute it
freely, subject to the following restrictions:

1. The origin of this software must not be misrepresented; you must not
claim that you wrote the original software. If you use this software
in a product, an acknowledgment in the product documentation would be
appreciated but is not required.

2. Altered source versions must be plainly marked as such, and must not be
misrepresented as being the original software.

3. This notice may not be removed or altered from any source distribution.
```
