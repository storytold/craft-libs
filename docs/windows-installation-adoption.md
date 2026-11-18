# Windows installation policy adoption

This is the inventory for the policy across the live app repositories and the launcher, not
merely the repositories on one developer's machine. The app packages below were merged on
27/10/2026 and released on 28/10/2026 as dual-scope MSIs whose authored default is per-user.
The policy since 29/10/2026 is per-machine for new installs: today the launcher applies it
(`ALLUSERS=1`), and per-machine-only packages are follow-up work, one PR per app (see
[installation-policy.md](installation-policy.md)).

| Repository | Prepared Windows scope | Verification boundary |
|---|---|---|
| `effectcraft` | Released dual-scope, per-user authored default; the launcher installs per-machine | x64, x86 and arm64 metadata checks pass; real msiexec runs pass in both scopes (fresh install, upgrade, repair, uninstall; wizard and silent); clean-machine and second-account gates pending |
| `pdfcraft` | Released dual-scope, per-user authored default; the launcher installs per-machine | x64, x86 and arm64 metadata checks pass; real msiexec runs pass in both scopes (fresh install, upgrade, repair, uninstall; wizard and silent); clean-machine and second-account gates pending |
| `lightcraft` | Released dual-scope, per-user authored default; the launcher installs per-machine | x64, x86 and arm64 metadata checks pass; real msiexec runs pass in both scopes (fresh install, upgrade, repair, uninstall; wizard and silent); clean-machine and second-account gates pending |
| `photocraft` | Released dual-scope, per-user authored default; the launcher installs per-machine | x64, x86 and arm64 metadata checks pass; real msiexec runs pass in both scopes (fresh install, upgrade, repair, uninstall; wizard and silent); clean-machine and second-account gates pending |
| `figcraft` | Released dual-scope, per-user authored default; the launcher installs per-machine | x64, x86 and arm64 metadata checks pass; real msiexec runs pass in both scopes (fresh install, upgrade, repair, uninstall; wizard and silent); clean-machine and second-account gates pending |
| `vectorcraft` | Released dual-scope, per-user authored default; the launcher installs per-machine | x64, x86 and arm64 metadata checks pass; real msiexec runs pass in both scopes (fresh install, upgrade, repair, uninstall; wizard and silent); clean-machine and second-account gates pending |
| `wordcraft` | Released dual-scope, per-user authored default; the launcher installs per-machine | x64, x86 and arm64 metadata checks pass; real msiexec runs pass in both scopes (fresh install, upgrade, repair, uninstall; wizard and silent); clean-machine and second-account gates pending |
| `gridcraft` | Released dual-scope, per-user authored default; the launcher installs per-machine | x64, x86 and arm64 metadata checks pass; real msiexec runs pass in both scopes (fresh install, upgrade, repair, uninstall; wizard and silent); clean-machine and second-account gates pending |
| `deckcraft` | Released dual-scope, per-user authored default; the launcher installs per-machine | x64, x86 and arm64 metadata checks pass; real msiexec runs pass in both scopes (fresh install, upgrade, repair, uninstall; wizard and silent); clean-machine and second-account gates pending |
| `filmcraft` | Released dual-scope, per-user authored default; the launcher installs per-machine | x64, x86 and arm64 metadata checks pass; real msiexec runs pass in both scopes (fresh install, upgrade, repair, uninstall; wizard and silent); clean-machine and second-account gates pending |
| `craft-launcher` | Per-user launcher; context-safe app updates | x64, x86 and arm64 metadata checks pass; real installation gates pending |
| `artcraft` | Not applicable | No Windows MSI definition on the inventoried default branch |
| `soundcraft` | Released dual-scope, per-user authored default; the launcher installs per-machine | x64, x86 and arm64 metadata checks pass; real msiexec runs pass in both scopes (fresh install, upgrade, repair, uninstall; wizard and silent); clean-machine and second-account gates pending |
| `cadcraft` | Released dual-scope, per-user authored default; the launcher installs per-machine | x64, x86 and arm64 metadata checks pass; real msiexec runs pass in both scopes (fresh install, upgrade, repair, uninstall; wizard and silent); clean-machine and second-account gates pending |
| `designcraft` | Released dual-scope, per-user authored default; the launcher installs per-machine | x64, x86 and arm64 metadata checks pass; real msiexec runs pass in both scopes (fresh install, upgrade, repair, uninstall; wizard and silent); clean-machine and second-account gates pending |
| `artcraftx` | Not applicable | No Windows MSI definition on the inventoried default branch |
| `AniCraft` | Not applicable | No Windows MSI definition on the inventoried default branch |

## Before publication and release

- Maintainer acceptance of the shared path/scope/model-ownership policy is still required.
- Keep app changes independent. Reconcile open installer UI and file-association PRs rather
  than assuming their work is absent or overwriting it.
- Test real release payloads and previous release MSIs in an authorised clean Windows test
  environment. Verify both standalone and launcher routes, old/custom folders, both scopes,
  second-account visibility, optional shortcuts and menus, active apps, cancellation, rollback,
  repair, uninstall and launcher self-update. Validate actual MSI assignment context, not only the
  registry hive containing a display/uninstall entry.
- Metadata-only fixtures never substitute for those tests. x86/arm64 package validation on an
  x64 host does not prove application execution on those architectures.
- No macOS/Linux layout change, settings migration, user-model relocation or new cleanup is
  included. These platforms retain their native conventions.
- ArtCraft and ArtCraftX have no Windows MSI definition in the inventoried source; AniCraft
  has no default branch/source to package. Do not invent installers for those repositories as
  part of this rollout.
