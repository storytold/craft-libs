# craft-host

The services every Craft library feature reads from the app: who the app is (`AppInfo`), the
catalogs and current language, settings sections, command words and background jobs. The app builds
one `Host` at startup and installs it on the egui context; features read it with `Host::of(ui)` and
never take services as parameters.

```ignore
let host = craft_host::Host::builder(craft_host::AppInfo { id: "photocraft", name: "PhotoCraft", version: "1.0" })
    .i18n(i18n)
    .settings(settings)
    .build();
host.install(&ctx); // feature `ui`
```

Every service has a default, so a test writes `Host::builder(app).install(ctx)`:

| Service | Trait | Default |
| --- | --- | --- |
| Settings sections | `SectionStore` | `MemorySections` |
| Command words | `Commands` | `NoCommands` (English labels) |
| Background work | `Jobs` | `ThreadJobs`; `InlineJobs` on wasm32 |
| Catalogs | `craft_i18n::I18n` | none (English) |

`queue::JobQueue` runs prioritised background work with one job per key on the host's `Jobs`.
Stories and tests step it with `ManualJobs`, which keeps each job until `run_next` runs it.

`Host::install_scoped` installs a host until the returned `HostScope` drops, then puts back the
one installed before. A story swaps in a fake service this way for the time it draws.

## Boundaries

- Depends on `craft-i18n`, `serde` and `toml` only. Implementations depend on this crate:
  `craft-settings` implements `SectionStore`, `craft-keymap` implements `Commands`.
- Feature `ui` adds egui for `install` and `of`. It never depends on `craft-ui`.
- A settings section is a serde struct saved as one TOML table under `Section::KEY`.
