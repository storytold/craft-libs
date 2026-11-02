# craft-update

## Experimental OCI migration

The `experimental-oci-migration` Cargo feature is disabled by default and only acts on Linux.
Enable it when building to test migrating existing first party Flatpak bundles to the
ArtCraft OCI remote. The launcher exposes the same feature:

```sh
cargo run -p artcraft-launcher --features experimental-oci-migration
```

On an update, a first party installation with a bundle-created `-origin` remote may be
reinstalled from the OCI repository in the same installation scope, using the same full
Flatpak ref. Existing installations managed by another repository are not redirected.
The migration reuses a remote with URL `oci+https://getartcraft.com`, or adds `artcraft-oci`
using `https://getartcraft.com/artcraft-oci.flatpakrepo`. A conflicting remote name is an error.
The current descriptor is a preview with `GPGVerify=false`; enabling this feature explicitly
opts into that repository configuration. The launcher does not uninstall the app or remove
its data. Subsequent updates use the detected OCI remote.

If the exact ref is absent, the existing verified bundle update proceeds. A repository error,
cancellation, or missing/different repository version stops the update; migration requires
an exact match to the selected release version. Launcher pinned/nightly tracks and explicit
replacement operations do not trigger migration. The feature does not change new installs.

Test with an isolated Flatpak installation before enabling this in release builds. The
feature is not enabled by the packaging defaults.
