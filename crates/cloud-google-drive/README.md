# craft-cloud-google-drive

Google Drive behind `craft-ui`'s shared cloud Open and Save pages: a `CloudBackend` that talks to
a person's own Google Drive directly, with no service in between and no OAuth client built in.

## What it does

| Job | Against Drive |
|---|---|
| `Account` | The signed-in person's name (`about.user`). No stored sign-in: `Fault::NotLinked`, so the page offers to link. No client: `Fault::Refused` with what to set up. |
| `SignIn`, `LinkStart`, `LinkPoll` | Google's installed-app flow: a one-shot listener on `127.0.0.1`, the consent page in the system browser, PKCE (RFC 7636, S256) and a random `state`. |
| `List(path)` | One folder, folders first. Paths are slash-separated names; the backend maps them to Drive folder ids. |
| `SharedWithMe` | Files other people shared, with who shared them. Folders are left out (they have no path here). Offered only with the full `drive` scope. |
| `Read` | The file's bytes. Google Docs, Sheets and Slides are exported as `.docx`, `.xlsx` and `.pptx`. |
| `Write` | Replaces the file if its content revision is still the one that was read; otherwise `Fault::Conflict` and nothing is uploaded. Best effort: the revision is checked just before the upload, not by Drive (see **Conflicts**). |
| `Create` | A new file in the folder being browsed. A name already there is refused. |
| `SignOut` | Revokes the grant at Google and removes the token. |

- **Conflicts.** Drive v3 has no conditional update, so the check reads the file's revision and
  then uploads (and reads it again before a retry after a refreshed token). A change landing in
  the fraction of a second between the check and the upload isn't caught. Open reads the
  revision before and after the download, so the bytes and the revision a save checks against
  match.
  Renaming or sharing a file doesn't count as a change: only its content does.
- **Trash.** A file someone moved to the trash is never saved over: Save As keeps a copy.
- **Native Google files** are never overwritten. They list as not writable, so Save points to
  Save As, which writes an Office copy.
- **Paths.** Paths come from listings: two folders with one name in the same folder get
  distinct paths, numbered in id order (`Reports`, `Reports (2)`) whatever order Drive answers
  in, and a `/` in a name shows as `∕`. A path the backend hasn't seen is found by listing its
  parents, never by searching for a name. The numbers are positions, not identities: when one
  of two same-named folders is removed or renamed, the other's path can change.
- **Tokens.** The refresh token is kept in the system keychain through `craft-suite-account`'s
  `TokenStore`, one entry per OAuth client and scope: narrowing `"drive"` to `"file"` asks for
  consent again rather than reusing the wider grant. It is never written to a file. With no keychain on the machine,
  a sign-in lasts until the app quits. **Sign out** removes the token first, even during a
  transfer.
- **Limits.** 256 MB per document; a folder lists its first 5,000 entries.
- **Logging.** `ureq_proto` dumps the wire at `TRACE` level without redacting it, tokens
  included. While a host's logger has that on, the backend sends nothing and says why, so a
  token never reaches a log.
- **Desktop only.** On the web the crate compiles to nothing: a browser can't receive Google's
  loopback redirect.

## Wiring it into an app

```rust
use craft_cloud_google_drive::GoogleDriveBackend;

// The client from `GRIDCRAFT_GOOGLE_CLIENT_ID` / `…_SECRET`, else `google-drive.json` in the
// profile folder. A changed registration is a new backend, connected again.
let backend = GoogleDriveBackend::from_profile(Some(&profile_dir), "GRIDCRAFT", "GridCraft");
app.cloud_mut().connect(backend);
```

`write_client` and `import_client` write `google-drive.json` (owner-only on Unix) for an app's
settings page; `downloads_dir().and_then(|d| newest_client_file(&d))` finds the JSON Google Cloud console just
downloaded.

## Setting up a client

Each person, or their organisation, uses their own Google Cloud project:

1. In the Google Cloud console, create a project and enable the **Google Drive API**.
2. Set up the **OAuth consent screen**. For yourself: user type *External*, publishing status
   *Testing*, and your own account under **Test users**. Google expires a *Testing* app's
   sign-ins after 7 days; publishing the app (or, in Google Workspace, user type *Internal*)
   avoids that.
3. Under **Clients**, create an OAuth client ID of type **Desktop app** and download its JSON.
4. Save it as `google-drive.json` in the app's profile folder, or let the app import it:

```json
{ "client_id": "1234-abc.apps.googleusercontent.com", "client_secret": "…", "scope": "drive" }
```

`scope` is `"drive"` (the default: any file in the person's Drive) or `"file"` (Drive's
`drive.file` scope: only files the app created or was given). Google documents a Desktop
client's secret as not confidential; keep it out of repositories anyway.

## Tests

`cargo test -p craft-cloud-google-drive`. The OAuth and Drive layers run against a scripted
transport and an in-memory token store; `tests/backend.rs` drives the backend through
`craft_ui::cloud`'s public functions, the way an app does. Nothing reaches the network or the
keychain. The sign-in test uses the real loopback listener on `127.0.0.1`.

Written from Google's public OAuth 2.0 and Drive v3 REST documentation.
