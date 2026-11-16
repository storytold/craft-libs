# craft-cloud-local

A reference `CloudBackend` over one folder on disk, proving that `craft-cloud`'s contract needs
no specific provider behind it.

## No-escape policy

Every path this crate is handed, a `Job::List`/`Read`/`Write` path or a `Job::Create` name, is
confined to the configured root before anything touches a filesystem.

A path's segments, and a `Create` name on its own, are accepted only when each one parses as a
single `Component::Normal`: no `..`, no `.`, no empty segment, no absolute form, and no Windows
drive letter, UNC path or backslash. That last group is refused on every platform this crate
builds for, not only on Windows, because a name reaches this backend from whatever platform the
UI runs on.

Once a candidate path is built from nothing but checked names, it is still confirmed against a
symlink or (on Windows) a directory junction placed somewhere under the root: the nearest
ancestor that already exists is canonicalised and must stay under the canonicalised root. A path
that reaches outside the root this way is refused before any read, write, listing or creation
touches it.
