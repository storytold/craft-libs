# Installing the Craft apps on Windows

This is a proposed policy, not a claim about a published release. Preparation covers every
current Craft app with a Windows MSI, plus the launcher's new-install folder. The
[adoption inventory](windows-installation-adoption.md) records the scope and remaining gates.
Folders are preserved except for the proposed automatic migration of an exact former default
path (see below). That exception also covers a manually chosen path with the same spelling;
all other chosen folders stay. Maintainer acceptance of this exception is part of accepting
the policy. Storage of settings, caches and models is separate: [settings-storage.md](settings-storage.md).

## Installation contexts

**The policy (amended 29/10/2026): new installs are per-machine**, so every app sits in one
`Program Files\ArtCraft` folder. Per-user copies that already exist keep their context.

**What the released packages do today differs, and this is a known gap.** Each app ships one
dual-scope MSI (Microsoft's "Single Package Authoring": `ALLUSERS=2` with `MSIINSTALLPERUSER=1`
in the Property table, written by WiX's `Package/Scope="perUserOrMachine"`). That authored
default is **per-user**: run with no scope property, the package installs for one account. So
today only the launcher, which passes `ALLUSERS=1` for a new install, follows the policy by
itself. A person running the MSI directly gets per-machine only by choosing "Everyone" in its
wizard. Closing the gap is follow-up work: one small PR per app making its package per-machine
only (`Scope="perMachine"`, no scope page), which also removes the per-user default here and
in the [adoption inventory](windows-installation-adoption.md).

| Context | Default destination | Who chooses it today |
|---|---|---|
| Per-machine (the policy for new installs) | `%ProgramFiles%\ArtCraft\<App>` (`Program Files (x86)` for a 32-bit package) | The launcher, with `ALLUSERS=1`; the setup wizard's "Everyone" choice |
| Per-user (the package's authored default; kept for existing copies) | `%LOCALAPPDATA%\Programs\ArtCraft\<App>` | An MSI run with no scope property; the wizard's "Only for me" choice; copies installed per-user keep their context on update |

**Permission prompts.** Each all-users install or update asks Windows for permission. One
prompt per **Update all** run was built (craft-launcher#974) with a hidden PowerShell, which
antivirus flagged; it was reverted (craft-launcher#1077). The launcher's signed installer
helper (craft-launcher#1080, in review) does all-users downgrades under one prompt, with no script.
Update all keeps one prompt per all-users package until it uses that helper too. No resident
service is planned.

The launcher itself remains per-user only, with new copies under
`%LOCALAPPDATA%\Programs\ArtCraft\Launcher`. Existing launcher folders and its current
self-update mechanism are preserved.

In a per-user install Windows Installer itself redirects the `ProgramFiles` folder to the
user's own Programs folder and the `HKMU` registry root to `HKCU`, so one package serves both
contexts. Everything a package writes follows its context automatically:

- `InstallDir` record: `HKLM\Software\<App>` per-machine, `HKCU\Software\<App>` per-user.
- File associations and "Open with" registrations: `HKLM\Software\Classes` per-machine,
  `HKCU\Software\Classes` per-user.
- Registry writes use context-sensitive roots. Existing component key paths and app-owned
  associations are retained where valid. Each package must pass the compiled install-policy
  checks; unresolved ICE57 shortcut diagnostics are tracked separately from the path policy.

Scope is chosen before launch conditions and directory costing. The scope page must not be
added as a late checkbox after the default directory has already been resolved.
The page also has to leave `ALLUSERS` unset for "Only for me". Setting `ALLUSERS=2` there makes
Windows Installer search for related products as if the run were per-machine, so an interactive
upgrade skips the existing per-user copy and installs a second registration beside it.
Windows Installer also resolves Program Files once, for the context it started in, so the wizard
sets the new-install folder itself after the choice. Without that, "Everyone" installed into the
user profile with a machine-wide registration.

For the same scope and folder, the ArtCraft Launcher and a direct install of the same MSI
produce the same result: the launcher dispatches the same package and only adds properties.
For an update, those are `INSTALLFOLDER` (the remembered folder) and the context properties of
the copy being updated. For a new install, the launcher also sets `ALLUSERS=1`, so the copy is
per-machine; a direct run of the MSI keeps the package's per-user default unless the person
picks "Everyone" in the wizard.

## What an update must preserve

- **The exact folder, subject to the following exception.** An ordinary update installs into
  the remembered folder of the copy being replaced. Every chosen folder outside the exact
  former default is preserved.
- **Explicit automatic-migration exception: the former default.** This proposal moves a copy
  whose remembered folder is exactly `%ProgramFiles%\<App>\` to
  `%ProgramFiles%\ArtCraft\<App>` on its next update, without a separate migration confirmation.
  It also moves that exact path when someone typed it by hand: Windows Installer kept no
  record that distinguishes a chosen path from an accepted default. Matching the path is not
  evidence that the person accepted a default. An unreadable or different folder stays.
  Maintainers must accept this exact-path exception as part of the policy before it is
  adopted; it must not be described as preserving every manually chosen folder. Package-owned
  shortcuts are recreated, but a shortcut someone made by hand to the old path may break.
  Requiring migration consent instead is a change to this proposed rule; an in-app choice
  would not itself require another Windows elevation prompt.
- **The context.** A per-machine copy updates per-machine (`ALLUSERS=1`), a per-user copy
  per-user. MSI major upgrades cannot cross installation contexts, so a context change is a
  deliberate uninstall-and-reinstall, never an ordinary upgrade. The launcher queries
  authoritative MSI registration, not the hive containing a display entry. An unknown or
  conflicting context stops the update rather than selecting the new-install default.
- **No duplicates.** Installing in the other context while a copy already exists is a visible
  stop with a message naming the fix, never a silent second copy. The wizard pre-ticks the
  matching scope when a copy in the other context is registered.
- **The chosen folder survives elevation.** `INSTALLFOLDER` is a secure MSI property, so
  the elevated installer receives the same exact destination selected in the setup wizard.
- **Custom folders stay first-class.** A chosen exact app folder is used as-is; a chosen
  suite root gets the app subfolder appended once (`D:\Craft` becomes `D:\Craft\PhotoCraft`,
  never `D:\Craft\PhotoCraft\PhotoCraft`). A missing drive or an unavailable folder is a
  visible error, not permission to fall back to the default.

## What an app must still do without the launcher

Install, launch, update and uninstall work standalone: the MSI's own wizard, its App Paths
registration (`Win+R`), its Start Menu shortcut and its entry in "Apps & features" never
require the launcher. The launcher reuses a standalone installation it finds rather than
installing a duplicate, and detection reads both contexts' registrations.

## Other platforms

macOS `.app` bundles, Linux packages/AppImages and FreeBSD tarballs keep their platform-native
conventions; nothing imitates the Windows directory tree. Linux settings live under
`XDG_CONFIG_HOME` and persistent models under `XDG_DATA_HOME` (`~/.local/share/ArtCraft/models`)
per [settings-storage.md](settings-storage.md).

## Models are not caches

Downloaded model files are persistent user assets. An app's cache cleanup and an app's
uninstaller must never delete them, another app's data, or user documents; existing model
folders are discovered read-only. See [settings-storage.md](settings-storage.md) for where
models live and what is disposable. Model resources shipped inside an application package,
such as bundled OCR resources, remain part of that package; this does not give an installer
ownership of separately downloaded shared models or discovered model folders.
