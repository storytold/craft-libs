# craft-update

Opt-in update notices and administrator-triggered updates for **self-hosted web deployments**
of the Craft apps (GridCraft, WordCraft, DeckCraft). It is off by default and not meant for
desktop users: a desktop build never includes it, and a web build includes it only when built
with the app's `self-hosted-update` feature. Without that feature the app makes no requests and
shows nothing.

For someone who serves an app's wasm build from their own server, it answers two questions
they otherwise have to answer by hand: *is there a newer release?* and *how do I deploy it?*

- Every visitor sees a notice when a newer release exists ("A new version (0.6.1) is
  available. This site runs 0.4.0.").
- Only the administrator, after signing in, can start the update.
- The administrator gets an email when a new release is first seen, and when an update
  finishes (with the end of the build log if it failed).

## How it fits together

```text
browser (app wasm)  --HTTPS-->  web server  --static files-->  /srv/gridcraft/live -> releases/0.6.1-…
        |                           |
        |  /gridcraft/craft-update/ |  reverse proxy, loopback only
        +-------------------------->+-------------------> craft-updated (127.0.0.1:8790)
                                                            |  checks GitHub releases on a timer
                                                            |  emails the administrator (SMTP)
                                                            |  git fetch + checkout tag, build, swap link
```

A web deployment is a folder of static files; no part of it runs on the server. The update
work therefore lives in a separate, deliberately small process, **`craft-updated`**, that the
self-hoster runs next to the web server:

- It listens on a **loopback address only** (it refuses any other) and is reached through a
  reverse-proxied path on the site, so it never faces the internet directly.
- Requests can't say *what* to build or *how*: the only action is "update to the newest
  release you found on GitHub". The tag (held to `v<major>.<minor>.<patch>`), the git clone,
  the build command and the folders all come from its private configuration file. A stolen or
  forged administrator session can at most deploy the project's own latest release early.
- It checks the administrator's credentials itself, on every action. The app's **Update now**
  button is enabled only after a successful sign-in, but that is a convenience; the check that
  matters is in `craft-updated`.
- It runs programs without a shell, under its own account. Run it as a dedicated user that owns
  the clone and the release folders, not as root and not as the web server's user.

## The pieces

| Piece | Feature | Targets |
|---|---|---|
| `Version`, `release` (GitHub release parsing), `protocol` (wire types), `password`, `mail` (notice texts), `client` (request/answer types) | always | all, wasm included |
| `ui::UpdateBanner`: the notice and the sign-in window | `egui` | all |
| `web::Client`: `fetch` to `craft-updated` | `web` | wasm32 |
| `server`: `Daemon`, HTTP listener, GitHub, SMTP, build runner; the `craft-updated` binary | `server` | native |

Apps enable `egui` and `web` in their web crate behind their own `self-hosted-update` feature.
`server` is never part of an app build.

### Requests

| Request | Who | Answer |
|---|---|---|
| `GET api/status` | anyone | `200` current and latest version, whether an update is available or running |
| `POST api/login` `{"username","password"}` | anyone | `200` `{"token","expires_in"}`; `401`; `429` after 5 failures in a row (15 minutes, for everyone) |
| `POST api/update` | administrator (`Authorization: Bearer`) | `202`; `401`; `409` up to date or already running |
| `POST api/check` | administrator | `200` after asking GitHub now; `401` |
| `POST api/logout` | administrator | `204` |

Sessions are random 256-bit tokens held in `craft-updated`'s memory for 15 minutes, never in
cookies (so no cross-site request forgery) and never written to disk; a restart ends them. The
public status leaves out failure details, which go to the administrator's email and the log.

### Storage and secrets

- **Configuration** (`craft-updated setup` writes it): a TOML file created `0600`;
  `craft-updated` refuses to start if group or others can read it, as `ssh` does with keys. It
  holds the SMTP password in clear (the SMTP server needs it) and the administrator password as
  a hash.
- **Administrator password**: iterated, salted SHA-512 (`craft-sha512$<iterations>$<salt>$<hash>`),
  the ECMA-376 construction GridCraft already verifies for workbook protection, with 210,000
  iterations and a 16-byte random salt, compared in constant time. It uses `sha2`, already in
  every app's tree. It is not memory-hard like Argon2; reading it requires the service account.
- **State** (next to the configuration, `<name>.state.toml`): the deployed version and the last
  announced release, so a restart doesn't repeat a notice. The update log is `<name>.state.log`.

### Emails

- A new release: once per release, the first time a check sees a version newer than the
  deployed one. If sending fails it is retried at the next check.
- An update finished: success, or failure with the last 40 lines of the log.
- Nothing when an update starts (the administrator just asked for it).

## Setting it up

1. **Build the app with the feature** and keep a clone for the updater:

   ```sh
   sudo useradd --system --home /srv/gridcraft craftupd
   sudo -u craftupd git clone https://github.com/storytold/gridcraft.git /srv/gridcraft/src
   cd /srv/gridcraft/src/apps/gridcraft-web && trunk build --release --features self-hosted-update
   mkdir -p /srv/gridcraft/releases && cp -r /srv/gridcraft/src/dist/web /srv/gridcraft/releases/0.6.1-0
   ln -sfn /srv/gridcraft/releases/0.6.1-0 /srv/gridcraft/live
   ```

   `live` must be a symbolic link: updates replace it atomically with `rename`, so visitors see
   either the old release or the new one, never a half-copied folder.

2. **Install and configure `craft-updated`**:

   ```sh
   cargo install --locked --git https://github.com/storytold/craft-libs craft-update --features server
   sudo -u craftupd craft-updated setup --config /srv/gridcraft/craft-updated.toml
   sudo -u craftupd craft-updated test-email --config /srv/gridcraft/craft-updated.toml
   ```

   `setup` asks for the repository, the administrator's username, password (twice, not echoed)
   and email, the SMTP server (`starttls` on 587, `tls` on 465, or `none` for a mail server on
   the same machine only), and the build paths. Run it again to change anything;
   `craft-updated password` changes only the password, and `craft-updated hash-password` prints a
   hash for configuration-management tools.

3. **Proxy a path to it** from the web server, next to the app. nginx:

   ```nginx
   location /gridcraft/ {
       alias /srv/gridcraft/live/;
       add_header Cache-Control "no-cache";   # revalidate, so a reload picks up a new release
   }
   location /gridcraft/craft-update/ {
       proxy_pass http://127.0.0.1:8790/;   # the trailing slash strips the prefix
       client_max_body_size 4k;
   }
   ```

   Caddy: `handle_path /gridcraft/craft-update/* { reverse_proxy 127.0.0.1:8790 }`. The app asks
   for `craft-update/api/status` relative to its own page.

4. **Run it** as a service:

   ```ini
   # /etc/systemd/system/craft-updated-gridcraft.service
   [Unit]
   Description=craft-updated for GridCraft
   After=network-online.target
   Wants=network-online.target

   [Service]
   User=craftupd
   ExecStart=/usr/local/bin/craft-updated serve --config /srv/gridcraft/craft-updated.toml
   Environment=PATH=/srv/gridcraft/.cargo/bin:/usr/local/bin:/usr/bin:/bin
   Restart=on-failure
   NoNewPrivileges=true
   PrivateTmp=true
   ProtectSystem=strict
   ReadWritePaths=/srv/gridcraft

   [Install]
   WantedBy=multi-user.target
   ```

   The build needs the Rust toolchain, the `wasm32-unknown-unknown` target and `trunk` on the
   service's `PATH`. One `craft-updated` per app, each with its own configuration and port.

`craft-updated check` and `craft-updated update` do the same from a shell or cron, without the
web interface.

## Configuration reference

```toml
name = "GridCraft"                      # for the emails
repository = "storytold/gridcraft"      # GitHub owner/name
listen = "127.0.0.1:8790"               # loopback only
site_url = "https://example.org/gridcraft/"   # optional, for the emails
check_interval_minutes = 360            # at least 15
# state_file = "/srv/gridcraft/craft-updated.state.toml"

[admin]
username = "admin"
password_hash = "craft-sha512$210000$…$…"
email = "admin@example.org"

[smtp]
host = "smtp.example.org"
port = 587
security = "starttls"                   # "starttls", "tls", or "none" (this machine only)
username = "updates@example.org"        # optional, with password
password = "…"
from = "GridCraft <updates@example.org>"

[build]
source_dir = "/srv/gridcraft/src"       # a clone used only by the updater
remote = "origin"
work_dir = "apps/gridcraft-web"         # relative to source_dir
command = ["trunk", "build", "--release", "--features", "self-hosted-update"]
output_dir = "dist/web"                 # relative to source_dir; must contain index.html
releases_dir = "/srv/gridcraft/releases"
live_link = "/srv/gridcraft/live"       # the symbolic link the web server serves
keep_releases = 3
timeout_minutes = 60
```

An update runs `git fetch --tags <remote>`, verifies and checks out `refs/tags/<tag>` (detached;
local changes in the clone stop it), removes the old output folder, runs the build command in
`work_dir`, copies `output_dir` to `releases_dir/<version>-<unix time>` (symbolic links in the
output are refused), points `live_link` at it and removes releases beyond `keep_releases`
(only folders it named, never the live one). Swapping needs symbolic links, so updates run on
Unix; checking and emailing work everywhere.

## Limits and what it doesn't do

- It trusts the project's GitHub releases and tags as much as a manual `git pull` would; it does
  not verify tag signatures.
- The lockout after failed sign-ins is global (behind a proxy there's no trustworthy client
  address), so someone guessing passwords can keep the update button locked; updating from the
  command line still works.
- It doesn't roll back automatically; the previous releases stay in `releases_dir`, and pointing
  `live_link` back at one is a single `ln -sfn`.
- One administrator account, no user management.

## Tests

`cargo test -p craft-update --all-features` covers version ordering, release parsing (including
tags that must never reach git), the password hash, notice texts, the HTTP parser, and the daemon
with stand-ins for GitHub, SMTP and the build: sign-in, lockout, session expiry, every action
refused without a session (also over a real loopback socket), one email per release across
restarts, success and failure updates, and that a request can't choose what gets built. The
banner tests (`egui_kittest`) check that visitors see the notice with a disabled **Update now**,
that only a successful sign-in enables it, and that a session the service rejects goes back to
sign-in. On Unix the runner's link swap, pruning, symlink refusal and build timeout are tested on
real files and processes.
