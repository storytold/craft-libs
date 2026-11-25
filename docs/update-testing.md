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

## Copies a package manager updates

A distribution's package (a Nix package, say) can't update itself, and a release downloaded
by hand may not run there. Its wrapper script sets both of these before it starts the app:

```sh
export CRAFT_UPDATE_MANAGED_BY='Nix'
export CRAFT_UPDATE_MANAGED_HINT='Update your Nix sources and rebuild to get the new version.'
```

The check still runs. When it finds a newer release, the shared startup prompt (also after
Help → Check for Updates) shows the release notes link and "{app} {version} is available. {app}
was installed with {manager}, so it can't update itself.", followed by the hint in full. A hint
taller than the space the card has for it (the window height less the header, footer and
margins) scrolls. The prompt offers no Download, no Update and Restart and no Save and
Update. Nothing self-updates, even with an inherited `APPIMAGE` variable, and the `Updater`'s
check at start leaves the update records of another copy sharing the data folder alone. The
sentence goes through the app's translator, which falls back to English. The package manager's
name and the hint are trimmed and otherwise shown as given. Plain ASCII is safest for both,
since an app's fonts may lack other characters.

Both variables must have a value. When one has a value and the other is unset or empty, both
are ignored and a warning is logged; when neither has one, nothing is logged. A version chosen
in the ArtCraft Launcher still comes first. Programs that `craft-update` starts (an app the
launcher starts, a Flatpak run, a file opened) and apps started through `craft-ipc` don't
inherit the variables; a Craft App gets them from its own wrapper. An app with update screens of
its own reads the setting through `craft_update::distribution()` to show the hint.

To preview the prompt, combine these variables with a pretend version from above.
