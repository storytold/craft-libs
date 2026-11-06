# Testing update prompts

Every native Craft App accepts an environment variable derived from its app id:
uppercase, hyphens replaced with underscores, followed by `_PRETEND_VERSION`.
Run each command from the corresponding app checkout, with a `craft-update` pin that
includes this fix. These app packages are not part of the `craft-libs` workspace.
For example:

```sh
LIGHTCRAFT_PRETEND_VERSION=0.0.1 cargo run -p lightcraft
WORDCRAFT_PRETEND_VERSION=0.0.1 cargo run -p wordcraft
PDFCRAFT_PRETEND_VERSION=0.0.1 cargo run -p pdfcraft
```

The shared updater uses this version for startup checks, manual checks, the prompt's
running-version label, and installation checks. It still fetches real release metadata;
a network error or an app with no published newer release cannot produce an update prompt.
Use Help → Check for Updates to see errors that the startup check only logs.

An app-specific setting takes priority over the legacy shared
`ARTCRAFT_LAUNCHER_PRETEND_VERSION` setting. An invalid version falls back to the
compiled package version. Leave the variables unset for normal version detection.
The restart helper clears both settings for the app so a newly installed copy reports
its real version.

`CRAFT_UPDATE_DEV=1` separately lets a development executable be treated as an installed
copy for testing installation. It is unnecessary to preview a prompt. Download/install
buttons perform a real update; a pretend version does not simulate installation.
The explicit override also applies to the installation version check, allowing tests
to reinstall the current release or replace a newer development build. Leave it unset
to retain the normal protection against installing an older or equal version.

Web builds have no updater. These settings apply to native builds only.
