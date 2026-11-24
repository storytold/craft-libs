# Craft Flags

`craft-flags` lets a feature ship in every Craft app and the ArtCraft Launcher while it stays off
until a person turns it on. Every flag is off by default.

- **One file for the suite:** `ArtCraft/flags.toml` in the per-user local configuration folder
  (`%LOCALAPPDATA%` on Windows, `~/Library/Application Support` on macOS, `$XDG_CONFIG_HOME` on
  Linux), beside the AI layer's `ArtCraft/ai`. `CRAFT_FLAGS_FILE` names another file.
- **One run:** `CRAFT_FLAGS=AI-MCP,-AI-MODELS` overrides the file.
- **Never crashes:** a missing or damaged file means every flag is at its default, with the
  reason in `Flags::problems`. `set` changes one flag atomically and keeps everything else in the
  file; it refuses to touch a file it cannot read.
- **Browser:** no file, no environment: every flag is at its default.

## Flags

| Constant | Id | What it gates |
|---|---|---|
| `AI_MCP` | `AI-MCP` | AI agents using the Craft apps through MCP: the launcher's AI agents settings, the `craft-ai-mcp` gateway, the apps' agent connection settings |
| `AI_MODELS` | `AI-MODELS` | Choosing AI models and services for AI tools such as Generative Fill |

A new flag is a new constant here and a row in `ALL`, so the launcher can list it and every app
names it the same way.

## Use

```rust
let flags = craft_flags::load();
if flags.enabled(&craft_flags::AI_MCP) {
    // show the agent connection settings
}
```

The launcher turns a flag on with `craft_flags::set(&path, craft_flags::AI_MCP.id, true)`, where
`path` is `craft_flags::default_path()`. Apps read the flags when they start; a flag changed
while an app runs takes effect at its next start, unless the app loads them again.
