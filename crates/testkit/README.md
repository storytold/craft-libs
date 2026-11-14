# craft-testkit

Test support shared by the Craft apps and the craft-libs crates. Apps and craft-libs crates take it
under `[dev-dependencies]`:

```toml
[dev-dependencies]
craft-testkit = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }
```

## `bin`

`bin` is native only: it is not built for `wasm32`.

`bin_path!("my-program")` expands in the calling integration test and returns the path of the
package's binary `my-program`. The name is the binary target's name with its hyphens. It reads, in
order:

1. `NEXTEST_BIN_EXE_my_program` (nextest 0.9.113 and later),
2. `NEXTEST_BIN_EXE_my-program`,
3. `CARGO_BIN_EXE_my-program` when the test runs (Cargo 1.94 and later, nextest 0.9.130 and later),
4. `CARGO_BIN_EXE_my-program` as Cargo set it when it compiled the test.

A value counts only when it is an absolute path to an existing regular file; an empty, relative or
missing path, or a folder, is passed over. A test run from an extracted nextest archive therefore
starts the extracted binary, and not the build machine's path that Cargo compiled into the test
([nextest: archiving](https://nexte.st/docs/ci-features/archiving/),
[nextest: environment variables](https://nexte.st/docs/configuration/env-vars/),
[Cargo: environment variables](https://doc.rust-lang.org/cargo/reference/environment-variables.html)).
When no variable names a file, the macro fails the test with a message that lists each variable,
its value and why it was passed over. `.exe` is never added. `resolve_binary` returns that message
as a `BinPathError` instead.

`run(path, args, env, max_output)` starts the program with stdin connected to nothing and returns
its `Output`. The child inherits the test's environment with the `env` pairs set on top; the test's
own environment is not changed. stdout and stderr are read at the same time, each into at most
`max_output` bytes. A stream that passes the limit stops the child, waits for it and returns an
`io::Error` holding `OutputLimitExceeded`. `DEFAULT_MAX_OUTPUT` is 1 MiB for each stream, a value of
this crate. A non-zero exit status is returned in the `Output`. `run` has no time limit.

```rust
use std::ffi::OsString;

#[test]
fn help_prints_usage() {
    let output = craft_testkit::bin::run(
        &craft_testkit::bin_path!("soundcraft-cli"),
        &[OsString::from("--help")],
        &[(OsString::from("SOUNDCRAFT_NO_PREFS"), OsString::from("1"))],
        craft_testkit::bin::DEFAULT_MAX_OUTPUT,
    )
    .unwrap();
    assert!(output.status.success());
}
```

## Origin

| Module | Source | Path | Commit |
|---|---|---|---|
| `bin` (`bin_path!`, both nextest spellings) | storytold/soundcraft | `apps/soundcraft-cli/tests/common.rs` | `2688ccac88` |
| `bin` (`bin_path!`, runtime `CARGO_BIN_EXE_`) | storytold/vectorcraft | `apps/vectorcraft-cli/tests/it/main.rs` | `bada8e1710` |
| `bin` (lookup tests `cli_binary_prefers_relocated_runner_path`, `cli_binary_defaults_to_cargo_path`) | storytold/texturecraft | `apps/texturecraft-cli/tests/cli.rs` | `386630ee85` |
| `bin` (test `cli_executable_is_resolved`) | storytold/filmcraft | `apps/filmcraft-cli/tests/it/main.rs` | `e4ab3c6a99` |
| `bin` (`run`: null stdin, environment on top of the inherited one) | storytold/soundcraft | `apps/soundcraft-cli/tests/it/behaviour.rs` | `2688ccac88` |

## Migration

| Consumer | Replaces | Source commit | State |
|---|---|---|---|
| SoundCraft | `apps/soundcraft-cli/tests/common.rs` lookup, `tests/it/behaviour.rs` runner | `2688ccac88` | Planned |
| VectorCraft | `apps/vectorcraft-cli/tests/it/main.rs` lookup | `bada8e1710` | Planned |
| CADCraft | `apps/cadcraft-cli/tests/it/cli.rs` runner | `8e69890223` | Planned |
| DeckCraft | `apps/deckcraft-cli/tests/it/main.rs` lookup, `cli.rs` runner | `b0c77ebd6a` | Planned |
| FilmCraft | `apps/filmcraft-cli/tests/it/main.rs` lookup, `cli.rs` runner | `e4ab3c6a99` | Planned |
| GridCraft | `apps/gridcraft-cli/tests/it/cli.rs` runner | `f2044337c7` | Planned |
| PdfCraft | `apps/pdfcraft-cli/tests/it/help.rs` runner | `eead454105` | Planned |
| TextureCraft | `apps/texturecraft-cli/tests/cli.rs` lookup | `386630ee85` | Planned |
| WordCraft | `apps/wordcraft-cli/tests/it/help.rs` runner | `f8c66d0692` | Planned |
