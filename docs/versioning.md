# Versioning and releases

craft-libs releases give applications a fixed dependency baseline. Consumers decide when to
upgrade; development on `main` is not a reason to refresh a working application's pins.

## One workspace, one release

All crates inherit `workspace.package.version` and ship together under an annotated `vX.Y.Z`
Git tag. A release contains the crates listed in that tag's README, not proposed crates in
open pull requests. A crate can keep the same implementation across several workspace releases.
Use the same tag or commit for every craft-libs dependency in a consumer's dependency graph;
mixing Git sources can create duplicate crate instances and incompatible Rust types.

Tags identify reviewed commits and are never moved, deleted or reused to fix a release.
Corrections get a new version. The release notes and policy are stored with the source so a
consumer can read the contract that applied to its version. GitHub Releases carry the same notes.
Publishing to crates.io will be a separate step; these tags do not imply registry availability.

## Compatibility while versions are 0.x

The initial release is `0.1.0`. APIs are still developing, but compatibility within a minor
release line is intentional:

- **Patch (`0.1.0` → `0.1.1`):** compatible fixes, documentation and additive APIs that preserve
  existing callers. Keep public signatures, feature defaults, supported targets, minimum Rust
  version and documented behavior compatible. Check that additions do not break exhaustive
  matches, struct literals, trait implementations or other existing source patterns.
- **Minor (`0.1.x` → `0.2.0`):** changes requiring consumer action. This includes removing or
  changing public APIs, incompatible persisted representations, feature/default changes,
  dropping a supported target, raising the minimum Rust version, or upgrading a dependency
  such as egui when its exposed types become incompatible. Explain the before/after and migration.
- **`1.0.0`:** a separate decision that the public contracts are ready for long-term stability.
  After that, breaking changes require a major release; compatible additions use minor releases.

Documented invalid input can be rejected more strictly in a patch to fix a defect or security
issue. Call out any observable change. Do not use “0.x” or “bug fix” to silently break a valid,
documented workflow. App-owned settings schemas and migration policy remain the app's
responsibility; library releases do not automatically migrate consumer data.

For public egui APIs in `0.1.x`, keep the egui 0.36 compatibility line. The minimum Rust version
is 1.95. Compatible transitive updates still need validation and release-note coverage when
observable. Cargo's version-range rules do not select newer Git tags: changing a release pin
is always an explicit manifest edit.

## Support and scope

The newest minor line receives routine fixes. Important fixes to older lines are considered
case by case; there is no promise of maintaining every 0.x line or an LTS schedule. Existing
tags remain available so consumers can upgrade on their own schedule. Release notes identify
known limitations; a tag is not a claim that every driver, OS version or consuming app was tested.

Platform boundaries are crate-specific. See each crate's API/README and the release notes.
In particular, native printing needs the OS spooler, settings persistence is native-only, and
browser UI compatibility does not imply native services work in a browser. Consumers must test
their own save/close flows, accessibility, rendering and printing where they use those services.

## Preparing a release

1. Choose a reviewed commit on `main`. Inspect open defects, changes since the previous tag,
   crate metadata and any changed platform/dependency contracts. Update the shared version
   and lockfile when needed; the first release already uses `0.1.0`.
2. Add a dated `CHANGELOG.md` entry with the included crates, changes, compatibility requirements,
   migration steps and known limits. Update the README's recommended pin. Keep fixes and
   unrelated pending features out of the release preparation unless they block the baseline.
3. Run formatting, workspace Clippy with warnings denied, and workspace tests, both default
   and all features. The `Workspace` workflow runs these on Linux, macOS and Windows, requires
   Windows spooler regression tests, checks the minimum compiler on Linux, and checks the
   documented UI/settings WASM surface. All release jobs must pass for the candidate source.
   Repository-local build commands must also follow the host's storage/allocation rules.
4. Review consumer impact and existing platform evidence. Exercise affected native workflows
   when behavior changes; unit tests and cross-compilation cannot establish GUI or physical
   printer acceptance. Record limitations instead of extending the support claim beyond evidence.
5. Merge the preparation PR and verify the checks for the actual release commit. Create an
   annotated tag on that exact commit (`git tag -a vX.Y.Z FULL_SHA -m "craft-libs vX.Y.Z"`),
   push only that tag, and create a GitHub Release using that version's changelog entry.
   Tag-triggered CI should also pass. Do not publish crates as part of this Git-only release.
6. Verify that the remote tag resolves to the intended commit, the release notes are visible,
   and a small external consumer resolves and checks with the documented pin. Preserve the
   source identity, check results and release URL. Never repair a published tag by moving it.

The workflow validates source; it does not automatically tag or publish on every push.
