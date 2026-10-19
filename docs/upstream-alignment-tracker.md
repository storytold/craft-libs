# Upstream Alignment Tracker — craft-libs#132 Epic

**Scope:** craft-runtime / tier-store / worker-host / craft-gpu provider stack (web perf),
and every pending upstream handoff that blocks a clean story between the wdunn001 fork,
the Forgejo mirror, and storytold/craft-libs.

**As of:** 2026-10-15 · fork HEAD `1d4ffc1` · storytold upstream HEAD `8f5a283`

---

## 1. Fork vs Upstream Divergence

The wdunn001 fork's `main` is significantly ahead of the storytold upstream. Every crate
introduced by this epic landed via PR #11 on the fork. None of it has been submitted or
merged to `storytold/craft-libs` outside this track's upstream PRs.

| Crate | In wdunn001 fork | In storytold upstream |
|---|---|---|
| `craft-cloud` | ✅ (`137d9eb`) | ❌ open PR #48 |
| `craft-tier-store` | ✅ (`1e81490`) | ❌ no upstream PR |
| `craft-worker-host` | ✅ (`54cef4f`) | ❌ no upstream PR |
| `craft-runtime` | ✅ (`145aa03`) | ❌ no upstream PR |
| `craft-gpu` / `craft-gpu-wgpu` / `craft-gpu-null` / `craft-gpu-providers` | ✅ (PR #11 merge `2d23f6d`) | ❌ no upstream PR |
| `craft-ui` cloud/presence/menus | ✅ (`194a7f1`) | ❌ open PR #52 (stacked on #48) |

The storytold upstream's recent merges are limited to the print stack, CI timeout limits
(#136), and docking/UI fixes. Nothing from the runtime epic has crossed over.

---

## 2. craft-libs#123 — collab-craft Transfer Checklist

collab-craft is ready at `wdunn001/collab-craft @ 80bfaec`. It includes PR #12 (invite
flow), the suite client crates (`craft-cloud-client`, `collabcraft-share`), and GHCR
image paths that already reference `ghcr.io/storytold/collab-craft`.

The transfer is blocked only by org-owner action.

**Checklist (in order):**

- [ ] Org owner creates `storytold/collab-craft` **or** enables repository transfers into
  the storytold org.
- [ ] Transfer `wdunn001/collab-craft` → `storytold/collab-craft`; confirm default branch
  `main` and branch protection consistent with other Craft repos.
- [ ] Verify GHCR image path resolves to `ghcr.io/storytold/collab-craft` post-transfer.
- [ ] Update any app `Cargo.toml` git deps that reference `wdunn001/collabcraft` to
  `storytold/collab-craft` at tagged releases or pinned revs.
- [ ] Merge `craft-cloud` to `storytold/craft-libs` main (PR #48 → see §3) so the
  collab-craft workspace pin at `storytold/craft-libs@289d562` can advance to main.

**Note from #123:** collab-craft's workspace `Cargo.toml` currently pins
`storytold/craft-libs` at rev `289d562`. That rev predates the cloud crates on the
upstream. The pin can only advance once §3 is resolved.

---

## 3. craft-libs#48 / #52 — Pin State vs wdunn001 Fork

Two PRs carry the cloud contracts and UI into the storytold upstream, and both are open.

**PR #48** — `craft-cloud` (provider-neutral contracts) + `craft-ui` share dialog and
account control. No IO, no vendor; builds for `wasm32`. Gates passed on `rust:1-slim` at
rev `e9546e1`.

**PR #52** — Presence row, overflow/split menus, multi-account support. Stacked on #48.
Gates passed on `rust:1-slim` at `23f7a82`. Identified a `chunks_exact_to_as_chunks` lint
regression in `crates/cms/tests/fogra39.rs` at rustc 1.99 (not caused by the PR; not
caught by the current workflow).

**Pin audit:**

| Consumer | Remote pinned | Rev pinned |
|---|---|---|
| `wdunn001/designcraft` (`designcraft#878`) | `wdunn001/craft-libs` | `effb5ca` |
| `wdunn001/collab-craft` | `storytold/craft-libs` | `289d562` |

These two consumers point at different remotes and different revisions. Until #48 merges to
`storytold/craft-libs`:

- designcraft's git dep remains non-compliant with `designcraft#878` (see §4).
- collab-craft cannot advance its pin past `289d562` without pinning the fork directly,
  which is also non-compliant.

**Action required:** Review and merge #48, then #52 (unstacking it in the process). Once
they land, both consumers can be updated to `storytold/craft-libs` at the merge rev.

---

## 4. designcraft#878 — No Permanent wdunn001 Git Deps

designcraft#878 introduces the CloudApp / Destination / etag pattern for DesignCraft cloud
open/save. The implementation requires `craft-ui`'s `cloud` feature. The PR's `cargo tree`
check currently reports `wdunn001/craft-libs@effb5ca` as the only revision for `craft-ui`
across all three crates that use it (`designcraft-ui-egui`, `designcraft`, and
`collabcraft-share`). This is correct for the single-rev invariant, but the remote is wrong.

The constraint from the issue is clear: no wdunn001 git dep should be permanent. Every
`path = "..."` dep is fine; every `git = "https://github.com/wdunn001/..."` dep in a
published ref must resolve before a release.

**Current violation:** `designcraft` → `wdunn001/craft-libs@effb5ca`

**Resolution path:**
1. #48 merges to `storytold/craft-libs`.
2. `designcraft#878` is rebased or re-pointed to the new storytold rev.
3. `cargo tree -i craft-ui` confirms a single rev from `storytold/craft-libs`.
4. Same update applied to `collabcraft-share` if it carries a git dep on the fork.

Until step 1 completes, `designcraft#878` cannot be merged without violating the constraint.

---

## 5. Merged vs Lab-Only — Summary

**Merged to `storytold/craft-libs` main:**
- Print stack: `craft-print`, `craft-job`, `craft-scan`, `craft-wsd` (multiple PRs through
  #118–#122, now in DRAFT/OPEN continuations)
- CI runtime limits: #136
- Docking, UI, and test-portability fixes: #107–#110

**Lab-only (wdunn001 fork + Forgejo mirror; not on storytold upstream):**
- All of craft-libs#132's crates: `craft-cloud`, `craft-tier-store`, `craft-worker-host`,
  `craft-runtime`, `craft-gpu`, `craft-gpu-wgpu`, `craft-gpu-null`, `craft-gpu-providers`
- Cloud and presence UI from #48 / #52
- `collab-craft` (pending transfer, §2)
- `designcraft#878` CloudApp / etag pattern (pinned to fork, §4)

The fork is at least 30 commits ahead of the upstream on the runtime/cloud axis. The
Forgejo mirror tracks the fork. The storytold upstream has never seen these crates.

---

## 6. Ordered Action Plan

| Priority | Action | Blocks |
|---|---|---|
| 1 | Reviewer approves and merges storytold/craft-libs#48 | §3, §4, §2 (pin advance) |
| 2 | Merge storytold/craft-libs#52 after #48 lands | §4 |
| 3 | Org owner executes collab-craft transfer (#123) | §2 |
| 4 | Update collab-craft workspace pin from `289d562` to post-#48 rev | §2 |
| 5 | Rebase `designcraft#878` onto storytold/craft-libs, repoint git dep | §4 |
| 6 | Open upstream PRs for `craft-tier-store`, `craft-worker-host`, `craft-runtime`, `craft-gpu*` | §1 |

Nothing in steps 3–6 is safe to attempt before step 1 completes. The craft-cloud contracts
are the common dependency everything else imports.

---

*Drafted from `gh issue view` output for storytold/craft-libs#48, #52, #123, #132 and
storytold/designcraft#878, plus local `git log` on a local checkout of the wdunn001 fork.
All status claims are verified against the GitHub API or the local object store; nothing
in this document is inferred.*
