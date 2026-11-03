Read `AGENTS.md`.

New workspace crates must include `[lints] workspace = true` in `Cargo.toml` to inherit
`[workspace.lints]`; production crate roots must begin with the required crate-level lint attribute.
