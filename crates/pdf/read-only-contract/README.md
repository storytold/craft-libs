# Independent read-only contract

This nested workspace tests the shipping `craft-pdf/read` graph separately from the parent
workspace, whose facade dev-dependencies include an interpreter. Its sole normal dependency is
`craft-pdf` with defaults disabled and `read` enabled. The local syntax patch is required to
exercise the vendored decode-limit and incremental-Flate behavior. No interpreter, rasterizer,
writer, or image codec dependency is declared here.

The same three assertions run natively and under `wasm-bindgen-test`: valid Flate page opening
and complete incremental consumption; independent per-document ordinary stream limits including
zero and a tighter per-call limit; and the caller input limit on borrowed/shared opening. The
17-byte synthetic compressed payload expands to 512 known bytes. The only target-specific dev
dependency is the WASM test harness.

From the repository root:

```sh
cargo test --manifest-path crates/pdf/read-only-contract/Cargo.toml --locked
cargo test --manifest-path crates/pdf/read-only-contract/Cargo.toml --locked --target wasm32-unknown-unknown
cargo tree --manifest-path crates/pdf/read-only-contract/Cargo.toml --locked --edges normal,features
```

Use a matching `wasm-bindgen-test-runner` through `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER` for
actual WASM runtime execution. The native normal dependency graph must contain only the
`craft-pdf/read` path and its syntax/std dependencies; `hayro-interpret`, `hayro`, `krilla`,
`hayro-ccitt`, `hayro-jbig2`, `hayro-jpeg2000`, `zune-jpeg`, and the syntax `images`/`unsafe` features
must be absent. Keep the fixture's own lockfile for reproducibility.
