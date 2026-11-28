# craft-cloud-local

A reference `CloudBackend` over one folder on disk, proving that `craft-cloud`'s contract needs
no specific provider behind it.

## No-escape policy

Every path this crate is handed, a `Job::List`/`Read`/`Write` path or a `Job::Create` name, is
confined to the configured root before anything touches a filesystem, in two layers.

A path's segments are accepted only when each one parses as a single `Component::Normal`: no
`..`, no `.`, no empty segment, and no absolute form. A `Create` name goes through a stricter
check of its own, since it is one chosen name and not a path: it additionally refuses a
backslash, a colon, or a Windows reserved device name (`CON`, `COM1`, `CONIN$`, and so on), on
every platform this crate builds for, not only on Windows, because a chosen name reaches this
backend from whatever platform the UI runs on. An existing name that already has one of those
characters, discovered through `list`, stays reachable: `:` and `\` are not path separators on
any platform's filesystem, so refusing to read a real file back because of a byte in its name
(macOS stores a `/` typed in Finder as `:` on disk) would be a usability bug with no matching
security benefit.

The second layer is the one that actually keeps resolution inside the root: every already
segment-checked relative path is handed to a [`cap-std`](https://docs.rs/cap-std) `Dir`
capability opened on the root, never joined onto the root as an absolute OS path this crate
builds itself. `cap-std` resolves one path component at a time against real, already-open
directory handles, so a symlink or (on Windows) a directory junction anywhere under the root,
dangling or already pointing outside, cannot walk resolution above the root. There is no separate
"does this still look like it is inside the root" check with a window for the directory tree to
change underneath it between the check and the actual read or write; the one resolution is the
open.

## Hard links

A hard link is not a symlink, and this backend does not refuse one: from inside the sandbox the
inode it names is an ordinary file. What it protects instead is the shared inode's content: a
write never truncates its target in place. It writes a new sibling file and renames that sibling
over the target, so the target's directory entry is replaced rather than its bytes being
overwritten. A rename only ever changes one name; it does not reach through to any other name,
inside the root or out, that happens to point at the same inode.

## Concurrent saves

A `Job::Write` checks the file's etag again just before the rename, and holds one lock for this
process from that check until the rename is done. Within one process, when two saves are made
from the same etag, the second gets `Fault::Conflict`, unless the first save left the file with an
unchanged etag (see the next paragraph). A second process writing the same folder does not take
that lock, so its save can still land between the check and the rename. A deployment that needs
the guarantee across processes needs a backend whose provider offers a conditional write.

The etag is the file's length and modification time. On a filesystem whose timestamps are coarser
than the time between two saves, two saves of the same length can leave the same etag, and the
second save is then not refused.
